use core::{
    path::TreePath,
    types::{Identifier, Value},
};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};

use dmi::{IconFile, metadata::Dir};
use dmm::{Coord, Map, MapFormat, Prefab, Size};
use editor::{
    EditorState,
    Environment,
    clipboard::{self, TileBlock},
    command::EditGroupId,
    document::{DocumentId, MapDocument, PrefabInstanceId, PrefabLocation, Selection, VarMutation},
    focus::AreaFocus,
    frame::{self, FrameInstances, FrameOptions, FrameRenderOptions, PrefabUpdate, TypeVisibility},
    progress::{Progress, Stage},
    tool::{
        BlockSelectionMode,
        FillError,
        FillMode,
        MAX_FILL_TILES,
        SelectionMask,
        SelectionPlacement,
        SelectionRotation,
        SelectionTransform,
        Tool,
        ToolContext,
        ToolEdit,
        default_tile_paths,
        fill_selection as build_selection_fill,
        is_placeable,
        place_selection_with_mode as build_selection_placement,
        rotate_point,
        rotate_prefab,
        rotated_selection_at,
        transform_selection_with_mode as build_selection_transform,
        transformed_selection,
    },
    visual,
};
use objtree::{ObjectTree, TypeId};
use render::{
    Frame,
    FrameUpdate,
    GuideLine,
    MapViewFrame,
    MapViewInteraction,
    MapViewRect,
    SpriteInstance,
    SpritePreview,
    SpriteTexture,
    texture::TextureCatalog,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct GuideBadge {
    pub position: [f32; 2],
    pub z: u32,
}

#[derive(Debug, Default)]
pub(crate) struct SelectionGuides {
    pub lines: Vec<GuideLine>,
    pub badges: Vec<GuideBadge>,
    pub connected: Vec<PrefabInstanceId>,
}

use crate::{
    baker::{self, Baker},
    external_editor::SourceLocation,
    loader::{LoadedCodebase, LoadedMap},
};

fn always_highlighted(bake: Option<&editor::bake::Bake>) -> Vec<PrefabInstanceId> {
    let Some(bake) = bake else {
        return Vec::new();
    };

    let mut always = bake
        .highlighted()
        .filter(|(_, list)| {
            list.iter()
                .any(|highlight| highlight.shown_when(editor::bake::HIGHLIGHT_ALWAYS))
        })
        .filter_map(|(id, _)| PrefabInstanceId::from_raw(id))
        .collect::<Vec<_>>();
    always.sort_unstable();

    always
}

fn report_bake_output(bake: &mut editor::bake::Bake) {
    for line in bake.take_output() {
        log::info!("DM: {line}");
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SelectedTransform {
    pub selected: PrefabInstanceId,
    pub sprite: SpriteInstance,
    pub pixel: [i32; 2],
    pub step: [i32; 2],
    pub is_movable: bool,
    pub dir: u32,
    pub dmi_directions: Option<u32>,
    pub directional_types: Option<DirectionalTypes>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DirectionalTypes {
    pub supported: [bool; 8],
    pub current: Option<Dir>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DirectionState {
    pub dir: u32,
    pub dmi_directions: Option<u32>,
    pub directional_types: Option<DirectionalTypes>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PrefabThumbnail {
    pub texture: SpriteTexture,
    pub uv0: [f32; 2],
    pub uv1: [f32; 2],
    pub tint: [f32; 4],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PlacementPreview {
    pub thumbnail: PrefabThumbnail,
    pub offset: [i32; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockPreviewSource {
    Selection(SelectionMask),
    Clipboard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BlockPreviewKey {
    source: BlockPreviewSource,
    rotation: SelectionRotation,
    document_revision: u64,
    clipboard_revision: u64,
    texture_revision: u64,
    tile_size: u32,
    show_areas: bool,
}

struct BlockPreviewCache {
    key: BlockPreviewKey,
    sprites: Vec<SpriteInstance>,
    revision: u64,
    destination: Coord,
    visible: bool,
}

const MAX_REPORTED_DIAGNOSTICS: usize = 500;
const MAX_MAP_DIMENSION: u32 = 255;
const STANDALONE_CACHE_LIMIT: usize = 256;

const PREVIEW_OWNER: PrefabInstanceId = match PrefabInstanceId::from_raw(1) {
    Some(id) => id,
    None => unreachable!(),
};

#[derive(Default)]
struct DocumentCache {
    instances: FrameInstances,
    revision: u64,
    frame_update: Option<FrameUpdate>,
    lighting_revision: u64,
    lighting_update: Option<render::LightingUpdate>,
    preview: Option<BlockPreviewCache>,
    bake: Option<editor::bake::Bake>,
    always_highlights: Vec<PrefabInstanceId>,
}

pub struct Session {
    pub state: EditorState,
    pub textures: TextureCatalog,
    pub options: FrameOptions,
    pub diagnostics: editor::environment::LoadDiagnostics,
    caches: HashMap<DocumentId, DocumentCache>,
    next_revision: u64,
    next_preview_revision: u64,
    type_visibility: TypeVisibility,
    type_thumbnails: HashMap<TypeId, Option<PrefabThumbnail>>,
    texture_revision: u64,
    maps: Vec<PathBuf>,
    baker: Baker,
    queued_bakes: Vec<DocumentId>,
    standalone_baker: editor::bake::Standalone,
    standalone: Vec<(Prefab, Option<visual::Appearance>)>,
    ui_fault: Option<vm::FaultKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FillOutcome {
    Applied,
    NoChange,
    TooLarge { limit: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LevelChange {
    Changed,
    NewLevelRequested,
    Unchanged,
}

impl Session {
    pub fn new() -> Self {
        Self {
            state: EditorState::new(),
            textures: TextureCatalog::default(),
            options: FrameOptions::default(),
            diagnostics: editor::environment::LoadDiagnostics::default(),
            caches: HashMap::new(),
            next_revision: 1,
            next_preview_revision: 1,
            type_visibility: TypeVisibility::default(),
            type_thumbnails: HashMap::new(),
            texture_revision: 0,
            maps: Vec::new(),
            baker: Baker::default(),
            queued_bakes: Vec::new(),
            standalone_baker: editor::bake::Standalone::default(),
            standalone: Vec::new(),
            ui_fault: None,
        }
    }

    pub fn apply_codebase(&mut self, loaded: LoadedCodebase) -> LoadReport {
        let LoadedCodebase {
            environment,
            diagnostics,
            textures,
            thumbnails,
            maps,
        } = loaded;

        let report = report(&environment, &diagnostics);
        self.diagnostics = diagnostics;
        self.textures = textures;
        self.type_thumbnails = thumbnails;
        self.standalone.clear();
        self.standalone_baker = editor::bake::Standalone::default();
        self.texture_revision = self.texture_revision.wrapping_add(1);
        self.type_visibility = TypeVisibility::default();
        self.maps = maps;
        self.state.environment = Some(Arc::new(environment));
        self.rebake_all();

        report
    }

    pub fn apply_map(&mut self, loaded: LoadedMap) {
        let LoadedMap { path, map, z, errors } = loaded;

        for error in &errors {
            log::error!("{}: {error}", path.display());
        }

        if let Some(id) = self.state.document_for_path(&path) {
            self.state.set_active(id);

            return;
        }

        self.activate_document(MapDocument::open(path, map, z));
    }

    pub fn close_map(&mut self, id: DocumentId) -> bool {
        let closed = self.state.close_document(id).is_some();
        if closed {
            self.caches.remove(&id);
        }

        closed
    }

    pub fn create_map(&mut self, path: &Path, size: Size, format: MapFormat) -> Result<(), Box<dyn std::error::Error>> {
        if size.x == 0
            || size.y == 0
            || size.z == 0
            || size.x > MAX_MAP_DIMENSION
            || size.y > MAX_MAP_DIMENSION
            || size.z > MAX_MAP_DIMENSION
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("map dimensions must each be between 1 and {MAX_MAP_DIMENSION}"),
            )
            .into());
        }
        if !path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("dmm"))
        {
            return Err(
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "map path must use the .dmm extension").into(),
            );
        }
        if path.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("{} already exists", path.display()),
            )
            .into());
        }
        if !path.parent().is_some_and(Path::is_dir) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("parent directory for {} does not exist", path.display()),
            )
            .into());
        }

        let tree = self
            .tree()
            .ok_or_else(|| std::io::Error::other("no codebase is loaded"))?;
        let (turf, area) = default_tile_paths(tree)
            .ok_or_else(|| std::io::Error::other("the codebase does not define usable /turf and /area types"))?;
        let mut map = Map::new(size);
        map.format = format;
        let key = map.intern_tile(vec![Prefab::new(turf), Prefab::new(area)]);
        for cell in map.grid.iter_mut().flatten().flatten() {
            *cell = key;
        }

        self.activate_document(MapDocument::create(path, map, 1));

        Ok(())
    }

    pub fn first_map(&self) -> Option<PathBuf> {
        let environment = self.state.environment.as_ref()?;

        environment
            .maps
            .iter()
            .find(|path| path.is_file())
            .or_else(|| environment.maps.first())
            .cloned()
    }

    pub fn tree(&self) -> Option<&ObjectTree> { self.state.environment.as_ref().map(|environment| &environment.tree) }

    pub(crate) fn type_source(&self, id: TypeId) -> Option<SourceLocation> {
        let environment = self.state.environment.as_ref()?;
        let location = environment.tree.get(id)?.location;
        if location.begin.line == 0 || location.begin.col == 0 {
            return None;
        }

        Some(SourceLocation {
            path: environment.file(location.file)?.to_path_buf(),
            line: location.begin.line,
            column: location.begin.col,
        })
    }

    pub fn map(&self) -> Option<&Map> { self.state.active_document().map(|document| &document.map) }

    pub fn codebase_name(&self) -> Option<&str> {
        let tree = self.tree()?;
        let world = tree.id_of(&TreePath::parse("/world"))?;

        tree.var_inherited(world, &Identifier::from("name"))?
            .value
            .as_text()
            .filter(|name| !name.is_empty())
    }

    pub fn environment_path(&self) -> Option<&Path> {
        self.state
            .environment
            .as_ref()
            .map(|environment| environment.root.as_path())
    }

    pub fn codebase_dir(&self) -> Option<&Path> { self.state.environment.as_deref().map(Environment::base_dir) }

    pub fn maps(&self) -> &[PathBuf] { &self.maps }

    pub fn map_path(&self) -> Option<&Path> {
        self.state
            .active_document()
            .and_then(|document| document.path.as_deref())
    }

    pub fn map_format(&self) -> Option<MapFormat> { self.map().map(|map| map.format) }

    pub fn undo_label(&self) -> Option<&str> { self.state.active_document()?.undo_label() }

    pub fn redo_label(&self) -> Option<&str> { self.state.active_document()?.redo_label() }

    pub fn undo(&mut self) -> bool {
        let Some(affected) = self
            .state
            .active_document_mut()
            .and_then(MapDocument::undo_with_affected)
        else {
            return false;
        };

        self.update_instances(&affected);

        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(affected) = self
            .state
            .active_document_mut()
            .and_then(MapDocument::redo_with_affected)
        else {
            return false;
        };

        self.update_instances(&affected);

        true
    }

    pub fn can_save_map_in_place(&self) -> bool {
        self.state
            .active_document()
            .is_some_and(|document| document.path.is_some() && !document.needs_initial_save())
    }

    pub fn save_map(&mut self) -> std::io::Result<()> {
        let Some(id) = self.state.active() else {
            return Err(std::io::Error::other("no map is open"));
        };
        let levels = self.state.document(id).map_or(0, |document| document.map.size.z);
        let result = self
            .state
            .document_mut(id)
            .ok_or_else(|| std::io::Error::other("no map is open"))?
            .save();

        if result.is_ok()
            && self
                .state
                .document(id)
                .is_some_and(|document| document.map.size.z != levels)
        {
            self.rebake(id);
        }

        result
    }

    pub fn save_map_as(&mut self, path: &Path, format: MapFormat) -> std::io::Result<()> {
        let Some(id) = self.state.active() else {
            return Err(std::io::Error::other("no map is open"));
        };
        let levels = self.state.document(id).map_or(0, |document| document.map.size.z);
        let result = self
            .state
            .document_mut(id)
            .ok_or_else(|| std::io::Error::other("no map is open"))?
            .save_as(path, format);

        if result.is_ok()
            && self
                .state
                .document(id)
                .is_some_and(|document| document.map.size.z != levels)
        {
            self.rebake(id);
        }

        result
    }

    pub fn z(&self) -> u32 { self.state.active_document().map_or(1, |document| document.z) }

    pub fn level_count(&self) -> u32 {
        self.state
            .active_document()
            .map_or(1, |document| document.map.size.z.max(1))
    }

    pub fn set_level(&mut self, z: u32) {
        let Some(document) = self.state.active_document_mut() else {
            return;
        };

        if z == document.z || !(1..=document.map.size.z.max(1)).contains(&z) {
            return;
        }

        document.z = z;
        document.set_focus(None);
        document.selection = None;
    }

    pub fn can_change_level(&self, delta: i32) -> bool {
        let Some(document) = self.state.active_document() else {
            return false;
        };

        if delta < 0 {
            return document.z > 1;
        }
        if delta == 0 {
            return false;
        }

        let levels = document.map.size.z.max(1);
        document.z < levels || (levels < MAX_MAP_DIMENSION && self.tree().is_some())
    }

    pub fn change_level(&mut self, delta: i32) -> LevelChange {
        if delta == 0 {
            return LevelChange::Unchanged;
        }

        let Some(id) = self.state.active() else {
            return LevelChange::Unchanged;
        };
        let Some(document) = self.state.document(id) else {
            return LevelChange::Unchanged;
        };

        let current = document.z;
        let levels = document.map.size.z.max(1);
        let target = if delta < 0 {
            current.saturating_sub(delta.unsigned_abs()).max(1)
        } else {
            current.saturating_add(delta as u32)
        };
        if target > levels && levels < MAX_MAP_DIMENSION {
            return LevelChange::NewLevelRequested;
        }

        let Some(document) = self.state.document_mut(id) else {
            return LevelChange::Unchanged;
        };
        let next = target.min(levels);

        if next != document.z {
            document.z = next;
            document.set_focus(None);
            document.selection = None;

            LevelChange::Changed
        } else {
            LevelChange::Unchanged
        }
    }

    pub fn create_level(&mut self, id: DocumentId, type_path: &str) -> Result<u32, String> {
        let type_path = type_path.trim();
        if type_path.is_empty() {
            return Err(String::from("enter a type path"));
        }

        let requested = TreePath::parse(type_path);
        let path = {
            let tree = self.tree().ok_or_else(|| String::from("no codebase is loaded"))?;
            let type_id = tree
                .id_of(&requested)
                .ok_or_else(|| format!("unknown type path: {type_path}"))?;
            if !is_placeable(tree, type_id) {
                return Err(format!("type path is not an atom: {type_path}"));
            }

            tree.get(type_id)
                .map(|declaration| TreePath::parse(&declaration.path.to_string()))
                .ok_or_else(|| format!("unknown type path: {type_path}"))?
        };

        let document = self
            .state
            .document_mut(id)
            .ok_or_else(|| String::from("the map is no longer open"))?;
        let levels = document.map.size.z.max(1);
        if document.z != levels {
            return Err(String::from("the map is no longer on its highest Z level"));
        }
        if levels >= MAX_MAP_DIMENSION {
            return Err(format!("maps cannot exceed {MAX_MAP_DIMENSION} Z levels"));
        }

        let z = document
            .append_level(&[Prefab::new(path)])
            .ok_or_else(|| String::from("could not allocate another Z level"))?;
        document.z = z;
        document.set_focus(None);
        document.selection = None;
        self.state.set_active(id);
        self.rebake(id);

        Ok(z)
    }

    pub fn selected_instance(&self) -> Option<PrefabInstanceId> {
        self.state.active_document().and_then(MapDocument::selected_instance)
    }

    pub fn tool(&self) -> Tool { self.state.tool }

    pub fn set_tool(&mut self, tool: Tool) {
        self.state.tool = tool;
        if tool == Tool::BlockSelect
            && let Some(document) = self.state.active_document_mut()
        {
            document.select_instance(None);
        }
    }

    pub fn selection(&self) -> Option<Selection> {
        let document = self.state.active_document()?;

        document.selection.filter(|selection| selection.min.z == document.z)
    }

    pub fn selection_mask(&self) -> Option<SelectionMask> { self.state.active_document()?.selection_mask() }

    pub fn selection_mode(&self) -> BlockSelectionMode {
        self.selection_mask().map_or(BlockSelectionMode::Full, |mask| mask.mode)
    }

    pub(crate) fn edit_revision(&self) -> Option<u64> { Some(self.caches.get(&self.state.active()?)?.revision) }

    pub fn can_select_block(&self, mask: SelectionMask) -> bool {
        self.state.active_document().is_some_and(|document| {
            mask.bounds.is_well_formed()
                && mask.bounds.min.z == document.z
                && mask.bounds.max.x <= document.map.size.x
                && mask.bounds.max.y <= document.map.size.y
                && mask.mode.tiles(mask.bounds).all(|coord| document.allows_edit_at(coord))
        })
    }

    pub fn try_select_block(&mut self, mask: SelectionMask) -> bool {
        self.can_select_block(mask) && self.select_block_with_mode(Some(mask.bounds), mask.mode)
    }

    pub fn select_block(&mut self, selection: Option<Selection>) -> bool {
        self.select_block_with_mode(selection, BlockSelectionMode::Full)
    }

    pub fn select_block_with_mode(&mut self, selection: Option<Selection>, mode: BlockSelectionMode) -> bool {
        let Some(document) = self.state.active_document_mut() else {
            return false;
        };

        let selection = selection.filter(|selection| {
            selection.is_well_formed()
                && selection.min.z == document.z
                && selection.max.x <= document.map.size.x
                && selection.max.y <= document.map.size.y
                && mode.tiles(*selection).all(|coord| document.allows_edit_at(coord))
        });
        document.selection = selection;
        document.selection_mode = mode;
        document.select_instance(None);

        selection.is_some()
    }

    pub fn place_selected_block_with_mode(
        &mut self, target_min: Coord, rotation: SelectionRotation, placement: SelectionPlacement,
        mode: BlockSelectionMode,
    ) -> bool {
        if !self.can_place_selected_block_with_mode(target_min, rotation, placement, mode) {
            return false;
        }

        let built = {
            let Some((environment, document)) = self.state.active_pair_mut() else {
                return false;
            };
            let Some(selection) = document.selection else {
                return false;
            };

            build_selection_placement(
                document,
                &environment.tree,
                selection,
                target_min,
                rotation,
                placement,
                mode,
            )
        };

        let Some((action, selection)) = built else {
            return false;
        };

        if !self.commit(action, None) {
            return false;
        }

        if let Some(document) = self.state.active_document_mut() {
            document.selection = Some(selection);
            document.selection_mode = mode;
            document.select_instance(None);
        }

        true
    }

    pub fn can_place_selected_block_with_mode(
        &self, target_min: Coord, rotation: SelectionRotation, _placement: SelectionPlacement, mode: BlockSelectionMode,
    ) -> bool {
        if self.tool() != Tool::BlockSelect {
            return false;
        }

        let Some(environment) = self.state.environment.as_ref() else {
            return false;
        };

        let roots = environment.tree.roots();
        if roots.turf.is_none() || roots.area.is_none() {
            return false;
        }

        let Some(document) = self.state.active_document() else {
            return false;
        };

        let Some(selection) = self.selection() else {
            return false;
        };

        let Some(target) = rotated_selection_at(selection, target_min, rotation) else {
            return false;
        };

        if target == selection && rotation == SelectionRotation::Original {
            // why do you want to select and place? get help
            return false;
        }

        target.max.x <= document.map.size.x
            && target.max.y <= document.map.size.y
            && mode
                .tiles(selection)
                .chain(mode.tiles(target))
                .all(|coord| document.allows_edit_at(coord))
    }

    pub fn fill_selected_block(
        &mut self, target_min: Coord, rotation: SelectionRotation, mode: BlockSelectionMode,
    ) -> bool {
        if !self.can_fill_selected_block(target_min, rotation, mode) {
            return false;
        }

        let Some(prefab) = self.state.palette.clone() else {
            return false;
        };
        let built = {
            let Some((environment, document)) = self.state.active_pair_mut() else {
                return false;
            };
            let Some(selection) = document.selection else {
                return false;
            };
            let Some(target) = rotated_selection_at(selection, target_min, rotation) else {
                return false;
            };

            build_selection_fill(document, &environment.tree, target, &prefab, mode).map(|action| (action, target))
        };

        let Some((action, target)) = built else {
            return false;
        };

        if !self.commit(action, None) {
            return false;
        }

        if let Some(document) = self.state.active_document_mut() {
            document.selection = Some(target);
            document.selection_mode = mode;
            document.select_instance(None);
        }
        self.state.choose_prefab(prefab);

        true
    }

    pub fn can_fill_selected_block(
        &self, target_min: Coord, rotation: SelectionRotation, mode: BlockSelectionMode,
    ) -> bool {
        if self.tool() != Tool::BlockSelect {
            return false;
        }

        let Some(environment) = self.state.environment.as_ref() else {
            return false;
        };
        let Some(prefab) = self.state.palette.as_ref() else {
            return false;
        };
        let Some(prefab_id) = environment.tree.id_of(&prefab.path) else {
            return false;
        };

        if !is_placeable(&environment.tree, prefab_id) {
            return false;
        }

        let Some(document) = self.state.active_document() else {
            return false;
        };
        let Some(selection) = self.selection() else {
            return false;
        };
        let Some(target) = rotated_selection_at(selection, target_min, rotation) else {
            return false;
        };

        target.is_well_formed()
            && target.max.x <= document.map.size.x
            && target.max.y <= document.map.size.y
            && mode.tiles(target).all(|coord| document.allows_edit_at(coord))
    }

    pub fn transform_selected_block_with_mode(
        &mut self, transform: SelectionTransform, mode: BlockSelectionMode,
    ) -> bool {
        if self.tool() != Tool::BlockSelect || !self.can_transform_selected_block_with_mode(transform, mode) {
            return false;
        }

        let built = {
            let Some((environment, document)) = self.state.active_pair_mut() else {
                return false;
            };
            let Some(selection) = document.selection else {
                return false;
            };

            build_selection_transform(document, &environment.tree, selection, transform, mode)
        };

        let Some((action, selection)) = built else {
            return false;
        };

        if !self.commit(action, None) {
            return false;
        }

        if let Some(document) = self.state.active_document_mut() {
            document.selection = Some(selection);
            document.selection_mode = mode;
            document.select_instance(None);
        }

        true
    }

    pub fn can_transform_selected_block_with_mode(
        &self, transform: SelectionTransform, mode: BlockSelectionMode,
    ) -> bool {
        // holy fuck we need a better solution to this, just copy pasting  same shit over and over

        if self.tool() != Tool::BlockSelect {
            return false;
        }

        let Some(environment) = self.state.environment.as_ref() else {
            return false;
        };

        let roots = environment.tree.roots();
        if roots.turf.is_none() || roots.area.is_none() {
            return false;
        }

        let Some(document) = self.state.active_document() else {
            return false;
        };

        let Some(selection) = self.selection() else {
            return false;
        };

        let Some(target) = transformed_selection(selection, transform) else {
            return false;
        };

        target.max.x <= document.map.size.x
            && target.max.y <= document.map.size.y
            && mode
                .tiles(selection)
                .chain(mode.tiles(target))
                .all(|coord| document.allows_edit_at(coord))
    }

    pub fn copy_selection(&mut self, mode: BlockSelectionMode) -> bool {
        let Some(selection) = self.selection() else {
            return false;
        };
        let Some(document) = self.state.active_document() else {
            return false;
        };
        let Some(block) = clipboard::copy_block(document, selection, mode) else {
            return false;
        };
        self.state.set_clipboard(block);

        true
    }

    pub fn clipboard(&self) -> Option<&TileBlock> { self.state.clipboard() }

    pub fn clipboard_footprint(&self, min: Coord, rotation: SelectionRotation) -> Option<Selection> {
        self.state.clipboard()?.footprint(min, rotation)
    }

    pub fn clipboard_selection_mask(&self, min: Coord, rotation: SelectionRotation) -> Option<SelectionMask> {
        let block = self.state.clipboard()?;
        Some(SelectionMask {
            bounds: self.clipboard_footprint(min, rotation)?,
            mode: block.selection_mode(),
        })
    }

    pub fn can_paste_clipboard(&self, min: Coord, rotation: SelectionRotation) -> bool {
        let Some(block) = self.state.clipboard() else {
            return false;
        };
        let Some(document) = self.state.active_document() else {
            return false;
        };

        clipboard::can_paste(document, block, min, rotation)
    }

    pub fn paste_clipboard(&mut self, min: Coord, rotation: SelectionRotation) -> bool {
        let built = {
            let Some((environment, document, block)) = self.state.active_paste_mut() else {
                return false;
            };

            clipboard::paste_block(document, &environment.tree, block, min, rotation)
                .map(|built| (built, block.selection_mode()))
        };

        let Some(((action, target), mode)) = built else {
            return false;
        };

        if !self.commit(action, None) {
            return false;
        }

        if let Some(document) = self.state.active_document_mut() {
            document.selection = Some(target);
            document.selection_mode = mode;
            document.select_instance(None);
        }

        true
    }

    pub(crate) fn hide_block_preview(&mut self, id: DocumentId) {
        if let Some(preview) = self.caches.get_mut(&id).and_then(|cache| cache.preview.as_mut()) {
            preview.visible = false;
        }
    }

    pub(crate) fn prepare_block_preview(
        &mut self, id: DocumentId, source: BlockPreviewSource, destination: Coord, rotation: SelectionRotation,
    ) {
        let Some(cache) = self.caches.get(&id) else {
            return;
        };

        if self.state.active() != Some(id) {
            return;
        }
        let key = BlockPreviewKey {
            source,
            rotation,
            document_revision: cache.revision,
            clipboard_revision: if source == BlockPreviewSource::Clipboard {
                self.state.clipboard_revision()
            } else {
                0
            },
            texture_revision: self.texture_revision,
            tile_size: self.options.tile_size,
            show_areas: self.options.show_areas,
        };

        if cache.preview.as_ref().is_none_or(|preview| preview.key != key) {
            let origin = Coord::new(1, 1, 1);
            let sprites = match source {
                BlockPreviewSource::Selection(mask) => rotated_selection_at(mask.bounds, origin, rotation)
                    .map_or_else(Vec::new, |target| {
                        self.block_preview_sprites(mask.bounds, target, rotation, mask.mode)
                    }),
                BlockPreviewSource::Clipboard => self
                    .clipboard_footprint(origin, rotation)
                    .map_or_else(Vec::new, |target| self.clipboard_preview_sprites(target, rotation)),
            };
            let revision = self.next_preview_revision;
            self.next_preview_revision = revision.wrapping_add(1).max(1);

            if let Some(cache) = self.caches.get_mut(&id) {
                cache.preview = Some(BlockPreviewCache {
                    key,
                    sprites,
                    revision,
                    destination,
                    visible: true,
                });
            }
        } else if let Some(preview) = self.caches.get_mut(&id).and_then(|cache| cache.preview.as_mut()) {
            preview.destination = destination;
            preview.visible = true;
        }
    }

    pub(crate) fn block_preview_sprites(
        &self, source: Selection, target: Selection, rotation: SelectionRotation, mode: BlockSelectionMode,
    ) -> Vec<SpriteInstance> {
        let Some(document) = self.state.active_document() else {
            return Vec::new();
        };

        let Some(environment) = self.state.environment.as_ref() else {
            return Vec::new();
        };

        let mut previews = Vec::new();
        for coord in mode.tiles(source) {
            let relative = (coord.x - source.min.x, coord.y - source.min.y);
            let transformed = rotate_point(relative, source.width(), source.height(), rotation);
            let destination = Coord::new(target.min.x + transformed.0, target.min.y + transformed.1, target.min.z);
            let Some(tile) = document.placed_tile(coord) else {
                continue;
            };
            for placed in tile {
                previews.extend(self.preview_sprite(environment, placed.id(), placed.prefab(), destination, rotation));
            }
        }

        previews.sort_by(|left, right| left.depth.total_cmp(&right.depth));

        previews
    }

    pub(crate) fn clipboard_preview_sprites(
        &self, target: Selection, rotation: SelectionRotation,
    ) -> Vec<SpriteInstance> {
        let Some(environment) = self.state.environment.as_ref() else {
            return Vec::new();
        };
        let Some(block) = self.state.clipboard() else {
            return Vec::new();
        };

        let mut previews = Vec::new();
        for (destination, tile) in block.destinations(target, rotation) {
            for prefab in tile {
                previews.extend(self.preview_sprite(environment, PREVIEW_OWNER, prefab, destination, rotation));
            }
        }

        previews.sort_by(|left, right| left.depth.total_cmp(&right.depth));

        previews
    }

    fn preview_sprite(
        &self, environment: &Environment, owner: PrefabInstanceId, prefab: &Prefab, destination: Coord,
        rotation: SelectionRotation,
    ) -> Option<SpriteInstance> {
        let mut prefab = prefab.clone();
        rotate_prefab(&environment.tree, &mut prefab, rotation);

        let id = environment.tree.id_of(&prefab.path)?;
        if !self.type_visibility.is_visible(id) {
            return None;
        }

        let is_area = environment
            .tree
            .roots()
            .area
            .is_some_and(|area| environment.tree.is_subtype_of(id, area));
        if is_area && !self.options.show_areas {
            return None;
        }

        let appearance = visual::resolve_id(&environment.tree, id, &prefab);
        let texture = frame::sprite_texture(&environment.icons, &self.textures, &appearance)?;
        let sprite = frame::instance_for(
            owner,
            &appearance,
            texture,
            destination,
            self.options.tile_size,
            is_area,
        );

        Some(SpriteInstance {
            color: sprite.color.map(|channel| channel * 0.55),
            ..sprite
        })
    }

    pub fn palette(&self) -> Option<&Prefab> { self.state.palette.as_ref() }

    pub fn recent_prefabs(&self) -> &[Prefab] { self.state.recent_prefabs() }

    pub(crate) fn prefab_appearance(&mut self, prefab: &Prefab) -> Option<visual::Appearance> {
        let environment = self.state.environment.clone()?;
        let appearance = visual::resolve(&environment.tree, prefab);
        if frame::sprite_texture(&environment.icons, &self.textures, &appearance).is_some() {
            return Some(appearance);
        }

        if let Some((_, cached)) = self.standalone.iter().find(|(cached, _)| cached == prefab) {
            return cached.clone().or(Some(appearance));
        }

        let derived = environment.tree.id_of(&prefab.path).and_then(|id| {
            let delta = self.standalone_baker.appearance(&environment, prefab)?;

            Some(visual::resolve_delta(&environment.tree, id, prefab, &delta))
        });
        if self.standalone.len() >= STANDALONE_CACHE_LIMIT {
            self.standalone.clear();
        }
        self.standalone.push((prefab.clone(), derived.clone()));

        derived.or(Some(appearance))
    }

    pub(crate) fn prefab_thumbnail(&mut self, prefab: &Prefab) -> Option<PrefabThumbnail> {
        let appearance = self.prefab_appearance(prefab)?;
        let environment = self.state.environment.as_ref()?;

        self.prefab_thumbnail_for(environment, &appearance)
    }

    pub(crate) fn type_thumbnail(&self, id: TypeId) -> Option<PrefabThumbnail> {
        self.type_thumbnails.get(&id).copied().flatten()
    }

    pub(crate) fn placement_preview(&mut self) -> Option<PlacementPreview> {
        if self.tool() != Tool::Place {
            return None;
        }

        let prefab = self.palette()?.clone();
        let appearance = self.prefab_appearance(&prefab)?;
        let environment = self.state.environment.as_ref()?;
        let thumbnail = self.prefab_thumbnail_for(environment, &appearance)?;
        let offset = [
            appearance
                .step_x
                .saturating_add(appearance.pixel_x)
                .saturating_add(appearance.pixel_w),
            appearance
                .step_y
                .saturating_add(appearance.pixel_y)
                .saturating_add(appearance.pixel_z),
        ];

        Some(PlacementPreview { thumbnail, offset })
    }

    fn prefab_thumbnail_for(
        &self, environment: &Environment, appearance: &visual::Appearance,
    ) -> Option<PrefabThumbnail> {
        prefab_thumbnail_for(&self.textures, environment, appearance)
    }

    pub fn choose_type(&mut self, selected: TypeId) -> bool {
        let prefab = self.state.environment.as_ref().and_then(|environment| {
            is_placeable(&environment.tree, selected)
                .then(|| environment.tree.get(selected))
                .flatten()
                .map(|declaration| Prefab::new(TreePath::parse(&declaration.path.to_string())))
        });
        let Some(prefab) = prefab else {
            return false;
        };

        self.state.choose_prefab(prefab);
        if !matches!(self.state.tool, Tool::Fill | Tool::BlockSelect) {
            self.state.tool = Tool::Place;
        }

        true
    }

    pub fn choose_recent(&mut self, index: usize) -> bool {
        if !self.state.choose_recent(index) {
            return false;
        }
        if !matches!(self.state.tool, Tool::Fill | Tool::BlockSelect) {
            self.state.tool = Tool::Place;
        }

        true
    }

    pub fn place_at(&mut self, coord: Coord, group: Option<EditGroupId>) -> Option<PrefabInstanceId> {
        if self.state.tool != Tool::Place || !self.can_edit_at(coord) {
            return None;
        }
        let prefab = self.state.palette.clone()?;
        let action = {
            let (environment, document) = self.state.active_pair_mut()?;

            Tool::Place.build_edit(&mut ToolContext {
                document,
                tree: &environment.tree,
                prefab: Some(&prefab),
                target: None,
                coord,
                anchor: None,
                fill_mode: FillMode::default(),
                custom_fill_boundaries: &[],
            })
        }?;
        let selected = action.selected?;
        if !self.commit(action, group) {
            return None;
        }

        if let Some(document) = self.state.active_document_mut() {
            document.select_instance(Some(selected));
        }
        self.state.choose_prefab(prefab);

        Some(selected)
    }

    pub fn fill_at(&mut self, coord: Coord, fill_mode: FillMode, custom_fill_boundaries: &[TreePath]) -> FillOutcome {
        self.fill_at_with_limit(coord, fill_mode, custom_fill_boundaries, Some(MAX_FILL_TILES))
    }

    pub(crate) fn fill_at_unlimited(
        &mut self, coord: Coord, fill_mode: FillMode, custom_fill_boundaries: &[TreePath],
    ) -> FillOutcome {
        self.fill_at_with_limit(coord, fill_mode, custom_fill_boundaries, None)
    }

    fn fill_at_with_limit(
        &mut self, coord: Coord, fill_mode: FillMode, custom_fill_boundaries: &[TreePath], max_tiles: Option<usize>,
    ) -> FillOutcome {
        if self.state.tool != Tool::Fill || !self.can_edit_at(coord) {
            return FillOutcome::NoChange;
        }

        let Some(prefab) = self.state.palette.clone() else {
            return FillOutcome::NoChange;
        };

        let mask = self.selection_mask();

        let action = {
            let Some((environment, document)) = self.state.active_pair_mut() else {
                return FillOutcome::NoChange;
            };

            Tool::Fill.build_fill_edit_with_mask(
                &mut ToolContext {
                    document,
                    tree: &environment.tree,
                    prefab: Some(&prefab),
                    target: None,
                    coord,
                    anchor: None,
                    fill_mode,
                    custom_fill_boundaries,
                },
                max_tiles,
                mask,
            )
        };

        let action = match action {
            Ok(Some(action)) => action,
            Ok(None) => return FillOutcome::NoChange,
            Err(FillError::TooLarge { limit }) => return FillOutcome::TooLarge { limit },
        };

        if !self.commit(action, None) {
            return FillOutcome::NoChange;
        }

        self.state.choose_prefab(prefab);

        FillOutcome::Applied
    }

    pub fn delete_instance(&mut self, target: PrefabInstanceId) -> bool {
        if self.state.tool != Tool::Delete {
            return false;
        }

        self.build_delete(target)
            .is_some_and(|action| self.commit(action, None))
    }

    fn build_delete(&mut self, target: PrefabInstanceId) -> Option<ToolEdit> {
        let (environment, document) = self.state.active_pair_mut()?;
        let coord = Coord::new(1, 1, document.z);

        Tool::Delete.build_edit(&mut ToolContext {
            document,
            tree: &environment.tree,
            prefab: None,
            target: Some(target),
            coord,
            anchor: None,
            fill_mode: FillMode::default(),
            custom_fill_boundaries: &[],
        })
    }

    fn commit(&mut self, action: ToolEdit, group: Option<EditGroupId>) -> bool {
        let affected = action.affected;
        let applied = self
            .state
            .active_document_mut()
            .is_some_and(|document| document.apply_grouped(action.edit, group));
        if applied {
            self.update_instances(&affected);
        }

        applied
    }

    pub fn selected_prefab(&self) -> Option<&Prefab> {
        let document = self.state.active_document()?;
        let selected = document.selected_instance()?;

        document.prefab_instance(selected).map(|(prefab, _)| prefab)
    }

    pub fn selected_location(&self) -> Option<PrefabLocation> {
        let document = self.state.active_document()?;

        document.instance_location(document.selected_instance()?)
    }

    pub(crate) fn selected_directional_types(&self) -> Option<DirectionalTypes> {
        let environment = self.state.environment.as_ref()?;
        let document = self.state.active_document()?;
        let selected = document.selected_instance()?;
        let (prefab, _) = document.prefab_instance(selected)?;
        let id = environment.tree.id_of(&prefab.path)?;

        directional_types_for(&environment.tree, id)
    }

    pub(crate) fn placement_direction(&self) -> Option<DirectionState> {
        if self.tool() != Tool::Place {
            return None;
        }

        let environment = self.state.environment.as_ref()?;
        let prefab = self.palette()?;
        let id = environment.tree.id_of(&prefab.path)?;
        let appearance = visual::resolve_id(&environment.tree, id, prefab);

        Some(direction_state(environment, id, &appearance))
    }

    pub(crate) fn selected_transform(&self) -> Option<SelectedTransform> {
        let environment = self.state.environment.as_ref()?;
        let document = self.state.active_document()?;
        let selected = document.selected_instance()?;
        let (prefab, _) = document.prefab_instance(selected)?;
        let id = environment.tree.id_of(&prefab.path)?;
        let atom = environment.tree.roots().atom?;
        if !environment.tree.is_subtype_of(id, atom) {
            return None;
        }

        let appearance = visual::resolve_id(&environment.tree, id, prefab);
        let is_movable = environment
            .tree
            .roots()
            .movable
            .is_some_and(|movable| environment.tree.is_subtype_of(id, movable));
        let direction = direction_state(environment, id, &appearance);

        Some(SelectedTransform {
            selected,
            sprite: *self.instances()?.sprite(selected)?,
            pixel: [appearance.pixel_x, appearance.pixel_y],
            step: [appearance.step_x, appearance.step_y],
            is_movable,
            dir: direction.dir,
            dmi_directions: direction.dmi_directions,
            directional_types: direction.directional_types,
        })
    }

    pub(crate) fn selected_offset_guide(&self) -> Option<GuideLine> {
        let environment = self.state.environment.as_ref()?;
        let document = self.state.active_document()?;
        let selected = document.selected_instance()?;
        let (prefab, location) = document.prefab_instance(selected)?;
        if location.coord.z != document.z {
            return None;
        }

        let id = environment.tree.id_of(&prefab.path)?;
        let appearance = visual::resolve_id(&environment.tree, id, prefab);
        let displacement = [
            appearance
                .step_x
                .saturating_add(appearance.pixel_x)
                .saturating_add(appearance.pixel_w),
            appearance
                .step_y
                .saturating_add(appearance.pixel_y)
                .saturating_add(appearance.pixel_z),
        ];
        if displacement == [0, 0] {
            return None;
        }

        let sprite = self.instances()?.sprite(selected)?;
        let tile_size = self.options.tile_size.max(1) as f32;

        Some(GuideLine {
            origin: [
                (location.coord.x as f32 - 0.5) * tile_size,
                (location.coord.y as f32 - 0.5) * tile_size,
            ],
            target: [sprite.x + sprite.width * 0.5, sprite.y + sprite.height * 0.5],
        })
    }

    pub(crate) fn selected_guides(&self) -> SelectionGuides {
        let mut guides = SelectionGuides::default();
        if let Some(offset) = self.selected_offset_guide() {
            guides.lines.push(offset);
        }

        let Some(document_id) = self.state.active() else {
            return guides;
        };
        let Some(document) = self.state.document(document_id) else {
            return guides;
        };
        let Some(selected) = document.selected_instance() else {
            return guides;
        };
        let Some((_, selected_location)) = document.prefab_instance(selected) else {
            return guides;
        };

        if selected_location.coord.z != document.z {
            return guides;
        }

        let Some(cache) = self.caches.get(&document_id) else {
            return guides;
        };

        let Some(bake) = cache.bake.as_ref() else {
            return guides;
        };

        let tile_size = self.options.tile_size.max(1) as f32;
        let center = |id: PrefabInstanceId, coord: Coord| {
            cache.instances.sprite(id).map_or_else(
                || [(coord.x as f32 - 0.5) * tile_size, (coord.y as f32 - 0.5) * tile_size],
                |sprite| [sprite.x + sprite.width * 0.5, sprite.y + sprite.height * 0.5],
            )
        };

        let selected_center = center(selected, selected_location.coord);
        let mut badged = HashSet::new();

        for connected in bake.connections(selected.get()) {
            let Some(connected) = PrefabInstanceId::from_raw(connected) else {
                continue;
            };
            let Some((_, location)) = document.prefab_instance(connected) else {
                continue;
            };
            let target = center(connected, location.coord);
            guides.lines.push(GuideLine {
                origin: selected_center,
                target,
            });
            guides.connected.push(connected);

            if location.coord.z != document.z && badged.insert((location.coord.x, location.coord.y, location.coord.z)) {
                guides.badges.push(GuideBadge {
                    position: target,
                    z: location.coord.z,
                });
            }
        }

        guides
    }

    pub(crate) fn highlights(&self, id: DocumentId, hovered: Option<Coord>) -> Vec<&editor::bake::Highlight> {
        let Some(document) = self.state.document(id) else {
            return Vec::new();
        };
        let Some(cache) = self.caches.get(&id) else {
            return Vec::new();
        };
        let Some(bake) = cache.bake.as_ref() else {
            return Vec::new();
        };

        let z = document.z as i32;
        let mut seen = HashSet::new();
        let mut shown = Vec::new();
        let mut sources = cache
            .always_highlights
            .iter()
            .map(|owner| (*owner, editor::bake::HIGHLIGHT_ALWAYS))
            .collect::<Vec<_>>();
        if let Some(selected) = document.selected_instance() {
            sources.push((selected, editor::bake::HIGHLIGHT_SELECTED));
        }

        if let Some(hovered) = hovered {
            sources.extend(
                document
                    .instance_ids_at(hovered)
                    .iter()
                    .map(|owner| (*owner, editor::bake::HIGHLIGHT_HOVERED)),
            );
        }

        for (owner, when) in sources {
            for (index, highlight) in bake.highlights(owner.get()).iter().enumerate() {
                if highlight.z == z && highlight.shown_when(when) && seen.insert((owner, index)) {
                    shown.push(highlight);
                }
            }
        }

        shown
    }

    pub(crate) fn dm_ui(&mut self, dockspace: u32, feedback: editor::bake::UiFeedback) -> editor::bake::UiFrame {
        let Some(id) = self.state.active() else {
            return editor::bake::UiFrame::default();
        };
        let Some(program) = self
            .state
            .environment
            .as_ref()
            .and_then(|environment| environment.bake_program.as_ref())
        else {
            return editor::bake::UiFrame::default();
        };
        let target = self
            .state
            .document(id)
            .and_then(|document| document.selected_instance())
            .map(PrefabInstanceId::get);
        let Some(bake) = self.caches.get_mut(&id).and_then(|cache| cache.bake.as_mut()) else {
            return editor::bake::UiFrame::default();
        };

        let drawn = bake.ui(&program.tree, &program.module, target, dockspace, feedback);
        report_bake_output(bake);

        match drawn {
            Ok(frame) => {
                self.ui_fault = None;

                frame
            },
            Err(fault) => {
                if self.ui_fault.as_ref() != Some(&fault.kind) {
                    log::warn!("demir_ui: {fault}");
                    self.ui_fault = Some(fault.kind);
                }

                editor::bake::UiFrame::default()
            },
        }
    }

    pub(crate) fn dm_ui_rebake(&mut self, request: editor::bake::UiRebake) {
        let Some(id) = self.state.active() else {
            return;
        };
        let Some(program) = self
            .state
            .environment
            .as_ref()
            .and_then(|environment| environment.bake_program.as_ref())
        else {
            return;
        };
        let Some(bake) = self.caches.get_mut(&id).and_then(|cache| cache.bake.as_mut()) else {
            return;
        };

        let update = bake.rebake(&program.tree, &program.module, request);
        report_bake_output(bake);

        self.apply_bake_update(
            id,
            editor::bake::BakeUpdate {
                appearances: update
                    .appearances
                    .into_iter()
                    .filter_map(PrefabInstanceId::from_raw)
                    .collect(),
                lighting: update.lighting,
            },
        );
    }

    pub fn icon_metadata(&self, name: &str) -> Option<&dmi::metadata::Metadata> {
        self.state.environment.as_ref()?.icon(name)
    }

    pub fn select_instance(&mut self, selected: Option<PrefabInstanceId>) {
        let prefab = self.state.active_document().and_then(|document| {
            let selected = selected?;

            document.prefab_instance(selected).map(|(prefab, _)| prefab.clone())
        });
        if let Some(document) = self.state.active_document_mut() {
            document.select_instance(selected);
        }
        if let Some(prefab) = prefab {
            self.state.choose_prefab(prefab);
        }
    }

    pub fn set_selected_instance_var(&mut self, name: Identifier, value: Value) -> Option<bool> {
        let label = format!("set {name}");

        self.edit_selected_instance_vars(label, &[VarMutation::Set(name, value)], None)
    }

    pub fn edit_selected_instance_vars(
        &mut self, label: impl Into<String>, mutations: &[VarMutation], group: Option<EditGroupId>,
    ) -> Option<bool> {
        let selected = self.selected_instance()?;
        let document = self.state.active_document_mut()?;

        let changed = document.edit_instance_vars(selected, label, mutations, group)?;

        if changed {
            self.update_instance(selected);
        }

        Some(changed)
    }

    pub(crate) fn set_selected_directional_type(&mut self, direction: Dir, group: Option<EditGroupId>) -> Option<bool> {
        let selected = self.selected_instance()?;
        let path = {
            let environment = self.state.environment.as_ref()?;
            let document = self.state.active_document()?;
            let (prefab, _) = document.prefab_instance(selected)?;
            let id = environment.tree.id_of(&prefab.path)?;

            directional_type_target(&environment.tree, id, direction)?
        };
        let document = self.state.active_document_mut()?;
        let changed = document.replace_instance_path(
            selected,
            "set direction",
            path,
            &[VarMutation::Remove(Identifier::from("dir"))],
            group,
        )?;

        if changed {
            self.update_instance(selected);
        }

        Some(changed)
    }

    pub(crate) fn set_placement_direction(&mut self, direction: Dir) -> Option<bool> {
        if self.tool() != Tool::Place {
            return None;
        }

        let mut prefab = self.palette()?.clone();
        let id = self.state.environment.as_ref()?.tree.id_of(&prefab.path)?;
        let directional = self
            .state
            .environment
            .as_ref()
            .and_then(|environment| directional_types_for(&environment.tree, id))
            .is_some();

        if directional {
            let environment = self.state.environment.as_ref()?;
            prefab.path = directional_type_target(&environment.tree, id, direction)?;
            prefab.remove_var(&Identifier::from("dir"));
        } else {
            prefab.set_var(Identifier::from("dir"), Value::Num(direction.to_bits() as f32));
        }

        Some(self.state.replace_palette(prefab))
    }

    pub fn move_selected_instance(
        &mut self, to_coord: Coord, label: impl Into<String>, mutations: &[VarMutation], group: Option<EditGroupId>,
    ) -> Option<bool> {
        let selected = self.selected_instance()?;
        let document = self.state.active_document_mut()?;

        let changed = document.move_instance(selected, to_coord, label, mutations, group)?;

        if changed {
            self.update_instance(selected);
        }

        Some(changed)
    }

    pub fn area_at(&self, id: DocumentId, coord: Coord) -> Option<PrefabInstanceId> {
        let environment = self.state.environment.as_ref()?;
        let area = environment.tree.roots().area?;
        let document = self.state.document(id)?;
        let tile = document.map.tile_at(coord)?;

        tile.iter()
            .zip(document.instance_ids_at(coord))
            .find_map(|(prefab, owner)| {
                let id = environment.tree.id_of(&prefab.path)?;

                environment.tree.is_subtype_of(id, area).then_some(*owner)
            })
    }

    pub fn focused_area(&self) -> Option<PrefabInstanceId> { Some(self.state.active_document()?.focus()?.component()) }

    fn instances(&self) -> Option<&FrameInstances> {
        self.caches.get(&self.state.active()?).map(|cache| &cache.instances)
    }

    #[cfg(test)]
    fn active_cache(&self) -> &DocumentCache {
        self.state
            .active()
            .and_then(|id| self.caches.get(&id))
            .expect("a document is open")
    }

    #[cfg(test)]
    fn revision(&self) -> u64 { self.active_cache().revision }

    #[cfg(test)]
    fn frame_update(&self) -> Option<FrameUpdate> { self.active_cache().frame_update }

    pub fn toggle_focus_at(&mut self, coord: Option<Coord>) {
        if self.focused_area().is_some()
            && coord.is_none_or(|coord| {
                self.instances()
                    .and_then(|instances| instances.area_component_at(coord))
                    == self.focused_area()
            })
        {
            self.set_focus(None);

            return;
        }

        let focus = coord.and_then(|seed| self.resolve_focus(seed));
        self.set_focus(focus);
    }

    pub fn can_edit_at(&self, coord: Coord) -> bool {
        self.state
            .active_document()
            .is_none_or(|document| document.allows_edit_at(coord))
    }

    fn set_focus(&mut self, focus: Option<AreaFocus>) {
        if let Some(document) = self.state.active_document_mut() {
            document.set_focus(focus);
        }
    }

    fn resolve_focus(&self, seed: Coord) -> Option<AreaFocus> {
        let owner = self.area_at(self.state.active()?, seed)?;
        let prefab = self.state.active_document()?.prefab_instance(owner)?.0.clone();
        let instances = self.instances()?;
        let component = instances.area_component_at(seed)?;

        Some(AreaFocus::new(
            seed,
            prefab,
            component,
            instances.area_component_tiles(component),
        ))
    }

    fn revalidate_focus(&mut self) {
        let Some(seed) = self.state.active_document().and_then(|document| {
            let focus = document.focus()?;

            Some((focus.seed(), focus.prefab().clone()))
        }) else {
            return;
        };

        let resolved = self
            .resolve_focus(seed.0)
            .filter(|focus| frame::same_area(&seed.1, focus.prefab()));
        self.set_focus(resolved);
    }

    pub fn toggle_areas(&mut self) { self.options.show_areas = !self.options.show_areas; }

    pub fn toggle_area_outlines(&mut self) { self.options.show_area_outlines = !self.options.show_area_outlines; }

    pub fn toggle_lighting(&mut self) { self.options.show_lighting = !self.options.show_lighting; }

    fn collect_bake_diagnostics(&mut self) {
        self.diagnostics.bake = self
            .caches
            .values()
            .filter_map(|cache| cache.bake.as_ref())
            .flat_map(|bake| bake.diagnostics.entries.iter())
            .filter(|entry| entry.count > 0)
            .cloned()
            .collect();
    }

    pub fn is_type_visible(&self, id: TypeId) -> bool {
        self.tree().is_some_and(|tree| tree.get(id).is_some()) && self.type_visibility.is_visible(id)
    }

    pub fn toggle_type_visibility(&mut self, id: TypeId) -> bool {
        let visible = !self.type_visibility.is_visible(id);
        let changed = {
            let Some(environment) = self.state.environment.as_ref() else {
                return false;
            };
            if environment.tree.get(id).is_none() {
                return false;
            }

            self.type_visibility.set_subtree(&environment.tree, id, visible)
        };
        if changed {
            self.rebuild_all_instances();
        }

        changed
    }

    pub fn set_underlay_depth(&mut self, depth: u32) {
        if depth == self.options.underlay_depth {
            return;
        }

        self.options.underlay_depth = depth;
    }

    pub fn map_view_frame<'a>(
        &'a self, id: DocumentId, rect: MapViewRect, camera: render::Camera, interaction: MapViewInteraction,
        guide_lines: &'a [GuideLine], connected: &'a [PrefabInstanceId],
    ) -> Option<MapViewFrame<'a>> {
        let document = self.state.document(id)?;
        let cache = self.caches.get(&id)?;

        Some(MapViewFrame {
            rect,
            camera,
            sprite_instances: &cache.instances.sprites,
            area_tiles: &cache.instances.area_tiles,
            focused_area: document.focus().map(AreaFocus::component),
            active_z: document.z,
            level_count: document.map.size.z.max(1),
            revision: cache.revision,
            pending_update: cache.frame_update,
            lighting: (self.options.show_lighting && !cache.instances.light_tiles.is_empty()).then_some(
                render::LightingFrame {
                    size: cache.instances.lighting_size,
                    tiles: &cache.instances.light_tiles,
                    tile_size: self.options.tile_size,
                    revision: cache.lighting_revision,
                    pending_update: cache.lighting_update,
                },
            ),
            guide_lines,
            connected,
            interaction,
            preview: cache
                .preview
                .as_ref()
                .filter(|preview| {
                    preview.visible
                        && preview.destination.z == document.z
                        && preview.key.document_revision == cache.revision
                })
                .map(|preview| SpritePreview {
                    sprites: &preview.sprites,
                    revision: preview.revision,
                    offset: [
                        preview.destination.x.saturating_sub(1) as f32 * self.options.tile_size.max(1) as f32,
                        preview.destination.y.saturating_sub(1) as f32 * self.options.tile_size.max(1) as f32,
                    ],
                }),
        })
    }

    pub fn frame<'a>(&self, map_views: &'a [MapViewFrame<'a>], picking: Option<usize>) -> Frame<'a> {
        Frame {
            map_views,
            underlay_depth: self.options.underlay_depth,
            show_areas: self.options.show_areas,
            show_area_outlines: self.options.show_area_outlines,
            picking,
        }
    }

    pub fn texture_revision(&self) -> u64 { self.texture_revision }

    pub fn extent_px(&self) -> (f32, f32) {
        self.state
            .active()
            .map_or_else(|| self.tile_extent(None), |id| self.extent_px_of(id))
    }

    pub fn selected_instance_of(&self, id: DocumentId) -> Option<PrefabInstanceId> {
        self.state.document(id).and_then(MapDocument::selected_instance)
    }

    pub fn extent_px_of(&self, id: DocumentId) -> (f32, f32) {
        self.tile_extent(self.state.document(id).map(|document| document.map.size))
    }

    fn tile_extent(&self, size: Option<Size>) -> (f32, f32) {
        let tile = self.options.tile_size.max(1) as f32;

        size.map_or((tile, tile), |size| {
            (size.x.max(1) as f32 * tile, size.y.max(1) as f32 * tile)
        })
    }

    fn rebuild_all_instances(&mut self) {
        for id in self.state.document_ids() {
            self.rebuild_instances(id);
        }
    }

    fn rebake_all(&mut self) {
        for id in self.state.document_ids() {
            self.rebake(id);
        }
    }

    fn rebake(&mut self, id: DocumentId) {
        self.caches.entry(id).or_default().bake = None;
        self.collect_bake_diagnostics();
        self.rebuild_instances(id);

        if !self.queued_bakes.contains(&id) {
            self.queued_bakes.push(id);
        }

        self.start_next_bake();
    }

    fn start_next_bake(&mut self) {
        while !self.baker.is_busy() {
            let Some(id) = self.queued_bakes.first().copied() else {
                return;
            };
            self.queued_bakes.remove(0);

            let Some(environment) = self.state.environment.clone() else {
                return;
            };
            let Some(document) = self.state.document(id) else {
                continue;
            };
            if environment.bake_program.is_none() {
                return;
            }

            self.baker.start(baker::Request {
                document: id,
                path: document
                    .path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default(),
                atoms: editor::bake::atoms(&environment, document),
                size: editor::bake::size(document),
                environment,
            });
        }
    }

    pub fn poll_bake(&mut self) {
        if let Some((id, mut bake, outdated)) = self.baker.poll() {
            let open = self.state.document(id).is_some();
            if outdated && open {
                self.rebake(id);
            } else if open {
                if let Some(bake) = bake.as_mut() {
                    report_bake_output(bake);
                }

                let cache = self.caches.entry(id).or_default();
                cache.bake = bake;
                cache.always_highlights = always_highlighted(cache.bake.as_ref());
                self.collect_bake_diagnostics();
                self.rebuild_instances(id);
            }
        }

        self.start_next_bake();
    }

    pub fn bake_view(&self) -> Option<crate::loader::LoadView> { self.baker.view() }

    fn rebuild_instances(&mut self, id: DocumentId) {
        let bake = self.caches.get(&id).and_then(|cache| cache.bake.as_ref());
        let instances = match (self.state.environment.as_ref(), self.state.document(id)) {
            (Some(environment), Some(document)) => frame::build_with_options(
                &environment.tree,
                &environment.icons,
                &self.textures,
                document,
                FrameRenderOptions {
                    visibility: &self.type_visibility,
                    tile_size: self.options.tile_size,
                    appearances: editor::bake::appearances(bake),
                    lighting: bake.and_then(|bake| bake.lighting.as_ref()),
                },
            ),
            _ => FrameInstances::default(),
        };

        let revision = self.bump_revision();
        let cache = self.caches.entry(id).or_default();
        cache.instances = instances;
        cache.revision = revision;
        cache.frame_update = None;
        cache.lighting_revision = revision;
        cache.lighting_update = None;
        self.revalidate_focus();
    }

    fn bump_revision(&mut self) -> u64 {
        let revision = self.next_revision;
        self.next_revision = self.next_revision.wrapping_add(1).max(1);

        revision
    }

    fn activate_document(&mut self, document: MapDocument) -> DocumentId {
        let id = self.state.open_document(document);
        self.rebake(id);

        id
    }

    fn update_instance(&mut self, selected: PrefabInstanceId) { self.update_instances(&[selected]); }

    fn update_instances(&mut self, affected: &[PrefabInstanceId]) {
        let Some(id) = self.state.active() else {
            return;
        };

        let Self {
            state, caches, baker, ..
        } = self;
        let cache = caches.entry(id).or_default();
        let bake_update = match (cache.bake.as_mut(), state.environment.as_ref(), state.document(id)) {
            (Some(bake), Some(environment), Some(document)) => {
                let update = editor::bake::update(bake, environment, document, affected);
                report_bake_output(bake);

                update
            },
            _ => {
                baker.invalidate(id);

                editor::bake::BakeUpdate {
                    appearances: affected.to_vec(),
                    lighting: None,
                }
            },
        };

        self.apply_bake_update(id, bake_update);
    }

    fn apply_bake_update(&mut self, id: DocumentId, bake_update: editor::bake::BakeUpdate) {
        let Self {
            state,
            textures,
            caches,
            type_visibility,
            options,
            next_revision,
            ..
        } = self;
        let cache = caches.entry(id).or_default();
        cache.always_highlights = always_highlighted(cache.bake.as_ref());
        let affected = bake_update.appearances;
        let update = match (state.environment.as_ref(), state.document(id)) {
            (Some(environment), Some(document)) => frame::update_prefabs_with_options(
                &mut cache.instances,
                &environment.tree,
                &environment.icons,
                textures,
                document,
                &affected,
                FrameRenderOptions {
                    visibility: type_visibility,
                    tile_size: options.tile_size,
                    appearances: editor::bake::appearances(cache.bake.as_ref()),
                    lighting: cache.bake.as_ref().and_then(|bake| bake.lighting.as_ref()),
                },
            ),
            _ => PrefabUpdate::Unchanged,
        };

        if let Some(range) = bake_update.lighting {
            cache.instances.update_lighting(
                cache.bake.as_ref().and_then(|bake| bake.lighting.as_ref()),
                Some(range.clone()),
            );
            let previous_revision = cache.lighting_revision;
            cache.lighting_revision = *next_revision;
            *next_revision = next_revision.wrapping_add(1).max(1);
            cache.lighting_update = Some(render::LightingUpdate {
                previous_revision,
                tiles: render::UpdateRange {
                    start: range.start,
                    end: range.end,
                },
            });
        }

        match update {
            PrefabUpdate::Unchanged => {},
            PrefabUpdate::Buffers { sprites, area_tiles } => {
                let previous_revision = cache.revision;
                cache.revision = *next_revision;
                *next_revision = next_revision.wrapping_add(1).max(1);
                cache.frame_update = Some(FrameUpdate {
                    previous_revision,
                    sprites,
                    area_tiles,
                });
            },
            PrefabUpdate::Rebuild => self.rebuild_instances(id),
        }
        self.revalidate_focus();
    }
}

fn direction_state(environment: &Environment, id: TypeId, appearance: &visual::Appearance) -> DirectionState {
    let dmi_directions = appearance
        .icon
        .as_deref()
        .and_then(|icon| environment.icon(icon))
        .and_then(|metadata| metadata.find(appearance.icon_state.as_deref().unwrap_or_default()))
        .map(|state| state.dirs);

    DirectionState {
        dir: appearance.dir,
        dmi_directions,
        directional_types: directional_types_for(&environment.tree, id),
    }
}

fn direction_from_name(name: &str) -> Option<Dir> {
    match name {
        "south" => Some(Dir::South),
        "north" => Some(Dir::North),
        "east" => Some(Dir::East),
        "west" => Some(Dir::West),
        "southeast" => Some(Dir::Southeast),
        "southwest" => Some(Dir::Southwest),
        "northeast" => Some(Dir::Northeast),
        "northwest" => Some(Dir::Northwest),
        _ => None,
    }
}

fn directional_type_group(tree: &ObjectTree, selected: TypeId) -> Option<(TypeId, Option<Dir>)> {
    let declaration = tree.get(selected)?;
    let name = declaration.path.name()?.as_str();
    if name == "directional" {
        return Some((selected, None));
    }

    if let Some(direction) = direction_from_name(name)
        && let Some(parent) = declaration.parent
        && tree
            .get(parent)
            .and_then(|parent| parent.path.name())
            .is_some_and(|name| name.as_str() == "directional")
    {
        return Some((parent, Some(direction)));
    }

    declaration.children.iter().copied().find_map(|child| {
        tree.get(child)
            .and_then(|child| child.path.name())
            .is_some_and(|name| name.as_str() == "directional")
            .then_some((child, None))
    })
}

fn directional_types_for(tree: &ObjectTree, selected: TypeId) -> Option<DirectionalTypes> {
    let (group, current) = directional_type_group(tree, selected)?;
    let mut supported = [false; 8];

    for child in &tree.get(group)?.children {
        let Some(direction) = tree
            .get(*child)
            .and_then(|child| child.path.name())
            .and_then(|name| direction_from_name(name.as_str()))
        else {
            continue;
        };
        if let Some(index) = Dir::ORDER.iter().position(|candidate| *candidate == direction) {
            supported[index] = true;
        }
    }

    supported
        .iter()
        .any(|supported| *supported)
        .then_some(DirectionalTypes { supported, current })
}

fn directional_type_target(tree: &ObjectTree, selected: TypeId, direction: Dir) -> Option<TreePath> {
    let (group, _) = directional_type_group(tree, selected)?;

    tree.get(group)?.children.iter().find_map(|child| {
        let child = tree.get(*child)?;

        (child.path.name().and_then(|name| direction_from_name(name.as_str())) == Some(direction))
            .then(|| TreePath::parse(&child.path.to_string()))
    })
}

pub(crate) fn validate_level(z: u32, levels: u32) -> std::io::Result<()> {
    let levels = levels.max(1);

    if !(1..=levels).contains(&z) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("z level {z} is out of range; map has {levels} level(s)"),
        ));
    }

    Ok(())
}

pub(crate) fn discover_maps(root: &Path) -> Vec<PathBuf> {
    let mut maps = Vec::new();
    let mut pending = vec![root.to_path_buf()];

    while let Some(directory) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };

            if kind.is_dir() {
                if !is_hidden(&path) {
                    pending.push(path);
                }
            } else if kind.is_file() && is_map(&path) {
                maps.push(path);
            }
        }
    }

    maps.sort();

    maps
}

fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.'))
}

fn is_map(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("dmm"))
}

pub(crate) fn prefab_thumbnail_for(
    textures: &TextureCatalog, environment: &Environment, appearance: &visual::Appearance,
) -> Option<PrefabThumbnail> {
    let texture = frame::sprite_texture(&environment.icons, textures, appearance)?;
    let sheet = textures.texture(texture.index)?;
    let sheet_width = sheet.width() as f32;
    let sheet_height = sheet.height() as f32;
    let right = texture.source_position[0].checked_add(texture.width)?;
    let bottom = texture.source_position[1].checked_add(texture.height)?;
    let mut tint = appearance
        .color
        .as_deref()
        .and_then(render::color::parse)
        .unwrap_or([1.0; 4]);
    tint[3] *= f32::from(appearance.alpha) / 255.0;

    Some(PrefabThumbnail {
        texture,
        uv0: [
            texture.source_position[0] as f32 / sheet_width,
            texture.source_position[1] as f32 / sheet_height,
        ],
        uv1: [right as f32 / sheet_width, bottom as f32 / sheet_height],
        tint,
    })
}

pub(crate) fn build_textures(environment: &Environment, progress: &Progress) -> TextureCatalog {
    let mut textures = TextureCatalog::default();
    let base = environment.base_dir();
    let names = environment.icon_paths();

    progress.enter(Stage::Textures, names.len());
    for name in names {
        if progress.is_cancelled() {
            break;
        }
        progress.advance(name);

        if !environment.icons.contains_key(name) {
            continue;
        }

        let mut candidates =
            std::iter::once(base.join(name)).chain(environment.resource_dirs.iter().map(|dir| dir.join(name)));

        let Some(path) = candidates.find(|path| path.is_file()) else {
            log::warn!("could not find '{name}' on disk");
            continue;
        };

        match IconFile::load_info(&path) {
            Ok(info) => {
                if let Err(e) = textures.insert_info(name, &info) {
                    log::warn!("{e}");
                }
            },
            Err(e) => log::warn!("could not read '{name}': {e}"),
        }
    }

    textures
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct LoadReport {
    pub preprocess: usize,
    pub sema: usize,
    pub codegen: usize,
    pub icons: usize,
    pub lines: Vec<String>,
}

impl LoadReport {
    pub fn is_empty(&self) -> bool { self.total() == 0 }

    pub fn total(&self) -> usize { self.preprocess + self.sema + self.codegen + self.icons }

    pub fn summary(&self) -> String {
        let parts = [
            (self.preprocess, "preprocessor"),
            (self.sema, "analysis"),
            (self.codegen, "bytecode"),
            (self.icons, "icon"),
        ]
        .into_iter()
        .filter(|(count, _)| *count > 0)
        .map(|(count, label)| format!("{count} {label}"))
        .collect::<Vec<_>>();
        let plural = if self.total() == 1 { "" } else { "s" };

        match parts.split_last() {
            None => String::from("No diagnostics"),
            Some((last, [])) => format!("{last} diagnostic{plural}"),
            Some((last, rest)) => format!("{} and {last} diagnostic{plural}", rest.join(", ")),
        }
    }
}

pub(crate) fn report(environment: &Environment, diagnostics: &editor::environment::LoadDiagnostics) -> LoadReport {
    let root = environment.base_dir();
    let path = |file| {
        let path = environment.file(file)?;

        Some(path.strip_prefix(root).unwrap_or(path))
    };
    let bake_path = |file| {
        let path = environment.bake_file(file)?;

        Some(path.strip_prefix(root).unwrap_or(path))
    };
    let mut lines = Vec::new();
    let mut collect = |level, line: String, prefix: &str| {
        log::log!(level, "{}", line.strip_prefix(prefix).unwrap_or(&line));
        if lines.len() < MAX_REPORTED_DIAGNOSTICS {
            lines.push(line);
        }
    };

    for error in &diagnostics.preprocess {
        collect(
            log::Level::Error,
            error.display(path(error.location.file)).to_string(),
            "",
        );
    }

    for error in &diagnostics.sema {
        collect(
            log::Level::Error,
            error.display(path(error.location.file)).to_string(),
            "",
        );
    }

    for error in &diagnostics.bake_preprocess {
        collect(
            log::Level::Error,
            error.display(bake_path(error.location.file)).to_string(),
            "",
        );
    }

    for error in &diagnostics.bake_sema {
        collect(
            log::Level::Error,
            error.display(bake_path(error.location.file)).to_string(),
            "",
        );
    }

    if let Some(error) = &diagnostics.codegen {
        collect(
            log::Level::Warn,
            format!("warning: baking is off, bytecode generation failed: {error}"),
            "warning: ",
        );
    }

    for (name, error) in &diagnostics.icons {
        collect(
            log::Level::Warn,
            format!("warning: could not read '{name}': {error}"),
            "warning: ",
        );
    }

    LoadReport {
        preprocess: diagnostics.preprocess.len() + diagnostics.bake_preprocess.len(),
        sema: diagnostics.sema.len() + diagnostics.bake_sema.len(),
        codegen: usize::from(diagnostics.codegen.is_some()),
        icons: diagnostics.icons.len(),
        lines,
    }
}

/// The editor always loads in the background, but tests want one blocking call.
#[cfg(test)]
impl Session {
    pub(crate) fn load_environment(&mut self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        self.apply_codebase(crate::loader::load_codebase(
            path,
            &editor::environment::BakeOptions::default(),
            &Progress::new(),
        )?);

        Ok(())
    }

    fn open_map(&mut self, path: &Path, z: u32) -> Result<(), Box<dyn std::error::Error>> {
        self.apply_map(crate::loader::load_map(path, z, &Progress::new())?);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use core::{
        location::{FileId, Location, Position},
        path::TreePath,
        types::Value,
    };
    use std::{
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use dmi::{IconFile, metadata::Dir};
    use dmm::{Coord, Map, MapFormat, Prefab, Size};
    use editor::{
        Environment,
        command::EditGroupId,
        document::{MapDocument, PlacedPrefab, PrefabInstanceId, Selection, VarMutation},
        tool::{BlockSelectionMode, FillMode, SelectionMask, SelectionPlacement, SelectionRotation, Tool},
    };
    use objtree::ObjectTree;
    use render::{GuideLine, SpriteInstance};

    use super::{
        FillOutcome,
        GuideBadge,
        LevelChange,
        LoadReport,
        MAX_MAP_DIMENSION,
        Progress,
        Session,
        build_textures,
        directional_type_target,
        directional_types_for,
        discover_maps,
        validate_level,
    };

    fn examples() -> PathBuf {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env");

        path.canonicalize().unwrap_or(path)
    }

    #[test]
    fn type_sources_resolve_compiler_locations_to_loaded_files() {
        let location = Location::in_file(FileId(0), Position::new(42, 7), Position::new(42, 16));
        let mut tree = ObjectTree::new();
        let id = tree.register(&TreePath::parse("/obj/item/tool"), location);
        let mut environment = Environment::new("station.dme", tree);
        environment.files.push(PathBuf::from("code/items.dm"));
        let mut session = Session::new();
        session.state.environment = Some(Arc::new(environment));

        let source = session.type_source(id).expect("known type source");

        assert_eq!(source.path, PathBuf::from("code/items.dm"));
        assert_eq!(source.line, 42);
        assert_eq!(source.column, 7);
    }

    #[test]
    fn both_open_maps_contribute_their_own_sprites_to_one_frame() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).expect("codebase");
        session.open_map(&root.join("test.dmm"), 1).expect("first map");
        session.open_map(&root.join("test2.dmm"), 1).expect("second map");
        let ids = session.state.document_ids();

        // Two maps docked side by side in one window.
        let left = render::MapViewRect {
            x: 0,
            y: 0,
            width: 400,
            height: 600,
        };
        let right = render::MapViewRect {
            x: 400,
            y: 0,
            width: 400,
            height: 600,
        };
        let map_views = [
            session
                .map_view_frame(ids[0], left, Default::default(), Default::default(), &[], &[])
                .expect("left map view"),
            session
                .map_view_frame(ids[1], right, Default::default(), Default::default(), &[], &[])
                .expect("right map view"),
        ];
        let frame = session.frame(&map_views, Some(1));

        assert_eq!(frame.map_views.len(), 2, "both maps are in the frame");
        assert_eq!(frame.picking, Some(1));
        assert_ne!(frame.map_views[0].rect, frame.map_views[1].rect);
        // Each map view carries its own map's sprites, not a shared cache.
        assert!(!frame.map_views[0].sprite_instances.is_empty());
        assert!(!frame.map_views[1].sprite_instances.is_empty());
        assert_ne!(
            frame.map_views[0].sprite_instances.len(),
            frame.map_views[1].sprite_instances.len(),
        );
    }

    #[test]
    fn a_map_draws_before_its_bake_lands() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).expect("codebase");
        session.open_map(&root.join("test.dmm"), 1).expect("map");

        assert!(session.baker.is_busy());
        assert!(
            session
                .instances()
                .is_some_and(|instances| !instances.sprites.is_empty())
        );
        assert!(session.active_cache().bake.is_none());

        settle_bake(&mut session);

        assert!(session.active_cache().bake.is_some());
        assert_render_cache_matches_rebuild(&session);
    }

    #[test]
    fn an_edit_while_a_bake_is_running_throws_its_result_away_and_bakes_again() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).expect("codebase");
        session.open_map(&root.join("test.dmm"), 1).expect("map");
        let id = session.state.active().expect("active map");
        let coord = Coord::new(6, 3, 1);
        let target = session
            .state
            .active_document()
            .and_then(|document| document.instance_ids_at(coord).first())
            .copied()
            .expect("an instance to delete");

        session.set_tool(Tool::Delete);
        session.select_instance(Some(target));

        assert!(session.delete_instance(target));
        assert!(session.baker.invalidated(id));

        settle_bake(&mut session);

        // the bake that lands is the one that ran after the delete, so it knows nothing of the atom
        assert!(
            session
                .active_cache()
                .bake
                .as_ref()
                .is_some_and(|bake| bake.position(target.get()).is_none())
        );
    }

    #[test]
    fn hiding_a_type_redraws_from_the_cached_bake() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).expect("codebase");
        session.open_map(&root.join("test.dmm"), 1).expect("map");
        settle_bake(&mut session);
        let id = session.state.active().expect("active map");
        let table = session
            .tree()
            .and_then(|tree| tree.id_of(&TreePath::parse("/obj/structure/table")))
            .expect("table type");

        // A bake that ran again would start its counters over.
        session
            .caches
            .get_mut(&id)
            .and_then(|cache| cache.bake.as_mut())
            .expect("the example map is baked")
            .cache_hits = usize::MAX;

        assert!(session.toggle_type_visibility(table));
        assert_eq!(
            session.active_cache().bake.as_ref().map(|bake| bake.cache_hits),
            Some(usize::MAX)
        );
    }

    #[test]
    fn a_map_view_frame_follows_its_own_documents_z_level() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).expect("codebase");
        session.open_map(&root.join("test.dmm"), 1).expect("first map");
        let first = session.state.active().expect("first is active");
        // test2.dmm has two z levels.
        session.open_map(&root.join("test2.dmm"), 2).expect("second map");
        let second = session.state.active().expect("second is active");

        let rect = render::MapViewRect::default();
        let a = session
            .map_view_frame(first, rect, Default::default(), Default::default(), &[], &[])
            .expect("first map view");
        let b = session
            .map_view_frame(second, rect, Default::default(), Default::default(), &[], &[])
            .expect("second map view");

        assert_eq!(a.active_z, 1);
        assert_eq!(b.active_z, 2, "z is per document, not per editor");
    }

    #[test]
    fn two_open_maps_never_share_a_sprite_revision() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).expect("codebase");
        session.open_map(&root.join("test.dmm"), 1).expect("first map");
        session.open_map(&root.join("test2.dmm"), 1).expect("second map");

        let ids = session.state.document_ids();
        assert_eq!(ids.len(), 2, "both maps stay open");

        // The renderer holds a single `uploaded_revision` for whatever is on the
        // GPU. Two documents claiming one revision makes it skip the upload and
        // keep drawing the map that got there first.
        let revisions: Vec<u64> = ids.iter().map(|id| session.caches[id].revision).collect();
        assert_ne!(revisions[0], revisions[1]);

        // And the caches really do describe different maps.
        let first = session
            .map_view_frame(
                ids[0],
                render::MapViewRect::default(),
                render::Camera::default(),
                Default::default(),
                &[],
                &[],
            )
            .expect("first frame");
        let second = session
            .map_view_frame(
                ids[1],
                render::MapViewRect::default(),
                render::Camera::default(),
                Default::default(),
                &[],
                &[],
            )
            .expect("second frame");
        assert_ne!(first.revision, second.revision);
        assert_ne!(first.sprite_instances.len(), second.sprite_instances.len());
    }

    #[test]
    fn editing_one_open_map_leaves_the_other_cache_alone() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).expect("codebase");
        session.open_map(&root.join("test.dmm"), 1).expect("first map");
        let first = session.state.active().expect("first is active");
        session.open_map(&root.join("test2.dmm"), 1).expect("second map");
        let second = session.state.active().expect("second is active");

        let untouched = session.caches[&first].revision;
        let before = session.caches[&second].revision;

        session.choose_type(
            session
                .tree()
                .and_then(|tree| tree.id_of(&TreePath::parse("/obj/structure/table")))
                .expect("a placeable type"),
        );
        assert!(session.place_at(Coord::new(2, 2, 1), None).is_some(), "edit applied");

        assert_eq!(session.caches[&first].revision, untouched, "the other map is untouched");
        assert_ne!(session.caches[&second].revision, before, "the edited map re-uploads");
    }

    #[test]
    fn a_block_copied_from_one_map_pastes_into_another() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).expect("codebase");
        session.open_map(&root.join("test.dmm"), 1).expect("first map");
        let first = session.state.active().expect("first is active");

        session.set_tool(Tool::BlockSelect);
        session.choose_type(
            session
                .tree()
                .and_then(|tree| tree.id_of(&TreePath::parse("/obj/structure/table")))
                .expect("a placeable type"),
        );
        session.set_tool(Tool::Place);
        assert!(
            session.place_at(Coord::new(1, 1, 1), None).is_some(),
            "something to copy"
        );
        session.set_tool(Tool::BlockSelect);

        let source = Selection::from_drag(Coord::new(1, 1, 1), Coord::new(2, 2, 1));
        assert!(session.select_block(Some(source)));
        assert!(session.copy_selection(BlockSelectionMode::Full));
        let copied = session
            .state
            .active_document()
            .and_then(|document| document.placed_tile(Coord::new(1, 1, 1)))
            .expect("source tile");

        session.open_map(&root.join("test2.dmm"), 1).expect("second map");
        let second = session.state.active().expect("second is active");
        assert_ne!(first, second);

        let target_min = Coord::new(3, 3, 1);
        assert!(session.can_paste_clipboard(target_min, SelectionRotation::Original));
        assert!(session.paste_clipboard(target_min, SelectionRotation::Original));

        let pasted = session
            .state
            .document(second)
            .and_then(|document| document.placed_tile(target_min))
            .expect("pasted tile");
        assert_eq!(
            pasted.iter().map(PlacedPrefab::prefab).collect::<Vec<_>>(),
            copied.iter().map(PlacedPrefab::prefab).collect::<Vec<_>>(),
            "the prefabs survive the trip"
        );
        assert_eq!(
            session.selection(),
            Some(Selection::from_drag(target_min, Coord::new(4, 4, 1))),
            "what landed is selected"
        );

        // The block is still on the clipboard, and the map it came from never moved.
        assert!(session.clipboard().is_some());
        assert_eq!(
            session
                .state
                .document(first)
                .and_then(|document| document.placed_tile(Coord::new(1, 1, 1))),
            Some(copied)
        );
    }

    #[test]
    fn a_paste_is_one_undo_in_the_map_it_landed_in() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).expect("codebase");
        session.open_map(&root.join("test.dmm"), 1).expect("first map");
        session.set_tool(Tool::BlockSelect);
        assert!(session.select_block(Some(Selection::from_drag(Coord::new(1, 1, 1), Coord::new(2, 2, 1)))));
        assert!(session.copy_selection(BlockSelectionMode::Full));

        session.open_map(&root.join("test2.dmm"), 1).expect("second map");
        let second = session.state.active().expect("second is active");
        let before = session
            .state
            .document(second)
            .and_then(|document| document.placed_tile(Coord::new(3, 3, 1)))
            .expect("tile");

        assert!(session.paste_clipboard(Coord::new(3, 3, 1), SelectionRotation::Original));
        assert!(session.state.document(second).is_some_and(MapDocument::is_dirty));

        // `History` is per document and the paste went in as a single edit, so
        // one undo takes all of it back.
        assert!(
            session.state.document_mut(second).is_some_and(MapDocument::undo),
            "the paste is undoable"
        );
        assert_eq!(
            session
                .state
                .document(second)
                .and_then(|document| document.placed_tile(Coord::new(3, 3, 1))),
            Some(before)
        );
    }

    #[test]
    fn cross_map_paste_preserves_the_copied_mask_contents_and_destination_hole() {
        for mode in [
            BlockSelectionMode::Full,
            BlockSelectionMode::Hollow { line_width: 1 },
            BlockSelectionMode::Hollow { line_width: 2 },
        ] {
            for rotation in [
                SelectionRotation::Original,
                SelectionRotation::Clockwise,
                SelectionRotation::Half,
                SelectionRotation::CounterClockwise,
            ] {
                let mut session = flat_session(9, 11);
                let first = session.state.active().unwrap();
                let source = Selection::from_drag(Coord::new(2, 2, 1), Coord::new(6, 8, 1));
                let mut red = Prefab::new(TreePath::parse("/turf/open/floor"));
                red.set_var("color".into(), Value::Text("#ff0000".into()));
                session.state.choose_prefab(red.clone());
                session.set_tool(Tool::BlockSelect);
                assert!(session.select_block_with_mode(Some(source), mode));
                assert!(session.fill_selected_block(source.min, SelectionRotation::Original, mode));
                assert!(session.copy_selection(session.selection_mode()));
                let copied = session.clipboard().unwrap().clone();
                let source_grid = session.map().unwrap().grid.clone();
                let source_revision = session.caches[&first].revision;

                let mut map = Map::new(Size { x: 15, y: 15, z: 2 });
                let base = map.intern_tile(vec![
                    Prefab::new(TreePath::parse("/turf/open/floor")),
                    Prefab::new(TreePath::parse("/area/station")),
                ]);
                for row in map.grid.iter_mut().flatten() {
                    row.fill(base);
                }
                let second = session.activate_document(MapDocument::new(map, 2));
                let destination_grid = session.map().unwrap().grid.clone();
                let min = Coord::new(4, 4, 2);
                let target = session.clipboard_footprint(min, rotation).unwrap();
                let before = target
                    .iter()
                    .map(|coord| {
                        (
                            coord,
                            session.state.active_document().unwrap().placed_tile(coord).unwrap(),
                        )
                    })
                    .collect::<std::collections::HashMap<_, _>>();
                assert_eq!(session.selection_mask(), None);
                assert_eq!(
                    session.clipboard_preview_sprites(target, rotation).len(),
                    mode.tiles(source).count()
                );
                assert!(session.paste_clipboard(min, rotation));
                for coord in target.iter() {
                    let document = session.state.active_document().unwrap();
                    if mode.includes(target, coord) {
                        let mut expected = copied
                            .destinations(target, rotation)
                            .find(|(destination, _)| *destination == coord)
                            .unwrap()
                            .1
                            .clone();
                        for prefab in &mut expected {
                            editor::tool::rotate_prefab(session.tree().unwrap(), prefab, rotation);
                        }
                        assert_eq!(
                            document.map.tile_at(coord).unwrap(),
                            &expected,
                            "{mode:?}, {rotation:?}, {coord:?}"
                        );
                        assert_ne!(document.placed_tile(coord).unwrap()[0].id(), before[&coord][0].id());
                    } else {
                        assert_eq!(
                            document.placed_tile(coord).unwrap(),
                            before[&coord],
                            "the hole must preserve the destination's contents and IDs"
                        );
                    }
                }
                assert_eq!(session.selection_mask(), Some(SelectionMask { bounds: target, mode }));
                assert_render_cache_matches_rebuild(&session);
                assert!(session.undo());
                assert_eq!(session.map().unwrap().grid, destination_grid);
                assert!(!session.undo());
                assert!(session.redo());
                assert_render_cache_matches_rebuild(&session);
                assert_eq!(session.state.document(first).unwrap().map.grid, source_grid);
                assert_eq!(session.caches[&first].revision, source_revision);
                assert_eq!(session.state.active(), Some(second));
            }
        }
    }

    #[test]
    fn session_undo_and_redo_update_the_active_render_cache() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        session.open_map(&root.join("test.dmm"), 1).unwrap();
        let coord = Coord::new(2, 2, 1);
        let before = session.state.active_document().unwrap().placed_tile(coord).unwrap();
        let table = session
            .tree()
            .unwrap()
            .id_of(&TreePath::parse("/obj/structure/table"))
            .unwrap();

        assert_eq!(session.undo_label(), None);
        assert_eq!(session.redo_label(), None);
        assert!(session.choose_type(table));
        let placed = session.place_at(coord, None).unwrap();
        assert_eq!(session.undo_label(), Some("place /obj/structure/table"));
        assert!(session.state.active_document().unwrap().is_dirty());

        let edit_revision = session.revision();
        assert!(session.undo());
        assert_eq!(
            session.state.active_document().unwrap().placed_tile(coord),
            Some(before)
        );
        assert_eq!(session.redo_label(), Some("place /obj/structure/table"));
        assert!(!session.state.active_document().unwrap().is_dirty());
        assert_eq!(session.revision(), edit_revision.wrapping_add(1));
        assert_render_cache_matches_rebuild(&session);

        let undo_revision = session.revision();
        assert!(session.redo());
        assert_eq!(
            session
                .state
                .active_document()
                .unwrap()
                .instance_location(placed)
                .map(|location| location.coord),
            Some(coord)
        );
        assert_eq!(session.undo_label(), Some("place /obj/structure/table"));
        assert_eq!(session.redo_label(), None);
        assert!(session.state.active_document().unwrap().is_dirty());
        assert_eq!(session.revision(), undo_revision.wrapping_add(1));
        assert_render_cache_matches_rebuild(&session);
    }

    #[test]
    fn session_history_commands_only_change_the_active_document() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        session.open_map(&root.join("test.dmm"), 1).unwrap();
        let first = session.state.active().unwrap();
        let coord = Coord::new(2, 2, 1);
        let first_before = session.state.active_document().unwrap().placed_tile(coord).unwrap();
        let table = session
            .tree()
            .unwrap()
            .id_of(&TreePath::parse("/obj/structure/table"))
            .unwrap();
        assert!(session.choose_type(table));
        session.place_at(coord, None).unwrap();

        session.open_map(&root.join("test2.dmm"), 1).unwrap();
        let second = session.state.active().unwrap();
        session.place_at(coord, None).unwrap();
        let second_after = session.state.active_document().unwrap().placed_tile(coord).unwrap();

        session.state.set_active(first);
        assert!(session.undo());

        assert_eq!(
            session.state.document(first).unwrap().placed_tile(coord),
            Some(first_before)
        );
        assert_eq!(
            session.state.document(second).unwrap().placed_tile(coord),
            Some(second_after)
        );
        assert_render_cache_matches_rebuild(&session);
    }

    #[test]
    fn reopening_an_open_map_focuses_it_instead_of_duplicating_it() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).expect("codebase");
        session.open_map(&root.join("test.dmm"), 1).expect("first map");
        let first = session.state.active().expect("first is active");
        session.open_map(&root.join("test2.dmm"), 1).expect("second map");

        session
            .open_map(&root.join("test.dmm"), 1)
            .expect("reopen the first map");

        assert_eq!(session.state.document_ids().len(), 2, "no duplicate document");
        assert_eq!(session.state.active(), Some(first), "the existing tab is focused");
    }

    #[test]
    fn closing_a_map_drops_its_render_cache() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).expect("codebase");
        session.open_map(&root.join("test.dmm"), 1).expect("first map");
        let first = session.state.active().expect("first is active");
        session.open_map(&root.join("test2.dmm"), 1).expect("second map");
        let second = session.state.active().expect("second is active");

        assert!(session.close_map(first));

        assert!(!session.caches.contains_key(&first));
        assert!(session.caches.contains_key(&second));
        assert_eq!(session.state.active(), Some(second));
        assert!(
            session
                .map_view_frame(
                    first,
                    render::MapViewRect::default(),
                    render::Camera::default(),
                    Default::default(),
                    &[],
                    &[],
                )
                .is_none()
        );
    }

    #[test]
    fn a_report_summarises_only_the_kinds_of_diagnostic_it_saw() {
        let mut report = LoadReport::default();
        assert!(report.is_empty());
        assert_eq!(report.summary(), "No diagnostics");

        report.icons = 1;
        assert!(!report.is_empty());
        assert_eq!(report.summary(), "1 icon diagnostic");

        report.preprocess = 12;
        assert_eq!(report.summary(), "12 preprocessor and 1 icon diagnostics");

        report.sema = 3;
        assert_eq!(report.summary(), "12 preprocessor, 3 analysis and 1 icon diagnostics");

        report.codegen = 1;
        assert_eq!(
            report.summary(),
            "12 preprocessor, 3 analysis, 1 bytecode and 1 icon diagnostics"
        );
    }

    #[test]
    fn map_discovery_finds_maps_the_environment_never_includes() {
        let maps = discover_maps(&examples());

        assert!(
            maps.iter().any(|map| map.ends_with("test.dmm")),
            "expected test.dmm in {maps:?}"
        );
        assert!(maps.iter().all(|map| map.extension().is_some_and(|e| e == "dmm")));
    }

    #[test]
    fn map_discovery_is_sorted_and_skips_missing_roots() {
        let maps = discover_maps(&examples());
        let mut sorted = maps.clone();
        sorted.sort();

        assert_eq!(maps, sorted);
        assert!(discover_maps(&examples().join("does-not-exist")).is_empty());
    }

    #[test]
    fn a_new_map_uses_the_requested_size_and_codebase_tile_defaults() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        let path = std::env::temp_dir().join(format!("rmd-new-map-{}.dmm", std::process::id()));
        let _ = std::fs::remove_file(&path);

        session
            .create_map(&path, Size { x: 3, y: 2, z: 2 }, MapFormat::Tgm)
            .unwrap();

        let document = session.state.active_document().unwrap();
        assert_eq!(document.path.as_deref(), Some(path.as_path()));
        assert_eq!(document.map.size, Size { x: 3, y: 2, z: 2 });
        assert_eq!(document.map.format, MapFormat::Tgm);
        assert_eq!(document.z, 1);
        assert!(document.is_dirty());
        assert_eq!(document.map.dictionary.len(), 1);
        for coord in [Coord::new(1, 1, 1), Coord::new(3, 2, 2)] {
            assert_eq!(
                document
                    .map
                    .tile_at(coord)
                    .unwrap()
                    .iter()
                    .map(|prefab| prefab.path.to_string())
                    .collect::<Vec<_>>(),
                ["/turf", "/area"]
            );
        }

        session.save_map_as(&path, MapFormat::Tgm).unwrap();
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .starts_with("//MAP CONVERTED BY dmm2tgm.py")
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn in_place_save_reuses_the_confirmed_path_and_format() {
        let path = std::env::temp_dir().join(format!("rmd-save-map-{}.dmm", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut session = Session::new();
        session
            .state
            .open_document(MapDocument::create(&path, Map::new(Size { x: 1, y: 1, z: 1 }), 1));

        assert!(!session.can_save_map_in_place());
        session.save_map_as(&path, MapFormat::Tgm).expect("initial save");
        assert!(session.can_save_map_in_place());

        std::fs::write(&path, "replace me").expect("replace target contents");
        session.save_map().expect("in-place save");
        assert!(
            std::fs::read_to_string(&path)
                .expect("written map")
                .starts_with("//MAP CONVERTED BY dmm2tgm.py")
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn new_map_creation_rejects_invalid_dimensions_and_existing_targets() {
        let mut session = Session::new();
        let path = std::env::temp_dir().join(format!("rmd-new-map-collision-{}.dmm", std::process::id()));

        let error = session
            .create_map(&path, Size { x: 0, y: 1, z: 1 }, MapFormat::Standard)
            .unwrap_err();
        assert!(error.to_string().contains("between 1 and 255"));

        std::fs::write(&path, "existing").unwrap();
        let error = session
            .create_map(&path, Size { x: 1, y: 1, z: 1 }, MapFormat::Standard)
            .unwrap_err();
        assert!(error.to_string().contains("already exists"));
        let _ = std::fs::remove_file(path);
    }

    fn area_at(session: &Session, coord: Coord) -> Option<PrefabInstanceId> {
        session.area_at(session.state.active()?, coord)
    }

    fn focus_session() -> Session {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();

        let mut map = Map::new(Size { x: 4, y: 1, z: 5 });
        let base = map.intern_tile(vec![
            Prefab::new(TreePath::parse("/turf/open/floor")),
            Prefab::new(TreePath::parse("/area/station")),
        ]);
        let mut engineering = Prefab::new(TreePath::parse("/area/station"));
        engineering.set_var("name".into(), Value::Text(String::from("Engineering")));
        let other = map.intern_tile(vec![Prefab::new(TreePath::parse("/turf/open/floor")), engineering]);
        map.grid[0][0] = vec![base, base, other, base];
        for level in &mut map.grid {
            level[0] = vec![base, base, other, base];
        }
        let id = session.state.open_document(MapDocument::new(map, 1));
        session.rebuild_instances(id);

        session
    }

    fn flat_session(width: u32, height: u32) -> Session {
        let mut session = focus_session();
        let mut map = Map::new(Size {
            x: width,
            y: height,
            z: 1,
        });
        let base = map.intern_tile(vec![
            Prefab::new(TreePath::parse("/turf/open/floor")),
            Prefab::new(TreePath::parse("/area/station")),
        ]);
        for row in &mut map.grid[0] {
            row.fill(base);
        }
        let id = session.state.open_document(MapDocument::new(map, 1));
        session.rebuild_instances(id);

        session
    }

    fn assert_same_sprites(actual: &[SpriteInstance], expected: &[SpriteInstance]) {
        let mut remaining = expected.to_vec();
        assert_eq!(actual.len(), remaining.len());
        for sprite in actual {
            let index = remaining
                .iter()
                .position(|expected| expected == sprite)
                .expect("updated sprite must match a clean frame build");
            remaining.swap_remove(index);
        }
        assert!(remaining.is_empty());
    }

    /// Waits for the background bake, since a worker thread has no deterministic finish time.
    fn settle_bake(session: &mut Session) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);

        while session.baker.is_busy() && std::time::Instant::now() < deadline {
            session.poll_bake();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    fn assert_render_cache_matches_rebuild(session: &Session) {
        let environment = session.state.environment.as_ref().unwrap();
        let document = session.state.active_document().unwrap();
        let expected = editor::frame::build_with_options(
            &environment.tree,
            &environment.icons,
            &session.textures,
            document,
            editor::frame::FrameRenderOptions {
                visibility: &session.type_visibility,
                tile_size: session.options.tile_size,
                appearances: editor::bake::appearances(session.active_cache().bake.as_ref()),
                lighting: session
                    .active_cache()
                    .bake
                    .as_ref()
                    .and_then(|bake| bake.lighting.as_ref()),
            },
        );

        assert_same_sprites(&session.instances().unwrap().sprites, &expected.sprites);
        assert_same_sprites(&session.instances().unwrap().area_tiles, &expected.area_tiles);
    }

    #[test]
    fn validates_one_based_map_levels() {
        assert!(validate_level(1, 3).is_ok());
        assert!(validate_level(3, 3).is_ok());
        assert!(validate_level(0, 3).is_err());
        assert!(validate_level(4, 3).is_err());
    }

    #[test]
    fn environment_loading_packs_every_cell_in_each_dmi() {
        let root = examples();
        let (environment, diagnostics) = Environment::load(root.join("test.dme")).expect("load environment");
        assert!(diagnostics.icons.is_empty(), "{:?}", diagnostics.icons);
        let roots = environment.tree.roots();
        let obj = roots.obj.expect("embedded definitions register /obj");
        let atom = roots.atom.expect("embedded definitions register /atom");
        let movable = roots.movable.expect("embedded definitions register /atom/movable");
        assert!(environment.tree.is_subtype_of(obj, atom));
        assert!(environment.tree.is_subtype_of(obj, movable));
        assert_eq!(
            environment
                .tree
                .var_inherited(obj, &"pixel_x".into())
                .map(|variable| &variable.value),
            Some(&Value::Num(0.0)),
        );
        assert_eq!(
            environment
                .tree
                .var_inherited(obj, &"step_x".into())
                .map(|variable| &variable.value),
            Some(&Value::Num(0.0)),
        );
        let file = IconFile::load(root.join("icons/test.dmi")).expect("load icon");

        let textures = build_textures(&environment, &Progress::new());

        assert_eq!(textures.len(), 1);
        assert_eq!(textures.cell_count(), file.cell_count());
        assert!((0..file.cell_count()).all(|cell| textures.lookup("icons/test.dmi", cell).is_some()));
    }

    #[test]
    fn prefab_thumbnails_resolve_overrides_and_sheet_coordinates() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        let mut prefab = Prefab::new(TreePath::parse("/obj/structure/table"));
        let inherited = session.prefab_thumbnail(&prefab).expect("inherited table thumbnail");

        prefab.set_var("icon_state".into(), Value::Text(String::from("light")));
        prefab.set_var("color".into(), Value::Text(String::from("#ff000080")));
        prefab.set_var("alpha".into(), Value::Num(128.0));
        let overridden = session.prefab_thumbnail(&prefab).expect("overridden light thumbnail");

        assert_eq!(inherited.texture.width, 32);
        assert_eq!(inherited.texture.height, 32);
        assert_ne!(inherited.uv0, overridden.uv0);
        assert_ne!(inherited.uv1, overridden.uv1);
        assert_eq!(overridden.tint[0..3], [1.0, 0.0, 0.0]);
        assert!((overridden.tint[3] - (128.0 / 255.0) * (128.0 / 255.0)).abs() < f32::EPSILON);
        assert!(
            session
                .prefab_thumbnail(&Prefab::new(TreePath::parse("/area/station")))
                .is_none()
        );
    }

    /// A smoothed type's static `icon_state` is only a prefix, so the sheet holds nothing under it
    /// and the palette, the recent list and the placement preview would all draw nothing.
    #[test]
    fn palette_thumbnails_fall_back_to_a_standalone_bake_when_no_sprite_matches() {
        let root = examples();
        let profile = r#"
/turf/closed/wall/smoothed
    icon_state = "smooth"
/datum/demir/test/bake(atom/target)
    if(istype(target, /turf/closed/wall/smoothed))
        target.icon_state = "wall"
"#;
        let compile = |baking| {
            let arena = core::arena::StrArena::new();
            let prelude = preprocessor::prelude_files()
                .into_iter()
                .chain([preprocessor::PreludeFile::Embedded("<test-standalone.dm>", profile)]);
            let preprocessed = preprocessor::Preprocessor::new(&arena)
                .with_prelude(prelude)
                .with_baking(baking)
                .run(root.join("test.dm"))
                .expect("preprocess");
            assert!(preprocessed.is_ok(), "{:?}", preprocessed.errors);

            let ast = ast::parse(&preprocessed.tokens).expect("parse");
            let (tree, module, errors) = sema::analyze(&ast);
            assert!(errors.is_empty(), "{errors:?}");

            (tree, module)
        };
        let (editor_tree, _) = compile(false);
        let (bake_tree, module) = compile(true);
        let mut environment = editor::Environment::new(root.join("test.dme"), editor_tree);
        environment.bake_program = Some(editor::BakeProgram {
            tree: bake_tree,
            module: codegen::generate(&module).expect("codegen"),
            files: Default::default(),
            icon_states: Default::default(),
        });
        assert!(environment.load_icons(&[], &Progress::new()).is_empty());

        let mut session = Session::new();
        session.textures = build_textures(&environment, &Progress::new());
        session.state.environment = Some(Arc::new(environment));

        let smoothed = Prefab::new(TreePath::parse("/turf/closed/wall/smoothed"));

        // "smooth" is in no sheet, so only a standalone bake makes this drawable
        assert_eq!(
            session.prefab_appearance(&smoothed).and_then(|a| a.icon_state),
            Some(String::from("wall"))
        );
        assert!(session.prefab_thumbnail(&smoothed).is_some());

        session.state.choose_prefab(smoothed.clone());
        session.set_tool(Tool::Place);

        assert!(session.placement_preview().is_some());

        // a type whose static state already matches never reaches the baker
        let plain = Prefab::new(TreePath::parse("/turf/closed/wall"));

        assert_eq!(
            session.prefab_appearance(&plain).and_then(|a| a.icon_state),
            Some(String::from("wall"))
        );
        assert_eq!(session.standalone.len(), 1);
    }

    #[test]
    fn placement_previews_require_place_mode_and_preserve_visual_offsets() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        let mut prefab = Prefab::new(TreePath::parse("/obj/structure/table"));
        prefab.set_var("step_x".into(), Value::Num(-2.0));
        prefab.set_var("pixel_x".into(), Value::Num(5.0));
        prefab.set_var("pixel_w".into(), Value::Num(1.0));
        prefab.set_var("step_y".into(), Value::Num(3.0));
        prefab.set_var("pixel_y".into(), Value::Num(-4.0));
        prefab.set_var("pixel_z".into(), Value::Num(2.0));

        assert!(session.placement_preview().is_none());
        session.state.choose_prefab(prefab.clone());
        assert!(session.placement_preview().is_none());

        session.set_tool(Tool::Place);
        let preview = session.placement_preview().expect("resolved placement preview");

        assert_eq!(preview.thumbnail, session.prefab_thumbnail(&prefab).unwrap());
        assert_eq!(preview.offset, [4, 1]);

        session
            .state
            .choose_prefab(Prefab::new(TreePath::parse("/obj/unresolved")));
        assert!(session.placement_preview().is_none());
    }

    #[test]
    fn selection_guides_connect_the_tile_center_to_the_rendered_offset() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        session.open_map(&root.join("test.dmm"), 1).unwrap();
        let coord = Coord::new(6, 3, 1);
        let selected = session
            .state
            .active_document()
            .and_then(|document| document.instance_ids_at(coord).first())
            .copied()
            .unwrap();
        session.select_instance(Some(selected));

        assert_eq!(session.selected_offset_guide(), None);
        assert_eq!(
            session.edit_selected_instance_vars(
                "offset selected object",
                &[
                    VarMutation::Set("step_x".into(), Value::Num(-2.0)),
                    VarMutation::Set("pixel_x".into(), Value::Num(5.0)),
                    VarMutation::Set("pixel_w".into(), Value::Num(1.0)),
                    VarMutation::Set("step_y".into(), Value::Num(3.0)),
                    VarMutation::Set("pixel_y".into(), Value::Num(-4.0)),
                    VarMutation::Set("pixel_z".into(), Value::Num(2.0)),
                ],
                None,
            ),
            Some(true),
        );

        let guide = session.selected_offset_guide().unwrap();
        assert_eq!(guide.origin, [176.0, 80.0]);
        assert_eq!(guide.target, [180.0, 81.0]);

        session.state.active_document_mut().unwrap().z = 2;
        assert_eq!(session.selected_offset_guide(), None);
        session.state.active_document_mut().unwrap().z = 1;
        assert_eq!(
            session.edit_selected_instance_vars(
                "cancel selected object offset",
                &[
                    VarMutation::Set("step_x".into(), Value::Num(-5.0)),
                    VarMutation::Set("pixel_x".into(), Value::Num(5.0)),
                    VarMutation::Set("pixel_w".into(), Value::Num(0.0)),
                    VarMutation::Set("step_y".into(), Value::Num(4.0)),
                    VarMutation::Set("pixel_y".into(), Value::Num(-4.0)),
                    VarMutation::Set("pixel_z".into(), Value::Num(0.0)),
                ],
                None,
            ),
            Some(true),
        );
        assert_eq!(session.selected_offset_guide(), None);
    }

    #[test]
    fn selection_guides_follow_profile_connections_in_both_directions_and_across_levels() {
        static NEXT: AtomicUsize = AtomicUsize::new(0);

        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("dmed-connection-guides-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&root).expect("temp connection fixture");
        std::fs::copy(examples().join("icons/test.dmi"), root.join("test.dmi")).expect("fixture icon");
        std::fs::write(
            root.join("game.dm"),
            r#"
/area/test
/turf/floor
    icon = 'test.dmi'
    icon_state = "floor"
/obj/source
    icon = 'test.dmi'
    icon_state = "table"
    pixel_x = 4
    var/channel
/obj/target
    icon = 'test.dmi'
    icon_state = "light"
    var/channel
"#,
        )
        .expect("fixture game");
        std::fs::write(
            root.join("profile.dm"),
            r#"
#ifdef __DEMIR_BAKE__
/datum/demir/test/highlights(atom/target)
    if(!istype(target, /obj/source))
        return
    return list(list("width" = 3, "height" = 1, "when" = DEMIR_HIGHLIGHT_SELECTED))
/datum/demir/test/connections(atom/target)
    var/list/connections = list()
    if(istype(target, /obj/source))
        var/obj/source/source = target
        connections[source.channel] = DEMIR_CONNECTION_SOURCE
    else if(istype(target, /obj/target))
        var/obj/target/destination = target
        connections[destination.channel] = DEMIR_CONNECTION_TARGET
    return connections
#endif
"#,
        )
        .expect("fixture profile");
        std::fs::write(root.join("test.dme"), "#include \"game.dm\"\n#include \"profile.dm\"\n")
            .expect("fixture environment");

        let mut session = Session::new();
        session
            .load_environment(&root.join("test.dme"))
            .expect("connection environment");
        let mut map = Map::new(Size { x: 3, y: 1, z: 2 });
        let floor = || Prefab::new(TreePath::parse("/turf/floor"));
        let area = || Prefab::new(TreePath::parse("/area/test"));
        let mut source = Prefab::new(TreePath::parse("/obj/source"));
        source.set_var("channel".into(), Value::Text(String::from("doors")));
        let mut target = Prefab::new(TreePath::parse("/obj/target"));
        target.set_var("channel".into(), Value::Text(String::from("doors")));
        let source_tile = map.intern_tile(vec![source, floor(), area()]);
        let target_tile = map.intern_tile(vec![target.clone(), floor(), area()]);
        let floor_tile = map.intern_tile(vec![floor(), area()]);
        map.grid[0][0] = vec![source_tile, floor_tile, target_tile];
        map.grid[1][0] = vec![floor_tile, target_tile, floor_tile];
        session.activate_document(MapDocument::new(map, 1));
        settle_bake(&mut session);

        let endpoint = |session: &Session, coord: Coord, path: &str| {
            let document = session.state.active_document().expect("active fixture map");
            document
                .instance_ids_at(coord)
                .iter()
                .copied()
                .find(|id| {
                    document
                        .prefab_instance(*id)
                        .is_some_and(|(prefab, _)| prefab.path == TreePath::parse(path))
                })
                .expect("fixture endpoint")
        };
        let source = endpoint(&session, Coord::new(1, 1, 1), "/obj/source");
        let same_level = endpoint(&session, Coord::new(3, 1, 1), "/obj/target");
        session.select_instance(Some(source));

        let guides = session.selected_guides();
        assert_eq!(guides.lines.len(), 3, "one offset guide and two connections");
        assert!(guides.lines.contains(&GuideLine {
            origin: [16.0, 16.0],
            target: [20.0, 16.0],
        }));
        assert!(guides.lines.contains(&GuideLine {
            origin: [20.0, 16.0],
            target: [80.0, 16.0],
        }));
        assert!(guides.lines.contains(&GuideLine {
            origin: [20.0, 16.0],
            target: [48.0, 16.0],
        }));
        assert_eq!(
            guides.badges,
            vec![GuideBadge {
                position: [48.0, 16.0],
                z: 2,
            }]
        );
        assert_eq!(
            guides.connected.len(),
            2,
            "both endpoints are highlighted, the offset guide adds none"
        );
        assert!(guides.connected.contains(&same_level));

        let document_id = session.state.active().expect("active fixture document");
        let highlights = session.highlights(document_id, None);
        let [highlight] = highlights.as_slice() else {
            panic!("the selected source declares one highlight");
        };
        assert_eq!(
            highlight.tiles.iter().map(|tile| tile.position).collect::<Vec<_>>(),
            vec![[1, 1], [2, 1], [3, 1]]
        );
        assert!(
            session.highlights(document_id, Some(Coord::new(1, 1, 1))).len() == 1,
            "a selection-only highlight is not repeated by hovering its own tile"
        );

        session.select_instance(Some(same_level));
        let reverse = session.selected_guides();
        assert_eq!(
            reverse.lines,
            vec![GuideLine {
                origin: [80.0, 16.0],
                target: [20.0, 16.0],
            }]
        );
        assert!(reverse.badges.is_empty());
        assert_eq!(reverse.connected, vec![source]);
        assert!(
            session.highlights(document_id, None).is_empty(),
            "only the source declares a highlight"
        );

        assert_eq!(
            session.set_selected_instance_var("channel".into(), Value::Text(String::from("other"))),
            Some(true),
        );
        let disconnected = session.selected_guides();
        assert!(disconnected.lines.is_empty());
        assert!(disconnected.connected.is_empty());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn directional_type_groups_are_discovered_from_base_group_and_direction_paths() {
        let mut tree = ObjectTree::new();
        let base = tree.register(&TreePath::parse("/obj/alarm"), Location::default());
        let group = tree.register(&TreePath::parse("/obj/alarm/directional"), Location::default());
        let north = tree.register(&TreePath::parse("/obj/alarm/directional/north"), Location::default());
        tree.register(&TreePath::parse("/obj/alarm/directional/east"), Location::default());
        tree.register(
            &TreePath::parse("/obj/alarm/directional/northwest"),
            Location::default(),
        );
        let unrelated = tree.register(&TreePath::parse("/obj/alarm/party"), Location::default());

        let base_types = directional_types_for(&tree, base).unwrap();
        assert_eq!(base_types.current, None);
        assert_eq!(
            base_types.supported,
            [false, true, true, false, false, false, false, true]
        );
        assert_eq!(directional_types_for(&tree, group), Some(base_types));

        let north_types = directional_types_for(&tree, north).unwrap();
        assert_eq!(north_types.current, Some(Dir::North));
        assert_eq!(north_types.supported, base_types.supported);
        assert_eq!(
            directional_type_target(&tree, north, Dir::Northwest),
            Some(TreePath::parse("/obj/alarm/directional/northwest"))
        );
        assert_eq!(directional_types_for(&tree, unrelated), None);
    }

    #[test]
    fn placement_rotation_replaces_directional_paths_in_one_recent_slot() {
        let mut tree = ObjectTree::new();
        tree.register(&TreePath::parse("/obj/alarm/directional/north"), Location::default());
        tree.register(&TreePath::parse("/obj/alarm/directional/east"), Location::default());
        let mut session = Session::new();
        session.state.environment = Some(Arc::new(Environment::new(".", tree)));
        let mut prefab = Prefab::new(TreePath::parse("/obj/alarm/directional/north"));
        prefab.set_var("dir".into(), Value::Num(Dir::South.to_bits() as f32));
        session.state.choose_prefab(prefab);
        session.set_tool(Tool::Place);

        let before = session.placement_direction().unwrap();
        assert_eq!(before.directional_types.unwrap().current, Some(Dir::North));
        assert_eq!(session.set_placement_direction(Dir::East), Some(true));

        let rotated = session.palette().unwrap();
        assert_eq!(rotated.path, TreePath::parse("/obj/alarm/directional/east"));
        assert_eq!(rotated.var(&"dir".into()), None);
        assert_eq!(session.recent_prefabs(), std::slice::from_ref(rotated));
        assert_eq!(
            session
                .placement_direction()
                .unwrap()
                .directional_types
                .unwrap()
                .current,
            Some(Dir::East)
        );
        assert_eq!(session.set_placement_direction(Dir::East), Some(false));
    }

    #[test]
    fn view_changes_do_not_change_the_sprite_revision() {
        let mut session = Session::new();
        let id = session
            .state
            .open_document(MapDocument::new(Map::new(Size { x: 1, y: 1, z: 3 }), 1));
        session.caches.entry(id).or_default().revision = 7;

        session.set_level(2);
        session.set_underlay_depth(1);
        session.toggle_areas();
        assert!(session.options.show_areas);
        assert!(session.options.show_area_outlines);
        session.toggle_area_outlines();

        let view = session
            .map_view_frame(
                id,
                render::MapViewRect::default(),
                Default::default(),
                Default::default(),
                &[],
                &[],
            )
            .expect("the open map has a map view");
        assert_eq!(view.revision, 7, "changing the view does not re-upload sprites");
        assert_eq!(view.active_z, 2);

        let map_views = [view];
        let frame = session.frame(&map_views, None);
        assert_eq!(frame.underlay_depth, 1);
        assert!(frame.show_areas);
        assert!(!frame.show_area_outlines);
    }

    #[test]
    fn toggling_lighting_off_withholds_the_light_grid() {
        let mut session = Session::new();
        let id = session
            .state
            .open_document(MapDocument::new(Map::new(Size { x: 1, y: 1, z: 1 }), 1));
        let cache = session.caches.entry(id).or_default();
        cache.instances.lighting_size = [1, 1, 1];
        cache.instances.light_tiles = vec![render::LightTile { corners: [[1.0; 3]; 4] }];

        let view = |session: &Session| {
            session
                .map_view_frame(
                    id,
                    render::MapViewRect::default(),
                    Default::default(),
                    Default::default(),
                    &[],
                    &[],
                )
                .expect("the open map has a map view")
                .lighting
                .is_some()
        };

        assert!(session.options.show_lighting);
        assert!(view(&session));

        session.toggle_lighting();
        assert!(!view(&session));

        session.toggle_lighting();
        assert!(view(&session));
    }

    #[test]
    fn focus_toggles_connected_regions_and_clears_without_an_area() {
        let mut session = focus_session();
        let first = Coord::new(1, 1, 1);
        let same_region = Coord::new(2, 1, 1);
        let different_area = Coord::new(3, 1, 1);
        let disconnected_match = Coord::new(4, 1, 1);

        session.toggle_focus_at(Some(first));
        let first_focus = session.focused_area().unwrap();
        assert!(session.can_edit_at(first));
        assert!(session.can_edit_at(same_region));
        assert!(!session.can_edit_at(different_area));
        assert!(!session.can_edit_at(disconnected_match));

        session.toggle_focus_at(Some(same_region));
        assert_eq!(session.focused_area(), None);

        session.toggle_focus_at(Some(first));
        session.toggle_focus_at(Some(disconnected_match));
        assert_ne!(session.focused_area(), Some(first_focus));
        assert!(session.can_edit_at(disconnected_match));
        assert!(!session.can_edit_at(first));

        session.toggle_focus_at(None);
        assert_eq!(session.focused_area(), None);
        assert!(session.can_edit_at(first));
    }

    #[test]
    fn focus_blocks_direct_placement_outside_the_region() {
        let mut session = focus_session();
        let inside = Coord::new(1, 1, 1);
        let outside = Coord::new(4, 1, 1);
        let table = session
            .tree()
            .unwrap()
            .id_of(&TreePath::parse("/obj/structure/table"))
            .unwrap();
        let outside_before = session.map().unwrap().tile_at(outside).unwrap().clone();

        assert!(session.choose_type(table));
        session.toggle_focus_at(Some(inside));
        assert_eq!(session.place_at(outside, None), None);
        assert_eq!(session.map().unwrap().tile_at(outside), Some(&outside_before));
        assert!(session.place_at(inside, None).is_some());
    }

    #[test]
    fn focus_blocks_every_tool_not_just_placement() {
        let mut session = focus_session();
        let inside = Coord::new(1, 1, 1);
        let outside = Coord::new(4, 1, 1);
        session.toggle_focus_at(Some(inside));
        assert!(session.can_edit_at(inside));
        assert!(!session.can_edit_at(outside));

        let outside_target = session.state.active_document().unwrap().instance_ids_at(outside)[0];
        let inside_target = session.state.active_document().unwrap().instance_ids_at(inside)[0];
        let outside_before = session.map().unwrap().tile_at(outside).unwrap().clone();

        session.set_tool(Tool::Delete);
        assert!(!session.delete_instance(outside_target));
        assert!(session.delete_instance(inside_target));
        assert_eq!(session.map().unwrap().tile_at(outside), Some(&outside_before));
        assert_eq!(
            session
                .state
                .active_document()
                .unwrap()
                .instance_location(inside_target),
            None
        );

        // the gizmo drags the selection, and dragging it out of the region is the same escape
        let neighbor = Coord::new(2, 1, 1);
        let movable = session.state.active_document().unwrap().instance_ids_at(neighbor)[0];
        session.select_instance(Some(movable));
        assert_eq!(
            session.move_selected_instance(outside, "move out", &[], None),
            Some(false)
        );
        assert_eq!(session.selected_location().unwrap().coord, neighbor);
        assert_eq!(session.move_selected_instance(inside, "move in", &[], None), Some(true));
    }

    #[test]
    fn changing_levels_clears_focus() {
        let mut session = focus_session();
        session.toggle_focus_at(Some(Coord::new(1, 1, 1)));
        session.set_tool(Tool::BlockSelect);
        assert!(session.select_block(Some(Selection::from_drag(Coord::new(1, 1, 1), Coord::new(2, 1, 1),))));
        assert!(session.focused_area().is_some());
        assert!(session.selection().is_some());

        session.change_level(1);

        assert_eq!(session.z(), 2);
        assert_eq!(session.focused_area(), None);
        assert_eq!(session.selection(), None);
        assert!(session.can_edit_at(Coord::new(4, 1, 2)));
    }

    #[test]
    fn changing_up_from_the_last_level_requests_and_uses_a_fill_type() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).expect("codebase");
        let mut map = Map::new(Size { x: 2, y: 1, z: 1 });
        let key = map.intern_tile(vec![
            Prefab::new(TreePath::parse("/turf")),
            Prefab::new(TreePath::parse("/area")),
        ]);
        for cell in map.grid.iter_mut().flatten().flatten() {
            *cell = key;
        }
        let id = session.activate_document(MapDocument::new(map, 1));
        let revision = session.revision();

        assert!(session.can_change_level(1));
        assert!(!session.can_change_level(-1));
        assert_eq!(session.change_level(1), LevelChange::NewLevelRequested);
        assert_eq!(session.level_count(), 1, "the request does not create a level yet");
        assert!(session.create_level(id, "/missing/type").is_err());
        assert_eq!(session.level_count(), 1, "an invalid fill path changes nothing");
        assert_eq!(session.create_level(id, "/turf"), Ok(2));

        let document = session.state.active_document().expect("active document");
        assert_eq!(document.map.size.z, 2);
        assert_eq!(document.z, 2);
        assert!(document.is_dirty());
        for x in 1..=2 {
            assert_eq!(
                document.map.tile_at(Coord::new(x, 1, 2)).expect("filled tile")[0].path,
                TreePath::parse("/turf")
            );
        }
        assert_ne!(
            session.revision(),
            revision,
            "the appended level is rendered immediately"
        );
        assert_eq!(
            session
                .map_view_frame(id, Default::default(), Default::default(), Default::default(), &[], &[])
                .unwrap()
                .level_count,
            2
        );

        session.change_level(-1);
        assert_eq!(session.z(), 1);
        assert_eq!(
            session.level_count(),
            2,
            "moving down does not delete the appended level"
        );
    }

    #[test]
    fn level_creation_stops_at_the_map_dimension_limit() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).expect("codebase");
        let mut map = Map::new(Size {
            x: 1,
            y: 1,
            z: MAX_MAP_DIMENSION,
        });
        let key = map.intern_tile(vec![
            Prefab::new(TreePath::parse("/turf")),
            Prefab::new(TreePath::parse("/area")),
        ]);
        for cell in map.grid.iter_mut().flatten().flatten() {
            *cell = key;
        }
        session.activate_document(MapDocument::new(map, MAX_MAP_DIMENSION));

        assert!(!session.can_change_level(1));
        assert_eq!(session.change_level(1), LevelChange::Unchanged);
        assert_eq!(session.z(), MAX_MAP_DIMENSION);
        assert_eq!(session.level_count(), MAX_MAP_DIMENSION);
    }

    #[test]
    fn block_selection_clears_object_selection_and_respects_focus() {
        let mut session = focus_session();
        let inside = Coord::new(1, 1, 1);
        let outside = Coord::new(3, 1, 1);
        let object = session.state.active_document().unwrap().instance_ids_at(inside)[0];
        session.select_instance(Some(object));
        session.set_tool(Tool::BlockSelect);
        assert_eq!(session.selected_instance(), None);
        session.toggle_focus_at(Some(inside));

        assert!(!session.select_block(Some(Selection::from_drag(inside, outside))));
        assert_eq!(session.selection(), None);
        assert_eq!(session.selected_instance(), None);
        assert!(session.select_block(Some(Selection::from_drag(inside, Coord::new(2, 1, 1)))));
        assert_eq!(
            session.selection(),
            Some(Selection::from_drag(inside, Coord::new(2, 1, 1)))
        );
    }

    #[test]
    fn resizing_selection_preserves_contents_history_and_the_last_valid_region() {
        let mut session = focus_session();
        session.set_tool(Tool::BlockSelect);
        let mask = SelectionMask {
            bounds: Selection::from_drag(Coord::new(1, 1, 1), Coord::new(1, 1, 1)),
            mode: BlockSelectionMode::Hollow { line_width: 1 },
        };
        assert!(session.try_select_block(mask));
        session.toggle_focus_at(Some(mask.bounds.min));
        let revision = session.revision();
        let tiles = session.map().unwrap().grid.clone();
        let expanded = SelectionMask {
            bounds: Selection::from_drag(mask.bounds.min, Coord::new(2, 1, 1)),
            ..mask
        };
        assert!(session.try_select_block(expanded));
        assert!(!session.try_select_block(SelectionMask {
            bounds: Selection::from_drag(mask.bounds.min, Coord::new(3, 1, 1)),
            ..mask
        }));
        assert_eq!(session.selection_mask(), Some(expanded));
        assert_eq!(session.revision(), revision);
        assert_eq!(session.map().unwrap().grid, tiles);
        assert!(!session.undo());
    }

    #[test]
    fn rectangle_fill_and_bucket_share_the_mask_across_tool_switches_and_undo() {
        for mode in [BlockSelectionMode::Full, BlockSelectionMode::Hollow { line_width: 1 }] {
            let mut session = flat_session(7, 7);
            let mask = SelectionMask {
                bounds: Selection::from_drag(Coord::new(2, 2, 1), Coord::new(6, 6, 1)),
                mode,
            };
            session.set_tool(Tool::BlockSelect);
            assert!(session.try_select_block(mask));
            let mut red = Prefab::new(TreePath::parse("/turf/open/floor"));
            red.set_var("color".into(), Value::Text("#ff0000".into()));
            session.state.choose_prefab(red.clone());
            assert!(session.fill_selected_block(
                mask.bounds.min,
                SelectionRotation::Original,
                session.selection_mode()
            ));
            let mut blue = red.clone();
            blue.set_var("color".into(), Value::Text("#0000ff".into()));
            session.set_tool(Tool::Fill);
            session.state.choose_prefab(blue.clone());
            assert_eq!(session.selection_mask(), Some(mask));
            assert_eq!(
                session.fill_at(Coord::new(1, 1, 1), FillMode::Wall, &[]),
                FillOutcome::NoChange
            );
            assert_eq!(
                session.fill_at(mask.bounds.min, FillMode::Wall, &[]),
                FillOutcome::Applied
            );
            for coord in Selection::from_drag(Coord::new(1, 1, 1), Coord::new(7, 7, 1)).iter() {
                let tile = session.map().unwrap().tile_at(coord).unwrap();
                assert_eq!(tile.contains(&blue), mask.includes(coord));
            }
            assert_render_cache_matches_rebuild(&session);
            assert!(session.undo());
            assert!(session.map().unwrap().tile_at(mask.bounds.min).unwrap().contains(&red));
            assert!(session.undo());
            assert!(!session.undo());
            assert!(session.redo());
            assert!(session.redo());
            session.set_tool(Tool::BlockSelect);
            assert_eq!(session.selection_mask(), Some(mask));
            assert_render_cache_matches_rebuild(&session);
        }
    }

    #[test]
    fn one_tile_block_placements_update_multi_level_render_caches() {
        for placement in [SelectionPlacement::Move, SelectionPlacement::Copy] {
            let mut session = focus_session();
            let source = Coord::new(1, 1, 1);
            let destination = Coord::new(2, 1, 1);
            let untouched = Coord::new(1, 1, 5);
            let untouched_before = session.map().unwrap().tile_at(untouched).unwrap().clone();
            session.set_tool(Tool::BlockSelect);
            assert!(session.select_block(Some(Selection::from_drag(source, source))));
            let revision = session.revision();

            assert!(session.place_selected_block_with_mode(
                destination,
                SelectionRotation::Original,
                placement,
                BlockSelectionMode::Full,
            ));

            assert_eq!(
                session.selection(),
                Some(Selection::from_drag(destination, destination))
            );
            assert_eq!(session.map().unwrap().tile_at(untouched), Some(&untouched_before));
            assert_eq!(session.revision(), revision.wrapping_add(1));
            assert_eq!(session.frame_update().unwrap().previous_revision, revision);
            assert_render_cache_matches_rebuild(&session);
        }
    }

    #[test]
    fn block_fill_commits_the_palette_across_the_target_and_updates_render_caches() {
        let mut session = focus_session();
        let source = Coord::new(1, 1, 1);
        let destination = Coord::new(2, 1, 1);
        let untouched = Coord::new(1, 1, 5);
        let untouched_before = session.map().unwrap().tile_at(untouched).unwrap().clone();
        let table = session
            .tree()
            .unwrap()
            .id_of(&TreePath::parse("/obj/structure/table"))
            .unwrap();
        assert!(session.choose_type(table));
        let prefab = session.palette().unwrap().clone();
        session.set_tool(Tool::BlockSelect);
        assert!(session.select_block(Some(Selection::from_drag(source, source))));
        let revision = session.revision();

        assert!(session.can_fill_selected_block(source, SelectionRotation::Original, BlockSelectionMode::Full,));
        assert!(session.can_fill_selected_block(destination, SelectionRotation::Original, BlockSelectionMode::Full,));
        assert!(session.fill_selected_block(destination, SelectionRotation::Original, BlockSelectionMode::Full,));

        assert_eq!(
            session.selection(),
            Some(Selection::from_drag(destination, destination))
        );
        assert_eq!(session.palette(), Some(&prefab));
        assert!(
            session
                .map()
                .unwrap()
                .tile_at(destination)
                .unwrap()
                .iter()
                .any(|placed| placed == &prefab)
        );
        assert_eq!(session.map().unwrap().tile_at(untouched), Some(&untouched_before));
        assert_eq!(session.revision(), revision.wrapping_add(1));
        assert_eq!(session.frame_update().unwrap().previous_revision, revision);
        assert_render_cache_matches_rebuild(&session);
    }

    #[test]
    fn hollow_block_fill_passes_the_border_mode_through_the_session() {
        let mut session = flat_session(5, 5);

        let table = session
            .tree()
            .unwrap()
            .id_of(&TreePath::parse("/obj/structure/table"))
            .unwrap();
        assert!(session.choose_type(table));
        let prefab = session.palette().unwrap().clone();
        let selection = Selection::from_drag(Coord::new(1, 1, 1), Coord::new(5, 5, 1));
        session.set_tool(Tool::BlockSelect);
        assert!(session.select_block(Some(selection)));
        let revision = session.revision();

        assert!(session.fill_selected_block(
            selection.min,
            SelectionRotation::Original,
            BlockSelectionMode::Hollow { line_width: 1 },
        ));

        let has_prefab = |coord| {
            session
                .map()
                .unwrap()
                .tile_at(coord)
                .unwrap()
                .iter()
                .any(|placed| placed == &prefab)
        };
        assert!(has_prefab(Coord::new(1, 3, 1)));
        assert!(has_prefab(Coord::new(5, 3, 1)));
        assert!(!has_prefab(Coord::new(3, 3, 1)));
        assert_eq!(session.revision(), revision.wrapping_add(1));
        assert_eq!(session.frame_update().unwrap().previous_revision, revision);
        assert_render_cache_matches_rebuild(&session);
    }

    #[test]
    fn hollow_block_moves_pass_the_border_mode_through_the_session() {
        let mut session = flat_session(7, 5);
        let table = session
            .tree()
            .unwrap()
            .id_of(&TreePath::parse("/obj/structure/table"))
            .unwrap();
        assert!(session.choose_type(table));
        let prefab = session.palette().unwrap().clone();
        // (1, 3) sits on the ring and (4, 3) inside the hole of both the source and the destination
        let on_ring = session.place_at(Coord::new(1, 3, 1), None).unwrap();
        let in_hole = session.place_at(Coord::new(4, 3, 1), None).unwrap();

        let selection = Selection::from_drag(Coord::new(1, 1, 1), Coord::new(5, 5, 1));
        let mode = BlockSelectionMode::Hollow { line_width: 1 };
        session.set_tool(Tool::BlockSelect);
        assert!(session.select_block_with_mode(Some(selection), mode));
        let revision = session.revision();

        assert!(session.place_selected_block_with_mode(
            Coord::new(3, 1, 1),
            SelectionRotation::Original,
            SelectionPlacement::Move,
            mode,
        ));

        let target = Selection::from_drag(Coord::new(3, 1, 1), Coord::new(7, 5, 1));
        assert_eq!(session.selection(), Some(target));
        let document = session.state.active_document().unwrap();
        assert_eq!(document.instance_location(on_ring).unwrap().coord, Coord::new(3, 3, 1));
        assert_eq!(document.instance_location(in_hole).unwrap().coord, Coord::new(4, 3, 1));
        let has_prefab = |coord| {
            session
                .map()
                .unwrap()
                .tile_at(coord)
                .unwrap()
                .iter()
                .any(|placed| placed == &prefab)
        };
        assert!(!has_prefab(Coord::new(1, 3, 1)));
        assert!(has_prefab(Coord::new(3, 3, 1)));
        assert!(has_prefab(Coord::new(4, 3, 1)));

        assert_eq!(session.revision(), revision.wrapping_add(1));
        assert_eq!(session.frame_update().unwrap().previous_revision, revision);
        assert_render_cache_matches_rebuild(&session);
    }

    #[test]
    fn hollow_block_previews_ghost_only_the_ring_tiles() {
        let session = flat_session(5, 5);
        let selection = Selection::from_drag(Coord::new(1, 1, 1), Coord::new(5, 5, 1));

        let full = session.block_preview_sprites(
            selection,
            selection,
            SelectionRotation::Original,
            BlockSelectionMode::Full,
        );
        let hollow = session.block_preview_sprites(
            selection,
            selection,
            SelectionRotation::Original,
            BlockSelectionMode::Hollow { line_width: 1 },
        );

        // every tile of the flat map ghosts the same sprites, so the ring is 16/25ths of the whole block
        assert!(!full.is_empty());
        assert_eq!(full.len() % 25, 0);
        assert_eq!(hollow.len(), full.len() / 25 * 16);
    }

    #[test]
    fn area_edits_retain_the_seed_component_or_clear_an_invalid_seed() {
        let mut session = focus_session();
        let seed = Coord::new(1, 1, 1);
        let neighbor = Coord::new(2, 1, 1);
        session.toggle_focus_at(Some(seed));

        let neighbor_area = area_at(&session, neighbor).unwrap();
        session.select_instance(Some(neighbor_area));
        assert_eq!(
            session.set_selected_instance_var("name".into(), Value::Text(String::from("Engineering"))),
            Some(true),
        );
        assert!(session.focused_area().is_some());
        assert!(session.can_edit_at(seed));
        assert!(!session.can_edit_at(neighbor));

        let seed_area = area_at(&session, seed).unwrap();
        session.select_instance(Some(seed_area));
        assert_eq!(
            session.set_selected_instance_var("name".into(), Value::Text(String::from("Engineering"))),
            Some(true),
        );
        assert_eq!(session.focused_area(), None);
    }

    #[test]
    fn reanchoring_the_selected_instance_keeps_its_rendered_position() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        session.open_map(&root.join("test.dmm"), 1).unwrap();
        let coord = Coord::new(6, 3, 1);
        let selected = session
            .state
            .active_document()
            .and_then(|document| document.instance_ids_at(coord).first())
            .copied()
            .unwrap();
        session.select_instance(Some(selected));
        let before = session.selected_transform().unwrap();
        let destination = Coord::new(coord.x + 1, coord.y, coord.z);

        let moved = session.move_selected_instance(
            destination,
            "re-anchor object",
            &[editor::document::VarMutation::Set(
                "pixel_x".into(),
                Value::Num((before.pixel[0] - 32) as f32),
            )],
            None,
        );

        assert_eq!(moved, Some(true));
        assert_eq!(session.selected_location().unwrap().coord, destination);
        let after = session.selected_transform().unwrap();
        assert_eq!(after.pixel, [before.pixel[0] - 32, before.pixel[1]]);
        assert_eq!([after.sprite.x, after.sprite.y], [before.sprite.x, before.sprite.y]);
    }

    #[test]
    fn editing_the_selected_prefab_updates_only_its_cached_sprite() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        session.open_map(&root.join("test.dmm"), 1).unwrap();
        let coord = Coord::new(6, 3, 1);
        let selected = session
            .state
            .active_document()
            .and_then(|document| document.instance_ids_at(coord).first())
            .copied()
            .unwrap();
        assert_eq!(session.selected_prefab(), None);
        session.select_instance(Some(selected));
        let transform = session.selected_transform().unwrap();
        assert_eq!(transform.selected, selected);
        assert!(transform.is_movable);
        assert_eq!(transform.pixel, [0, 0]);
        assert_eq!(transform.step, [0, 0]);
        assert_eq!(transform.dir, 2);
        assert_eq!(transform.dmi_directions, Some(1));
        assert_eq!(transform.directional_types, None);
        assert_eq!(transform.sprite.owner, selected);
        let revision = session.revision();
        let before = session.instances().unwrap().sprites.clone();

        assert_eq!(
            session.set_selected_instance_var("pixel_x".into(), Value::Num(7.0)),
            Some(true),
        );
        assert_eq!(session.revision(), revision.wrapping_add(1));
        let update = session.frame_update().unwrap();
        assert_eq!(update.previous_revision, revision);
        let sprites = update.sprites.unwrap();
        assert_eq!(sprites.end, sprites.start + 1);
        assert_eq!(session.instances().unwrap().sprites[sprites.start].owner, selected);
        assert_eq!(session.selected_transform().unwrap().pixel, [7, 0]);
        assert_eq!(
            session
                .instances()
                .unwrap()
                .sprites
                .iter()
                .zip(before)
                .filter(|(after, before)| *after != before)
                .count(),
            1,
        );

        let rendered_revision = session.revision();
        assert_eq!(
            session.set_selected_instance_var("name".into(), Value::Text("edited".into())),
            Some(true),
        );
        assert_eq!(session.revision(), rendered_revision);
        assert_eq!(
            session.selected_prefab().and_then(|prefab| prefab.var(&"name".into())),
            Some(&Value::Text("edited".into())),
        );
    }

    #[test]
    fn editing_a_hidden_type_keeps_it_hidden_and_showing_it_uses_the_latest_appearance() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        session.open_map(&root.join("test.dmm"), 1).unwrap();
        let table = session
            .tree()
            .unwrap()
            .id_of(&TreePath::parse("/obj/structure/table"))
            .unwrap();
        let light = session
            .tree()
            .unwrap()
            .id_of(&TreePath::parse("/obj/machinery/light"))
            .unwrap();
        let coord = Coord::new(6, 3, 1);
        let selected = session.state.active_document().unwrap().instance_ids_at(coord)[0];
        session.select_instance(Some(selected));
        let light_texture = session.type_thumbnail(light).unwrap().texture;
        let before_revision = session.revision();

        assert!(session.toggle_type_visibility(table));
        assert!(!session.is_type_visible(table));
        assert_eq!(session.revision(), before_revision.wrapping_add(1));
        assert!(session.instances().unwrap().sprite(selected).is_none());
        assert_eq!(
            session.set_selected_instance_var("icon_state".into(), Value::Text(String::from("light"))),
            Some(true),
        );
        assert!(session.instances().unwrap().sprite(selected).is_none());

        assert!(session.toggle_type_visibility(table));
        assert!(session.is_type_visible(table));
        assert_eq!(
            session.instances().unwrap().sprite(selected).unwrap().texture,
            light_texture
        );
        assert_render_cache_matches_rebuild(&session);
    }

    #[test]
    fn reloading_an_environment_resets_type_visibility() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        session.open_map(&root.join("test.dmm"), 1).unwrap();
        let table = session
            .tree()
            .unwrap()
            .id_of(&TreePath::parse("/obj/structure/table"))
            .unwrap();
        let coord = Coord::new(6, 3, 1);
        let selected = session.state.active_document().unwrap().instance_ids_at(coord)[0];

        assert!(session.toggle_type_visibility(table));
        assert!(session.instances().unwrap().sprite(selected).is_none());
        session.load_environment(&root.join("test.dme")).unwrap();
        let reloaded_table = session
            .tree()
            .unwrap()
            .id_of(&TreePath::parse("/obj/structure/table"))
            .unwrap();

        assert!(session.is_type_visible(reloaded_table));
        assert!(session.instances().unwrap().sprite(selected).is_some());
        assert_render_cache_matches_rebuild(&session);
    }

    #[test]
    fn editing_an_area_updates_component_membership_incrementally() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        session.open_map(&root.join("test.dmm"), 1).unwrap();
        let area = area_at(&session, Coord::new(3, 3, 1)).unwrap();
        session.select_instance(Some(area));
        let revision = session.revision();

        assert_eq!(
            session.set_selected_instance_var("name".into(), Value::Text(String::from("Engineering"))),
            Some(true),
        );
        assert_eq!(session.revision(), revision.wrapping_add(1));
        let update = session.frame_update().expect("area edit must remain incremental");
        assert_eq!(update.previous_revision, revision);
        assert!(update.sprites.is_some());
    }

    #[test]
    fn tree_choices_place_new_objects_incrementally_and_select_them() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        session.open_map(&root.join("test.dmm"), 1).unwrap();
        let table = session
            .tree()
            .unwrap()
            .id_of(&TreePath::parse("/obj/structure/table"))
            .unwrap();
        let coord = Coord::new(2, 2, 1);
        let before_len = session.map().unwrap().tile_at(coord).unwrap().len();
        let revision = session.revision();

        assert_eq!(session.place_at(coord, None), None);
        assert!(session.choose_type(table));
        assert_eq!(session.tool(), Tool::Place);
        let selected = session.place_at(coord, None).unwrap();

        assert_eq!(session.selected_instance(), Some(selected));
        assert_eq!(session.map().unwrap().tile_at(coord).unwrap().len(), before_len + 1);
        assert_eq!(
            session.selected_prefab().unwrap().path,
            TreePath::parse("/obj/structure/table")
        );
        assert_eq!(session.recent_prefabs()[0], *session.selected_prefab().unwrap());
        assert!(session.instances().unwrap().sprite(selected).is_some());
        assert_eq!(session.revision(), revision.wrapping_add(1));
        assert_eq!(session.frame_update().unwrap().previous_revision, revision);
    }

    #[test]
    fn palette_choices_preserve_fill_and_block_select_modes() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        let floor = session
            .tree()
            .unwrap()
            .id_of(&TreePath::parse("/turf/open/floor"))
            .unwrap();

        for tool in [Tool::Fill, Tool::BlockSelect] {
            session.set_tool(tool);
            assert!(session.choose_type(floor));
            assert_eq!(session.tool(), tool);

            assert!(session.choose_recent(0));
            assert_eq!(session.tool(), tool);
        }
    }

    #[test]
    fn fill_commits_through_the_session_and_updates_rendered_turfs() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        session.open_map(&root.join("test.dmm"), 1).unwrap();
        let coord = Coord::new(2, 2, 1);
        let boundary = Coord::new(1, 1, 1);
        let turf = session.tree().unwrap().roots().turf.unwrap();
        let turf_id = session
            .state
            .active_document()
            .unwrap()
            .map
            .tile_at(coord)
            .unwrap()
            .iter()
            .zip(session.state.active_document().unwrap().instance_ids_at(coord))
            .find_map(|(prefab, id)| {
                let candidate = session.tree().unwrap().id_of(&prefab.path)?;

                session.tree().unwrap().is_subtype_of(candidate, turf).then_some(*id)
            })
            .unwrap();
        let before_sprite = *session.instances().unwrap().sprite(turf_id).unwrap();
        let mut floor = Prefab::new(TreePath::parse("/turf/open/floor"));
        floor.set_var("color".into(), Value::Text("#ff0000".into()));
        session.state.choose_prefab(floor.clone());

        assert_eq!(session.fill_at(coord, FillMode::Wall, &[]), FillOutcome::NoChange);
        session.set_tool(Tool::Fill);
        let revision = session.revision();
        assert_eq!(session.fill_at(coord, FillMode::Wall, &[]), FillOutcome::Applied);

        assert_eq!(session.revision(), revision.wrapping_add(1));
        assert!(session.frame_update().is_some());
        assert_ne!(*session.instances().unwrap().sprite(turf_id).unwrap(), before_sprite);
        assert_eq!(
            session
                .map()
                .unwrap()
                .tile_at(coord)
                .unwrap()
                .iter()
                .find(|prefab| prefab.path == floor.path)
                .unwrap(),
            &floor
        );
        assert_eq!(
            session
                .map()
                .unwrap()
                .tile_at(boundary)
                .unwrap()
                .iter()
                .find(|prefab| prefab.path.to_string().starts_with("/turf"))
                .unwrap()
                .path,
            TreePath::parse("/turf/closed/wall")
        );
        assert_eq!(session.palette(), Some(&floor));
    }

    #[test]
    fn delete_tool_removes_the_target_instance_and_its_cached_sprites() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        session.open_map(&root.join("test.dmm"), 1).unwrap();
        let coord = Coord::new(6, 3, 1);
        let target = session
            .state
            .active_document()
            .and_then(|document| document.instance_ids_at(coord).first())
            .copied()
            .unwrap();
        let before_len = session.map().unwrap().tile_at(coord).unwrap().len();
        session.select_instance(Some(target));

        assert!(!session.delete_instance(target));
        assert_eq!(session.selected_instance(), Some(target));

        session.set_tool(Tool::Delete);
        let revision = session.revision();
        assert!(session.delete_instance(target));
        assert_eq!(session.map().unwrap().tile_at(coord).unwrap().len(), before_len - 1);
        assert_eq!(session.selected_instance(), None);
        assert_eq!(session.state.active_document().unwrap().instance_location(target), None);
        assert!(
            session
                .instances()
                .unwrap()
                .sprites
                .iter()
                .all(|sprite| sprite.owner != target)
        );
        assert_eq!(session.revision(), revision.wrapping_add(1));
        assert!(!session.delete_instance(target));
    }

    #[test]
    fn deleting_areas_revalidates_or_clears_the_active_focus() {
        let mut session = focus_session();
        let seed = Coord::new(1, 1, 1);
        let neighbor = Coord::new(2, 1, 1);
        session.toggle_focus_at(Some(seed));
        session.set_tool(Tool::Delete);

        let neighbor_area = area_at(&session, neighbor).unwrap();
        assert!(session.delete_instance(neighbor_area));
        assert!(session.focused_area().is_some());
        assert!(session.can_edit_at(seed));
        assert!(!session.can_edit_at(neighbor));

        let seed_area = area_at(&session, seed).unwrap();
        assert!(session.delete_instance(seed_area));
        assert_eq!(session.focused_area(), None);
    }

    #[test]
    fn grouped_placements_are_undone_and_redone_as_one_stroke() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        session.open_map(&root.join("test.dmm"), 1).unwrap();
        let table = session
            .tree()
            .unwrap()
            .id_of(&TreePath::parse("/obj/structure/table"))
            .unwrap();
        let first = Coord::new(2, 2, 1);
        let second = Coord::new(3, 2, 1);
        let first_before = session.state.active_document().unwrap().placed_tile(first).unwrap();
        let second_before = session.state.active_document().unwrap().placed_tile(second).unwrap();

        assert!(session.choose_type(table));
        let group = EditGroupId::new();
        session.place_at(first, Some(group)).unwrap();
        session.place_at(second, Some(group)).unwrap();
        let first_after = session.state.active_document().unwrap().placed_tile(first).unwrap();
        let second_after = session.state.active_document().unwrap().placed_tile(second).unwrap();

        let document = session.state.active_document_mut().unwrap();
        assert!(document.undo());
        assert_eq!(document.placed_tile(first), Some(first_before));
        assert_eq!(document.placed_tile(second), Some(second_before));
        assert!(!document.undo());

        assert!(document.redo());
        assert_eq!(document.placed_tile(first), Some(first_after));
        assert_eq!(document.placed_tile(second), Some(second_after));
        assert!(!document.redo());
    }

    #[test]
    fn turf_placement_reuses_the_existing_id_and_picks_preserve_overrides() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        session.open_map(&root.join("test.dmm"), 1).unwrap();
        let coord = Coord::new(2, 2, 1);
        let turf_root = session.tree().unwrap().roots().turf.unwrap();
        let turf_id = session
            .state
            .active_document()
            .unwrap()
            .map
            .tile_at(coord)
            .unwrap()
            .iter()
            .zip(session.state.active_document().unwrap().instance_ids_at(coord))
            .find_map(|(prefab, id)| {
                let candidate = session.tree().unwrap().id_of(&prefab.path)?;

                session
                    .tree()
                    .unwrap()
                    .is_subtype_of(candidate, turf_root)
                    .then_some(*id)
            })
            .unwrap();
        let wall = session
            .tree()
            .unwrap()
            .id_of(&TreePath::parse("/turf/closed/wall"))
            .unwrap();

        assert!(session.choose_type(wall));
        assert_eq!(session.place_at(coord, None), Some(turf_id));
        assert_eq!(session.place_at(coord, None), None);
        assert_eq!(session.selected_instance(), Some(turf_id));
        assert_eq!(
            session.selected_prefab().unwrap().path,
            TreePath::parse("/turf/closed/wall")
        );

        assert_eq!(
            session.set_selected_instance_var("name".into(), Value::Text("custom wall".into())),
            Some(true)
        );
        session.set_tool(Tool::Select);
        session.select_instance(Some(turf_id));
        assert_eq!(session.tool(), Tool::Select);
        assert_eq!(
            session.palette().unwrap().var(&"name".into()),
            Some(&Value::Text("custom wall".into()))
        );
        assert_eq!(session.recent_prefabs().len(), 2);
    }

    #[test]
    fn exporting_the_open_map_as_tgm_round_trips_through_the_parser() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        let source = root.join("test.dmm");
        session.open_map(&source, 1).unwrap();

        let dir = std::env::temp_dir().join(format!("rmd-session-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let exported = dir.join("exported.dmm");

        let before = session.map().cloned().unwrap();
        session.save_map_as(&exported, dmm::MapFormat::Tgm).expect("export");

        assert_eq!(session.map_path(), Some(exported.as_path()));
        assert_eq!(session.map_format(), Some(dmm::MapFormat::Tgm));

        let (reparsed, errors) = dmm::parser::parse(&std::fs::read_to_string(&exported).expect("written map"));
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(reparsed.format, dmm::MapFormat::Tgm);
        assert_eq!(reparsed.grid, before.grid);
        assert_eq!(reparsed.dictionary, before.dictionary);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
