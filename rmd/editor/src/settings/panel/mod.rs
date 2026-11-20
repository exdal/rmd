use dear_imgui_rs::{Condition, Ui, WindowFlags, WindowKey, WindowKeyError};
use ui::{
    theme::Themes,
    widgets::{center_next_window, dpi, focus_window_on_hover},
};

use crate::{
    app::process,
    frontend::{ProfileReload, dialog::draw_keybind_preset_dialog},
    session::Session,
    settings::{
        ObjectTreeFilterOptions, ObjectTreeSearchOptions, Settings,
        keybind::{KeyBindings, KeybindAction},
        paths::backup_dir,
    },
};

use self::{
    appearance::{FontPicker, draw_appearance_settings},
    compiler::{draw_compiler_settings, draw_profile_reload_dialog},
    keybinds::{draw_keybinding_settings, finish_keybind_capture},
    theme::ThemeEditor,
    viewport::draw_viewport_settings,
};

mod appearance;
mod compiler;
mod keybinds;
mod theme;
mod viewport;

const SETTINGS_WINDOW_SIZE: [f32; 2] = [760.0, 560.0];
const SETTINGS_WINDOW_MIN_SIZE: [f32; 2] = [620.0, 420.0];
const SETTINGS_CATEGORY_WIDTH: f32 = 160.0;
const SETTINGS_TITLE_SCALE: f32 = 1.4;
const SETTINGS_SECTION_SCALE: f32 = 1.2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum SettingsCategory {
    #[default]
    General,
    Appearance,
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
    pending_profile: &'a mut Option<ProfileReload>,
    ui_scale_draft: &'a mut Option<u32>,
    theme_editor: &'a mut ThemeEditor,
    font_picker: &'a mut FontPicker,
    profiles: &'a [String],
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct SettingsWindowOutput {
    pub object_tree_changed: bool,
    pub reload_profile: Option<ProfileReload>,
}

impl SettingsCategory {
    const ALL: [Self; 7] = [
        Self::General,
        Self::Appearance,
        Self::Viewport,
        Self::Compiler,
        Self::Git,
        Self::ObjectTree,
        Self::Keybindings,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Viewport => "Viewport",
            Self::Compiler => "Compiler",
            Self::Git => "Git",
            Self::ObjectTree => "Object Tree",
            Self::Keybindings => "Keybindings",
        }
    }
}

pub(crate) struct SettingsWindow {
    window: WindowKey,
    open: bool,
    category: SettingsCategory,
    capturing: Option<KeybindAction>,
    resetting_keybinds: bool,
    pending_profile: Option<ProfileReload>,
    ui_scale_draft: Option<u32>,
    theme_editor: ThemeEditor,
    font_picker: FontPicker,
    profiles: Vec<String>,
}

impl SettingsWindow {
    pub(crate) fn new() -> Result<Self, WindowKeyError> {
        Ok(Self {
            window: WindowKey::new("settings-v2", "Settings")?,
            open: false,
            category: SettingsCategory::default(),
            capturing: None,
            resetting_keybinds: false,
            pending_profile: None,
            ui_scale_draft: None,
            theme_editor: ThemeEditor::default(),
            font_picker: FontPicker::default(),
            profiles: Vec::new(),
        })
    }

    pub(crate) fn open(&mut self) {
        self.open = true;
    }

    pub(crate) fn set_profiles(&mut self, profiles: Vec<String>) {
        self.profiles = profiles;
    }

    pub(crate) const fn is_capturing_keybind(&self) -> bool {
        self.capturing.is_some()
    }

    pub(crate) fn draw(
        &mut self, ui: &Ui, session: &mut Session, settings: &mut Settings, themes: &mut Themes, loading: bool,
    ) -> SettingsWindowOutput {
        draw_settings_window(
            ui,
            &self.window,
            SettingsWindowState {
                open: &mut self.open,
                category: &mut self.category,
                capturing: &mut self.capturing,
                resetting_keybinds: &mut self.resetting_keybinds,
                pending_profile: &mut self.pending_profile,
                ui_scale_draft: &mut self.ui_scale_draft,
                theme_editor: &mut self.theme_editor,
                font_picker: &mut self.font_picker,
                profiles: &self.profiles,
            },
            session,
            settings,
            themes,
            loading,
        )
    }

    pub(crate) fn finish_keybind_capture(&mut self, ui: &Ui, keybindings: &mut KeyBindings) {
        finish_keybind_capture(ui, &mut self.capturing, keybindings);
    }
}

fn draw_settings_window(
    ui: &Ui, window: &WindowKey, state: SettingsWindowState<'_>, session: &mut Session, settings: &mut Settings,
    themes: &mut Themes, loading: bool,
) -> SettingsWindowOutput {
    let SettingsWindowState {
        open,
        category,
        capturing,
        resetting_keybinds,
        pending_profile,
        ui_scale_draft,
        theme_editor,
        font_picker,
        profiles,
    } = state;
    if !*open {
        *capturing = None;
        *ui_scale_draft = None;
        font_picker.size_draft = None;

        return SettingsWindowOutput::default();
    }

    center_next_window(ui, Condition::Appearing);
    let flags = WindowFlags::NO_COLLAPSE | WindowFlags::NO_DOCKING;
    let mut object_tree_changed = false;
    let scale = dpi(ui);
    ui.window(window)
        .opened(open)
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
                        SettingsCategory::General => draw_general_settings(ui, settings),
                        SettingsCategory::Appearance => {
                            draw_appearance_settings(ui, settings, themes, ui_scale_draft, theme_editor, font_picker)
                        },
                        SettingsCategory::Viewport => draw_viewport_settings(ui, session, settings),
                        SettingsCategory::Compiler => {
                            draw_compiler_settings(ui, session, settings, loading, profiles, pending_profile)
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
        });

    if !*open {
        *capturing = None;
        *ui_scale_draft = None;
        font_picker.size_draft = None;
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

fn draw_section_heading(ui: &Ui, label: &str) {
    draw_heading(ui, label, SETTINGS_SECTION_SCALE);
}

fn draw_git_settings(ui: &Ui, settings: &mut Settings) {
    ui.checkbox("Enable Git map integration", &mut settings.git_enabled);
    ui.set_next_item_width(180.0 * dpi(ui));
    ui.slider("Blame history depth", 1, 10_000, &mut settings.blame_depth);
    ui.text_disabled("Tile history follows the first parent of HEAD and does not follow renames.");
}

fn draw_general_settings(ui: &Ui, settings: &mut Settings) {
    draw_section_heading(ui, "External editor");
    ui.text("Command");
    ui.set_next_item_width(-1.0);
    ui.input_text("##preferred-editor", &mut settings.preferred_editor)
        .build();
    ui.text_disabled("Placeholders: {file}, {line}, {column}, {root}");
    ui.text_disabled("{root} is the workspace directory containing the loaded .dme file.");

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
}

fn show_backups() {
    let result = backup_dir().and_then(|dir| {
        std::fs::create_dir_all(&dir)?;
        process::open(&dir)
    });
    if let Err(error) = result {
        log::error!("could not open the backups folder: {error}");
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

pub(crate) fn draw_object_tree_search_settings(ui: &Ui, options: &mut ObjectTreeSearchOptions) -> bool {
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

pub(crate) fn draw_object_tree_filter_settings(ui: &Ui, options: &mut ObjectTreeFilterOptions) -> bool {
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

#[cfg(test)]
mod tests;
