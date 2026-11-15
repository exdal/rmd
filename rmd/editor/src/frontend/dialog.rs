use std::path::Path;

use dear_imgui_rs::{Ui, WindowFlags};
use dmm::Coord;
use ui::widgets::{DialogButton, begin_centered_modal, dialog_buttons};

use super::{OpenRequest, UiState, VisibleMapView, menu::MenuActions};
use crate::{
    coop::panel::{
        CoopDialog, CoopDialogKind, OutOfDateChoice, draw_coop_dialog, draw_out_of_date_dialog,
        notice::{CoopNoticeChoice, draw_coop_notice},
        presence::CoopPresence,
    },
    document::{
        DocumentId, MapDocument,
        panel::{
            GO_TO_POPUP, GoToDialog, NEW_MAP_POPUP, NewMapDialog, RESIZE_MAP_POPUP, ResizeMapDialog, SaveDialog,
            TileFillSearch, draw_delete_level_dialog, draw_go_to_dialog, draw_new_level_dialog, draw_new_map_dialog,
            draw_resize_map_dialog,
        },
    },
    environment::{
        panel::{LoadPopup, draw_load_popup},
        worker::LoadView,
    },
    session::Session,
    settings::{Settings, keybind::KeybindPreset},
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

const RELOAD_CONFLICTS_POPUP: &str = "Load merge conflicts##reload-conflicts";

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

    pub(super) fn draw_reload_conflicts_popup(&mut self, ui: &Ui, session: &Session) -> Option<DocumentId> {
        let id = self.pending_conflict_reload?;
        if !session.state.document(id).is_some_and(MapDocument::is_dirty) {
            self.pending_conflict_reload = None;
            return Some(id);
        }

        if !ui.is_popup_open(RELOAD_CONFLICTS_POPUP) {
            ui.open_popup(RELOAD_CONFLICTS_POPUP);
        }

        let _modal = ui::widgets::begin_centered_modal(
            ui,
            RELOAD_CONFLICTS_POPUP,
            WindowFlags::ALWAYS_AUTO_RESIZE | WindowFlags::NO_SAVED_SETTINGS,
        )?;
        ui.text("Discard unsaved changes and load conflicts from Git?");
        let mut load_conflicts = None;
        let pressed = ui::widgets::dialog_buttons(
            ui,
            &[
                ui::widgets::DialogButton::cancel("Cancel"),
                ui::widgets::DialogButton::destructive("Discard and Load", false),
            ],
            true,
        );
        if pressed == Some(1) {
            load_conflicts = Some(id);
        }

        if pressed.is_some() {
            self.pending_conflict_reload = None;
            ui.close_current_popup();
        }

        load_conflicts
    }

    pub(super) fn draw_new_map(&mut self, ui: &Ui, session: &mut Session, open: bool) -> bool {
        if open {
            self.new_map_dialog = Some(NewMapDialog::default());
            ui.open_popup(NEW_MAP_POPUP);
        }

        let (pick_path, created) = draw_new_map_dialog(ui, session, &mut self.new_map_dialog);
        if created {
            self.show_welcome = false;
            self.request_refit(session.state.active());
        }

        pick_path
    }

    pub(super) fn draw_load_window(&mut self, ui: &Ui, load: Option<&LoadView>) -> LoadPopup {
        let popup = draw_load_popup(
            ui,
            &self.load_window,
            load,
            self.load_notice.as_mut(),
            &self.diagnostics,
        );
        if popup.dismiss {
            if self.load_notice.is_some() {
                self.load_notice = None;
            } else {
                self.diagnostics.open = None;
            }
        }

        popup
    }

    pub(super) fn draw_open_maps(
        &mut self, ui: &Ui, session: &mut Session, settings: &mut Settings, refit: bool,
    ) -> (Vec<VisibleMapView>, Option<usize>) {
        self.follow_coop_peer(ui, session);
        let (map_views, picking, coop) = if session.state.is_empty() {
            (Vec::new(), None, CoopPresence::default())
        } else {
            self.draw_map_views(ui, session, settings, refit)
        };
        self.stop_following_when_moved(session);
        session.coop_cursor(coop.cursor);
        session.coop_view(coop.view);
        session.coop_selection(coop.selection);
        self.edit_command = None;

        (map_views, picking)
    }

    pub(super) fn draw_map_dialogs(&mut self, ui: &Ui, session: &mut Session, menu: &MenuActions) {
        draw_new_level_dialog(ui, session, &mut self.new_level_dialog, &mut self.tile_fill);
        if let Some(state) = &self.delete_level_dialog
            && state.open
        {
            self.cancel_edit_gestures(session, Some(state.document));
        }

        draw_delete_level_dialog(ui, session, &mut self.delete_level_dialog);

        if menu.resize_map
            && let Some(size) = session.map().map(|map| map.size())
        {
            self.cancel_edit_gestures(session, session.state.active());
            let fill = TileFillSearch::new(session, self.tile_fill.as_ref());
            self.resize_map_dialog = Some(ResizeMapDialog::new(size, fill));
            ui.open_popup(RESIZE_MAP_POPUP);
        }

        if draw_resize_map_dialog(ui, session, &mut self.resize_map_dialog, &mut self.tile_fill) {
            self.request_refit(session.state.active());
        }

        if menu.go_to
            && let Some(size) = session.map().map(|map| map.size())
        {
            let (x, y) = self
                .last_go_to
                .map_or((size.x.div_ceil(2), size.y.div_ceil(2)), |last| (last.x, last.y));
            self.go_to_dialog = Some(GoToDialog::new(Coord::new(x, y, session.z())));
            ui.open_popup(GO_TO_POPUP);
        }

        if let Some(coord) = draw_go_to_dialog(ui, session, &mut self.go_to_dialog)
            && let Some(document) = session.state.active()
        {
            self.last_go_to = Some(coord);
            self.go_to_tile(session, document, coord, ui.time());
        }
    }

    pub(super) fn draw_coop_dialogs(
        &mut self, ui: &Ui, session: &mut Session, settings: &mut Settings, loading: bool, kind: Option<CoopDialogKind>,
    ) -> Option<OpenRequest> {
        if let Some(kind) = kind {
            let dialog = CoopDialog::new(kind, settings);
            ui.open_popup(dialog.popup());
            self.coop_dialog = Some(dialog);
        }

        draw_coop_dialog(ui, session, settings, &mut self.coop_dialog);

        match draw_out_of_date_dialog(ui, session) {
            Some((id, OutOfDateChoice::SaveMine)) => {
                session.set_active_document(id);
                self.save_requested = true;
            },
            Some((id, OutOfDateChoice::DiscardMine)) => session.discard_for_coop_map(id),
            Some((_, OutOfDateChoice::Leave)) => session.leave_coop(),
            None => {},
        }

        match draw_coop_notice(ui, session.coop(), loading, &mut self.coop_notice) {
            Some(CoopNoticeChoice::Retry) => {
                match session.retry_coop() {
                    Ok(()) => self.coop_notice.reset(),
                    Err(error) => self.open_error = Some(error),
                }

                None
            },
            Some(CoopNoticeChoice::ReloadAndRetry) => Some(OpenRequest::ReloadCoop),
            Some(CoopNoticeChoice::Dismiss) => {
                session.leave_coop();
                None
            },
            None => None,
        }
    }
}

#[cfg(test)]
mod tests;
