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
    visual,
};
use objtree::{ObjectTree, TypeId};
use render::{
    Frame,
    SpriteInstance,
    SpriteUpdate,
    frame::{FrameInstances, FrameOptions, PrefabUpdate},
    texture::TextureCatalog,
};

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

pub struct Session {
    pub state: EditorState,
    pub textures: TextureCatalog,
    pub options: FrameOptions,
    instances: FrameInstances,
    revision: u64,
    sprite_update: Option<SpriteUpdate>,
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
            sprite_update: None,
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
                render::frame::build(
                    &environment.tree,
                    &environment.icons,
                    &self.textures,
                    &document,
                    self.options.tile_size,
                )
            });
        self.state.open_document(document);
        self.revision = self.revision.wrapping_add(1);
        self.sprite_update = None;

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
        let dmi_directions = appearance
            .icon
            .as_deref()
            .and_then(|icon| environment.icon(icon))
            .and_then(|metadata| metadata.find(appearance.icon_state.as_deref().unwrap_or_default()))
            .map(|state| state.dirs);
        let directional_types = directional_types_for(&environment.tree, id);

        Some(SelectedTransform {
            selected,
            sprite: *self.instances.sprite(selected)?,
            pixel: [appearance.pixel_x, appearance.pixel_y],
            step: [appearance.step_x, appearance.step_y],
            is_movable,
            dir: appearance.dir,
            dmi_directions,
            directional_types,
        })
    }

    pub fn icon_metadata(&self, name: &str) -> Option<&dmi::metadata::Metadata> {
        self.state.environment.as_ref()?.icon(name)
    }

    pub fn select_instance(&mut self, selected: Option<PrefabInstanceId>) {
        if let Some(document) = self.state.active_document_mut() {
            document.select_instance(selected);
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
            sprite_update: self.sprite_update,
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
            (Some(environment), Some(document)) => render::frame::build(
                &environment.tree,
                &environment.icons,
                &self.textures,
                document,
                self.options.tile_size,
            ),
            _ => FrameInstances::default(),
        };
        self.revision = self.revision.wrapping_add(1);
        self.sprite_update = None;
    }

    fn update_instance(&mut self, selected: PrefabInstanceId) {
        let update = match (self.state.environment.as_ref(), self.state.active) {
            (Some(environment), Some(active)) => self.state.documents.get(active).map(|document| {
                render::frame::update_prefab(
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
            PrefabUpdate::Sprites { start, end } => {
                let previous_revision = self.revision;
                self.revision = self.revision.wrapping_add(1);
                self.sprite_update = Some(SpriteUpdate {
                    previous_revision,
                    start,
                    end,
                });
            },
            PrefabUpdate::Rebuild => self.rebuild_instances(),
        }
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
    use dmm::{Coord, Map, Size};
    use editor::{Environment, document::MapDocument};
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
        assert!(!session.options.show_area_outlines);
        session.toggle_area_outlines();

        let frame = session.frame(Default::default());
        assert_eq!(frame.revision, 7);
        assert_eq!(frame.active_z, 2);
        assert_eq!(frame.underlay_depth, 1);
        assert!(frame.show_areas);
        assert!(frame.show_area_outlines);
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
        let update = session.sprite_update.unwrap();
        assert_eq!(update.previous_revision, revision);
        assert_eq!(update.end, update.start + 1);
        assert_eq!(session.instances.sprites[update.start].owner, selected);
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
}
