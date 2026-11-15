use base::{
    path::TreePath,
    types::{Identifier, Value},
    vars,
};

use dmi::metadata::Dir;
use objtree::{ObjectTree, TypeId};

use crate::{
    document::{InstanceChange, VarMutation, command::EditGroupId},
    environment::Environment,
    frame::visual,
    inspector::session::EditScope,
    session::Session,
    tool::{
        Tool,
        transform::{direction_from_name, directional_type_group, directional_type_target},
    },
};

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

pub(crate) fn direction_state(
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
mod tests;
