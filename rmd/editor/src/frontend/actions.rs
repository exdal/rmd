use std::mem;

use dear_imgui_rs::Ui;

use super::{SCREENSHOT_KEYS, ScreenshotRequest, StartupPanelFocus, UiState, menu::MenuActions};
use crate::{
    document::panel::{SAVE_MAP_POPUP, SaveDialog},
    search::panel::SimilarMatchKind,
    session::Session,
    settings::{OpenPanels, Panel, Settings, keybind::KeybindAction},
    tool::panel::request_level_change,
    viewport::panel::edit::EDIT_KEYS,
};

impl UiState {
    pub(super) fn apply_window_actions(&mut self, settings: &mut Settings, menu: &MenuActions) {
        settings.show_dm_ui ^= menu.toggle_dm_ui;
        self.reset_layout = menu.reset_layout;
        if menu.reset_layout {
            settings.panels = OpenPanels::default();
        }

        let Some(panel) = menu.toggle_panel else {
            return;
        };

        let open = settings.panels.get_mut(panel);
        *open = !*open;
        if !*open {
            return;
        }

        match panel {
            Panel::ObjectTree => self.select_object_tree = StartupPanelFocus::new(),
            Panel::Git => self.git_panel.focus(),
            Panel::Inspector => self.select_inspector = StartupPanelFocus::new(),
            Panel::Search => self.find.request(),
            Panel::MapIssues => self.issues.request(None),
            Panel::Runtimes => self.runtimes.request(),
            Panel::Stash => {},
        }
    }

    fn map_keys_enabled(&self, ui: &Ui, session: &Session) -> bool {
        session.map().is_some()
            && self.save_dialog.is_none()
            && !self.settings_window.is_capturing_keybind()
            && !ui.io().want_text_input()
    }

    pub(super) fn apply_history_actions(&mut self, session: &mut Session, menu: &MenuActions) {
        if !menu.undo && !menu.redo {
            return;
        }

        self.cancel_edit_gestures(session, session.state.active());
        if menu.undo {
            session.undo();
        } else {
            session.redo();
        }
    }

    pub(super) fn read_edit_keys(&self, ui: &Ui, session: &Session, settings: &Settings, menu: &mut MenuActions) {
        if !self.map_keys_enabled(ui, session) {
            return;
        }

        if !unsafe { dear_imgui_rs::sys::igGetTopMostPopupModal() }.is_null() {
            return;
        }

        menu.undo |= settings.keybindings.get(KeybindAction::Undo).is_pressed_repeating(ui);
        menu.redo |= settings.keybindings.get(KeybindAction::Redo).is_pressed_repeating(ui);
        if menu.edit.is_none() {
            menu.edit = EDIT_KEYS
                .into_iter()
                .find(|(action, _)| settings.keybindings.get(*action).is_pressed(ui))
                .map(|(_, command)| command);
        }
    }

    pub(super) fn apply_file_actions(
        &mut self, ui: &Ui, session: &mut Session, settings: &Settings, menu: &mut MenuActions,
    ) {
        // https://i.redd.it/ggq5w2aliicf1.png
        let is_save_requested = mem::take(&mut self.save_requested);
        let is_save_pressed =
            self.map_keys_enabled(ui, session) && settings.keybindings.get(KeybindAction::Save).is_pressed(ui);
        if session.map().is_some()
            && !session.is_stash_active()
            && !menu.open_save_dialog
            && self.save_dialog.is_none()
            && (is_save_requested || is_save_pressed)
        {
            if session.can_save_map_in_place() {
                if let Err(error) = session.save_map() {
                    self.open_save_dialog(ui, session, Some(error.to_string()));
                }
            } else {
                menu.open_save_dialog = true;
            }
        }

        let is_map_keys_enabled = self.map_keys_enabled(ui, session);
        menu.save_all |= is_map_keys_enabled && settings.keybindings.get(KeybindAction::SaveAll).is_pressed(ui);
        menu.close_map |= is_map_keys_enabled && settings.keybindings.get(KeybindAction::CloseMap).is_pressed(ui);
        if menu.save_all {
            let outcome = session.save_all();
            if let Some(error) = outcome.error {
                self.open_error = Some(error);
            } else if let Some(id) = outcome.needs_path {
                session.set_active_document(id);
                menu.open_save_dialog = true;
            }
        }

        if menu.close_map
            && !session.is_stash_active()
            && let Some(id) = session.state.active()
        {
            self.request_close(session, [id]);
        }

        if menu.close_all {
            self.request_close(session, session.state.document_ids());
        }

        if self.map_keys_enabled(ui, session) {
            for (action, area, copy) in SCREENSHOT_KEYS {
                if settings.keybindings.get(action).is_pressed(ui) {
                    menu.screenshot = Some(ScreenshotRequest { area, copy });
                }
            }
        }
    }

    pub(super) fn apply_search_actions(
        &mut self, ui: &Ui, session: &mut Session, settings: &Settings, menu: &mut MenuActions,
    ) {
        let is_map_keys_enabled = self.map_keys_enabled(ui, session);
        menu.search |= is_map_keys_enabled && settings.keybindings.get(KeybindAction::Find).is_pressed(ui);
        menu.go_to |= is_map_keys_enabled && settings.keybindings.get(KeybindAction::GoTo).is_pressed(ui);
        if menu.search {
            match session
                .state
                .active_document()
                .and_then(|document| Some((document.id(), document.picked_instance()?)))
            {
                Some((document, instance)) => {
                    self.find
                        .open_for(session, document, instance, SimilarMatchKind::Prefab);
                },
                None => self.find.request(),
            }
        }

        if !is_map_keys_enabled {
            return;
        }

        let next = settings
            .keybindings
            .get(KeybindAction::FindNext)
            .is_pressed_repeating(ui);
        let previous = settings
            .keybindings
            .get(KeybindAction::FindPrevious)
            .is_pressed_repeating(ui);
        if (next || previous)
            && let Some(target) = self.find.step(session, next)
        {
            self.jump_to_instance(session, target);
        }
    }

    pub(super) fn apply_view_toggles(&mut self, session: &mut Session, settings: &mut Settings, menu: &MenuActions) {
        if menu.toggle_areas {
            session.toggle_areas();
        }

        if menu.toggle_area_outlines {
            session.toggle_area_outlines();
        }

        if menu.toggle_lighting {
            session.toggle_lighting();
        }

        if menu.toggle_tile_grid {
            settings.show_tile_grid = !settings.show_tile_grid;
        }

        if menu.toggle_pixel_grid {
            settings.show_selected_pixel_grid = !settings.show_selected_pixel_grid;
        }

        if menu.level_delta != 0 {
            request_level_change(session, menu.level_delta, &mut self.new_level_dialog);
        }

        if let Some(depth) = menu.underlay_depth {
            session.set_underlay_depth(depth);
        }
    }

    pub(super) fn open_save_dialog(&mut self, ui: &Ui, session: &Session, error: Option<String>) {
        let Some(document) = session.state.active() else { return };
        self.save_dialog = Some(SaveDialog {
            document,
            close_after_save: false,
            path: session
                .map_path()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
            format: session.map_format().unwrap_or_default(),
            error,
        });
        ui.open_popup(SAVE_MAP_POPUP);
    }
}
