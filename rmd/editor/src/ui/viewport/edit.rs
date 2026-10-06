use dear_imgui_rs::{Key, PopupQueryFlags, Ui};
use editor::tool::{SelectionRotation, Tool};
use render::PickRequest;

use super::{ViewFrame, tools::request_pick};
use crate::{
    session::Session,
    settings::{KeybindAction, Settings},
    ui::{
        PendingPaste,
        UiState,
        centered_paste_min,
        context_menu::{Action as MenuAction, NodeContext, draw_popup as draw_map_menu},
        restore_rectangle_gesture,
    },
};

pub(in crate::ui) const EDIT_KEYS: [(KeybindAction, EditCommand); 5] = [
    (KeybindAction::Copy, EditCommand::Copy),
    (KeybindAction::Cut, EditCommand::Cut),
    (KeybindAction::Delete, EditCommand::Delete),
    (KeybindAction::Paste, EditCommand::Paste),
    (KeybindAction::Deselect, EditCommand::Deselect),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum EditCommand {
    Copy,
    Cut,
    Delete,
    Paste,
    Deselect,
}

impl UiState {
    pub(super) fn apply_edit_command(&mut self, session: &mut Session, frame: &mut ViewFrame<'_>) {
        match self.edit_command.take() {
            Some(EditCommand::Copy) => {
                session.copy_selection(session.selection_mode());
            },
            Some(command @ (EditCommand::Cut | EditCommand::Delete))
                if frame.gestures.paste.is_none() && session.selection().is_some() =>
            {
                frame.gestures.cancel(session, frame.id);
                self.gizmo.cancel();

                if command == EditCommand::Cut {
                    session.cut_selection();
                } else {
                    session.delete_selection();
                }
            },
            Some(EditCommand::Paste) if let Some(block) = session.clipboard() => {
                let (width, height) = (block.width(), block.height());
                restore_rectangle_gesture(session, frame.id, &mut frame.gestures.rectangle_gesture);
                let anchor = frame.coord.or_else(|| {
                    let size = session.map()?.size();

                    frame
                        .camera
                        .screen_to_tile(frame.layout.center(), size, session.options.tile_size, session.z())
                });
                if let (Some(anchor), Some(size)) = (anchor, session.map().map(|map| map.size())) {
                    session.set_tool(Tool::BlockSelect);
                    self.gizmo.cancel();
                    frame.gestures.block_placement = None;
                    frame.gestures.block_selection_anchor = None;
                    frame.gestures.paste = Some(PendingPaste {
                        min: centered_paste_min(width, height, anchor, size),
                        rotation: SelectionRotation::Original,
                    });
                }
            },
            Some(EditCommand::Deselect) => {
                frame.gestures.cancel(session, frame.id);
                self.gizmo.cancel();
                session.select_block(None);
                session.select_instance(None);
            },
            _ => {},
        }
    }

    pub(super) fn cancel_on_escape(
        &mut self, ui: &Ui, session: &mut Session, frame: &mut ViewFrame<'_>, tool: Tool,
    ) -> bool {
        let is_escape = frame.is_focused
            && !ui.io().want_text_input()
            && !self.popup_was_open
            && !ui.is_popup_open_with_flags("", PopupQueryFlags::ANY_POPUP)
            && ui.is_key_pressed(Key::Escape);
        if !is_escape {
            return false;
        }

        let gestures = &mut *frame.gestures;
        if gestures.rectangle_gesture.is_some() {
            restore_rectangle_gesture(session, frame.id, &mut gestures.rectangle_gesture);
            gestures.block_selection_anchor = None;
        } else if gestures.block_placement.is_some() || gestures.paste.is_some() {
            gestures.block_placement = None;
            gestures.paste = None;
        } else if matches!(tool, Tool::BlockSelect | Tool::Fill) {
            session.select_block(None);
        }

        self.gizmo.cancel();
        session.cancel_node_drag();

        true
    }

    pub(super) fn apply_map_menu(
        &mut self, ui: &Ui, session: &mut Session, settings: &Settings, frame: &mut ViewFrame<'_>,
    ) {
        let Some(target) = frame.context.as_mut() else {
            return;
        };

        let Some(action) = draw_map_menu(ui, session, settings, target) else {
            return;
        };

        if matches!(
            action,
            MenuAction::Undo | MenuAction::Redo | MenuAction::Paste(_) | MenuAction::Cut(_) | MenuAction::Delete(_)
        ) {
            self.cancel_view_edits(session, frame);
        }

        let id = frame.id;
        match action {
            MenuAction::Conflict(coords, side) => {
                session.resolve_conflict(id, &coords, side);
            },
            MenuAction::BlameCopy(hash) => {
                self.copy_to_clipboard = Some(hash);
            },
            MenuAction::BlamePin(commit) => {
                session.pin_blame(id, commit);
            },
            MenuAction::BlameRun => {
                session.run_blame(id, settings.blame_depth as usize);
            },
            MenuAction::Restore(coords) => {
                session.restore_diff(id, &coords);
            },
            MenuAction::Undo => {
                session.undo();
            },
            MenuAction::Redo => {
                session.redo();
            },
            MenuAction::Copy(coord) => {
                session.copy_tile(coord);
            },
            MenuAction::Paste(coord) => {
                session.paste_clipboard(coord, SelectionRotation::Original);
            },
            MenuAction::Cut(coord) => {
                session.cut_tile(coord);
            },
            MenuAction::Delete(coord) => {
                session.delete_tile(coord, None);
            },
            MenuAction::Select(instance) => {
                session.select_instance(Some(instance));
                self.reveal_selected_instance(session);
            },
            MenuAction::DeleteAtom(instance) => {
                session.delete_context_instance(instance);
            },
            MenuAction::Reorder(instance, to_top) => {
                session.reorder_instance(instance, to_top);
            },
            MenuAction::Reset(instance) => {
                session.reset_instance_to_default(instance);
            },
            MenuAction::Replace(instance, path) => {
                session.replace_context_instance(instance, path);
            },
            MenuAction::Search(instance, kind) => {
                self.find.open_for(session, id, instance, kind);
            },
            MenuAction::Node(node) => match node {
                NodeContext::Standalone(coord) => {
                    session.delete_standalone_node(coord);
                },
                NodeContext::Connection(connection) => {
                    session.delete_node_connection(&connection);
                },
                NodeContext::Pick(coord, pixel) => {
                    frame.interaction.cursor = Some(pixel);
                    request_pick(frame.interaction, PickRequest::NodeDelete(coord));
                },
            },
            MenuAction::Mirror(transform) => {
                session.transform_selected_block_with_mode(transform, session.selection_mode());
            },
        }
    }
}
