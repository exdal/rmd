use std::path::Path;

use dear_imgui_rs::{Ui, WindowFlags};
use ui::widgets::{DialogButton, begin_centered_modal, dialog_buttons};

use super::UiState;
use crate::{
    document::{DocumentId, MapDocument, panel::SaveDialog},
    session::Session,
    settings::keybind::KeybindPreset,
};

pub(crate) const MODAL_FLAGS: WindowFlags = WindowFlags::ALWAYS_AUTO_RESIZE
    .union(WindowFlags::NO_RESIZE)
    .union(WindowFlags::NO_MOVE)
    .union(WindowFlags::NO_COLLAPSE)
    .union(WindowFlags::NO_SAVED_SETTINGS)
    .union(WindowFlags::NO_DOCKING);

pub(crate) const DIALOG_FIELD_WIDTH: f32 = 320.0;

const KEYBIND_PRESET_POPUP: &str = "Choose keybindings##keybind-preset";

pub(crate) const SAVE_ERROR_COLOR: [f32; 4] = [1.0, 0.4, 0.4, 1.0];

const CLOSE_MAP_POPUP: &str = "Unsaved changes##close-map";

const EXIT_POPUP: &str = "Unsaved changes##exit";

pub(crate) fn draw_keybind_preset_dialog(ui: &Ui, open: &mut bool) -> Option<KeybindPreset> {
    if !*open {
        return None;
    }
    if !ui.is_popup_open(KEYBIND_PRESET_POPUP) {
        ui.open_popup(KEYBIND_PRESET_POPUP);
    }

    let _modal = begin_centered_modal(ui, KEYBIND_PRESET_POPUP, MODAL_FLAGS)?;

    ui.text("Which keybinding preset would you prefer?");
    ui.text_disabled("You can customize individual bindings later in Settings.");

    let selected = match dialog_buttons(
        ui,
        &[
            DialogButton::secondary("Use Default"),
            DialogButton::secondary("Use StrongDMM"),
        ],
        true,
    ) {
        Some(0) => Some(KeybindPreset::Default),
        Some(_) => Some(KeybindPreset::StrongDmm),
        None => None,
    };

    if selected.is_some() {
        *open = false;
        ui.close_current_popup();
    }

    selected
}

impl UiState {
    pub(super) fn close_map_view(&mut self, session: &mut Session, id: DocumentId) {
        let was_active = session.state.active() == Some(id);
        if session.close_map(id) && was_active {
            self.close_focus = Some(match session.state.active() {
                Some(id) => super::CloseFocus::Map(id),
                None => {
                    self.show_welcome = true;
                    super::CloseFocus::Welcome
                },
            });
        }
    }

    pub(super) fn draw_exit_confirmation(&mut self, ui: &Ui, session: &Session) -> bool {
        if !self.exit_requested {
            return false;
        }

        let unsaved = session
            .state
            .documents()
            .iter()
            .filter(|document| document.is_dirty())
            .count();
        if unsaved == 0 {
            self.exit_requested = false;

            return true;
        }

        if !ui.is_popup_open(EXIT_POPUP) {
            ui.open_popup(EXIT_POPUP);
        }
        let Some(_modal) = begin_centered_modal(ui, EXIT_POPUP, MODAL_FLAGS) else {
            return false;
        };

        if unsaved == 1 {
            ui.text("A map has unsaved changes.");
        } else {
            ui.text(format!("{unsaved} maps have unsaved changes."));
        }
        ui.text("Exit without saving?");

        let pressed = dialog_buttons(
            ui,
            &[
                DialogButton::cancel("Cancel"),
                DialogButton::destructive("Exit Without Saving", false),
            ],
            true,
        );
        if pressed.is_some() {
            self.exit_requested = false;
            ui.close_current_popup();
        }

        pressed == Some(1)
    }

    pub(crate) fn request_close(&mut self, session: &mut Session, ids: impl IntoIterator<Item = DocumentId>) {
        for id in ids {
            if !session.state.document(id).is_some_and(MapDocument::is_dirty) {
                self.close_map_view(session, id);
            } else if !self.close_queue.contains(&id) {
                self.close_queue.push_back(id);
            }
        }
    }

    pub(crate) fn draw_close_confirmation(&mut self, ui: &Ui, session: &mut Session) {
        if self.save_dialog.as_ref().is_some_and(|dialog| dialog.close_after_save) {
            return;
        }

        while let Some(id) = self.close_queue.front().copied()
            && session.state.document(id).is_none_or(|document| !document.is_dirty())
        {
            self.close_queue.pop_front();
            self.close_map_view(session, id);
        }
        let Some(id) = self.close_queue.front().copied() else {
            return;
        };
        let Some(title) = session.state.document(id).map(MapDocument::title) else {
            return;
        };
        if !ui.is_popup_open(CLOSE_MAP_POPUP) {
            session.set_active_document(id);
            self.close_focus = None;
            ui.set_window_focus(Some(&format!("###viewport-{}", id.get())));
            ui.open_popup(CLOSE_MAP_POPUP);
        }

        let Some(_modal) = begin_centered_modal(ui, CLOSE_MAP_POPUP, MODAL_FLAGS) else {
            return;
        };
        let writable = session
            .state
            .document(id)
            .is_some_and(|document| document.path.is_some() && !document.needs_initial_save());

        ui.text(format!("{title} has unsaved changes."));
        if self.close_queue.len() > 1 {
            ui.text_disabled(format!(
                "{} more map(s) with unsaved changes are waiting to close.",
                self.close_queue.len() - 1
            ));
        }

        let pressed = dialog_buttons(
            ui,
            &[
                DialogButton::cancel("Cancel"),
                DialogButton::destructive("Discard Changes", false),
                DialogButton::primary("Save Map"),
            ],
            true,
        );

        if pressed == Some(0) {
            self.close_queue.clear();
            ui.close_current_popup();
        }

        if pressed == Some(1) {
            self.close_map_view(session, id);
            self.close_queue.pop_front();
            ui.close_current_popup();
        }

        if pressed == Some(2) {
            session.set_active_document(id);
            match session.map_path().map(Path::to_path_buf).filter(|_| writable) {
                Some(path) => {
                    let format = session.map_format().unwrap_or_default();
                    match session.save_map_as(&path, format) {
                        Ok(()) => {
                            self.close_map_view(session, id);
                            self.close_queue.pop_front();
                            ui.close_current_popup();
                        },
                        Err(e) => self.open_error = Some(e.to_string()),
                    }
                },
                None => {
                    self.save_dialog = Some(SaveDialog {
                        document: id,
                        close_after_save: true,
                        path: session
                            .map_path()
                            .map(|path| path.display().to_string())
                            .unwrap_or_default(),
                        format: session.map_format().unwrap_or_default(),
                        error: None,
                    });
                    ui.close_current_popup();
                },
            }
        }
    }
}

#[cfg(test)]
mod tests;
