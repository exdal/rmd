use core::path::TreePath;
use std::path::{Path, PathBuf};

use dear_imgui_rs::{DragFlags, Key, Ui, WindowFlags};
use dmm::{Coord, MapFormat, Prefab, Size};
use editor::{
    document::{DocumentId, MapDocument},
    icons::materialdesignicons::ICON_DOTS_HORIZONTAL,
    tool::{FillMode, SelectionMask, Tool},
};

use super::{DIAGNOSTIC_WARNING_COLOR, UiState};
use crate::{session::Session, settings::KeybindPreset};

pub(super) const FILL_LIMIT_WARNING_POPUP: &str = "Large fill##fill-limit-warning";

pub(super) const NEW_MAP_POPUP: &str = "New map##new-map";

const NEW_LEVEL_POPUP: &str = "Create Z level##new-z-level";

pub(super) const RESIZE_MAP_POPUP: &str = "Resize map##resize-map";

const RESIZE_MAP_WIDTH: f32 = 320.0;

const NEW_MAP_PATH_WIDTH: f32 = 460.0;

const NEW_MAP_DEFAULT_WIDTH: i32 = 255;

const NEW_MAP_DEFAULT_HEIGHT: i32 = 255;

const NEW_MAP_DEFAULT_LEVELS: i32 = 1;

const NEW_MAP_MAX_DIMENSION: i32 = 255;

pub(super) const SAVE_MAP_POPUP: &str = "Save map##save-map";

const KEYBIND_PRESET_POPUP: &str = "Choose keybindings##keybind-preset";

const SAVE_MAP_PATH_WIDTH: f32 = 460.0;

pub(super) const SAVE_ERROR_COLOR: [f32; 4] = [1.0, 0.4, 0.4, 1.0];

pub(super) const CLOSE_MAP_POPUP: &str = "Unsaved changes##close-map";

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
    pub(super) path: String,
    pub(super) format: MapFormat,
    pub(super) error: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NewLevelDialog {
    pub(super) document: DocumentId,
    pub(super) error: Option<String>,
    pub(super) open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ResizeMapDialog {
    width: i32,
    height: i32,
    losses: Option<(i32, i32, usize)>,
    error: Option<String>,
}

impl ResizeMapDialog {
    pub(super) fn new(size: Size) -> Self {
        Self {
            width: size.x as i32,
            height: size.y as i32,
            losses: None,
            error: None,
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

    let flags = WindowFlags::ALWAYS_AUTO_RESIZE
        | WindowFlags::NO_RESIZE
        | WindowFlags::NO_MOVE
        | WindowFlags::NO_COLLAPSE
        | WindowFlags::NO_SAVED_SETTINGS
        | WindowFlags::NO_DOCKING;
    let _modal = ui.begin_modal_popup_config(KEYBIND_PRESET_POPUP).flags(flags).begin()?;

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
    let flags = WindowFlags::ALWAYS_AUTO_RESIZE
        | WindowFlags::NO_RESIZE
        | WindowFlags::NO_MOVE
        | WindowFlags::NO_COLLAPSE
        | WindowFlags::NO_SAVED_SETTINGS
        | WindowFlags::NO_DOCKING;
    let mut close = false;
    let mut pick_path = false;
    let mut created = false;

    if let Some(state) = dialog.as_mut()
        && let Some(_modal) = ui.begin_modal_popup_config(NEW_MAP_POPUP).flags(flags).begin()
    {
        ui.text("Path");
        let button_size = ui.frame_height();
        let spacing = ui.clone_style().item_spacing()[0];
        ui.set_next_item_width(NEW_MAP_PATH_WIDTH - button_size - spacing);
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
            ui.set_next_item_width(NEW_MAP_PATH_WIDTH);
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
    }

    if close {
        *dialog = None;
    }

    (pick_path, created)
}

/// Returns whether the map was resized
pub(super) fn draw_resize_map_dialog(ui: &Ui, session: &mut Session, dialog: &mut Option<ResizeMapDialog>) -> bool {
    let flags = WindowFlags::ALWAYS_AUTO_RESIZE
        | WindowFlags::NO_RESIZE
        | WindowFlags::NO_MOVE
        | WindowFlags::NO_COLLAPSE
        | WindowFlags::NO_SAVED_SETTINGS
        | WindowFlags::NO_DOCKING;
    let mut close = false;
    let mut resized = false;

    if let Some(state) = dialog.as_mut()
        && let Some(_modal) = ui.begin_modal_popup_config(RESIZE_MAP_POPUP).flags(flags).begin()
    {
        let Some(size) = session.map().map(|map| map.size) else {
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
            ui.set_next_item_width(RESIZE_MAP_WIDTH);
            ui.drag_int_config(id)
                .range(1, NEW_MAP_MAX_DIMENSION)
                .flags(DragFlags::ALWAYS_CLAMP)
                .build(ui, value);
        }

        let wrap = ui.push_text_wrap_pos(ui.cursor_pos()[0] + RESIZE_MAP_WIDTH);
        let fill = session.default_fill().map(|fill| fill_label(&fill)).unwrap_or_default();
        ui.text_disabled(format!("New tiles be filled with {fill}."));
        let (width, height) = (state.width, state.height);
        let losses = match state.losses {
            Some((cached_width, cached_height, losses)) if (cached_width, cached_height) == (width, height) => losses,
            _ => {
                let losses = session.resize_losses(width as u32, height as u32);
                state.losses = Some((width, height, losses));
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

        if ui.button("Cancel") || ui.is_key_pressed(Key::Escape) {
            close = true;
            ui.close_current_popup();
        }
        ui.same_line();
        let changed = (width as u32, height as u32) != (size.x, size.y);
        let clicked = {
            let _disabled = ui.begin_disabled_with_cond(!changed);

            ui.button("Resize")
        };
        if changed && clicked {
            match session.resize_map(width as u32, height as u32) {
                Ok(()) => {
                    resized = true;
                    close = true;
                    ui.close_current_popup();
                },
                Err(error) => state.error = Some(error),
            }
        }
    }

    if close {
        *dialog = None;
    }

    resized
}

pub(super) fn draw_new_level_dialog(ui: &Ui, session: &mut Session, dialog: &mut Option<NewLevelDialog>) {
    let flags = WindowFlags::ALWAYS_AUTO_RESIZE
        | WindowFlags::NO_RESIZE
        | WindowFlags::NO_MOVE
        | WindowFlags::NO_COLLAPSE
        | WindowFlags::NO_SAVED_SETTINGS
        | WindowFlags::NO_DOCKING;
    let mut close = false;

    if let Some(state) = dialog.as_mut()
        && state.open
    {
        ui.open_popup(NEW_LEVEL_POPUP);
        state.open = false;
    }

    if let Some(state) = dialog.as_mut()
        && let Some(_modal) = ui.begin_modal_popup_config(NEW_LEVEL_POPUP).flags(flags).begin()
    {
        if let Some(document) = session.state.document(state.document) {
            ui.text(format!("Create Z level {}", document.map.size.z.saturating_add(1)));
        }
        match session.default_fill() {
            Ok(fill) => ui.text_disabled(format!("New tiles will be filled with {}.", fill_label(&fill))),
            Err(error) => ui.text_colored(SAVE_ERROR_COLOR, error),
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
        if ui.button("Create") || ui.is_key_pressed(Key::Enter) || ui.is_key_pressed(Key::KeypadEnter) {
            match session.create_level(state.document) {
                Ok(_) => {
                    close = true;
                    ui.close_current_popup();
                },
                Err(error) => state.error = Some(error),
            }
        }
    }

    if close {
        *dialog = None;
    }
}

/// `/turf/open/space and /area/space`
fn fill_label(fill: &[Prefab]) -> String {
    fill.iter()
        .map(|prefab| prefab.path.to_string())
        .collect::<Vec<_>>()
        .join(" and ")
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

pub(super) fn draw_save_dialog(ui: &Ui, session: &mut Session, dialog: &mut Option<SaveDialog>) {
    let flags = WindowFlags::ALWAYS_AUTO_RESIZE
        | WindowFlags::NO_RESIZE
        | WindowFlags::NO_MOVE
        | WindowFlags::NO_COLLAPSE
        | WindowFlags::NO_SAVED_SETTINGS
        | WindowFlags::NO_DOCKING;
    let mut close = false;

    if let Some(state) = dialog.as_mut()
        && let Some(_modal) = ui.begin_modal_popup_config(SAVE_MAP_POPUP).flags(flags).begin()
    {
        ui.text("Path");
        ui.set_next_item_width(SAVE_MAP_PATH_WIDTH);
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
            close = true;
            ui.close_current_popup();
        }
        ui.same_line();

        let path = state.path.trim();
        let can_save = !path.is_empty();
        let clicked = {
            let _disabled = ui.begin_disabled_with_cond(!can_save);

            ui.button("Save")
        };

        if can_save && (clicked || submitted) {
            match session.save_map_as(Path::new(path), state.format) {
                Ok(()) => {
                    close = true;
                    ui.close_current_popup();
                },
                Err(error) => state.error = Some(error.to_string()),
            }
        }
    }

    if close {
        *dialog = None;
    }
}

pub(super) fn draw_fill_limit_warning(
    ui: &Ui, session: &mut Session, pending: &mut Option<PendingFillWarning>, fill_mode: FillMode,
    boundaries: &[TreePath],
) {
    let mut fill_anyway = false;
    let mut dismiss = false;

    let flags = WindowFlags::ALWAYS_AUTO_RESIZE
        | WindowFlags::NO_RESIZE
        | WindowFlags::NO_MOVE
        | WindowFlags::NO_COLLAPSE
        | WindowFlags::NO_SAVED_SETTINGS
        | WindowFlags::NO_DOCKING;
    if let Some(warning) = pending.as_ref()
        && let Some(_modal) = ui
            .begin_modal_popup_config(FILL_LIMIT_WARNING_POPUP)
            .flags(flags)
            .begin()
    {
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
        let flags = WindowFlags::ALWAYS_AUTO_RESIZE
            | WindowFlags::NO_RESIZE
            | WindowFlags::NO_MOVE
            | WindowFlags::NO_COLLAPSE
            | WindowFlags::NO_SAVED_SETTINGS
            | WindowFlags::NO_DOCKING;
        let Some(_modal) = ui.begin_modal_popup_config(EXIT_POPUP).flags(flags).begin() else {
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

    pub(super) fn draw_close_confirmation(&mut self, ui: &Ui, session: &mut Session) {
        let Some(id) = self.pending_close else {
            return;
        };
        let Some(title) = session.state.document(id).map(MapDocument::title) else {
            self.pending_close = None;

            return;
        };

        let flags = WindowFlags::ALWAYS_AUTO_RESIZE
            | WindowFlags::NO_RESIZE
            | WindowFlags::NO_MOVE
            | WindowFlags::NO_COLLAPSE
            | WindowFlags::NO_SAVED_SETTINGS
            | WindowFlags::NO_DOCKING;

        let Some(_modal) = ui.begin_modal_popup_config(CLOSE_MAP_POPUP).flags(flags).begin() else {
            return;
        };
        let writable = session
            .state
            .document(id)
            .is_some_and(|document| document.path.is_some() && !document.needs_initial_save());

        ui.text(format!("{} has unsaved changes.", title.trim_end_matches(" *")));
        ui.dummy([0.0, ui.frame_height() * 0.25]);

        if ui.button("Save") {
            session.set_active_document(id);
            match session.map_path().map(Path::to_path_buf).filter(|_| writable) {
                Some(path) => {
                    let format = session.map_format().unwrap_or_default();
                    match session.save_map_as(&path, format) {
                        Ok(()) => {
                            session.close_map(id);
                            self.pending_close = None;
                            ui.close_current_popup();
                        },
                        Err(e) => self.open_error = Some(e.to_string()),
                    }
                },
                None => {
                    self.save_dialog = Some(SaveDialog {
                        path: session
                            .map_path()
                            .map(|path| path.display().to_string())
                            .unwrap_or_default(),
                        format: session.map_format().unwrap_or_default(),
                        error: None,
                    });
                    self.pending_close = None;
                    ui.close_current_popup();
                    ui.open_popup(SAVE_MAP_POPUP);
                },
            }
        }
        ui.same_line();
        if ui.button("Discard") {
            session.close_map(id);
            self.pending_close = None;
            ui.close_current_popup();
        }
        ui.same_line();
        if ui.button("Cancel") || ui.is_key_pressed(Key::Escape) {
            self.pending_close = None;
            ui.close_current_popup();
        }
    }
}

#[cfg(test)]
mod tests {
    use core::path::TreePath;

    use dmm::{Coord, MapFormat, Prefab, Size};
    use editor::{
        document::{MapDocument, Selection},
        tool::{BlockSelectionMode, FillMode, SelectionMask, Tool},
    };

    use super::{
        FillWarningContext,
        KEYBIND_PRESET_POPUP,
        NewLevelDialog,
        NewMapDialog,
        PendingFillWarning,
        RESIZE_MAP_POPUP,
        ResizeMapDialog,
        draw_keybind_preset_dialog,
        draw_new_level_dialog,
        draw_resize_map_dialog,
        resolve_new_map_path,
    };
    use crate::{session::Session, ui::IMGUI_CONTEXT};

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
        let mut dialog = Some(NewLevelDialog {
            document,
            error: None,
            open: true,
        });

        let ui = context.frame();
        draw_new_level_dialog(ui, &mut session, &mut dialog);
        assert!(context.render_legacy().valid());
        assert_eq!(session.level_count(), 1, "opening the dialog creates nothing");

        context.io_mut().add_key_event(dear_imgui_rs::Key::Enter, true);
        let ui = context.frame();
        draw_new_level_dialog(ui, &mut session, &mut dialog);
        assert!(context.render_legacy().valid());

        assert!(dialog.is_none());
        assert_eq!(session.level_count(), 2);
        assert_eq!(
            session.map().unwrap().tile_at(Coord::new(2, 1, 2)),
            session.default_fill().ok().as_ref()
        );
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
        let mut dialog = Some(ResizeMapDialog::new(Size { x: 3, y: 1, z: 1 }));

        for width in [3, 1] {
            dialog.as_mut().unwrap().width = width;
            let ui = context.frame();
            ui.open_popup(RESIZE_MAP_POPUP);
            assert!(!draw_resize_map_dialog(ui, &mut session, &mut dialog));
            assert!(context.render_legacy().valid());
        }

        assert_eq!(dialog.unwrap().losses, Some((1, 1, 2)));
        assert_eq!(
            session.map().unwrap().size.x,
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
}
