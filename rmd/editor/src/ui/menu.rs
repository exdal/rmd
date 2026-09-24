use dear_imgui_rs::Ui;

use super::{OpenRequest, UiState};
use crate::{
    session::Session,
    settings::{KeybindAction, Settings},
};

/// How far down the "Blur below" menu goes. The option itself takes any depth.
const MAX_UNDERLAY_DEPTH: u32 = 3;

/// What the main menu bar asked for this frame
#[derive(Default)]
pub(super) struct MenuActions {
    pub(super) open: Option<OpenRequest>,
    pub(super) show_welcome: bool,
    pub(super) open_save_dialog: bool,
    pub(super) toggle_areas: bool,
    pub(super) toggle_area_outlines: bool,
    pub(super) toggle_lighting: bool,
    pub(super) toggle_tile_grid: bool,
    pub(super) toggle_pixel_grid: bool,
    pub(super) level_delta: i32,
    pub(super) underlay_depth: Option<u32>,
    pub(super) refit: bool,
    pub(super) undo: bool,
    pub(super) redo: bool,
}

impl UiState {
    pub(super) fn draw_menu_bar(
        &mut self, ui: &Ui, session: &mut Session, settings: &Settings, loading: bool,
    ) -> MenuActions {
        let mut actions = MenuActions::default();
        ui.main_menu_bar(|| {
            ui.menu("File", || {
                if ui.menu_item_enabled_selected_no_shortcut(
                    "Open codebase...",
                    false,
                    session.tree().is_none() && !loading,
                ) {
                    actions.open = Some(OpenRequest::PickCodebase);
                }
                if ui.menu_item_enabled_selected_no_shortcut("Open map...", false, session.tree().is_some() && !loading)
                {
                    actions.open = Some(OpenRequest::PickMap);
                }
                ui.separator();
                if ui.menu_item_enabled_selected_with_shortcut(
                    "Save...",
                    settings.keybindings.get(KeybindAction::Save).label(ui),
                    false,
                    session.map().is_some(),
                ) {
                    actions.open_save_dialog = true;
                }
                ui.separator();
                if ui.menu_item("Welcome") {
                    actions.show_welcome = true;
                }
                if ui.menu_item("Settings...") {
                    self.settings_window.open();
                }
                ui.separator();
                if ui.menu_item("Exit") {
                    self.request_exit();
                }
            });
            ui.menu("Edit", || {
                let next = session.undo_label();
                let label = next.map_or_else(|| String::from("Undo"), |edit| format!("Undo {edit}"));
                if ui.menu_item_enabled_selected_with_shortcut(
                    label,
                    settings.keybindings.get(KeybindAction::Undo).label(ui),
                    false,
                    next.is_some(),
                ) {
                    actions.undo = true;
                }

                let next = session.redo_label();
                let label = next.map_or_else(|| String::from("Redo"), |edit| format!("Redo {edit}"));
                if ui.menu_item_enabled_selected_with_shortcut(
                    label,
                    settings.keybindings.get(KeybindAction::Redo).label(ui),
                    false,
                    next.is_some(),
                ) {
                    actions.redo = true;
                }
            });
            ui.menu("View", || {
                if ui.menu_item_enabled_selected_with_shortcut(
                    "Show areas",
                    settings.keybindings.get(KeybindAction::ShowAreas).label(ui),
                    session.options.show_areas,
                    true,
                ) {
                    actions.toggle_areas = true;
                }
                if ui.menu_item_enabled_selected_with_shortcut(
                    "Show area outlines",
                    settings.keybindings.get(KeybindAction::ShowAreaOutlines).label(ui),
                    session.options.show_area_outlines,
                    true,
                ) {
                    actions.toggle_area_outlines = true;
                }
                if ui.menu_item_enabled_selected_with_shortcut(
                    "Show lighting",
                    settings.keybindings.get(KeybindAction::ShowLighting).label(ui),
                    session.options.show_lighting,
                    true,
                ) {
                    actions.toggle_lighting = true;
                }
                if ui.menu_item_enabled_selected_with_shortcut(
                    "Show tile grid",
                    settings.keybindings.get(KeybindAction::ShowTileGrid).label(ui),
                    settings.show_tile_grid,
                    true,
                ) {
                    actions.toggle_tile_grid = true;
                }
                if ui.menu_item_enabled_selected_with_shortcut(
                    "Show pixel grid",
                    settings.keybindings.get(KeybindAction::ShowPixelGrid).label(ui),
                    settings.show_selected_pixel_grid,
                    true,
                ) {
                    actions.toggle_pixel_grid = true;
                }
                if ui.menu_item_with_shortcut("Z up", settings.keybindings.get(KeybindAction::LevelUp).label(ui)) {
                    actions.level_delta += 1;
                }
                if ui.menu_item_with_shortcut("Z down", settings.keybindings.get(KeybindAction::LevelDown).label(ui)) {
                    actions.level_delta -= 1;
                }
                ui.menu("Blur below", || {
                    for depth in 0..=MAX_UNDERLAY_DEPTH {
                        let label = match depth {
                            0 => String::from("Off"),
                            depth => format!("{depth} level(s)"),
                        };

                        if ui.menu_item_enabled_selected(
                            label,
                            None::<&str>,
                            session.options.underlay_depth == depth,
                            true,
                        ) {
                            actions.underlay_depth = Some(depth);
                        }
                    }
                });
                if ui.menu_item_with_shortcut("Refit", settings.keybindings.get(KeybindAction::Refit).label(ui)) {
                    actions.refit = true;
                }
            });
            ui.menu("Git", || {
                if ui.menu_item("Git Panel") {
                    self.git_panel.focus();
                }

                if let Some(id) = session.state.active() {
                    let shown = session.git_state(id).is_some_and(|git| git.show_blame);
                    if ui.menu_item_enabled_selected_no_shortcut("Show Blame", shown, settings.git_enabled) {
                        session.toggle_blame(id, settings.blame_depth as usize);
                    }

                    let shown = session.git_state(id).is_some_and(|git| git.show_diff);
                    if ui.menu_item_enabled_selected_no_shortcut("Show Diff", shown, settings.git_enabled) {
                        session.toggle_diff(id);
                    }
                }

                if ui.menu_item_enabled_selected_no_shortcut("Refresh", false, settings.git_enabled) {
                    session.refresh_git();
                }
            });
        });

        actions
    }
}
