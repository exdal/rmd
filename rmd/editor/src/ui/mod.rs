mod inspector;
mod object_tree;
mod settings;

use core::path::TreePath;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use dear_imgui_rs::{
    Condition,
    DockLayout,
    DockLayoutApply,
    DockNodeFlags,
    DockSplit,
    DockspaceError,
    DragFlags,
    DrawListMut,
    Id,
    InputTextMultilineFlags,
    Key,
    MouseButton,
    StyleColor,
    StyleVar,
    Ui,
    WindowFlags,
    WindowHoveredFlags,
    WindowKey,
    WindowKeyError,
};
use dmm::{Coord, MapFormat, Prefab, Size};
use editor::{
    command::EditGroupId,
    document::{DocumentId, MapDocument, Selection},
    icons::materialdesignicons::{
        ICON_CLOSE_THICK,
        ICON_DOTS_HORIZONTAL,
        ICON_ERASER,
        ICON_EYEDROPPER,
        ICON_FORMAT_COLOR_FILL,
        ICON_IMAGE_BROKEN,
        ICON_MENU_DOWN,
        ICON_PENCIL,
        ICON_SELECT_DRAG,
    },
    progress::{Snapshot, Stage},
    tool::{
        BlockSelectionMode,
        FillMode,
        SelectionMask,
        SelectionPlacement,
        SelectionRotation,
        SelectionTransform,
        Tool,
        rotated_selection_at,
    },
};
pub(crate) use inspector::TransformMode;
use objtree::ObjectTree;
use render::{Camera, InteractionMode, MapViewInteraction, MapViewRect, PickRequest, PlacementFlash, Renderer};

use self::{
    inspector::{InspectorPanel, JumpTarget},
    object_tree::ObjectTreePanel,
    settings::SettingsWindow,
};
use crate::{
    camera::Controller,
    external_editor::SourceLocation,
    gizmo::{BlockGizmoKind, BlockGizmoTarget, GizmoMapView, GizmoState},
    loader::LoadView,
    session::{BlockPreviewSource, FillOutcome, LevelChange, PlacementPreview, SelectedTransform, Session},
    settings::{KeyBindings, KeybindAction, KeybindPreset, Settings},
};

/// How far down the "Blur below" menu goes. The option itself takes any depth.
const MAX_UNDERLAY_DEPTH: u32 = 3;

const DOCKSPACE_ID: &str = "rmd-main-dockspace-v3";
const OVERLAY_PADDING: f32 = 4.0;
const OVERLAY_BG: [f32; 4] = [0.0, 0.0, 0.0, 0.55];
const RECENT_ICON_SIZE: f32 = 48.0;
const RECENT_BADGE_BG: [f32; 4] = [0.0, 0.0, 0.0, 0.8];
const RECENT_BADGE_TEXT: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const PLACEMENT_FLASH_DURATION: f64 = 0.25;
const PLACEMENT_PREVIEW_PERIOD: f64 = 1.5;
const DEFAULT_CUSTOM_FILL_BOUNDARY: &str = "/turf/closed/wall";
const MAX_CUSTOM_FILL_SEARCH_RESULTS: usize = 50;
const BLOCK_PLACEMENT_LABELS: [&str; 4] = ["Move", "Copy", "Fill selection", "Clear selection"];
const PASTE_LABELS: [&str; 2] = ["Paste", "Cancel"];
const FILL_LIMIT_WARNING_POPUP: &str = "Large fill##fill-limit-warning";
const NEW_MAP_POPUP: &str = "New map##new-map";
const NEW_LEVEL_POPUP: &str = "Create Z level##new-z-level";
const NEW_MAP_PATH_WIDTH: f32 = 460.0;
const NEW_LEVEL_PATH_WIDTH: f32 = 420.0;
const NEW_LEVEL_SEARCH_HEIGHT: f32 = 180.0;
const NEW_MAP_DEFAULT_WIDTH: i32 = 255;
const NEW_MAP_DEFAULT_HEIGHT: i32 = 255;
const NEW_MAP_DEFAULT_LEVELS: i32 = 1;
const NEW_MAP_MAX_DIMENSION: i32 = 255;
const SAVE_MAP_POPUP: &str = "Save map##save-map";
const KEYBIND_PRESET_POPUP: &str = "Choose keybindings##keybind-preset";
const LOAD_POPUP_WIDTH: f32 = 420.0;
const LOAD_TEXT_WIDTH: f32 = 620.0;
const LOAD_DIAGNOSTICS_LINES: usize = 14;
const LOAD_TEXT_MIN_LINES: usize = 4;
const LOAD_ERROR_MAX_LINES: usize = 10;
const LOAD_PATH_MAX_CHARS: usize = 60;
const SAVE_MAP_PATH_WIDTH: f32 = 460.0;
const SAVE_ERROR_COLOR: [f32; 4] = [1.0, 0.4, 0.4, 1.0];
const WELCOME_TITLE_SIZE: f32 = 40.0;
const WELCOME_CONTENT_WIDTH: f32 = 640.0;
const WELCOME_MIN_INDENT: f32 = 24.0;
const WELCOME_MAP_PREVIEW: usize = 10;
const BUILD_VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"));
const BUILD_GIT_SHORT_HASH: Option<&str> = option_env!("RMD_GIT_SHORT_HASH");
const BUILD_VERSION_URL: Option<&str> = option_env!("RMD_VERSION_URL");
const BUILD_COMMIT_URL: Option<&str> = option_env!("RMD_COMMIT_URL");
const CLOSE_MAP_POPUP: &str = "Unsaved changes##close-map";
const EXIT_POPUP: &str = "Unsaved changes##exit";
const BLOCK_SELECTION_POPUP: &str = "Block selection##block-selection";
const BLOCK_SELECTION_GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const BLOCK_SELECTION_WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const BLOCK_SELECTION_SHADOW: [f32; 4] = [0.0, 0.0, 0.0, 0.85];
const BLOCK_STRIPE_LENGTH: f32 = 6.0;
const BLOCK_STRIPE_SPEED: f32 = 12.0;

#[derive(Debug, Clone, Copy, PartialEq)]
struct ActivePlacementFlash {
    owner: dmm::PrefabInstanceId,
    coord: Coord,
    started_at: f64,
}

impl ActivePlacementFlash {
    fn sample(self, now: f64) -> Option<PlacementFlash> {
        let elapsed = (now - self.started_at).max(0.0);
        if !elapsed.is_finite() || elapsed >= PLACEMENT_FLASH_DURATION {
            return None;
        }

        Some(PlacementFlash {
            owner: self.owner,
            strength: (1.0 - elapsed / PLACEMENT_FLASH_DURATION) as f32,
        })
    }
}

#[derive(Debug)]
struct PlacementStroke {
    prefab: Prefab,
    z: u32,
    group: EditGroupId,
    visited: HashSet<Coord>,
}

impl PlacementStroke {
    fn new(prefab: Prefab, z: u32) -> Self {
        Self {
            prefab,
            z,
            group: EditGroupId::new(),
            visited: HashSet::new(),
        }
    }

    fn matches_context(&self, tool: Tool, prefab: Option<&Prefab>, z: u32) -> bool {
        tool == Tool::Place && prefab == Some(&self.prefab) && z == self.z
    }

    fn visit(&mut self, coord: Coord) -> Option<EditGroupId> {
        (coord.z == self.z && self.visited.insert(coord)).then_some(self.group)
    }
}

#[derive(Debug)]
struct DeletionStroke {
    cursor: [u32; 2],
}

impl DeletionStroke {
    const fn new(cursor: [u32; 2]) -> Self { Self { cursor } }

    fn move_to(&mut self, cursor: [u32; 2]) -> bool {
        let moved = self.cursor != cursor;
        self.cursor = cursor;

        moved
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BlockSelectionOptions {
    full_rectangle: bool,
    line_width: i32,
}

impl Default for BlockSelectionOptions {
    fn default() -> Self {
        Self {
            full_rectangle: true,
            line_width: 1,
        }
    }
}

impl BlockSelectionOptions {
    fn drawing_mode(self, shift: bool) -> BlockSelectionMode { if shift { self.hollow() } else { self.mode() } }

    fn mode(self) -> BlockSelectionMode {
        if self.full_rectangle {
            BlockSelectionMode::Full
        } else {
            self.hollow()
        }
    }

    fn hollow(self) -> BlockSelectionMode {
        BlockSelectionMode::Hollow {
            line_width: self.line_width.max(1) as u32,
        }
    }

    fn label(self) -> String {
        match self.mode() {
            BlockSelectionMode::Full => String::from("Full rectangle"),
            BlockSelectionMode::Hollow { line_width: 1 } => String::from("Border, 1 tile"),
            BlockSelectionMode::Hollow { line_width } => format!("Border, {line_width} tiles"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct PendingFillWarning {
    context: FillWarningContext,
    coord: Coord,
    fill_mode: FillMode,
    custom_fill_boundaries: Vec<TreePath>,
    limit: usize,
}

#[derive(Debug, Clone, PartialEq)]
struct FillWarningContext {
    document: Option<DocumentId>,
    revision: Option<u64>,
    z: u32,
    mask: Option<SelectionMask>,
    prefab: Option<Prefab>,
    focus: Option<dmm::PrefabInstanceId>,
}

impl FillWarningContext {
    fn capture(session: &Session) -> Self {
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
struct SaveDialog {
    path: String,
    format: MapFormat,
    error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NewMapDialog {
    path: String,
    format: MapFormat,
    width: i32,
    height: i32,
    levels: i32,
    error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NewLevelDialog {
    document: DocumentId,
    type_path: String,
    error: Option<String>,
    open: bool,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingBlockPlacement {
    source: Selection,
    target: Selection,
    rotation: SelectionRotation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RectangleGesture {
    start: Option<SelectionMask>,
    z: u32,
}

fn restore_rectangle_gesture(session: &mut Session, id: DocumentId, gesture: &mut Option<RectangleGesture>) {
    if let Some(gesture) = gesture.take()
        && let Some(document) = session.state.document_mut(id)
        && document.z == gesture.z
    {
        document.selection = gesture.start.map(|mask| mask.bounds);
        document.selection_mode = gesture.start.map_or(BlockSelectionMode::Full, |mask| mask.mode);
    }
}

fn rectangle_drag_coord(
    camera: &Controller, mouse: [f32; 2], viewport_min: [f32; 2], size: Size, tile_size: u32, z: u32,
) -> Option<Coord> {
    let point = camera.screen_to_map([mouse[0] - viewport_min[0], mouse[1] - viewport_min[1]]);
    if !point.iter().all(|value| value.is_finite()) || size.x == 0 || size.y == 0 {
        return None;
    }
    let tile_size = tile_size.max(1) as f32;
    Some(Coord::new(
        ((point[0] / tile_size).floor() as i64)
            .saturating_add(1)
            .clamp(1, i64::from(size.x)) as u32,
        ((point[1] / tile_size).floor() as i64)
            .saturating_add(1)
            .clamp(1, i64::from(size.y)) as u32,
        z,
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockPlacementAction {
    Move,
    Copy,
    Fill,
    Cancel,
}

#[derive(Clone, Copy)]
enum PlacementControls {
    Block,
    Paste,
}

impl PlacementControls {
    fn labels(self) -> &'static [&'static str] {
        match self {
            Self::Block => &BLOCK_PLACEMENT_LABELS,
            Self::Paste => &PASTE_LABELS,
        }
    }

    fn rows(self) -> usize {
        match self {
            Self::Block => 3,
            Self::Paste => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingPaste {
    /// lower left corner of the footprint, in destination-map tiles
    min: Coord,
    rotation: SelectionRotation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PasteAction {
    Paste,
    Cancel,
}

pub(super) fn focus_window_on_hover(ui: &Ui) {
    if !ui.is_window_focused()
        && !ui.is_any_item_active()
        && ui.is_window_hovered_with_flags(WindowHoveredFlags::CHILD_WINDOWS)
    {
        ui.set_window_focus(None);
    }
}

fn draw_overlay_underlay(ui: &Ui, bounds: OverlayRect) {
    ui.get_window_draw_list()
        .add_rect(bounds.min, bounds.max, OVERLAY_BG)
        .filled(true)
        .build();
}

pub struct VisibleMapView {
    pub document: DocumentId,
    pub rect: MapViewRect,
    pub camera: Camera,
    pub interaction: MapViewInteraction,
}

pub struct UiOutput {
    pub exit: bool,
    pub map_views: Vec<VisibleMapView>,
    pub picking: Option<usize>,
    pub open: Option<OpenRequest>,
    pub open_source: Option<SourceLocation>,
    pub pick_new_map_path: bool,
    pub cancel_load: bool,
    pub copy_to_clipboard: Option<String>,
    pub(crate) keybind_preset: Option<KeybindPreset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ForgetRequest {
    Codebase(PathBuf),
    Map(PathBuf),
}

#[derive(Debug, Default, PartialEq, Eq)]
struct WelcomeOutput {
    open: Option<OpenRequest>,
    new_map_dialog: bool,
    forget: Option<ForgetRequest>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct RecentEntry {
    open: bool,
    forget: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenRequest {
    PickCodebase,
    PickMap,
    Codebase(PathBuf),
    Map(PathBuf),
}

struct MapViewState {
    window: WindowKey,
    camera: Controller,
    size: (u32, u32),
    rect: MapViewRect,
    visible: bool,
    refit: bool,
    focus: bool,
    block_selection_anchor: Option<Coord>,
    rectangle_gesture: Option<RectangleGesture>,
    block_placement: Option<PendingBlockPlacement>,
    paste: Option<PendingPaste>,
}

struct MapViewDraw<'a> {
    id: DocumentId,
    index: usize,
    view: &'a mut MapViewState,
    interaction: &'a mut MapViewInteraction,
    refit_requested: bool,
    keep_open: &'a mut bool,
}

impl MapViewState {
    fn new(id: DocumentId) -> Result<Self, WindowKeyError> {
        Ok(Self {
            window: WindowKey::new(format!("viewport-{}", id.get()), "Map View")?,
            camera: Controller::new(),
            size: (1, 1),
            rect: MapViewRect::default(),
            visible: false,
            refit: true,
            focus: false,
            block_selection_anchor: None,
            rectangle_gesture: None,
            block_placement: None,
            paste: None,
        })
    }
}

pub struct UiState {
    object_tree: ObjectTreePanel,
    map_views: HashMap<DocumentId, MapViewState>,
    central_node: Option<Id>,
    dockspace_root: Option<Id>,
    welcome_window: WindowKey,
    inspector: InspectorPanel,
    settings_window: SettingsWindow,
    layout: DockLayout,
    gizmo: GizmoState,
    gizmo_context: Option<(DocumentId, Tool, u32)>,
    placement_flash: Option<ActivePlacementFlash>,
    placement_stroke: Option<PlacementStroke>,
    deletion_stroke: Option<DeletionStroke>,
    block_selection_options: BlockSelectionOptions,
    fill_mode: FillMode,
    custom_fill_boundaries: Vec<TreePath>,
    custom_fill_search: String,
    pending_fill_warning: Option<PendingFillWarning>,
    popup_was_open: bool,
    new_map_dialog: Option<NewMapDialog>,
    new_level_dialog: Option<NewLevelDialog>,
    new_level_type_path: String,
    save_dialog: Option<SaveDialog>,
    pending_close: Option<DocumentId>,
    exit_requested: bool,
    show_welcome: bool,
    welcome_map_filter: String,
    welcome_maps_expanded: bool,
    open_error: Option<String>,
    load_window: WindowKey,
    load_notice: Option<LoadNotice>,
    load_window_size: [f32; 2],
    keybind_preset_prompt: bool,
}

impl UiState {
    pub fn new(keybind_preset_prompt: bool) -> Result<Self, WindowKeyError> {
        let object_tree = ObjectTreePanel::new()?;
        let welcome_window = WindowKey::new("welcome", "Welcome")?;
        let inspector = InspectorPanel::new()?;
        let settings_window = SettingsWindow::new()?;
        let load_window = WindowKey::new("load", "Loading")?;
        let layout = DockLayout::split(
            DockSplit::Left,
            0.25,
            DockLayout::tabs([object_tree.window()]),
            DockLayout::split(
                DockSplit::Right,
                0.20 / 0.75,
                DockLayout::tabs([inspector.window()]),
                DockLayout::tabs([&welcome_window]),
            ),
        );

        Ok(Self {
            object_tree,
            map_views: HashMap::new(),
            central_node: None,
            dockspace_root: None,
            welcome_window,
            inspector,
            settings_window,
            layout,
            gizmo: GizmoState::default(),
            gizmo_context: None,
            placement_flash: None,
            placement_stroke: None,
            deletion_stroke: None,
            block_selection_options: BlockSelectionOptions::default(),
            fill_mode: FillMode::default(),
            custom_fill_boundaries: vec![TreePath::parse(DEFAULT_CUSTOM_FILL_BOUNDARY)],
            custom_fill_search: String::new(),
            pending_fill_warning: None,
            popup_was_open: false,
            new_map_dialog: None,
            new_level_dialog: None,
            new_level_type_path: String::new(),
            save_dialog: None,
            pending_close: None,
            exit_requested: false,
            show_welcome: true,
            welcome_map_filter: String::new(),
            welcome_maps_expanded: false,
            open_error: None,
            load_window,
            load_notice: None,
            load_window_size: [0.0, 0.0],
            keybind_preset_prompt,
        })
    }

    pub fn set_open_error(&mut self, error: Option<String>) { self.open_error = error; }

    pub fn reveal_selected_instance(&mut self, session: &Session) {
        self.object_tree.reveal_selected_instance(session);
    }

    pub fn set_load_notice(&mut self, notice: Option<LoadNotice>) { self.load_notice = notice; }

    pub fn request_exit(&mut self) { self.exit_requested = true; }

    pub fn request_refit(&mut self, id: Option<DocumentId>) {
        match id.and_then(|id| self.map_views.get_mut(&id)) {
            Some(view) => view.refit = true,
            None => {
                for view in self.map_views.values_mut() {
                    view.refit = true;
                }
            },
        }
    }

    fn cancel_edit_gestures(&mut self, session: &mut Session, id: Option<DocumentId>) {
        self.gizmo.cancel();
        self.placement_flash = None;
        self.placement_stroke = None;
        self.deletion_stroke = None;

        if let Some(id) = id
            && let Some(view) = self.map_views.get_mut(&id)
        {
            restore_rectangle_gesture(session, id, &mut view.rectangle_gesture);
            view.block_selection_anchor = None;
            view.block_placement = None;
            view.paste = None;
        }
    }

    pub fn set_new_map_path(&mut self, path: PathBuf, codebase_dir: &Path) {
        let Some(dialog) = self.new_map_dialog.as_mut() else {
            return;
        };

        dialog.path = path.strip_prefix(codebase_dir).unwrap_or(&path).display().to_string();
        dialog.error = None;
    }

    pub fn draw(
        &mut self, ui: &Ui, session: &mut Session, settings: &mut Settings, load: Option<&LoadView>,
    ) -> Result<UiOutput, DockspaceError> {
        let loading = load.is_some() || self.load_notice.is_some();
        let root = ui.get_id(DOCKSPACE_ID);
        ui.dockspace()
            .main_viewport()
            .root_id(root)
            .flags(DockNodeFlags::PASSTHRU_CENTRAL_NODE)
            .layout(&self.layout, DockLayoutApply::IfMissing)
            .build()?;
        self.dockspace_root = Some(root);

        if self.keybind_preset_prompt {
            let keybind_preset = draw_keybind_preset_dialog(ui, &mut self.keybind_preset_prompt);
            let exit = self.draw_exit_confirmation(ui, session);

            return Ok(UiOutput {
                exit,
                map_views: Vec::new(),
                picking: None,
                open: None,
                open_source: None,
                pick_new_map_path: false,
                cancel_load: false,
                copy_to_clipboard: None,
                keybind_preset,
            });
        }

        let mut open = None;
        let mut show_welcome = false;
        let mut open_new_map_dialog = false;
        let mut open_save_dialog = false;
        let mut pick_new_map_path = false;
        let mut toggle_areas = false;
        let mut toggle_area_outlines = false;
        let mut level_delta = 0;
        let mut underlay_depth = None;
        let mut refit = false;
        let mut undo = false;
        let mut redo = false;

        ui.main_menu_bar(|| {
            ui.menu("File", || {
                if ui.menu_item_enabled_selected_no_shortcut(
                    "Open codebase...",
                    false,
                    session.tree().is_none() && !loading,
                ) {
                    open = Some(OpenRequest::PickCodebase);
                }
                if ui.menu_item_enabled_selected_no_shortcut("Open map...", false, session.tree().is_some() && !loading)
                {
                    open = Some(OpenRequest::PickMap);
                }
                ui.separator();
                if ui.menu_item_enabled_selected_with_shortcut(
                    "Save...",
                    settings.keybindings.get(KeybindAction::Save).label(ui),
                    false,
                    session.map().is_some(),
                ) {
                    open_save_dialog = true;
                }
                ui.separator();
                if ui.menu_item("Welcome") {
                    show_welcome = true;
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
                    undo = true;
                }

                let next = session.redo_label();
                let label = next.map_or_else(|| String::from("Redo"), |edit| format!("Redo {edit}"));
                if ui.menu_item_enabled_selected_with_shortcut(
                    label,
                    settings.keybindings.get(KeybindAction::Redo).label(ui),
                    false,
                    next.is_some(),
                ) {
                    redo = true;
                }
            });
            ui.menu("View", || {
                if ui.menu_item_enabled_selected_with_shortcut(
                    "Show areas",
                    settings.keybindings.get(KeybindAction::ShowAreas).label(ui),
                    session.options.show_areas,
                    true,
                ) {
                    toggle_areas = true;
                }
                if ui.menu_item_enabled_selected_with_shortcut(
                    "Show area outlines",
                    settings.keybindings.get(KeybindAction::ShowAreaOutlines).label(ui),
                    session.options.show_area_outlines,
                    true,
                ) {
                    toggle_area_outlines = true;
                }
                if ui.menu_item_with_shortcut("Z up", settings.keybindings.get(KeybindAction::LevelUp).label(ui)) {
                    level_delta += 1;
                }
                if ui.menu_item_with_shortcut("Z down", settings.keybindings.get(KeybindAction::LevelDown).label(ui)) {
                    level_delta -= 1;
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
                            underlay_depth = Some(depth);
                        }
                    }
                });
                if ui.menu_item_with_shortcut("Refit", settings.keybindings.get(KeybindAction::Refit).label(ui)) {
                    refit = true;
                }
            });
        });

        if session.map().is_some()
            && !open_save_dialog
            && self.save_dialog.is_none()
            && !self.settings_window.is_capturing_keybind()
            && !ui.io().want_text_input()
            && settings.keybindings.get(KeybindAction::Save).is_pressed(ui)
        {
            if session.can_save_map_in_place() {
                if let Err(error) = session.save_map() {
                    self.save_dialog = Some(SaveDialog {
                        path: session
                            .map_path()
                            .map(|path| path.display().to_string())
                            .unwrap_or_default(),
                        format: session.map_format().unwrap_or_default(),
                        error: Some(error.to_string()),
                    });
                    ui.open_popup(SAVE_MAP_POPUP);
                }
            } else {
                open_save_dialog = true;
            }
        }

        if undo || redo {
            self.cancel_edit_gestures(session, session.state.active());
            if undo {
                session.undo();
            } else {
                session.redo();
            }
        }

        if toggle_areas {
            session.toggle_areas();
        }
        if toggle_area_outlines {
            session.toggle_area_outlines();
        }
        if level_delta != 0 {
            request_level_change(
                session,
                level_delta,
                &mut self.new_level_dialog,
                &self.new_level_type_path,
            );
        }
        if let Some(depth) = underlay_depth {
            session.set_underlay_depth(depth);
        }

        if open_save_dialog {
            self.save_dialog = Some(SaveDialog {
                path: session
                    .map_path()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default(),
                format: session.map_format().unwrap_or_default(),
                error: None,
            });
            ui.open_popup(SAVE_MAP_POPUP);
        }
        draw_save_dialog(ui, session, &mut self.save_dialog);

        let object_tree_settings_changed = self.settings_window.draw(ui, session, settings);
        if object_tree_settings_changed {
            self.object_tree.invalidate_filter();
        }
        self.show_welcome |= show_welcome;

        let mut open_source = self.object_tree.draw(ui, session, settings);
        let inspector = self.inspector.draw(ui, session, settings);
        open_source = inspector.open_source.or(open_source);
        if let Some(target) = inspector.jump {
            self.jump_to_instance(session, target);
        }
        let mut welcome = WelcomeOutput::default();
        self.draw_welcome(ui, session, settings, loading, &mut welcome);
        open = welcome.open.or(open);
        open_new_map_dialog |= welcome.new_map_dialog;
        match welcome.forget {
            Some(ForgetRequest::Codebase(path)) => settings.forget_codebase(&path),
            Some(ForgetRequest::Map(path)) => settings.forget_recent(&path),
            None => {},
        }
        if open_new_map_dialog {
            self.new_map_dialog = Some(NewMapDialog::default());
            ui.open_popup(NEW_MAP_POPUP);
        }
        let (pick_path, created) = draw_new_map_dialog(ui, session, &mut self.new_map_dialog);
        pick_new_map_path |= pick_path;
        if created {
            self.show_welcome = false;
            self.request_refit(None);
        }
        let load_popup = draw_load_popup(
            ui,
            &self.load_window,
            &mut self.load_window_size,
            load,
            self.load_notice.as_mut(),
        );
        if load_popup.dismiss {
            self.load_notice = None;
        }

        let (map_views, picking) = if session.state.is_empty() {
            (Vec::new(), None)
        } else {
            self.draw_map_views(ui, session, settings, refit)
        };
        draw_new_level_dialog(ui, session, &mut self.new_level_dialog, &mut self.new_level_type_path);
        self.settings_window
            .finish_keybind_capture(ui, &mut settings.keybindings);
        let exit = self.draw_exit_confirmation(ui, session);
        self.popup_was_open = ui.is_popup_open_with_flags("", dear_imgui_rs::PopupQueryFlags::ANY_POPUP);

        Ok(UiOutput {
            exit,
            map_views,
            picking,
            open,
            open_source,
            pick_new_map_path,
            cancel_load: load_popup.cancel,
            copy_to_clipboard: load_popup.copy,
            keybind_preset: None,
        })
    }

    fn draw_welcome(
        &mut self, ui: &Ui, session: &Session, settings: &Settings, loading: bool, out: &mut WelcomeOutput,
    ) {
        if !self.show_welcome {
            return;
        }

        let show_welcome = &mut self.show_welcome;
        let central_node = &mut self.central_node;
        let map_filter = &mut self.welcome_map_filter;
        let maps_expanded = &mut self.welcome_maps_expanded;
        let codebase = session.environment_path();
        ui.window(&self.welcome_window).opened(show_welcome).build(|| {
            if settings.focus_windows_on_hover {
                focus_window_on_hover(ui);
            }
            let dock = ui.get_window_dock_id();
            if dock.raw() != 0 {
                *central_node = Some(dock);
            }

            let indent = ((ui.content_region_avail()[0] - WELCOME_CONTENT_WIDTH) / 2.0).max(WELCOME_MIN_INDENT);
            ui.dummy([0.0, WELCOME_MIN_INDENT]);
            ui.indent_by(indent);

            {
                let _font = ui.push_font_with_size(None, WELCOME_TITLE_SIZE);
                ui.text("Rapid Mapping Device");
            }

            match codebase {
                Some(codebase) => ui.text_disabled(codebase.display().to_string()),
                None => draw_welcome_subtitle(ui),
            }

            ui.dummy([0.0, WELCOME_MIN_INDENT]);

            let _disabled = ui.begin_disabled_with_cond(loading);
            match codebase {
                None => {
                    ui.text("Start");
                    if ui.text_link("Open codebase...") {
                        out.open = Some(OpenRequest::PickCodebase);
                    }

                    ui.dummy([0.0, WELCOME_MIN_INDENT]);
                    ui.text("Recent codebases");
                    if settings.recent_codebases.is_empty() {
                        ui.text_disabled("No recent codebases");
                    }
                    for (index, recent) in settings.recent_codebases.iter().enumerate() {
                        let entry = draw_recent_entry(ui, &recent.display().to_string(), &format!("codebase-{index}"));
                        if entry.open {
                            out.open = Some(OpenRequest::Codebase(recent.clone()));
                        }
                        if entry.forget {
                            out.forget = Some(ForgetRequest::Codebase(recent.clone()));
                        }
                    }
                },

                Some(codebase) => {
                    let base = session.codebase_dir().unwrap_or(codebase);

                    ui.text("Start");
                    if ui.text_link("New map...") {
                        out.new_map_dialog = true;
                    }
                    if ui.text_link("Open map...") {
                        out.open = Some(OpenRequest::PickMap);
                    }

                    ui.dummy([0.0, WELCOME_MIN_INDENT]);
                    ui.text("Recent maps");
                    let mut empty = true;
                    for (index, recent) in settings.recent_maps_for(codebase).enumerate() {
                        empty = false;
                        let label = codebase_relative(base, &recent.map);
                        let entry = draw_recent_entry(ui, &label, &format!("recent-{index}"));
                        if entry.open {
                            out.open = Some(OpenRequest::Map(recent.map.clone()));
                        }
                        if entry.forget {
                            out.forget = Some(ForgetRequest::Map(recent.map.clone()));
                        }
                    }

                    if empty {
                        ui.text_disabled("No recent maps in this codebase");
                    }

                    ui.dummy([0.0, WELCOME_MIN_INDENT]);
                    ui.text("Maps");
                    if session.maps().is_empty() {
                        ui.text_disabled("No maps found in this codebase");
                    } else {
                        ui.set_next_item_width(WELCOME_CONTENT_WIDTH);
                        ui.input_text("##welcome-map-filter", map_filter)
                            .hint("Filter maps")
                            .build();
                    }

                    let needle = map_filter.trim().to_ascii_lowercase();
                    let matching = session
                        .maps()
                        .iter()
                        .enumerate()
                        .filter(|(_, map)| map_matches(base, map, &needle));
                    let limit = if *maps_expanded {
                        usize::MAX
                    } else {
                        WELCOME_MAP_PREVIEW
                    };

                    let mut shown = 0;
                    for (index, map) in matching.clone().take(limit) {
                        shown += 1;
                        if ui.text_link(format!("{}##map-{index}", codebase_relative(base, map))) {
                            out.open = Some(OpenRequest::Map(map.clone()));
                        }
                    }

                    if shown == 0 && !session.maps().is_empty() {
                        ui.text_disabled("No maps match the filter");
                    }
                    let hidden = matching.count().saturating_sub(shown);
                    if hidden > 0 {
                        if ui.text_link(format!("Show more... ({hidden})")) {
                            *maps_expanded = true;
                        }
                    } else if *maps_expanded && shown > WELCOME_MAP_PREVIEW && ui.text_link("Show less") {
                        *maps_expanded = false;
                    }
                },
            }
        });
    }

    fn draw_map_views(
        &mut self, ui: &Ui, session: &mut Session, settings: &Settings, refit_active: bool,
    ) -> (Vec<VisibleMapView>, Option<usize>) {
        let mut closing = None;
        let mut visible = Vec::new();

        for (&id, view) in &mut self.map_views {
            if view.rectangle_gesture.is_some_and(|gesture| {
                session.state.active() != Some(id)
                    || session.tool() != Tool::BlockSelect
                    || session
                        .state
                        .document(id)
                        .is_none_or(|document| document.z != gesture.z)
            }) {
                restore_rectangle_gesture(session, id, &mut view.rectangle_gesture);
                view.block_selection_anchor = None;
                self.gizmo.cancel();
            }
        }

        for id in session.state.document_ids() {
            let mut view = match self.map_views.remove(&id) {
                Some(view) => view,
                None => match MapViewState::new(id) {
                    Ok(view) => view,
                    Err(e) => {
                        log::error!("could not create a map view window for document {}: {e}", id.get());

                        continue;
                    },
                },
            };

            let refit = refit_active && session.state.active() == Some(id);
            let mut keep_open = true;
            let active = session.state.active() == Some(id);
            let mut interaction = MapViewInteraction {
                selected: session.selected_instance_of(id),
                selection_guide: (active && settings.selection_guide_line)
                    .then(|| session.selected_offset_guide())
                    .flatten(),
                highlight: settings.selection_highlight.style(),
                mode: interaction_mode(session.tool()),
                ..Default::default()
            };
            let map_view_index = visible.len();
            self.draw_map_view(
                ui,
                session,
                settings,
                MapViewDraw {
                    id,
                    index: map_view_index,
                    view: &mut view,
                    interaction: &mut interaction,
                    refit_requested: refit,
                    keep_open: &mut keep_open,
                },
            );

            if view.visible && !view.rect.is_empty() {
                visible.push(VisibleMapView {
                    document: id,
                    rect: view.rect,
                    camera: view.camera.camera,
                    interaction,
                });
            }
            self.map_views.insert(id, view);

            if !keep_open {
                closing = Some(id);
            }
        }

        if let Some(id) = closing {
            if session.state.document(id).is_some_and(MapDocument::is_dirty) {
                self.pending_close = Some(id);
                ui.open_popup(CLOSE_MAP_POPUP);
            } else {
                session.close_map(id);
            }
        }
        self.draw_close_confirmation(ui, session);

        self.map_views.retain(|id, _| session.state.document(*id).is_some());

        let picking = visible.iter().position(|view| view.interaction.cursor.is_some());

        (visible, picking)
    }

    fn draw_exit_confirmation(&mut self, ui: &Ui, session: &Session) -> bool {
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

    fn draw_close_confirmation(&mut self, ui: &Ui, session: &mut Session) {
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
            session.state.set_active(id);
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

    fn draw_map_view(&mut self, ui: &Ui, session: &mut Session, settings: &Settings, draw: MapViewDraw<'_>) {
        let MapViewDraw {
            id,
            index: map_view_index,
            view,
            interaction,
            refit_requested,
            keep_open,
        } = draw;
        session.hide_block_preview(id);
        let Some(title) = session.state.document(id).map(MapDocument::title) else {
            return;
        };
        let MapViewState {
            window,
            camera,
            size: view_size,
            rect: view_rect,
            visible: view_visible,
            refit: view_refit,
            focus: view_focus,
            block_selection_anchor,
            rectangle_gesture,
            block_placement,
            paste,
        } = view;
        *view_visible = false;
        *view_refit |= refit_requested;
        let refit = view_refit;

        if let Some(node) = self.central_node.or(self.dockspace_root) {
            ui.set_next_window_dock_id_with_cond(node, Condition::FirstUseEver);
        }
        let map_window = ui
            .window(window.label(title.as_str()))
            .opened(keep_open)
            .focused(std::mem::take(view_focus));
        map_window.build(|| {
            if settings.focus_windows_on_hover {
                focus_window_on_hover(ui);
            }
            if ui.is_window_focused() {
                if session.state.active() != Some(id) {
                    self.cancel_edit_gestures(session, session.state.active());
                }
                session.state.set_active(id);
            }
            let is_active = session.state.active() == Some(id);
            if !is_active && rectangle_gesture.is_some() {
                restore_rectangle_gesture(session, id, rectangle_gesture);
                *block_selection_anchor = None;
                self.gizmo.cancel();
            }
            let dock = ui.get_window_dock_id();
            if dock.raw() != 0 {
                self.central_node = Some(dock);
            }

            let (image_size, viewport) = panel_extent(ui.content_region_avail());
            *view_size = viewport;
            camera.resize(viewport.0, viewport.1);
            if *refit {
                let (width, height) = session.extent_px_of(id);
                camera.frame_map(width, height);
                *refit = false;
            }

            *view_visible = true;

            let origin = ui.cursor_screen_pos();
            let scale = ui.io().display_framebuffer_scale();
            let target = ui.io().display_size();
            *view_rect = framebuffer_rect(origin, image_size, scale, target);
            ui.image(Renderer::map_view_texture(map_view_index), image_size);

            let image_hovered = ui.is_item_hovered();
            let viewport_min = ui.item_rect_min();
            let viewport_max = ui.item_rect_max();

            let top_overlay_height = ui.frame_height() + OVERLAY_PADDING * 2.0;
            let history_overlay_height = recent_button_size(ui) + OVERLAY_PADDING * 2.0;
            let top_overlay = OverlayRect {
                min: viewport_min,
                max: [
                    viewport_max[0],
                    (viewport_min[1] + top_overlay_height).min(viewport_max[1]),
                ],
            };
            let bottom_overlay = OverlayRect {
                min: [
                    viewport_min[0],
                    (viewport_max[1] - history_overlay_height).max(viewport_min[1]),
                ],
                max: viewport_max,
            };

            let mouse = ui.io().mouse_pos();
            let over_overlay = top_overlay.contains(mouse) || bottom_overlay.contains(mouse);
            let hovered = image_hovered && !over_overlay && is_active;
            let focused = ui.is_window_focused();
            if focused
                && !ui.io().want_text_input()
                && let Some(index) = settings.keybindings.pressed_recent(ui)
            {
                session.choose_recent(index);
            }

            if hovered && !ui.io().want_text_input() {
                let io = ui.io();
                if !self.gizmo.is_interacting() && ui.is_mouse_down(MouseButton::Middle) {
                    camera.pan_by(io.mouse_delta());
                }

                let wheel = io.mouse_wheel();
                if !self.gizmo.is_interacting() && wheel != 0.0 {
                    let mouse = io.mouse_pos();
                    camera.zoom_by(wheel, [mouse[0] - viewport_min[0], mouse[1] - viewport_min[1]]);
                }

                if settings.keybindings.get(KeybindAction::ShowAreas).is_pressed(ui) {
                    session.toggle_areas();
                }
                if settings.keybindings.get(KeybindAction::ShowAreaOutlines).is_pressed(ui) {
                    session.toggle_area_outlines();
                }
                if settings.keybindings.get(KeybindAction::LevelUp).is_pressed(ui) {
                    request_level_change(session, 1, &mut self.new_level_dialog, &self.new_level_type_path);
                }
                if settings.keybindings.get(KeybindAction::LevelDown).is_pressed(ui) {
                    request_level_change(session, -1, &mut self.new_level_dialog, &self.new_level_type_path);
                }
                if settings.keybindings.get(KeybindAction::Refit).is_pressed(ui) {
                    *refit = true;
                }
                if settings.keybindings.get(KeybindAction::PlaceTool).is_pressed(ui) {
                    session.set_tool(Tool::Place);
                }
                if settings.keybindings.get(KeybindAction::SelectTool).is_pressed(ui) {
                    session.set_tool(Tool::Select);
                }
                if settings.keybindings.get(KeybindAction::BlockSelectTool).is_pressed(ui) {
                    session.set_tool(Tool::BlockSelect);
                }
                if settings.keybindings.get(KeybindAction::DeleteTool).is_pressed(ui) {
                    session.set_tool(Tool::Delete);
                }
                if settings.keybindings.get(KeybindAction::FillTool).is_pressed(ui) {
                    session.set_tool(Tool::Fill);
                }

                if *refit {
                    let (width, height) = session.extent_px();
                    camera.frame_map(width, height);
                    *refit = false;
                }
            }

            if settings.show_tile_grid {
                draw_tile_grid(
                    ui,
                    session,
                    camera,
                    settings.tile_grid_min_pixels,
                    viewport_min,
                    viewport_max,
                );
            }
            if is_active
                && settings.show_selected_pixel_grid
                && let Some(transform) = session.selected_transform()
                && transform.sprite.z == session.z()
            {
                draw_selected_pixel_grid(
                    ui,
                    camera,
                    &transform,
                    session.options.tile_size,
                    settings.selected_pixel_grid_min_pixels,
                    viewport_min,
                    viewport_max,
                );
            }

            configure_tool_interaction(session.tool(), interaction);

            let in_viewport = |point: [f32; 2]| {
                let local = [point[0] - viewport_min[0], point[1] - viewport_min[1]];

                local
                    .iter()
                    .all(|value| value.is_finite() && *value >= 0.0)
                    .then_some(local)
                    .filter(|local| local[0] < viewport.0 as f32 && local[1] < viewport.1 as f32)
            };
            let cursor = hovered.then(|| ui.io().mouse_pos()).and_then(in_viewport);
            let pointed_coord = cursor.and_then(|cursor| {
                let size = session.map()?.size;

                camera.screen_to_tile(cursor, size, session.options.tile_size, session.z())
            });

            if hovered
                && !ui.io().want_text_input()
                && !has_modifiers(ui)
                && !settings.keybindings.is_any_pressed(ui)
                && ui.is_key_pressed(Key::F)
            {
                session.toggle_focus_at(pointed_coord);
                self.placement_stroke = None;
            }

            if is_active {
                if focused && !ui.io().want_text_input() {
                    let undo = settings.keybindings.get(KeybindAction::Undo).is_pressed_repeating(ui);
                    let redo = settings.keybindings.get(KeybindAction::Redo).is_pressed_repeating(ui);
                    if undo || redo {
                        restore_rectangle_gesture(session, id, rectangle_gesture);
                        self.gizmo.cancel();
                        self.placement_flash = None;
                        self.placement_stroke = None;
                        self.deletion_stroke = None;
                        *block_selection_anchor = None;
                        *block_placement = None;
                        *paste = None;

                        if undo {
                            session.undo();
                        } else {
                            session.redo();
                        }
                    }
                    if settings.keybindings.get(KeybindAction::Copy).is_pressed(ui) {
                        session.copy_selection(session.selection_mode());
                    }
                    if settings.keybindings.get(KeybindAction::Paste).is_pressed(ui)
                        && let Some(block) = session.clipboard()
                    {
                        let (width, height) = (block.width(), block.height());
                        restore_rectangle_gesture(session, id, rectangle_gesture);
                        let anchor = pointed_coord.or_else(|| {
                            let size = session.map()?.size;
                            let center = [viewport.0 as f32 * 0.5, viewport.1 as f32 * 0.5];

                            camera.screen_to_tile(center, size, session.options.tile_size, session.z())
                        });
                        if let (Some(anchor), Some(size)) = (anchor, session.map().map(|map| map.size)) {
                            session.set_tool(Tool::BlockSelect);
                            self.gizmo.cancel();
                            *block_placement = None;
                            *block_selection_anchor = None;
                            *paste = Some(PendingPaste {
                                min: centered_paste_min(width, height, anchor, size),
                                rotation: SelectionRotation::Original,
                            });
                        }
                    }
                }

                let tool = session.tool();
                let gizmo_context = (id, tool, session.z());
                if self.gizmo_context != Some(gizmo_context) {
                    self.gizmo.cancel();
                    self.gizmo_context = Some(gizmo_context);
                }
                if rectangle_gesture.is_some_and(|gesture| tool != Tool::BlockSelect || gesture.z != session.z()) {
                    restore_rectangle_gesture(session, id, rectangle_gesture);
                    *block_selection_anchor = None;
                    self.gizmo.cancel();
                }
                let escape = focused
                    && !ui.io().want_text_input()
                    && !self.popup_was_open
                    && !ui.is_popup_open_with_flags("", dear_imgui_rs::PopupQueryFlags::ANY_POPUP)
                    && ui.is_key_pressed(Key::Escape);
                if escape {
                    if rectangle_gesture.is_some() {
                        restore_rectangle_gesture(session, id, rectangle_gesture);
                        *block_selection_anchor = None;
                    } else if block_placement.is_some() || paste.is_some() {
                        *block_placement = None;
                        *paste = None;
                    } else if matches!(tool, Tool::BlockSelect | Tool::Fill) {
                        session.select_block(None);
                    }
                    self.gizmo.cancel();
                }
                let preview_coord = (tool == Tool::Place)
                    .then(|| self.gizmo.placement_coord().or(pointed_coord))
                    .flatten();
                let active_flash =
                    active_placement_flash(&mut self.placement_flash, ui.time(), settings.tile_place_flash);
                interaction.placement_flash = active_flash.map(|(_, flash)| flash);

                if let Some(coord) = preview_coord
                    && session.can_edit_at(coord)
                    && active_flash.is_none_or(|(flash_coord, _)| flash_coord != coord)
                {
                    draw_placement_preview(ui, session, camera, coord, viewport_min, viewport_max);
                }

                let gizmo_map_view = GizmoMapView {
                    min: viewport_min,
                    max: viewport_max,
                    hovered,
                };

                if tool != Tool::BlockSelect {
                    *block_selection_anchor = None;
                    *block_placement = None;
                    *paste = None;
                } else {
                    if block_selection_anchor.is_some_and(|anchor| anchor.z != session.z()) {
                        *block_selection_anchor = None;
                    }

                    if block_placement.is_some_and(|placement| Some(placement.source) != session.selection()) {
                        *block_placement = None;
                    }

                    if let Some(pending) = paste.as_mut() {
                        pending.min.z = session.z();
                    }
                }

                let paste_mask =
                    paste.and_then(|pending| session.clipboard_selection_mask(pending.min, pending.rotation));
                let paste_target = paste_mask.map(|mask| mask.bounds);
                if paste.is_some() && paste_target.is_none() {
                    *paste = None;
                }

                let block_controls_area = OverlayRect {
                    min: [viewport_min[0], top_overlay.max[1]],
                    max: [viewport_max[0], bottom_overlay.min[1]],
                };

                let controls_hit_test = match *paste {
                    Some(_) => paste_controls(self.gizmo.block_rotation_open(), paste_target)
                        .map(|target| (PlacementControls::Paste, target)),
                    None => block_controls_placement(
                        tool,
                        rectangle_gesture.is_some(),
                        self.gizmo.block_rotation_open(),
                        session.selection(),
                        *block_placement,
                    )
                    .map(|placement| (PlacementControls::Block, placement.target)),
                };
                let block_controls_capture_mouse = controls_hit_test.is_some_and(|(controls, target)| {
                    let (_, bounds) = block_placement_controls_layout(
                        ui,
                        camera,
                        controls,
                        target,
                        session.options.tile_size,
                        viewport_min,
                        block_controls_area,
                    );

                    bounds.contains(mouse)
                });

                let gizmo_map_view = GizmoMapView {
                    hovered: hovered && !block_controls_capture_mouse && !escape && !ui.io().want_text_input(),
                    ..gizmo_map_view
                };

                let gizmo_captures_mouse = match tool {
                    Tool::Select => {
                        self.gizmo
                            .draw(
                                ui,
                                session,
                                settings,
                                camera,
                                self.inspector.transform_mode(),
                                gizmo_map_view,
                            )
                            .captures_mouse
                    },
                    Tool::Place => {
                        self.gizmo
                            .draw_placement_direction(ui, session, settings, camera, pointed_coord, gizmo_map_view)
                            .captures_mouse
                    },
                    Tool::BlockSelect => {
                        if let (Some(pending), Some(target), Some(size)) =
                            (*paste, paste_target, session.map().map(|map| map.size))
                        {
                            let response = self.gizmo.draw_block(
                                ui,
                                settings,
                                camera,
                                BlockGizmoTarget {
                                    selection: target,
                                    rotation: pending.rotation,
                                    map_size: size,
                                    tile_size: session.options.tile_size,
                                    kind: BlockGizmoKind::Clipboard,
                                },
                                gizmo_map_view,
                            );

                            if response.selection.min != target.min || response.rotation != pending.rotation {
                                *paste = Some(PendingPaste {
                                    min: response.selection.min,
                                    rotation: response.rotation,
                                });
                            }

                            response.captures_mouse
                        } else if block_selection_anchor.is_some() {
                            self.gizmo.cancel();

                            false
                        } else if let (Some(source), Some(size)) =
                            (session.selection(), session.map().map(|map| map.size))
                        {
                            let (displayed, rotation) = block_placement
                                .map_or((source, SelectionRotation::Original), |placement| {
                                    (placement.target, placement.rotation)
                                });

                            let response = self.gizmo.draw_block(
                                ui,
                                settings,
                                camera,
                                BlockGizmoTarget {
                                    selection: displayed,
                                    rotation,
                                    map_size: size,
                                    tile_size: session.options.tile_size,
                                    kind: if block_placement.is_some() {
                                        BlockGizmoKind::Placement
                                    } else {
                                        BlockGizmoKind::Selection
                                    },
                                },
                                gizmo_map_view,
                            );

                            if response.resizing {
                                rectangle_gesture.get_or_insert(RectangleGesture {
                                    start: session.selection_mask(),
                                    z: session.z(),
                                });
                                session.try_select_block(SelectionMask {
                                    bounds: response.selection,
                                    mode: session.selection_mode(),
                                });
                            } else if response.selection != displayed || response.rotation != rotation {
                                *block_selection_anchor = None;
                                *block_placement =
                                    rotated_selection_at(source, response.selection.min, response.rotation)
                                        .filter(|target| {
                                            *target != source || response.rotation != SelectionRotation::Original
                                        })
                                        .map(|target| PendingBlockPlacement {
                                            source,
                                            target,
                                            rotation: response.rotation,
                                        });
                            }

                            response.captures_mouse
                        } else {
                            self.gizmo.cancel();

                            false
                        }
                    },
                    Tool::Delete => {
                        self.gizmo.cancel();

                        false
                    },
                    Tool::Fill => {
                        self.gizmo.cancel();

                        false
                    },
                };
                let left_clicked = ui.is_mouse_clicked(MouseButton::Left);
                let left_down = ui.is_mouse_down(MouseButton::Left);
                if left_clicked {
                    self.placement_stroke = None;
                    self.deletion_stroke = None;
                }
                if !left_down || session.tool() != Tool::Delete {
                    self.deletion_stroke = None;
                }
                if self.placement_stroke.as_ref().is_some_and(|stroke| {
                    !left_down
                        || gizmo_captures_mouse
                        || !stroke.matches_context(session.tool(), session.palette(), session.z())
                }) {
                    self.placement_stroke = None;
                }
                if !escape
                    && !gizmo_captures_mouse
                    && !block_controls_capture_mouse
                    && let Some(cursor) = cursor
                {
                    let pixel = [cursor[0].floor() as u32, cursor[1].floor() as u32];
                    interaction.cursor = Some(cursor_in_map_view(cursor, scale));

                    if let Some(coord) = pointed_coord {
                        interaction.hovered_area = session.area_at(id, coord);
                        match session.tool() {
                            Tool::Place => {
                                if left_clicked {
                                    self.placement_stroke = session
                                        .palette()
                                        .cloned()
                                        .map(|prefab| PlacementStroke::new(prefab, session.z()));
                                }
                                let group = session
                                    .can_edit_at(coord)
                                    .then(|| self.placement_stroke.as_mut().and_then(|stroke| stroke.visit(coord)))
                                    .flatten();
                                let placed = group.and_then(|group| session.place_at(coord, Some(group)));
                                if settings.tile_place_flash
                                    && let Some(owner) = placed
                                {
                                    let flash = ActivePlacementFlash {
                                        owner,
                                        coord,
                                        started_at: ui.time(),
                                    };
                                    self.placement_flash = Some(flash);
                                    interaction.placement_flash = flash.sample(ui.time());
                                }
                            },
                            Tool::Select => {
                                self.placement_stroke = None;
                                if left_clicked {
                                    request_pick(interaction, PickRequest::Cursor);
                                }
                            },
                            Tool::BlockSelect => {
                                self.placement_stroke = None;
                                if block_placement.is_none() && paste.is_none() {
                                    if left_clicked && session.can_edit_at(coord) {
                                        *block_placement = None;
                                        let selection = Selection::from_drag(coord, coord);
                                        *rectangle_gesture = Some(RectangleGesture {
                                            start: session.selection_mask(),
                                            z: session.z(),
                                        });
                                        if session.try_select_block(SelectionMask {
                                            bounds: selection,
                                            mode: self.block_selection_options.drawing_mode(ui.io().key_shift()),
                                        }) {
                                            *block_selection_anchor = Some(coord);
                                        }
                                    }

                                    if ui.is_mouse_clicked(MouseButton::Right)
                                        && block_selection_anchor.is_none()
                                        && session.selection().is_some_and(|selection| {
                                            session.selection_mode().includes(selection, coord)
                                        })
                                    {
                                        ui.open_popup(BLOCK_SELECTION_POPUP);
                                    }
                                }
                            },
                            Tool::Delete => {
                                self.placement_stroke = None;
                                let requests_pick = if left_clicked {
                                    self.deletion_stroke = Some(DeletionStroke::new(pixel));

                                    true
                                } else {
                                    left_down
                                        && self
                                            .deletion_stroke
                                            .as_mut()
                                            .is_some_and(|stroke| stroke.move_to(pixel))
                                };
                                if requests_pick {
                                    request_pick(interaction, PickRequest::Cursor);
                                }
                            },
                            Tool::Fill => {
                                self.placement_stroke = None;
                                if session.selection_mask().is_some_and(|mask| !mask.includes(coord)) {
                                    ui.tooltip_text("Click a selected tile, or Clear selection to fill elsewhere");
                                }
                                if left_clicked
                                    && let FillOutcome::TooLarge { limit } =
                                        session.fill_at(coord, self.fill_mode, &self.custom_fill_boundaries)
                                {
                                    self.pending_fill_warning = Some(PendingFillWarning {
                                        context: FillWarningContext::capture(session),
                                        coord,
                                        fill_mode: self.fill_mode,
                                        custom_fill_boundaries: self.custom_fill_boundaries.clone(),
                                        limit,
                                    });
                                    ui.open_popup(FILL_LIMIT_WARNING_POPUP);
                                }
                            },
                        }
                    }
                }

                if let Some(anchor) = *block_selection_anchor
                    && let Some(size) = session.map().map(|map| map.size)
                    && let Some(coord) =
                        rectangle_drag_coord(camera, mouse, viewport_min, size, session.options.tile_size, anchor.z)
                {
                    session.try_select_block(SelectionMask {
                        bounds: Selection::from_drag(anchor, coord),
                        mode: if left_down {
                            self.block_selection_options.drawing_mode(ui.io().key_shift())
                        } else {
                            session.selection_mode()
                        },
                    });
                }

                if !left_down {
                    *block_selection_anchor = None;
                    *rectangle_gesture = None;
                }

                let overlay_viewport = OverlayRect {
                    min: viewport_min,
                    max: viewport_max,
                };

                let block_overlay = match (*paste, paste_mask) {
                    (Some(pending), Some(mask)) => {
                        let target = mask.bounds;
                        draw_block_outline(ui, session, camera, target, mask.mode, overlay_viewport);

                        Some((target, pending.rotation))
                    },
                    _ if matches!(session.tool(), Tool::BlockSelect | Tool::Fill) => {
                        session.selection().map(|source| {
                            let (displayed, rotation) = block_placement
                                .map_or((source, SelectionRotation::Original), |placement| {
                                    (placement.target, placement.rotation)
                                });
                            draw_block_outline(
                                ui,
                                session,
                                camera,
                                displayed,
                                session.selection_mode(),
                                overlay_viewport,
                            );

                            (displayed, rotation)
                        })
                    },
                    _ => None,
                };

                if let Some((displayed, rotation)) = block_overlay
                    && tool == Tool::BlockSelect
                    && block_selection_anchor.is_none()
                    && let Some(map_size) = session.map().map(|map| map.size)
                {
                    self.gizmo.draw_block_overlay(
                        ui,
                        camera,
                        BlockGizmoTarget {
                            selection: displayed,
                            rotation,
                            map_size,
                            tile_size: session.options.tile_size,
                            kind: if paste.is_some() {
                                BlockGizmoKind::Clipboard
                            } else if block_placement.is_some() {
                                BlockGizmoKind::Placement
                            } else {
                                BlockGizmoKind::Selection
                            },
                        },
                        gizmo_map_view,
                    );
                }

                draw_block_selection_menu(ui, session, session.selection_mode());
                let paste_action = paste
                    .zip(paste_controls(self.gizmo.block_rotation_open(), paste_target))
                    .and_then(|(pending, target)| {
                        let can_paste = session.can_paste_clipboard(pending.min, pending.rotation);

                        draw_paste_controls(
                            ui,
                            camera,
                            target,
                            session.options.tile_size,
                            viewport_min,
                            block_controls_area,
                            can_paste,
                        )
                        .map(|action| (action, pending))
                    });

                let controls_placement = paste
                    .is_none()
                    .then(|| {
                        block_controls_placement(
                            tool,
                            rectangle_gesture.is_some(),
                            self.gizmo.block_rotation_open(),
                            session.selection(),
                            *block_placement,
                        )
                    })
                    .flatten();
                let placement_action = controls_placement.and_then(|placement| {
                    draw_block_placement_controls(
                        ui,
                        camera,
                        placement,
                        viewport_min,
                        block_controls_area,
                        session,
                        &mut self.block_selection_options,
                    )
                    .map(|action| (action, placement))
                });
                draw_top_overlay(
                    ui,
                    session,
                    top_overlay,
                    TopOverlayState {
                        keybindings: settings.keybindings,
                        selection_busy: rectangle_gesture.is_some() || block_placement.is_some() || paste.is_some(),
                        block_selection_options: &mut self.block_selection_options,
                        fill_mode: &mut self.fill_mode,
                        custom_fill_boundaries: &mut self.custom_fill_boundaries,
                        custom_fill_search: &mut self.custom_fill_search,
                        new_level_dialog: &mut self.new_level_dialog,
                        new_level_type_path: &self.new_level_type_path,
                    },
                );
                if let Some((action, pending)) = paste_action {
                    if action == PasteAction::Paste {
                        session.paste_clipboard(pending.min, pending.rotation);
                    }
                    *paste = None;
                    self.gizmo.cancel();
                }
                if let Some((action, placement)) = placement_action {
                    let finished = match action {
                        BlockPlacementAction::Move => session.place_selected_block_with_mode(
                            placement.target.min,
                            placement.rotation,
                            SelectionPlacement::Move,
                            session.selection_mode(),
                        ),
                        BlockPlacementAction::Copy => session.place_selected_block_with_mode(
                            placement.target.min,
                            placement.rotation,
                            SelectionPlacement::Copy,
                            session.selection_mode(),
                        ),
                        BlockPlacementAction::Fill => session.fill_selected_block(
                            placement.target.min,
                            placement.rotation,
                            session.selection_mode(),
                        ),
                        BlockPlacementAction::Cancel => {
                            if block_placement.is_none() {
                                session.select_block(None);
                            }

                            true
                        },
                    };
                    if finished {
                        *block_placement = None;
                        self.gizmo.cancel();
                    }
                }
                let recent_prefabs = session.recent_prefabs();
                if !recent_prefabs.is_empty() {
                    draw_history_overlay(
                        ui,
                        session,
                        settings.keybindings,
                        bottom_overlay,
                        recent_prefabs.to_vec(),
                    );
                }
                configure_tool_interaction(session.tool(), interaction);
                if session.focused_area().is_some() {
                    interaction.hovered_area = None;
                }

                draw_fill_limit_warning(
                    ui,
                    session,
                    &mut self.pending_fill_warning,
                    self.fill_mode,
                    &self.custom_fill_boundaries,
                );

                if let Some(pending) = *paste {
                    session.prepare_block_preview(id, BlockPreviewSource::Clipboard, pending.min, pending.rotation);
                } else if let Some(placement) = *block_placement
                    && session.tool() == Tool::BlockSelect
                {
                    session.prepare_block_preview(
                        id,
                        BlockPreviewSource::Selection(SelectionMask {
                            bounds: placement.source,
                            mode: session.selection_mode(),
                        }),
                        placement.target.min,
                        placement.rotation,
                    );
                }
            }
        });
    }

    fn jump_to_instance(&mut self, session: &mut Session, target: JumpTarget) {
        let Some(location) = session
            .state
            .document(target.document)
            .and_then(|document| document.instance_location(target.instance))
        else {
            return;
        };

        self.cancel_edit_gestures(session, session.state.active());
        session.state.set_active(target.document);
        session.set_level(location.coord.z);
        if let Some(document) = session.state.active_document_mut() {
            document.set_focus(None);
            document.selection = None;
        }
        session.select_instance(Some(target.instance));

        let view = match self.map_views.get_mut(&target.document) {
            Some(view) => view,
            None => {
                let Ok(view) = MapViewState::new(target.document) else {
                    log::error!(
                        "could not create a map view window for document {}",
                        target.document.get()
                    );

                    return;
                };
                self.map_views.insert(target.document, view);
                self.map_views
                    .get_mut(&target.document)
                    .expect("the inserted map view is available")
            },
        };
        view.camera.center_on_tile(location.coord, session.options.tile_size);
        view.refit = false;
        view.focus = true;
    }
}

fn draw_keybind_preset_dialog(ui: &Ui, open: &mut bool) -> Option<KeybindPreset> {
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

fn draw_welcome_subtitle(ui: &Ui) {
    match BUILD_VERSION_URL {
        Some(url) => {
            ui.text_link_open_url(BUILD_VERSION, url);
        },
        None => ui.text_disabled(BUILD_VERSION),
    }

    ui.same_line();
    ui.text_disabled("\u{2022}");
    ui.same_line();

    let hash = BUILD_GIT_SHORT_HASH.unwrap_or("unknown");
    match (BUILD_GIT_SHORT_HASH, BUILD_COMMIT_URL) {
        (Some(_), Some(url)) => {
            ui.text_link_open_url(hash, url);
        },
        _ => ui.text_disabled(hash),
    }

    ui.same_line();
    ui.text_disabled("\u{2022}");
    ui.same_line();
    ui.text_disabled("A map editor for BYOND");
}

fn draw_recent_entry(ui: &Ui, label: &str, id: &str) -> RecentEntry {
    let icon = ICON_CLOSE_THICK.to_string();
    let icon_width = ui.calc_text_size(&icon)[0];

    let open = ui.text_link(format!("{label}##{id}"));
    let link_min = ui.item_rect_min();
    let link_max = ui.item_rect_max();
    let icon_min = [link_max[0] + ui.clone_style().item_spacing()[0], link_min[1]];
    let icon_max = [icon_min[0] + icon_width, link_max[1]];
    let mut forget = false;

    if ui.is_mouse_hovering_rect(link_min, icon_max) {
        let hovered = ui.is_mouse_hovering_rect(icon_min, icon_max);
        let color = if hovered {
            StyleColor::Text
        } else {
            StyleColor::TextDisabled
        };

        ui.get_window_draw_list()
            .add_text([icon_min[0], icon_min[1] + 2.0], ui.style_color(color), &icon);

        if hovered {
            ui.tooltip_text("Remove from this list");
            forget = ui.is_mouse_clicked(MouseButton::Left);
        }
    }

    RecentEntry { open, forget }
}

fn map_matches(base: &Path, map: &Path, needle: &str) -> bool {
    needle.is_empty() || codebase_relative(base, map).to_ascii_lowercase().contains(needle)
}

fn codebase_relative(base: &Path, path: &Path) -> String {
    path.strip_prefix(base).unwrap_or(path).display().to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadNotice {
    Failed {
        title: String,
        path: String,
        message: String,
    },
    Diagnostics {
        path: String,
        summary: String,
        text: String,
    },
}

impl LoadNotice {
    pub fn failed(title: &str, path: &Path, message: impl Into<String>) -> Self {
        Self::Failed {
            title: format!("{title} failed"),
            path: path.display().to_string(),
            message: message.into(),
        }
    }

    pub fn diagnostics(path: &Path, summary: String, lines: Vec<String>) -> Self {
        Self::Diagnostics {
            path: path.display().to_string(),
            summary,
            text: lines.join("\n"),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct LoadPopup {
    cancel: bool,
    dismiss: bool,
    copy: Option<String>,
}

fn draw_load_popup(
    ui: &Ui, window: &WindowKey, measured: &mut [f32; 2], load: Option<&LoadView>, notice: Option<&mut LoadNotice>,
) -> LoadPopup {
    let mut popup = LoadPopup::default();
    if load.is_none() && notice.is_none() {
        return popup;
    }

    let center = ui.main_viewport().work_center();
    let position = [center[0] - measured[0] / 2.0, center[1] - measured[1] / 2.0];
    let flags = WindowFlags::ALWAYS_AUTO_RESIZE
        | WindowFlags::NO_TITLE_BAR
        | WindowFlags::NO_RESIZE
        | WindowFlags::NO_MOVE
        | WindowFlags::NO_COLLAPSE
        | WindowFlags::NO_SAVED_SETTINGS
        | WindowFlags::NO_DOCKING;

    ui.window(window)
        .flags(flags)
        .position(position, Condition::Always)
        .build(|| draw_load_body(ui, measured, load, notice, &mut popup));

    popup
}

fn draw_load_body(
    ui: &Ui, measured: &mut [f32; 2], load: Option<&LoadView>, notice: Option<&mut LoadNotice>, popup: &mut LoadPopup,
) {
    let escape = ui.is_key_pressed(Key::Escape);
    match (load, notice) {
        (Some(view), _) => {
            draw_load_heading(ui, view.title, &view.path);
            draw_load_progress(ui, &view.snapshot);
            ui.separator();

            let _disabled = ui.begin_disabled_with_cond(view.cancelling);
            if ui.button(if view.cancelling { "Cancelling..." } else { "Cancel" }) || escape {
                popup.cancel = true;
            }
        },

        (None, Some(LoadNotice::Failed { title, path, message })) => {
            draw_load_heading(ui, title, path);
            let lines = wrapped_lines(ui, message).clamp(LOAD_TEXT_MIN_LINES, LOAD_ERROR_MAX_LINES);
            draw_selectable_text(ui, "##load-error", message, lines, Some(SAVE_ERROR_COLOR));
            ui.separator();

            if ui.button("Close") || escape {
                popup.dismiss = true;
            }
            ui.same_line();
            if ui.button("Copy") {
                popup.copy = Some(message.clone());
            }
        },

        (None, Some(LoadNotice::Diagnostics { path, summary, text })) => {
            draw_load_heading(ui, "Loaded with diagnostics", path);
            ui.text(summary);
            let lines = wrapped_lines(ui, text).clamp(LOAD_TEXT_MIN_LINES, LOAD_DIAGNOSTICS_LINES);
            draw_selectable_text(ui, "##load-diagnostics", text, lines, None);
            ui.separator();

            if ui.button("Close") || escape {
                popup.dismiss = true;
            }
            ui.same_line();
            if ui.button("Copy") {
                popup.copy = Some(text.clone());
            }
        },

        (None, None) => {},
    }

    *measured = ui.window_size();
}

fn draw_selectable_text(ui: &Ui, id: &str, text: &mut String, lines: usize, color: Option<[f32; 4]>) {
    let _color = color.map(|color| ui.push_style_color(StyleColor::Text, color));
    let height = ui.text_line_height_with_spacing() * lines as f32 + ui.clone_style().frame_padding()[1] * 2.0;

    ui.input_text_multiline(id, text, [LOAD_TEXT_WIDTH, height])
        .flags(InputTextMultilineFlags::READ_ONLY | InputTextMultilineFlags::WORD_WRAP)
        .build();
}

fn wrapped_lines(ui: &Ui, text: &str) -> usize {
    text.lines()
        .map(|line| {
            let width = ui.calc_text_size(line)[0];

            ((width / LOAD_TEXT_WIDTH).ceil() as usize).max(1)
        })
        .sum::<usize>()
        .max(1)
}

fn draw_load_heading(ui: &Ui, title: &str, path: &str) {
    ui.text(title);
    ui.text_disabled(shorten_path(path, LOAD_PATH_MAX_CHARS));
    ui.separator();
}

fn draw_load_progress(ui: &Ui, snapshot: &Snapshot) {
    match snapshot.fraction() {
        Some(fraction) => ui
            .progress_bar(fraction)
            .size([LOAD_POPUP_WIDTH, 0.0])
            .overlay_text(format!("{} / {}", grouped(snapshot.done), grouped(snapshot.total)))
            .build(),

        None => {
            let bar = ui.progress_bar(-(ui.time() as f32)).size([LOAD_POPUP_WIDTH, 0.0]);
            match (snapshot.stage, snapshot.done) {
                (Stage::Preprocess, done) if done > 0 => bar.overlay_text(format!("{} files", grouped(done))).build(),
                _ => bar.overlay_text("").build(),
            }
        },
    }

    ui.text(snapshot.stage.label());
    ui.text_disabled(shorten_path(&snapshot.detail, LOAD_PATH_MAX_CHARS));
}

fn shorten_path(path: &str, max: usize) -> String {
    let count = path.chars().count();
    if count <= max {
        return String::from(path);
    }

    let kept = path.chars().skip(count - max.saturating_sub(1)).collect::<String>();

    format!("\u{2026}{kept}")
}

fn grouped(value: usize) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);

    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }

    out
}

fn active_placement_flash(
    active: &mut Option<ActivePlacementFlash>, now: f64, enabled: bool,
) -> Option<(Coord, PlacementFlash)> {
    if !enabled {
        *active = None;

        return None;
    }

    let placement = (*active)?;
    let Some(flash) = placement.sample(now) else {
        *active = None;

        return None;
    };

    Some((placement.coord, flash))
}

fn request_level_change(
    session: &mut Session, delta: i32, dialog: &mut Option<NewLevelDialog>, remembered_type_path: &str,
) {
    if session.change_level(delta) != LevelChange::NewLevelRequested {
        return;
    }
    let Some(document) = session.state.active() else {
        return;
    };

    *dialog = Some(NewLevelDialog {
        document,
        type_path: remembered_type_path.to_owned(),
        error: None,
        open: true,
    });
}

fn draw_z_levels(ui: &Ui, session: &mut Session, dialog: &mut Option<NewLevelDialog>, remembered_type_path: &str) {
    let current = session.z();
    let button_size = ui.frame_height();
    ui.align_text_to_frame_padding();
    ui.text("Z:");
    ui.same_line();

    let down = {
        let _disabled = ui.begin_disabled_with_cond(!session.can_change_level(-1));

        ui.button_with_size("<##z-level-down", [button_size, button_size])
    };
    ui.same_line();
    ui.align_text_to_frame_padding();
    ui.text(current.to_string());
    ui.same_line();
    let up = {
        let _disabled = ui.begin_disabled_with_cond(!session.can_change_level(1));

        ui.button_with_size(">##z-level-up", [button_size, button_size])
    };

    if down {
        request_level_change(session, -1, dialog, remembered_type_path);
    } else if up {
        request_level_change(session, 1, dialog, remembered_type_path);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct OverlayRect {
    min: [f32; 2],
    max: [f32; 2],
}

impl OverlayRect {
    fn contains(self, point: [f32; 2]) -> bool {
        point[0] >= self.min[0] && point[0] < self.max[0] && point[1] >= self.min[1] && point[1] < self.max[1]
    }
}

const TILE_GRID_COLOR: [f32; 4] = [1.0, 1.0, 1.0, 0.12];
const SELECTED_PIXEL_GRID_COLOR: [f32; 4] = [1.0, 1.0, 1.0, 0.2];

fn draw_map_grid_line(
    draw: &DrawListMut<'_>, camera: &Controller, viewport_min: [f32; 2], from: [f32; 2], to: [f32; 2], color: [f32; 4],
) {
    let from = camera.map_to_screen(from);
    let to = camera.map_to_screen(to);
    draw.add_line(
        [viewport_min[0] + from[0], viewport_min[1] + from[1]],
        [viewport_min[0] + to[0], viewport_min[1] + to[1]],
        color,
    )
    .thickness(1.0)
    .build();
}

fn draw_tile_grid(
    ui: &Ui, session: &Session, camera: &Controller, min_pixels: u32, viewport_min: [f32; 2], viewport_max: [f32; 2],
) {
    let Some(size) = session.map().map(|map| map.size) else {
        return;
    };
    let tile_size = session.options.tile_size.max(1) as f32;
    let map_width = size.x as f32 * tile_size;
    let map_height = size.y as f32 * tile_size;

    if tile_size * camera.camera.zoom < min_pixels as f32 {
        return;
    }

    let local_max = [viewport_max[0] - viewport_min[0], viewport_max[1] - viewport_min[1]];
    let corner_a = camera.screen_to_map([0.0, 0.0]);
    let corner_b = camera.screen_to_map(local_max);
    let (min_x, max_x) = (corner_a[0].min(corner_b[0]), corner_a[0].max(corner_b[0]));
    let (min_y, max_y) = (corner_a[1].min(corner_b[1]), corner_a[1].max(corner_b[1]));

    let first_col = (min_x / tile_size).floor().max(0.0) as u32;
    let last_col = (max_x / tile_size).ceil().min(size.x as f32) as u32;
    let first_row = (min_y / tile_size).floor().max(0.0) as u32;
    let last_row = (max_y / tile_size).ceil().min(size.y as f32) as u32;

    let draw = ui.get_window_draw_list();
    draw.with_clip_rect(viewport_min, viewport_max, || {
        for col in first_col..=last_col {
            let x = col as f32 * tile_size;
            draw_map_grid_line(&draw, camera, viewport_min, [x, map_height], [x, 0.0], TILE_GRID_COLOR);
        }

        for row in first_row..=last_row {
            let y = row as f32 * tile_size;
            draw_map_grid_line(&draw, camera, viewport_min, [0.0, y], [map_width, y], TILE_GRID_COLOR);
        }
    });
}

fn draw_selected_pixel_grid(
    ui: &Ui, camera: &Controller, transform: &SelectedTransform, tile_size: u32, min_pixels: u32,
    viewport_min: [f32; 2], viewport_max: [f32; 2],
) {
    let tile_size = tile_size.max(1) as f32;

    if camera.camera.zoom < min_pixels as f32 {
        return;
    }

    let center = [
        (transform.sprite.x / tile_size).round() * tile_size,
        (transform.sprite.y / tile_size).round() * tile_size,
    ];
    let origin = [center[0] - tile_size, center[1] - tile_size];
    let extent = tile_size * 3.0;
    let draw = ui.get_window_draw_list();
    draw.with_clip_rect(viewport_min, viewport_max, || {
        let steps = extent.round() as u32;
        for col in 0..=steps {
            let x = origin[0] + col as f32;
            draw_map_grid_line(
                &draw,
                camera,
                viewport_min,
                [x, origin[1]],
                [x, origin[1] + extent],
                SELECTED_PIXEL_GRID_COLOR,
            );
        }

        for row in 0..=steps {
            let y = origin[1] + row as f32;
            draw_map_grid_line(
                &draw,
                camera,
                viewport_min,
                [origin[0], y],
                [origin[0] + extent, y],
                SELECTED_PIXEL_GRID_COLOR,
            );
        }
    });
}

fn draw_placement_preview(
    ui: &Ui, session: &Session, camera: &Controller, coord: Coord, viewport_min: [f32; 2], viewport_max: [f32; 2],
) {
    let Some(preview) = session.placement_preview() else {
        return;
    };
    let bounds = placement_preview_bounds(camera, viewport_min, coord, session.options.tile_size, preview);
    let mut tint = preview.thumbnail.tint;
    tint[3] *= placement_preview_opacity(ui.time());
    let draw = ui.get_window_draw_list();

    draw.with_clip_rect(viewport_min, viewport_max, || {
        draw.add_image(
            Renderer::sprite_texture(preview.thumbnail.texture.index),
            bounds.min,
            bounds.max,
            preview.thumbnail.uv0,
            preview.thumbnail.uv1,
            tint,
        );
    });
}

fn placement_preview_bounds(
    camera: &Controller, viewport_min: [f32; 2], coord: Coord, tile_size: u32, preview: PlacementPreview,
) -> OverlayRect {
    let tile_size = tile_size.max(1);
    let x = coord.x.saturating_sub(1).saturating_mul(tile_size) as f32 + preview.offset[0] as f32;
    let y = coord.y.saturating_sub(1).saturating_mul(tile_size) as f32 + preview.offset[1] as f32;
    let width = preview.thumbnail.texture.width as f32;
    let height = preview.thumbnail.texture.height as f32;
    let top_left = camera.map_to_screen([x, y + height]);
    let bottom_right = camera.map_to_screen([x + width, y]);

    OverlayRect {
        min: [viewport_min[0] + top_left[0], viewport_min[1] + top_left[1]],
        max: [viewport_min[0] + bottom_right[0], viewport_min[1] + bottom_right[1]],
    }
}

fn placement_preview_opacity(time: f64) -> f32 {
    let phase = time.rem_euclid(PLACEMENT_PREVIEW_PERIOD) / PLACEMENT_PREVIEW_PERIOD * std::f64::consts::TAU;

    (0.9 + 0.1 * phase.cos()) as f32
}

fn block_selection_bounds(
    camera: &Controller, viewport_min: [f32; 2], selection: Selection, tile_size: u32,
) -> OverlayRect {
    let tile_size = tile_size.max(1) as f32;
    let left = selection.min.x.saturating_sub(1) as f32 * tile_size;
    let right = selection.max.x as f32 * tile_size;
    let bottom = selection.min.y.saturating_sub(1) as f32 * tile_size;
    let top = selection.max.y as f32 * tile_size;
    let top_left = camera.map_to_screen([left, top]);
    let bottom_right = camera.map_to_screen([right, bottom]);

    OverlayRect {
        min: [viewport_min[0] + top_left[0], viewport_min[1] + top_left[1]],
        max: [viewport_min[0] + bottom_right[0], viewport_min[1] + bottom_right[1]],
    }
}

fn draw_block_outline(
    ui: &Ui, session: &Session, camera: &Controller, displayed: Selection, mode: BlockSelectionMode,
    viewport: OverlayRect,
) {
    let bounds = block_selection_bounds(camera, viewport.min, displayed, session.options.tile_size);
    let inner_bounds = hollow_selection_inner(displayed, mode)
        .map(|inner| block_selection_bounds(camera, viewport.min, inner, session.options.tile_size));
    let draw = ui.get_window_draw_list();
    draw.with_clip_rect(viewport.min, viewport.max, || {
        let tint = [0.25, 0.85, 0.5, 0.12];
        if let Some(inner) = inner_bounds {
            for region in [
                OverlayRect {
                    min: bounds.min,
                    max: [bounds.max[0], inner.min[1]],
                },
                OverlayRect {
                    min: [bounds.min[0], inner.max[1]],
                    max: bounds.max,
                },
                OverlayRect {
                    min: [bounds.min[0], inner.min[1]],
                    max: [inner.min[0], inner.max[1]],
                },
                OverlayRect {
                    min: [inner.max[0], inner.min[1]],
                    max: [bounds.max[0], inner.max[1]],
                },
            ] {
                draw.add_rect(region.min, region.max, tint).filled(true).build();
            }
        } else {
            draw.add_rect(bounds.min, bounds.max, tint).filled(true).build();
        }
        draw.add_rect(bounds.min, bounds.max, BLOCK_SELECTION_SHADOW)
            .thickness(4.0)
            .build();
        if let Some(inner) = inner_bounds {
            draw.add_rect(inner.min, inner.max, BLOCK_SELECTION_SHADOW)
                .thickness(4.0)
                .build();
        }
        let offset = (ui.time() as f32 * BLOCK_STRIPE_SPEED).rem_euclid(BLOCK_STRIPE_LENGTH * 2.0);
        for border in std::iter::once(bounds).chain(inner_bounds) {
            for (start, end, green) in block_border_segments(border, viewport, offset) {
                draw.add_line(
                    start,
                    end,
                    if green {
                        BLOCK_SELECTION_GREEN
                    } else {
                        BLOCK_SELECTION_WHITE
                    },
                )
                .thickness(2.0)
                .build();
            }
        }
        let mode_label = match mode {
            BlockSelectionMode::Full => String::from("Full"),
            BlockSelectionMode::Hollow { line_width } => format!("Border {line_width}"),
        };
        let label = format!("{} × {} · {mode_label}", displayed.width(), displayed.height());
        let text_size = ui.calc_text_size(&label);
        let label_min = [
            bounds.min[0].max(viewport.min[0] + 4.0),
            (bounds.min[1] - text_size[1] - 18.0).max(viewport.min[1] + 4.0),
        ];
        draw.add_rect(
            [label_min[0] - 3.0, label_min[1] - 2.0],
            [label_min[0] + text_size[0] + 3.0, label_min[1] + text_size[1] + 2.0],
            OVERLAY_BG,
        )
        .filled(true)
        .build();
        draw.add_text(label_min, [1.0; 4], label);
    });
}

fn hollow_selection_inner(selection: Selection, mode: BlockSelectionMode) -> Option<Selection> {
    let BlockSelectionMode::Hollow { line_width } = mode else {
        return None;
    };
    let line_width = line_width.max(1);
    let min = Coord::new(
        selection.min.x.checked_add(line_width)?,
        selection.min.y.checked_add(line_width)?,
        selection.min.z,
    );
    let max = Coord::new(
        selection.max.x.checked_sub(line_width)?,
        selection.max.y.checked_sub(line_width)?,
        selection.max.z,
    );

    (min.x <= max.x && min.y <= max.y).then_some(Selection { min, max })
}

fn block_border_segments(bounds: OverlayRect, clip: OverlayRect, offset: f32) -> Vec<([f32; 2], [f32; 2], bool)> {
    let width = (bounds.max[0] - bounds.min[0]).max(0.0);
    let height = (bounds.max[1] - bounds.min[1]).max(0.0);
    let perimeter = 2.0 * (width + height);
    if perimeter <= f32::EPSILON {
        return Vec::new();
    }

    let mut segments = Vec::new();
    for (path_start, start, end) in [
        (0.0, bounds.min, [bounds.max[0], bounds.min[1]]),
        (width, [bounds.max[0], bounds.min[1]], bounds.max),
        (width + height, bounds.max, [bounds.min[0], bounds.max[1]]),
        (width * 2.0 + height, [bounds.min[0], bounds.max[1]], bounds.min),
    ] {
        if let Some((visible_start, visible_end)) = visible_edge_interval(start, end, path_start, clip) {
            let mut stripe = ((visible_start - offset) / BLOCK_STRIPE_LENGTH).floor() as i64;
            let mut part_start = visible_start;
            while part_start < visible_end {
                let stripe_end = offset + (stripe + 1) as f32 * BLOCK_STRIPE_LENGTH;
                let part_end = if stripe_end.is_finite() && stripe_end > part_start {
                    stripe_end.min(visible_end)
                } else {
                    visible_end
                };
                let green = stripe.rem_euclid(2) != 0;
                segments.push((
                    block_border_point(bounds, part_start),
                    block_border_point(bounds, part_end),
                    green,
                ));
                part_start = part_end;
                stripe += 1;
            }
        }
    }

    segments
}

fn visible_edge_interval(start: [f32; 2], end: [f32; 2], path_start: f32, clip: OverlayRect) -> Option<(f32, f32)> {
    if start[1] == end[1] {
        if !(clip.min[1]..=clip.max[1]).contains(&start[1]) {
            return None;
        }
        let low = start[0].min(end[0]).max(clip.min[0]);
        let high = start[0].max(end[0]).min(clip.max[0]);
        if high <= low {
            return None;
        }
        let (local_start, local_end) = if end[0] >= start[0] {
            (low - start[0], high - start[0])
        } else {
            (start[0] - high, start[0] - low)
        };

        Some((path_start + local_start, path_start + local_end))
    } else {
        if !(clip.min[0]..=clip.max[0]).contains(&start[0]) {
            return None;
        }
        let low = start[1].min(end[1]).max(clip.min[1]);
        let high = start[1].max(end[1]).min(clip.max[1]);
        if high <= low {
            return None;
        }
        let (local_start, local_end) = if end[1] >= start[1] {
            (low - start[1], high - start[1])
        } else {
            (start[1] - high, start[1] - low)
        };

        Some((path_start + local_start, path_start + local_end))
    }
}

fn block_border_point(bounds: OverlayRect, distance: f32) -> [f32; 2] {
    let width = (bounds.max[0] - bounds.min[0]).max(0.0);
    let height = (bounds.max[1] - bounds.min[1]).max(0.0);
    if distance <= width {
        [bounds.min[0] + distance, bounds.min[1]]
    } else if distance <= width + height {
        [bounds.max[0], bounds.min[1] + distance - width]
    } else if distance <= width * 2.0 + height {
        [bounds.max[0] - (distance - width - height), bounds.max[1]]
    } else {
        [bounds.min[0], bounds.max[1] - (distance - width * 2.0 - height)]
    }
}

fn draw_block_selection_menu(ui: &Ui, session: &mut Session, mode: BlockSelectionMode) {
    let mut requested = None;
    if let Some(_popup) = ui.begin_popup(BLOCK_SELECTION_POPUP) {
        for (label, transform) in [
            ("Mirror horizontally", SelectionTransform::MirrorHorizontal),
            ("Mirror vertically", SelectionTransform::MirrorVertical),
        ] {
            let enabled = session.can_transform_selected_block_with_mode(transform, mode);
            if ui.menu_item_enabled_selected_no_shortcut(label, false, enabled) {
                requested = Some(transform);
            }
        }
    }
    if let Some(transform) = requested {
        session.transform_selected_block_with_mode(transform, mode);
    }
}

fn block_controls_placement(
    tool: Tool, selecting: bool, rotation_open: bool, selection: Option<Selection>,
    pending: Option<PendingBlockPlacement>,
) -> Option<PendingBlockPlacement> {
    (matches!(tool, Tool::BlockSelect | Tool::Fill) && !selecting && !rotation_open)
        .then(|| {
            selection.map(|source| {
                pending.unwrap_or(PendingBlockPlacement {
                    source,
                    target: source,
                    rotation: SelectionRotation::Original,
                })
            })
        })
        .flatten()
}

fn block_placement_controls_layout(
    ui: &Ui, camera: &Controller, controls: PlacementControls, target: Selection, tile_size: u32,
    viewport_min: [f32; 2], controls_bounds: OverlayRect,
) -> ([f32; 2], OverlayRect) {
    let labels = controls.labels();
    let rows = controls.rows();
    let style = ui.clone_style();
    let padding = style.frame_padding();
    let spacing = style.item_spacing()[0];
    let width = labels
        .iter()
        .map(|label| ui.calc_text_size(*label)[0] + padding[0] * 2.0)
        .sum::<f32>()
        + spacing * labels.len().saturating_sub(1) as f32;
    let width = if rows > 1 { width.max(280.0) } else { width };
    let height = ui.frame_height() * rows as f32 + style.item_spacing()[1] * rows.saturating_sub(1) as f32;
    let selection = block_selection_bounds(camera, viewport_min, target, tile_size);
    let center = [
        (selection.min[0] + selection.max[0]) * 0.5,
        (selection.min[1] + selection.max[1]) * 0.5,
    ];
    let min_x = controls_bounds.min[0] + OVERLAY_PADDING;
    let min_y = controls_bounds.min[1] + OVERLAY_PADDING;
    let max_x = (controls_bounds.max[0] - width - OVERLAY_PADDING).max(min_x);
    let max_y = (controls_bounds.max[1] - height - OVERLAY_PADDING).max(min_y);
    let position = [
        (center[0] - width * 0.5).clamp(min_x, max_x),
        (if rows > 1 {
            if selection.max[1] + 20.0 <= max_y {
                selection.max[1] + 20.0
            } else if selection.min[1] - height - 20.0 >= min_y {
                selection.min[1] - height - 20.0
            } else {
                max_y
            }
        } else {
            center[1] + 14.0
        })
        .clamp(min_y, max_y),
    ];
    let bounds = OverlayRect {
        min: [position[0] - OVERLAY_PADDING, position[1] - OVERLAY_PADDING],
        max: [
            position[0] + width + OVERLAY_PADDING,
            position[1] + height + OVERLAY_PADDING,
        ],
    };

    (position, bounds)
}

fn draw_block_placement_controls(
    ui: &Ui, camera: &Controller, placement: PendingBlockPlacement, viewport_min: [f32; 2],
    controls_bounds: OverlayRect, session: &mut Session, options: &mut BlockSelectionOptions,
) -> Option<BlockPlacementAction> {
    let (position, bounds) = block_placement_controls_layout(
        ui,
        camera,
        PlacementControls::Block,
        placement.target,
        session.options.tile_size,
        viewport_min,
        controls_bounds,
    );
    draw_overlay_underlay(ui, bounds);
    ui.set_cursor_screen_pos(position);

    let mode = session.selection_mode();
    let mode_label = match mode {
        BlockSelectionMode::Full => String::from("Full"),
        BlockSelectionMode::Hollow { line_width } => format!("Border ({line_width})"),
    };
    ui.align_text_to_frame_padding();
    ui.text(format!(
        "{} × {} · {mode_label}",
        placement.target.width(),
        placement.target.height()
    ));
    let row_height = ui.frame_height() + ui.clone_style().item_spacing()[1];
    ui.set_cursor_screen_pos([position[0], position[1] + row_height]);
    let pending = placement.target != placement.source || placement.rotation != SelectionRotation::Original;
    ui.with_disabled_if(pending, || draw_selection_mode_controls(ui, session, options));
    let mode = session.selection_mode();
    let can_move = session.can_place_selected_block_with_mode(
        placement.target.min,
        placement.rotation,
        SelectionPlacement::Move,
        mode,
    );
    let can_fill = session.can_fill_selected_block(placement.target.min, placement.rotation, mode);

    let mut action = None;
    ui.set_cursor_screen_pos([position[0], position[1] + row_height * 2.0]);
    if session.tool() == Tool::BlockSelect {
        if ui.button(BLOCK_PLACEMENT_LABELS[0]) {
            action = Some(BlockPlacementAction::Move);
        }
        ui.same_line();
        if ui.button(BLOCK_PLACEMENT_LABELS[1]) {
            action = Some(BlockPlacementAction::Copy);
        }
        ui.same_line();
        if ui.with_disabled_if(!can_fill, || ui.button(BLOCK_PLACEMENT_LABELS[2])) {
            action = Some(BlockPlacementAction::Fill);
        }
        ui.set_item_tooltip(session.palette().map_or_else(
            || String::from("Choose a type to fill the selection"),
            |prefab| format!("Fill selected tiles with {}", prefab.path),
        ));
        ui.same_line();
    }
    if ui.button(if pending { "Cancel" } else { "Clear selection" }) {
        action = Some(BlockPlacementAction::Cancel);
    }

    if action.is_none()
        && ui.is_window_focused()
        && !ui.io().want_text_input()
        && !ui.is_popup_open_with_flags("", dear_imgui_rs::PopupQueryFlags::ANY_POPUP)
        && ui.is_key_pressed(Key::Enter)
        && can_move
    {
        action = Some(BlockPlacementAction::Move);
    }

    action
}

fn draw_selection_mode_controls(ui: &Ui, session: &mut Session, options: &mut BlockSelectionOptions) {
    let current = session.selection_mask();
    let mode = current.map_or(options.mode(), |mask| mask.mode);
    let mut next = *options;
    next.full_rectangle = mode == BlockSelectionMode::Full;
    if let BlockSelectionMode::Hollow { line_width } = mode {
        next.line_width = line_width.min(i32::MAX as u32) as i32;
    }
    let mut changed = false;
    for (label, full) in [("Border", false), ("Full", true)] {
        if ui.radio_button(label, next.full_rectangle == full) {
            next.full_rectangle = full;
            changed = true;
        }
        ui.same_line();
    }
    if !next.full_rectangle {
        ui.set_next_item_width(100.0);
        changed |= ui
            .drag_int_config("##selection-border-width")
            .range(1, i32::MAX)
            .flags(DragFlags::ALWAYS_CLAMP)
            .build(ui, &mut next.line_width);
        ui.set_item_tooltip("Border width in tiles");
    } else {
        ui.text(" ");
    }
    if changed {
        if current.is_none_or(|mask| {
            session.try_select_block(SelectionMask {
                bounds: mask.bounds,
                mode: next.mode(),
            })
        }) {
            *options = next;
        } else {
            ui.tooltip_text("This selection would extend outside the focused area");
        }
    }
}

fn paste_controls(rotation_open: bool, target: Option<Selection>) -> Option<Selection> {
    (!rotation_open).then_some(target).flatten()
}

fn draw_paste_controls(
    ui: &Ui, camera: &Controller, target: Selection, tile_size: u32, viewport_min: [f32; 2],
    controls_bounds: OverlayRect, can_paste: bool,
) -> Option<PasteAction> {
    let (position, bounds) = block_placement_controls_layout(
        ui,
        camera,
        PlacementControls::Paste,
        target,
        tile_size,
        viewport_min,
        controls_bounds,
    );
    draw_overlay_underlay(ui, bounds);
    ui.set_cursor_screen_pos(position);

    let mut action = None;
    if ui.with_disabled_if(!can_paste, || ui.button(PASTE_LABELS[0])) {
        action = Some(PasteAction::Paste);
    }
    ui.same_line();
    if ui.button(PASTE_LABELS[1]) {
        action = Some(PasteAction::Cancel);
    }
    if action.is_none()
        && ui.is_window_focused()
        && !ui.io().want_text_input()
        && !ui.is_popup_open_with_flags("", dear_imgui_rs::PopupQueryFlags::ANY_POPUP)
        && ui.is_key_pressed(Key::Enter)
        && can_paste
    {
        action = Some(PasteAction::Paste);
    }

    action
}

fn centered_paste_min(width: u32, height: u32, anchor: Coord, map_size: Size) -> Coord {
    let place = |anchor: u32, extent: u32, limit: u32| {
        let extent = extent.max(1);
        let last = limit.saturating_sub(extent - 1).max(1);

        anchor.saturating_sub((extent - 1) / 2).max(1).min(last)
    };

    Coord::new(
        place(anchor.x, width, map_size.x),
        place(anchor.y, height, map_size.y),
        anchor.z,
    )
}

fn configure_tool_interaction(tool: Tool, interaction: &mut MapViewInteraction) {
    interaction.mode = match (tool, interaction.mode) {
        (Tool::Place | Tool::BlockSelect | Tool::Fill, _) => InteractionMode::Place,
        (Tool::Select, InteractionMode::Select { pick }) => InteractionMode::Select { pick },
        (Tool::Delete, InteractionMode::Delete { pick }) => InteractionMode::Delete { pick },
        (Tool::Select, _) => InteractionMode::Select { pick: None },
        (Tool::Delete, _) => InteractionMode::Delete { pick: None },
    };
    match tool {
        Tool::Place | Tool::BlockSelect | Tool::Fill => {
            interaction.cursor = None;
            interaction.hovered_area = None;
            interaction.selected = None;
            interaction.selection_guide = None;
        },
        Tool::Delete => {
            interaction.selected = None;
            interaction.selection_guide = None;
        },
        Tool::Select => {},
    }
}

fn interaction_mode(tool: Tool) -> InteractionMode {
    match tool {
        Tool::Place | Tool::BlockSelect | Tool::Fill => InteractionMode::Place,
        Tool::Select => InteractionMode::Select { pick: None },
        Tool::Delete => InteractionMode::Delete { pick: None },
    }
}

fn request_pick(interaction: &mut MapViewInteraction, request: PickRequest) {
    match &mut interaction.mode {
        InteractionMode::Place => {},
        InteractionMode::Select { pick } | InteractionMode::Delete { pick } => *pick = Some(request),
    }
}

fn framebuffer_rect(origin: [f32; 2], size: [f32; 2], scale: [f32; 2], target: [f32; 2]) -> MapViewRect {
    let limit = |value: f32, max: f32| value.max(0.0).min(max.max(0.0)).floor() as u32;
    let (max_x, max_y) = (target[0] * scale[0], target[1] * scale[1]);
    MapViewRect {
        x: limit(origin[0] * scale[0], max_x),
        y: limit(origin[1] * scale[1], max_y),
        width: limit(size[0] * scale[0], u32::MAX as f32),
        height: limit(size[1] * scale[1], u32::MAX as f32),
    }
}

fn cursor_in_map_view(local: [f32; 2], scale: [f32; 2]) -> [u32; 2] {
    [
        (local[0] * scale[0]).max(0.0) as u32,
        (local[1] * scale[1]).max(0.0) as u32,
    ]
}

struct TopOverlayState<'a> {
    keybindings: KeyBindings,
    selection_busy: bool,
    block_selection_options: &'a mut BlockSelectionOptions,
    fill_mode: &'a mut FillMode,
    custom_fill_boundaries: &'a mut Vec<TreePath>,
    custom_fill_search: &'a mut String,
    new_level_dialog: &'a mut Option<NewLevelDialog>,
    new_level_type_path: &'a str,
}

fn draw_top_overlay(ui: &Ui, session: &mut Session, bounds: OverlayRect, state: TopOverlayState<'_>) {
    let TopOverlayState {
        keybindings,
        selection_busy,
        block_selection_options,
        fill_mode,
        custom_fill_boundaries,
        custom_fill_search,
        new_level_dialog,
        new_level_type_path,
    } = state;
    draw_overlay_underlay(ui, bounds);

    ui.set_cursor_screen_pos([bounds.min[0] + OVERLAY_PADDING, bounds.min[1] + OVERLAY_PADDING]);

    draw_tool_button(ui, session, Tool::Place, ICON_PENCIL);
    ui.same_line();
    draw_tool_button(ui, session, Tool::Select, ICON_EYEDROPPER);
    ui.same_line();
    draw_block_select_tool_button(ui, session, block_selection_options, keybindings, selection_busy);
    ui.same_line();
    draw_tool_button(ui, session, Tool::Delete, ICON_ERASER);
    ui.same_line();
    let tools_end = draw_fill_tool_button(
        ui,
        session,
        fill_mode,
        custom_fill_boundaries,
        custom_fill_search,
        keybindings,
    );

    let levels_width = z_level_width(ui, session.level_count());
    let levels_x = (bounds.max[0] - OVERLAY_PADDING - levels_width).max(tools_end + OVERLAY_PADDING);
    ui.set_cursor_screen_pos([levels_x, bounds.min[1] + OVERLAY_PADDING]);
    draw_z_levels(ui, session, new_level_dialog, new_level_type_path);
}

fn draw_new_map_dialog(ui: &Ui, session: &mut Session, dialog: &mut Option<NewMapDialog>) -> (bool, bool) {
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

fn draw_new_level_dialog(
    ui: &Ui, session: &mut Session, dialog: &mut Option<NewLevelDialog>, remembered_type_path: &mut String,
) {
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
        ui.text("Fill type path");
        ui.set_next_item_width(NEW_LEVEL_PATH_WIDTH);
        if ui.is_window_appearing() {
            ui.set_keyboard_focus_here();
        }
        let submitted = ui
            .input_text("##new-z-level-type-path", &mut state.type_path)
            .hint("/turf")
            .enter_returns_true(true)
            .build();

        let mut selected_path = None;
        if state.type_path.trim().is_empty() {
            ui.text_disabled("Type a path to search");
        } else if let Some(tree) = session.tree() {
            let matches = matching_type_paths(tree, &state.type_path);
            if matches.is_empty() {
                ui.text_disabled("No matching types");
            } else {
                ui.child_window("new-z-level-search-results")
                    .size([NEW_LEVEL_PATH_WIDTH, NEW_LEVEL_SEARCH_HEIGHT])
                    .border(true)
                    .build(ui, || {
                        for path in matches {
                            let label = path.to_string();
                            if ui.selectable_config(&label).selected(state.type_path == label).build() {
                                selected_path = Some(label);
                            }
                        }
                    });
            }
        } else {
            ui.text_disabled("No environment loaded");
        }

        if let Some(path) = selected_path {
            state.type_path = path;
            state.error = None;
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

        let can_create = !state.type_path.trim().is_empty();
        let clicked = {
            let _disabled = ui.begin_disabled_with_cond(!can_create);

            ui.button("Create")
        };
        if can_create && (clicked || submitted) {
            match session.create_level(state.document, &state.type_path) {
                Ok(_) => {
                    remembered_type_path.clone_from(&state.type_path);
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

fn draw_save_dialog(ui: &Ui, session: &mut Session, dialog: &mut Option<SaveDialog>) {
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

fn draw_fill_limit_warning(
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

fn draw_block_select_tool_button(
    ui: &Ui, session: &mut Session, options: &mut BlockSelectionOptions, keybindings: KeyBindings, selection_busy: bool,
) {
    const POPUP: &str = "block-selection-options-popup";

    let active_color = ui.style_color(StyleColor::PlotHistogramHovered);
    let active = (session.tool() == Tool::BlockSelect).then(|| ui.push_style_color(StyleColor::Button, active_color));
    let spacing = ui.clone_style().item_spacing();
    let connected = ui.push_style_var(StyleVar::ItemSpacing([0.0, spacing[1]]));

    if ui.button(format!("{ICON_SELECT_DRAG}##block-select-tool")) {
        session.set_tool(Tool::BlockSelect);
    }
    let separator_x = ui.item_rect_max()[0];
    let separator_min_y = ui.item_rect_min()[1];
    let separator_max_y = ui.item_rect_max()[1];
    ui.set_item_tooltip(format!(
        "Rectangle selection ({})\nDrag: {} · Hold Shift while drawing: Border\nShift+gizmo or edge handles: resize",
        keybindings.get(KeybindAction::BlockSelectTool).label(ui),
        options.label()
    ));
    ui.same_line();

    let button_size = ui.frame_height();
    let arrow_width = (button_size * 0.65)
        .ceil()
        .max((ui.calc_text_size(ICON_MENU_DOWN.to_string())[0] + 2.0).ceil());
    let frame_padding = ui.clone_style().frame_padding();
    let arrow_clicked = {
        let _padding = ui.push_style_var(StyleVar::FramePadding([0.0, frame_padding[1]]));
        let _alignment = ui.push_style_var(StyleVar::ButtonTextAlign([0.5, 0.5]));
        ui.button_with_size(
            format!("{ICON_MENU_DOWN}##block-fill-options"),
            [arrow_width, button_size],
        )
    };
    if arrow_clicked {
        ui.open_popup(POPUP);
    }
    ui.set_item_tooltip(format!("Block selection: {}", options.label()));
    ui.get_window_draw_list().add_line_v(
        separator_x,
        separator_min_y + frame_padding[1],
        separator_max_y - frame_padding[1],
        [0.0, 0.0, 0.0, 0.7],
        1.0,
    );

    drop(connected);
    drop(active);

    if let Some(_popup) = ui.begin_popup(POPUP) {
        ui.with_disabled_if(selection_busy, || draw_selection_mode_controls(ui, session, options));
        ui.text("Hold Shift while drawing for Border.");
        ui.text("Drag edge handles or Shift+gizmo to resize.");
        if selection_busy {
            ui.text("Finish or cancel the current gesture to change mode.");
        }
    }
}

fn draw_fill_tool_button(
    ui: &Ui, session: &mut Session, fill_mode: &mut FillMode, custom_fill_boundaries: &mut Vec<TreePath>,
    custom_fill_search: &mut String, keybindings: KeyBindings,
) -> f32 {
    const POPUP: &str = "fill-mode-popup";

    let active_color = ui.style_color(StyleColor::PlotHistogramHovered);
    let active = (session.tool() == Tool::Fill).then(|| ui.push_style_color(StyleColor::Button, active_color));
    let spacing = ui.clone_style().item_spacing();
    let connected = ui.push_style_var(StyleVar::ItemSpacing([0.0, spacing[1]]));

    if ui.button(format!("{ICON_FORMAT_COLOR_FILL}##fill-tool")) {
        session.set_tool(Tool::Fill);
    }
    let separator_x = ui.item_rect_max()[0];
    let separator_min_y = ui.item_rect_min()[1];
    let separator_max_y = ui.item_rect_max()[1];
    ui.set_item_tooltip(format!(
        "Bucket ({}) · {}\n{}",
        keybindings.get(KeybindAction::FillTool).label(ui),
        fill_mode.label(),
        if session.selection().is_some() {
            "Fills connected selected tiles. Clear selection to fill elsewhere"
        } else {
            "Click to fill a connected region"
        }
    ));
    ui.same_line();

    let button_size = ui.frame_height();
    let arrow_width = (button_size * 0.65)
        .ceil()
        .max((ui.calc_text_size(ICON_MENU_DOWN.to_string())[0] + 2.0).ceil());
    let frame_padding = ui.clone_style().frame_padding();
    let arrow_clicked = {
        let _padding = ui.push_style_var(StyleVar::FramePadding([0.0, frame_padding[1]]));
        let _alignment = ui.push_style_var(StyleVar::ButtonTextAlign([0.5, 0.5]));
        ui.button_with_size(format!("{ICON_MENU_DOWN}##fill-mode"), [arrow_width, button_size])
    };
    if arrow_clicked {
        ui.open_popup(POPUP);
    }
    let tools_end = ui.item_rect_max()[0];
    ui.set_item_tooltip(format!("Fill mode: {}", fill_mode.label()));
    ui.get_window_draw_list().add_line_v(
        separator_x,
        separator_min_y + frame_padding[1],
        separator_max_y - frame_padding[1],
        [0.0, 0.0, 0.0, 0.7],
        1.0,
    );

    drop(connected);
    drop(active);

    if let Some(_popup) = ui.begin_popup(POPUP) {
        for mode in [FillMode::Wall, FillMode::EntireArea, FillMode::Custom] {
            if ui.menu_item_enabled_selected_no_shortcut(mode.label(), *fill_mode == mode, true) {
                *fill_mode = mode;
                ui.close_current_popup();
            }
        }

        ui.separator();
        let selected = session.palette().map(|prefab| prefab.path.clone());
        let can_add = selected
            .as_ref()
            .is_some_and(|path| !custom_fill_boundaries.contains(path));
        let add_label = selected
            .as_ref()
            .map_or_else(|| "Add selected type".to_owned(), |path| format!("Add {path}"));
        if ui.menu_item_enabled_selected_no_shortcut(add_label, false, can_add)
            && let Some(path) = selected
        {
            custom_fill_boundaries.push(path);
            *fill_mode = FillMode::Custom;
        }

        let mut searched_path = None;
        if let Some(_menu) = ui.begin_menu("Search type paths") {
            ui.set_next_item_width(320.0);
            ui.input_text("##custom-fill-boundary-search", custom_fill_search)
                .build();

            if custom_fill_search.trim().is_empty() {
                ui.text_disabled("Type a path to search");
            } else if let Some(tree) = session.tree() {
                let matches = matching_type_paths(tree, custom_fill_search);
                if matches.is_empty() {
                    ui.text_disabled("No matching types");
                } else {
                    for path in matches {
                        let enabled = !custom_fill_boundaries.contains(&path);
                        if ui.menu_item_enabled_selected_no_shortcut(path.to_string(), false, enabled) {
                            searched_path = Some(path);
                        }
                    }
                }
            } else {
                ui.text_disabled("No environment loaded");
            }
        }

        if let Some(path) = searched_path {
            custom_fill_boundaries.push(path);
            *fill_mode = FillMode::Custom;
        }

        let mut remove = None;
        if let Some(_menu) = ui.begin_menu_with_enabled(
            format!("Remove boundary ({})", custom_fill_boundaries.len()),
            !custom_fill_boundaries.is_empty(),
        ) {
            for (index, path) in custom_fill_boundaries.iter().enumerate() {
                if ui.menu_item(format!("{path}##custom-fill-boundary-{index}")) {
                    remove = Some(index);
                }
            }
        }
        if let Some(index) = remove {
            custom_fill_boundaries.remove(index);
        }

        let default = TreePath::parse(DEFAULT_CUSTOM_FILL_BOUNDARY);
        let is_default = custom_fill_boundaries.as_slice() == [default.clone()];
        if ui.menu_item_enabled_selected_no_shortcut("Reset custom boundaries", false, !is_default) {
            custom_fill_boundaries.clear();
            custom_fill_boundaries.push(default);
        }
    }

    tools_end
}

fn matching_type_paths(tree: &ObjectTree, query: &str) -> Vec<TreePath> {
    matching_type_paths_up_to(tree, query, MAX_CUSTOM_FILL_SEARCH_RESULTS)
}

// im not sure why every single fucking icons appear not centered fuck you

fn matching_type_paths_up_to(tree: &ObjectTree, query: &str, limit: usize) -> Vec<TreePath> {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return Vec::new();
    }

    let mut matches = tree
        .iter()
        .filter(|decl| decl.path.to_string().to_ascii_lowercase().contains(&query))
        .map(|decl| decl.path.clone())
        .take(limit)
        .collect::<Vec<_>>();
    matches.sort_by_key(ToString::to_string);

    matches
}

fn draw_tool_button(ui: &Ui, session: &mut Session, tool: Tool, icon: char) {
    let color = match tool {
        Tool::Delete => [1.0, 0.0, 0.0, 1.0],
        _ => ui.style_color(StyleColor::PlotHistogramHovered),
    };

    let _color = (session.tool() == tool).then(|| ui.push_style_color(StyleColor::Button, color));
    let clicked = ui.button(icon.to_string());
    ui.set_item_tooltip(match tool {
        Tool::BlockSelect => "Block Select",
        _ => tool.label(),
    });
    if clicked {
        session.set_tool(tool);
    }
}

fn draw_history_overlay(
    ui: &Ui, session: &mut Session, keybindings: KeyBindings, bounds: OverlayRect, recent_prefabs: Vec<Prefab>,
) {
    draw_overlay_underlay(ui, bounds);

    let button_size = recent_button_size(ui);
    let palette = session.palette().cloned();
    let mut chosen = None;
    ui.set_cursor_screen_pos([bounds.min[0] + OVERLAY_PADDING, bounds.min[1] + OVERLAY_PADDING]);

    for (index, prefab) in recent_prefabs.iter().enumerate() {
        if index > 0 {
            ui.same_line();
        }
        let key = keybindings
            .recent(index)
            .map(|binding| binding.label(ui))
            .unwrap_or_else(|| String::from("?"));
        let _selected = (palette.as_ref() == Some(prefab))
            .then(|| ui.push_style_color(StyleColor::Button, ui.style_color(StyleColor::ButtonActive)));
        let clicked = match session.prefab_thumbnail(prefab) {
            Some(thumbnail) => {
                let image_size = fit_recent_icon(thumbnail.texture.width, thumbnail.texture.height);
                let padding = [(button_size - image_size[0]) * 0.5, (button_size - image_size[1]) * 0.5];
                let _padding = ui.push_style_var(StyleVar::FramePadding(padding));

                ui.image_button_config(
                    format!("recent-{index}"),
                    Renderer::sprite_texture(thumbnail.texture.index),
                    image_size,
                )
                .uv0(thumbnail.uv0)
                .uv1(thumbnail.uv1)
                .tint_color(thumbnail.tint)
                .build()
            },
            None => ui.button_with_size(format!("{ICON_IMAGE_BROKEN}##recent-{index}"), [button_size; 2]),
        };
        draw_recent_badge(ui, &key);
        if clicked {
            chosen = Some(index);
        }
        ui.set_item_tooltip(prefab_tooltip(prefab));
    }

    if let Some(index) = chosen {
        session.choose_recent(index);
    }
}

fn has_modifiers(ui: &Ui) -> bool {
    let io = ui.io();

    io.key_ctrl() || io.key_shift() || io.key_alt() || io.key_super()
}

fn recent_button_size(ui: &Ui) -> f32 {
    let padding = ui.clone_style().frame_padding();

    RECENT_ICON_SIZE + padding[0].max(padding[1]) * 2.0
}

fn fit_recent_icon(width: u32, height: u32) -> [f32; 2] { fit_icon(width, height, RECENT_ICON_SIZE) }

fn fit_icon(width: u32, height: u32, extent: f32) -> [f32; 2] {
    let width = width.max(1) as f32;
    let height = height.max(1) as f32;
    let scale = extent.max(1.0) / width.max(height);

    [width * scale, height * scale]
}

fn draw_recent_badge(ui: &Ui, key: &str) {
    let item_min = ui.item_rect_min();
    let text_size = ui.calc_text_size(key);
    let badge_min = [item_min[0] + 2.0, item_min[1] + 2.0];
    let badge_max = [badge_min[0] + text_size[0] + 4.0, badge_min[1] + text_size[1] + 2.0];
    let draw = ui.get_window_draw_list();
    draw.add_rect(badge_min, badge_max, RECENT_BADGE_BG)
        .rounding(2.0)
        .filled(true)
        .build();
    draw.add_text([badge_min[0] + 2.0, badge_min[1] + 1.0], RECENT_BADGE_TEXT, key);
}

fn prefab_tooltip(prefab: &Prefab) -> String {
    let mut tooltip = prefab.path.to_string();
    for (name, value) in &prefab.vars {
        tooltip.push_str(&format!("\n{name} = {}", dmm::writer::format_value(&value.value)));
    }

    tooltip
}

fn z_level_width(ui: &Ui, levels: u32) -> f32 {
    let style = ui.clone_style();
    let item_spacing = style.item_spacing()[0];
    let button = ui.frame_height();
    let label = ui.calc_text_size("Z:")[0];
    let level = ui.calc_text_size(levels.to_string())[0];

    label + item_spacing * 3.0 + button * 2.0 + level
}

fn panel_extent(available: [f32; 2]) -> ([f32; 2], (u32, u32)) {
    let width = if available[0].is_finite() {
        available[0].max(1.0)
    } else {
        1.0
    };
    let height = if available[1].is_finite() {
        available[1].max(1.0)
    } else {
        1.0
    };

    ([width, height], (width.floor() as u32, height.floor() as u32))
}

#[cfg(test)]
static IMGUI_CONTEXT: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use core::{location::Location, path::TreePath};

    use dmm::{PrefabInstanceId, Size};
    use render::{HighlightStyle, SpriteTexture};

    use super::*;
    use crate::settings::KeyBinding;

    fn rectangle_context() -> dear_imgui_rs::Context {
        let mut context = dear_imgui_rs::Context::create();
        context.set_ini_filename(None::<PathBuf>).unwrap();
        context.font_atlas().try_claim_legacy_renderer().unwrap().build();
        context.io_mut().set_display_size([800.0, 600.0]);
        context.io_mut().set_delta_time(1.0 / 60.0);
        context.io_mut().set_config_input_trickle_event_queue(false);
        context
    }

    struct RectangleUiHarness {
        context: dear_imgui_rs::Context,
        state: UiState,
        session: Session,
        settings: Settings,
        view: MapViewState,
        id: DocumentId,
        fill_button: Option<[f32; 2]>,
        mode_buttons: Option<([f32; 2], [f32; 2])>,
    }

    impl RectangleUiHarness {
        fn new() -> Self {
            let context = rectangle_context();
            let mut session = Session::new();
            session
                .load_environment(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env/test.dme"))
                .unwrap();
            let mut map = dmm::Map::new(Size { x: 20, y: 20, z: 1 });
            let base = map.intern_tile(vec![
                Prefab::new(TreePath::parse("/turf/open/floor")),
                Prefab::new(TreePath::parse("/area/station")),
            ]);
            for row in &mut map.grid[0] {
                row.fill(base);
            }
            session.apply_map(crate::loader::LoadedMap {
                path: PathBuf::from("rectangle-ui-test.dmm"),
                map,
                z: 1,
                errors: vec![],
            });
            let id = session.state.active().unwrap();
            let mut view = MapViewState::new(id).unwrap();
            view.refit = false;
            view.focus = true;
            view.camera.camera.x = 320.0;
            view.camera.camera.y = 320.0;
            let mut harness = Self {
                context,
                state: UiState::new(false).unwrap(),
                session,
                settings: Settings::default(),
                view,
                id,
                fill_button: None,
                mode_buttons: None,
            };
            for _ in 0..3 {
                harness.step();
            }
            harness
        }

        fn step(&mut self) {
            let ui = self.context.frame();
            let name = format!("###viewport-{}", self.id.get());
            ui.set_window_pos_by_name(&name, [0.0; 2]);
            ui.set_window_size_by_name(&name, [800.0, 600.0]);
            self.state.draw_map_view(
                ui,
                &mut self.session,
                &self.settings,
                MapViewDraw {
                    id: self.id,
                    index: 0,
                    view: &mut self.view,
                    interaction: &mut MapViewInteraction::default(),
                    refit_requested: false,
                    keep_open: &mut true,
                },
            );
            self.fill_button = None;
            if let Some(selection) = self.session.selection() {
                let min = [self.view.rect.x as f32, self.view.rect.y as f32];
                let max = [
                    min[0] + self.view.rect.width as f32,
                    min[1] + self.view.rect.height as f32,
                ];
                let controls = OverlayRect {
                    min: [min[0], min[1] + ui.frame_height() + 2.0 * OVERLAY_PADDING],
                    max: [max[0], max[1] - recent_button_size(ui) - 2.0 * OVERLAY_PADDING],
                };
                let (position, _) = block_placement_controls_layout(
                    ui,
                    &self.view.camera,
                    PlacementControls::Block,
                    selection,
                    self.session.options.tile_size,
                    min,
                    controls,
                );
                let style = ui.clone_style();
                let row_height = ui.frame_height() + style.item_spacing()[1];
                let button_width = |label: &str| ui.calc_text_size(label)[0] + style.frame_padding()[0] * 2.0;
                self.fill_button = Some([
                    position[0]
                        + button_width("Move")
                        + button_width("Copy")
                        + 2.0 * style.item_spacing()[0]
                        + button_width("Fill selection") * 0.5,
                    position[1] + 2.0 * row_height + ui.frame_height() * 0.5,
                ]);
                let radio_width =
                    |label: &str| ui.frame_height() + style.item_inner_spacing()[0] + ui.calc_text_size(label)[0];
                self.mode_buttons = Some((
                    [position[0] + 5.0, position[1] + row_height + 5.0],
                    [
                        position[0] + radio_width("Border") + style.item_spacing()[0] + 5.0,
                        position[1] + row_height + 5.0,
                    ],
                ));
            }
            assert!(self.context.render_legacy().valid());
        }

        fn tile(&self, x: u32, y: u32) -> [f32; 2] {
            let point = self
                .view
                .camera
                .map_to_screen([(x as f32 - 0.5) * 32.0, (y as f32 - 0.5) * 32.0]);
            [point[0] + self.view.rect.x as f32, point[1] + self.view.rect.y as f32]
        }

        fn pointer(&mut self, point: [f32; 2], down: bool) {
            self.context.io_mut().add_mouse_pos_event(point);
            self.context.io_mut().add_mouse_button_event(MouseButton::Left, down);
            self.step();
        }

        fn click(&mut self, point: [f32; 2]) {
            self.pointer(point, false);
            self.pointer(point, true);
            self.pointer(point, false);
        }

        fn key(&mut self, key: Key, down: bool) {
            self.context.io_mut().add_key_event(key, down);
            self.step();
        }
    }

    #[test]
    fn rectangle_ui_supports_drawing_resizing_filling_and_switching_to_the_bucket() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut app = RectangleUiHarness::new();
        app.pointer(app.tile(5, 8), false);
        app.key(Key::ModShift, true);
        app.key(Key::S, true);
        assert_eq!(app.session.tool(), Tool::BlockSelect);
        assert!(app.session.selection().is_none());
        app.key(Key::S, false);

        // Shift is still held from the shortcut, so the drag draws a border.
        app.pointer(app.tile(5, 8), true);
        app.pointer(app.tile(7, 10), true);
        app.pointer(app.tile(7, 10), false);
        app.key(Key::ModShift, false);
        let border = SelectionMask {
            bounds: Selection::from_drag(Coord::new(5, 8, 1), Coord::new(7, 10, 1)),
            mode: BlockSelectionMode::Hollow { line_width: 1 },
        };
        assert_eq!(app.session.selection_mask(), Some(border));
        assert_eq!(app.session.undo_label(), None);

        // Explicit mode controls change the current selection without painting.
        app.click(app.mode_buttons.unwrap().1);
        assert_eq!(app.session.selection_mode(), BlockSelectionMode::Full);
        app.click(app.mode_buttons.unwrap().0);
        assert_eq!(app.session.selection_mask(), Some(border));

        // A Shift-gizmo drag resizes bounds; releasing Shift mid-drag keeps the border.
        app.pointer(app.tile(6, 9), false);
        app.key(Key::ModShift, true);
        app.pointer(app.tile(6, 9), true);
        app.pointer(app.tile(8, 11), true);
        app.key(Key::ModShift, false);
        app.pointer(app.tile(8, 11), false);
        let resized = SelectionMask {
            bounds: Selection::from_drag(border.bounds.min, Coord::new(9, 12, 1)),
            ..border
        };
        assert_eq!(app.session.selection_mask(), Some(resized));
        assert!(app.view.block_placement.is_none());
        assert_eq!(app.session.undo_label(), None);

        let mut red = Prefab::new(TreePath::parse("/turf/open/floor"));
        red.set_var("color".into(), core::types::Value::Text("#ff0000".into()));
        app.session.state.choose_prefab(red.clone());
        app.step();
        app.click(app.fill_button.unwrap());
        assert!(
            app.session
                .map()
                .unwrap()
                .tile_at(resized.bounds.min)
                .unwrap()
                .contains(&red)
        );
        assert!(
            !app.session
                .map()
                .unwrap()
                .tile_at(Coord::new(7, 10, 1))
                .unwrap()
                .contains(&red)
        );

        app.pointer(app.tile(5, 8), false);
        app.key(Key::Q, true);
        app.key(Key::Q, false);
        assert_eq!(app.session.tool(), Tool::Fill);
        assert_eq!(app.session.selection_mask(), Some(resized));
        let mut blue = red.clone();
        blue.set_var("color".into(), core::types::Value::Text("#0000ff".into()));
        app.session.state.choose_prefab(blue.clone());
        app.click(app.tile(7, 10));
        assert!(
            !app.session
                .map()
                .unwrap()
                .tile_at(resized.bounds.min)
                .unwrap()
                .contains(&blue)
        );
        app.click(app.tile(5, 8));
        assert!(
            app.session
                .map()
                .unwrap()
                .tile_at(resized.bounds.min)
                .unwrap()
                .contains(&blue)
        );
        assert!(
            !app.session
                .map()
                .unwrap()
                .tile_at(Coord::new(4, 8, 1))
                .unwrap()
                .contains(&blue)
        );
        app.key(Key::Escape, true);
        app.key(Key::Escape, false);
        assert_eq!(app.session.selection_mask(), None);
        app.key(Key::ModCtrl, true);
        app.key(Key::Z, true);
        app.key(Key::Z, false);
        app.key(Key::ModCtrl, false);
        assert!(
            app.session
                .map()
                .unwrap()
                .tile_at(resized.bounds.min)
                .unwrap()
                .contains(&red)
        );
    }

    #[test]
    fn rectangle_ui_keeps_shift_draw_mode_and_restores_cancelled_gestures() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut app = RectangleUiHarness::new();
        app.session.set_tool(Tool::BlockSelect);
        app.pointer(app.tile(5, 8), true);
        app.pointer(app.tile(7, 10), true);
        assert_eq!(app.session.selection_mode(), BlockSelectionMode::Full);
        app.key(Key::ModShift, true);
        assert_eq!(
            app.session.selection_mode(),
            BlockSelectionMode::Hollow { line_width: 1 }
        );
        app.context.io_mut().add_key_event(Key::ModShift, false);
        app.pointer(app.tile(7, 10), false);
        let original = app.session.selection_mask().unwrap();
        assert_eq!(original.mode, BlockSelectionMode::Hollow { line_width: 1 });
        assert_eq!(app.state.block_selection_options.mode(), BlockSelectionMode::Full);

        app.pointer(app.tile(12, 12), true);
        app.pointer(app.tile(14, 14), true);
        assert_eq!(app.session.selection_mode(), BlockSelectionMode::Full);
        app.key(Key::Escape, true);
        assert_eq!(app.session.selection_mask(), Some(original));
        app.key(Key::Escape, false);
        app.pointer(app.tile(14, 14), false);
        assert_eq!(app.session.selection_mask(), Some(original));

        // A corner handle resizes without Shift, and tool changes cancel the active resize.
        let corner = app.tile(7, 10);
        let corner = [corner[0] + 23.0, corner[1] - 23.0];
        app.pointer(corner, false);
        app.pointer(corner, true);
        app.pointer([corner[0] + 32.0, corner[1] - 32.0], true);
        assert_eq!(app.session.selection().unwrap().max, Coord::new(8, 11, 1));
        assert!(app.view.block_placement.is_none());
        app.key(Key::Q, true);
        app.key(Key::Q, false);
        assert_eq!(app.session.tool(), Tool::Fill);
        assert_eq!(app.session.selection_mask(), Some(original));
        app.pointer(corner, false);
        assert_eq!(app.session.undo_label(), None);
        assert!(app.view.rectangle_gesture.is_none());
        app.click(app.mode_buttons.unwrap().0);
        assert_eq!(
            app.state.block_selection_options.mode(),
            BlockSelectionMode::Hollow { line_width: 1 }
        );
    }

    #[test]
    fn rectangle_ui_copies_a_border_to_another_map_and_keeps_it_hollow_after_paste() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut app = RectangleUiHarness::new();
        let source_id = app.id;
        app.session.set_tool(Tool::BlockSelect);
        app.key(Key::ModShift, true);
        app.pointer(app.tile(5, 8), true);
        app.pointer(app.tile(9, 14), true);
        app.pointer(app.tile(9, 14), false);
        app.key(Key::ModShift, false);
        let source = app.session.selection_mask().unwrap();
        assert_eq!(source.mode, BlockSelectionMode::Hollow { line_width: 1 });
        let mut red = Prefab::new(TreePath::parse("/turf/open/floor"));
        red.set_var("color".into(), core::types::Value::Text("#ff0000".into()));
        app.session.state.choose_prefab(red.clone());
        app.step();
        app.click(app.fill_button.unwrap());
        app.pointer(app.tile(5, 8), false);
        app.key(Key::ModCtrl, true);
        app.key(Key::C, true);
        app.key(Key::C, false);
        app.key(Key::ModCtrl, false);
        let clipboard = app.session.clipboard().unwrap().clone();
        assert_eq!(clipboard.filled().count(), source.mode.tiles(source.bounds).count());
        let source_grid = app.session.map().unwrap().grid.clone();

        let mut destination = dmm::Map::new(Size { x: 20, y: 20, z: 1 });
        let blue = Prefab::new(TreePath::parse("/turf/open/floor"));
        let key = destination.intern_tile(vec![blue.clone(), Prefab::new(TreePath::parse("/area/station"))]);
        for row in &mut destination.grid[0] {
            row.fill(key);
        }
        let destination_grid = destination.grid.clone();
        app.session.apply_map(crate::loader::LoadedMap {
            path: PathBuf::from("rectangle-ui-destination.dmm"),
            map: destination,
            z: 1,
            errors: vec![],
        });
        app.id = app.session.state.active().unwrap();
        let mut destination_view = MapViewState::new(app.id).unwrap();
        destination_view.refit = false;
        destination_view.focus = true;
        destination_view.camera.camera.x = 320.0;
        destination_view.camera.camera.y = 320.0;
        let source_view = std::mem::replace(&mut app.view, destination_view);
        app.state.map_views.insert(source_id, source_view);
        // Different destination settings and palette must not change what was copied.
        app.state.block_selection_options.full_rectangle = false;
        app.state.block_selection_options.line_width = 3;
        app.session.state.choose_prefab(blue.clone());
        for _ in 0..3 {
            app.step();
        }
        app.pointer(app.tile(12, 10), false);
        app.key(Key::ModCtrl, true);
        app.key(Key::V, true);
        app.key(Key::V, false);
        app.key(Key::ModCtrl, false);
        let pending = app.view.paste.expect("Ctrl+V starts a preview in the destination");
        let target = app
            .session
            .clipboard_selection_mask(pending.min, pending.rotation)
            .unwrap();
        assert_eq!(target.mode, source.mode);
        assert_eq!(
            app.session
                .clipboard_preview_sprites(target.bounds, pending.rotation)
                .len(),
            clipboard.filled().count()
        );
        assert_eq!(
            app.session.map().unwrap().grid,
            destination_grid,
            "preview does not paint"
        );
        app.key(Key::Enter, true);
        app.key(Key::Enter, false);
        assert!(app.view.paste.is_none());
        assert_eq!(app.session.selection_mask(), Some(target));
        for coord in target.bounds.iter() {
            let tile = app.session.map().unwrap().tile_at(coord).unwrap();
            assert_eq!(tile.contains(&red), target.includes(coord));
            assert_eq!(tile.contains(&blue), !target.includes(coord));
        }
        assert_eq!(app.session.state.document(source_id).unwrap().map.grid, source_grid);
        app.key(Key::ModCtrl, true);
        app.key(Key::Z, true);
        app.key(Key::Z, false);
        app.key(Key::ModCtrl, false);
        assert_eq!(app.session.map().unwrap().grid, destination_grid);
        assert_eq!(app.session.state.document(source_id).unwrap().map.grid, source_grid);
        app.key(Key::ModCtrl, true);
        app.key(Key::Y, true);
        app.key(Key::Y, false);
        app.key(Key::ModCtrl, false);
        assert_eq!(app.session.selection_mask(), Some(target));

        // The next Fill selection must still act on the copied border only.
        let mut green = blue.clone();
        green.set_var("color".into(), core::types::Value::Text("#00ff00".into()));
        app.session.state.choose_prefab(green.clone());
        app.step();
        app.click(app.fill_button.unwrap());
        for coord in target.bounds.iter() {
            assert_eq!(
                app.session.map().unwrap().tile_at(coord).unwrap().contains(&green),
                target.includes(coord)
            );
        }
    }

    fn gizmo_frame(
        context: &mut dear_imgui_rs::Context, gizmo: &mut GizmoState, camera: &Controller, settings: &Settings,
        target: BlockGizmoTarget, hovered: bool,
    ) -> crate::gizmo::BlockGizmoResponse {
        let ui = context.frame();
        let view = GizmoMapView {
            min: [0.0; 2],
            max: [800.0, 600.0],
            hovered,
        };
        let response = ui
            .window("rectangle-test")
            .position([0.0; 2], Condition::Always)
            .size([800.0, 600.0], Condition::Always)
            .build(|| {
                let response = gizmo.draw_block(ui, settings, camera, target, view);
                gizmo.draw_block_overlay(
                    ui,
                    camera,
                    BlockGizmoTarget {
                        selection: response.selection,
                        ..target
                    },
                    view,
                );
                response
            })
            .unwrap();
        assert!(context.render_legacy().valid());
        response
    }

    #[test]
    fn resize_and_move_gestures_keep_their_mouse_down_action_when_shift_changes() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        for resize in [true, false] {
            let mut context = rectangle_context();
            let mut camera = Controller::new();
            camera.resize(800, 600);
            camera.center_on_tile(Coord::new(5, 5, 1), 32);
            let selection = Selection::from_drag(Coord::new(3, 3, 1), Coord::new(5, 5, 1));
            let bounds = block_selection_bounds(&camera, [0.0; 2], selection, 32);
            let origin = [
                (bounds.min[0] + bounds.max[0]) * 0.5,
                (bounds.min[1] + bounds.max[1]) * 0.5,
            ];
            let mut target = BlockGizmoTarget {
                selection,
                rotation: SelectionRotation::Original,
                map_size: Size { x: 10, y: 10, z: 1 },
                tile_size: 32,
                kind: BlockGizmoKind::Selection,
            };
            let mut gizmo = GizmoState::default();
            let settings = Settings::default();
            context.io_mut().add_mouse_pos_event(origin);
            context.io_mut().add_key_event(Key::ModShift, resize);
            context.io_mut().add_mouse_button_event(MouseButton::Left, true);
            let start = gizmo_frame(&mut context, &mut gizmo, &camera, &settings, target, true);
            assert_eq!(start.resizing, resize);
            context.io_mut().add_key_event(Key::ModShift, !resize);
            context
                .io_mut()
                .add_mouse_pos_event([origin[0] + 32.0, origin[1] - 32.0]);
            let dragged = gizmo_frame(&mut context, &mut gizmo, &camera, &settings, target, true);
            assert_eq!(dragged.resizing, resize);
            assert_eq!(dragged.selection.width(), if resize { 4 } else { 3 });
            assert_eq!(
                dragged.selection.min,
                if resize { selection.min } else { Coord::new(4, 4, 1) }
            );
            target.selection = dragged.selection;
            context.io_mut().add_mouse_button_event(MouseButton::Left, false);
            context.io_mut().add_mouse_pos_event([2000.0, -2000.0]);
            let released = gizmo_frame(&mut context, &mut gizmo, &camera, &settings, target, false);
            assert_eq!(released.resizing, resize);
            assert!(released.captures_mouse);
            assert!(!gizmo.is_interacting());
            assert_eq!(released.selection.max, Coord::new(10, 10, 1));
        }
    }

    #[test]
    fn rotation_shortcuts_work_with_both_presets_and_a_custom_modifier_binding() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        for (preset, modifier, key, custom) in [
            (KeybindPreset::Default, None, Key::R, false),
            (KeybindPreset::StrongDmm, Some(Key::ModShift), Key::R, false),
            (KeybindPreset::Default, Some(Key::ModCtrl), Key::T, true),
        ] {
            let mut context = rectangle_context();
            let mut settings = Settings {
                keybindings: preset.bindings(),
                ..Settings::default()
            };
            if custom {
                settings
                    .keybindings
                    .rebind(KeybindAction::Rotate, KeyBinding::with_ctrl(Key::T));
            }
            let mut camera = Controller::new();
            camera.resize(800, 600);
            let mut gizmo = GizmoState::default();
            let target = BlockGizmoTarget {
                selection: Selection::from_drag(Coord::new(2, 2, 1), Coord::new(4, 5, 1)),
                rotation: SelectionRotation::Original,
                map_size: Size { x: 10, y: 10, z: 1 },
                tile_size: 32,
                kind: BlockGizmoKind::Selection,
            };
            context.io_mut().add_mouse_pos_event([400.0, 300.0]);
            if let Some(modifier) = modifier {
                context.io_mut().add_key_event(modifier, true);
            }
            context.io_mut().add_key_event(key, true);
            let pressed = gizmo_frame(&mut context, &mut gizmo, &camera, &settings, target, true);
            assert!(!pressed.resizing);
            context.io_mut().add_key_event(key, false);
            let released = gizmo_frame(&mut context, &mut gizmo, &camera, &settings, target, true);
            assert_eq!(released.rotation, SelectionRotation::Clockwise);
            assert_eq!(released.selection, target.selection);
        }
    }

    #[test]
    fn cancelling_rectangle_gestures_restores_only_the_originating_document_and_level() {
        let mut session = Session::new();
        let id = session
            .state
            .open_document(MapDocument::new(dmm::Map::new(Size { x: 10, y: 10, z: 2 }), 1));
        let start = SelectionMask {
            bounds: Selection::from_drag(Coord::new(2, 2, 1), Coord::new(4, 4, 1)),
            mode: BlockSelectionMode::Hollow { line_width: 1 },
        };
        session.try_select_block(start);
        let mut gesture = Some(RectangleGesture {
            start: Some(start),
            z: 1,
        });
        session.try_select_block(SelectionMask {
            bounds: Selection::from_drag(start.bounds.min, Coord::new(8, 8, 1)),
            mode: BlockSelectionMode::Full,
        });
        let other = session
            .state
            .open_document(MapDocument::new(dmm::Map::new(Size { x: 3, y: 3, z: 1 }), 1));
        restore_rectangle_gesture(&mut session, id, &mut gesture);
        assert_eq!(session.state.document(id).unwrap().selection_mask(), Some(start));
        assert_eq!(session.state.document(other).unwrap().selection_mask(), None);
        assert!(gesture.is_none());
        session.state.set_active(id);
        gesture = Some(RectangleGesture {
            start: Some(start),
            z: 1,
        });
        session.set_level(2);
        restore_rectangle_gesture(&mut session, id, &mut gesture);
        assert_eq!(session.selection_mask(), None);
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
    fn a_short_path_is_left_alone_and_a_long_one_keeps_its_tail() {
        assert_eq!(shorten_path("code/game/area.dm", 60), "code/game/area.dm");
        assert_eq!(shorten_path("abcdef", 6), "abcdef");
        assert_eq!(shorten_path("abcdef", 4), "\u{2026}def");
        assert_eq!(shorten_path("", 8), "");
    }

    #[test]
    fn file_counts_are_grouped_in_threes() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(7), "7");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1482), "1,482");
        assert_eq!(grouped(1_234_567), "1,234,567");
    }

    #[test]
    fn a_load_notice_keeps_the_job_title_and_message() {
        let notice = LoadNotice::failed("Opening codebase", Path::new("game/tg.dme"), "no such file");

        assert_eq!(
            notice,
            LoadNotice::Failed {
                title: String::from("Opening codebase failed"),
                path: String::from("game/tg.dme"),
                message: String::from("no such file"),
            }
        );
    }

    #[test]
    fn diagnostics_are_joined_into_one_selectable_buffer() {
        let notice = LoadNotice::diagnostics(
            Path::new("game/tg.dme"),
            String::from("2 preprocessor diagnostics"),
            vec![String::from("first"), String::from("second")],
        );

        assert_eq!(
            notice,
            LoadNotice::Diagnostics {
                path: String::from("game/tg.dme"),
                summary: String::from("2 preprocessor diagnostics"),
                text: String::from("first\nsecond"),
            }
        );
    }

    #[test]
    fn the_default_dock_layout_is_valid() {
        let state = UiState::new(false).expect("valid window keys");

        assert_eq!(state.layout.validate(), Ok(()));
        assert_eq!(state.block_selection_options, BlockSelectionOptions::default());
        assert_eq!(state.fill_mode, FillMode::Wall);
        assert_eq!(
            state.custom_fill_boundaries,
            [TreePath::parse(DEFAULT_CUSTOM_FILL_BOUNDARY)]
        );
        assert!(state.custom_fill_search.is_empty());
        assert!(state.pending_fill_warning.is_none());
        assert!(state.new_map_dialog.is_none());
        assert!(state.load_notice.is_none());
        assert!(state.save_dialog.is_none());
        assert!(!state.exit_requested);
        assert!(state.show_welcome);
        assert!(state.open_error.is_none());
        assert!(!state.keybind_preset_prompt);
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
    fn a_map_view_rect_is_measured_in_framebuffer_pixels() {
        // ImGui lays out in logical units; the render target is physical pixels.
        let rect = framebuffer_rect([100.0, 50.0], [400.0, 300.0], [1.5, 1.5], [1000.0, 800.0]);

        assert_eq!(rect.x, 150);
        assert_eq!(rect.y, 75);
        assert_eq!(rect.width, 600);
        assert_eq!(rect.height, 450);
    }

    #[test]
    fn a_map_view_rect_is_the_same_at_unit_scale() {
        let rect = framebuffer_rect([0.0, 22.0], [640.0, 480.0], [1.0, 1.0], [1280.0, 720.0]);

        assert_eq!(
            rect,
            MapViewRect {
                x: 0,
                y: 22,
                width: 640,
                height: 480,
            }
        );
    }

    #[test]
    fn a_map_view_rect_is_clamped_to_the_target() {
        // Position stays inside the main framebuffer, while the independent map
        // attachment keeps its full size when the ImGui window is clipped.
        let rect = framebuffer_rect([-40.0, -10.0], [200.0, 100.0], [1.0, 1.0], [120.0, 60.0]);

        assert_eq!(rect.x, 0);
        assert_eq!(rect.y, 0);
        assert_eq!(rect.width, 200);
        assert_eq!(rect.height, 100);

        let collapsed = framebuffer_rect([10.0, 10.0], [0.0, 0.0], [1.0, 1.0], [100.0, 100.0]);
        assert!(collapsed.is_empty(), "a zero-size split contributes no draw");
    }

    #[test]
    fn a_cursor_is_local_to_its_map_view() {
        // Picking chooses a visibility attachment by hovered ImGui window, so
        // its cursor stays local regardless of where that window is placed.
        assert_eq!(cursor_in_map_view([10.0, 20.0], [1.0, 1.0]), [10, 20]);
    }

    #[test]
    fn a_cursor_scales_with_the_display() {
        assert_eq!(cursor_in_map_view([30.0, 40.0], [2.0, 2.0]), [60, 80]);
    }

    #[test]
    fn the_central_node_holds_only_the_welcome_window() {
        let state = UiState::new(false).expect("valid window keys");
        let DockLayout::Split { second, .. } = &state.layout else {
            panic!("the root is split between the object tree and everything else");
        };
        let DockLayout::Split { second, .. } = second.as_ref() else {
            panic!("the remainder is split between the inspector and the central node");
        };

        // Map views are minted per open map and dock themselves in at runtime,
        // so the declared layout cannot name them.
        assert_eq!(second.as_ref(), &DockLayout::tabs([&state.welcome_window]));
    }

    #[test]
    fn every_document_gets_its_own_map_view_window_key() {
        let first = DocumentId::new();
        let second = DocumentId::new();
        let first = MapViewState::new(first).expect("valid window key");
        let second = MapViewState::new(second).expect("valid window key");

        // Two windows sharing a key would be one window, and the dock layout
        // compiler rejects the duplicate outright.
        assert_ne!(first.window.stable_id(), second.window.stable_id());
        assert!(DockLayout::tabs([&first.window, &second.window]).validate().is_ok());
    }

    #[test]
    fn a_new_map_view_starts_out_wanting_to_frame_its_map() {
        let view = MapViewState::new(DocumentId::new()).expect("valid window key");

        assert!(view.refit);
        assert!(!view.focus);
        assert!(view.block_selection_anchor.is_none());
        assert!(view.block_placement.is_none());
    }

    #[test]
    fn a_map_reads_as_its_path_inside_the_codebase() {
        let base = Path::new("/tg");
        let map = PathBuf::from("/tg/_maps/map_files/station.dmm");

        assert_eq!(
            codebase_relative(base, &map),
            Path::new("_maps/map_files/station.dmm").display().to_string()
        );
    }

    #[test]
    fn the_map_filter_matches_any_part_of_the_codebase_relative_path() {
        let base = Path::new("/tg");
        let map = Path::new("/tg/_maps/map_files/MetaStation/MetaStation.dmm");

        assert!(map_matches(base, map, ""), "an empty filter keeps everything");
        assert!(map_matches(base, map, "metastation"), "matching ignores case");
        assert!(map_matches(base, map, "map_files"), "a directory segment matches");
        assert!(!map_matches(base, map, "deltastation"));
    }

    #[test]
    fn the_map_filter_does_not_match_the_codebase_directory_itself() {
        // The label is relative, so a codebase living under a directory called "maps" must not make
        // every one of its maps match the word.
        let base = Path::new("/home/maps/tg");
        let map = Path::new("/home/maps/tg/station.dmm");

        assert!(!map_matches(base, map, "home"));
    }

    #[test]
    fn a_map_outside_the_codebase_keeps_its_full_path() {
        let outside = PathBuf::from("/elsewhere/station.dmm");

        assert_eq!(
            codebase_relative(Path::new("/tg"), &outside),
            outside.display().to_string()
        );
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
    fn custom_fill_search_includes_parent_types_and_ignores_case() {
        let mut tree = ObjectTree::new();
        tree.register(&TreePath::parse("/obj/structure/table"), Location::default());
        tree.register(&TreePath::parse("/turf/closed/wall"), Location::default());

        assert_eq!(
            matching_type_paths(&tree, "STRUCT")
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["/obj/structure", "/obj/structure/table"]
        );
        assert_eq!(
            matching_type_paths(&tree, "wall")
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["/turf/closed/wall"]
        );
        assert!(matching_type_paths(&tree, "  ").is_empty());
    }

    #[test]
    fn panel_extent_rejects_non_finite_or_empty_sizes() {
        assert_eq!(panel_extent([0.0, f32::NAN]), ([1.0, 1.0], (1, 1)));
        assert_eq!(panel_extent([320.75, 200.25]), ([320.75, 200.25], (320, 200)));
    }

    #[test]
    fn recent_icons_fit_inside_a_square_without_changing_aspect_ratio() {
        assert_eq!(fit_recent_icon(32, 32), [RECENT_ICON_SIZE, RECENT_ICON_SIZE]);
        assert_eq!(fit_recent_icon(64, 32), [RECENT_ICON_SIZE, RECENT_ICON_SIZE / 2.0]);
        assert_eq!(fit_recent_icon(16, 32), [RECENT_ICON_SIZE / 2.0, RECENT_ICON_SIZE]);
        assert_eq!(fit_recent_icon(0, 0), [RECENT_ICON_SIZE, RECENT_ICON_SIZE]);
    }

    #[test]
    fn placement_preview_breathes_between_full_and_eighty_percent_opacity() {
        assert!((placement_preview_opacity(0.0) - 1.0).abs() < f32::EPSILON);
        assert!((placement_preview_opacity(0.75) - 0.8).abs() < f32::EPSILON);
        assert!((placement_preview_opacity(1.5) - 1.0).abs() < f32::EPSILON);
        assert!((placement_preview_opacity(3.75) - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn placement_flash_fades_once_over_a_quarter_second() {
        let owner = PrefabInstanceId::from_raw(7).unwrap();
        let flash = ActivePlacementFlash {
            owner,
            coord: Coord::new(2, 3, 1),
            started_at: 10.0,
        };

        assert_eq!(flash.sample(10.0), Some(PlacementFlash { owner, strength: 1.0 }));
        assert_eq!(flash.sample(10.125), Some(PlacementFlash { owner, strength: 0.5 }));
        assert_eq!(flash.sample(10.25), None);
        assert_eq!(flash.sample(11.0), None);
    }

    #[test]
    fn disabling_placement_flash_clears_an_active_effect() {
        let owner = PrefabInstanceId::from_raw(7).unwrap();
        let mut active = Some(ActivePlacementFlash {
            owner,
            coord: Coord::new(2, 3, 1),
            started_at: 10.0,
        });

        assert_eq!(active_placement_flash(&mut active, 10.0, false), None);
        assert_eq!(active, None);
    }

    #[test]
    fn placement_strokes_process_each_tile_once_until_a_new_stroke_begins() {
        let prefab = Prefab::new(TreePath::parse("/obj/table"));
        let first = Coord::new(2, 3, 1);
        let second = Coord::new(3, 3, 1);
        let mut stroke = PlacementStroke::new(prefab.clone(), 1);

        let group = stroke.visit(first).expect("first tile in stroke");
        assert_eq!(stroke.visit(first), None);
        assert_eq!(stroke.visit(second), Some(group));
        assert_eq!(stroke.visit(first), None);
        assert_eq!(stroke.visit(Coord::new(2, 3, 2)), None);

        let mut next_stroke = PlacementStroke::new(prefab, 1);
        assert!(next_stroke.visit(first).is_some());
    }

    #[test]
    fn placement_strokes_only_match_their_starting_context() {
        let prefab = Prefab::new(TreePath::parse("/obj/table"));
        let other = Prefab::new(TreePath::parse("/obj/chair"));
        let stroke = PlacementStroke::new(prefab.clone(), 2);

        assert!(stroke.matches_context(Tool::Place, Some(&prefab), 2));
        assert!(!stroke.matches_context(Tool::Select, Some(&prefab), 2));
        assert!(!stroke.matches_context(Tool::Place, Some(&other), 2));
        assert!(!stroke.matches_context(Tool::Place, Some(&prefab), 1));
        assert!(!stroke.matches_context(Tool::Place, None, 2));
    }

    #[test]
    fn placement_preview_bounds_preserve_anchor_offsets_dimensions_and_zoom() {
        let mut camera = Controller::new();
        camera.resize(100, 80);
        camera.camera.x = 50.0;
        camera.camera.y = 40.0;
        camera.camera.zoom = 2.0;
        let preview = PlacementPreview {
            thumbnail: crate::session::PrefabThumbnail {
                texture: SpriteTexture {
                    width: 64,
                    height: 32,
                    ..Default::default()
                },
                uv0: [0.0; 2],
                uv1: [1.0; 2],
                tint: [1.0; 4],
            },
            offset: [-16, 4],
        };

        let bounds = placement_preview_bounds(&camera, [10.0, 20.0], Coord::new(2, 2, 1), 32, preview);

        assert_eq!(bounds.min, [-8.0, 4.0]);
        assert_eq!(bounds.max, [120.0, 68.0]);
    }

    #[test]
    fn a_pasted_block_lands_centered_on_the_cursor() {
        let size = Size { x: 20, y: 20, z: 1 };

        // An odd extent sits dead centre; an even one leans one tile left/down.
        assert_eq!(
            centered_paste_min(3, 3, Coord::new(10, 10, 1), size),
            Coord::new(9, 9, 1)
        );
        assert_eq!(
            centered_paste_min(4, 4, Coord::new(10, 10, 1), size),
            Coord::new(9, 9, 1)
        );
        assert_eq!(
            centered_paste_min(1, 1, Coord::new(10, 10, 1), size),
            Coord::new(10, 10, 1)
        );
    }

    #[test]
    fn a_pasted_block_is_nudged_back_inside_the_map() {
        let size = Size { x: 10, y: 10, z: 1 };

        // Centring near an edge would hang the block off the map, so it slides
        // back until the whole footprint fits.
        assert_eq!(centered_paste_min(5, 5, Coord::new(1, 1, 1), size), Coord::new(1, 1, 1));
        assert_eq!(
            centered_paste_min(5, 5, Coord::new(10, 10, 1), size),
            Coord::new(6, 6, 1)
        );
        // A block larger than the map still starts at the origin rather than
        // clamping to nothing.
        assert_eq!(
            centered_paste_min(40, 40, Coord::new(5, 5, 1), size),
            Coord::new(1, 1, 1)
        );
    }

    #[test]
    fn editing_commands_are_bound_to_the_usual_chords() {
        let bindings = KeyBindings::default();

        assert_eq!(
            bindings.get(KeybindAction::Save),
            KeyBinding::with_ctrl(dear_imgui_rs::Key::S)
        );
        assert_eq!(
            bindings.get(KeybindAction::Undo),
            KeyBinding::with_ctrl(dear_imgui_rs::Key::Z)
        );
        assert_eq!(
            bindings.get(KeybindAction::Redo),
            KeyBinding::with_ctrl(dear_imgui_rs::Key::Y)
        );
        assert_eq!(
            bindings.get(KeybindAction::Copy),
            KeyBinding::with_ctrl(dear_imgui_rs::Key::C)
        );
        assert_eq!(
            bindings.get(KeybindAction::Paste),
            KeyBinding::with_ctrl(dear_imgui_rs::Key::V)
        );
        // Every action is reachable from the settings list, or it cannot be rebound.
        assert_eq!(KeybindAction::ALL.len(), 26);
        assert_eq!(KeybindAction::RECENT.len(), 10);
        assert!(KeybindAction::ALL.contains(&KeybindAction::Save));
        assert!(KeybindAction::ALL.contains(&KeybindAction::Undo));
        assert!(KeybindAction::ALL.contains(&KeybindAction::Redo));
        assert!(KeybindAction::ALL.contains(&KeybindAction::Copy));
        assert!(KeybindAction::ALL.contains(&KeybindAction::Paste));
        assert!(KeybindAction::ALL.contains(&KeybindAction::Recent1));
        assert!(KeybindAction::ALL.contains(&KeybindAction::Recent0));
    }

    #[test]
    fn block_selection_bounds_follow_whole_tile_edges() {
        let mut camera = Controller::new();
        camera.resize(64, 64);
        camera.camera.x = 32.0;
        camera.camera.y = 32.0;
        let selection = Selection::from_drag(Coord::new(1, 1, 1), Coord::new(2, 1, 1));

        assert_eq!(
            block_selection_bounds(&camera, [10.0, 20.0], selection, 32),
            OverlayRect {
                min: [10.0, 52.0],
                max: [74.0, 84.0],
            }
        );
    }

    #[test]
    fn block_selection_options_clamp_the_line_width_and_label_the_active_mode() {
        let mut options = BlockSelectionOptions::default();
        assert_eq!(options.mode(), BlockSelectionMode::Full);
        assert_eq!(options.label(), "Full rectangle");
        assert_eq!(options.drawing_mode(true), BlockSelectionMode::Hollow { line_width: 1 });
        assert_eq!(options.drawing_mode(false), options.mode());

        // the width stays configured while the full rectangle is selected, and is ignored
        options.line_width = 4;
        assert_eq!(options.mode(), BlockSelectionMode::Full);
        assert_eq!(options.drawing_mode(true), BlockSelectionMode::Hollow { line_width: 4 });

        options.full_rectangle = false;
        assert_eq!(options.mode(), BlockSelectionMode::Hollow { line_width: 4 });
        assert_eq!(options.label(), "Border, 4 tiles");

        options.line_width = 1;
        assert_eq!(options.mode(), BlockSelectionMode::Hollow { line_width: 1 });
        assert_eq!(options.label(), "Border, 1 tile");

        // the drag widget clamps to one, but a stale or hand-edited value must not wrap round the cast
        for line_width in [0, -1, i32::MIN] {
            options.line_width = line_width;
            assert_eq!(options.mode(), BlockSelectionMode::Hollow { line_width: 1 });
            assert_eq!(options.drawing_mode(true), BlockSelectionMode::Hollow { line_width: 1 });
        }
    }

    #[test]
    fn hollow_block_selection_exposes_the_inset_hole() {
        let selection = Selection::from_drag(Coord::new(2, 3, 1), Coord::new(6, 7, 1));

        assert_eq!(hollow_selection_inner(selection, BlockSelectionMode::Full), None);
        assert_eq!(
            hollow_selection_inner(selection, BlockSelectionMode::Hollow { line_width: 1 }),
            Some(Selection::from_drag(Coord::new(3, 4, 1), Coord::new(5, 6, 1)))
        );
        assert_eq!(
            hollow_selection_inner(selection, BlockSelectionMode::Hollow { line_width: 2 }),
            Some(Selection::from_drag(Coord::new(4, 5, 1), Coord::new(4, 5, 1)))
        );
        assert_eq!(
            hollow_selection_inner(selection, BlockSelectionMode::Hollow { line_width: 3 }),
            None
        );
    }

    #[test]
    fn block_controls_are_available_before_a_selection_is_moved() {
        let selection = Selection::from_drag(Coord::new(1, 1, 1), Coord::new(2, 2, 1));
        let expected = PendingBlockPlacement {
            source: selection,
            target: selection,
            rotation: SelectionRotation::Original,
        };

        assert_eq!(
            block_controls_placement(Tool::BlockSelect, false, false, Some(selection), None),
            Some(expected)
        );
        assert_eq!(
            block_controls_placement(Tool::BlockSelect, true, false, Some(selection), None),
            None
        );
        assert_eq!(
            block_controls_placement(Tool::BlockSelect, false, true, Some(selection), None),
            None
        );
    }

    #[test]
    fn block_border_stripes_stay_on_each_rectangle_edge() {
        let bounds = OverlayRect {
            min: [10.0, 20.0],
            max: [42.0, 68.0],
        };
        let segments = block_border_segments(bounds, bounds, 3.0);

        assert!(!segments.is_empty());
        assert!(segments.iter().any(|(_, _, green)| *green));
        assert!(segments.iter().any(|(_, _, green)| !*green));
        assert!(segments.iter().all(|(start, end, _)| {
            (start[0] == end[0] || start[1] == end[1])
                && [start, end].into_iter().flatten().all(|value| value.is_finite())
        }));
    }

    #[test]
    fn block_border_generation_is_limited_to_visible_edges() {
        let bounds = OverlayRect {
            min: [-1_000_000.0, 20.0],
            max: [1_000_000.0, 68.0],
        };
        let clip = OverlayRect {
            min: [0.0, 0.0],
            max: [100.0, 100.0],
        };
        let segments = block_border_segments(bounds, clip, 0.0);

        assert!(segments.len() < 40);
        assert!(segments.iter().all(|(start, end, _)| {
            [start, end]
                .into_iter()
                .flatten()
                .all(|value| (-f32::EPSILON..=100.0 + f32::EPSILON).contains(value))
        }));
    }

    #[test]
    fn tool_interaction_configures_highlights_and_pick_requests() {
        let owner = PrefabInstanceId::from_raw(7).unwrap();
        let interaction = MapViewInteraction {
            cursor: Some([10, 20]),
            hovered_area: Some(owner),
            selected: Some(owner),
            selection_guide: None,
            placement_flash: Some(PlacementFlash { owner, strength: 0.5 }),
            highlight: HighlightStyle::Tint,
            mode: InteractionMode::Select {
                pick: Some(PickRequest::Cursor),
            },
        };
        let mut selected = interaction;

        configure_tool_interaction(Tool::Select, &mut selected);
        assert_eq!(selected, interaction);

        let mut delete = interaction;
        configure_tool_interaction(Tool::Delete, &mut delete);
        assert_eq!(
            delete,
            MapViewInteraction {
                selected: None,
                selection_guide: None,
                mode: InteractionMode::Delete { pick: None },
                ..interaction
            }
        );

        configure_tool_interaction(Tool::Place, &mut selected);
        assert_eq!(
            selected,
            MapViewInteraction {
                placement_flash: interaction.placement_flash,
                highlight: interaction.highlight,
                ..Default::default()
            }
        );

        let mut fill = interaction;
        configure_tool_interaction(Tool::Fill, &mut fill);
        assert_eq!(
            fill,
            MapViewInteraction {
                placement_flash: interaction.placement_flash,
                highlight: interaction.highlight,
                ..Default::default()
            }
        );

        let mut block = interaction;
        configure_tool_interaction(Tool::BlockSelect, &mut block);
        assert_eq!(
            block,
            MapViewInteraction {
                placement_flash: interaction.placement_flash,
                highlight: interaction.highlight,
                ..Default::default()
            }
        );
    }

    #[test]
    fn deletion_strokes_only_continue_when_the_cursor_moves() {
        let mut stroke = DeletionStroke::new([10, 20]);

        assert!(!stroke.move_to([10, 20]));
        assert!(stroke.move_to([11, 20]));
        assert!(!stroke.move_to([11, 20]));
        assert!(stroke.move_to([10, 20]));
    }

    #[test]
    fn prefab_tooltips_include_the_path_and_exact_overrides() {
        let mut prefab = Prefab::new(TreePath::parse("/obj/table"));
        prefab.set_var("name".into(), core::types::Value::Text("custom".into()));

        assert_eq!(prefab_tooltip(&prefab), "/obj/table\nname = \"custom\"");
    }

    #[test]
    fn overlay_bounds_exclude_their_far_edges() {
        let bounds = OverlayRect {
            min: [10.0, 20.0],
            max: [30.0, 40.0],
        };

        assert!(bounds.contains([10.0, 20.0]));
        assert!(bounds.contains([29.0, 39.0]));
        assert!(!bounds.contains([30.0, 39.0]));
        assert!(!bounds.contains([29.0, 40.0]));
    }
}
