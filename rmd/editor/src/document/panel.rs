use std::path::{Path, PathBuf};

use base::coord::Coord;
use dear_imgui_rs::{DragFlags, Ui};
use dmm::{MapFormat, Size};
use prefab::Prefab;
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
mod tests;
