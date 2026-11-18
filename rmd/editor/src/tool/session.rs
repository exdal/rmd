use base::{coord::Coord, path::TreePath};

use objtree::ObjectTree;
use prefab::{Prefab, PrefabInstanceId};

use crate::session::{FillOutcome, Session};
use crate::{
    document::{
        command::{Edit, EditGroupId, SelectionChange},
        instance::VarMutation,
    },
    tool::{self, FillError, FillMode, MAX_FILL_TILES, SelectionMask, Tool, ToolContext, ToolEdit},
};

pub(crate) fn is_reorder_label(label: &str) -> bool {
    matches!(label, "move atom to top" | "move atom to bottom")
}

pub(crate) fn context_placement_group(tree: &ObjectTree, path: &TreePath) -> Option<u8> {
    let id = tree.id_of(path)?;
    let roots = tree.roots();
    if roots.turf.is_some_and(|root| tree.is_subtype_of(id, root)) {
        Some(1)
    } else if roots.area.is_some_and(|root| tree.is_subtype_of(id, root)) {
        Some(2)
    } else if roots.atom.is_some_and(|root| tree.is_subtype_of(id, root)) {
        Some(0)
    } else {
        None
    }
}

impl Session {
    pub fn tool(&self) -> Tool {
        self.state.tool
    }

    pub fn set_tool(&mut self, tool: Tool) {
        if tool == Tool::Node && !self.node_tool_available() {
            return;
        }

        if tool == Tool::Comment && !self.comment_tool_available() {
            return;
        }

        if self.state.tool == Tool::Node && tool != Tool::Node {
            self.cancel_node_edit();
        }

        if self.state.tool == Tool::Select && tool != Tool::Select {
            self.cancel_live_move();
        }
        self.state.tool = tool;
        if tool == Tool::Select
            && let Some(document) = self.state.active_document_mut()
        {
            document.pick_instance(None);
        }
    }

    pub fn place_at(&mut self, coord: Coord, group: Option<EditGroupId>) -> Option<PrefabInstanceId> {
        self.place_with(coord, group, false)
    }

    pub fn place_replacing_objs_at(&mut self, coord: Coord, group: Option<EditGroupId>) -> Option<PrefabInstanceId> {
        self.place_with(coord, group, true)
    }

    fn place_with(&mut self, coord: Coord, group: Option<EditGroupId>, replace_objs: bool) -> Option<PrefabInstanceId> {
        if self.state.tool != Tool::Place || !self.can_edit_at(coord) {
            return None;
        }
        let prefab = self.state.palette.clone()?;
        let hidden = replace_objs.then(|| self.hidden_types());
        let action = {
            let (environment, document) = self.state.active_pair_mut()?;

            match &hidden {
                Some(hidden) => tool::place::place_replacing_objs(document, &environment.tree, coord, &prefab, hidden),
                None => Tool::Place.build_edit(&mut ToolContext {
                    document,
                    tree: &environment.tree,
                    world: &environment.world,
                    prefab: Some(&prefab),
                    target: None,
                    coord,
                    anchor: None,
                    fill_mode: FillMode::default(),
                    custom_fill_boundaries: &[],
                }),
            }
        }?;
        let picked = action.picked?;
        if !self.commit(action, group) {
            return None;
        }

        if let Some(document) = self.state.active_document_mut() {
            document.pick_instance(Some(picked));
        }
        self.state.choose_prefab(prefab);

        Some(picked)
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
                    world: &environment.world,
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

        let target = self.source_root_under(target).unwrap_or(target);

        self.build_delete(target)
            .is_some_and(|action| self.commit(action, None))
    }

    pub fn delete_picked_instances(&mut self) -> bool {
        let targets = self.picked_instances();

        self.build_delete_instances(&targets, "delete")
            .is_some_and(|action| self.commit(action, None))
    }

    pub fn replace_instance(&mut self, target: PrefabInstanceId) -> bool {
        if self.state.tool != Tool::Replace {
            return false;
        }

        self.state
            .active()
            .is_some_and(|document| self.replace_instances(document, &[target]))
    }

    pub fn delete_context_instance(&mut self, target: PrefabInstanceId) -> bool {
        self.build_delete(target)
            .is_some_and(|action| self.commit(action, None))
    }

    pub fn can_edit_instance(&self, target: PrefabInstanceId) -> bool {
        self.state
            .active_document()
            .is_some_and(|document| document.allows_edit_instance(target))
    }

    pub fn reorder_instance(&mut self, target: PrefabInstanceId, to_top: bool) -> bool {
        let Some((environment, document)) = self.state.active_pair_mut() else {
            return false;
        };
        let Some(location) = document.instance_location(target) else {
            return false;
        };
        if !document.allows_edit_instance(target) {
            return false;
        }
        let Some(mut tile) = document.placed_tile(location.coord) else {
            return false;
        };
        let is_object = |prefab: &Prefab| context_placement_group(&environment.tree, &prefab.path) == Some(0);
        if !tile
            .get(location.prefab_index)
            .is_some_and(|placed| is_object(placed.prefab()))
        {
            return false;
        }
        let positions = tile
            .iter()
            .enumerate()
            .filter_map(|(index, placed)| is_object(placed.prefab()).then_some(index))
            .collect::<Vec<_>>();
        let destination = if to_top {
            *positions.last().unwrap()
        } else {
            positions[0]
        };
        if destination == location.prefab_index {
            return false;
        }
        let placed = tile.remove(location.prefab_index);
        tile.insert(destination, placed);
        let mut edit = Edit::new(if to_top {
            "move atom to top"
        } else {
            "move atom to bottom"
        });
        edit.change(document, location.coord, tile);
        let applied = self
            .state
            .active_document_mut()
            .is_some_and(|document| document.apply_grouped(edit, None));
        if applied {
            self.refresh_reordered_tile(location.coord);
        }

        applied
    }

    pub fn reset_instance_to_default(&mut self, target: PrefabInstanceId) -> bool {
        if !self.can_edit_instance(target) {
            return false;
        }

        let Some(document) = self.state.active_document_mut() else {
            return false;
        };

        let Some(instance) = document.prefab_instance(target) else {
            return false;
        };

        let mutations = instance
            .prefab()
            .vars
            .iter()
            .map(|(name, _)| VarMutation::Remove(name.clone()))
            .collect::<Vec<_>>();
        let changed = document
            .edit_instance_vars(target, "reset atom to default", &mutations, None)
            .unwrap_or(false);

        if changed {
            self.update_instance(target);
        }

        changed
    }

    pub fn replace_context_instance(&mut self, target: PrefabInstanceId, path: TreePath) -> bool {
        if !self.can_edit_instance(target) {
            return false;
        }
        let Some(environment) = self.state.environment.as_ref() else {
            return false;
        };
        let Some(document) = self.state.active_document() else {
            return false;
        };
        let Some(instance) = document.prefab_instance(target) else {
            return false;
        };
        let prefab = instance.prefab();
        let kind = context_placement_group(&environment.tree, &prefab.path);
        if kind.is_none() || kind != context_placement_group(&environment.tree, &path) {
            return false;
        }
        let mutations = prefab
            .vars
            .iter()
            .map(|(name, _)| VarMutation::Remove(name.clone()))
            .collect::<Vec<_>>();
        let changed = self
            .state
            .active_document_mut()
            .and_then(|document| document.replace_instance_path(target, "replace atom", path, &mutations, None))
            .unwrap_or(false);

        if changed {
            self.update_instance(target);
        }

        changed
    }

    pub(crate) fn build_delete_instances(&mut self, targets: &[PrefabInstanceId], verb: &str) -> Option<ToolEdit> {
        let (environment, document) = self.state.active_pair_mut()?;

        tool::place::delete_instances(document, &environment.tree, &environment.world, targets, verb)
    }

    fn build_delete(&mut self, target: PrefabInstanceId) -> Option<ToolEdit> {
        let (environment, document) = self.state.active_pair_mut()?;
        let coord = Coord::new(1, 1, document.z);

        Tool::Delete.build_edit(&mut ToolContext {
            document,
            tree: &environment.tree,
            world: &environment.world,
            prefab: None,
            target: Some(target),
            coord,
            anchor: None,
            fill_mode: FillMode::default(),
            custom_fill_boundaries: &[],
        })
    }

    /// Commits a region edit that leaves `after` selected, undo puts the old selection back
    pub(crate) fn commit_selecting(
        &mut self, mut action: ToolEdit, after: SelectionMask, group: Option<EditGroupId>,
    ) -> bool {
        action.edit = action.edit.selecting(SelectionChange {
            before: self.selection_mask(),
            after: Some(after),
        });

        if !self.commit(action, group) {
            return false;
        }

        if let Some(document) = self.state.active_document_mut() {
            document.set_selection(Some(after.bounds), after.mode);
            document.pick_instance(None);
        }

        true
    }

    pub(crate) fn commit(&mut self, action: ToolEdit, group: Option<EditGroupId>) -> bool {
        let affected = action.affected;
        let applied = self
            .state
            .active_document_mut()
            .is_some_and(|document| document.apply_grouped(action.edit, group));
        if applied {
            self.update_instances(&affected);
            if let Some(id) = self.state.active()
                && let Some(cache) = self.caches.get_mut(&id)
            {
                cache.map_revision = cache.map_revision.wrapping_add(1);
            }
        }

        applied
    }
}

#[cfg(test)]
mod tests;
