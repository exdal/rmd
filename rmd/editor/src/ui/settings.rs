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

use crate::{
    session::Session,
    settings::{
        BINDABLE_KEYS,
        KeyBinding,
        KeyBindings,
        KeybindAction,
        ObjectTreeFilterOptions,
        ObjectTreeSearchOptions,
        SelectionHighlight,
        Settings,
    },
};

const SETTINGS_WINDOW_SIZE: [f32; 2] = [760.0, 560.0];
const SETTINGS_WINDOW_MIN_SIZE: [f32; 2] = [620.0, 420.0];
const SETTINGS_CATEGORY_WIDTH: f32 = 160.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum SettingsCategory {
    #[default]
    General,
    Viewport,
    ObjectTree,
    Keybindings,
}

struct SettingsWindowState<'a> {
    open: &'a mut bool,
    category: &'a mut SettingsCategory,
    capturing: &'a mut Option<KeybindAction>,
    measured: &'a mut [f32; 2],
}

impl SettingsCategory {
    const ALL: [Self; 4] = [Self::General, Self::Viewport, Self::ObjectTree, Self::Keybindings];

    const fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Viewport => "Viewport",
            Self::ObjectTree => "Object Tree",
            Self::Keybindings => "Keybindings",
        }
    }
}

const SETTINGS_KEYBINDING_GROUPS: &[(&str, &[KeybindAction])] = &[
    ("Application", &[KeybindAction::Save]),
    (
        "Editing",
        &[
            KeybindAction::Undo,
            KeybindAction::Redo,
            KeybindAction::Copy,
            KeybindAction::Paste,
        ],
    ),
    (
        "Viewport",
        &[
            KeybindAction::ShowAreas,
            KeybindAction::ShowAreaOutlines,
            KeybindAction::ShowTileGrid,
            KeybindAction::ShowPixelGrid,
            KeybindAction::LevelUp,
            KeybindAction::LevelDown,
            KeybindAction::Refit,
        ],
    ),
    (
        "Tools",
        &[
            KeybindAction::PlaceTool,
            KeybindAction::SelectTool,
            KeybindAction::BlockSelectTool,
            KeybindAction::DeleteTool,
            KeybindAction::FillTool,
            KeybindAction::Rotate,
        ],
    ),
    ("Recent", &KeybindAction::RECENT),
];

pub(super) struct SettingsWindow {
    window: WindowKey,
    open: bool,
    category: SettingsCategory,
    capturing: Option<KeybindAction>,
    measured: [f32; 2],
}

impl SettingsWindow {
    pub(super) fn new() -> Result<Self, WindowKeyError> {
        Ok(Self {
            window: WindowKey::new("settings-v2", "Settings")?,
            open: false,
            category: SettingsCategory::default(),
            capturing: None,
            measured: SETTINGS_WINDOW_SIZE,
        })
    }

    pub(super) fn open(&mut self) { self.open = true; }

    pub(super) const fn is_capturing_keybind(&self) -> bool { self.capturing.is_some() }

    pub(super) fn draw(&mut self, ui: &Ui, session: &mut Session, settings: &mut Settings) -> bool {
        draw_settings_window(
            ui,
            &self.window,
            SettingsWindowState {
                open: &mut self.open,
                category: &mut self.category,
                capturing: &mut self.capturing,
                measured: &mut self.measured,
            },
            session,
            settings,
        )
    }

    pub(super) fn finish_keybind_capture(&mut self, ui: &Ui, keybindings: &mut KeyBindings) {
        finish_keybind_capture(ui, &mut self.capturing, keybindings);
    }
}

fn draw_settings_window(
    ui: &Ui, window: &WindowKey, state: SettingsWindowState<'_>, session: &mut Session, settings: &mut Settings,
) -> bool {
    let SettingsWindowState {
        open,
        category,
        capturing,
        measured,
    } = state;
    if !*open {
        *capturing = None;

        return false;
    }

    let center = ui.main_viewport().work_center();
    let position = [center[0] - measured[0] / 2.0, center[1] - measured[1] / 2.0];
    let flags = WindowFlags::NO_COLLAPSE | WindowFlags::NO_DOCKING;
    let mut object_tree_changed = false;
    ui.window(window)
        .opened(open)
        .position(position, Condition::Appearing)
        .size(SETTINGS_WINDOW_SIZE, Condition::FirstUseEver)
        .size_constraints(SETTINGS_WINDOW_MIN_SIZE, [f32::MAX, f32::MAX])
        .flags(flags)
        .build(|| {
            if settings.focus_windows_on_hover {
                super::focus_window_on_hover(ui);
            }
            let content_height = ui.content_region_avail()[1].max(1.0);
            ui.child_window("settings-categories")
                .size([SETTINGS_CATEGORY_WIDTH, content_height])
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
                    ui.text(category.label());
                    ui.separator();

                    match category {
                        SettingsCategory::General => draw_general_settings(ui, settings),
                        SettingsCategory::Viewport => draw_viewport_settings(ui, session, settings),
                        SettingsCategory::ObjectTree => {
                            object_tree_changed |= draw_object_tree_settings(ui, settings);
                        },
                        SettingsCategory::Keybindings => draw_keybinding_settings(ui, capturing, settings),
                    }
                });

            *measured = ui.window_size();
        });

    if !*open {
        *capturing = None;
    }

    object_tree_changed
}

fn draw_general_settings(ui: &Ui, settings: &mut Settings) {
    ui.text("External editor");
    ui.text("Command");
    ui.set_next_item_width(-1.0);
    ui.input_text("##preferred-editor", &mut settings.preferred_editor)
        .build();
    ui.text_disabled("Placeholders: {file}, {line}, {column}");

    ui.separator();
    ui.text("Windows");
    ui.checkbox("Focus windows on hover", &mut settings.focus_windows_on_hover);
}

fn draw_viewport_settings(ui: &Ui, session: &mut Session, settings: &mut Settings) {
    ui.text("Areas");
    ui.checkbox("Show areas", &mut session.options.show_areas);
    ui.checkbox("Show area outlines", &mut session.options.show_area_outlines);

    ui.separator();
    ui.text("Feedback");
    ui.checkbox("Tile placement flash", &mut settings.tile_place_flash);
    ui.checkbox("Selection guide line", &mut settings.selection_guide_line);

    ui.separator();
    ui.text("Grid");
    ui.checkbox("Tile grid overlay", &mut settings.show_tile_grid);
    ui.set_next_item_width(120.0);
    drag_min_pixels(
        ui,
        "Hide tile grid below (px per tile)",
        &mut settings.tile_grid_min_pixels,
    );
    ui.checkbox("Tile grid axis", &mut settings.show_tile_grid_axis);

    ui.checkbox("Pixel grid on selected tile", &mut settings.show_selected_pixel_grid);
    ui.set_next_item_width(120.0);
    drag_min_pixels(
        ui,
        "Hide pixel grid below (px per world px)",
        &mut settings.selected_pixel_grid_min_pixels,
    );
    ui.checkbox("Pixel grid axis", &mut settings.show_pixel_grid_axis);

    ui.separator();
    ui.text("Selection");
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
    ui.text("Search fields");
    let mut changed = draw_object_tree_search_settings(ui, &mut settings.object_tree_search);

    ui.separator();
    ui.text("Type filters");
    changed |= draw_object_tree_filter_settings(ui, &mut settings.object_tree_filter);

    ui.separator();
    ui.text("Appearance");
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

fn draw_keybinding_settings(ui: &Ui, capturing: &mut Option<KeybindAction>, settings: &mut Settings) {
    if ui.button("Reset keybindings") {
        settings.keybindings = KeyBindings::default();
        *capturing = None;
    }

    for (index, &(group, actions)) in SETTINGS_KEYBINDING_GROUPS.iter().enumerate() {
        ui.separator();
        ui.text(group);
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
    let Some(key) = BINDABLE_KEYS
        .iter()
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
    use super::{super::IMGUI_CONTEXT, *};

    #[test]
    fn settings_categories_have_a_stable_order_and_default() {
        assert_eq!(SettingsCategory::default(), SettingsCategory::General);
        assert_eq!(
            SettingsCategory::ALL.map(SettingsCategory::label),
            ["General", "Viewport", "Object Tree", "Keybindings"]
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
            let mut measured = SETTINGS_WINDOW_SIZE;
            let mut session = Session::new();
            let mut settings = Settings::default();

            draw_settings_window(
                ui,
                &window,
                SettingsWindowState {
                    open: &mut open,
                    category: &mut category,
                    capturing: &mut capturing,
                    measured: &mut measured,
                },
                &mut session,
                &mut settings,
            );

            assert!(context.render_legacy().valid());
        }
    }
}
