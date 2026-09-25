use dear_imgui_rs::Ui;

use super::{
    LAYER_KEYS,
    OpenRequest,
    ScreenshotArea,
    ScreenshotRequest,
    UiState,
    viewport::EditCommand,
    welcome::codebase_relative,
};
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
    pub(super) new_map: bool,
    pub(super) save_all: bool,
    pub(super) close_map: bool,
    pub(super) close_all: bool,
    pub(super) edit: Option<EditCommand>,
    pub(super) screenshot: Option<ScreenshotRequest>,
    pub(super) toggle_areas: bool,
    pub(super) toggle_area_outlines: bool,
    pub(super) toggle_lighting: bool,
    pub(super) toggle_tile_grid: bool,
    pub(super) toggle_pixel_grid: bool,
    pub(super) toggle_mirror_camera: bool,
    pub(super) level_delta: i32,
    pub(super) underlay_depth: Option<u32>,
    pub(super) refit: bool,
    pub(super) undo: bool,
    pub(super) redo: bool,
    pub(super) search: bool,
    pub(super) go_to: bool,
    pub(super) resize_map: bool,
    pub(super) reset_layout: bool,
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
                if ui.menu_item_enabled_selected_no_shortcut("New map...", false, session.tree().is_some() && !loading)
                {
                    actions.new_map = true;
                }
                if ui.menu_item_enabled_selected_no_shortcut("Open map...", false, session.tree().is_some() && !loading)
                {
                    actions.open = Some(OpenRequest::PickMap);
                }
                let codebase = session.environment_path().filter(|_| !loading);
                let has_recent = codebase.is_some_and(|codebase| settings.recent_maps_for(codebase).next().is_some());
                if let Some(_recent) = ui.begin_menu_with_enabled("Recent maps", has_recent)
                    && let Some(codebase) = codebase
                {
                    let base = session.codebase_dir().unwrap_or(codebase);
                    for recent in settings.recent_maps_for(codebase) {
                        if ui.menu_item(codebase_relative(base, &recent.map)) {
                            actions.open = Some(OpenRequest::Map(recent.map.clone()));
                        }
                    }
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
                if ui.menu_item_enabled_selected_with_shortcut(
                    "Save all",
                    settings.keybindings.get(KeybindAction::SaveAll).label(ui),
                    false,
                    session.state.documents().iter().any(|document| document.is_dirty()),
                ) {
                    actions.save_all = true;
                }
                if let Some(_screenshot) = ui.begin_menu_with_enabled("Screenshot", session.map().is_some()) {
                    let shortcut = settings.keybindings.get(KeybindAction::Screenshot).label(ui);
                    let selected = session.selection().is_some();
                    for (label, area, copy, enabled) in [
                        ("Save map...", ScreenshotArea::Map, false, true),
                        ("Save selection...", ScreenshotArea::Selection, false, selected),
                        ("Copy map", ScreenshotArea::Map, true, true),
                        ("Copy selection", ScreenshotArea::Selection, true, selected),
                    ] {
                        let shortcut = (area == ScreenshotArea::Map && !copy).then_some(shortcut.as_str());
                        if ui.menu_item_enabled_selected(label, shortcut, false, enabled) {
                            actions.screenshot = Some(ScreenshotRequest { area, copy });
                        }
                    }
                }
                ui.separator();
                if ui.menu_item_enabled_selected_with_shortcut(
                    "Close map",
                    settings.keybindings.get(KeybindAction::CloseMap).label(ui),
                    false,
                    session.map().is_some(),
                ) {
                    actions.close_map = true;
                }
                if ui.menu_item_enabled_selected_no_shortcut("Close all", false, !session.state.is_empty()) {
                    actions.close_all = true;
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

                ui.separator();
                let selected = session.selection().is_some();
                for (label, action, command, enabled) in [
                    ("Copy", KeybindAction::Copy, EditCommand::Copy, selected),
                    ("Cut", KeybindAction::Cut, EditCommand::Cut, selected),
                    (
                        "Paste",
                        KeybindAction::Paste,
                        EditCommand::Paste,
                        session.clipboard().is_some(),
                    ),
                    ("Delete", KeybindAction::Delete, EditCommand::Delete, selected),
                    (
                        "Deselect",
                        KeybindAction::Deselect,
                        EditCommand::Deselect,
                        selected || session.selected_instance().is_some(),
                    ),
                ] {
                    if ui.menu_item_enabled_selected_with_shortcut(
                        label,
                        settings.keybindings.get(action).label(ui),
                        false,
                        enabled && session.map().is_some(),
                    ) {
                        actions.edit = Some(command);
                    }
                }

                ui.separator();
                if ui.menu_item_enabled_selected_with_shortcut(
                    "Search...",
                    settings.keybindings.get(KeybindAction::Find).label(ui),
                    false,
                    session.map().is_some(),
                ) {
                    actions.search = true;
                }
                if ui.menu_item_enabled_selected_with_shortcut(
                    "Go to...",
                    settings.keybindings.get(KeybindAction::GoTo).label(ui),
                    false,
                    session.map().is_some(),
                ) {
                    actions.go_to = true;
                }
                if ui.menu_item_enabled_selected_no_shortcut("Resize map...", false, session.map().is_some()) {
                    actions.resize_map = true;
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
                ui.menu("Type layers", || {
                    for (action, layer) in LAYER_KEYS {
                        if ui.menu_item_enabled_selected_with_shortcut(
                            layer.label(),
                            settings.keybindings.get(action).label(ui),
                            session.is_layer_visible(layer),
                            session.tree().is_some(),
                        ) {
                            session.toggle_layer(layer);
                        }
                    }
                    ui.separator();
                    if ui.menu_item_enabled_selected_with_shortcut(
                        "Show all",
                        settings.keybindings.get(KeybindAction::ShowAllLayers).label(ui),
                        false,
                        session.hides_types(),
                    ) {
                        session.show_all_types();
                    }
                });
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
                if ui.menu_item_enabled_selected_no_shortcut("Mirror camera", settings.mirror_camera, true) {
                    actions.toggle_mirror_camera = true;
                }
                ui.set_item_tooltip("Keep pan, zoom and Z level the same in every map view");
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
            ui.menu("Window", || {
                if ui.menu_item("Reset layout") {
                    actions.reset_layout = true;
                }
            });
        });

        actions
    }
}
