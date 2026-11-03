use core::{
    path::TreePath,
    types::{Identifier, Value},
    vars,
};

use dmi::metadata::Dir;
use editor::{
    Environment,
    command::EditGroupId,
    document::{InstanceChange, VarMutation},
    tool::Tool,
    visual,
};
use objtree::{ObjectTree, TypeId};

use super::{EditScope, Session};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DirectionalTypes {
    pub supported: [bool; 8],
    pub current: Option<Dir>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DirectionState {
    pub dir: u32,
    pub dmi_directions: Option<u32>,
    pub declared: Option<[bool; 8]>,
    pub directional_types: Option<DirectionalTypes>,
}

fn direction_mutation(direction: Dir) -> VarMutation {
    VarMutation::Set(Identifier::from(vars::DIR), Value::Num(direction.to_bits() as f32))
}

pub(super) fn direction_state(
    environment: &Environment, id: TypeId, appearance: &visual::Appearance, declared: Option<[bool; 8]>,
) -> DirectionState {
    let dmi_directions = appearance
        .icon
        .as_deref()
        .and_then(|icon| environment.icon(icon))
        .and_then(|metadata| metadata.find(appearance.icon_state.as_deref().unwrap_or_default()))
        .map(|state| state.dirs);

    DirectionState {
        dir: appearance.dir,
        dmi_directions,
        declared,
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

impl Session {
    pub(crate) fn declared_directions(&self, path: &TreePath) -> Option<[bool; 8]> {
        let program = self.state.environment.as_ref()?.bake_program.as_ref()?;
        let bake = self.caches.get(&self.state.active()?)?.bake.as_ref()?;
        let ty = program.tree.id_of(path)?;
        let rotation = vm::bake::rotation_for(&program.tree, bake.rotations(), ty)?;
        let mut supported = [false; 8];
        for direction in rotation.directions.iter().filter_map(|bits| Dir::from_bits(*bits)) {
            if let Some(index) = Dir::ORDER.iter().position(|candidate| *candidate == direction) {
                supported[index] = true;
            }
        }

        Some(supported)
    }

    pub(crate) fn picked_directional_types(&self) -> Option<DirectionalTypes> {
        let environment = self.state.environment.as_ref()?;
        let document = self.state.active_document()?;
        let picked = document.picked_instance()?;
        let prefab = document.prefab_instance(picked)?.prefab();
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

        Some(direction_state(
            environment,
            id,
            &appearance,
            self.declared_directions(&prefab.path),
        ))
    }

    pub(crate) fn set_picked_direction(&mut self, direction: Dir, group: Option<EditGroupId>) -> Option<bool> {
        if self.picked_instances().len() > 1 {
            return self.set_direction_each(direction, group);
        }

        if self.picked_directional_types().is_some() {
            return self.set_picked_directional_type(direction, group);
        }

        self.edit_picked_instance_vars("set dir", &[direction_mutation(direction)], group)
    }

    pub(crate) fn set_picked_directional_type(&mut self, direction: Dir, group: Option<EditGroupId>) -> Option<bool> {
        self.set_directional_type_in(EditScope::Selected, direction, group)
    }

    pub(crate) fn set_directional_type_in(
        &mut self, scope: EditScope, direction: Dir, group: Option<EditGroupId>,
    ) -> Option<bool> {
        if scope == EditScope::Selected && self.picked_instances().len() > 1 {
            return self.set_direction_each(direction, group);
        }

        let picked = self.picked_instance()?;
        let path = {
            let environment = self.state.environment.as_ref()?;
            let document = self.state.active_document()?;
            let prefab = document.prefab_instance(picked)?.prefab();
            let id = environment.tree.id_of(&prefab.path)?;

            directional_type_target(&environment.tree, id, direction)?
        };

        self.edit_instances_in(
            scope,
            "set direction",
            Some(&path),
            &[VarMutation::Remove(Identifier::from(vars::DIR))],
            group,
        )
    }

    fn set_direction_each(&mut self, direction: Dir, group: Option<EditGroupId>) -> Option<bool> {
        let environment = self.state.environment.as_ref()?;
        let document = self.state.active_document()?;
        let changes = self
            .picked_instances()
            .into_iter()
            .filter(|id| document.allows_edit_instance(*id))
            .filter_map(|id| {
                let prefab = document.prefab_instance(id)?.prefab();
                let type_id = environment.tree.id_of(&prefab.path)?;
                let (path, mutation) = if directional_types_for(&environment.tree, type_id).is_some() {
                    let target = directional_type_target(&environment.tree, type_id, direction)?;

                    (Some(target), VarMutation::Remove(Identifier::from(vars::DIR)))
                } else {
                    (None, direction_mutation(direction))
                };

                Some(InstanceChange {
                    id,
                    to_coord: None,
                    path,
                    mutations: vec![mutation],
                })
            })
            .collect::<Vec<_>>();
        if changes.is_empty() {
            return None;
        }

        self.change_instances(&changes, "set direction", group)
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
            prefab.remove_var(&Identifier::from(vars::DIR));
        } else {
            prefab.set_var(Identifier::from(vars::DIR), Value::Num(direction.to_bits() as f32));
        }

        Some(self.state.replace_palette(prefab))
    }
}

#[cfg(test)]
mod tests {
    use core::{
        location::Location,
        path::TreePath,
        types::{Identifier, Value},
    };
    use std::sync::Arc;

    use dmi::metadata::Dir;
    use dmm::Prefab;
    use editor::{Environment, document::VarMutation, tool::Tool};
    use objtree::ObjectTree;

    use super::{direction_mutation, directional_type_target, directional_types_for};
    use crate::session::{EditScope, Session};

    #[test]
    fn an_identical_scope_rotation_moves_every_matching_placement_to_the_new_subtype() {
        let mut tree = ObjectTree::new();
        tree.register(&TreePath::parse("/obj/alarm/directional/north"), Location::default());
        tree.register(&TreePath::parse("/obj/alarm/directional/east"), Location::default());
        let north = Prefab::new(TreePath::parse("/obj/alarm/directional/north"));
        let mut map = dmm::Map::new(dmm::Size { x: 3, y: 1, z: 1 });
        let key = map.intern_tile(vec![north.clone()]);
        let empty = map.intern_tile(vec![]);
        map.grid[0][0] = vec![key, empty, key];
        let mut session = Session::new();
        session.state.environment = Some(Arc::new(Environment::new(".", tree)));
        let id = session.state.open_document(editor::document::MapDocument::new(map, 1));
        session.rebuild_instances(id);
        let first = session
            .state
            .active_document()
            .unwrap()
            .instance_ids_at(dmm::Coord::new(1, 1, 1))[0];
        session.pick_instance(Some(first));

        assert_eq!(
            session.set_directional_type_in(EditScope::Identical, Dir::East, None),
            Some(true)
        );

        let east = Prefab::new(TreePath::parse("/obj/alarm/directional/east"));
        let document = session.state.active_document().unwrap();
        assert_eq!(document.identical_instances(&east).len(), 2);
        assert!(document.identical_instances(&north).is_empty());
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
    fn direction_mutations_use_byond_direction_bits() {
        for direction in Dir::ORDER {
            assert_eq!(
                direction_mutation(direction),
                VarMutation::Set(Identifier::from("dir"), Value::Num(direction.to_bits() as f32))
            );
        }
    }

    #[test]
    fn facing_several_picks_changes_the_path_of_directional_types_and_the_dir_of_the_rest() {
        let mut tree = ObjectTree::new();
        tree.register(&TreePath::parse("/obj/alarm/directional/north"), Location::default());
        tree.register(&TreePath::parse("/obj/alarm/directional/east"), Location::default());
        tree.register(&TreePath::parse("/obj/plain"), Location::default());
        let mut map = dmm::Map::new(dmm::Size { x: 2, y: 1, z: 1 });
        let alarm = map.intern_tile(vec![Prefab::new(TreePath::parse("/obj/alarm/directional/north"))]);
        let plain = map.intern_tile(vec![Prefab::new(TreePath::parse("/obj/plain"))]);
        map.grid[0][0] = vec![alarm, plain];
        let mut session = Session::new();
        session.state.environment = Some(Arc::new(Environment::new(".", tree)));
        let id = session.state.open_document(editor::document::MapDocument::new(map, 1));
        session.rebuild_instances(id);
        let ids = (1..=2)
            .map(|x| {
                session
                    .state
                    .active_document()
                    .unwrap()
                    .instance_ids_at(dmm::Coord::new(x, 1, 1))[0]
            })
            .collect::<Vec<_>>();
        session.toggle_pick(ids[0]);
        session.toggle_pick(ids[1]);

        assert_eq!(session.set_picked_direction(Dir::East, None), Some(true));

        let document = session.state.active_document().unwrap();
        let alarm = document.prefab_instance(ids[0]).unwrap().prefab();
        assert_eq!(alarm.path, TreePath::parse("/obj/alarm/directional/east"));
        assert_eq!(alarm.var(&"dir".into()), None);
        let plain = document.prefab_instance(ids[1]).unwrap().prefab();
        assert_eq!(plain.path, TreePath::parse("/obj/plain"));
        assert_eq!(plain.var(&"dir".into()), Some(&Value::Num(Dir::East.to_bits() as f32)));

        assert!(session.undo());
        let document = session.state.active_document().unwrap();
        let alarm = document.prefab_instance(ids[0]).unwrap().prefab();
        assert_eq!(alarm.path, TreePath::parse("/obj/alarm/directional/north"));
        assert_eq!(
            document.prefab_instance(ids[1]).unwrap().prefab().var(&"dir".into()),
            None
        );
    }
}
