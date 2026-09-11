use core::{
    path::TreePath,
    types::{Identifier, Value},
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use dmi::{IconFile, metadata::Dir};
use dmm::{Coord, Map, MapFormat, Prefab, Size};
use editor::{
    EditorState,
    Environment,
    command::EditGroupId,
    document::{MapDocument, PrefabInstanceId, PrefabLocation, Selection, VarMutation},
    focus::AreaFocus,
    frame::{self, FrameInstances, FrameOptions, FrameRenderOptions, PrefabUpdate, TypeVisibility},
    tool::{
        FillError,
        FillMode,
        MAX_FILL_TILES,
        SelectionPlacement,
        SelectionRotation,
        SelectionTransform,
        Tool,
        ToolContext,
        ToolEdit,
        default_tile_paths,
        is_placeable,
        place_selection as build_selection_placement,
        rotate_point,
        rotate_prefab,
        rotated_selection_at,
        transform_selection as build_selection_transform,
        transformed_selection,
    },
    visual,
};
use objtree::{ObjectTree, TypeId};
use render::{Frame, FrameUpdate, SelectionGuide, SpriteInstance, SpriteTexture, texture::TextureCatalog};

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

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BlockPreviewSprite {
    pub sprite: SpriteInstance,
    pub uv0: [f32; 2],
    pub uv1: [f32; 2],
    pub tint: [f32; 4],
}

pub struct Session {
    pub state: EditorState,
    pub textures: TextureCatalog,
    pub options: FrameOptions,
    instances: FrameInstances,
    type_visibility: TypeVisibility,
    type_thumbnails: HashMap<TypeId, Option<PrefabThumbnail>>,
    revision: u64,
    frame_update: Option<FrameUpdate>,
    texture_revision: u64,
    maps: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FillOutcome {
    Applied,
    NoChange,
    TooLarge { limit: usize },
}

impl Session {
    pub fn new() -> Self {
        Self {
            state: EditorState::new(),
            textures: TextureCatalog::default(),
            options: FrameOptions::default(),
            instances: FrameInstances::default(),
            type_visibility: TypeVisibility::default(),
            type_thumbnails: HashMap::new(),
            revision: 0,
            frame_update: None,
            texture_revision: 0,
            maps: Vec::new(),
        }
    }

    pub fn load_environment(&mut self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let (environment, diagnostics) = Environment::load(path)?;

        report(&environment, &diagnostics);
        self.textures = build_textures(&environment);
        self.type_thumbnails = environment
            .tree
            .iter()
            .map(|declaration| {
                let prefab = Prefab::new(declaration.path.clone());
                let appearance = visual::resolve_id(&environment.tree, declaration.id, &prefab);

                (declaration.id, self.prefab_thumbnail_for(&environment, &appearance))
            })
            .collect();
        self.texture_revision = self.texture_revision.wrapping_add(1);
        self.type_visibility = TypeVisibility::default();
        self.maps = discover_maps(environment.base_dir());
        self.state.environment = Some(environment);
        if self.state.active_document().is_some() {
            self.rebuild_instances();
        }

        Ok(())
    }

    pub fn open_map(&mut self, path: &Path, z: u32) -> Result<(), Box<dyn std::error::Error>> {
        let source = std::fs::read_to_string(path)?;
        let (map, errors) = dmm::parser::parse(&source);

        for error in &errors {
            eprintln!("{}: {error}", path.display());
        }

        validate_level(z, map.size.z)?;
        self.activate_document(MapDocument::open(path, map, z));

        Ok(())
    }

    pub fn create_map(&mut self, path: &Path, size: Size, format: MapFormat) -> Result<(), Box<dyn std::error::Error>> {
        const MAX_DIMENSION: u32 = 255;

        if size.x == 0
            || size.y == 0
            || size.z == 0
            || size.x > MAX_DIMENSION
            || size.y > MAX_DIMENSION
            || size.z > MAX_DIMENSION
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("map dimensions must each be between 1 and {MAX_DIMENSION}"),
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

    pub fn codebase_dir(&self) -> Option<&Path> { self.state.environment.as_ref().map(Environment::base_dir) }

    pub fn maps(&self) -> &[PathBuf] { &self.maps }

    pub fn map_path(&self) -> Option<&Path> {
        self.state
            .active_document()
            .and_then(|document| document.path.as_deref())
    }

    pub fn map_format(&self) -> Option<MapFormat> { self.map().map(|map| map.format) }

    pub fn save_map_as(&mut self, path: &Path, format: MapFormat) -> std::io::Result<()> {
        let Some(document) = self.state.active_document_mut() else {
            return Err(std::io::Error::other("no map is open"));
        };

        document.save_as(path, format)
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

    pub fn change_level(&mut self, delta: i32) {
        let Some(document) = self.state.active_document_mut() else {
            return;
        };

        let levels = document.map.size.z.max(1);
        let next = (document.z as i32 + delta).clamp(1, levels as i32) as u32;

        if next != document.z {
            document.z = next;
            document.set_focus(None);
            document.selection = None;
        }
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

    pub fn select_block(&mut self, selection: Option<Selection>) -> bool {
        let Some(document) = self.state.active_document_mut() else {
            return false;
        };

        let selection = selection.filter(|selection| {
            selection.is_well_formed()
                && selection.min.z == document.z
                && selection.max.x <= document.map.size.x
                && selection.max.y <= document.map.size.y
                && selection.iter().all(|coord| document.allows_edit_at(coord))
        });
        document.selection = selection;
        document.select_instance(None);

        selection.is_some()
    }

    pub fn place_selected_block(
        &mut self, target_min: Coord, rotation: SelectionRotation, placement: SelectionPlacement,
    ) -> bool {
        if !self.can_place_selected_block(target_min, rotation, placement) {
            return false;
        }

        let built = {
            let Some(environment) = self.state.environment.as_ref() else {
                return false;
            };
            let Some(active) = self.state.active else {
                return false;
            };
            let Some(document) = self.state.documents.get_mut(active) else {
                return false;
            };
            let Some(selection) = document.selection else {
                return false;
            };

            build_selection_placement(document, &environment.tree, selection, target_min, rotation, placement)
        };

        let Some((action, selection)) = built else {
            return false;
        };

        if !self.commit(action, None) {
            return false;
        }

        if let Some(document) = self.state.active_document_mut() {
            document.selection = Some(selection);
            document.select_instance(None);
        }

        true
    }

    pub fn can_place_selected_block(
        &self, target_min: Coord, rotation: SelectionRotation, _placement: SelectionPlacement,
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
            && selection
                .iter()
                .chain(target.iter())
                .all(|coord| document.allows_edit_at(coord))
    }

    pub fn transform_selected_block(&mut self, transform: SelectionTransform) -> bool {
        if self.tool() != Tool::BlockSelect || !self.can_transform_selected_block(transform) {
            return false;
        }

        let built = {
            let Some(environment) = self.state.environment.as_ref() else {
                return false;
            };
            let Some(active) = self.state.active else {
                return false;
            };
            let Some(document) = self.state.documents.get_mut(active) else {
                return false;
            };
            let Some(selection) = document.selection else {
                return false;
            };

            build_selection_transform(document, &environment.tree, selection, transform)
        };

        let Some((action, selection)) = built else {
            return false;
        };

        if !self.commit(action, None) {
            return false;
        }

        if let Some(document) = self.state.active_document_mut() {
            document.selection = Some(selection);
            document.select_instance(None);
        }

        true
    }

    pub fn can_transform_selected_block(&self, transform: SelectionTransform) -> bool {
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
            && selection
                .iter()
                .chain(target.iter())
                .all(|coord| document.allows_edit_at(coord))
    }

    pub(crate) fn block_preview_sprites(
        &self, source: Selection, target: Selection, rotation: SelectionRotation,
    ) -> Vec<BlockPreviewSprite> {
        let Some(document) = self.state.active_document() else {
            return Vec::new();
        };

        let Some(environment) = self.state.environment.as_ref() else {
            return Vec::new();
        };

        let area = environment.tree.roots().area;
        let mut previews = Vec::new();
        for coord in source.iter() {
            let relative = (coord.x - source.min.x, coord.y - source.min.y);
            let transformed = rotate_point(relative, source.width(), source.height(), rotation);
            let destination = Coord::new(target.min.x + transformed.0, target.min.y + transformed.1, target.min.z);
            let Some(tile) = document.placed_tile(coord) else {
                continue;
            };
            for placed in tile {
                let mut prefab = placed.prefab().clone();
                rotate_prefab(&environment.tree, &mut prefab, rotation);

                let Some(id) = environment.tree.id_of(&prefab.path) else {
                    continue;
                };
                if !self.type_visibility.is_visible(id) {
                    continue;
                }

                let is_area = area.is_some_and(|area| environment.tree.is_subtype_of(id, area));
                if is_area && !self.options.show_areas {
                    continue;
                }

                let appearance = visual::resolve_id(&environment.tree, id, &prefab);
                let Some(texture) = frame::sprite_texture(&environment.icons, &self.textures, &appearance) else {
                    continue;
                };

                let sprite = frame::instance_for(
                    placed.id(),
                    &appearance,
                    texture,
                    destination,
                    self.options.tile_size,
                    is_area,
                );

                if let Some(preview) = self.block_preview_sprite(sprite) {
                    previews.push(preview);
                }
            }
        }

        previews.sort_by(|left, right| left.sprite.depth.total_cmp(&right.sprite.depth));

        previews
    }

    fn block_preview_sprite(&self, sprite: SpriteInstance) -> Option<BlockPreviewSprite> {
        let sheet = self.textures.texture(sprite.texture.index)?;
        let right = sprite.texture.source_position[0].checked_add(sprite.texture.width)?;
        let bottom = sprite.texture.source_position[1].checked_add(sprite.texture.height)?;
        let alpha = sprite.color[3];
        let tint = if alpha > f32::EPSILON {
            [
                sprite.color[0] / alpha,
                sprite.color[1] / alpha,
                sprite.color[2] / alpha,
                alpha,
            ]
        } else {
            [1.0, 1.0, 1.0, 0.0]
        };

        Some(BlockPreviewSprite {
            sprite,
            uv0: [
                sprite.texture.source_position[0] as f32 / sheet.width() as f32,
                sprite.texture.source_position[1] as f32 / sheet.height() as f32,
            ],
            uv1: [
                right as f32 / sheet.width() as f32,
                bottom as f32 / sheet.height() as f32,
            ],
            tint,
        })
    }

    pub fn palette(&self) -> Option<&Prefab> { self.state.palette.as_ref() }

    pub fn recent_prefabs(&self) -> &[Prefab] { self.state.recent_prefabs() }

    pub(crate) fn prefab_thumbnail(&self, prefab: &Prefab) -> Option<PrefabThumbnail> {
        let environment = self.state.environment.as_ref()?;
        let appearance = visual::resolve(&environment.tree, prefab);

        self.prefab_thumbnail_for(environment, &appearance)
    }

    pub(crate) fn type_thumbnail(&self, id: TypeId) -> Option<PrefabThumbnail> {
        self.type_thumbnails.get(&id).copied().flatten()
    }

    pub(crate) fn placement_preview(&self) -> Option<PlacementPreview> {
        if self.tool() != Tool::Place {
            return None;
        }

        let prefab = self.palette()?;
        let environment = self.state.environment.as_ref()?;
        let appearance = visual::resolve(&environment.tree, prefab);
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
        let texture = frame::sprite_texture(&environment.icons, &self.textures, appearance)?;
        let sheet = self.textures.texture(texture.index)?;
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
        if self.state.tool != Tool::Fill {
            self.state.tool = Tool::Place;
        }

        true
    }

    pub fn choose_recent(&mut self, index: usize) -> bool {
        if !self.state.choose_recent(index) {
            return false;
        }
        if self.state.tool != Tool::Fill {
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
            let environment = self.state.environment.as_ref()?;
            let active = self.state.active?;
            let document = self.state.documents.get_mut(active)?;

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

        let action = {
            let Some(environment) = self.state.environment.as_ref() else {
                return FillOutcome::NoChange;
            };
            let Some(active) = self.state.active else {
                return FillOutcome::NoChange;
            };
            let Some(document) = self.state.documents.get_mut(active) else {
                return FillOutcome::NoChange;
            };

            Tool::Fill.build_fill_edit(
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
        let environment = self.state.environment.as_ref()?;
        let active = self.state.active?;
        let document = self.state.documents.get_mut(active)?;
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
            sprite: *self.instances.sprite(selected)?,
            pixel: [appearance.pixel_x, appearance.pixel_y],
            step: [appearance.step_x, appearance.step_y],
            is_movable,
            dir: direction.dir,
            dmi_directions: direction.dmi_directions,
            directional_types: direction.directional_types,
        })
    }

    pub(crate) fn selected_offset_guide(&self) -> Option<SelectionGuide> {
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

        let sprite = self.instances.sprite(selected)?;
        let tile_size = self.options.tile_size.max(1) as f32;

        Some(SelectionGuide {
            origin: [
                (location.coord.x as f32 - 0.5) * tile_size,
                (location.coord.y as f32 - 0.5) * tile_size,
            ],
            target: [sprite.x + sprite.width * 0.5, sprite.y + sprite.height * 0.5],
        })
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

    pub fn area_at(&self, coord: Coord) -> Option<PrefabInstanceId> {
        let environment = self.state.environment.as_ref()?;
        let area = environment.tree.roots().area?;
        let document = self.state.active_document()?;
        let tile = document.map.tile_at(coord)?;

        tile.iter()
            .zip(document.instance_ids_at(coord))
            .find_map(|(prefab, owner)| {
                let id = environment.tree.id_of(&prefab.path)?;

                environment.tree.is_subtype_of(id, area).then_some(*owner)
            })
    }

    pub fn focused_area(&self) -> Option<PrefabInstanceId> { Some(self.state.active_document()?.focus()?.component()) }

    pub fn toggle_focus_at(&mut self, coord: Option<Coord>) {
        if self.focused_area().is_some()
            && coord.is_none_or(|coord| self.instances.area_component_at(coord) == self.focused_area())
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
        let owner = self.area_at(seed)?;
        let prefab = self.state.active_document()?.prefab_instance(owner)?.0.clone();
        let component = self.instances.area_component_at(seed)?;

        Some(AreaFocus::new(
            seed,
            prefab,
            component,
            self.instances.area_component_tiles(component),
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
            self.rebuild_instances();
        }

        changed
    }

    pub fn set_underlay_depth(&mut self, depth: u32) {
        if depth == self.options.underlay_depth {
            return;
        }

        self.options.underlay_depth = depth;
    }

    pub fn frame(&self, camera: render::Camera) -> Frame<'_> {
        Frame {
            sprite_instances: &self.instances.sprites,
            area_tiles: &self.instances.area_tiles,
            focused_area: self.focused_area(),
            active_z: self.z(),
            underlay_depth: self.options.underlay_depth,
            show_areas: self.options.show_areas,
            show_area_outlines: self.options.show_area_outlines,
            camera,
            revision: self.revision,
            pending_update: self.frame_update,
        }
    }

    pub fn texture_revision(&self) -> u64 { self.texture_revision }

    pub fn extent_px(&self) -> (f32, f32) {
        let tile = self.options.tile_size.max(1) as f32;

        self.map().map_or((tile, tile), |map| {
            (map.size.x.max(1) as f32 * tile, map.size.y.max(1) as f32 * tile)
        })
    }

    fn rebuild_instances(&mut self) {
        self.instances = match (self.state.environment.as_ref(), self.state.active_document()) {
            (Some(environment), Some(document)) => frame::build_with_options(
                &environment.tree,
                &environment.icons,
                &self.textures,
                document,
                FrameRenderOptions {
                    visibility: &self.type_visibility,
                    tile_size: self.options.tile_size,
                },
            ),
            _ => FrameInstances::default(),
        };
        self.revision = self.revision.wrapping_add(1);
        self.frame_update = None;
        self.revalidate_focus();
    }

    fn activate_document(&mut self, document: MapDocument) {
        self.state.open_document(document);
        self.rebuild_instances();
    }

    fn update_instance(&mut self, selected: PrefabInstanceId) { self.update_instances(&[selected]); }

    fn update_instances(&mut self, affected: &[PrefabInstanceId]) {
        let update = match (self.state.environment.as_ref(), self.state.active) {
            (Some(environment), Some(active)) => self.state.documents.get(active).map(|document| {
                frame::update_prefabs_with_options(
                    &mut self.instances,
                    &environment.tree,
                    &environment.icons,
                    &self.textures,
                    document,
                    affected,
                    FrameRenderOptions {
                        visibility: &self.type_visibility,
                        tile_size: self.options.tile_size,
                    },
                )
            }),
            _ => None,
        }
        .unwrap_or(PrefabUpdate::Unchanged);

        match update {
            PrefabUpdate::Unchanged => {},
            PrefabUpdate::Buffers { sprites, area_tiles } => {
                let previous_revision = self.revision;
                self.revision = self.revision.wrapping_add(1);
                self.frame_update = Some(FrameUpdate {
                    previous_revision,
                    sprites,
                    area_tiles,
                });
            },
            PrefabUpdate::Rebuild => self.rebuild_instances(),
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

fn validate_level(z: u32, levels: u32) -> std::io::Result<()> {
    let levels = levels.max(1);

    if !(1..=levels).contains(&z) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("z level {z} is out of range; map has {levels} level(s)"),
        ));
    }

    Ok(())
}

fn discover_maps(root: &Path) -> Vec<PathBuf> {
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

fn build_textures(environment: &Environment) -> TextureCatalog {
    let mut textures = TextureCatalog::default();
    let base = environment.base_dir();

    for name in environment.icon_paths() {
        if !environment.icons.contains_key(name) {
            continue;
        }

        let mut candidates =
            std::iter::once(base.join(name)).chain(environment.resource_dirs.iter().map(|dir| dir.join(name)));

        let Some(path) = candidates.find(|path| path.is_file()) else {
            eprintln!("warning: could not find '{name}' on disk");
            continue;
        };

        match IconFile::load_info(&path) {
            Ok(info) => {
                if let Err(e) = textures.insert_info(name, &info) {
                    eprintln!("warning: {e}");
                }
            },
            Err(e) => eprintln!("warning: could not read '{name}': {e}"),
        }
    }

    textures
}

fn report(environment: &Environment, diagnostics: &editor::environment::LoadDiagnostics) {
    let root = environment.base_dir();
    let path = |file| {
        let path = environment.file(file)?;

        Some(path.strip_prefix(root).unwrap_or(path))
    };

    for error in &diagnostics.preprocess {
        eprintln!("{}", error.display(path(error.location.file)));
    }

    for error in &diagnostics.sema {
        eprintln!("{}", error.display(path(error.location.file)));
    }

    for (name, error) in &diagnostics.icons {
        eprintln!("warning: could not read '{name}': {error}");
    }
}

#[cfg(test)]
mod tests {
    use core::{location::Location, path::TreePath, types::Value};
    use std::path::PathBuf;

    use dmi::{IconFile, metadata::Dir};
    use dmm::{Coord, Map, MapFormat, Prefab, Size};
    use editor::{
        Environment,
        command::EditGroupId,
        document::{MapDocument, Selection, VarMutation},
        tool::{FillMode, SelectionPlacement, SelectionRotation, Tool},
    };
    use objtree::ObjectTree;
    use render::SpriteInstance;

    use super::{
        FillOutcome,
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
        session.state.open_document(MapDocument::new(map, 1));
        session.rebuild_instances();

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
            },
        );

        assert_same_sprites(&session.instances.sprites, &expected.sprites);
        assert_same_sprites(&session.instances.area_tiles, &expected.area_tiles);
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

        let textures = build_textures(&environment);

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
        session.state.environment = Some(Environment::new(".", tree));
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
        session
            .state
            .open_document(MapDocument::new(Map::new(Size { x: 1, y: 1, z: 3 }), 1));
        session.revision = 7;

        session.set_level(2);
        session.set_underlay_depth(1);
        session.toggle_areas();
        assert!(session.options.show_areas);
        assert!(session.options.show_area_outlines);
        session.toggle_area_outlines();

        let frame = session.frame(Default::default());
        assert_eq!(frame.revision, 7);
        assert_eq!(frame.active_z, 2);
        assert_eq!(frame.underlay_depth, 1);
        assert!(frame.show_areas);
        assert!(!frame.show_area_outlines);
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
    fn one_tile_block_placements_update_multi_level_render_caches() {
        for placement in [SelectionPlacement::Move, SelectionPlacement::Copy] {
            let mut session = focus_session();
            let source = Coord::new(1, 1, 1);
            let destination = Coord::new(2, 1, 1);
            let untouched = Coord::new(1, 1, 5);
            let untouched_before = session.map().unwrap().tile_at(untouched).unwrap().clone();
            session.set_tool(Tool::BlockSelect);
            assert!(session.select_block(Some(Selection::from_drag(source, source))));
            let revision = session.revision;

            assert!(session.place_selected_block(destination, SelectionRotation::Original, placement));

            assert_eq!(
                session.selection(),
                Some(Selection::from_drag(destination, destination))
            );
            assert_eq!(session.map().unwrap().tile_at(untouched), Some(&untouched_before));
            assert_eq!(session.revision, revision.wrapping_add(1));
            assert_eq!(session.frame_update.unwrap().previous_revision, revision);
            assert_render_cache_matches_rebuild(&session);
        }
    }

    #[test]
    fn area_edits_retain_the_seed_component_or_clear_an_invalid_seed() {
        let mut session = focus_session();
        let seed = Coord::new(1, 1, 1);
        let neighbor = Coord::new(2, 1, 1);
        session.toggle_focus_at(Some(seed));

        let neighbor_area = session.area_at(neighbor).unwrap();
        session.select_instance(Some(neighbor_area));
        assert_eq!(
            session.set_selected_instance_var("name".into(), Value::Text(String::from("Engineering"))),
            Some(true),
        );
        assert!(session.focused_area().is_some());
        assert!(session.can_edit_at(seed));
        assert!(!session.can_edit_at(neighbor));

        let seed_area = session.area_at(seed).unwrap();
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
        let revision = session.revision;
        let before = session.instances.sprites.clone();

        assert_eq!(
            session.set_selected_instance_var("pixel_x".into(), Value::Num(7.0)),
            Some(true),
        );
        assert_eq!(session.revision, revision.wrapping_add(1));
        let update = session.frame_update.unwrap();
        assert_eq!(update.previous_revision, revision);
        let sprites = update.sprites.unwrap();
        assert_eq!(sprites.end, sprites.start + 1);
        assert_eq!(session.instances.sprites[sprites.start].owner, selected);
        assert_eq!(session.selected_transform().unwrap().pixel, [7, 0]);
        assert_eq!(
            session
                .instances
                .sprites
                .iter()
                .zip(before)
                .filter(|(after, before)| *after != before)
                .count(),
            1,
        );

        let rendered_revision = session.revision;
        assert_eq!(
            session.set_selected_instance_var("name".into(), Value::Text("edited".into())),
            Some(true),
        );
        assert_eq!(session.revision, rendered_revision);
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
        let before_revision = session.revision;

        assert!(session.toggle_type_visibility(table));
        assert!(!session.is_type_visible(table));
        assert_eq!(session.revision, before_revision.wrapping_add(1));
        assert!(session.instances.sprite(selected).is_none());
        assert_eq!(
            session.set_selected_instance_var("icon_state".into(), Value::Text(String::from("light"))),
            Some(true),
        );
        assert!(session.instances.sprite(selected).is_none());

        assert!(session.toggle_type_visibility(table));
        assert!(session.is_type_visible(table));
        assert_eq!(session.instances.sprite(selected).unwrap().texture, light_texture);
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
        assert!(session.instances.sprite(selected).is_none());
        session.load_environment(&root.join("test.dme")).unwrap();
        let reloaded_table = session
            .tree()
            .unwrap()
            .id_of(&TreePath::parse("/obj/structure/table"))
            .unwrap();

        assert!(session.is_type_visible(reloaded_table));
        assert!(session.instances.sprite(selected).is_some());
        assert_render_cache_matches_rebuild(&session);
    }

    #[test]
    fn editing_an_area_updates_component_membership_incrementally() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        session.open_map(&root.join("test.dmm"), 1).unwrap();
        let area = session.area_at(Coord::new(3, 3, 1)).unwrap();
        session.select_instance(Some(area));
        let revision = session.revision;

        assert_eq!(
            session.set_selected_instance_var("name".into(), Value::Text(String::from("Engineering"))),
            Some(true),
        );
        assert_eq!(session.revision, revision.wrapping_add(1));
        let update = session.frame_update.expect("area edit must remain incremental");
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
        let revision = session.revision;

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
        assert!(session.instances.sprite(selected).is_some());
        assert_eq!(session.revision, revision.wrapping_add(1));
        assert_eq!(session.frame_update.unwrap().previous_revision, revision);
    }

    #[test]
    fn palette_choices_preserve_fill_mode() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        let floor = session
            .tree()
            .unwrap()
            .id_of(&TreePath::parse("/turf/open/floor"))
            .unwrap();

        session.set_tool(Tool::Fill);
        assert!(session.choose_type(floor));
        assert_eq!(session.tool(), Tool::Fill);

        assert!(session.choose_recent(0));
        assert_eq!(session.tool(), Tool::Fill);
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
        let before_sprite = *session.instances.sprite(turf_id).unwrap();
        let mut floor = Prefab::new(TreePath::parse("/turf/open/floor"));
        floor.set_var("color".into(), Value::Text("#ff0000".into()));
        session.state.choose_prefab(floor.clone());

        assert_eq!(session.fill_at(coord, FillMode::Wall, &[]), FillOutcome::NoChange);
        session.set_tool(Tool::Fill);
        let revision = session.revision;
        assert_eq!(session.fill_at(coord, FillMode::Wall, &[]), FillOutcome::Applied);

        assert_eq!(session.revision, revision.wrapping_add(1));
        assert!(session.frame_update.is_some());
        assert_ne!(*session.instances.sprite(turf_id).unwrap(), before_sprite);
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
        let revision = session.revision;
        assert!(session.delete_instance(target));
        assert_eq!(session.map().unwrap().tile_at(coord).unwrap().len(), before_len - 1);
        assert_eq!(session.selected_instance(), None);
        assert_eq!(session.state.active_document().unwrap().instance_location(target), None);
        assert!(session.instances.sprites.iter().all(|sprite| sprite.owner != target));
        assert_eq!(session.revision, revision.wrapping_add(1));
        assert!(!session.delete_instance(target));
    }

    #[test]
    fn deleting_areas_revalidates_or_clears_the_active_focus() {
        let mut session = focus_session();
        let seed = Coord::new(1, 1, 1);
        let neighbor = Coord::new(2, 1, 1);
        session.toggle_focus_at(Some(seed));
        session.set_tool(Tool::Delete);

        let neighbor_area = session.area_at(neighbor).unwrap();
        assert!(session.delete_instance(neighbor_area));
        assert!(session.focused_area().is_some());
        assert!(session.can_edit_at(seed));
        assert!(!session.can_edit_at(neighbor));

        let seed_area = session.area_at(seed).unwrap();
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
