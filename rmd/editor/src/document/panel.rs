use std::path::{Path, PathBuf};

use dear_imgui_rs::{DragFlags, Ui};
use dmm::{Coord, MapFormat, Prefab, Size};
use ui::{
    icons::materialdesignicons::ICON_DOTS_HORIZONTAL,
    widgets::{DialogButton, begin_centered_modal, dialog_buttons, dpi},
};

use crate::object_tree::panel::type_search::{MAX_CUSTOM_FILL_SEARCH_RESULTS, draw_type_path_search};
use crate::{
    document::{DocumentId, MapDocument},
    environment::panel::DIAGNOSTIC_WARNING_COLOR,
    frontend::dialog::{DIALOG_FIELD_WIDTH, MODAL_FLAGS, SAVE_ERROR_COLOR},
    session::Session,
};

pub(crate) const NEW_MAP_POPUP: &str = "New map##new-map";

const NEW_LEVEL_POPUP: &str = "Create Z level##new-z-level";
const DELETE_LEVEL_POPUP: &str = "Delete Z level##delete-z-level";

pub(crate) const RESIZE_MAP_POPUP: &str = "Resize map##resize-map";

pub(crate) const GO_TO_POPUP: &str = "Go to coordinates##go-to";

const NEW_MAP_PATH_WIDTH: f32 = 460.0;

const NEW_MAP_DEFAULT_WIDTH: u32 = 255;

const NEW_MAP_DEFAULT_HEIGHT: u32 = 255;

const NEW_MAP_DEFAULT_LEVELS: u32 = 1;

pub(crate) const SAVE_MAP_POPUP: &str = "Save map##save-map";

const SAVE_MAP_PATH_WIDTH: f32 = 460.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SaveDialog {
    pub(crate) document: DocumentId,
    pub(crate) close_after_save: bool,
    pub(crate) path: String,
    pub(crate) format: MapFormat,
    pub(crate) error: Option<String>,
}

pub(crate) enum SaveDialogOutcome {
    Saved {
        document: DocumentId,
        close_after_save: bool,
    },
    Cancelled {
        close_after_save: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NewMapDialog {
    pub(crate) path: String,
    format: MapFormat,
    width: u32,
    height: u32,
    levels: u32,
    pub(crate) error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TileFillPaths {
    turf: String,
    area: String,
}

impl TileFillPaths {
    fn defaults(session: &Session) -> Self {
        session
            .default_fill()
            .map(|fill| Self {
                turf: fill[0].path.to_string(),
                area: fill[1].path.to_string(),
            })
            .unwrap_or_default()
    }

    fn get_mut(&mut self, field: TileFillField) -> &mut String {
        match field {
            TileFillField::Turf => &mut self.turf,
            TileFillField::Area => &mut self.area,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TileFillField {
    Turf,
    Area,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TileFillSearch {
    paths: TileFillPaths,
    query: String,
    searching: bool,
    was_searching: bool,
}

impl TileFillSearch {
    pub(crate) fn new(session: &Session, remembered: Option<&TileFillPaths>) -> Self {
        Self {
            paths: remembered.cloned().unwrap_or_else(|| TileFillPaths::defaults(session)),
            ..Self::default()
        }
    }

    fn captures_keys(&self) -> bool {
        self.searching || self.was_searching
    }

    fn remember(&self, session: &Session, remembered: &mut Option<TileFillPaths>) {
        *remembered = (self.paths != TileFillPaths::defaults(session)).then(|| self.paths.clone());
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NewLevelDialog {
    pub(crate) document: DocumentId,
    fill: TileFillSearch,
    error: Option<String>,
    open: bool,
}

impl NewLevelDialog {
    pub(crate) fn new(document: DocumentId) -> Self {
        Self {
            document,
            fill: TileFillSearch::default(),
            error: None,
            open: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeleteLevelDialog {
    pub(crate) document: DocumentId,
    z: u32,
    // peers' tile edits don't matter, a level change does
    levels: u32,
    pub(crate) open: bool,
    error: Option<String>,
}

impl DeleteLevelDialog {
    pub(crate) fn new(document: &MapDocument) -> Self {
        Self {
            document: document.id(),
            z: document.z,
            levels: document.map.size().z,
            open: true,
            error: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResizeMapDialog {
    width: u32,
    height: u32,
    fill: TileFillSearch,
    losses: Option<(u32, u32, Vec<Prefab>, usize)>,
    error: Option<String>,
}

impl ResizeMapDialog {
    pub(crate) fn new(size: Size, fill: TileFillSearch) -> Self {
        Self {
            width: size.x,
            height: size.y,
            fill,
            losses: None,
            error: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GoToDialog {
    x: i32,
    y: i32,
    z: i32,
}

impl GoToDialog {
    pub(crate) fn new(target: Coord) -> Self {
        Self {
            x: target.x as i32,
            y: target.y as i32,
            z: target.z as i32,
        }
    }
}

impl Default for NewMapDialog {
    fn default() -> Self {
        Self {
            path: String::new(),
            format: MapFormat::Tgm,
            width: NEW_MAP_DEFAULT_WIDTH,
            height: NEW_MAP_DEFAULT_HEIGHT,
            levels: NEW_MAP_DEFAULT_LEVELS,
            error: None,
        }
    }
}

pub(crate) fn draw_new_map_dialog(ui: &Ui, session: &mut Session, dialog: &mut Option<NewMapDialog>) -> (bool, bool) {
    let Some(state) = dialog.as_mut() else {
        return (false, false);
    };

    let Some(_modal) = begin_centered_modal(ui, NEW_MAP_POPUP, MODAL_FLAGS) else {
        return (false, false);
    };

    let mut close = false;
    let mut pick_path = false;
    let mut created = false;

    ui.text("Path");
    let button_size = ui.frame_height();
    let spacing = ui.clone_style().item_spacing()[0];
    ui.set_next_item_width(NEW_MAP_PATH_WIDTH * dpi(ui) - button_size - spacing);
    let submitted = ui
        .input_text("##new-map-path", &mut state.path)
        .enter_returns_true(true)
        .build();
    ui.same_line();
    if ui.button_with_size(
        format!("{ICON_DOTS_HORIZONTAL}##new-map-path-picker"),
        [button_size, button_size],
    ) {
        pick_path = true;
    }

    ui.set_item_tooltip("Choose a map path");

    ui.text("Format");
    if ui.radio_button("DMM", state.format == MapFormat::Standard) {
        state.format = MapFormat::Standard;
    }

    ui.same_line();
    if ui.radio_button("TGM", state.format == MapFormat::Tgm) {
        state.format = MapFormat::Tgm;
    }

    for (label, id, value) in [
        ("Width", "##new-map-width", &mut state.width),
        ("Height", "##new-map-height", &mut state.height),
        ("Z levels", "##new-map-levels", &mut state.levels),
    ] {
        ui.text(label);
        ui.set_next_item_width(NEW_MAP_PATH_WIDTH * dpi(ui));
        ui.drag_config(id)
            .range(1, u32::MAX)
            .flags(DragFlags::ALWAYS_CLAMP)
            .build(ui, value);
    }

    if let Some(error) = state.error.as_deref() {
        ui.text_colored(SAVE_ERROR_COLOR, error);
    }

    let valid_dimensions = [state.width, state.height, state.levels]
        .into_iter()
        .all(|dimension| dimension > 0);
    let can_create = !state.path.trim().is_empty() && valid_dimensions;
    let pressed = dialog_buttons(
        ui,
        &[
            DialogButton::cancel("Cancel"),
            DialogButton::primary("Create Map").enabled(can_create),
        ],
        true,
    );

    if pressed == Some(0) {
        close = true;
        ui.close_current_popup();
    }

    if can_create && (pressed == Some(1) || submitted) {
        let result = session
            .codebase_dir()
            .ok_or_else(|| String::from("no codebase is loaded"))
            .and_then(|base| resolve_new_map_path(base, &state.path))
            .and_then(|path| {
                session
                    .create_map(
                        &path,
                        Size {
                            x: state.width,
                            y: state.height,
                            z: state.levels,
                        },
                        state.format,
                    )
                    .map_err(|error| error.to_string())
            });
        match result {
            Ok(()) => {
                created = true;
                close = true;
                ui.close_current_popup();
            },
            Err(error) => state.error = Some(error),
        }
    }

    if close {
        *dialog = None;
    }

    (pick_path, created)
}

pub(crate) fn draw_resize_map_dialog(
    ui: &Ui, session: &mut Session, dialog: &mut Option<ResizeMapDialog>, remembered_fill: &mut Option<TileFillPaths>,
) -> bool {
    let Some(state) = dialog.as_mut() else {
        return false;
    };

    let Some(_modal) = begin_centered_modal(ui, RESIZE_MAP_POPUP, MODAL_FLAGS) else {
        return false;
    };

    let mut close = false;
    let mut resized = false;

    let Some(size) = session.map().map(|map| map.size()) else {
        *dialog = None;
        ui.close_current_popup();
        return false;
    };

    ui.text(format!("Currently {}x{} on {} Z level(s)", size.x, size.y, size.z));
    for (label, id, value) in [
        ("Width", "##resize-map-width", &mut state.width),
        ("Height", "##resize-map-height", &mut state.height),
    ] {
        ui.text(label);
        ui.set_next_item_width(DIALOG_FIELD_WIDTH * dpi(ui));
        ui.drag_config(id)
            .range(1, u32::MAX)
            .flags(DragFlags::ALWAYS_CLAMP)
            .build(ui, value);
    }

    let fill = draw_tile_fill_search(ui, session, &mut state.fill);
    let wrap = ui.push_text_wrap_pos(ui.cursor_pos()[0] + DIALOG_FIELD_WIDTH * dpi(ui));
    let (width, height) = (state.width, state.height);
    let losses = match (&state.losses, &fill) {
        (_, None) => 0,
        (Some((cached_width, cached_height, cached_fill, losses)), Some(fill))
            if (*cached_width, *cached_height, cached_fill) == (width, height, fill) =>
        {
            *losses
        },
        (_, Some(fill)) => {
            let losses = session.resize_losses(width, height, fill);
            state.losses = Some((width, height, fill.clone(), losses));
            losses
        },
    };

    if losses > 0 {
        let noun = if losses == 1 { "tile" } else { "tiles" };
        ui.text_colored(
            DIAGNOSTIC_WARNING_COLOR,
            format!("{losses} {noun} will be deleted! You can revert this operation with undo."),
        );
    }

    if let Some(error) = state.error.as_deref() {
        ui.text_colored(SAVE_ERROR_COLOR, error);
    }

    wrap.end();

    let changed = (width, height) != (size.x, size.y);
    let resize_button = if losses == 0 {
        DialogButton::primary("Resize Map")
    } else {
        DialogButton::destructive("Resize Map", true)
    };
    let pressed = dialog_buttons(
        ui,
        &[
            DialogButton::cancel("Cancel"),
            resize_button.enabled(changed && fill.is_some()),
        ],
        !state.fill.captures_keys(),
    );

    if pressed == Some(0) {
        close = true;
        ui.close_current_popup();
    }

    if changed
        && pressed == Some(1)
        && let Some(fill) = fill
    {
        match session.resize_map(width, height, &fill) {
            Ok(()) => {
                state.fill.remember(session, remembered_fill);
                resized = true;
                close = true;
                ui.close_current_popup();
            },
            Err(error) => state.error = Some(error),
        }
    }

    if close {
        *dialog = None;
    }

    resized
}

pub(crate) fn draw_go_to_dialog(ui: &Ui, session: &Session, dialog: &mut Option<GoToDialog>) -> Option<Coord> {
    let state = dialog.as_mut()?;
    let _modal = begin_centered_modal(ui, GO_TO_POPUP, MODAL_FLAGS)?;

    let mut close = false;
    let mut target = None;

    let Some(size) = session.map().map(|map| map.size()) else {
        *dialog = None;
        ui.close_current_popup();
        return None;
    };

    ui.text(format!("Map is {}x{} on {} Z level(s)", size.x, size.y, size.z));
    for (label, id, value, max) in [
        ("X", "##go-to-x", &mut state.x, size.x),
        ("Y", "##go-to-y", &mut state.y, size.y),
        ("Z", "##go-to-z", &mut state.z, size.z),
    ] {
        ui.text(label);
        ui.set_next_item_width(DIALOG_FIELD_WIDTH * dpi(ui));
        if label == "X" && ui.is_window_appearing() {
            ui.set_keyboard_focus_here();
        }
        ui.input_int(id, value);
        *value = (*value).clamp(1, max.max(1) as i32);
    }

    let pressed = dialog_buttons(
        ui,
        &[DialogButton::cancel("Cancel"), DialogButton::primary("Go to Tile")],
        true,
    );

    if pressed == Some(0) {
        close = true;
        ui.close_current_popup();
    }

    if pressed == Some(1) {
        target = Some(Coord::new(state.x as u32, state.y as u32, state.z as u32));
        close = true;
        ui.close_current_popup();
    }

    if close {
        *dialog = None;
    }

    target
}

pub(crate) fn draw_new_level_dialog(
    ui: &Ui, session: &mut Session, dialog: &mut Option<NewLevelDialog>, remembered_fill: &mut Option<TileFillPaths>,
) {
    let Some(state) = dialog.as_mut() else {
        return;
    };

    if state.open {
        ui.open_popup(NEW_LEVEL_POPUP);
        state.fill = TileFillSearch::new(session, remembered_fill.as_ref());
        state.open = false;
    }

    let Some(_modal) = begin_centered_modal(ui, NEW_LEVEL_POPUP, MODAL_FLAGS) else {
        return;
    };

    let mut close = false;
    if let Some(document) = session.state.document(state.document) {
        ui.text(format!("Create Z level {}", document.map.size().z.saturating_add(1)));
    }

    let fill = draw_tile_fill_search(ui, session, &mut state.fill);
    if let Some(error) = state.error.as_deref() {
        ui.text_colored(SAVE_ERROR_COLOR, error);
    }

    let pressed = dialog_buttons(
        ui,
        &[
            DialogButton::cancel("Cancel"),
            DialogButton::primary("Create Level").enabled(fill.is_some()),
        ],
        !state.fill.captures_keys(),
    );

    if pressed == Some(0) {
        close = true;
        ui.close_current_popup();
    }

    if pressed == Some(1)
        && let Some(fill) = fill
    {
        match session.create_level(state.document, &fill) {
            Ok(_) => {
                state.fill.remember(session, remembered_fill);
                close = true;
                ui.close_current_popup();
            },
            Err(error) => state.error = Some(error),
        }
    }

    if close {
        *dialog = None;
    }
}

pub(crate) fn draw_delete_level_dialog(ui: &Ui, session: &mut Session, dialog: &mut Option<DeleteLevelDialog>) {
    let Some(state) = dialog.as_mut() else {
        return;
    };

    if state.open {
        ui.open_popup(DELETE_LEVEL_POPUP);
        state.open = false;
    }

    let Some(_modal) = begin_centered_modal(ui, DELETE_LEVEL_POPUP, MODAL_FLAGS) else {
        return;
    };

    let mut close = false;
    let is_valid = session.state.active() == Some(state.document)
        && session.can_delete_level(state.document)
        && session
            .state
            .document(state.document)
            .is_some_and(|document| document.map.size().z == state.levels && document.z == state.z);
    ui.text(format!("Delete Z level {} and all its contents?", state.z));
    ui.text_disabled("Higher levels will shift down. You can undo this deletion.");

    if !is_valid {
        ui.text_colored(
            SAVE_ERROR_COLOR,
            "The map changed or the level cannot be deleted. Cancel and try again.",
        );
    }

    if let Some(error) = &state.error {
        ui.text_colored(SAVE_ERROR_COLOR, error);
    }

    let pressed = dialog_buttons(
        ui,
        &[
            DialogButton::cancel("Cancel"),
            DialogButton::destructive("Delete Level", true).enabled(is_valid),
        ],
        true,
    );

    if pressed == Some(0) {
        close = true;
        ui.close_current_popup();
    }

    if is_valid && pressed == Some(1) {
        match session.delete_level(state.document, state.z) {
            Ok(()) => {
                close = true;
                ui.close_current_popup();
            },
            Err(error) => state.error = Some(error),
        }
    }

    if close {
        *dialog = None;
    }
}

fn draw_tile_fill_search(ui: &Ui, session: &Session, search: &mut TileFillSearch) -> Option<Vec<Prefab>> {
    search.was_searching = search.searching;
    search.searching = false;
    let tree = session.tree();
    let roots = tree.map(|tree| tree.roots());

    ui.text_disabled("New tiles will be filled with:");
    for (label, id, field, root) in [
        (
            "Turf",
            "##tile-fill-turf",
            TileFillField::Turf,
            roots.and_then(|roots| roots.turf),
        ),
        (
            "Area",
            "##tile-fill-area",
            TileFillField::Area,
            roots.and_then(|roots| roots.area),
        ),
    ] {
        ui.text(label);
        ui.set_next_item_width(DIALOG_FIELD_WIDTH * dpi(ui));
        let Some(_combo) = ui.begin_combo(id, search.paths.get_mut(field).as_str()) else {
            continue;
        };
        search.searching = true;
        if ui.is_window_appearing() {
            search.query.clone_from(search.paths.get_mut(field));
            ui.set_keyboard_focus_here();
        }
        let picked = draw_type_path_search(
            ui,
            tree,
            &mut search.query,
            "##tile-fill-search",
            MAX_CUSTOM_FILL_SEARCH_RESULTS,
            |tree, path| {
                root.zip(tree.id_of(path))
                    .is_some_and(|(root, id)| tree.is_subtype_of(id, root))
            },
            |_| true,
        );
        if let Some(path) = picked {
            *search.paths.get_mut(field) = path.to_string();
        }
    }

    match session.tile_fill(&search.paths.turf, &search.paths.area) {
        Ok(fill) => Some(fill),
        Err(error) => {
            ui.text_colored(SAVE_ERROR_COLOR, error);
            None
        },
    }
}

fn resolve_new_map_path(codebase_dir: &Path, input: &str) -> Result<PathBuf, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err(String::from("enter a map path"));
    }

    let mut path = PathBuf::from(input);
    match path.extension().and_then(|extension| extension.to_str()) {
        None => {
            path.set_extension("dmm");
        },
        Some(extension) if extension.eq_ignore_ascii_case("dmm") => {},
        Some(_) => return Err(String::from("map path must use the .dmm extension")),
    }

    let path = if path.is_absolute() {
        path
    } else {
        codebase_dir.join(path)
    };
    if path.exists() {
        return Err(format!("{} already exists", path.display()));
    }
    if !path.parent().is_some_and(Path::is_dir) {
        return Err(format!("parent directory for {} does not exist", path.display()));
    }

    Ok(path)
}

pub(crate) fn draw_save_dialog(
    ui: &Ui, session: &mut Session, dialog: &mut Option<SaveDialog>,
) -> Option<SaveDialogOutcome> {
    let state = dialog.as_mut()?;
    if !ui.is_popup_open(SAVE_MAP_POPUP) {
        ui.open_popup(SAVE_MAP_POPUP);
    }

    let _modal = begin_centered_modal(ui, SAVE_MAP_POPUP, MODAL_FLAGS)?;

    let mut outcome = None;
    ui.text("Path");
    ui.set_next_item_width(SAVE_MAP_PATH_WIDTH * dpi(ui));
    let submitted = ui
        .input_text("##save-map-path", &mut state.path)
        .enter_returns_true(true)
        .build();

    ui.text("Format");
    if ui.radio_button("DMM", state.format == MapFormat::Standard) {
        state.format = MapFormat::Standard;
    }

    ui.same_line();
    if ui.radio_button("TGM", state.format == MapFormat::Tgm) {
        state.format = MapFormat::Tgm;
    }

    if let Some(error) = state.error.as_deref() {
        ui.text_colored(SAVE_ERROR_COLOR, error);
    }

    let path = state.path.trim();
    let can_save = !path.is_empty();
    let pressed = dialog_buttons(
        ui,
        &[
            DialogButton::cancel("Cancel"),
            DialogButton::primary("Save Map").enabled(can_save),
        ],
        true,
    );

    if pressed == Some(0) {
        outcome = Some(SaveDialogOutcome::Cancelled {
            close_after_save: state.close_after_save,
        });
        ui.close_current_popup();
    }

    if outcome.is_none() && can_save && (pressed == Some(1) || submitted) {
        let result = if session.set_active_document(state.document) {
            session.save_map_as(Path::new(path), state.format)
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "This map is no longer open.",
            ))
        };

        match result {
            Ok(()) => {
                outcome = Some(SaveDialogOutcome::Saved {
                    document: state.document,
                    close_after_save: state.close_after_save,
                });
                ui.close_current_popup();
            },
            Err(error) => state.error = Some(error.to_string()),
        }
    }

    if outcome.is_some() {
        *dialog = None;
    }

    outcome
}

#[cfg(test)]
mod tests {
    use base::path::TreePath;
    use std::{error::Error, path::PathBuf};

    use dear_imgui_rs::Key;
    use dmm::{Coord, Map, MapFormat, Prefab, Size};
    use net::LevelContents;
    use ui::testing::{CONTEXT_LOCK, rectangle_context};

    use super::{
        DeleteLevelDialog, GO_TO_POPUP, GoToDialog, NewLevelDialog, NewMapDialog, RESIZE_MAP_POPUP, ResizeMapDialog,
        TileFillPaths, TileFillSearch, draw_delete_level_dialog, draw_go_to_dialog, draw_new_level_dialog,
        draw_new_map_dialog, draw_resize_map_dialog, resolve_new_map_path,
    };
    use crate::{
        document::{MapDocument, patch},
        frontend::fixtures::click_dialog_button,
        session::Session,
        testing::TempDir,
    };

    #[test]
    fn enter_creates_a_z_level_filled_with_the_default_turf_and_area() {
        let _guard = CONTEXT_LOCK.lock().unwrap();
        let mut context = ui::testing::rectangle_context();
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env");
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        let document = session
            .state
            .open_document(MapDocument::new(dmm::Map::new(Size { x: 2, y: 1, z: 1 }), 1));
        let mut dialog = Some(NewLevelDialog::new(document));
        let mut remembered = None;

        let ui = context.frame();
        draw_new_level_dialog(ui, &mut session, &mut dialog, &mut remembered);
        assert!(context.render_legacy().valid());
        assert_eq!(session.level_count(), 1, "opening the dialog creates nothing");
        assert_eq!(
            dialog.as_ref().unwrap().fill.paths,
            TileFillPaths::defaults(&session),
            "the search starts at the defaults"
        );

        context.io_mut().add_key_event(dear_imgui_rs::Key::Enter, true);
        let ui = context.frame();
        draw_new_level_dialog(ui, &mut session, &mut dialog, &mut remembered);
        assert!(context.render_legacy().valid());

        assert!(dialog.is_none());
        assert_eq!(remembered, None, "unchanged defaults are not remembered");
        assert_eq!(session.level_count(), 2);
        assert_eq!(
            session.map().unwrap().tile_at(Coord::new(2, 1, 2)),
            session.default_fill().ok().as_ref()
        );
    }

    #[test]
    fn a_changed_fill_is_remembered_for_the_next_z_level_and_resize() {
        let _guard = CONTEXT_LOCK.lock().unwrap();
        let mut context = ui::testing::rectangle_context();
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env");
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        let document = session
            .state
            .open_document(MapDocument::new(dmm::Map::new(Size { x: 2, y: 1, z: 1 }), 1));
        let mut dialog = Some(NewLevelDialog::new(document));
        let mut remembered = None;

        let ui = context.frame();
        draw_new_level_dialog(ui, &mut session, &mut dialog, &mut remembered);
        assert!(context.render_legacy().valid());
        dialog.as_mut().unwrap().fill.paths.turf = String::from("/turf/open/floor");

        context.io_mut().add_key_event(dear_imgui_rs::Key::Enter, true);
        let ui = context.frame();
        draw_new_level_dialog(ui, &mut session, &mut dialog, &mut remembered);
        assert!(context.render_legacy().valid());

        let expected = TileFillPaths {
            turf: String::from("/turf/open/floor"),
            area: TileFillPaths::defaults(&session).area,
        };
        assert_eq!(remembered.as_ref(), Some(&expected));
        assert_eq!(
            session.map().unwrap().tile_at(Coord::new(1, 1, 2)).unwrap()[0].path,
            TreePath::parse("/turf/open/floor")
        );
        assert_eq!(
            ResizeMapDialog::new(
                Size { x: 2, y: 1, z: 2 },
                TileFillSearch::new(&session, remembered.as_ref())
            )
            .fill
            .paths,
            expected
        );
    }

    #[test]
    fn enter_in_an_open_fill_search_does_not_create_a_z_level() {
        let _guard = CONTEXT_LOCK.lock().unwrap();
        let mut context = ui::testing::rectangle_context();
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env");
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        let document = session
            .state
            .open_document(MapDocument::new(dmm::Map::new(Size { x: 2, y: 1, z: 1 }), 1));
        let mut dialog = Some(NewLevelDialog::new(document));
        let mut remembered = None;

        let ui = context.frame();
        draw_new_level_dialog(ui, &mut session, &mut dialog, &mut remembered);
        assert!(context.render_legacy().valid());
        dialog.as_mut().unwrap().fill.searching = true;

        context.io_mut().add_key_event(dear_imgui_rs::Key::Enter, true);
        let ui = context.frame();
        draw_new_level_dialog(ui, &mut session, &mut dialog, &mut remembered);
        assert!(context.render_legacy().valid());

        assert!(dialog.is_some());
        assert_eq!(session.level_count(), 1);
    }

    #[test]
    fn an_invalid_fill_path_blocks_z_level_creation() {
        let _guard = CONTEXT_LOCK.lock().unwrap();
        let mut context = ui::testing::rectangle_context();
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env");
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        let document = session
            .state
            .open_document(MapDocument::new(dmm::Map::new(Size { x: 2, y: 1, z: 1 }), 1));
        let mut dialog = Some(NewLevelDialog::new(document));
        let mut remembered = None;

        let ui = context.frame();
        draw_new_level_dialog(ui, &mut session, &mut dialog, &mut remembered);
        assert!(context.render_legacy().valid());
        dialog.as_mut().unwrap().fill.paths.area = String::from("/turf");

        context.io_mut().add_key_event(dear_imgui_rs::Key::Enter, true);
        let ui = context.frame();
        draw_new_level_dialog(ui, &mut session, &mut dialog, &mut remembered);
        assert!(context.render_legacy().valid());

        assert!(dialog.is_some());
        assert_eq!(remembered, None);
        assert_eq!(session.level_count(), 1);
    }

    #[test]
    fn the_go_to_dialog_clamps_to_the_map_and_enter_returns_the_tile() {
        let _guard = CONTEXT_LOCK.lock().unwrap();
        let mut context = ui::testing::rectangle_context();
        let mut session = Session::new();
        session
            .state
            .open_document(MapDocument::new(dmm::Map::new(Size { x: 3, y: 2, z: 1 }), 1));
        let mut dialog = Some(GoToDialog::new(Coord::new(9, 1, 5)));

        let ui = context.frame();
        ui.open_popup(GO_TO_POPUP);
        assert_eq!(draw_go_to_dialog(ui, &session, &mut dialog), None);
        assert!(context.render_legacy().valid());
        assert_eq!(dialog, Some(GoToDialog { x: 3, y: 1, z: 1 }));

        context.io_mut().add_key_event(dear_imgui_rs::Key::Enter, true);
        let ui = context.frame();
        assert_eq!(draw_go_to_dialog(ui, &session, &mut dialog), Some(Coord::new(3, 1, 1)));
        assert!(context.render_legacy().valid());
        assert!(dialog.is_none());
    }

    #[test]
    fn the_resize_dialog_warns_about_the_tiles_a_shrink_deletes() {
        let _guard = CONTEXT_LOCK.lock().unwrap();
        let mut context = ui::testing::rectangle_context();
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env");
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        let mut map = dmm::Map::new(Size { x: 3, y: 1, z: 1 });
        let table = map.intern_tile(vec![Prefab::new(TreePath::parse("/obj/structure/table"))]);
        map.grid[0][0].fill(table);
        session.state.open_document(MapDocument::new(map, 1));
        let fill = TileFillSearch::new(&session, None);
        let mut dialog = Some(ResizeMapDialog::new(Size { x: 3, y: 1, z: 1 }, fill));
        let mut remembered = None;

        for width in [3, 1] {
            dialog.as_mut().unwrap().width = width;
            let ui = context.frame();
            ui.open_popup(RESIZE_MAP_POPUP);
            assert!(!draw_resize_map_dialog(ui, &mut session, &mut dialog, &mut remembered));
            assert!(context.render_legacy().valid());
        }

        let losses = dialog
            .unwrap()
            .losses
            .map(|(width, height, _, losses)| (width, height, losses));
        assert_eq!(losses, Some((1, 1, 2)));
        assert_eq!(
            session.map().unwrap().size().x,
            3,
            "nothing changes until Resize is pressed"
        );
    }

    #[test]
    fn the_resize_dialog_accepts_dimensions_above_255() {
        let _guard = CONTEXT_LOCK.lock().unwrap();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env");
        for (width, height) in [(256, 1), (1, 256)] {
            let mut context = rectangle_context();
            let mut session = Session::new();
            session.load_environment(&root.join("test.dme")).unwrap();
            session
                .state
                .open_document(MapDocument::new(Map::new(Size { x: 1, y: 1, z: 1 }), 1));
            let fill = session.default_fill().unwrap();
            let mut dialog = Some(ResizeMapDialog::new(
                session.map().unwrap().size(),
                TileFillSearch::new(&session, None),
            ));
            dialog.as_mut().unwrap().width = width;
            dialog.as_mut().unwrap().height = height;
            let mut remembered = None;
            let mut resized = false;

            click_dialog_button(
                &mut context,
                c"Resize map##resize-map",
                &["Cancel", "Resize Map"],
                |ui| resized |= draw_resize_map_dialog(ui, &mut session, &mut dialog, &mut remembered),
            );

            assert!(resized);
            assert!(dialog.is_none());
            assert_eq!(
                session.map().unwrap().size(),
                Size {
                    x: width,
                    y: height,
                    z: 1,
                }
            );
            assert_eq!(
                session.map().unwrap().tile_at(Coord::new(width, height, 1)),
                Some(&fill)
            );
        }
    }

    #[test]
    fn the_resize_dialog_preserves_dimensions_up_to_u32_max() {
        let _guard = CONTEXT_LOCK.lock().unwrap();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env");
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        session
            .state
            .open_document(MapDocument::new(Map::new(Size { x: 1, y: 1, z: 1 }), 1));

        for dimension in [i32::MAX as u32 + 1, u32::MAX] {
            let mut context = rectangle_context();
            let mut dialog = Some(ResizeMapDialog::new(
                Size {
                    x: dimension,
                    y: dimension,
                    z: 1,
                },
                TileFillSearch::new(&session, None),
            ));
            let mut remembered = None;

            let ui = context.frame();
            ui.open_popup(RESIZE_MAP_POPUP);
            assert!(!draw_resize_map_dialog(ui, &mut session, &mut dialog, &mut remembered));
            assert!(context.render_legacy().valid());

            let state = dialog.unwrap();
            assert_eq!((state.width, state.height), (dimension, dimension));
            assert_eq!(state.losses.unwrap().0, dimension);
            assert_eq!(session.map().unwrap().size(), Size { x: 1, y: 1, z: 1 });
        }
    }

    #[test]
    fn new_map_defaults_are_255_by_255_with_one_level() {
        let dialog = NewMapDialog::default();

        assert!(dialog.path.is_empty());
        assert_eq!(dialog.format, MapFormat::Tgm);
        assert_eq!((dialog.width, dialog.height, dialog.levels), (255, 255, 1));
        assert!(dialog.error.is_none());
    }

    #[test]
    fn the_new_map_dialog_accepts_dimensions_above_255() {
        let _guard = CONTEXT_LOCK.lock().unwrap();
        let directory = TempDir::new("large-map-dialog");
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env");
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        let fill = session.default_fill().unwrap();

        for (width, height, levels) in [(256, 1, 1), (1, 256, 1), (1, 1, 256)] {
            let mut context = rectangle_context();
            let path = directory.path().join(format!("map-{width}-{height}-{levels}.dmm"));
            let mut dialog = Some(NewMapDialog {
                path: path.to_str().unwrap().to_owned(),
                width,
                height,
                levels,
                ..NewMapDialog::default()
            });
            let mut created = false;

            click_dialog_button(&mut context, c"New map##new-map", &["Cancel", "Create Map"], |ui| {
                created |= draw_new_map_dialog(ui, &mut session, &mut dialog).1;
            });

            assert!(created);
            assert!(dialog.is_none());
            let document = session.state.active_document().unwrap();
            assert_eq!(document.path.as_deref(), Some(path.as_path()));
            assert_eq!(
                document.map.size(),
                Size {
                    x: width,
                    y: height,
                    z: levels,
                }
            );
            assert_eq!(document.map.tile_at(Coord::new(width, height, levels)), Some(&fill));
        }
    }

    #[test]
    fn the_new_map_dialog_preserves_dimensions_up_to_u32_max() {
        let _guard = CONTEXT_LOCK.lock().unwrap();
        let mut session = Session::new();

        for dimension in [i32::MAX as u32 + 1, u32::MAX] {
            let mut context = rectangle_context();
            let mut dialog = Some(NewMapDialog {
                width: dimension,
                height: dimension,
                levels: dimension,
                ..NewMapDialog::default()
            });

            let ui = context.frame();
            ui.open_popup(super::NEW_MAP_POPUP);
            assert_eq!(draw_new_map_dialog(ui, &mut session, &mut dialog), (false, false));
            assert!(context.render_legacy().valid());

            let state = dialog.unwrap();
            assert_eq!(
                (state.width, state.height, state.levels),
                (dimension, dimension, dimension)
            );
            assert!(session.state.active_document().is_none());
        }
    }

    #[test]
    fn new_map_paths_resolve_from_the_codebase_and_validate_the_target() {
        let root = std::env::temp_dir().join(format!("rmd-new-map-paths-{}", std::process::id()));
        let codebase = root.join("codebase");
        let maps = codebase.join("maps");
        std::fs::create_dir_all(&maps).unwrap();

        assert_eq!(
            resolve_new_map_path(&codebase, "maps/station").unwrap(),
            maps.join("station.dmm")
        );
        let outside = root.join("outside.dmm");
        assert_eq!(
            resolve_new_map_path(&codebase, &outside.display().to_string()).unwrap(),
            outside
        );
        assert!(
            resolve_new_map_path(&codebase, "station.txt")
                .unwrap_err()
                .contains(".dmm extension")
        );
        assert!(
            resolve_new_map_path(&codebase, "missing/station.dmm")
                .unwrap_err()
                .contains("does not exist")
        );

        let existing = maps.join("existing.dmm");
        std::fs::write(&existing, "existing").unwrap();
        assert!(
            resolve_new_map_path(&codebase, &existing.display().to_string())
                .unwrap_err()
                .contains("already exists")
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn confirming_deletion_removes_the_selected_level_and_can_be_undone() -> Result<(), Box<dyn Error>> {
        let _guard = CONTEXT_LOCK.lock().map_err(|_| "UI context lock is poisoned")?;
        let mut context = rectangle_context();
        let mut session = Session::new();
        let id = session
            .state
            .open_document(MapDocument::new(Map::new(Size { x: 2, y: 1, z: 3 }), 2));
        let mut dialog = Some(DeleteLevelDialog::new(
            session.state.document(id).ok_or("document is missing")?,
        ));
        draw_delete_level_dialog(context.frame(), &mut session, &mut dialog);
        assert!(context.render_legacy().valid());
        assert_eq!(session.level_count(), 3);
        context.io_mut().add_key_event(Key::Enter, true);
        draw_delete_level_dialog(context.frame(), &mut session, &mut dialog);
        assert!(context.render_legacy().valid());
        assert!(dialog.is_none());
        assert_eq!(session.level_count(), 2);
        assert_eq!(session.z(), 2);
        assert!(session.undo());
        assert_eq!(session.level_count(), 3);
        assert!(session.redo());
        assert_eq!(session.level_count(), 2);
        Ok(())
    }

    #[test]
    fn cancellation_or_a_changed_document_does_not_delete_a_level() -> Result<(), Box<dyn Error>> {
        let _guard = CONTEXT_LOCK.lock().map_err(|_| "UI context lock is poisoned")?;
        for stale in [false, true] {
            let mut context = rectangle_context();
            let mut session = Session::new();
            let id = session
                .state
                .open_document(MapDocument::new(Map::new(Size { x: 1, y: 1, z: 2 }), 2));
            let mut dialog = Some(DeleteLevelDialog::new(
                session.state.document(id).ok_or("document is missing")?,
            ));
            draw_delete_level_dialog(context.frame(), &mut session, &mut dialog);
            assert!(context.render_legacy().valid());
            if stale {
                let document = session.state.document_mut(id).ok_or("document is missing")?;
                let cells = document.level_cells(&LevelContents::Fill(patch::encode_tile(&[])));
                assert!(document.insert_remote_level(3, cells));
            }

            context
                .io_mut()
                .add_key_event(if stale { Key::Enter } else { Key::Escape }, true);
            draw_delete_level_dialog(context.frame(), &mut session, &mut dialog);
            assert!(context.render_legacy().valid());
            assert_eq!(session.level_count(), if stale { 3 } else { 2 });
            assert_eq!(dialog.is_some(), stale);
            assert!(
                !session
                    .state
                    .document(id)
                    .ok_or("document is missing")?
                    .history
                    .can_undo()
            );
        }

        Ok(())
    }

    #[test]
    fn read_only_and_last_level_restrictions_apply_at_confirmation() -> Result<(), Box<dyn Error>> {
        let _guard = CONTEXT_LOCK.lock().map_err(|_| "UI context lock is poisoned")?;
        for levels in [1, 2] {
            let mut context = rectangle_context();
            let mut session = Session::new();
            let id = session
                .state
                .open_document(MapDocument::new(Map::new(Size { x: 1, y: 1, z: levels }), levels));
            let mut dialog = Some(DeleteLevelDialog::new(
                session.state.document(id).ok_or("document is missing")?,
            ));
            draw_delete_level_dialog(context.frame(), &mut session, &mut dialog);
            assert!(context.render_legacy().valid());
            if levels == 2 {
                session
                    .state
                    .document_mut(id)
                    .ok_or("document is missing")?
                    .set_read_only(true);
            }

            context.io_mut().add_key_event(Key::Enter, true);
            draw_delete_level_dialog(context.frame(), &mut session, &mut dialog);
            assert!(context.render_legacy().valid());
            assert!(dialog.is_some());
            assert!(!session.can_delete_level(id));
            assert!(session.delete_level(id, levels).is_err());
            assert_eq!(session.level_count(), levels);
        }

        Ok(())
    }
}
