use core::{
    path::TreePath,
    types::{Identifier, Value},
};
use std::path::{Path, PathBuf};

use dmi::{IconFile, metadata::Dir};
use dmm::{Coord, Map, Prefab};
use editor::{
    EditorState,
    Environment,
    command::EditGroupId,
    document::{MapDocument, PrefabInstanceId, PrefabLocation, VarMutation},
    frame::{self, FrameInstances, FrameOptions, PrefabUpdate},
    tool::{Tool, ToolContext, is_placeable},
    visual,
};
use objtree::{ObjectTree, TypeId};
use render::{Frame, FrameUpdate, SpriteInstance, SpriteTexture, texture::TextureCatalog};

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

pub struct Session {
    pub state: EditorState,
    pub textures: TextureCatalog,
    pub options: FrameOptions,
    instances: FrameInstances,
    revision: u64,
    frame_update: Option<FrameUpdate>,
    texture_revision: u64,
}

impl Session {
    pub fn new() -> Self {
        Self {
            state: EditorState::new(),
            textures: TextureCatalog::default(),
            options: FrameOptions::default(),
            instances: FrameInstances::default(),
            revision: 0,
            frame_update: None,
            texture_revision: 0,
        }
    }

    pub fn load_environment(&mut self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let (environment, diagnostics) = Environment::load(path)?;

        report(&environment, &diagnostics);
        self.textures = build_textures(&environment);
        self.texture_revision = self.texture_revision.wrapping_add(1);
        self.state.environment = Some(environment);

        Ok(())
    }

    pub fn open_map(&mut self, path: &Path, z: u32) -> Result<(), Box<dyn std::error::Error>> {
        let source = std::fs::read_to_string(path)?;
        let (map, errors) = dmm::parser::parse(&source);

        for error in &errors {
            eprintln!("{}: {error}", path.display());
        }

        validate_level(z, map.size.z)?;
        let document = MapDocument::open(path, map, z);

        self.instances = self
            .state
            .environment
            .as_ref()
            .map_or_else(FrameInstances::default, |environment| {
                frame::build(
                    &environment.tree,
                    &environment.icons,
                    &self.textures,
                    &document,
                    self.options.tile_size,
                )
            });
        self.state.open_document(document);
        self.revision = self.revision.wrapping_add(1);
        self.frame_update = None;

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
    }

    pub fn change_level(&mut self, delta: i32) {
        let Some(document) = self.state.active_document_mut() else {
            return;
        };

        let levels = document.map.size.z.max(1);
        let next = (document.z as i32 + delta).clamp(1, levels as i32) as u32;

        if next != document.z {
            document.z = next;
        }
    }

    pub fn selected_instance(&self) -> Option<PrefabInstanceId> {
        self.state.active_document().and_then(MapDocument::selected_instance)
    }

    pub fn tool(&self) -> Tool { self.state.tool }

    pub fn set_tool(&mut self, tool: Tool) { self.state.tool = tool; }

    pub fn palette(&self) -> Option<&Prefab> { self.state.palette.as_ref() }

    pub fn recent_prefabs(&self) -> &[Prefab] { self.state.recent_prefabs() }

    pub(crate) fn prefab_thumbnail(&self, prefab: &Prefab) -> Option<PrefabThumbnail> {
        let environment = self.state.environment.as_ref()?;
        let appearance = visual::resolve(&environment.tree, prefab);

        self.prefab_thumbnail_for(environment, &appearance)
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
        self.state.tool = Tool::Place;

        true
    }

    pub fn choose_recent(&mut self, index: usize) -> bool {
        if !self.state.choose_recent(index) {
            return false;
        }
        self.state.tool = Tool::Place;

        true
    }

    pub fn place_at(&mut self, coord: Coord, group: Option<EditGroupId>) -> Option<PrefabInstanceId> {
        if self.state.tool != Tool::Place {
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
                coord,
                anchor: None,
            })
        }?;
        let selected = action.selected;
        let affected = action.affected;
        if let Some(document) = self.state.active_document_mut() {
            document.apply_grouped(action.edit, group);
            document.select_instance(Some(selected));
        }
        self.state.choose_prefab(prefab);

        if affected.len() == 1 {
            self.update_instance(affected[0]);
        } else {
            self.rebuild_instances();
        }

        Some(selected)
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

    pub fn toggle_areas(&mut self) { self.options.show_areas = !self.options.show_areas; }

    pub fn toggle_area_outlines(&mut self) { self.options.show_area_outlines = !self.options.show_area_outlines; }

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
            (Some(environment), Some(document)) => frame::build(
                &environment.tree,
                &environment.icons,
                &self.textures,
                document,
                self.options.tile_size,
            ),
            _ => FrameInstances::default(),
        };
        self.revision = self.revision.wrapping_add(1);
        self.frame_update = None;
    }

    fn update_instance(&mut self, selected: PrefabInstanceId) {
        let update = match (self.state.environment.as_ref(), self.state.active) {
            (Some(environment), Some(active)) => self.state.documents.get(active).map(|document| {
                frame::update_prefab(
                    &mut self.instances,
                    &environment.tree,
                    &environment.icons,
                    &self.textures,
                    document,
                    selected,
                    self.options.tile_size,
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
    use dmm::{Coord, Map, Prefab, Size};
    use editor::{Environment, command::EditGroupId, document::MapDocument, tool::Tool};
    use objtree::ObjectTree;

    use super::{Session, build_textures, directional_type_target, directional_types_for, validate_level};

    fn examples() -> PathBuf {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env");

        path.canonicalize().unwrap_or(path)
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
    fn editing_an_area_uses_a_partial_frame_update() {
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
        assert!(update.area_tiles.is_none());
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
}
