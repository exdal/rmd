use core::path::TreePath;

use dmm::{Coord, Prefab};
use editor::{
    command::{Edit, EditGroupId},
    document::{PrefabInstanceId, VarMutation},
    tool::{self, FillError, FillMode, MAX_FILL_TILES, Tool, ToolContext, ToolEdit},
};
use objtree::ObjectTree;

use super::{FillOutcome, Session};

pub(super) fn is_reorder_label(label: &str) -> bool { matches!(label, "move atom to top" | "move atom to bottom") }

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
    pub fn tool(&self) -> Tool { self.state.tool }

    pub fn set_tool(&mut self, tool: Tool) {
        if tool == Tool::Node && !self.node_tool_available() {
            return;
        }
        if self.state.tool == Tool::Node && tool != Tool::Node {
            self.cancel_node_edit();
        }
        self.state.tool = tool;
        if tool == Tool::BlockSelect
            && let Some(document) = self.state.active_document_mut()
        {
            document.select_instance(None);
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
                Some(hidden) => tool::place_replacing_objs(document, &environment.tree, coord, &prefab, hidden),
                None => Tool::Place.build_edit(&mut ToolContext {
                    document,
                    tree: &environment.tree,
                    prefab: Some(&prefab),
                    target: None,
                    coord,
                    anchor: None,
                    fill_mode: FillMode::default(),
                    custom_fill_boundaries: &[],
                }),
            }
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
        self.state.active_document().is_some_and(|document| {
            document
                .instance_location(target)
                .is_some_and(|location| document.allows_edit_at(location.coord))
        })
    }

    pub fn reorder_instance(&mut self, target: PrefabInstanceId, to_top: bool) -> bool {
        let Some((environment, document)) = self.state.active_pair_mut() else {
            return false;
        };
        let Some(location) = document.instance_location(target) else {
            return false;
        };
        if !document.allows_edit_at(location.coord) {
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

        let Some((prefab, _)) = document.prefab_instance(target) else {
            return false;
        };

        let mutations = prefab
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
        let Some((prefab, _)) = document.prefab_instance(target) else {
            return false;
        };
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

    pub(super) fn commit(&mut self, action: ToolEdit, group: Option<EditGroupId>) -> bool {
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
mod tests {
    use core::{path::TreePath, types::Value};

    use dmm::{Coord, Prefab};
    use editor::{
        command::EditGroupId,
        tool::{FillMode, SelectionRotation, Tool},
    };

    use crate::session::{
        FillOutcome,
        Session,
        fixtures::{area_at, examples, flat_session, focus_session},
    };

    #[test]
    fn the_replace_tool_swaps_a_picked_atom_for_a_brush_of_the_same_kind() {
        let mut session = flat_session(2, 1);
        let document = session.state.active().unwrap();
        let coord = Coord::new(1, 1, 1);
        session
            .state
            .choose_prefab(Prefab::new(TreePath::parse("/obj/structure/table")));
        session.set_tool(Tool::Place);
        let table = session.place_at(coord, None).unwrap();
        let light = Prefab::new(TreePath::parse("/obj/machinery/light"));
        session.state.choose_prefab(light.clone());

        assert!(
            !session.replace_instance(table),
            "only the Replace tool replaces what it picks"
        );
        session.set_tool(Tool::Replace);
        assert!(!session.replace_instance(session.turf_at(document, coord).unwrap()));
        assert!(session.replace_instance(table));
        let (prefab, _) = session.state.active_document().unwrap().prefab_instance(table).unwrap();
        assert_eq!(prefab, &light, "the placement keeps its id");
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
    fn context_atom_actions_preserve_ids_and_undo() {
        let mut session = flat_session(4, 4);
        let coord = Coord::new(2, 2, 1);
        let table = TreePath::parse("/obj/structure/table");
        let light = TreePath::parse("/obj/machinery/light");
        let table_type = session.tree().unwrap().id_of(&table).unwrap();
        let light_type = session.tree().unwrap().id_of(&light).unwrap();
        assert!(session.choose_type(table_type));
        let first = session.place_at(coord, None).unwrap();
        assert!(session.choose_type(light_type));
        let second = session.place_at(coord, None).unwrap();
        assert_eq!(
            session.state.active_document().unwrap().instance_ids_at(coord)[0..2],
            [first, second]
        );

        assert!(session.reorder_instance(first, true));
        assert_eq!(
            session.state.active_document().unwrap().instance_ids_at(coord)[0..2],
            [second, first]
        );
        assert!(session.undo());
        assert_eq!(
            session.state.active_document().unwrap().instance_ids_at(coord)[0..2],
            [first, second]
        );

        session.select_instance(Some(first));
        assert_eq!(
            session.set_selected_instance_var("name".into(), Value::Text("custom table".into())),
            Some(true)
        );
        assert!(session.reset_instance_to_default(first));
        assert!(
            session
                .state
                .active_document()
                .unwrap()
                .prefab_instance(first)
                .unwrap()
                .0
                .vars
                .is_empty()
        );
        assert!(session.undo());
        assert_eq!(
            session
                .state
                .active_document()
                .unwrap()
                .prefab_instance(first)
                .unwrap()
                .0
                .var(&"name".into()),
            Some(&Value::Text("custom table".into()))
        );
        assert!(session.replace_context_instance(first, light.clone()));
        let (prefab, _) = session.state.active_document().unwrap().prefab_instance(first).unwrap();
        assert_eq!(prefab.path, light);
        assert!(prefab.vars.is_empty());
        assert!(!session.replace_context_instance(first, TreePath::parse("/turf/open/floor")));
        assert!(session.undo());
        assert_eq!(
            session
                .state
                .active_document()
                .unwrap()
                .prefab_instance(first)
                .unwrap()
                .0
                .path,
            table
        );
        assert!(session.delete_context_instance(first));
        assert!(
            session
                .state
                .active_document()
                .unwrap()
                .instance_location(first)
                .is_none()
        );
        assert!(session.undo());
        assert!(
            session
                .state
                .active_document()
                .unwrap()
                .instance_location(first)
                .is_some()
        );
    }

    #[test]
    fn context_reorder_changes_equal_layer_draw_order_through_undo_and_redo() {
        let mut session = flat_session(2, 2);
        let coord = Coord::new(1, 1, 1);
        let table = session
            .tree()
            .unwrap()
            .id_of(&TreePath::parse("/obj/structure/table"))
            .unwrap();
        assert!(session.choose_type(table));
        let first = session.place_at(coord, None).unwrap();
        let second = session.place_at(coord, None).unwrap();
        let drawn = |session: &Session| {
            session
                .instances()
                .unwrap()
                .sprites
                .iter()
                .filter(|sprite| sprite.owner == first || sprite.owner == second)
                .map(|sprite| sprite.owner)
                .collect::<Vec<_>>()
        };
        assert_eq!(drawn(&session), vec![first, second]);
        let revision = session.revision();
        assert!(session.reorder_instance(first, true));
        assert_eq!(session.revision(), revision.wrapping_add(1));
        assert_eq!(drawn(&session), vec![second, first]);
        let revision = session.revision();
        assert!(session.undo());
        assert_eq!(session.revision(), revision.wrapping_add(1));
        assert_eq!(drawn(&session), vec![first, second]);
        let revision = session.revision();
        assert!(session.redo());
        assert_eq!(session.revision(), revision.wrapping_add(1));
        assert_eq!(drawn(&session), vec![second, first]);
    }

    #[test]
    fn context_edits_obey_area_focus_without_blocking_copy() {
        let mut session = focus_session();
        let inside = Coord::new(1, 1, 1);
        let outside = Coord::new(4, 1, 1);
        let table = session
            .tree()
            .unwrap()
            .id_of(&TreePath::parse("/obj/structure/table"))
            .unwrap();
        assert!(session.choose_type(table));
        let target = session.place_at(outside, None).unwrap();
        let before = session.map().unwrap().tile_at(outside).unwrap().clone();
        session.toggle_focus_at(Some(inside));
        assert!(session.copy_tile(outside));
        assert!(!session.can_clear_tile(outside));
        assert!(!session.cut_tile(outside));
        assert!(!session.delete_tile(outside, None));
        assert!(!session.delete_context_instance(target));
        assert!(!session.reset_instance_to_default(target));
        assert!(!session.replace_context_instance(target, TreePath::parse("/obj/machinery/light")));
        assert!(!session.can_paste_clipboard(outside, SelectionRotation::Original));
        assert_eq!(session.map().unwrap().tile_at(outside), Some(&before));
    }
}
