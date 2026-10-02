use dear_imgui_rs::{
    Condition,
    DragFlags,
    Key,
    TableFlags,
    TableSizingPolicy,
    Ui,
    WindowFlags,
    WindowKey,
    WindowKeyError,
};
use editor::environment::BundledProfile;

use super::{
    common::{button_width, dpi, focus_window_on_hover},
    dialog::draw_keybind_preset_dialog,
};
use crate::{
    session::Session,
    settings::{
        BINDABLE_KEYS,
        KeyBinding,
        KeyBindings,
        KeybindAction,
        MODIFIER_KEYS,
        ObjectTreeFilterOptions,
        ObjectTreeSearchOptions,
        SelectionHighlight,
        Settings,
        UI_SCALE_PERCENT,
        backup_dir,
    },
    ui::ProfileReload,
};

const SETTINGS_WINDOW_SIZE: [f32; 2] = [760.0, 560.0];
const SETTINGS_WINDOW_MIN_SIZE: [f32; 2] = [620.0, 420.0];
const SETTINGS_CATEGORY_WIDTH: f32 = 160.0;
const PROFILE_RELOAD_POPUP: &str = "Reload codebase profile?";
const SETTINGS_TITLE_SCALE: f32 = 1.4;
const SETTINGS_SECTION_SCALE: f32 = 1.2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum SettingsCategory {
    #[default]
    General,
    Viewport,
    Compiler,
    Git,
    ObjectTree,
    Keybindings,
}

struct SettingsWindowState<'a> {
    open: &'a mut bool,
    category: &'a mut SettingsCategory,
    capturing: &'a mut Option<KeybindAction>,
    resetting_keybinds: &'a mut bool,
    measured: &'a mut [f32; 2],
    pending_profile: &'a mut Option<ProfileReload>,
    ui_scale_draft: &'a mut Option<u32>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct SettingsWindowOutput {
    pub object_tree_changed: bool,
    pub reload_profile: Option<ProfileReload>,
}

impl SettingsCategory {
    const ALL: [Self; 6] = [
        Self::General,
        Self::Viewport,
        Self::Compiler,
        Self::Git,
        Self::ObjectTree,
        Self::Keybindings,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Viewport => "Viewport",
            Self::Compiler => "Compiler",
            Self::Git => "Git",
            Self::ObjectTree => "Object Tree",
            Self::Keybindings => "Keybindings",
        }
    }
}

const SETTINGS_KEYBINDING_GROUPS: &[(&str, &[KeybindAction])] = &[
    (
        "Application",
        &[
            KeybindAction::Save,
            KeybindAction::SaveAll,
            KeybindAction::CloseMap,
            KeybindAction::Screenshot,
        ],
    ),
    (
        "Editing",
        &[
            KeybindAction::Undo,
            KeybindAction::Redo,
            KeybindAction::Copy,
            KeybindAction::Cut,
            KeybindAction::Delete,
            KeybindAction::Paste,
            KeybindAction::Deselect,
            KeybindAction::Find,
            KeybindAction::FindNext,
            KeybindAction::FindPrevious,
        ],
    ),
    (
        "Viewport",
        &[
            KeybindAction::ToggleAreaLayer,
            KeybindAction::ToggleTurfLayer,
            KeybindAction::ToggleObjLayer,
            KeybindAction::ToggleMobLayer,
            KeybindAction::ShowAllLayers,
            KeybindAction::ShowAreas,
            KeybindAction::ShowAreaOutlines,
            KeybindAction::ShowLighting,
            KeybindAction::ShowTileGrid,
            KeybindAction::ShowPixelGrid,
            KeybindAction::LevelUp,
            KeybindAction::LevelDown,
            KeybindAction::Refit,
            KeybindAction::GoTo,
        ],
    ),
    (
        "Camera",
        &[
            KeybindAction::PanLeft,
            KeybindAction::PanRight,
            KeybindAction::PanUp,
            KeybindAction::PanDown,
            KeybindAction::PanFaster,
            KeybindAction::PanDrag,
            KeybindAction::ZoomIn,
            KeybindAction::ZoomOut,
        ],
    ),
    (
        "Tools",
        &[
            KeybindAction::PlaceTool,
            KeybindAction::SelectTool,
            KeybindAction::NodeTool,
            KeybindAction::BlockSelectTool,
            KeybindAction::DeleteTool,
            KeybindAction::ReplaceTool,
            KeybindAction::FillTool,
            KeybindAction::CommentTool,
            KeybindAction::Rotate,
            KeybindAction::ToolAlternate,
        ],
    ),
    ("Recent", &KeybindAction::RECENT),
];

pub(super) struct SettingsWindow {
    window: WindowKey,
    open: bool,
    category: SettingsCategory,
    capturing: Option<KeybindAction>,
    resetting_keybinds: bool,
    measured: [f32; 2],
    pending_profile: Option<ProfileReload>,
    ui_scale_draft: Option<u32>,
}

impl SettingsWindow {
    pub(super) fn new() -> Result<Self, WindowKeyError> {
        Ok(Self {
            window: WindowKey::new("settings-v2", "Settings")?,
            open: false,
            category: SettingsCategory::default(),
            capturing: None,
            resetting_keybinds: false,
            measured: SETTINGS_WINDOW_SIZE,
            pending_profile: None,
            ui_scale_draft: None,
        })
    }

    pub(super) fn open(&mut self) { self.open = true; }

    pub(super) const fn is_capturing_keybind(&self) -> bool { self.capturing.is_some() }

    pub(super) fn draw(
        &mut self, ui: &Ui, session: &mut Session, settings: &mut Settings, loading: bool,
    ) -> SettingsWindowOutput {
        draw_settings_window(
            ui,
            &self.window,
            SettingsWindowState {
                open: &mut self.open,
                category: &mut self.category,
                capturing: &mut self.capturing,
                resetting_keybinds: &mut self.resetting_keybinds,
                measured: &mut self.measured,
                pending_profile: &mut self.pending_profile,
                ui_scale_draft: &mut self.ui_scale_draft,
            },
            session,
            settings,
            loading,
        )
    }

    pub(super) fn finish_keybind_capture(&mut self, ui: &Ui, keybindings: &mut KeyBindings) {
        finish_keybind_capture(ui, &mut self.capturing, keybindings);
    }
}

fn draw_settings_window(
    ui: &Ui, window: &WindowKey, state: SettingsWindowState<'_>, session: &mut Session, settings: &mut Settings,
    loading: bool,
) -> SettingsWindowOutput {
    let SettingsWindowState {
        open,
        category,
        capturing,
        resetting_keybinds,
        measured,
        pending_profile,
        ui_scale_draft,
    } = state;
    if !*open {
        *capturing = None;
        *ui_scale_draft = None;

        return SettingsWindowOutput::default();
    }

    let center = ui.main_viewport().work_center();
    let position = [center[0] - measured[0] / 2.0, center[1] - measured[1] / 2.0];
    let flags = WindowFlags::NO_COLLAPSE | WindowFlags::NO_DOCKING;
    let mut object_tree_changed = false;
    let scale = dpi(ui);
    ui.window(window)
        .opened(open)
        .position(position, Condition::Appearing)
        .size(SETTINGS_WINDOW_SIZE.map(|size| size * scale), Condition::FirstUseEver)
        .size_constraints(SETTINGS_WINDOW_MIN_SIZE.map(|size| size * scale), [f32::MAX, f32::MAX])
        .flags(flags)
        .build(|| {
            if settings.focus_windows_on_hover {
                focus_window_on_hover(ui);
            }
            let content_height = ui.content_region_avail()[1].max(1.0);
            ui.child_window("settings-categories")
                .size([SETTINGS_CATEGORY_WIDTH * dpi(ui), content_height])
                .border(true)
                .build(ui, || {
                    for candidate in SettingsCategory::ALL {
                        if ui
                            .selectable_config(candidate.label())
                            .selected(*category == candidate)
                            .build()
                            && *category != candidate
                        {
                            *category = candidate;
                            *capturing = None;
                        }
                    }
                });

            ui.same_line();
            ui.child_window(format!("settings-content-{}", category.label()))
                .size([0.0, content_height])
                .border(true)
                .build(ui, || {
                    draw_heading(ui, category.label(), SETTINGS_TITLE_SCALE);
                    ui.separator();

                    match category {
                        SettingsCategory::General => draw_general_settings(ui, settings, ui_scale_draft),
                        SettingsCategory::Viewport => draw_viewport_settings(ui, session, settings),
                        SettingsCategory::Compiler => {
                            draw_compiler_settings(ui, session, settings, loading, pending_profile)
                        },
                        SettingsCategory::Git => draw_git_settings(ui, settings),
                        SettingsCategory::ObjectTree => {
                            object_tree_changed |= draw_object_tree_settings(ui, settings);
                        },
                        SettingsCategory::Keybindings => {
                            draw_keybinding_settings(ui, capturing, resetting_keybinds, settings)
                        },
                    }
                });

            *measured = ui.window_size();
        });

    if !*open {
        *capturing = None;
        *ui_scale_draft = None;
    }
    if let Some(preset) = draw_keybind_preset_dialog(ui, resetting_keybinds) {
        settings.keybindings = preset.bindings();
    }

    SettingsWindowOutput {
        object_tree_changed,
        reload_profile: draw_profile_reload_dialog(ui, pending_profile),
    }
}

fn draw_heading(ui: &Ui, label: &str, scale: f32) {
    let _font = ui.push_font_with_size(None, ui.clone_style().font_size_base() * scale);
    ui.text(label);
}

fn draw_section_heading(ui: &Ui, label: &str) { draw_heading(ui, label, SETTINGS_SECTION_SCALE); }

fn draw_git_settings(ui: &Ui, settings: &mut Settings) {
    ui.checkbox("Enable Git map integration", &mut settings.git_enabled);
    ui.set_next_item_width(180.0 * dpi(ui));
    ui.slider("Blame history depth", 1, 10_000, &mut settings.blame_depth);
    ui.text_disabled("Tile history follows the first parent of HEAD and does not follow renames.");
}

fn draw_general_settings(ui: &Ui, settings: &mut Settings, ui_scale_draft: &mut Option<u32>) {
    draw_section_heading(ui, "External editor");
    ui.text("Command");
    ui.set_next_item_width(-1.0);
    ui.input_text("##preferred-editor", &mut settings.preferred_editor)
        .build();
    ui.text_disabled("Placeholders: {file}, {line}, {column}");

    ui.separator();
    draw_section_heading(ui, "Saving");
    ui.checkbox("Sanitize variables", &mut settings.sanitize_vars_on_save);
    ui.set_item_tooltip("Leave out variables whose value matches the type's default when writing the map file");
    if ui.button("Show backups") {
        show_backups();
    }

    ui.separator();
    draw_section_heading(ui, "Windows");
    ui.checkbox("Focus windows on hover", &mut settings.focus_windows_on_hover);

    ui.separator();
    draw_section_heading(ui, "Updates");
    ui.checkbox("Check for new releases on startup", &mut settings.check_for_updates);

    ui.separator();
    draw_section_heading(ui, "Appearance");
    let os_scale_percent = os_scale_percent(ui);
    ui.text(format!("UI scale (%) (OS: {os_scale_percent}%)"));
    let active_percent = settings.ui_scale_override_percent.unwrap_or(os_scale_percent);
    let draft = ui_scale_draft.get_or_insert(active_percent);
    let spacing = ui.clone_style().item_spacing()[0];
    let buttons_width = button_width(ui, "Apply") + spacing + button_width(ui, "Reset");
    let available = ui.content_region_avail()[0];
    ui.set_next_item_width((260.0 * dpi(ui)).min((available - buttons_width - spacing).max(1.0)));
    ui.slider(
        "##ui-scale-percent",
        *UI_SCALE_PERCENT.start(),
        *UI_SCALE_PERCENT.end(),
        draft,
    );
    ui.same_line();
    let can_apply =
        *draft != active_percent || (settings.ui_scale_override_percent.is_some() && *draft == os_scale_percent);
    {
        let _disabled = ui.begin_disabled_with_cond(!can_apply);
        if ui.button("Apply") {
            settings.ui_scale_override_percent = (*draft != os_scale_percent).then_some(*draft);
        }
    }
    ui.same_line();
    let can_reset = settings.ui_scale_override_percent.is_some() || *draft != os_scale_percent;
    let _disabled = ui.begin_disabled_with_cond(!can_reset);
    if ui.button("Reset") {
        settings.ui_scale_override_percent = None;
        *draft = os_scale_percent;
    }
    ui.set_item_tooltip(format!("Use the OS scale ({os_scale_percent}%)"));
}

fn os_scale_percent(ui: &Ui) -> u32 {
    let scale = ui
        .main_viewport()
        .dpi_scale()
        .max(ui.io().display_framebuffer_scale()[0]);
    if scale.is_finite() && scale > 0.0 {
        (scale * 100.0).round().clamp(50.0, 500.0) as u32
    } else {
        100
    }
}

fn show_backups() {
    let result = backup_dir().and_then(|dir| {
        std::fs::create_dir_all(&dir)?;
        editor::process::open_directory(&dir)
    });
    if let Err(error) = result {
        log::error!("could not open the backups folder: {error}");
    }
}

fn draw_viewport_settings(ui: &Ui, session: &mut Session, settings: &mut Settings) {
    draw_section_heading(ui, "Areas");
    ui.checkbox("Show areas", &mut session.options.show_areas);
    ui.checkbox("Show area outlines", &mut session.options.show_area_outlines);

    ui.separator();
    draw_section_heading(ui, "Lighting");
    ui.checkbox("Show lighting", &mut session.options.show_lighting);
    ui.set_next_item_width(200.0 * dpi(ui));
    ui.slider(
        "Minimum brightness (%)",
        0,
        100,
        &mut session.options.minimum_light_brightness_percent,
    );
    ui.set_item_tooltip("0% keeps original lighting, 100% removes darkness.");

    ui.separator();
    draw_section_heading(ui, "Feedback");
    ui.checkbox("Tile placement flash", &mut settings.tile_place_flash);
    ui.checkbox("Selection guide lines", &mut settings.selection_guide_line);

    ui.separator();
    draw_section_heading(ui, "Grid");
    ui.checkbox("Tile grid overlay", &mut settings.show_tile_grid);
    ui.set_next_item_width(120.0 * dpi(ui));
    drag_min_pixels(
        ui,
        "Hide tile grid below (px per tile)",
        &mut settings.tile_grid_min_pixels,
    );
    ui.checkbox("Tile grid axis", &mut settings.show_tile_grid_axis);

    ui.checkbox("Pixel grid on selected tile", &mut settings.show_selected_pixel_grid);
    ui.set_next_item_width(120.0 * dpi(ui));
    drag_min_pixels(
        ui,
        "Hide pixel grid below (px per world px)",
        &mut settings.selected_pixel_grid_min_pixels,
    );
    ui.checkbox("Pixel grid axis", &mut settings.show_pixel_grid_axis);

    ui.separator();
    draw_section_heading(ui, "Selection");
    ui.text("Highlight style");
    for (index, highlight) in SelectionHighlight::ALL.into_iter().enumerate() {
        if index > 0 {
            ui.same_line();
        }
        if ui.radio_button(highlight.label(), settings.selection_highlight == highlight) {
            settings.selection_highlight = highlight;
        }
    }
}

fn draw_compiler_settings(
    ui: &Ui, session: &Session, settings: &mut Settings, loading: bool, pending_profile: &mut Option<ProfileReload>,
) {
    draw_section_heading(ui, "Codebase");
    draw_profile_setting(ui, session, settings, loading, pending_profile);

    ui.separator();
    draw_optimization_settings(ui, session, settings);

    ui.separator();
    draw_section_heading(ui, "On next load");
    ui.checkbox("Run DM appearance baking", &mut settings.bake_enabled);

    ui.separator();
    draw_section_heading(ui, "Diagnostics");

    let retained = session.diagnostics.bake.iter().map(|entry| entry.count).sum::<usize>();
    ui.text(format!("{retained} atoms retained their static appearance"));
    if retained == 0 {
        return;
    }

    let Some(_tree) = ui.tree_node("Bake diagnostics") else {
        return;
    };

    for diagnostic in &session.diagnostics.bake {
        let fault = &diagnostic.fault;
        let file = session
            .state
            .environment
            .as_ref()
            .and_then(|environment| environment.bake_file(fault.location.file));
        ui.text_wrapped(format!(
            "{} atoms: {:?} at {}",
            diagnostic.count,
            fault.kind,
            fault.location.display(file)
        ));
    }
}

fn draw_optimization_settings(ui: &Ui, session: &Session, settings: &mut Settings) {
    draw_section_heading(ui, "Optimization");
    ui.checkbox("Enable compiler optimizations", &mut settings.optimizations_enabled);

    let Some(environment) = session.state.environment.as_deref() else {
        ui.text_disabled("Pass timings are available after loading a codebase.");

        return;
    };
    let timings = environment.optimization_timings;
    let samples = timings.samples();
    if samples == 0 {
        ui.text_disabled("No pass timing data for this codebase.");

        return;
    }

    let run_label = if samples == 1 { "run" } else { "runs" };
    ui.text_disabled(format!(
        "Last load: average pass time across {samples} compiler {run_label}"
    ));
    ui.table("compiler-optimization-timings")
        .flags(TableFlags::BORDERS_INNER_V | TableFlags::ROW_BG)
        .sizing_policy(TableSizingPolicy::StretchProp)
        .column("Pass")
        .weight(1.0)
        .done()
        .column("Average")
        .width(100.0)
        .done()
        .build(|ui| {
            for pass in ir::opt::OptimizationPass::ALL {
                if !environment.bake_options.optimizations_enabled && pass != ir::opt::OptimizationPass::SimplifyPhis {
                    continue;
                }
                ui.table_next_row();
                ui.table_next_column();
                ui.text(pass.label());
                ui.table_next_column();
                ui.text(format_pass_time(timings.average(pass)));
            }

            ui.table_next_row();
            ui.table_next_column();
            ui.text("Total");
            ui.table_next_column();
            ui.text(format_pass_time(timings.average_total()));
        });
}

fn format_pass_time(duration: std::time::Duration) -> String { format!("{:.3} ms", duration.as_secs_f64() * 1_000.0) }

fn draw_profile_setting(
    ui: &Ui, session: &Session, settings: &Settings, loading: bool, pending_profile: &mut Option<ProfileReload>,
) {
    let Some(environment) = session.state.environment.as_deref() else {
        ui.text_disabled("No codebase loaded");

        return;
    };
    if !settings.bake_enabled || !environment.bake_options.enabled {
        ui.text_disabled("DM appearance baking is disabled");

        return;
    }

    let forced = environment.bake_options.forced_profile;
    ui.text("Forced bundled profile");
    {
        let _disabled = ui.begin_disabled_with_cond(loading);
        let preview = forced
            .map(BundledProfile::label)
            .unwrap_or("None - use codebase profile");
        ui.set_next_item_width(-1.0);
        if let Some(combo) = ui.begin_combo("##forced-bundled-profile", preview) {
            if ui
                .selectable_config("None - use codebase profile")
                .selected(forced.is_none())
                .build()
                && forced.is_some()
            {
                *pending_profile = Some(ProfileReload::Force(None));
            }
            for profile in BundledProfile::ALL {
                if ui
                    .selectable_config(profile.label())
                    .selected(forced == Some(profile))
                    .build()
                    && forced != Some(profile)
                {
                    *pending_profile = Some(ProfileReload::Force(Some(profile)));
                }
            }
            combo.end();
        }
    }
    if loading {
        ui.text_disabled("Available after the current load finishes");
    }

    ui.text("Profile");
    if forced.is_some() {
        ui.text_disabled("Codebase profile selection is disabled while a bundled profile is forced");

        return;
    }
    let Some(profiles) = environment.profiles.as_ref() else {
        if session.diagnostics.profile.is_some() {
            ui.text_disabled("Profile declarations are invalid");
        } else {
            ui.text_disabled("This codebase defines no profiles");
        }

        return;
    };

    let _disabled = ui.begin_disabled_with_cond(loading);
    ui.set_next_item_width(-1.0);
    if let Some(combo) = ui.begin_combo("##codebase-profile", &profiles.active) {
        for profile in &profiles.available {
            let label = if *profile == profiles.default {
                format!("{profile} (default)")
            } else {
                profile.clone()
            };
            if ui
                .selectable_config(&label)
                .selected(*profile == profiles.active)
                .build()
                && *profile != profiles.active
            {
                *pending_profile = Some(ProfileReload::Select(profile.clone()));
            }
        }
        combo.end();
    }
    if loading {
        ui.text_disabled("Available after the current load finishes");
    }
}

fn draw_profile_reload_dialog(ui: &Ui, pending_profile: &mut Option<ProfileReload>) -> Option<ProfileReload> {
    let message = match pending_profile.as_ref()? {
        ProfileReload::Select(profile) => format!("Reload the codebase with {profile}?"),
        ProfileReload::Force(Some(profile)) => {
            format!("Reload the codebase with the bundled {} profile?", profile.label())
        },
        ProfileReload::Force(None) => String::from("Stop forcing a bundled profile and reload the codebase?"),
    };
    if !ui.is_popup_open(PROFILE_RELOAD_POPUP) {
        ui.open_popup(PROFILE_RELOAD_POPUP);
    }
    let flags = WindowFlags::ALWAYS_AUTO_RESIZE
        | WindowFlags::NO_RESIZE
        | WindowFlags::NO_MOVE
        | WindowFlags::NO_COLLAPSE
        | WindowFlags::NO_SAVED_SETTINGS
        | WindowFlags::NO_DOCKING;
    let _modal = ui.begin_modal_popup_config(PROFILE_RELOAD_POPUP).flags(flags).begin()?;

    ui.text_wrapped(message);
    ui.text("Open maps and unsaved changes will be preserved.");
    ui.separator();
    if ui.button("Cancel") || ui.is_key_pressed(Key::Escape) {
        *pending_profile = None;
        ui.close_current_popup();

        return None;
    }
    ui.same_line();
    if ui.button("Reload") {
        let request = pending_profile.take();
        ui.close_current_popup();

        return request;
    }

    None
}

fn drag_min_pixels(ui: &Ui, label: &str, value: &mut u32) {
    let mut pixels = *value as i32;
    if ui
        .drag_int_config(label)
        .range(1, 128)
        .flags(DragFlags::ALWAYS_CLAMP)
        .build(ui, &mut pixels)
    {
        *value = pixels.max(1) as u32;
    }
}

fn draw_object_tree_settings(ui: &Ui, settings: &mut Settings) -> bool {
    draw_section_heading(ui, "Search fields");
    let mut changed = draw_object_tree_search_settings(ui, &mut settings.object_tree_search);

    ui.separator();
    draw_section_heading(ui, "Type filters");
    changed |= draw_object_tree_filter_settings(ui, &mut settings.object_tree_filter);

    ui.separator();
    draw_section_heading(ui, "Appearance");
    ui.checkbox("Line indicators", &mut settings.object_tree_line_indicators);

    changed
}

pub(super) fn draw_object_tree_search_settings(ui: &Ui, options: &mut ObjectTreeSearchOptions) -> bool {
    let mut changed = false;
    let disable_type_paths = options.type_paths && !options.names;
    {
        let _disabled = ui.begin_disabled_with_cond(disable_type_paths);
        changed |= ui.checkbox("Type paths", &mut options.type_paths);
    }

    let disable_names = options.names && !options.type_paths;
    {
        let _disabled = ui.begin_disabled_with_cond(disable_names);
        changed |= ui.checkbox("Names (atom/name)", &mut options.names);
    }

    changed
}

pub(super) fn draw_object_tree_filter_settings(ui: &Ui, options: &mut ObjectTreeFilterOptions) -> bool {
    let mut changed = false;
    changed |= ui.checkbox("Filter /atom", &mut options.atom);
    changed |= ui.checkbox("Filter /movable", &mut options.movable);
    changed |= ui.checkbox("Filter /obj", &mut options.obj);
    changed |= ui.checkbox("Filter /turf", &mut options.turf);

    ui.separator();
    changed |= ui.checkbox("Filter custom type path and subtypes", &mut options.custom_enabled);
    let _disabled = ui.begin_disabled_with_cond(!options.custom_enabled);
    ui.set_next_item_width(ui.content_region_avail()[0].clamp(1.0, 360.0));
    changed |= ui
        .input_text("##object-tree-custom-type-filter", &mut options.custom_type_path)
        .hint("/path/to/type")
        .build();

    changed
}

fn draw_keybinding_settings(
    ui: &Ui, capturing: &mut Option<KeybindAction>, resetting_keybinds: &mut bool, settings: &mut Settings,
) {
    if ui.button("Reset keybindings") {
        *resetting_keybinds = true;
        *capturing = None;
    }

    for (index, &(group, actions)) in SETTINGS_KEYBINDING_GROUPS.iter().enumerate() {
        ui.separator();
        draw_section_heading(ui, group);
        ui.table(format!("settings-keybindings-{index}"))
            .flags(TableFlags::BORDERS_INNER_V | TableFlags::ROW_BG)
            .sizing_policy(TableSizingPolicy::StretchProp)
            .column("Action")
            .weight(1.0)
            .done()
            .column("Binding")
            .width(170.0)
            .done()
            .build(|ui| {
                for &action in actions {
                    ui.table_next_row();
                    ui.table_next_column();
                    ui.align_text_to_frame_padding();
                    ui.text(action.label());
                    ui.table_next_column();

                    let binding = settings.keybindings.get(action).label(ui);
                    let visible = if *capturing == Some(action) {
                        "Press a key..."
                    } else {
                        binding.as_str()
                    };
                    let width = ui.content_region_avail()[0].max(1.0);
                    if ui.button_with_size(format!("{visible}##keybind-{}", action.id()), [width, 0.0]) {
                        *capturing = (*capturing != Some(action)).then_some(action);
                    }
                }
            });
    }
}

fn finish_keybind_capture(ui: &Ui, capturing: &mut Option<KeybindAction>, keybindings: &mut KeyBindings) {
    let Some(action) = *capturing else {
        return;
    };
    if ui.is_key_pressed_with_repeat(Key::Escape, false) {
        *capturing = None;

        return;
    }
    let modifiers = if action.is_held() { MODIFIER_KEYS } else { &[] };
    let Some(key) = modifiers
        .iter()
        .chain(BINDABLE_KEYS)
        .copied()
        .find(|key| ui.is_key_pressed_with_repeat(*key, false))
    else {
        return;
    };

    keybindings.rebind(action, KeyBinding::from_input(ui, key));
    *capturing = None;
}

#[cfg(test)]
mod tests {
    use dear_imgui_rs::MouseButton;

    use super::{super::IMGUI_CONTEXT, *};

    #[test]
    fn ui_scale_changes_only_when_apply_is_clicked() {
        let _context = IMGUI_CONTEXT.lock().unwrap();
        let mut context = super::super::fixtures::rectangle_context();
        context.io_mut().set_display_framebuffer_scale([1.5, 1.5]);
        let mut settings = Settings::default();
        let mut draft = None;
        let draw = |context: &mut dear_imgui_rs::Context, settings: &mut Settings, draft: &mut Option<u32>| {
            let ui = context.frame();
            let mut centers = [[0.0; 2]; 2];
            ui.window("ui-scale-settings-test")
                .position([20.0, 20.0], Condition::Always)
                .size([650.0, 500.0], Condition::Always)
                .build(|| {
                    draw_general_settings(ui, settings, draft);
                    let min = ui.item_rect_min();
                    let max = ui.item_rect_max();
                    let y = (min[1] + max[1]) * 0.5;
                    let apply_max = min[0] - ui.clone_style().item_spacing()[0];
                    centers = [
                        [apply_max - button_width(ui, "Apply") * 0.5, y],
                        [(min[0] + max[0]) * 0.5, y],
                    ];
                });
            assert!(context.render_legacy().valid());
            centers
        };
        let click = |context: &mut dear_imgui_rs::Context,
                     settings: &mut Settings,
                     draft: &mut Option<u32>,
                     position: [f32; 2]| {
            context.io_mut().add_mouse_pos_event(position);
            draw(context, settings, draft);
            context.io_mut().add_mouse_button_event(MouseButton::Left, true);
            draw(context, settings, draft);
            context.io_mut().add_mouse_button_event(MouseButton::Left, false);
            draw(context, settings, draft);
        };

        draw(&mut context, &mut settings, &mut draft);
        assert_eq!(draft, Some(150));
        draft = Some(100);
        let [apply, _] = draw(&mut context, &mut settings, &mut draft);
        assert_eq!(settings.ui_scale_override_percent, None);

        click(&mut context, &mut settings, &mut draft, apply);
        assert_eq!(settings.ui_scale_override_percent, Some(100));

        draft = Some(150);
        let [apply, _] = draw(&mut context, &mut settings, &mut draft);
        click(&mut context, &mut settings, &mut draft, apply);
        assert_eq!(settings.ui_scale_override_percent, None);

        draft = Some(120);
        let [apply, _] = draw(&mut context, &mut settings, &mut draft);
        click(&mut context, &mut settings, &mut draft, apply);
        assert_eq!(settings.ui_scale_override_percent, Some(120));

        let [_, reset] = draw(&mut context, &mut settings, &mut draft);
        click(&mut context, &mut settings, &mut draft, reset);
        assert_eq!(settings.ui_scale_override_percent, None);
        assert_eq!(draft, Some(150));
    }

    #[test]
    fn only_held_actions_capture_a_lone_modifier() {
        let _context = IMGUI_CONTEXT.lock().unwrap();
        let mut context = dear_imgui_rs::Context::create();
        context.font_atlas().try_claim_legacy_renderer().unwrap().build();
        context.io_mut().set_display_size([800.0, 600.0]);
        context.io_mut().set_delta_time(1.0 / 60.0);
        let mut keybindings = KeyBindings::default();

        for (action, captured) in [(KeybindAction::Save, false), (KeybindAction::ToolAlternate, true)] {
            let mut capturing = Some(action);
            let before = keybindings.get(action);
            context.io_mut().add_key_event(Key::ModShift, true);
            let ui = context.frame();
            finish_keybind_capture(ui, &mut capturing, &mut keybindings);
            let label = keybindings.get(action).label(ui);
            assert!(context.render_legacy().valid());
            context.io_mut().add_key_event(Key::ModShift, false);
            context.frame();
            assert!(context.render_legacy().valid());

            assert_eq!(capturing.is_none(), captured, "{action:?}");
            if captured {
                assert_eq!(keybindings.get(action), KeyBinding::new(Key::ModShift));
                assert_eq!(label, "Shift");
            } else {
                assert_eq!(keybindings.get(action), before);
            }
        }
    }

    #[test]
    fn settings_categories_have_a_stable_order_and_default() {
        assert_eq!(SettingsCategory::default(), SettingsCategory::General);
        assert_eq!(
            SettingsCategory::ALL.map(SettingsCategory::label),
            ["General", "Viewport", "Compiler", "Git", "Object Tree", "Keybindings"]
        );
    }

    #[test]
    fn settings_keybinding_groups_include_every_action_once() {
        let grouped = SETTINGS_KEYBINDING_GROUPS
            .iter()
            .flat_map(|(_, actions)| actions.iter().copied())
            .collect::<Vec<_>>();

        assert_eq!(grouped.len(), KeybindAction::ALL.len());
        for action in KeybindAction::ALL {
            assert_eq!(grouped.iter().filter(|candidate| **candidate == action).count(), 1);
        }
    }

    #[test]
    fn every_settings_category_renders_in_the_two_pane_window() {
        let _context = IMGUI_CONTEXT.lock().unwrap();
        for mut category in SettingsCategory::ALL {
            let mut context = dear_imgui_rs::Context::create();
            context
                .font_atlas()
                .try_claim_legacy_renderer()
                .expect("legacy renderer font atlas should be available")
                .build();
            context.io_mut().set_display_size([1280.0, 720.0]);
            context.io_mut().set_delta_time(1.0 / 60.0);
            let ui = context.frame();
            let window = WindowKey::new(format!("settings-test-{}", category.label()), "Settings")
                .expect("valid settings window key");
            let mut open = true;
            let mut capturing = None;
            let mut resetting_keybinds = false;
            let mut measured = SETTINGS_WINDOW_SIZE;
            let mut pending_profile = None;
            let mut ui_scale_draft = None;
            let mut session = Session::new();
            let mut settings = Settings::default();

            draw_settings_window(
                ui,
                &window,
                SettingsWindowState {
                    open: &mut open,
                    category: &mut category,
                    capturing: &mut capturing,
                    resetting_keybinds: &mut resetting_keybinds,
                    measured: &mut measured,
                    pending_profile: &mut pending_profile,
                    ui_scale_draft: &mut ui_scale_draft,
                },
                &mut session,
                &mut settings,
                false,
            );

            assert!(context.render_legacy().valid());
        }
    }

    #[test]
    fn compiler_settings_render_native_and_forced_profile_states() {
        let _context = IMGUI_CONTEXT.lock().unwrap();
        for forced_profile in [None, Some(BundledProfile::Tgstation)] {
            let mut context = dear_imgui_rs::Context::create();
            context
                .font_atlas()
                .try_claim_legacy_renderer()
                .expect("legacy renderer font atlas should be available")
                .build();
            context.io_mut().set_display_size([1280.0, 720.0]);
            context.io_mut().set_delta_time(1.0 / 60.0);
            let ui = context.frame();
            let window = WindowKey::new(format!("settings-profile-test-{forced_profile:?}"), "Settings")
                .expect("valid settings window key");
            let mut open = true;
            let mut category = SettingsCategory::Compiler;
            let mut capturing = None;
            let mut resetting_keybinds = false;
            let mut measured = SETTINGS_WINDOW_SIZE;
            let mut pending_profile = None;
            let mut ui_scale_draft = None;
            let mut session = Session::new();
            let mut environment = editor::Environment::new("station.dme", objtree::ObjectTree::new());
            environment.bake_options.forced_profile = forced_profile;
            environment.profiles = Some(editor::Profiles {
                available: vec![
                    String::from("/datum/demir/tgstation"),
                    String::from("/datum/demir/tgstation/debug"),
                ],
                default: String::from("/datum/demir/tgstation"),
                active: String::from("/datum/demir/tgstation/debug"),
            });
            session.state.environment = Some(std::sync::Arc::new(environment));
            let mut settings = Settings::default();

            let output = draw_settings_window(
                ui,
                &window,
                SettingsWindowState {
                    open: &mut open,
                    category: &mut category,
                    capturing: &mut capturing,
                    resetting_keybinds: &mut resetting_keybinds,
                    measured: &mut measured,
                    pending_profile: &mut pending_profile,
                    ui_scale_draft: &mut ui_scale_draft,
                },
                &mut session,
                &mut settings,
                false,
            );

            assert_eq!(output, SettingsWindowOutput::default());
            assert!(context.render_legacy().valid());
        }
    }
}
