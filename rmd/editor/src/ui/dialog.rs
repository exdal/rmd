use core::path::TreePath;
use std::path::{Path, PathBuf};

use dear_imgui_rs::{DragFlags, Key, Ui, WindowFlags};
use dmm::{Coord, MapFormat, Prefab, Size};
use editor::{
    document::{DocumentId, MapDocument},
    icons::materialdesignicons::ICON_DOTS_HORIZONTAL,
    tool::{FillMode, SelectionMask, Tool},
};

use super::{
    DIAGNOSTIC_WARNING_COLOR,
    MAX_CUSTOM_FILL_SEARCH_RESULTS,
    UiState,
    common::{begin_centered_modal, dpi},
    draw_type_path_search,
};
use crate::{session::Session, settings::KeybindPreset};

pub(super) const MODAL_FLAGS: WindowFlags = WindowFlags::ALWAYS_AUTO_RESIZE
    .union(WindowFlags::NO_RESIZE)
    .union(WindowFlags::NO_MOVE)
    .union(WindowFlags::NO_COLLAPSE)
    .union(WindowFlags::NO_SAVED_SETTINGS)
    .union(WindowFlags::NO_DOCKING);

pub(super) const FILL_LIMIT_WARNING_POPUP: &str = "Large fill##fill-limit-warning";

pub(super) const NEW_MAP_POPUP: &str = "New map##new-map";

const NEW_LEVEL_POPUP: &str = "Create Z level##new-z-level";
const DELETE_LEVEL_POPUP: &str = "Delete Z level##delete-z-level";

pub(super) const RESIZE_MAP_POPUP: &str = "Resize map##resize-map";

pub(super) const GO_TO_POPUP: &str = "Go to coordinates##go-to";

pub(super) const DIALOG_FIELD_WIDTH: f32 = 320.0;

const NEW_MAP_PATH_WIDTH: f32 = 460.0;

const NEW_MAP_DEFAULT_WIDTH: i32 = 255;

const NEW_MAP_DEFAULT_HEIGHT: i32 = 255;

const NEW_MAP_DEFAULT_LEVELS: i32 = 1;

const NEW_MAP_MAX_DIMENSION: i32 = 255;

pub(super) const SAVE_MAP_POPUP: &str = "Save map##save-map";

const KEYBIND_PRESET_POPUP: &str = "Choose keybindings##keybind-preset";

const SAVE_MAP_PATH_WIDTH: f32 = 460.0;

pub(super) const SAVE_ERROR_COLOR: [f32; 4] = [1.0, 0.4, 0.4, 1.0];

const CLOSE_MAP_POPUP: &str = "Unsaved changes##close-map";

const EXIT_POPUP: &str = "Unsaved changes##exit";

#[derive(Debug, Clone, PartialEq)]
pub(super) struct PendingFillWarning {
    pub(super) context: FillWarningContext,
    pub(super) coord: Coord,
    pub(super) fill_mode: FillMode,
    pub(super) custom_fill_boundaries: Vec<TreePath>,
    pub(super) limit: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct FillWarningContext {
    document: Option<DocumentId>,
    revision: Option<u64>,
    z: u32,
    mask: Option<SelectionMask>,
    prefab: Option<Prefab>,
    focus: Option<dmm::PrefabInstanceId>,
}

impl FillWarningContext {
    pub(super) fn capture(session: &Session) -> Self {
        Self {
            document: session.state.active(),
            revision: session.edit_revision(),
            z: session.z(),
            mask: session.selection_mask(),
            prefab: session.palette().cloned(),
            focus: session.focused_area(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SaveDialog {
    pub(super) document: DocumentId,
    pub(super) close_after_save: bool,
    pub(super) path: String,
    pub(super) format: MapFormat,
    pub(super) error: Option<String>,
}

pub(super) enum SaveDialogOutcome {
    Saved {
        document: DocumentId,
        close_after_save: bool,
    },
    Cancelled {
        close_after_save: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NewMapDialog {
    pub(super) path: String,
    format: MapFormat,
    width: i32,
    height: i32,
    levels: i32,
    pub(super) error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct TileFillPaths {
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
pub(super) struct TileFillSearch {
    paths: TileFillPaths,
    query: String,
    searching: bool,
    was_searching: bool,
}

impl TileFillSearch {
    pub(super) fn new(session: &Session, remembered: Option<&TileFillPaths>) -> Self {
        Self {
            paths: remembered.cloned().unwrap_or_else(|| TileFillPaths::defaults(session)),
            ..Self::default()
        }
    }

    fn captures_keys(&self) -> bool { self.searching || self.was_searching }

    fn remember(&self, session: &Session, remembered: &mut Option<TileFillPaths>) {
        *remembered = (self.paths != TileFillPaths::defaults(session)).then(|| self.paths.clone());
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NewLevelDialog {
    pub(super) document: DocumentId,
    fill: TileFillSearch,
    error: Option<String>,
    open: bool,
}

impl NewLevelDialog {
    pub(super) fn new(document: DocumentId) -> Self {
        Self {
            document,
            fill: TileFillSearch::default(),
            error: None,
            open: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DeleteLevelDialog {
    pub(super) document: DocumentId,
    z: u32,
    // peers' tile edits don't matter, a level change does
    levels: u32,
    pub(super) open: bool,
    error: Option<String>,
}

impl DeleteLevelDialog {
    pub(super) fn new(document: &MapDocument) -> Self {
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
pub(super) struct ResizeMapDialog {
    width: i32,
    height: i32,
    fill: TileFillSearch,
    losses: Option<(i32, i32, Vec<Prefab>, usize)>,
    error: Option<String>,
}

impl ResizeMapDialog {
    pub(super) fn new(size: Size, fill: TileFillSearch) -> Self {
        Self {
            width: size.x as i32,
            height: size.y as i32,
            fill,
            losses: None,
            error: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct GoToDialog {
    x: i32,
    y: i32,
    z: i32,
}

impl GoToDialog {
    pub(super) fn new(target: Coord) -> Self {
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

pub(super) fn draw_keybind_preset_dialog(ui: &Ui, open: &mut bool) -> Option<KeybindPreset> {
    if !*open {
        return None;
    }
    if !ui.is_popup_open(KEYBIND_PRESET_POPUP) {
        ui.open_popup(KEYBIND_PRESET_POPUP);
    }

    let _modal = begin_centered_modal(ui, KEYBIND_PRESET_POPUP, MODAL_FLAGS)?;

    ui.text("Which keybinding preset would you prefer?");
    ui.text_disabled("You can customize individual bindings later in Settings.");
    ui.dummy([0.0, ui.frame_height() * 0.25]);

    let selected = if ui.button("Default") {
        Some(KeybindPreset::Default)
    } else {
        ui.same_line();
        ui.button("StrongDMM").then_some(KeybindPreset::StrongDmm)
    };

    if selected.is_some() {
        *open = false;
        ui.close_current_popup();
    }

    selected
}

pub(super) fn draw_new_map_dialog(ui: &Ui, session: &mut Session, dialog: &mut Option<NewMapDialog>) -> (bool, bool) {
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
        ui.drag_int_config(id)
            .range(1, NEW_MAP_MAX_DIMENSION)
            .flags(DragFlags::ALWAYS_CLAMP)
            .build(ui, value);
    }

    if let Some(error) = state.error.as_deref() {
        ui.text_colored(SAVE_ERROR_COLOR, error);
    }

    ui.separator();

    if ui.button("Cancel") || ui.is_key_pressed(Key::Escape) {
        close = true;
        ui.close_current_popup();
    }

    ui.same_line();

    let valid_dimensions = [state.width, state.height, state.levels]
        .into_iter()
        .all(|dimension| (1..=NEW_MAP_MAX_DIMENSION).contains(&dimension));
    let can_create = !state.path.trim().is_empty() && valid_dimensions;
    let clicked = {
        let _disabled = ui.begin_disabled_with_cond(!can_create);

        ui.button("Create")
    };

    if can_create && (clicked || submitted) {
        let result = session
            .codebase_dir()
            .ok_or_else(|| String::from("no codebase is loaded"))
            .and_then(|base| resolve_new_map_path(base, &state.path))
            .and_then(|path| {
                session
                    .create_map(
                        &path,
                        Size {
                            x: state.width as u32,
                            y: state.height as u32,
                            z: state.levels as u32,
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

pub(super) fn draw_resize_map_dialog(
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
        ui.drag_int_config(id)
            .range(1, NEW_MAP_MAX_DIMENSION)
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
            let losses = session.resize_losses(width as u32, height as u32, fill);
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
    ui.separator();

    let shortcuts = !state.fill.captures_keys();
    if ui.button("Cancel") || (shortcuts && ui.is_key_pressed(Key::Escape)) {
        close = true;
        ui.close_current_popup();
    }

    ui.same_line();
    let changed = (width as u32, height as u32) != (size.x, size.y);
    let clicked = {
        let _disabled = ui.begin_disabled_with_cond(!changed || fill.is_none());

        ui.button("Resize")
    };

    if changed
        && clicked
        && let Some(fill) = fill
    {
        match session.resize_map(width as u32, height as u32, &fill) {
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

pub(super) fn draw_go_to_dialog(ui: &Ui, session: &Session, dialog: &mut Option<GoToDialog>) -> Option<Coord> {
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

    ui.separator();

    if ui.button("Cancel") || ui.is_key_pressed(Key::Escape) {
        close = true;
        ui.close_current_popup();
    }

    ui.same_line();
    if ui.button("Go") || ui.is_key_pressed(Key::Enter) || ui.is_key_pressed(Key::KeypadEnter) {
        target = Some(Coord::new(state.x as u32, state.y as u32, state.z as u32));
        close = true;
        ui.close_current_popup();
    }

    if close {
        *dialog = None;
    }

    target
}

pub(super) fn draw_new_level_dialog(
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

    ui.separator();

    let shortcuts = !state.fill.captures_keys();
    if ui.button("Cancel") || (shortcuts && ui.is_key_pressed(Key::Escape)) {
        close = true;
        ui.close_current_popup();
    }

    ui.same_line();
    let clicked = {
        let _disabled = ui.begin_disabled_with_cond(fill.is_none());

        ui.button("Create")
    };

    let submitted = shortcuts && (ui.is_key_pressed(Key::Enter) || ui.is_key_pressed(Key::KeypadEnter));
    if (clicked || submitted)
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

pub(super) fn draw_delete_level_dialog(ui: &Ui, session: &mut Session, dialog: &mut Option<DeleteLevelDialog>) {
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

    ui.separator();
    if ui.button("Cancel") || ui.is_key_pressed(Key::Escape) {
        close = true;
        ui.close_current_popup();
    }

    ui.same_line();

    let clicked = {
        let _disabled = ui.begin_disabled_with_cond(!is_valid);
        ui.button("Delete")
    };

    if !close && is_valid && (clicked || ui.is_key_pressed(Key::Enter) || ui.is_key_pressed(Key::KeypadEnter)) {
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

pub(super) fn draw_save_dialog(
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

    ui.separator();

    if ui.button("Cancel") || ui.is_key_pressed(Key::Escape) {
        outcome = Some(SaveDialogOutcome::Cancelled {
            close_after_save: state.close_after_save,
        });
        ui.close_current_popup();
    }

    ui.same_line();

    let path = state.path.trim();
    let can_save = !path.is_empty();
    let clicked = {
        let _disabled = ui.begin_disabled_with_cond(!can_save);

        ui.button("Save")
    };

    if outcome.is_none() && can_save && (clicked || submitted) {
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

pub(super) fn draw_fill_limit_warning(
    ui: &Ui, session: &mut Session, pending: &mut Option<PendingFillWarning>, fill_mode: FillMode,
    boundaries: &[TreePath],
) {
    let Some(warning) = pending.as_ref() else {
        return;
    };

    let Some(_modal) = begin_centered_modal(ui, FILL_LIMIT_WARNING_POPUP, MODAL_FLAGS) else {
        return;
    };

    let mut fill_anyway = false;
    let mut dismiss = false;

    if !warning.matches(session, fill_mode, boundaries) {
        ui.close_current_popup();
        *pending = None;
        return;
    }

    ui.text(format!("This fill would change more than {} tiles.", warning.limit));
    ui.text("The operation may make the editor unresponsive.");
    ui.text("Do you want to fill it anyway?");
    ui.separator();

    if ui.button("Cancel") || ui.is_key_pressed(Key::Escape) {
        dismiss = true;
        ui.close_current_popup();
    }

    ui.same_line();
    if ui.button("Fill Anyway") {
        fill_anyway = true;
        dismiss = true;
        ui.close_current_popup();
    }

    if dismiss
        && let Some(warning) = pending.take()
        && fill_anyway
        && warning.matches(session, fill_mode, boundaries)
    {
        session.fill_at_unlimited(warning.coord, warning.fill_mode, &warning.custom_fill_boundaries);
    }
}

impl PendingFillWarning {
    fn matches(&self, session: &Session, fill_mode: FillMode, boundaries: &[TreePath]) -> bool {
        session.tool() == Tool::Fill
            && self.context == FillWarningContext::capture(session)
            && self.fill_mode == fill_mode
            && self.custom_fill_boundaries == boundaries
    }
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
        ui.dummy([0.0, ui.frame_height() * 0.25]);

        if ui.button("Cancel") || ui.is_key_pressed(Key::Escape) {
            self.exit_requested = false;
            ui.close_current_popup();

            return false;
        }
        ui.same_line();
        if ui.button("Exit without saving") {
            self.exit_requested = false;
            ui.close_current_popup();

            return true;
        }

        false
    }

    pub(super) fn request_close(&mut self, session: &mut Session, ids: impl IntoIterator<Item = DocumentId>) {
        for id in ids {
            if !session.state.document(id).is_some_and(MapDocument::is_dirty) {
                self.close_map_view(session, id);
            } else if !self.close_queue.contains(&id) {
                self.close_queue.push_back(id);
            }
        }
    }

    pub(super) fn draw_close_confirmation(&mut self, ui: &Ui, session: &mut Session) {
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
        ui.dummy([0.0, ui.frame_height() * 0.25]);

        if ui.button("Save") {
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
        ui.same_line();
        if ui.button("Discard") {
            self.close_map_view(session, id);
            self.close_queue.pop_front();
            ui.close_current_popup();
        }
        ui.same_line();
        if ui.button("Cancel") || ui.is_key_pressed(Key::Escape) {
            self.close_queue.clear();
            ui.close_current_popup();
        }
    }
}

#[cfg(test)]
mod tests {
    use core::path::TreePath;
    use std::{
        error::Error,
        ffi::{CStr, CString},
        fs,
        path::{Path, PathBuf},
    };

    use dear_imgui_rs::{ConfigFlags, Key, MouseButton, Ui, sys};
    use dmm::{Coord, Map, MapFormat, Prefab, Size};
    use editor::{
        document::{DocumentId, MapDocument, Selection},
        patch,
        tool::{BlockSelectionMode, FillMode, SelectionMask, Tool},
    };
    use net::LevelContents;

    use super::{
        DeleteLevelDialog,
        FillWarningContext,
        GO_TO_POPUP,
        GoToDialog,
        KEYBIND_PRESET_POPUP,
        NewLevelDialog,
        NewMapDialog,
        PendingFillWarning,
        RESIZE_MAP_POPUP,
        ResizeMapDialog,
        TileFillPaths,
        TileFillSearch,
        draw_delete_level_dialog,
        draw_go_to_dialog,
        draw_keybind_preset_dialog,
        draw_new_level_dialog,
        draw_resize_map_dialog,
        resolve_new_map_path,
    };
    use crate::{
        session::Session,
        settings::Settings,
        theme::{DEFAULT_THEME, Themes},
        ui::{
            IMGUI_CONTEXT,
            UiState,
            fixtures::{
                PopupContext,
                assert_window_centered,
                finish_frame,
                popup_context,
                rectangle_context,
                set_desktop_geometry,
            },
        },
    };

    struct CloseUi {
        context: PopupContext,
        state: UiState,
        session: Session,
        settings: Settings,
        themes: Themes,
        close_buttons: Vec<[f32; 2]>,
        save_buttons: Vec<[f32; 2]>,
        close_open: bool,
        save_open: bool,
    }

    impl CloseUi {
        fn new() -> Self {
            let mut context = popup_context(false);
            context.io_mut().set_display_size([1280.0, 720.0]);
            Self::with_context(context)
        }

        fn with_context(mut context: PopupContext) -> Self {
            let flags = context.io().config_flags() | ConfigFlags::DOCKING_ENABLE;
            context.io_mut().set_config_flags(flags);
            let themes = Themes::load(None, context.style(), DEFAULT_THEME);
            Self {
                context,
                state: UiState::new(false).unwrap(),
                session: Session::new(),
                settings: Settings {
                    check_for_updates: false,
                    focus_windows_on_hover: true,
                    ..Settings::default()
                },
                themes,
                close_buttons: Vec::new(),
                save_buttons: Vec::new(),
                close_open: false,
                save_open: false,
            }
        }

        fn add_map(&mut self, path: impl AsRef<Path>, dirty: bool, needs_path: bool) -> DocumentId {
            let path = path.as_ref().to_path_buf();
            let map = Map::new(Size { x: 4, y: 4, z: 1 });
            let mut document = if needs_path {
                MapDocument::create(path, map, 1)
            } else {
                MapDocument::open(path, map, 1)
            };
            if dirty {
                document.mark_unsaved();
            }
            self.session.state.open_document(document)
        }

        fn step(&mut self) {
            let ui = self.context.frame();
            self.state
                .draw(ui, &mut self.session, &mut self.settings, &mut self.themes, None)
                .unwrap();
            self.close_open = ui.is_popup_open("Unsaved changes##close-map");
            self.save_open = ui.is_popup_open("Save map##save-map");
            self.close_buttons = button_centers(ui, c"Unsaved changes##close-map", &["Save", "Discard", "Cancel"]);
            self.save_buttons = button_centers(ui, c"Save map##save-map", &["Cancel", "Save"]);
            finish_frame(&mut self.context);
        }

        fn settle(&mut self) {
            for _ in 0..4 {
                self.step();
            }
        }

        fn select(&mut self, id: DocumentId) {
            self.settle();
            self.session.set_active_document(id);
            self.state.map_views.get_mut(&id).unwrap().focus = true;
            self.settle();
            self.assert_selected_map(id);
        }

        fn click(&mut self, point: [f32; 2]) {
            for down in [false, true, false] {
                self.context.io_mut().add_mouse_pos_event(point);
                self.context.io_mut().add_mouse_button_event(MouseButton::Left, down);
                self.step();
            }
            self.settle();
        }

        fn close_tab(&mut self, id: DocumentId) {
            let keep_selected =
                self.session.state.active() == Some(id) && self.session.state.document(id).unwrap().is_dirty();
            let name = CString::new(format!("###viewport-{}", id.get())).unwrap();
            // SAFETY: this test owns the current context while holding IMGUI_CONTEXT.
            let point = unsafe {
                let window = &*sys::igFindWindowByName(name.as_ptr());
                let rect = window.DC.DockTabItemRect;
                let font_size = (*self.context.as_raw()).FontSize;
                [
                    rect.Max.x - self.context.style().frame_padding()[0] - font_size / 2.0,
                    (rect.Min.y + rect.Max.y) / 2.0,
                ]
            };
            for down in [false, true, false] {
                self.context.io_mut().add_mouse_pos_event(point);
                self.context.io_mut().add_mouse_button_event(MouseButton::Left, down);
                self.step();
                if keep_selected && self.state.close_queue.contains(&id) {
                    self.assert_selected_map(id);
                }
            }
            self.settle();
        }

        fn escape(&mut self) {
            self.context.io_mut().add_key_event(Key::Escape, true);
            self.step();
            self.context.io_mut().add_key_event(Key::Escape, false);
            self.settle();
        }

        fn assert_selected_map(&self, id: DocumentId) {
            assert_eq!(self.session.state.active(), Some(id));
            let name = CString::new(format!("###viewport-{}", id.get())).unwrap();
            self.assert_selected(&name);
        }

        fn assert_selected(&self, name: &CStr) {
            // SAFETY: the test owns the current context and its rendered docked windows.
            unsafe {
                let window = sys::igFindWindowByName(name.as_ptr());
                assert!(!window.is_null());
                let dock = (*window).DockNode;
                assert!(!dock.is_null(), "the tested window must be docked");
                assert!(
                    (*window).DockTabIsVisible(),
                    "the requested tab should be selected and visible"
                );
            }
        }
    }

    fn button_centers(ui: &Ui, name: &CStr, labels: &[&str]) -> Vec<[f32; 2]> {
        // SAFETY: this function runs during the test's current frame with IMGUI_CONTEXT held.
        let Some(window) = (unsafe { sys::igFindWindowByName(name.as_ptr()).as_ref() }) else {
            return Vec::new();
        };
        if !window.Active {
            return Vec::new();
        }
        let style = ui.clone_style();
        let mut x = window.Pos.x + style.window_padding()[0];
        let y = window.Pos.y + window.Size.y - style.window_padding()[1] - ui.frame_height() / 2.0;
        labels
            .iter()
            .map(|label| {
                let width = ui.calc_text_size(label)[0] + 2.0 * style.frame_padding()[0];
                let center = [x + width / 2.0, y];
                x += width + style.item_spacing()[0];
                center
            })
            .collect::<Vec<_>>()
    }

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("rmd-close-{name}-{}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
    }

    #[test]
    fn dirty_close_popup_recenters_after_leaving_fullscreen() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        for desktop in [false, true] {
            let context = popup_context(desktop);
            let mut app = CloseUi::with_context(context);
            set_desktop_geometry(&mut app.context, [0.0, 0.0], [1920.0, 1080.0]);
            let id = app.add_map("dirty.dmm", true, true);
            app.select(id);
            app.state.request_close(&mut app.session, [id]);
            app.settle();
            assert!(app.close_open);
            assert_window_centered(&mut app.context, c"Unsaved changes##close-map");
            app.escape();

            let position = if desktop { [180.0, 90.0] } else { [0.0, 0.0] };
            set_desktop_geometry(&mut app.context, position, [960.0, 640.0]);
            app.settle();
            app.state.request_close(&mut app.session, [id]);
            app.settle();
            assert!(app.close_open);
            assert_eq!(app.context.main_viewport().size(), [960.0, 640.0]);
            assert_eq!(app.context.main_viewport().pos(), position);
            assert_window_centered(&mut app.context, c"Unsaved changes##close-map");

            set_desktop_geometry(&mut app.context, position, [900.0, 600.0]);
            app.settle();
            assert_window_centered(&mut app.context, c"Unsaved changes##close-map");
            app.click(app.close_buttons[0]);
            assert!(app.save_open && !app.close_open);
            assert_window_centered(&mut app.context, c"Save map##save-map");
        }
    }

    #[test]
    fn dirty_tab_close_keeps_the_map_selected_until_cancel_or_escape() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut app = CloseUi::new();
        let id = app.add_map("dirty.dmm", true, false);
        app.select(id);
        for escape in [false, true] {
            app.close_tab(id);
            assert!(app.close_open);
            app.assert_selected_map(id);
            // SAFETY: this test owns the context and its active confirmation modal.
            unsafe {
                let focused = (*app.context.as_raw()).NavWindow;
                assert!(!focused.is_null());
                assert_ne!((*focused).Flags & sys::ImGuiWindowFlags_Modal, 0);
            }
            if escape {
                app.escape();
            } else {
                app.click(app.close_buttons[2]);
            }
            assert!(!app.close_open);
            assert!(app.state.close_queue.is_empty());
            assert!(app.session.state.document(id).unwrap().is_dirty());
            app.assert_selected_map(id);
        }
    }

    #[test]
    fn closing_an_inactive_dirty_tab_selects_that_map_for_confirmation() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut app = CloseUi::new();
        let dirty = app.add_map("dirty.dmm", true, false);
        let active = app.add_map("active.dmm", false, false);
        app.select(active);
        app.close_tab(dirty);
        assert!(app.close_open);
        app.assert_selected_map(dirty);
        app.escape();
        app.assert_selected_map(dirty);
        assert!(app.session.state.document(active).is_some());
    }

    #[test]
    fn clean_active_tab_closes_select_the_next_map_then_previous_then_welcome() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut app = CloseUi::new();
        let first = app.add_map("first.dmm", false, false);
        let second = app.add_map("second.dmm", false, false);
        let third = app.add_map("third.dmm", false, false);
        app.select(second);
        for (closing, remaining) in [(second, Some(third)), (third, Some(first)), (first, None)] {
            app.close_tab(closing);
            assert!(!app.close_open);
            assert!(app.session.state.document(closing).is_none());
            if let Some(id) = remaining {
                app.assert_selected_map(id);
            } else {
                app.assert_selected(c"###welcome");
            }
        }
    }

    #[test]
    fn discard_selects_the_next_map_then_the_previous_map_then_welcome() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut app = CloseUi::new();
        let first = app.add_map("first.dmm", true, false);
        let second = app.add_map("second.dmm", true, false);
        let third = app.add_map("third.dmm", true, false);
        app.select(second);
        for (closing, remaining) in [(second, Some(third)), (third, Some(first)), (first, None)] {
            app.close_tab(closing);
            assert!(app.close_open);
            app.assert_selected_map(closing);
            app.click(app.close_buttons[1]);
            assert!(app.session.state.document(closing).is_none());
            if let Some(id) = remaining {
                app.assert_selected_map(id);
            } else {
                app.assert_selected(c"###welcome");
            }
        }
    }

    #[test]
    fn closing_the_last_map_reopens_welcome_and_clean_background_closes_keep_focus() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut app = CloseUi::new();
        let background = app.add_map("background.dmm", false, false);
        let id = app.add_map("last.dmm", true, false);
        app.select(id);
        app.close_tab(background);
        assert!(app.session.state.document(background).is_none());
        app.assert_selected_map(id);
        app.state.show_welcome = false;
        app.settle();
        app.close_tab(id);
        app.assert_selected_map(id);
        app.click(app.close_buttons[1]);
        assert!(app.state.show_welcome);
        app.assert_selected(c"###welcome");
    }

    #[test]
    fn successful_save_closes_the_map_and_focuses_its_neighbor() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let dir = TestDirectory::new("save");
        let file = dir.0.join("saved.dmm");
        let mut app = CloseUi::new();
        let id = app.add_map(&file, true, false);
        let next = app.add_map("next.dmm", false, false);
        app.select(id);
        app.close_tab(id);
        app.assert_selected_map(id);
        app.click(app.close_buttons[0]);
        assert!(file.is_file());
        assert!(app.session.state.document(id).is_none());
        assert!(!app.close_open);
        app.assert_selected_map(next);
    }

    #[test]
    fn a_failed_save_keeps_the_confirmation_and_map_open() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let dir = TestDirectory::new("save-failure");
        let mut app = CloseUi::new();
        let id = app.add_map(&dir.0, true, false);
        app.select(id);
        app.close_tab(id);
        app.click(app.close_buttons[0]);
        assert!(app.close_open);
        assert!(app.state.open_error.is_some());
        assert!(app.session.state.document(id).unwrap().is_dirty());
        app.assert_selected_map(id);
        app.escape();
    }

    #[test]
    fn save_as_keeps_the_pending_close_until_it_succeeds() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let dir = TestDirectory::new("save-as");
        let file = dir.0.join("saved.dmm");
        let mut app = CloseUi::new();
        let id = app.add_map(&file, true, true);
        let next = app.add_map("next.dmm", false, false);
        app.select(id);
        app.close_tab(id);
        app.click(app.close_buttons[0]);
        assert!(app.save_open && !app.close_open);
        assert_eq!(app.state.close_queue.front(), Some(&id));
        assert!(app.state.save_dialog.as_ref().unwrap().close_after_save);
        app.assert_selected_map(id);

        app.state.save_dialog.as_mut().unwrap().path = dir.0.display().to_string();
        app.settle();
        app.click(app.save_buttons[1]);
        assert!(app.save_open);
        assert!(app.state.save_dialog.as_ref().unwrap().error.is_some());
        app.assert_selected_map(id);

        app.state.save_dialog.as_mut().unwrap().path = file.display().to_string();
        app.settle();
        // Save As must save its requesting document even if session activation changes.
        app.session.set_active_document(next);
        app.click(app.save_buttons[1]);
        assert!(file.is_file());
        assert!(app.session.state.document(id).is_none());
        assert!(app.state.close_queue.is_empty());
        assert!(app.state.save_dialog.is_none());
        app.assert_selected_map(next);
    }

    #[test]
    fn cancelling_save_as_cancels_the_close_batch_and_keeps_maps_open() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut app = CloseUi::new();
        let first = app.add_map("first.dmm", true, true);
        let second = app.add_map("second.dmm", true, true);
        app.select(first);
        app.state.request_close(&mut app.session, [first, second]);
        app.settle();
        assert_eq!(
            app.state.close_queue.iter().copied().collect::<Vec<_>>(),
            [first, second]
        );
        app.click(app.close_buttons[0]);
        assert!(app.save_open && !app.close_open);
        app.click(app.save_buttons[0]);
        assert!(!app.save_open && !app.close_open);
        assert!(app.state.close_queue.is_empty());
        for id in [first, second] {
            assert!(app.session.state.document(id).unwrap().is_dirty());
        }
        app.assert_selected_map(first);
    }

    #[test]
    fn save_as_success_advances_the_close_batch_to_the_next_dirty_map() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let dir = TestDirectory::new("save-as-batch");
        let file = dir.0.join("first.dmm");
        let mut app = CloseUi::new();
        let first = app.add_map(&file, true, true);
        let second = app.add_map("second.dmm", true, true);
        app.select(first);
        app.state.request_close(&mut app.session, [first, second]);
        app.settle();
        app.click(app.close_buttons[0]);
        app.click(app.save_buttons[1]);
        assert!(file.is_file());
        assert!(app.session.state.document(first).is_none());
        assert!(app.close_open && !app.save_open);
        assert_eq!(app.state.close_queue.front(), Some(&second));
        app.assert_selected_map(second);
        app.escape();
        assert!(app.session.state.document(second).unwrap().is_dirty());
    }

    #[test]
    fn pending_fill_is_invalidated_when_the_mask_document_or_palette_changes() {
        let mut session = Session::new();
        let size = Size { x: 10, y: 10, z: 2 };
        let id = session.state.open_document(MapDocument::new(dmm::Map::new(size), 1));
        session.set_tool(Tool::Fill);
        let mask = SelectionMask {
            bounds: Selection::from_drag(Coord::new(2, 2, 1), Coord::new(4, 4, 1)),
            mode: BlockSelectionMode::Full,
        };
        session.try_select_block(mask);
        let warning = PendingFillWarning {
            context: FillWarningContext::capture(&session),
            coord: mask.bounds.min,
            fill_mode: FillMode::Wall,
            custom_fill_boundaries: vec![],
            limit: 5000,
        };
        assert!(warning.matches(&session, FillMode::Wall, &[]));
        assert!(!warning.matches(&session, FillMode::EntireArea, &[]));
        assert!(!warning.matches(&session, FillMode::Wall, &[TreePath::parse("/obj/window")]));
        session.try_select_block(SelectionMask {
            mode: BlockSelectionMode::Hollow { line_width: 1 },
            ..mask
        });
        assert!(!warning.matches(&session, FillMode::Wall, &[]));
        session.try_select_block(mask);
        session.state.open_document(MapDocument::new(dmm::Map::new(size), 1));
        assert!(!warning.matches(&session, FillMode::Wall, &[]));
        session.state.set_active(id);
        session
            .state
            .choose_prefab(Prefab::new(TreePath::parse("/turf/open/floor")));
        assert!(!warning.matches(&session, FillMode::Wall, &[]));
    }

    #[test]
    fn enter_creates_a_z_level_filled_with_the_default_turf_and_area() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut context = crate::ui::fixtures::rectangle_context();
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
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut context = crate::ui::fixtures::rectangle_context();
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
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut context = crate::ui::fixtures::rectangle_context();
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
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut context = crate::ui::fixtures::rectangle_context();
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
    fn closing_several_maps_asks_about_each_unsaved_one_and_cancel_keeps_the_rest_open() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut context = crate::ui::fixtures::rectangle_context();
        let mut session = Session::new();
        let map = || dmm::Map::new(Size { x: 1, y: 1, z: 1 });
        let clean = session.state.open_document(MapDocument::new(map(), 1));
        let first = session.state.open_document(MapDocument::create("first.dmm", map(), 1));
        let second = session.state.open_document(MapDocument::create("second.dmm", map(), 1));
        let mut state = UiState::new(false).unwrap();

        let ids = session.state.document_ids();
        state.request_close(&mut session, ids);
        assert!(
            session.state.document(clean).is_none(),
            "a map without changes closes at once"
        );
        assert_eq!(Vec::from(state.close_queue.clone()), [first, second]);

        session.state.close_document(first);
        let ui = context.frame();
        state.draw_close_confirmation(ui, &mut session);
        assert!(context.render_legacy().valid());
        assert_eq!(
            Vec::from(state.close_queue.clone()),
            [second],
            "a map that went away leaves the queue"
        );

        context.io_mut().add_key_event(dear_imgui_rs::Key::Escape, true);
        let ui = context.frame();
        state.draw_close_confirmation(ui, &mut session);
        assert!(context.render_legacy().valid());
        assert!(state.close_queue.is_empty());
        assert!(session.state.document(second).is_some());
    }

    #[test]
    fn the_go_to_dialog_clamps_to_the_map_and_enter_returns_the_tile() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut context = crate::ui::fixtures::rectangle_context();
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
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut context = crate::ui::fixtures::rectangle_context();
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
    fn first_launch_opens_the_keybinding_preset_dialog() {
        let _context = IMGUI_CONTEXT.lock().unwrap();
        let mut context = dear_imgui_rs::Context::create();
        context
            .font_atlas()
            .try_claim_legacy_renderer()
            .expect("legacy renderer font atlas should be available")
            .build();
        context.io_mut().set_display_size([1280.0, 720.0]);
        context.io_mut().set_delta_time(1.0 / 60.0);
        let ui = context.frame();
        let mut open = true;

        assert_eq!(draw_keybind_preset_dialog(ui, &mut open), None);
        assert!(open);
        assert!(ui.is_popup_open(KEYBIND_PRESET_POPUP));
        assert!(context.render_legacy().valid());
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
        let _guard = IMGUI_CONTEXT.lock().map_err(|_| "UI context lock is poisoned")?;
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
        let _guard = IMGUI_CONTEXT.lock().map_err(|_| "UI context lock is poisoned")?;
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
        let _guard = IMGUI_CONTEXT.lock().map_err(|_| "UI context lock is poisoned")?;
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
