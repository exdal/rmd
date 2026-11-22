use dear_imgui_rs::{Key, TableFlags, TableSizingPolicy, Ui};
use ui::keybind::{BINDABLE_KEYS, KeyBinding, MODIFIER_KEYS};

use crate::settings::{
    Settings,
    keybind::{KeyBindings, KeybindAction},
};

use super::draw_section_heading;

pub(super) const SETTINGS_KEYBINDING_GROUPS: &[(&str, &[KeybindAction])] = &[
    (
        "Application",
        &[
            KeybindAction::Save,
            KeybindAction::SaveAll,
            KeybindAction::CloseMap,
            KeybindAction::ScreenshotMap,
            KeybindAction::ScreenshotSelection,
            KeybindAction::CopyMapScreenshot,
            KeybindAction::CopySelectionScreenshot,
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
            KeybindAction::CommandPalette,
            KeybindAction::CommandPaletteActions,
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
            KeybindAction::PickTool,
            KeybindAction::MoveTool,
            KeybindAction::NodeTool,
            KeybindAction::SelectTool,
            KeybindAction::DeleteTool,
            KeybindAction::ReplaceTool,
            KeybindAction::FillTool,
            KeybindAction::RectFillTool,
            KeybindAction::CommentTool,
            KeybindAction::Rotate,
            KeybindAction::ToolAlternate,
            KeybindAction::PrefabVariants,
        ],
    ),
    ("Recent", &KeybindAction::RECENT),
];

pub(super) fn draw_keybinding_settings(
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

pub(super) fn finish_keybind_capture(ui: &Ui, capturing: &mut Option<KeybindAction>, keybindings: &mut KeyBindings) {
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
