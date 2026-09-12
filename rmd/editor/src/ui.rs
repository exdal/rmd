use core::{path::TreePath, types::Identifier};
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
    Id,
    InputTextMultilineFlags,
    Key,
    ListClipper,
    MouseButton,
    StyleColor,
    StyleVar,
    TableFlags,
    TableSizingPolicy,
    Ui,
    WindowFlags,
    WindowKey,
    WindowKeyError,
};
use dmm::{Coord, MapFormat, Prefab, Size};
use editor::{
    command::EditGroupId,
    document::{DocumentId, MapDocument, PrefabInstanceId, PrefabLocation, Selection},
    icons::materialdesignicons::{
        ICON_CLOSE_THICK,
        ICON_COG,
        ICON_DOTS_HORIZONTAL,
        ICON_ERASER,
        ICON_EYE,
        ICON_EYE_OFF,
        ICON_EYEDROPPER,
        ICON_FILTER,
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
        SelectionPlacement,
        SelectionRotation,
        SelectionTransform,
        Tool,
        rotated_selection_at,
    },
};
use objtree::{ObjectTree, TypeId};
use render::{Camera, InteractionMode, MapViewInteraction, MapViewRect, PickRequest, PlacementFlash, Renderer};

use crate::{
    camera::Controller,
    external_editor::SourceLocation,
    gizmo::{BlockGizmoTarget, GizmoMapView, GizmoState},
    inspector::{InspectorOutput, InspectorState},
    loader::LoadView,
    session::{BlockPreviewSprite, FillOutcome, PlacementPreview, Session},
    settings::{
        BINDABLE_KEYS,
        KeyBinding,
        KeyBindings,
        KeybindAction,
        ObjectTreeFilterOptions,
        ObjectTreeSearchOptions,
        SelectionHighlight,
        Settings,
    },
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
const BLOCK_PLACEMENT_LABELS: [&str; 4] = ["Move", "Copy", "Fill", "Cancel"];
const PASTE_LABELS: [&str; 2] = ["Paste", "Cancel"];
const BLOCK_SELECTION_LINE_WIDTH_DRAG_WIDTH: f32 = 140.0;
const FILL_LIMIT_WARNING_POPUP: &str = "Large fill##fill-limit-warning";
const NEW_MAP_POPUP: &str = "New map##new-map";
const NEW_MAP_PATH_WIDTH: f32 = 460.0;
const NEW_MAP_DEFAULT_WIDTH: i32 = 255;
const NEW_MAP_DEFAULT_HEIGHT: i32 = 255;
const NEW_MAP_DEFAULT_LEVELS: i32 = 1;
const NEW_MAP_MAX_DIMENSION: i32 = 255;
const SAVE_MAP_POPUP: &str = "Save map##save-map";
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
const BLOCK_SELECTION_POPUP: &str = "Block selection##block-selection";
const BLOCK_SELECTION_GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const BLOCK_SELECTION_WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const BLOCK_SELECTION_SHADOW: [f32; 4] = [0.0, 0.0, 0.0, 0.85];
const BLOCK_GHOST_OPACITY: f32 = 0.55;
const BLOCK_STRIPE_LENGTH: f32 = 6.0;
const BLOCK_STRIPE_SPEED: f32 = 12.0;
const OBJECT_TREE_FILTER_OPTIONS_POPUP: &str = "object-tree-filter-options-popup";
const OBJECT_TREE_SEARCH_OPTIONS_POPUP: &str = "object-tree-search-options-popup";
const SIMILAR_INSTANCES_WINDOW_SIZE: [f32; 2] = [420.0, 320.0];

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
    fn mode(self) -> BlockSelectionMode {
        if self.full_rectangle {
            BlockSelectionMode::Full
        } else {
            BlockSelectionMode::Hollow {
                line_width: self.line_width.max(1) as u32,
            }
        }
    }

    fn label(self) -> String {
        match self.mode() {
            BlockSelectionMode::Full => String::from("Full rectangle"),
            BlockSelectionMode::Hollow { line_width: 1 } => String::from("Hollow, 1-tile line"),
            BlockSelectionMode::Hollow { line_width } => format!("Hollow, {line_width}-tile line"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingFillWarning {
    coord: Coord,
    fill_mode: FillMode,
    custom_fill_boundaries: Vec<TreePath>,
    limit: usize,
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
enum BlockPlacementAction {
    Move,
    Copy,
    Fill,
    Cancel,
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

#[derive(Debug, Default)]
struct ObjectTreeFilter {
    roots: Vec<TypeId>,
    children: HashMap<TypeId, Vec<TypeId>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ObjectTreeTypeFilter {
    atom: Option<TypeId>,
    movable: Option<TypeId>,
    obj: Option<TypeId>,
    turf: Option<TypeId>,
    custom: Option<TypeId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObjectTreeFilterAction {
    Keep,
    Prune,
}

impl ObjectTreeTypeFilter {
    fn new(tree: &ObjectTree, options: &ObjectTreeFilterOptions) -> Self {
        let roots = tree.roots();
        let custom = options
            .custom_enabled
            .then(|| options.custom_type_path.trim())
            .filter(|path| !path.is_empty())
            .and_then(|path| tree.id_of(&TreePath::parse(path)));

        Self {
            atom: options.atom.then_some(roots.atom).flatten(),
            movable: options.movable.then_some(roots.movable).flatten(),
            obj: options.obj.then_some(roots.obj).flatten(),
            turf: options.turf.then_some(roots.turf).flatten(),
            custom,
        }
    }

    fn action(self, tree: &ObjectTree, id: TypeId) -> ObjectTreeFilterAction {
        let filtered_by_builtin = [self.atom, self.movable, self.obj, self.turf]
            .into_iter()
            .flatten()
            .any(|filtered| tree.is_subtype_of(id, filtered));
        if filtered_by_builtin || self.custom.is_some_and(|custom| tree.is_subtype_of(id, custom)) {
            ObjectTreeFilterAction::Prune
        } else {
            ObjectTreeFilterAction::Keep
        }
    }
}

impl ObjectTreeSearchOptions {
    const fn hint(self) -> &'static str {
        match (self.type_paths, self.names) {
            (true, true) => "Search type paths or names",
            (true, false) => "Search type paths",
            (false, true) => "Search names",
            (false, false) => "Search types",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ObjectTreeRow {
    id: TypeId,
    parent: Option<usize>,
    leaf: bool,
}

#[derive(Clone, Copy, Default)]
struct ObjectTreeRowOptions<'a> {
    filter: Option<&'a ObjectTreeFilter>,
    type_filter: ObjectTreeTypeFilter,
    expand: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct ObjectTreeOutput {
    chosen: Option<TypeId>,
    visibility_toggle: Option<TypeId>,
    open_source: Option<SourceLocation>,
}

impl ObjectTreeFilter {
    fn new(
        tree: &ObjectTree, atom: TypeId, query: &str, options: ObjectTreeSearchOptions,
        type_filter: ObjectTreeTypeFilter,
    ) -> Self {
        let matches = matching_object_types(tree, query, options)
            .into_iter()
            .filter(|id| tree.is_subtype_of(*id, atom))
            .filter(|id| type_filter.action(tree, *id) == ObjectTreeFilterAction::Keep)
            .collect::<Vec<_>>();
        let match_set = matches.iter().copied().collect::<HashSet<_>>();
        let mut filter = Self::default();

        for id in matches {
            let matching_parent = tree
                .ancestors(id)
                .skip(1)
                .find(|ancestor| match_set.contains(&ancestor.id))
                .map(|ancestor| ancestor.id);
            if let Some(parent) = matching_parent {
                filter.children.entry(parent).or_default().push(id);
            } else {
                filter.roots.push(id);
            }
        }

        filter
    }

    fn is_empty(&self) -> bool { self.roots.is_empty() }

    fn children(&self, id: TypeId) -> &[TypeId] { self.children.get(&id).map_or(&[], Vec::as_slice) }
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
            block_placement: None,
            paste: None,
        })
    }
}

#[derive(Debug)]
struct SimilarInstancesState {
    document: DocumentId,
    prefab_path: String,
    instances: Vec<PrefabInstanceId>,
    focus: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct JumpTarget {
    document: DocumentId,
    instance: PrefabInstanceId,
}

pub struct UiState {
    object_tree: WindowKey,
    map_views: HashMap<DocumentId, MapViewState>,
    central_node: Option<Id>,
    dockspace_root: Option<Id>,
    welcome_window: WindowKey,
    inspector_window: WindowKey,
    similar_instances_window: WindowKey,
    settings_window: WindowKey,
    layout: DockLayout,
    selected: Option<TypeId>,
    object_tree_search: String,
    object_tree_filter: Option<ObjectTreeFilter>,
    object_tree_filter_revision: u64,
    inspector: InspectorState,
    similar_instances: Option<SimilarInstancesState>,
    gizmo: GizmoState,
    placement_flash: Option<ActivePlacementFlash>,
    placement_stroke: Option<PlacementStroke>,
    deletion_stroke: Option<DeletionStroke>,
    block_selection_options: BlockSelectionOptions,
    fill_mode: FillMode,
    custom_fill_boundaries: Vec<TreePath>,
    custom_fill_search: String,
    pending_fill_warning: Option<PendingFillWarning>,
    new_map_dialog: Option<NewMapDialog>,
    save_dialog: Option<SaveDialog>,
    pending_close: Option<DocumentId>,
    show_settings: bool,
    show_welcome: bool,
    welcome_map_filter: String,
    welcome_maps_expanded: bool,
    open_error: Option<String>,
    load_window: WindowKey,
    load_notice: Option<LoadNotice>,
    load_window_size: [f32; 2],
    load_popup_active: bool,
    settings_window_size: [f32; 2],
    capturing_keybind: Option<KeybindAction>,
}

impl UiState {
    pub fn new() -> Result<Self, WindowKeyError> {
        let object_tree = WindowKey::new("object-tree", "Object tree")?;
        let welcome_window = WindowKey::new("welcome", "Welcome")?;
        let inspector_window = WindowKey::new("inspector", "Inspector")?;
        let similar_instances_window = WindowKey::new("similar-instances", "Similar prefab instances")?;
        let settings_window = WindowKey::new("settings", "Settings")?;
        let load_window = WindowKey::new("load", "Loading")?;
        let layout = DockLayout::split(
            DockSplit::Left,
            0.25,
            DockLayout::tabs([&object_tree]),
            DockLayout::split(
                DockSplit::Right,
                0.20 / 0.75,
                DockLayout::tabs([&inspector_window]),
                DockLayout::tabs([&welcome_window]),
            ),
        );

        Ok(Self {
            object_tree,
            map_views: HashMap::new(),
            central_node: None,
            dockspace_root: None,
            welcome_window,
            inspector_window,
            similar_instances_window,
            settings_window,
            layout,
            selected: None,
            object_tree_search: String::new(),
            object_tree_filter: None,
            object_tree_filter_revision: 0,
            inspector: InspectorState::default(),
            similar_instances: None,
            gizmo: GizmoState::default(),
            placement_flash: None,
            placement_stroke: None,
            deletion_stroke: None,
            block_selection_options: BlockSelectionOptions::default(),
            fill_mode: FillMode::default(),
            custom_fill_boundaries: vec![TreePath::parse(DEFAULT_CUSTOM_FILL_BOUNDARY)],
            custom_fill_search: String::new(),
            pending_fill_warning: None,
            new_map_dialog: None,
            save_dialog: None,
            pending_close: None,
            show_settings: false,
            show_welcome: true,
            welcome_map_filter: String::new(),
            welcome_maps_expanded: false,
            open_error: None,
            load_window,
            load_notice: None,
            load_window_size: [0.0, 0.0],
            load_popup_active: false,
            settings_window_size: [420.0, 480.0],
            capturing_keybind: None,
        })
    }

    pub fn set_open_error(&mut self, error: Option<String>) { self.open_error = error; }

    pub fn set_load_notice(&mut self, notice: Option<LoadNotice>) { self.load_notice = notice; }

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

    fn cancel_edit_gestures(&mut self, id: Option<DocumentId>) {
        self.gizmo.cancel();
        self.placement_flash = None;
        self.placement_stroke = None;
        self.deletion_stroke = None;

        if let Some(view) = id.and_then(|id| self.map_views.get_mut(&id)) {
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
        self.load_popup_active = load.is_some() || self.load_notice.is_some();
        let loading = self.load_popup_active;
        let root = ui.get_id(DOCKSPACE_ID);
        ui.dockspace()
            .main_viewport()
            .root_id(root)
            .flags(DockNodeFlags::PASSTHRU_CENTRAL_NODE)
            .layout(&self.layout, DockLayoutApply::IfMissing)
            .build()?;
        self.dockspace_root = Some(root);

        let mut exit = false;
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
                if ui.menu_item_enabled_selected_no_shortcut("Save...", false, session.map().is_some()) {
                    open_save_dialog = true;
                }
                ui.separator();
                if ui.menu_item("Welcome") {
                    show_welcome = true;
                }
                if ui.menu_item("Settings...") {
                    self.show_settings = true;
                }
                ui.separator();
                if ui.menu_item_with_shortcut("Exit", "Esc") {
                    exit = true;
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

        if undo || redo {
            self.cancel_edit_gestures(session.state.active());
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
            session.change_level(level_delta);
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

        draw_settings_window(
            ui,
            &self.settings_window,
            &mut self.show_settings,
            &mut self.capturing_keybind,
            &mut self.settings_window_size,
            session,
            settings,
        );
        self.show_welcome |= show_welcome;

        let mut open_source = self.draw_object_tree(ui, session, settings);
        let inspector = self.draw_inspector(ui, session);
        open_source = inspector.open_source.or(open_source);
        if inspector.find_similar {
            self.open_similar_instances(session);
        }
        if let Some(target) = self.draw_similar_instances(ui, session) {
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
            exit |= ui.is_key_pressed(Key::Escape)
                && self.save_dialog.is_none()
                && self.capturing_keybind.is_none()
                && !load_popup.handles_escape;

            (Vec::new(), None)
        } else {
            let (map_view_exit, map_views, picking) = self.draw_map_views(ui, session, settings, refit);
            exit |= map_view_exit;

            (map_views, picking)
        };
        finish_keybind_capture(ui, &mut self.capturing_keybind, &mut settings.keybindings);

        Ok(UiOutput {
            exit,
            map_views,
            picking,
            open,
            open_source,
            pick_new_map_path,
            cancel_load: load_popup.cancel,
            copy_to_clipboard: load_popup.copy,
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

    fn draw_object_tree(&mut self, ui: &Ui, session: &mut Session, settings: &mut Settings) -> Option<SourceLocation> {
        let mut open_source = None;
        ui.window(&self.object_tree).build(|| {
            let mut output = ObjectTreeOutput::default();
            let available_width = ui.content_region_avail()[0];
            let button_size = ui.frame_height();
            let spacing = ui.clone_style().item_spacing()[0];
            ui.set_next_item_width((available_width - button_size * 2.0 - spacing * 2.0).max(1.0));
            let search_changed = ui
                .input_text("##object-tree-search", &mut self.object_tree_search)
                .hint(settings.object_tree_search.hint())
                .build();
            ui.same_line();
            let filter_clicked = ui.button_with_size("##object-tree-filter-options", [button_size, button_size]);
            draw_centered_icon(ui, ICON_FILTER);
            if filter_clicked {
                ui.open_popup(OBJECT_TREE_FILTER_OPTIONS_POPUP);
            }
            ui.set_item_tooltip("Type filters");

            ui.same_line();
            let options_clicked = ui.button_with_size("##object-tree-search-options", [button_size, button_size]);
            draw_centered_icon(ui, ICON_COG);
            if options_clicked {
                ui.open_popup(OBJECT_TREE_SEARCH_OPTIONS_POPUP);
            }
            ui.set_item_tooltip("Search options");

            let mut type_filter_changed = false;
            if let Some(_popup) = ui.begin_popup(OBJECT_TREE_FILTER_OPTIONS_POPUP) {
                type_filter_changed |= ui.checkbox("Filter /atom", &mut settings.object_tree_filter.atom);
                type_filter_changed |= ui.checkbox("Filter /movable", &mut settings.object_tree_filter.movable);
                type_filter_changed |= ui.checkbox("Filter /obj", &mut settings.object_tree_filter.obj);
                type_filter_changed |= ui.checkbox("Filter /turf", &mut settings.object_tree_filter.turf);

                ui.separator();
                type_filter_changed |= ui.checkbox(
                    "Filter custom type path and subtypes",
                    &mut settings.object_tree_filter.custom_enabled,
                );
                let _disabled = ui.begin_disabled_with_cond(!settings.object_tree_filter.custom_enabled);
                ui.set_next_item_width(280.0);
                type_filter_changed |= ui
                    .input_text(
                        "##object-tree-custom-type-filter",
                        &mut settings.object_tree_filter.custom_type_path,
                    )
                    .hint("/path/to/type")
                    .build();
            }

            let mut search_options_changed = false;
            if let Some(_popup) = ui.begin_popup(OBJECT_TREE_SEARCH_OPTIONS_POPUP) {
                let disable_type_paths = settings.object_tree_search.type_paths && !settings.object_tree_search.names;
                {
                    let _disabled = ui.begin_disabled_with_cond(disable_type_paths);
                    search_options_changed |= ui.checkbox("Type paths", &mut settings.object_tree_search.type_paths);
                }

                let disable_names = settings.object_tree_search.names && !settings.object_tree_search.type_paths;
                {
                    let _disabled = ui.begin_disabled_with_cond(disable_names);
                    search_options_changed |= ui.checkbox("Names (atom/name)", &mut settings.object_tree_search.names);
                }
            }

            let tree_revision = session.texture_revision();
            let Some(tree) = session.tree() else {
                ui.text_disabled("No environment loaded");

                return;
            };
            let Some(atom) = tree.roots().atom else {
                ui.text_disabled("No /atom type available");

                return;
            };
            let type_filter = ObjectTreeTypeFilter::new(tree, &settings.object_tree_filter);
            let rebuild_filter = search_changed
                || search_options_changed
                || type_filter_changed
                || self.object_tree_filter_revision != tree_revision;
            if rebuild_filter {
                self.object_tree_filter = (!self.object_tree_search.trim().is_empty()).then(|| {
                    ObjectTreeFilter::new(
                        tree,
                        atom,
                        &self.object_tree_search,
                        settings.object_tree_search,
                        type_filter,
                    )
                });
                self.object_tree_filter_revision = tree_revision;
            }

            if self.object_tree_filter.as_ref().is_some_and(ObjectTreeFilter::is_empty) {
                ui.text_disabled("No matching types");

                return;
            }
            let roots = self.object_tree_filter.as_ref().map_or_else(
                || visible_type_roots(tree, atom, type_filter),
                |filter| filter.roots.clone(),
            );
            if roots.is_empty() {
                ui.text_disabled("No types pass filters");

                return;
            }

            let mut alternate_row = ui.style_color(StyleColor::TableRowBgAlt);
            // need to handle this in themes, not here but lazy
            alternate_row[3] *= 0.4;
            let _alternate_row = ui.push_style_color(StyleColor::TableRowBgAlt, alternate_row);
            ui.table("object-tree-types")
                .flags(TableFlags::BORDERS_INNER_V | TableFlags::ROW_BG)
                .sizing_policy(TableSizingPolicy::StretchProp)
                .column("Type")
                .weight(1.0)
                .done()
                .column("Visibility")
                .width(ui.frame_height() * 2.0)
                .done()
                .build(|ui| {
                    let mut rows = Vec::new();
                    let options = ObjectTreeRowOptions {
                        filter: self.object_tree_filter.as_ref(),
                        type_filter,
                        expand: rebuild_filter && self.object_tree_filter.is_some(),
                    };
                    for root in roots.iter().copied() {
                        collect_type_rows(ui, tree, root, None, &mut rows, &options);
                    }
                    for index in ListClipper::new(rows.len()).begin(ui).iter() {
                        draw_type_row(ui, session, tree, &rows, index, &mut self.selected, &mut output);
                    }
                });

            if let Some(chosen) = output.chosen {
                session.choose_type(chosen);
            }
            if let Some(id) = output.visibility_toggle {
                session.toggle_type_visibility(id);
            }
            open_source = output.open_source;
        });

        open_source
    }

    fn draw_map_views(
        &mut self, ui: &Ui, session: &mut Session, settings: &Settings, refit_active: bool,
    ) -> (bool, Vec<VisibleMapView>, Option<usize>) {
        let mut exit = false;
        let mut closing = None;
        let mut visible = Vec::new();

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
            exit |= self.draw_map_view(
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

        (exit, visible, picking)
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

    fn draw_map_view(&mut self, ui: &Ui, session: &mut Session, settings: &Settings, draw: MapViewDraw<'_>) -> bool {
        let MapViewDraw {
            id,
            index: map_view_index,
            view,
            interaction,
            refit_requested,
            keep_open,
        } = draw;
        let mut exit = false;
        let Some(title) = session.state.document(id).map(MapDocument::title) else {
            return exit;
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
            if ui.is_window_focused() {
                session.state.set_active(id);
            }
            let is_active = session.state.active() == Some(id);
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
                && !has_modifiers(ui)
                && (!hovered || !settings.keybindings.is_any_pressed(ui))
                && let Some(index) = pressed_recent(ui)
            {
                session.choose_recent(index);
            }

            if hovered {
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
                    session.change_level(1);
                }
                if settings.keybindings.get(KeybindAction::LevelDown).is_pressed(ui) {
                    session.change_level(-1);
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
                    let undo = settings.keybindings.get(KeybindAction::Undo).is_pressed(ui);
                    let redo = settings.keybindings.get(KeybindAction::Redo).is_pressed(ui);
                    if undo || redo {
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
                        session.copy_selection(self.block_selection_options.mode());
                    }
                    if settings.keybindings.get(KeybindAction::Paste).is_pressed(ui)
                        && let Some(block) = session.clipboard()
                    {
                        let (width, height) = (block.width(), block.height());
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

                let paste_target = paste.and_then(|pending| session.clipboard_footprint(pending.min, pending.rotation));
                if paste.is_some() && paste_target.is_none() {
                    *paste = None;
                }

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
                                },
                                gizmo_map_view,
                            );

                            if response.selection != displayed || response.rotation != rotation {
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
                let block_controls_area = OverlayRect {
                    min: [viewport_min[0], top_overlay.max[1]],
                    max: [viewport_max[0], bottom_overlay.min[1]],
                };

                let controls_hit_test = match *paste {
                    Some(_) => paste_controls(self.gizmo.block_rotation_open(), paste_target)
                        .map(|target| (PASTE_LABELS.as_slice(), target)),
                    None => block_controls_placement(
                        tool,
                        block_selection_anchor.is_some(),
                        self.gizmo.block_rotation_open(),
                        session.selection(),
                        *block_placement,
                    )
                    .map(|placement| (BLOCK_PLACEMENT_LABELS.as_slice(), placement.target)),
                };
                let block_controls_capture_mouse = controls_hit_test.is_some_and(|(labels, target)| {
                    let (_, bounds) = block_placement_controls_layout(
                        ui,
                        camera,
                        labels,
                        target,
                        session.options.tile_size,
                        viewport_min,
                        block_controls_area,
                    );

                    bounds.contains(mouse)
                });

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
                if !gizmo_captures_mouse
                    && !block_controls_capture_mouse
                    && let Some(cursor) = cursor
                {
                    let pixel = [cursor[0].floor() as u32, cursor[1].floor() as u32];
                    interaction.cursor = Some(cursor_in_map_view(cursor, scale));

                    if let Some(coord) = pointed_coord {
                        interaction.hovered_area = session.area_at(coord);
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
                                        if session.select_block_with_mode(
                                            Some(selection),
                                            self.block_selection_options.mode(),
                                        ) {
                                            *block_selection_anchor = Some(coord);
                                        }
                                    }

                                    if left_down && let Some(anchor) = block_selection_anchor {
                                        session.select_block_with_mode(
                                            Some(Selection::from_drag(*anchor, coord)),
                                            self.block_selection_options.mode(),
                                        );
                                    }

                                    if ui.is_mouse_clicked(MouseButton::Right)
                                        && block_selection_anchor.is_none()
                                        && session.selection().is_some_and(|selection| {
                                            self.block_selection_options.mode().includes(selection, coord)
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
                                if left_clicked
                                    && let FillOutcome::TooLarge { limit } =
                                        session.fill_at(coord, self.fill_mode, &self.custom_fill_boundaries)
                                {
                                    self.pending_fill_warning = Some(PendingFillWarning {
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

                if !left_down {
                    *block_selection_anchor = None;
                }

                let overlay_viewport = OverlayRect {
                    min: viewport_min,
                    max: viewport_max,
                };

                let block_overlay = match (*paste, paste_target) {
                    (Some(pending), Some(target)) => {
                        draw_ghost_sprites(
                            ui,
                            camera,
                            session.clipboard_preview_sprites(target, pending.rotation),
                            overlay_viewport,
                        );
                        draw_block_outline(ui, session, camera, target, BlockSelectionMode::Full, overlay_viewport);

                        Some((target, pending.rotation))
                    },
                    _ if session.tool() == Tool::BlockSelect => session.selection().map(|source| {
                        let (displayed, rotation) = block_placement
                            .map_or((source, SelectionRotation::Original), |placement| {
                                (placement.target, placement.rotation)
                            });
                        draw_block_selection(
                            ui,
                            session,
                            camera,
                            displayed,
                            *block_placement,
                            self.block_selection_options.mode(),
                            overlay_viewport,
                        );

                        (displayed, rotation)
                    }),
                    _ => None,
                };

                if let Some((displayed, rotation)) = block_overlay
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
                        },
                        gizmo_map_view,
                    );
                }

                let block_menu_handles_escape =
                    draw_block_selection_menu(ui, session, self.block_selection_options.mode());
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
                            block_selection_anchor.is_some(),
                            self.gizmo.block_rotation_open(),
                            session.selection(),
                            *block_placement,
                        )
                    })
                    .flatten();
                let placement_action = controls_placement.and_then(|placement| {
                    let can_move = session.can_place_selected_block_with_mode(
                        placement.target.min,
                        placement.rotation,
                        SelectionPlacement::Move,
                        self.block_selection_options.mode(),
                    );
                    let can_copy = session.can_place_selected_block_with_mode(
                        placement.target.min,
                        placement.rotation,
                        SelectionPlacement::Copy,
                        self.block_selection_options.mode(),
                    );
                    let can_fill = session.can_fill_selected_block(
                        placement.target.min,
                        placement.rotation,
                        self.block_selection_options.mode(),
                    );
                    draw_block_placement_controls(
                        ui,
                        camera,
                        placement,
                        session.options.tile_size,
                        viewport_min,
                        block_controls_area,
                        (can_move, can_copy, can_fill),
                    )
                    .map(|action| (action, placement))
                });
                draw_top_overlay(
                    ui,
                    session,
                    top_overlay,
                    &mut self.block_selection_options,
                    &mut self.fill_mode,
                    &mut self.custom_fill_boundaries,
                    &mut self.custom_fill_search,
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
                            self.block_selection_options.mode(),
                        ),
                        BlockPlacementAction::Copy => session.place_selected_block_with_mode(
                            placement.target.min,
                            placement.rotation,
                            SelectionPlacement::Copy,
                            self.block_selection_options.mode(),
                        ),
                        BlockPlacementAction::Fill => session.fill_selected_block(
                            placement.target.min,
                            placement.rotation,
                            self.block_selection_options.mode(),
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
                    draw_history_overlay(ui, session, bottom_overlay, recent_prefabs.to_vec());
                }
                configure_tool_interaction(session.tool(), interaction);
                if session.focused_area().is_some() {
                    interaction.hovered_area = None;
                }

                let fill_warning_handles_escape = draw_fill_limit_warning(ui, session, &mut self.pending_fill_warning);
                let block_placement_handles_escape = block_placement.is_some() || paste.is_some();
                if ui.is_key_pressed(Key::Escape) && block_placement_handles_escape {
                    *block_placement = None;
                    *paste = None;
                    self.gizmo.cancel();
                }
                if ui.is_key_pressed(Key::Escape)
                    && !fill_warning_handles_escape
                    && !block_menu_handles_escape
                    && !block_placement_handles_escape
                    && !self.load_popup_active
                    && self.save_dialog.is_none()
                    && self.capturing_keybind.is_none()
                {
                    exit = true;
                }
            }
        });

        exit
    }

    fn draw_inspector(&mut self, ui: &Ui, session: &mut Session) -> InspectorOutput {
        let mut output = InspectorOutput::default();
        ui.window(&self.inspector_window).build(|| {
            output = self.inspector.draw(ui, session);
        });

        output
    }

    fn open_similar_instances(&mut self, session: &Session) {
        let Some(document) = session.state.active_document() else {
            return;
        };
        let Some(selected) = document.selected_instance() else {
            return;
        };
        let Some((prefab, _)) = document.prefab_instance(selected) else {
            return;
        };

        self.similar_instances = Some(SimilarInstancesState {
            document: document.id(),
            prefab_path: prefab.path.to_string(),
            instances: find_similar_instances(document, prefab),
            focus: true,
        });
    }

    fn draw_similar_instances(&mut self, ui: &Ui, session: &Session) -> Option<JumpTarget> {
        let document_id = self.similar_instances.as_ref()?.document;
        let Some(document) = session.state.document(document_id) else {
            self.similar_instances = None;

            return None;
        };
        let search = self.similar_instances.as_mut()?;
        let rows = resolve_similar_instances(document, &search.instances);
        let mut open = true;
        let mut jump = None;
        let focus = std::mem::take(&mut search.focus);

        ui.window(&self.similar_instances_window)
            .opened(&mut open)
            .size(SIMILAR_INSTANCES_WINDOW_SIZE, Condition::FirstUseEver)
            .focused(focus)
            .build(|| {
                ui.text_wrapped(&search.prefab_path);
                let suffix = if rows.len() == 1 { "instance" } else { "instances" };
                ui.text_disabled(format!("{} matching {suffix}", rows.len()));
                ui.separator();

                if rows.is_empty() {
                    ui.text_disabled("No matching instances remain");

                    return;
                }

                ui.table("similar-instances-table")
                    .flags(TableFlags::BORDERS_INNER_V | TableFlags::RESIZABLE | TableFlags::ROW_BG)
                    .sizing_policy(TableSizingPolicy::StretchProp)
                    .column("Tile")
                    .weight(0.7)
                    .done()
                    .column("Action")
                    .weight(0.3)
                    .done()
                    .build(|ui| {
                        for index in ListClipper::new(rows.len()).begin(ui).iter() {
                            let (instance, location) = rows[index];
                            let row_id = instance.get().to_string();
                            let _id = ui.push_id(&row_id);

                            ui.table_next_row();
                            ui.table_next_column();
                            ui.align_text_to_frame_padding();
                            ui.text(format!(
                                "{}, {}, {}",
                                location.coord.x, location.coord.y, location.coord.z
                            ));
                            ui.table_next_column();
                            if ui.small_button("Jump to") {
                                jump = Some(JumpTarget {
                                    document: document_id,
                                    instance,
                                });
                            }
                        }
                    });
            });

        if !open {
            self.similar_instances = None;
        }

        jump
    }

    fn jump_to_instance(&mut self, session: &mut Session, target: JumpTarget) {
        let Some(location) = session
            .state
            .document(target.document)
            .and_then(|document| document.instance_location(target.instance))
        else {
            return;
        };

        self.cancel_edit_gestures(Some(target.document));
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

fn find_similar_instances(document: &MapDocument, target: &Prefab) -> Vec<PrefabInstanceId> {
    let mut matches = document
        .prefab_instances()
        .filter_map(|(instance, prefab, location)| (prefab == target).then_some((instance, location)))
        .collect::<Vec<_>>();
    matches.sort_unstable_by_key(|(instance, location)| {
        (
            location.coord.z,
            location.coord.y,
            location.coord.x,
            location.prefab_index,
            instance.get(),
        )
    });

    matches.into_iter().map(|(instance, _)| instance).collect()
}

fn resolve_similar_instances(
    document: &MapDocument, instances: &[PrefabInstanceId],
) -> Vec<(PrefabInstanceId, PrefabLocation)> {
    instances
        .iter()
        .filter_map(|instance| {
            document
                .instance_location(*instance)
                .map(|location| (*instance, location))
        })
        .collect()
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
    handles_escape: bool,
}

fn draw_load_popup(
    ui: &Ui, window: &WindowKey, measured: &mut [f32; 2], load: Option<&LoadView>, notice: Option<&mut LoadNotice>,
) -> LoadPopup {
    let mut popup = LoadPopup::default();
    if load.is_none() && notice.is_none() {
        return popup;
    }

    popup.handles_escape = true;
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

fn draw_settings_window(
    ui: &Ui, window: &WindowKey, open: &mut bool, capturing: &mut Option<KeybindAction>, measured: &mut [f32; 2],
    session: &mut Session, settings: &mut Settings,
) {
    if !*open {
        *capturing = None;

        return;
    }

    let center = ui.main_viewport().work_center();
    let position = [center[0] - measured[0] / 2.0, center[1] - measured[1] / 2.0];
    let flags = WindowFlags::ALWAYS_AUTO_RESIZE | WindowFlags::NO_COLLAPSE | WindowFlags::NO_DOCKING;
    ui.window(window)
        .opened(open)
        .position(position, Condition::Appearing)
        .flags(flags)
        .build(|| {
            ui.checkbox("Show areas", &mut session.options.show_areas);
            ui.checkbox("Show area outlines", &mut session.options.show_area_outlines);
            ui.checkbox("Tile placement flash", &mut settings.tile_place_flash);
            ui.checkbox("Selection guide line", &mut settings.selection_guide_line);

            ui.text("Highlight");
            for highlight in SelectionHighlight::ALL {
                ui.same_line();
                if ui.radio_button(highlight.label(), settings.selection_highlight == highlight) {
                    settings.selection_highlight = highlight;
                }
            }

            ui.separator();
            ui.text("Preferred editor");
            ui.set_next_item_width(360.0);
            ui.input_text("##preferred-editor", &mut settings.preferred_editor)
                .build();
            ui.text_disabled("Placeholders: {file}, {line}, {column}");

            ui.separator();
            ui.text("Keybindings");
            for action in KeybindAction::ALL {
                ui.text(action.label());
                ui.same_line_with_pos(180.0);

                let binding = settings.keybindings.get(action).label(ui);
                let visible = if *capturing == Some(action) {
                    "Press a key..."
                } else {
                    binding.as_str()
                };
                if ui.button_with_size(format!("{visible}##keybind-{}", action.id()), [140.0, 0.0]) {
                    *capturing = (*capturing != Some(action)).then_some(action);
                }
            }

            if ui.button("Reset keybindings") {
                settings.keybindings = KeyBindings::default();
                *capturing = None;
            }

            *measured = ui.window_size();
        });

    if !*open {
        *capturing = None;
    }
}

fn finish_keybind_capture(ui: &Ui, capturing: &mut Option<KeybindAction>, keybindings: &mut KeyBindings) {
    let Some(action) = *capturing else {
        return;
    };
    if ui.is_key_pressed_with_repeat(Key::Escape, false) {
        *capturing = None;

        return;
    }
    let Some(key) = BINDABLE_KEYS
        .iter()
        .copied()
        .find(|key| ui.is_key_pressed_with_repeat(*key, false))
    else {
        return;
    };

    keybindings.rebind(action, KeyBinding::from_input(ui, key));
    *capturing = None;
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

fn draw_z_levels(ui: &Ui, session: &mut Session) {
    let current = session.z();
    let levels = session.level_count();
    ui.align_text_to_frame_padding();
    ui.text("Z");

    let mut selected = None;
    for z in 1..=levels {
        ui.same_line();

        if ui.radio_button(z.to_string(), current == z) {
            selected = Some(z);
        }
    }
    if let Some(z) = selected {
        session.set_level(z);
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

fn draw_block_selection(
    ui: &Ui, session: &Session, camera: &Controller, displayed: Selection, placement: Option<PendingBlockPlacement>,
    mode: BlockSelectionMode, viewport: OverlayRect,
) {
    if let Some(placement) = placement {
        draw_block_ghost(ui, session, camera, placement, mode, viewport);
    }

    draw_block_outline(ui, session, camera, displayed, mode, viewport);
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

fn draw_block_ghost(
    ui: &Ui, session: &Session, camera: &Controller, placement: PendingBlockPlacement, mode: BlockSelectionMode,
    viewport: OverlayRect,
) {
    let previews = session.block_preview_sprites(placement.source, placement.target, placement.rotation, mode);

    draw_ghost_sprites(ui, camera, previews, viewport);
}

fn draw_ghost_sprites(ui: &Ui, camera: &Controller, previews: Vec<BlockPreviewSprite>, viewport: OverlayRect) {
    let draw = ui.get_window_draw_list();
    draw.with_clip_rect(viewport.min, viewport.max, || {
        for BlockPreviewSprite {
            sprite,
            uv0,
            uv1,
            mut tint,
        } in previews
        {
            let left = sprite.x;
            let bottom = sprite.y;
            let top_left = camera.map_to_screen([left, bottom + sprite.height]);
            let bottom_right = camera.map_to_screen([left + sprite.width, bottom]);
            tint[3] *= BLOCK_GHOST_OPACITY;
            draw.add_image(
                Renderer::sprite_texture(sprite.texture.index),
                [viewport.min[0] + top_left[0], viewport.min[1] + top_left[1]],
                [viewport.min[0] + bottom_right[0], viewport.min[1] + bottom_right[1]],
                uv0,
                uv1,
                tint,
            );
        }
    });
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

fn draw_block_selection_menu(ui: &Ui, session: &mut Session, mode: BlockSelectionMode) -> bool {
    let was_open = ui.is_popup_open(BLOCK_SELECTION_POPUP);
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

    was_open
}

fn block_controls_placement(
    tool: Tool, selecting: bool, rotation_open: bool, selection: Option<Selection>,
    pending: Option<PendingBlockPlacement>,
) -> Option<PendingBlockPlacement> {
    (tool == Tool::BlockSelect && !selecting && !rotation_open)
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
    ui: &Ui, camera: &Controller, labels: &[&str], target: Selection, tile_size: u32, viewport_min: [f32; 2],
    controls_bounds: OverlayRect,
) -> ([f32; 2], OverlayRect) {
    let style = ui.clone_style();
    let padding = style.frame_padding();
    let spacing = style.item_spacing()[0];
    let width = labels
        .iter()
        .map(|label| ui.calc_text_size(*label)[0] + padding[0] * 2.0)
        .sum::<f32>()
        + spacing * labels.len().saturating_sub(1) as f32;
    let height = ui.frame_height();
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
        (center[1] + 14.0).clamp(min_y, max_y),
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
    ui: &Ui, camera: &Controller, placement: PendingBlockPlacement, tile_size: u32, viewport_min: [f32; 2],
    controls_bounds: OverlayRect, enabled: (bool, bool, bool),
) -> Option<BlockPlacementAction> {
    let (can_move, can_copy, can_fill) = enabled;
    let (position, bounds) = block_placement_controls_layout(
        ui,
        camera,
        &BLOCK_PLACEMENT_LABELS,
        placement.target,
        tile_size,
        viewport_min,
        controls_bounds,
    );
    draw_overlay_underlay(ui, bounds);
    ui.set_cursor_screen_pos(position);

    let mut action = None;
    if ui.with_disabled_if(!can_move, || ui.button(BLOCK_PLACEMENT_LABELS[0])) {
        action = Some(BlockPlacementAction::Move);
    }
    ui.same_line();
    if ui.with_disabled_if(!can_copy, || ui.button(BLOCK_PLACEMENT_LABELS[1])) {
        action = Some(BlockPlacementAction::Copy);
    }
    ui.same_line();
    if ui.with_disabled_if(!can_fill, || ui.button(BLOCK_PLACEMENT_LABELS[2])) {
        action = Some(BlockPlacementAction::Fill);
    }
    ui.same_line();
    if ui.button(BLOCK_PLACEMENT_LABELS[3]) {
        action = Some(BlockPlacementAction::Cancel);
    }
    if action.is_none() && !ui.io().want_text_input() && ui.is_key_pressed(Key::Enter) && can_move {
        action = Some(BlockPlacementAction::Move);
    }

    action
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
        &PASTE_LABELS,
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
    if action.is_none() && !ui.io().want_text_input() && ui.is_key_pressed(Key::Enter) && can_paste {
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

fn draw_top_overlay(
    ui: &Ui, session: &mut Session, bounds: OverlayRect, block_selection_options: &mut BlockSelectionOptions,
    fill_mode: &mut FillMode, custom_fill_boundaries: &mut Vec<TreePath>, custom_fill_search: &mut String,
) {
    draw_overlay_underlay(ui, bounds);

    ui.set_cursor_screen_pos([bounds.min[0] + OVERLAY_PADDING, bounds.min[1] + OVERLAY_PADDING]);

    draw_tool_button(ui, session, Tool::Place, ICON_PENCIL);
    ui.same_line();
    draw_tool_button(ui, session, Tool::Select, ICON_EYEDROPPER);
    ui.same_line();
    draw_block_select_tool_button(ui, session, block_selection_options);
    ui.same_line();
    draw_tool_button(ui, session, Tool::Delete, ICON_ERASER);
    ui.same_line();
    let tools_end = draw_fill_tool_button(ui, session, fill_mode, custom_fill_boundaries, custom_fill_search);

    let levels_width = z_level_width(ui, session.level_count());
    let levels_x = (bounds.max[0] - OVERLAY_PADDING - levels_width).max(tools_end + OVERLAY_PADDING);
    ui.set_cursor_screen_pos([levels_x, bounds.min[1] + OVERLAY_PADDING]);
    draw_z_levels(ui, session);
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

fn draw_fill_limit_warning(ui: &Ui, session: &mut Session, pending: &mut Option<PendingFillWarning>) -> bool {
    let was_pending = pending.is_some();
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
    {
        session.fill_at_unlimited(warning.coord, warning.fill_mode, &warning.custom_fill_boundaries);
    }

    was_pending
}

fn draw_block_select_tool_button(ui: &Ui, session: &mut Session, options: &mut BlockSelectionOptions) {
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
    ui.set_item_tooltip(format!("Block Select ({})", options.label()));
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
        ui.checkbox("Full rectangle", &mut options.full_rectangle);
        if !options.full_rectangle {
            ui.text("Line width");
            ui.set_next_item_width(BLOCK_SELECTION_LINE_WIDTH_DRAG_WIDTH);
            ui.drag_int_config("##block-selection-line-width")
                .range(1, i32::MAX)
                .flags(DragFlags::ALWAYS_CLAMP)
                .build(ui, &mut options.line_width);
        }
    }
}

fn draw_fill_tool_button(
    ui: &Ui, session: &mut Session, fill_mode: &mut FillMode, custom_fill_boundaries: &mut Vec<TreePath>,
    custom_fill_search: &mut String,
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
    ui.set_item_tooltip(format!("Fill ({})", fill_mode.label()));
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

fn matching_object_types(tree: &ObjectTree, query: &str, options: ObjectTreeSearchOptions) -> Vec<TypeId> {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return Vec::new();
    }

    let name = Identifier::from("name");
    let mut matches = tree
        .iter()
        .filter(|decl| {
            let path_matches = options.type_paths && decl.path.to_string().to_ascii_lowercase().contains(&query);
            let name_matches = options.names
                && tree
                    .var_inherited(decl.id, &name)
                    .and_then(|variable| variable.value.as_text())
                    .is_some_and(|value| value.to_ascii_lowercase().contains(&query));

            path_matches || name_matches
        })
        .map(|decl| decl.id)
        .collect::<Vec<_>>();
    matches.sort_by_key(|id| tree.get(*id).map(|decl| decl.path.to_string()).unwrap_or_default());

    matches
}

// im not sure why every single fucking icons appear not centered fuck you
fn draw_centered_icon(ui: &Ui, icon: char) {
    let item_min = ui.item_rect_min();
    let item_max = ui.item_rect_max();
    let center = [(item_min[0] + item_max[0]) * 0.5, (item_min[1] + item_max[1]) * 0.5];
    let glyph = ui.current_baked_font().glyph(icon);
    let icon = icon.to_string();
    let position = glyph.map_or_else(
        || {
            let size = ui.calc_text_size(&icon);
            [center[0] - size[0] * 0.5, center[1] - size[1] * 0.5]
        },
        |glyph| {
            let (glyph_min, glyph_max) = glyph.position_and_size();
            [
                center[0] - (glyph_min[0] + glyph_max[0]) * 0.5,
                center[1] - (glyph_min[1] + glyph_max[1]) * 0.5,
            ]
        },
    );

    ui.get_window_draw_list()
        .add_text(position, ui.style_color(StyleColor::Text), icon);
}

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

fn draw_history_overlay(ui: &Ui, session: &mut Session, bounds: OverlayRect, recent_prefabs: Vec<Prefab>) {
    draw_overlay_underlay(ui, bounds);

    let button_size = recent_button_size(ui);
    let palette = session.palette().cloned();
    let mut chosen = None;
    ui.set_cursor_screen_pos([bounds.min[0] + OVERLAY_PADDING, bounds.min[1] + OVERLAY_PADDING]);

    for (index, prefab) in recent_prefabs.iter().enumerate() {
        if index > 0 {
            ui.same_line();
        }
        let key = recent_key_label(index);
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
        draw_recent_badge(ui, key);
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

fn pressed_recent(ui: &Ui) -> Option<usize> {
    RECENT_KEYS
        .iter()
        .position(|(number, keypad)| ui.is_key_pressed(*number) || ui.is_key_pressed(*keypad))
}

const RECENT_KEYS: [(Key, Key); 10] = [
    (Key::Key1, Key::Keypad1),
    (Key::Key2, Key::Keypad2),
    (Key::Key3, Key::Keypad3),
    (Key::Key4, Key::Keypad4),
    (Key::Key5, Key::Keypad5),
    (Key::Key6, Key::Keypad6),
    (Key::Key7, Key::Keypad7),
    (Key::Key8, Key::Keypad8),
    (Key::Key9, Key::Keypad9),
    (Key::Key0, Key::Keypad0),
];

fn recent_key_label(index: usize) -> char {
    const LABELS: [char; 10] = ['1', '2', '3', '4', '5', '6', '7', '8', '9', '0'];

    LABELS.get(index).copied().unwrap_or('?')
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

fn draw_recent_badge(ui: &Ui, key: char) {
    let key = key.to_string();
    let item_min = ui.item_rect_min();
    let text_size = ui.calc_text_size(&key);
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
    let inner_spacing = style.item_inner_spacing()[0];
    let radio = ui.frame_height();
    let mut width = ui.calc_text_size("Z")[0];

    for z in 1..=levels {
        width += item_spacing + radio + inner_spacing + ui.calc_text_size(z.to_string())[0];
    }

    width
}

fn collect_type_rows(
    ui: &Ui, tree: &ObjectTree, id: TypeId, parent: Option<usize>, rows: &mut Vec<ObjectTreeRow>,
    options: &ObjectTreeRowOptions<'_>,
) {
    let Some(decl) = tree.get(id) else {
        return;
    };
    let children = options.filter.map_or_else(
        || visible_type_children(tree, id, options.type_filter),
        |filter| filter.children(id).to_vec(),
    );
    let leaf = children.is_empty();
    let row = rows.len();
    rows.push(ObjectTreeRow { id, parent, leaf });

    if leaf {
        return;
    }

    let node_id = decl.path.to_string();
    let storage_id = ui.get_id(&node_id);
    ui.with_current_state_storage(|mut storage| {
        let initialize_atom = tree.roots().atom == Some(id) && storage.get_int(storage_id, -1) == -1;
        if options.expand || initialize_atom {
            storage.set_bool(storage_id, true);
        }
    });
    if !ui.tree_node_get_open(storage_id) {
        return;
    }

    let _id = ui.push_id(&node_id);
    for child in children {
        collect_type_rows(ui, tree, child, Some(row), rows, options);
    }
}

fn draw_type_row(
    ui: &Ui, session: &Session, tree: &ObjectTree, rows: &[ObjectTreeRow], row: usize, selected: &mut Option<TypeId>,
    output: &mut ObjectTreeOutput,
) {
    let Some(row) = rows.get(row).copied() else {
        return;
    };
    let Some(decl) = tree.get(row.id) else {
        return;
    };

    let mut ancestors = Vec::new();
    let mut parent = row.parent;
    while let Some(index) = parent {
        let Some(ancestor) = rows.get(index) else {
            break;
        };
        ancestors.push(ancestor.id);
        parent = ancestor.parent;
    }
    let mut scopes = Vec::with_capacity(ancestors.len());
    for ancestor in ancestors.into_iter().rev() {
        if let Some(ancestor) = tree.get(ancestor) {
            scopes.push(ui.tree_push(ancestor.path.to_string()));
        }
    }

    let label = decl
        .path
        .segments
        .last()
        .map_or_else(|| decl.path.to_string(), ToString::to_string);
    let node_id = decl.path.to_string();
    {
        ui.table_next_row();
        ui.table_next_column();
        let cursor = ui.cursor_screen_pos();
        let icon_extent = ui.text_line_height();
        let icon_spacing = ui.clone_style().item_inner_spacing()[0];
        let space_width = ui.calc_text_size(" ")[0].max(1.0);
        let icon_padding = " ".repeat(((icon_extent + icon_spacing) / space_width).ceil() as usize);
        let _token = ui
            .tree_node_config(&node_id)
            .label(format!("{icon_padding}{label}"))
            .selected(*selected == Some(row.id))
            .leaf(row.leaf)
            .no_tree_push_on_open(true)
            .frame_padding(true)
            .span_avail_width(true)
            .push();
        draw_type_icon(ui, session.type_thumbnail(row.id), cursor);

        if ui.is_item_clicked() {
            *selected = Some(row.id);
            output.chosen = Some(row.id);
        }
        ui.set_item_tooltip(&node_id);
        let source = session.type_source(row.id);
        if let Some(_popup) = ui.begin_popup_context_item()
            && ui.menu_item_enabled_selected_no_shortcut("Open in editor", false, source.is_some())
        {
            output.open_source = source;
        }

        ui.table_next_column();
        let visible = session.is_type_visible(row.id);
        let icon = if visible { ICON_EYE } else { ICON_EYE_OFF };
        let transparent = [0.0, 0.0, 0.0, 0.0];
        let _button = ui.push_style_color(StyleColor::Button, transparent);
        let _button_hovered = ui.push_style_color(StyleColor::ButtonHovered, transparent);
        let _button_active = ui.push_style_color(StyleColor::ButtonActive, transparent);
        let _text = (!visible).then(|| ui.push_style_color(StyleColor::Text, ui.style_color(StyleColor::TextDisabled)));
        if ui.button_with_size(
            format!("{icon}##object-type-visibility-{node_id}"),
            [ui.content_region_avail()[0], ui.frame_height()],
        ) {
            output.visibility_toggle = Some(row.id);
        }
        ui.set_item_tooltip(if visible {
            format!("Hide {node_id} and its descendants")
        } else {
            format!("Show {node_id} and its descendants")
        });
    }

    while let Some(scope) = scopes.pop() {
        scope.pop();
    }
}

fn draw_type_icon(ui: &Ui, thumbnail: Option<crate::session::PrefabThumbnail>, cursor: [f32; 2]) {
    let row_min = ui.item_rect_min();
    let row_max = ui.item_rect_max();
    let row_height = (row_max[1] - row_min[1]).max(1.0);
    let icon_extent = ui.text_line_height().min(row_height);
    let icon_slot_x = cursor[0] + ui.tree_node_to_label_spacing();
    let draw = ui.get_window_draw_list();

    match thumbnail {
        Some(thumbnail) => {
            let image_size = fit_icon(thumbnail.texture.width, thumbnail.texture.height, icon_extent);
            let image_min = [
                icon_slot_x + (icon_extent - image_size[0]) * 0.5,
                row_min[1] + (row_height - image_size[1]) * 0.5,
            ];
            let image_max = [image_min[0] + image_size[0], image_min[1] + image_size[1]];
            draw.add_image(
                Renderer::sprite_texture(thumbnail.texture.index),
                image_min,
                image_max,
                thumbnail.uv0,
                thumbnail.uv1,
                thumbnail.tint,
            );
        },
        None => {
            let fallback = ICON_IMAGE_BROKEN.to_string();
            let fallback_width = ui.calc_text_size(&fallback)[0];
            draw.add_text(
                [
                    icon_slot_x + (icon_extent - fallback_width) * 0.5,
                    row_min[1] + (row_height - ui.text_line_height()) * 0.5,
                ],
                ui.style_color(StyleColor::TextDisabled),
                fallback,
            );
        },
    }
}

fn sorted_children(tree: &ObjectTree, parent: TypeId) -> Vec<TypeId> {
    let mut children = tree
        .get(parent)
        .into_iter()
        .flat_map(|decl| decl.children.iter().copied())
        .filter(|id| tree.get(*id).is_some())
        .collect::<Vec<_>>();

    children.sort_by(|left, right| {
        let left = tree.get(*left).map(|decl| decl.path.to_string()).unwrap_or_default();
        let right = tree.get(*right).map(|decl| decl.path.to_string()).unwrap_or_default();

        left.cmp(&right)
    });

    children
}

fn visible_type_roots(tree: &ObjectTree, root: TypeId, filter: ObjectTreeTypeFilter) -> Vec<TypeId> {
    match filter.action(tree, root) {
        ObjectTreeFilterAction::Keep => vec![root],
        ObjectTreeFilterAction::Prune => Vec::new(),
    }
}

fn visible_type_children(tree: &ObjectTree, parent: TypeId, filter: ObjectTreeTypeFilter) -> Vec<TypeId> {
    let mut visible = Vec::new();
    for child in sorted_children(tree, parent) {
        match filter.action(tree, child) {
            ObjectTreeFilterAction::Keep => visible.push(child),
            ObjectTreeFilterAction::Prune => {},
        }
    }

    visible
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
mod tests {
    use core::{
        location::Location,
        path::TreePath,
        types::{Identifier, Value, VarModifiers},
    };

    use dmm::{Map, PrefabInstanceId, Size};
    use editor::command::Edit;
    use objtree::VarDecl;
    use render::{HighlightStyle, SpriteTexture};

    use super::*;

    fn set_type_name(tree: &mut ObjectTree, id: TypeId, value: &str) {
        let name = Identifier::from("name");
        tree.get_mut(id).unwrap().vars.insert(
            name.clone(),
            VarDecl {
                name,
                declared_type: None,
                modifiers: VarModifiers::default(),
                value: Value::Text(value.to_owned()),
                location: Location::default(),
            },
        );
    }

    fn similar_instances_map() -> (Map, Prefab) {
        let mut target = Prefab::new(TreePath::parse("/obj/table"));
        target.set_var("name".into(), Value::Text(String::from("Conference")));
        let mut different_override = target.clone();
        different_override.set_var("name".into(), Value::Text(String::from("Coffee")));
        let different_path = Prefab::new(TreePath::parse("/obj/chair"));

        let mut map = Map::new(Size { x: 2, y: 1, z: 2 });
        let first = map.intern_tile(vec![target.clone(), different_override.clone()]);
        let second = map.intern_tile(vec![different_path, target.clone()]);
        let third = map.intern_tile(vec![target.clone()]);
        let fourth = map.intern_tile(vec![different_override]);
        map.grid[0][0][0] = first;
        map.grid[0][0][1] = second;
        map.grid[1][0][0] = third;
        map.grid[1][0][1] = fourth;

        (map, target)
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
        let state = UiState::new().expect("valid window keys");

        assert_eq!(state.layout.validate(), Ok(()));
        assert_eq!(state.block_selection_options, BlockSelectionOptions::default());
        assert_eq!(state.fill_mode, FillMode::Wall);
        assert!(state.object_tree_search.is_empty());
        assert!(state.object_tree_filter.is_none());
        assert!(state.similar_instances.is_none());
        assert_eq!(
            state.custom_fill_boundaries,
            [TreePath::parse(DEFAULT_CUSTOM_FILL_BOUNDARY)]
        );
        assert!(state.custom_fill_search.is_empty());
        assert!(state.pending_fill_warning.is_none());
        assert!(state.new_map_dialog.is_none());
        assert!(state.load_notice.is_none());
        assert!(!state.load_popup_active);
        assert!(state.save_dialog.is_none());
        assert!(state.show_welcome);
        assert!(state.open_error.is_none());
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
        let state = UiState::new().expect("valid window keys");
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
    fn similar_instances_match_the_exact_prefab_across_levels_in_tile_order() {
        let (map, target) = similar_instances_map();
        let document = MapDocument::new(map, 1);

        assert_eq!(
            find_similar_instances(&document, &target),
            [
                document.instance_ids_at(Coord::new(1, 1, 1))[0],
                document.instance_ids_at(Coord::new(2, 1, 1))[1],
                document.instance_ids_at(Coord::new(1, 1, 2))[0],
            ]
        );
    }

    #[test]
    fn similar_instance_snapshots_follow_moves_and_drop_deleted_placements() {
        let (map, target) = similar_instances_map();
        let mut document = MapDocument::new(map, 1);
        let matches = find_similar_instances(&document, &target);
        let moved = matches[0];
        let deleted = matches[2];

        assert_eq!(
            document.move_instance(moved, Coord::new(2, 1, 1), "move table", &[], None),
            Some(true)
        );
        let deleted_location = document.instance_location(deleted).unwrap();
        let mut after = document.placed_tile(deleted_location.coord).unwrap();
        after.remove(deleted_location.prefab_index);
        let mut edit = Edit::new("delete table");
        edit.change(&document, deleted_location.coord, after);
        assert!(document.apply(edit));

        let rows = resolve_similar_instances(&document, &matches);
        assert_eq!(rows.len(), matches.len() - 1);
        assert_eq!(
            rows.iter()
                .find(|(instance, _)| *instance == moved)
                .map(|(_, location)| location.coord),
            Some(Coord::new(2, 1, 1))
        );
        assert!(rows.iter().all(|(instance, _)| *instance != deleted));
    }

    #[test]
    fn jumping_to_an_instance_selects_its_level_and_centers_the_map_view() {
        let (map, _) = similar_instances_map();
        let document = MapDocument::new(map, 1);
        let instance = document.instance_ids_at(Coord::new(1, 1, 2))[0];
        let mut session = Session::new();
        let document_id = session.state.open_document(document);
        let mut state = UiState::new().expect("valid window keys");
        let mut view = MapViewState::new(document_id).expect("valid map view key");
        view.camera.camera.zoom = 2.5;
        state.map_views.insert(document_id, view);

        state.jump_to_instance(
            &mut session,
            JumpTarget {
                document: document_id,
                instance,
            },
        );

        assert_eq!(session.state.active(), Some(document_id));
        assert_eq!(session.z(), 2);
        assert_eq!(session.selected_instance(), Some(instance));
        let view = state.map_views.get(&document_id).unwrap();
        assert_eq!((view.camera.camera.x, view.camera.camera.y), (16.0, 16.0));
        assert_eq!(view.camera.camera.zoom, 2.5);
        assert!(view.focus);
        assert!(!view.refit);
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
    fn object_tree_search_is_not_limited_to_custom_fill_result_count() {
        let mut tree = ObjectTree::new();
        for index in 0..60 {
            tree.register(
                &TreePath::parse(&format!("/obj/floor/type_{index}")),
                Location::default(),
            );
        }

        assert_eq!(
            matching_type_paths(&tree, "floor").len(),
            MAX_CUSTOM_FILL_SEARCH_RESULTS
        );
        assert_eq!(
            matching_object_types(&tree, "floor", ObjectTreeSearchOptions::default()).len(),
            61
        );
    }

    #[test]
    fn object_tree_search_matches_inherited_atom_names() {
        let mut tree = ObjectTree::new();
        let atom = tree.register(&TreePath::parse("/atom"), Location::default());
        let parent = tree.register(&TreePath::parse("/atom/movable/tool"), Location::default());
        let child = tree.register(&TreePath::parse("/atom/movable/tool/wrench"), Location::default());
        tree.register(&TreePath::parse("/atom/structure/table"), Location::default());
        set_type_name(&mut tree, parent, "Portable Tool");

        let options = ObjectTreeSearchOptions {
            type_paths: false,
            names: true,
        };
        let matches = matching_object_types(&tree, "PORTABLE", options);
        let filter = ObjectTreeFilter::new(&tree, atom, "PORTABLE", options, ObjectTreeTypeFilter::default());

        assert_eq!(matches, [parent, child]);
        assert_eq!(filter.roots, [parent]);
        assert_eq!(filter.children(parent), [child]);
    }

    #[test]
    fn object_tree_search_combines_enabled_fields() {
        let mut tree = ObjectTree::new();
        tree.register(&TreePath::parse("/atom"), Location::default());
        let path_match = tree.register(&TreePath::parse("/atom/movable/needle"), Location::default());
        let name_match = tree.register(&TreePath::parse("/atom/movable/scalpel"), Location::default());
        set_type_name(&mut tree, name_match, "Needle tool");

        let path_only = matching_object_types(
            &tree,
            "needle",
            ObjectTreeSearchOptions {
                type_paths: true,
                names: false,
            },
        );
        let name_only = matching_object_types(
            &tree,
            "needle",
            ObjectTreeSearchOptions {
                type_paths: false,
                names: true,
            },
        );
        let combined = matching_object_types(
            &tree,
            "needle",
            ObjectTreeSearchOptions {
                type_paths: true,
                names: true,
            },
        );

        assert_eq!(path_only, [path_match]);
        assert_eq!(name_only, [name_match]);
        assert_eq!(combined, [path_match, name_match]);
    }

    #[test]
    fn object_tree_children_are_sorted_by_path() {
        let mut tree = ObjectTree::new();
        tree.register(&TreePath::parse("/obj/zeta"), Location::default());
        tree.register(&TreePath::parse("/obj/alpha"), Location::default());
        let obj = tree.id_of(&TreePath::parse("/obj")).expect("registered parent");
        let paths = sorted_children(&tree, obj)
            .into_iter()
            .filter_map(|id| tree.get(id))
            .map(|decl| decl.path.to_string())
            .collect::<Vec<_>>();

        assert_eq!(paths, ["/obj/alpha", "/obj/zeta"]);
    }

    #[test]
    fn built_in_object_tree_filters_prune_the_base_types_and_all_subtypes() {
        let mut tree = ObjectTree::new();
        let atom = tree.register(&TreePath::parse("/atom"), Location::default());
        let movable = tree.register(&TreePath::parse("/atom/movable"), Location::default());
        let item = tree.register(&TreePath::parse("/obj/item"), Location::default());
        let open_turf = tree.register(&TreePath::parse("/turf/open"), Location::default());
        let roots = tree.roots();
        tree.get_mut(roots.obj.unwrap()).unwrap().parent_type = Some(TreePath::parse("/atom/movable"));
        tree.get_mut(roots.turf.unwrap()).unwrap().parent_type = Some(TreePath::parse("/atom"));
        tree.resolve_parent_types();

        for (options, filtered_root, filtered_subtype) in [
            (
                ObjectTreeFilterOptions {
                    atom: true,
                    ..ObjectTreeFilterOptions::default()
                },
                atom,
                open_turf,
            ),
            (
                ObjectTreeFilterOptions {
                    movable: true,
                    ..ObjectTreeFilterOptions::default()
                },
                movable,
                item,
            ),
            (
                ObjectTreeFilterOptions {
                    obj: true,
                    ..ObjectTreeFilterOptions::default()
                },
                roots.obj.unwrap(),
                item,
            ),
            (
                ObjectTreeFilterOptions {
                    turf: true,
                    ..ObjectTreeFilterOptions::default()
                },
                roots.turf.unwrap(),
                open_turf,
            ),
        ] {
            let filter = ObjectTreeTypeFilter::new(&tree, &options);

            assert_eq!(filter.action(&tree, filtered_root), ObjectTreeFilterAction::Prune);
            assert_eq!(filter.action(&tree, filtered_subtype), ObjectTreeFilterAction::Prune);
        }
    }

    #[test]
    fn custom_object_tree_filter_prunes_the_type_and_all_subtypes() {
        let mut tree = ObjectTree::new();
        let atom = tree.register(&TreePath::parse("/atom"), Location::default());
        let keep = tree.register(&TreePath::parse("/atom/keep"), Location::default());
        let removed = tree.register(&TreePath::parse("/atom/remove"), Location::default());
        let removed_child = tree.register(&TreePath::parse("/atom/remove/child"), Location::default());
        let options = ObjectTreeFilterOptions {
            custom_enabled: true,
            custom_type_path: String::from("/atom/remove"),
            ..ObjectTreeFilterOptions::default()
        };
        let filter = ObjectTreeTypeFilter::new(&tree, &options);

        assert_eq!(visible_type_children(&tree, atom, filter), [keep]);
        assert_eq!(filter.action(&tree, removed), ObjectTreeFilterAction::Prune);
        assert_eq!(filter.action(&tree, removed_child), ObjectTreeFilterAction::Prune);
    }

    #[test]
    fn object_tree_search_respects_the_active_type_filters() {
        let mut tree = ObjectTree::new();
        let atom = tree.register(&TreePath::parse("/atom"), Location::default());
        let keep = tree.register(&TreePath::parse("/atom/keep_match"), Location::default());
        tree.register(&TreePath::parse("/atom/remove/keep_match"), Location::default());
        let type_filter = ObjectTreeTypeFilter::new(
            &tree,
            &ObjectTreeFilterOptions {
                custom_enabled: true,
                custom_type_path: String::from("/atom/remove"),
                ..ObjectTreeFilterOptions::default()
            },
        );

        let filter = ObjectTreeFilter::new(
            &tree,
            atom,
            "keep_match",
            ObjectTreeSearchOptions::default(),
            type_filter,
        );

        assert_eq!(filter.roots, [keep]);
        assert!(filter.children(keep).is_empty());
    }

    #[test]
    fn object_tree_rows_follow_expansion_and_render_a_clipped_search() {
        let mut tree = ObjectTree::new();
        tree.register(&TreePath::parse("/atom/structure/thing"), Location::default());
        tree.register(&TreePath::parse("/atom/movable/item"), Location::default());
        let atom = tree.roots().atom.expect("registered /atom root");
        let mut context = dear_imgui_rs::Context::create();
        context
            .font_atlas()
            .try_claim_legacy_renderer()
            .expect("legacy renderer font atlas should be available")
            .build();
        context.io_mut().set_display_size([128.0, 128.0]);
        context.io_mut().set_delta_time(1.0 / 60.0);
        let ui = context.frame();

        let expanded = ui
            .window("object-tree-expanded")
            .build(|| {
                let mut rows = Vec::new();
                collect_type_rows(
                    ui,
                    &tree,
                    atom,
                    None,
                    &mut rows,
                    &ObjectTreeRowOptions {
                        expand: true,
                        ..ObjectTreeRowOptions::default()
                    },
                );
                rows
            })
            .expect("test window should be visible");
        let paths = expanded
            .iter()
            .filter_map(|row| tree.get(row.id))
            .map(|decl| decl.path.to_string())
            .collect::<Vec<_>>();
        let parents = expanded.iter().map(|row| row.parent).collect::<Vec<_>>();

        assert_eq!(
            paths,
            [
                "/atom",
                "/atom/movable",
                "/atom/movable/item",
                "/atom/structure",
                "/atom/structure/thing"
            ]
        );
        assert_eq!(parents, [None, Some(0), Some(1), Some(0), Some(3)]);

        let opened_by_default = ui
            .window("object-tree-default-open")
            .build(|| {
                let mut rows = Vec::new();
                collect_type_rows(ui, &tree, atom, None, &mut rows, &ObjectTreeRowOptions::default());
                rows
            })
            .expect("test window should be visible");
        let default_paths = opened_by_default
            .iter()
            .filter_map(|row| tree.get(row.id))
            .map(|decl| decl.path.to_string())
            .collect::<Vec<_>>();
        assert_eq!(default_paths, ["/atom", "/atom/movable", "/atom/structure"]);

        let collapsed = ui
            .window("object-tree-collapsed")
            .build(|| {
                let atom_storage_id = ui.get_id("/atom");
                ui.with_current_state_storage(|mut storage| storage.set_bool(atom_storage_id, false));
                let mut rows = Vec::new();
                collect_type_rows(ui, &tree, atom, None, &mut rows, &ObjectTreeRowOptions::default());
                rows
            })
            .expect("test window should be visible");
        assert_eq!(
            collapsed,
            [ObjectTreeRow {
                id: atom,
                parent: None,
                leaf: false
            }]
        );

        let mut tree = ObjectTree::new();
        for index in 0..500 {
            tree.register(
                &TreePath::parse(&format!("/atom/common/type_{index}")),
                Location::default(),
            );
        }
        let mut session = Session::new();
        session.state.environment = Some(editor::Environment::new(".", tree));
        let mut state = UiState::new().expect("valid window keys");
        state.object_tree_search = String::from("atom");
        state.object_tree_filter_revision = u64::MAX;
        let mut settings = Settings::default();

        state.draw_object_tree(ui, &mut session, &mut settings);

        assert!(context.render_legacy().valid());
    }

    #[test]
    fn panel_extent_rejects_non_finite_or_empty_sizes() {
        assert_eq!(panel_extent([0.0, f32::NAN]), ([1.0, 1.0], (1, 1)));
        assert_eq!(panel_extent([320.75, 200.25]), ([320.75, 200.25], (320, 200)));
    }

    #[test]
    fn recent_slots_follow_the_number_row_order() {
        assert_eq!((0..10).map(recent_key_label).collect::<String>(), "1234567890");
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
        assert_eq!(KeybindAction::ALL.len(), 15);
        assert!(KeybindAction::ALL.contains(&KeybindAction::Undo));
        assert!(KeybindAction::ALL.contains(&KeybindAction::Redo));
        assert!(KeybindAction::ALL.contains(&KeybindAction::Copy));
        assert!(KeybindAction::ALL.contains(&KeybindAction::Paste));
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

        // the width stays configured while the full rectangle is selected, and is ignored
        options.line_width = 4;
        assert_eq!(options.mode(), BlockSelectionMode::Full);

        options.full_rectangle = false;
        assert_eq!(options.mode(), BlockSelectionMode::Hollow { line_width: 4 });
        assert_eq!(options.label(), "Hollow, 4-tile line");

        options.line_width = 1;
        assert_eq!(options.mode(), BlockSelectionMode::Hollow { line_width: 1 });
        assert_eq!(options.label(), "Hollow, 1-tile line");

        // the drag widget clamps to one, but a stale or hand-edited value must not wrap round the cast
        for line_width in [0, -1, i32::MIN] {
            options.line_width = line_width;
            assert_eq!(options.mode(), BlockSelectionMode::Hollow { line_width: 1 });
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
