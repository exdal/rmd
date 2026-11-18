use base::{coord::Coord, path::TreePath};

use prefab::{Prefab, PrefabInstanceId, PrefabLocation};
use render::SpriteInstance;

use crate::{
    document::{
        DocumentId, MapDocument, command::EditGroupId, instance::InstanceChange, instance::VarMutation,
        selection::Selection,
    },
    frame::visual,
    gizmo::direction::{DirectionalTypes, direction_state},
    session::Session,
    tool::{
        PlacementKind, SelectionRotation, Tool,
        place::placement_kind,
        transform::{rotate_point, rotate_prefab},
    },
};

#[derive(Debug)]
pub(crate) struct IdenticalCache {
    document: DocumentId,
    generation: u64,
    focus: Option<PrefabInstanceId>,
    prefab: Prefab,
    instances: Vec<PrefabInstanceId>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum EditScope {
    #[default]
    Selected,
    Identical,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PickedTransform {
    pub picked: PrefabInstanceId,
    pub sprite: SpriteInstance,
    pub pixel: [i32; 2],
    pub step: [i32; 2],
    pub is_movable: bool,
    pub dir: u32,
    pub dmi_directions: Option<u32>,
    pub declared: Option<[bool; 8]>,
    pub directional_types: Option<DirectionalTypes>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct GizmoMember {
    pub id: PrefabInstanceId,
    pub sprite: SpriteInstance,
    pub pixel: [i32; 2],
    pub step: [i32; 2],
    pub is_movable: bool,
    pub coord: Coord,
}

impl Session {
    pub fn picked_instance(&self) -> Option<PrefabInstanceId> {
        self.state.active_document().and_then(MapDocument::picked_instance)
    }

    pub fn picked_prefab(&self) -> Option<&Prefab> {
        let active = self.state.active()?;
        let picked = self.state.active_document()?.picked_instance()?;

        self.effective_grid(active)?
            .prefab_instance(picked)
            .map(|instance| instance.prefab())
    }

    pub(crate) fn picked_placement_kind(&self) -> Option<PlacementKind> {
        let environment = self.state.environment.as_ref()?;

        placement_kind(&environment.tree, self.picked_prefab()?)
    }

    pub fn picked_location(&self) -> Option<PrefabLocation> {
        let active = self.state.active()?;
        let picked = self.state.active_document()?.picked_instance()?;

        self.effective_grid(active)?.location(picked)
    }

    pub(crate) fn gizmo_transform(&self) -> Option<PickedTransform> {
        if self.tool() != Tool::Pick {
            return None;
        }

        self.picked_transform()
            .filter(|transform| transform.sprite.z == self.z())
    }

    pub(crate) fn gizmo_members(&self) -> Vec<GizmoMember> {
        if self.tool() != Tool::Pick {
            return Vec::new();
        }

        let (Some(active), Some(document)) = (self.state.active(), self.state.active_document()) else {
            return Vec::new();
        };
        let Some(grid) = self.effective_grid(active) else {
            return Vec::new();
        };
        let is_group = document.picked_instances().nth(1).is_some();

        document
            .picked_instances()
            .filter(|id| !is_group || document.allows_edit_instance(*id))
            .filter_map(|id| {
                let transform = self.picked_transform_of(id)?;
                if transform.sprite.z != self.z() {
                    return None;
                }

                Some(GizmoMember {
                    id,
                    sprite: transform.sprite,
                    pixel: transform.pixel,
                    step: transform.step,
                    is_movable: transform.is_movable,
                    coord: grid.location(id)?.coord,
                })
            })
            .collect()
    }

    pub(crate) fn picked_transform(&self) -> Option<PickedTransform> {
        let picked = self.state.active_document()?.picked_instance()?;

        self.picked_transform_of(picked)
    }

    fn picked_transform_of(&self, picked: PrefabInstanceId) -> Option<PickedTransform> {
        let environment = self.state.environment.as_ref()?;
        let document = self.state.active_document()?;
        let prefab = document.prefab_instance(picked)?.prefab();
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
        let direction = direction_state(
            environment,
            &self.textures,
            id,
            &appearance,
            self.declared_directions(&prefab.path),
        );

        Some(PickedTransform {
            picked,
            sprite: *self.instances()?.sprite(picked)?,
            pixel: [appearance.pixel_x, appearance.pixel_y],
            step: [appearance.step_x, appearance.step_y],
            is_movable,
            dir: direction.dir,
            dmi_directions: direction.dmi_directions,
            declared: direction.declared,
            directional_types: direction.directional_types,
        })
    }

    pub fn pick_instance(&mut self, picked: Option<PrefabInstanceId>) {
        let prefab = self.state.active().and_then(|active| {
            let picked = picked?;

            self.effective_grid(active)?
                .prefab_instance(picked)
                .map(|instance| instance.prefab().clone())
        });
        if let Some(document) = self.state.active_document_mut() {
            document.pick_instance(picked);
        }
        if let Some(prefab) = prefab {
            self.state.choose_prefab(prefab);
        }
    }

    pub fn pick_instances(&mut self, ids: &[PrefabInstanceId]) {
        let prefab = self.state.active().and_then(|active| {
            self.effective_grid(active)?
                .prefab_instance(*ids.last()?)
                .map(|instance| instance.prefab().clone())
        });
        if let Some(document) = self.state.active_document_mut() {
            document.pick_instances(ids);
        }
        if let Some(prefab) = prefab {
            self.state.choose_prefab(prefab);
        }
    }

    pub fn picked_instances(&self) -> Vec<PrefabInstanceId> {
        self.state
            .active_document()
            .map(|document| document.picked_instances().collect())
            .unwrap_or_default()
    }

    pub fn is_single_pick(&self) -> bool {
        self.state
            .active_document()
            .is_some_and(|document| document.picked_instances().count() == 1)
    }

    pub fn toggle_pick(&mut self, id: PrefabInstanceId) {
        let prefab = self.state.active().and_then(|active| {
            self.effective_grid(active)?
                .prefab_instance(id)
                .map(|instance| instance.prefab().clone())
        });
        let Some(document) = self.state.active_document_mut() else {
            return;
        };

        document.toggle_picked_instance(id);
        if document.picked_ids().contains(&id)
            && let Some(prefab) = prefab
        {
            self.state.choose_prefab(prefab);
        }
    }

    #[cfg(test)]
    pub fn set_picked_instance_var(
        &mut self, name: base::types::Identifier, value: base::types::Value,
    ) -> Option<bool> {
        let label = format!("set {name}");

        self.edit_picked_instance_vars(label, &[VarMutation::Set(name, value)], None)
    }

    pub fn edit_picked_instance_vars(
        &mut self, label: impl Into<String>, mutations: &[VarMutation], group: Option<EditGroupId>,
    ) -> Option<bool> {
        self.edit_instance_vars_in(EditScope::Selected, label, mutations, group)
    }

    pub(crate) fn edit_instance_vars_in(
        &mut self, scope: EditScope, label: impl Into<String>, mutations: &[VarMutation], group: Option<EditGroupId>,
    ) -> Option<bool> {
        self.edit_instances_in(scope, label, None, mutations, group)
    }

    pub(crate) fn edit_instances_in(
        &mut self, scope: EditScope, label: impl Into<String>, path: Option<&TreePath>, mutations: &[VarMutation],
        group: Option<EditGroupId>,
    ) -> Option<bool> {
        let targets = self.scope_targets(scope)?;
        let mut label = label.into();
        if targets.len() > 1 {
            label = format!("{label} on {} instances", targets.len());
        }
        let document = self.state.active_document_mut()?;

        let changed = document.edit_instances(&targets, label, path, mutations, group)?;

        if changed {
            self.update_instances(&targets);
        }

        Some(changed)
    }

    fn scope_targets(&mut self, scope: EditScope) -> Option<Vec<PrefabInstanceId>> {
        let picked = self.picked_instances();
        let primary = *picked.last()?;
        let mut targets = match scope {
            EditScope::Selected if picked.len() > 1 => {
                let document = self.state.active_document()?;
                let targets = picked
                    .into_iter()
                    .filter(|id| document.allows_edit_instance(*id))
                    .collect::<Vec<_>>();

                return (!targets.is_empty()).then_some(targets);
            },
            EditScope::Selected => Vec::new(),
            EditScope::Identical => {
                self.refresh_identical();
                self.identical().to_vec()
            },
        };
        if !targets.contains(&primary) {
            targets.push(primary);
        }

        Some(targets)
    }

    /// Placements identical to the selected one that an identical scope edit would change, as of the last
    /// [`Self::refresh_identical`]
    pub(crate) fn identical(&self) -> &[PrefabInstanceId] {
        self.identical.as_ref().map_or(&[], |cache| &cache.instances)
    }

    /// Map pixel bounds as `[left, bottom, width, height]` of the identical placements on the current level,
    /// besides the selected one
    pub(crate) fn identical_bounds(&self) -> Vec<[f32; 4]> {
        let Some(document) = self.state.active_document() else {
            return Vec::new();
        };
        let picked = document.picked_instances().collect::<Vec<_>>();
        let tile = self.options.tile_size.max(1) as f32;
        let instances = self.instances();

        self.identical()
            .iter()
            .filter(|id| !picked.contains(id))
            .filter_map(|id| {
                let location = document.instance_location(*id)?;
                if location.coord.z != document.z {
                    return None;
                }

                Some(match instances.and_then(|instances| instances.sprite(*id)) {
                    Some(sprite) => [sprite.x, sprite.y, sprite.width, sprite.height],
                    None => [
                        (location.coord.x - 1) as f32 * tile,
                        (location.coord.y - 1) as f32 * tile,
                        tile,
                        tile,
                    ],
                })
            })
            .collect()
    }

    pub(crate) fn refresh_identical(&mut self) {
        let Some(document) = self.state.active_document() else {
            self.identical = None;
            return;
        };
        let Some(instance) = document
            .picked_instance()
            .and_then(|picked| document.prefab_instance(picked))
        else {
            self.identical = None;
            return;
        };
        let prefab = instance.prefab();
        let stale = self.identical.as_ref().is_none_or(|cache| {
            cache.document != document.id()
                || cache.generation != document.generation()
                || cache.focus != document.focus().map(|focus| focus.component())
                || cache.prefab != *prefab
        });
        if stale {
            let instances = document
                .identical_instances(prefab)
                .into_iter()
                .filter(|id| document.allows_edit_instance(*id))
                .collect();
            self.identical = Some(IdenticalCache {
                document: document.id(),
                generation: document.generation(),
                focus: document.focus().map(|focus| focus.component()),
                prefab: prefab.clone(),
                instances,
            });
        }
    }

    pub(crate) fn change_instances(
        &mut self, changes: &[InstanceChange], label: impl Into<String>, group: Option<EditGroupId>,
    ) -> Option<bool> {
        let (environment, document) = self.state.active_pair_mut()?;

        let action = document.change_instances(&environment.tree, &environment.world, changes, label)?;
        if action.edit.is_empty() {
            return Some(false);
        }

        Some(self.commit(action, group))
    }

    /// Takes back `group`'s edit first, so the changes apply to the map the group started from
    pub(crate) fn rechange_instances(
        &mut self, changes: &[InstanceChange], label: impl Into<String>, group: EditGroupId,
    ) -> bool {
        let retracted = self
            .state
            .active_document_mut()
            .and_then(|document| document.retract_with_affected(group))
            .unwrap_or_default();
        let is_committed = self
            .state
            .active_pair_mut()
            .and_then(|(environment, document)| {
                document.change_instances(&environment.tree, &environment.world, changes, label)
            })
            .filter(|action| !action.edit.is_empty())
            .is_some_and(|mut action| {
                action.affected.extend_from_slice(&retracted);
                action.affected.sort_unstable();
                action.affected.dedup();

                self.commit(action, Some(group))
            });
        if !is_committed && !retracted.is_empty() {
            self.settle_retracted(&retracted);
        }

        is_committed || !retracted.is_empty()
    }

    pub(crate) fn rotate_picked_group(&mut self, rotation: SelectionRotation, group: Option<EditGroupId>) -> bool {
        let Some(changes) = self.picked_rotation_changes(rotation) else {
            return false;
        };

        self.change_instances(&changes, "rotate picks", group) == Some(true)
    }

    fn picked_rotation_changes(&self, rotation: SelectionRotation) -> Option<Vec<InstanceChange>> {
        let environment = self.state.environment.as_ref()?;
        let document = self.state.active_document()?;
        let size = document.map.size();

        if rotation == SelectionRotation::Original {
            return None;
        }

        let members = self
            .picked_instances()
            .into_iter()
            .filter(|id| document.allows_edit_instance(*id))
            .filter_map(|id| {
                let coord = document.instance_location(id)?.coord;

                (coord.z == self.z()).then_some((id, coord))
            })
            .collect::<Vec<_>>();
        if members.len() < 2 {
            return None;
        }

        let bounds = Selection::enclosing(members.iter().map(|(_, coord)| *coord))?;
        let (min, width, height) = (bounds.min, bounds.width(), bounds.height());
        let is_swapping = matches!(
            rotation,
            SelectionRotation::Clockwise | SelectionRotation::CounterClockwise
        );
        let (new_min_x, new_min_y) = if is_swapping {
            (
                i64::from(min.x) + (i64::from(width) - i64::from(height)) / 2,
                i64::from(min.y) + (i64::from(height) - i64::from(width)) / 2,
            )
        } else {
            (i64::from(min.x), i64::from(min.y))
        };

        let mut changes = Vec::with_capacity(members.len());
        for (id, coord) in members {
            let relative = (coord.x - min.x, coord.y - min.y);
            let (x, y) = rotate_point(relative, width, height, rotation);
            let destination_x = u32::try_from(new_min_x + i64::from(x))
                .ok()
                .filter(|x| (1..=size.x).contains(x))?;
            let destination_y = u32::try_from(new_min_y + i64::from(y))
                .ok()
                .filter(|y| (1..=size.y).contains(y))?;
            let destination = Coord::new(destination_x, destination_y, coord.z);
            if !document.allows_edit_at(destination) {
                return None;
            }

            let before = document.prefab_instance(id)?.prefab();
            let mut after = before.clone();
            rotate_prefab(&environment.tree, &mut after, rotation);

            let mut mutations = after
                .vars
                .iter()
                .filter(|(name, var)| before.var(name) != Some(&var.value))
                .map(|(name, var)| VarMutation::Set(name.clone(), var.value.clone()))
                .collect::<Vec<_>>();
            mutations.extend(
                before
                    .vars
                    .iter()
                    .filter(|(name, _)| after.var(name).is_none())
                    .map(|(name, _)| VarMutation::Remove(name.clone())),
            );

            changes.push(InstanceChange {
                id,
                to_coord: (destination != coord).then_some(destination),
                path: (after.path != before.path).then_some(after.path),
                mutations,
            });
        }

        Some(changes)
    }

    pub fn move_picked_instance(
        &mut self, to_coord: Coord, label: impl Into<String>, mutations: &[VarMutation], group: Option<EditGroupId>,
    ) -> Option<bool> {
        let picked = self.picked_instance()?;
        let (environment, document) = self.state.active_pair_mut()?;

        let changed = document.move_instance(&environment.tree, picked, to_coord, label, mutations, group)?;

        if changed {
            self.update_instance(picked);
        }

        Some(changed)
    }
}

#[cfg(test)]
mod tests;
