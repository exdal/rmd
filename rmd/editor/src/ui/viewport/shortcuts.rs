use dear_imgui_rs::{Key, MouseButton, Ui};
use editor::tool::Tool;

use super::ViewFrame;
use crate::{
    session::Session,
    settings::{KeyBinding, KeybindAction, Settings},
    ui::{LAYER_KEYS, UiState, request_level_change},
};

const TOOL_KEYS: [(KeybindAction, Tool); 8] = [
    (KeybindAction::PlaceTool, Tool::Place),
    (KeybindAction::SelectTool, Tool::Select),
    (KeybindAction::NodeTool, Tool::Node),
    (KeybindAction::BlockSelectTool, Tool::BlockSelect),
    (KeybindAction::DeleteTool, Tool::Delete),
    (KeybindAction::ReplaceTool, Tool::Replace),
    (KeybindAction::FillTool, Tool::Fill),
    (KeybindAction::CommentTool, Tool::Comment),
];

/// aka Alternate tool
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) struct MomentaryTool {
    binding: KeyBinding,
    tool: Tool,
    previous: Tool,
    used: bool,
}

fn has_modifiers(ui: &Ui) -> bool {
    let io = ui.io();

    io.key_ctrl() || io.key_shift() || io.key_alt() || io.key_super()
}

pub(super) fn choose_recent_on_key(ui: &Ui, session: &mut Session, settings: &Settings, is_focused: bool) {
    if is_focused
        && !ui.io().want_text_input()
        && let Some(index) = settings.keybindings.pressed_recent(ui)
    {
        session.choose_recent(index);
    }
}

impl UiState {
    pub(super) fn release_momentary_tool(&mut self, ui: &Ui, session: &mut Session, is_hovered: bool) {
        let Some(momentary) = self.momentary_tool.as_mut() else {
            return;
        };

        momentary.used |= is_hovered && ui.is_mouse_clicked(MouseButton::Left);
        if !momentary.binding.is_key_held(ui) {
            if momentary.used && session.tool() == momentary.tool {
                session.set_tool(momentary.previous);
            }

            self.momentary_tool = None;
        }
    }

    pub(super) fn apply_view_shortcuts(
        &mut self, ui: &Ui, session: &mut Session, settings: &mut Settings, refit: &mut bool,
    ) {
        if settings.keybindings.get(KeybindAction::ShowAreas).is_pressed(ui) {
            session.toggle_areas();
        }

        for (action, layer) in LAYER_KEYS {
            if settings.keybindings.get(action).is_pressed(ui) {
                session.toggle_layer(layer);
            }
        }

        if settings.keybindings.get(KeybindAction::ShowAllLayers).is_pressed(ui) {
            session.show_all_types();
        }

        if settings.keybindings.get(KeybindAction::ShowAreaOutlines).is_pressed(ui) {
            session.toggle_area_outlines();
        }

        if settings.keybindings.get(KeybindAction::ShowLighting).is_pressed(ui) {
            session.toggle_lighting();
        }

        if settings.keybindings.get(KeybindAction::ShowTileGrid).is_pressed(ui) {
            settings.show_tile_grid = !settings.show_tile_grid;
        }

        if settings.keybindings.get(KeybindAction::ShowPixelGrid).is_pressed(ui) {
            settings.show_selected_pixel_grid = !settings.show_selected_pixel_grid;
        }

        if settings.keybindings.get(KeybindAction::LevelUp).is_pressed(ui) {
            request_level_change(session, 1, &mut self.new_level_dialog);
        }

        if settings.keybindings.get(KeybindAction::LevelDown).is_pressed(ui) {
            request_level_change(session, -1, &mut self.new_level_dialog);
        }

        if settings.keybindings.get(KeybindAction::Refit).is_pressed(ui) {
            *refit = true;
        }

        for (action, tool) in TOOL_KEYS {
            let binding = settings.keybindings.get(action);
            if binding.is_pressed(ui) && session.tool() != tool {
                let previous = session.tool();
                session.set_tool(tool);
                self.momentary_tool = (session.tool() == tool).then_some(MomentaryTool {
                    binding,
                    tool,
                    previous,
                    used: false,
                });
            }
        }
    }

    pub(super) fn toggle_focus_on_key(
        &mut self, ui: &Ui, session: &mut Session, settings: &Settings, frame: &ViewFrame<'_>,
    ) {
        if frame.is_hovered
            && !ui.io().want_text_input()
            && !has_modifiers(ui)
            && !settings.keybindings.is_any_pressed(ui)
            && ui.is_key_pressed(Key::F)
        {
            session.toggle_focus_at(frame.coord);
            self.placement_stroke = None;
        }
    }
}
