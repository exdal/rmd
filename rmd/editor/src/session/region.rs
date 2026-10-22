use dmm::Coord;
use editor::{
    command::EditGroupId,
    document::{DocumentId, PrefabInstanceId, Selection},
    tool::{
        RegionSelectionMode, SelectionMask, SelectionPlacement, SelectionRotation, SelectionTransform, Tool,
        fill_selection as build_selection_fill, is_placeable, place_selection_with_mode as build_selection_placement,
        rotated_selection_at, transform_selection_with_mode as build_selection_transform, transformed_selection,
    },
};

use super::Session;

#[derive(Debug, Clone, Copy)]
pub(super) struct PendingMove {
    document: DocumentId,
    source: SelectionMask,
    state: PendingMoveState,
}

#[derive(Debug, Clone, Copy)]
enum PendingMoveState {
    Unmoved,
    Moved { group: EditGroupId, target: SelectionMask },
    // a fresh paste follows by pasting again, which puts back what it covered
    Floating,
}

impl Session {
    fn live_pending_move(&self) -> Option<PendingMove> {
        let pending = self.pending_move?;
        let document = self.state.active_document()?;
        if document.id() != pending.document {
            return None;
        }

        let is_live = match pending.state {
            PendingMoveState::Unmoved => self.selection_mask() == Some(pending.source),
            PendingMoveState::Moved { group, target } => {
                document.history.undo_group() == Some(group) && self.selection_mask() == Some(target)
            },
            PendingMoveState::Floating => self.live_floating_paste().is_some(),
        };

        is_live.then_some(pending)
    }

    pub(crate) fn pending_move_source(&self) -> Option<Selection> {
        self.live_pending_move().map(|pending| pending.source.bounds)
    }

    pub fn move_selected_region_live(&mut self, target_min: Coord) -> bool {
        let pending = if self.pending_move.is_some() {
            let Some(pending) = self.live_pending_move() else {
                return false;
            };

            pending
        } else {
            let (Some(document), Some(source)) = (self.state.active(), self.selection_mask()) else {
                return false;
            };

            let state = if self.live_floating_paste().is_some() {
                PendingMoveState::Floating
            } else {
                PendingMoveState::Unmoved
            };
            let pending = PendingMove {
                document,
                source,
                state,
            };
            self.pending_move = Some(pending);
            self.hold_bake(document);

            pending
        };

        if self.selection().is_some_and(|selection| selection.min == target_min) {
            return true;
        }

        match pending.state {
            PendingMoveState::Unmoved => self.commit_pending_move(pending, target_min, &[]),
            PendingMoveState::Moved { group, .. } => {
                let is_back = target_min == pending.source.bounds.min;
                if !is_back
                    && !self.can_place_region_with_mode(
                        pending.source.bounds,
                        target_min,
                        SelectionRotation::Original,
                        pending.source.mode,
                    )
                {
                    return false;
                }

                // moving from the untouched map puts back whatever the region passed over
                let Some(retracted) = self
                    .state
                    .active_document_mut()
                    .and_then(|document| document.retract_with_affected(group))
                else {
                    return false;
                };

                if !is_back && self.commit_pending_move(pending, target_min, &retracted) {
                    return true;
                }

                self.pending_move = Some(PendingMove {
                    state: PendingMoveState::Unmoved,
                    ..pending
                });
                self.settle_retracted(&retracted);

                is_back
            },
            PendingMoveState::Floating => {
                let Some(rotation) = self.live_floating_paste().map(|floating| floating.rotation) else {
                    return false;
                };

                if !self.can_place_selected_region_with_mode(
                    target_min,
                    SelectionRotation::Original,
                    SelectionPlacement::Move,
                    pending.source.mode,
                ) {
                    return false;
                }

                let Some(floating) = self.floating_paste.take() else {
                    return false;
                };

                self.store_pasted_floating(floating, target_min, rotation)
            },
        }
    }

    fn commit_pending_move(&mut self, pending: PendingMove, target_min: Coord, retracted: &[PrefabInstanceId]) -> bool {
        let mode = pending.source.mode;
        if !self.can_place_selected_region_with_mode(
            target_min,
            SelectionRotation::Original,
            SelectionPlacement::Move,
            mode,
        ) {
            return false;
        }

        let group = EditGroupId::new();
        if !self.commit_region_placement(
            target_min,
            SelectionRotation::Original,
            SelectionPlacement::Move,
            mode,
            Some(group),
            retracted,
        ) {
            return false;
        }

        let Some(target) = self.selection_mask() else {
            return false;
        };

        self.pending_move = Some(PendingMove {
            state: PendingMoveState::Moved { group, target },
            ..pending
        });

        true
    }

    fn settle_retracted(&mut self, retracted: &[PrefabInstanceId]) {
        self.update_instances(retracted);
        if let Some(id) = self.state.active()
            && let Some(cache) = self.caches.get_mut(&id)
        {
            cache.map_revision = cache.map_revision.wrapping_add(1);
        }
    }

    pub fn finish_live_move(&mut self) {
        if let Some(pending) = self.pending_move.take() {
            self.release_bake(pending.document);
        }
    }

    pub fn cancel_live_move(&mut self) {
        let pending = self.live_pending_move();
        let Some(document) = self.pending_move.take().map(|pending_move| pending_move.document) else {
            return;
        };

        if let Some(pending) = pending {
            match pending.state {
                PendingMoveState::Unmoved => {},
                PendingMoveState::Moved { group, .. } => {
                    if let Some(retracted) = self
                        .state
                        .active_document_mut()
                        .and_then(|document| document.retract_with_affected(group))
                    {
                        self.settle_retracted(&retracted);
                    }
                },
                PendingMoveState::Floating => {
                    if let Some(floating) = self.live_floating_paste()
                        && floating.target.bounds.min != pending.source.bounds.min
                        && let Some(floating) = self.floating_paste.take()
                    {
                        let rotation = floating.rotation;
                        self.store_pasted_floating(floating, pending.source.bounds.min, rotation);
                    }
                },
            }
        }

        self.release_bake(document);
    }

    pub fn selection(&self) -> Option<Selection> {
        self.selection_mask().map(|mask| mask.bounds)
    }

    pub fn selection_mask(&self) -> Option<SelectionMask> {
        self.state.active_document()?.selection_mask()
    }

    pub fn selection_mode(&self) -> RegionSelectionMode {
        self.selection_mask()
            .map_or(RegionSelectionMode::Full, |mask| mask.mode)
    }

    pub(crate) fn edit_revision(&self) -> Option<u64> {
        Some(self.caches.get(&self.state.active()?)?.revision)
    }

    pub fn can_select_region(&self, mask: SelectionMask) -> bool {
        self.state.active_document().is_some_and(|document| {
            mask.bounds.is_well_formed()
                && mask.bounds.min.z == document.z
                && mask.bounds.max.x <= document.map.size().x
                && mask.bounds.max.y <= document.map.size().y
                && mask
                    .mode
                    .tiles(mask.bounds)
                    .all(|coord| document.allows_select_at(coord))
        })
    }

    pub fn try_select_region(&mut self, mask: SelectionMask) -> bool {
        self.can_select_region(mask) && self.select_region_with_mode(Some(mask.bounds), mask.mode)
    }

    pub fn select_region(&mut self, selection: Option<Selection>) -> bool {
        self.select_region_with_mode(selection, RegionSelectionMode::Full)
    }

    pub fn select_region_with_mode(
        &mut self, selection: Option<Selection>, selection_mode: RegionSelectionMode,
    ) -> bool {
        let Some(document) = self.state.active_document_mut() else {
            return false;
        };

        let selection = selection.filter(|selection| {
            selection.is_well_formed()
                && selection.min.z == document.z
                && selection.max.x <= document.map.size().x
                && selection.max.y <= document.map.size().y
                && selection_mode
                    .tiles(*selection)
                    .all(|coord| document.allows_select_at(coord))
        });
        document.set_selection(selection, selection_mode);
        document.pick_instance(None);
        self.floating_paste = None;

        selection.is_some()
    }

    pub fn place_selected_region_with_mode(
        &mut self, target_min: Coord, rotation: SelectionRotation, placement: SelectionPlacement,
        selection_mode: RegionSelectionMode,
    ) -> bool {
        if !self.can_place_selected_region_with_mode(target_min, rotation, placement, selection_mode) {
            return false;
        }

        if placement == SelectionPlacement::Move
            && rotation == SelectionRotation::Original
            && let Some(floating) = self.live_floating_paste()
            && floating.target.mode == selection_mode
            && let Some(floating) = self.floating_paste.take()
        {
            let rotation = floating.rotation;

            return self.store_pasted_floating(floating, target_min, rotation);
        }

        self.commit_region_placement(target_min, rotation, placement, selection_mode, None, &[])
    }

    fn commit_region_placement(
        &mut self, target_min: Coord, rotation: SelectionRotation, placement: SelectionPlacement,
        selection_mode: RegionSelectionMode, group: Option<EditGroupId>, retracted: &[PrefabInstanceId],
    ) -> bool {
        let hidden = self.hidden_types();
        let built = {
            let Some((environment, document)) = self.state.active_pair_mut() else {
                return false;
            };
            let Some(selection) = document.selection() else {
                return false;
            };

            build_selection_placement(
                document,
                &environment.tree,
                *selection,
                target_min,
                rotation,
                placement,
                selection_mode,
                &hidden,
            )
        };

        let Some((mut action, selection)) = built else {
            return false;
        };

        if !retracted.is_empty() {
            action.affected.extend_from_slice(retracted);
            action.affected.sort_unstable();
            action.affected.dedup();
        }

        self.commit_selecting(
            action,
            SelectionMask {
                bounds: selection,
                mode: selection_mode,
            },
            group,
        )
    }

    pub fn can_place_selected_region_with_mode(
        &self, target_min: Coord, rotation: SelectionRotation, _placement: SelectionPlacement,
        mode: RegionSelectionMode,
    ) -> bool {
        self.selection()
            .is_some_and(|selection| self.can_place_region_with_mode(selection, target_min, rotation, mode))
    }

    fn can_place_region_with_mode(
        &self, selection: Selection, target_min: Coord, rotation: SelectionRotation, mode: RegionSelectionMode,
    ) -> bool {
        if self.tool() != Tool::Select {
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

        let Some(target) = rotated_selection_at(selection, target_min, rotation) else {
            return false;
        };

        if target == selection && rotation == SelectionRotation::Original {
            // why do you want to select and place? get help
            return false;
        }

        target.max.x <= document.map.size().x
            && target.max.y <= document.map.size().y
            && mode
                .tiles(selection)
                .chain(mode.tiles(target))
                .all(|coord| document.allows_edit_at(coord))
    }

    pub fn fill_rect(&mut self, mask: SelectionMask) -> bool {
        if !self.can_fill_rect(mask) {
            return false;
        }

        let Some(prefab) = self.state.palette.clone() else {
            return false;
        };
        let built = {
            let Some((environment, document)) = self.state.active_pair_mut() else {
                return false;
            };

            build_selection_fill(document, &environment.tree, mask.bounds, &prefab, mask.mode)
        };

        let Some(action) = built else {
            return false;
        };

        if !self.commit(action, None) {
            return false;
        }

        self.state.choose_prefab(prefab);

        true
    }

    pub fn can_fill_rect(&self, mask: SelectionMask) -> bool {
        let Some(environment) = self.state.environment.as_ref() else {
            return false;
        };
        let Some(prefab) = self.state.palette.as_ref() else {
            return false;
        };
        let Some(prefab_id) = environment.tree.id_of(&prefab.path) else {
            return false;
        };

        is_placeable(&environment.tree, prefab_id) && self.can_select_region(mask)
    }

    pub fn rotate_selected_region(&mut self) -> bool {
        self.transform_selected_region_with_mode(SelectionTransform::RotateClockwise, self.selection_mode())
    }

    pub fn transform_selected_region_with_mode(
        &mut self, transform: SelectionTransform, selection_mode: RegionSelectionMode,
    ) -> bool {
        if self.tool() != Tool::Select || !self.can_transform_selected_region_with_mode(transform, selection_mode) {
            return false;
        }

        if let Some(floating) = self.live_floating_paste()
            && floating.target.mode == selection_mode
        {
            let rotation = match transform {
                SelectionTransform::RotateClockwise => Some(floating.rotation.clockwise()),
                SelectionTransform::RotateCounterClockwise => Some(floating.rotation.counter_clockwise()),
                SelectionTransform::MirrorHorizontal | SelectionTransform::MirrorVertical => None,
            };
            if let Some(rotation) = rotation
                && let Some(floating) = self.floating_paste.take()
            {
                let min = floating.target.bounds.min;

                return self.store_pasted_floating(floating, min, rotation);
            }
        }

        let hidden = self.hidden_types();
        let built = {
            let Some((environment, document)) = self.state.active_pair_mut() else {
                return false;
            };
            let Some(selection) = document.selection().copied() else {
                return false;
            };

            build_selection_transform(
                document,
                &environment.tree,
                selection,
                transform,
                selection_mode,
                &hidden,
            )
        };

        let Some((action, selection)) = built else {
            return false;
        };

        self.commit_selecting(
            action,
            SelectionMask {
                bounds: selection,
                mode: selection_mode,
            },
            None,
        )
    }

    pub fn can_transform_selected_region_with_mode(
        &self, transform: SelectionTransform, mode: RegionSelectionMode,
    ) -> bool {
        // holy fuck we need a better solution to this, just copy pasting  same shit over and over

        if self.tool() != Tool::Select {
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

        target.max.x <= document.map.size().x
            && target.max.y <= document.map.size().y
            && mode
                .tiles(selection)
                .chain(mode.tiles(target))
                .all(|coord| document.allows_edit_at(coord))
    }
}

#[cfg(test)]
mod tests {
    use core::{path::TreePath, types::Value};

    use dmm::{Coord, Prefab};
    use editor::{
        document::{PrefabInstanceId, Selection},
        tool::{
            FillMode, RegionSelectionMode, SelectionMask, SelectionPlacement, SelectionRotation, SelectionTransform,
            Tool,
        },
    };

    use vm::world::Position;

    use crate::session::{
        BakeHold, FillOutcome, RegionPreviewSource, Session,
        fixtures::{assert_render_cache_matches_rebuild, examples, flat_session, focus_session, settle_bake},
    };

    #[test]
    fn a_region_move_leaves_areas_while_they_are_not_shown() {
        let mut session = flat_session(3, 1);
        let source = Coord::new(1, 1, 1);
        let destination = Coord::new(3, 1, 1);
        let mut engineering = Prefab::new(TreePath::parse("/area/station"));
        engineering.set_var("name".into(), Value::Text("Engineering".into()));
        for (prefab, coord) in [
            (Prefab::new(TreePath::parse("/obj/structure/table")), source),
            (engineering.clone(), destination),
        ] {
            session.state.choose_prefab(prefab);
            session.set_tool(Tool::Place);
            assert!(session.place_at(coord, None).is_some());
        }
        let area_at = |session: &Session, coord| {
            session
                .state
                .active_document()
                .unwrap()
                .placed_tile(coord)
                .unwrap()
                .last()
                .cloned()
                .unwrap()
        };
        let source_area = area_at(&session, source);
        let destination_area = area_at(&session, destination);

        session.set_tool(Tool::Select);
        assert!(session.select_region(Some(Selection::from_drag(source, source))));
        assert!(session.place_selected_region_with_mode(
            destination,
            SelectionRotation::Original,
            SelectionPlacement::Move,
            RegionSelectionMode::Full,
        ));

        assert_eq!(area_at(&session, source), source_area);
        assert_eq!(area_at(&session, destination), destination_area);
        assert_eq!(destination_area.prefab(), &engineering);
        assert_eq!(
            session.map().unwrap().tile_at(destination).unwrap()[0].path,
            TreePath::parse("/obj/structure/table")
        );
        assert_render_cache_matches_rebuild(&session);
    }

    #[test]
    fn region_selection_clears_object_selection_and_respects_focus() {
        let mut session = focus_session();
        let inside = Coord::new(1, 1, 1);
        let outside = Coord::new(3, 1, 1);
        let object = session.state.active_document().unwrap().instance_ids_at(inside)[0];
        session.pick_instance(Some(object));
        session.set_tool(Tool::Select);
        assert_eq!(session.picked_instance(), None);
        session.toggle_focus_at(Some(inside));

        assert!(!session.select_region(Some(Selection::from_drag(inside, outside))));
        assert_eq!(session.selection(), None);
        assert_eq!(session.picked_instance(), None);
        assert!(session.select_region(Some(Selection::from_drag(inside, Coord::new(2, 1, 1)))));
        assert_eq!(
            session.selection(),
            Some(Selection::from_drag(inside, Coord::new(2, 1, 1)))
        );
    }

    #[test]
    fn resizing_selection_preserves_contents_history_and_the_last_valid_region() {
        let mut session = focus_session();
        session.set_tool(Tool::Select);
        let mask = SelectionMask {
            bounds: Selection::from_drag(Coord::new(1, 1, 1), Coord::new(1, 1, 1)),
            mode: RegionSelectionMode::Hollow { line_width: 1 },
        };
        assert!(session.try_select_region(mask));
        session.toggle_focus_at(Some(mask.bounds.min));
        let revision = session.revision();
        let tiles = session.map().unwrap().clone();
        let expanded = SelectionMask {
            bounds: Selection::from_drag(mask.bounds.min, Coord::new(2, 1, 1)),
            ..mask
        };
        assert!(session.try_select_region(expanded));
        assert!(!session.try_select_region(SelectionMask {
            bounds: Selection::from_drag(mask.bounds.min, Coord::new(3, 1, 1)),
            ..mask
        }));
        assert_eq!(session.selection_mask(), Some(expanded));
        assert_eq!(session.revision(), revision);
        assert_eq!(*session.map().unwrap(), tiles);
        assert!(!session.undo());
    }

    #[test]
    fn rectangle_fill_and_bucket_share_the_mask_across_tool_switches_and_undo() {
        for mode in [RegionSelectionMode::Full, RegionSelectionMode::Hollow { line_width: 1 }] {
            let mut session = flat_session(7, 7);
            let mask = SelectionMask {
                bounds: Selection::from_drag(Coord::new(2, 2, 1), Coord::new(6, 6, 1)),
                mode,
            };
            session.set_tool(Tool::Select);
            assert!(session.try_select_region(mask));
            let mut red = Prefab::new(TreePath::parse("/turf/open/floor"));
            red.set_var("color".into(), Value::Text("#ff0000".into()));
            session.state.choose_prefab(red.clone());
            assert!(session.fill_rect(mask));
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
            session.set_tool(Tool::Select);
            assert_eq!(session.selection_mask(), Some(mask));
            assert_render_cache_matches_rebuild(&session);
        }
    }

    #[test]
    fn one_tile_region_placements_update_multi_level_render_caches() {
        for placement in [SelectionPlacement::Move, SelectionPlacement::Copy] {
            let mut session = focus_session();
            let source = Coord::new(1, 1, 1);
            let destination = Coord::new(2, 1, 1);
            let untouched = Coord::new(1, 1, 5);
            let untouched_before = session.map().unwrap().tile_at(untouched).unwrap().clone();
            session.set_tool(Tool::Select);
            assert!(session.select_region(Some(Selection::from_drag(source, source))));
            let revision = session.revision();

            assert!(session.place_selected_region_with_mode(
                destination,
                SelectionRotation::Original,
                placement,
                RegionSelectionMode::Full,
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
    fn rect_fill_commits_the_palette_across_the_rect_and_keeps_the_selection() {
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
        session.set_tool(Tool::Select);
        assert!(session.select_region(Some(Selection::from_drag(source, source))));
        let revision = session.revision();

        let mask = |coord| SelectionMask {
            bounds: Selection::from_drag(coord, coord),
            mode: RegionSelectionMode::Full,
        };
        assert!(session.can_fill_rect(mask(source)));
        assert!(session.can_fill_rect(mask(destination)));
        assert!(session.fill_rect(mask(destination)));

        assert_eq!(session.selection(), Some(Selection::from_drag(source, source)));
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
    fn hollow_rect_fill_passes_the_border_mode_through_the_session() {
        let mut session = flat_session(5, 5);

        let table = session
            .tree()
            .unwrap()
            .id_of(&TreePath::parse("/obj/structure/table"))
            .unwrap();
        assert!(session.choose_type(table));
        let prefab = session.palette().unwrap().clone();
        let revision = session.revision();

        assert!(session.fill_rect(SelectionMask {
            bounds: Selection::from_drag(Coord::new(1, 1, 1), Coord::new(5, 5, 1)),
            mode: RegionSelectionMode::Hollow { line_width: 1 },
        }));

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
    fn hollow_region_moves_pass_the_border_mode_through_the_session() {
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
        let mode = RegionSelectionMode::Hollow { line_width: 1 };
        session.set_tool(Tool::Select);
        assert!(session.select_region_with_mode(Some(selection), mode));
        let revision = session.revision();

        assert!(session.place_selected_region_with_mode(
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
    fn undo_and_redo_move_the_selection_with_region_edits() {
        let mut session = flat_session(8, 4);
        session.set_tool(Tool::Select);
        let source = Selection::from_drag(Coord::new(1, 1, 1), Coord::new(2, 1, 1));
        let border = RegionSelectionMode::Hollow { line_width: 1 };
        assert!(session.select_region_with_mode(Some(source), border));
        let moved = Selection::from_drag(Coord::new(4, 2, 1), Coord::new(5, 2, 1));
        assert!(session.place_selected_region_with_mode(
            moved.min,
            SelectionRotation::Original,
            SelectionPlacement::Move,
            border,
        ));
        assert!(session.rotate_selected_region());
        let rotated = session.selection().unwrap();
        assert_ne!(rotated, moved);

        assert!(session.undo());
        assert_eq!(session.selection(), Some(moved));
        assert!(session.undo());
        assert_eq!(
            session.selection_mask(),
            Some(SelectionMask {
                bounds: source,
                mode: border,
            }),
        );
        assert!(session.redo());
        assert_eq!(session.selection(), Some(moved));
        assert!(session.redo());
        assert_eq!(session.selection(), Some(rotated));

        assert!(session.copy_selection(RegionSelectionMode::Full));
        session.select_region(None);
        let pasted = Selection::from_drag(Coord::new(7, 3, 1), Coord::new(7, 4, 1));
        assert!(session.paste_clipboard(pasted.min, SelectionRotation::Original));
        assert_eq!(session.selection(), Some(pasted));
        assert!(session.undo());
        assert_eq!(session.selection(), None);
        assert!(session.redo());
        assert_eq!(session.selection(), Some(pasted));
    }

    #[test]
    fn hollow_region_previews_ghost_only_the_ring_tiles() {
        let session = flat_session(5, 5);
        let selection = Selection::from_drag(Coord::new(1, 1, 1), Coord::new(5, 5, 1));

        let full = session.region_preview_sprites(
            selection,
            selection,
            SelectionRotation::Original,
            RegionSelectionMode::Full,
        );
        let hollow = session.region_preview_sprites(
            selection,
            selection,
            SelectionRotation::Original,
            RegionSelectionMode::Hollow { line_width: 1 },
        );

        // every tile of the flat map ghosts the same sprites, so the ring is 16/25ths of the whole region
        assert!(!full.is_empty());
        assert_eq!(full.len() % 25, 0);
        assert_eq!(hollow.len(), full.len() / 25 * 16);
    }

    fn created_level_session() -> (Session, PrefabInstanceId) {
        let mut session = flat_session(4, 4);
        let id = session.state.active().unwrap();
        let fill = session.tile_fill("/turf/open/floor", "/area/station").unwrap();
        assert_eq!(session.create_level(id, &fill), Ok(2));
        let table = session
            .tree()
            .unwrap()
            .id_of(&TreePath::parse("/obj/structure/table"))
            .unwrap();
        assert!(session.choose_type(table));
        session.set_tool(Tool::Place);
        let placed = session.place_at(Coord::new(1, 1, 2), None).unwrap();
        session.set_tool(Tool::Select);

        (session, placed)
    }

    #[test]
    fn a_region_on_a_created_level_previews_its_ghost() {
        let (mut session, _) = created_level_session();
        let id = session.state.active().unwrap();
        let selection = Selection::from_drag(Coord::new(1, 1, 2), Coord::new(2, 2, 2));
        assert!(session.select_region(Some(selection)));

        session.prepare_region_preview(
            id,
            RegionPreviewSource::Selection(SelectionMask {
                bounds: selection,
                mode: RegionSelectionMode::Full,
            }),
            Coord::new(3, 3, 2),
            SelectionRotation::Clockwise,
        );

        let preview = session.active_cache().preview.as_ref().unwrap();
        assert!(!preview.sprites.is_empty());
        assert_eq!(preview.destination, Coord::new(3, 3, 2));
    }

    #[test]
    fn a_region_on_a_lower_level_of_a_loaded_map_previews_its_ghost() {
        let mut session = focus_session();
        let id = session.state.active().unwrap();
        session.set_level(3);
        session.set_tool(Tool::Select);
        let selection = Selection::from_drag(Coord::new(1, 1, 3), Coord::new(2, 1, 3));
        assert!(session.select_region(Some(selection)));

        session.prepare_region_preview(
            id,
            RegionPreviewSource::Selection(SelectionMask {
                bounds: selection,
                mode: RegionSelectionMode::Full,
            }),
            Coord::new(3, 1, 3),
            SelectionRotation::Original,
        );

        assert!(!session.active_cache().preview.as_ref().unwrap().sprites.is_empty());
    }

    #[test]
    fn a_region_on_a_created_level_moves_rotates_and_cuts() {
        let (mut session, table) = created_level_session();
        let location = |session: &Session| {
            session
                .state
                .active_document()
                .unwrap()
                .instance_location(table)
                .map(|location| location.coord)
        };
        assert!(session.select_region(Some(Selection::from_drag(Coord::new(1, 1, 2), Coord::new(2, 1, 2)))));

        assert!(session.place_selected_region_with_mode(
            Coord::new(2, 2, 2),
            SelectionRotation::Original,
            SelectionPlacement::Move,
            RegionSelectionMode::Full,
        ));
        assert_eq!(location(&session), Some(Coord::new(2, 2, 2)));
        assert_render_cache_matches_rebuild(&session);

        assert!(
            session.transform_selected_region_with_mode(SelectionTransform::RotateClockwise, RegionSelectionMode::Full)
        );
        assert_eq!(location(&session).map(|coord| coord.z), Some(2));
        assert_render_cache_matches_rebuild(&session);

        assert!(session.cut_selection());
        assert_eq!(location(&session), None);
        assert!(session.clipboard().is_some());
        assert_render_cache_matches_rebuild(&session);
    }

    fn table_beside_a_wall(wall: Coord) -> (Session, PrefabInstanceId) {
        let mut session = flat_session(6, 1);
        session
            .state
            .choose_prefab(Prefab::new(TreePath::parse("/turf/closed/wall")));
        session.set_tool(Tool::Place);
        assert!(session.place_at(wall, None).is_some());
        session
            .state
            .choose_prefab(Prefab::new(TreePath::parse("/obj/structure/table")));
        let table = session.place_at(Coord::new(1, 1, 1), None).unwrap();
        session.set_tool(Tool::Select);
        assert!(session.select_region(Some(Selection::from_drag(Coord::new(1, 1, 1), Coord::new(1, 1, 1)))));

        (session, table)
    }

    fn undo_depth(session: &Session) -> usize {
        session.state.active_document().unwrap().history.undo_depth()
    }

    #[test]
    fn a_live_region_move_clears_the_source_at_once_and_puts_back_what_it_passes_over() {
        let wall = Coord::new(3, 1, 1);
        let (mut session, table) = table_beside_a_wall(wall);
        let before = session.map().unwrap().clone();
        let covered = session.state.active_document().unwrap().placed_tile(wall).unwrap();
        let depth = undo_depth(&session);
        let location = |session: &Session| {
            session
                .state
                .active_document()
                .unwrap()
                .instance_location(table)
                .map(|location| location.coord)
        };

        assert!(session.move_selected_region_live(Coord::new(2, 1, 1)));
        assert_eq!(location(&session), Some(Coord::new(2, 1, 1)));
        assert!(session.move_selected_region_live(wall));
        assert_eq!(location(&session), Some(wall));
        assert!(session.move_selected_region_live(Coord::new(5, 1, 1)));
        session.finish_live_move();

        let document = session.state.active_document().unwrap();
        assert_eq!(location(&session), Some(Coord::new(5, 1, 1)));
        assert_eq!(document.placed_tile(wall).unwrap(), covered, "the wall keeps its ids");
        assert_eq!(
            session.selection(),
            Some(Selection::from_drag(Coord::new(5, 1, 1), Coord::new(5, 1, 1)))
        );
        assert_eq!(undo_depth(&session), depth + 1);
        assert_eq!(session.undo_label(), Some("move region"));
        assert_eq!(session.redo_label(), None);
        assert_render_cache_matches_rebuild(&session);

        assert!(session.undo());
        assert_eq!(*session.map().unwrap(), before);
        assert_eq!(location(&session), Some(Coord::new(1, 1, 1)));
        assert!(session.redo());
        assert_eq!(location(&session), Some(Coord::new(5, 1, 1)));
    }

    #[test]
    fn a_live_region_move_cancelled_or_dragged_back_keeps_the_redo_from_before() {
        let (mut session, _) = table_beside_a_wall(Coord::new(3, 1, 1));
        assert!(session.move_selected_region_live(Coord::new(2, 1, 1)));
        session.finish_live_move();
        let moved = session.map().unwrap().clone();
        assert!(session.undo());

        assert!(session.move_selected_region_live(Coord::new(4, 1, 1)));
        session.cancel_live_move();
        assert_eq!(session.redo_label(), Some("move region"));

        assert!(session.move_selected_region_live(Coord::new(4, 1, 1)));
        assert!(session.move_selected_region_live(Coord::new(1, 1, 1)));
        session.finish_live_move();
        assert_eq!(session.redo_label(), Some("move region"));

        assert!(session.redo());
        assert_eq!(*session.map().unwrap(), moved);
        assert_render_cache_matches_rebuild(&session);
    }

    #[test]
    fn a_live_region_move_back_to_its_start_or_cancelled_leaves_no_edit() {
        let (mut session, _) = table_beside_a_wall(Coord::new(3, 1, 1));
        let source = session.selection();
        let before = session.map().unwrap().clone();
        let depth = undo_depth(&session);

        assert!(session.move_selected_region_live(Coord::new(4, 1, 1)));
        assert!(session.move_selected_region_live(Coord::new(1, 1, 1)));
        assert_eq!(*session.map().unwrap(), before);
        assert_eq!(undo_depth(&session), depth);
        assert_eq!(session.redo_label(), None, "a move dragged back leaves nothing to redo");
        assert!(session.move_selected_region_live(Coord::new(4, 1, 1)));
        assert!(
            !session.move_selected_region_live(Coord::new(7, 1, 1)),
            "a target off the map keeps the last one"
        );
        assert_eq!(
            session.selection(),
            Some(Selection::from_drag(Coord::new(4, 1, 1), Coord::new(4, 1, 1)))
        );
        session.cancel_live_move();

        assert_eq!(*session.map().unwrap(), before);
        assert_eq!(session.selection(), source);
        assert_eq!(undo_depth(&session), depth);
        assert_eq!(session.redo_label(), None, "a cancelled move leaves nothing to redo");
        assert_render_cache_matches_rebuild(&session);
    }

    fn baked_table_on_the_example_map() -> (Session, PrefabInstanceId, Coord) {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).expect("codebase");
        session.open_map(&root.join("test.dmm"), 1).expect("map");
        settle_bake(&mut session);
        let source = Coord::new(3, 4, 1);
        let table = session
            .state
            .active_document()
            .and_then(|document| document.placed_tile(source))
            .and_then(|tile| {
                tile.iter()
                    .find(|placed| placed.prefab().path == TreePath::parse("/obj/structure/table"))
                    .map(|placed| placed.id())
            })
            .expect("the example map has a table");
        session.set_tool(Tool::Select);
        assert!(session.select_region(Some(Selection::from_drag(source, source))));

        (session, table, source)
    }

    fn baked_position(session: &Session, id: PrefabInstanceId) -> Option<Position> {
        session.active_cache().bake.as_ref()?.position(id.get())
    }

    fn position(coord: Coord) -> Option<Position> {
        Some(Position::new(coord.x as i32, coord.y as i32, coord.z as i32))
    }

    fn is_bake_held(session: &Session) -> bool {
        matches!(session.active_cache().bake_hold, BakeHold::Everything { .. })
    }

    #[test]
    fn a_live_region_move_bakes_once_when_it_is_placed() {
        let (mut session, table, source) = baked_table_on_the_example_map();
        let target = Coord::new(2, 3, 1);
        assert_eq!(baked_position(&session, table), position(source));

        // the gizmo holds module settling every frame it is dragged
        session.set_module_bake_held(true);
        assert!(session.move_selected_region_live(Coord::new(3, 3, 1)));
        session.set_module_bake_held(true);
        assert!(session.move_selected_region_live(target));
        session.set_module_bake_held(true);

        assert!(is_bake_held(&session));
        assert_eq!(
            baked_position(&session, table),
            position(source),
            "the bake waits for the drop"
        );
        assert_render_cache_matches_rebuild(&session);

        session.finish_live_move();
        session.set_module_bake_held(false);

        assert!(!is_bake_held(&session));
        assert!(!session.is_module_bake_held());
        assert_eq!(baked_position(&session, table), position(target));
        assert_render_cache_matches_rebuild(&session);

        assert!(session.move_selected_region_live(source));
        assert_eq!(
            baked_position(&session, table),
            position(target),
            "a later move holds the bake again"
        );
        session.finish_live_move();
        assert_eq!(baked_position(&session, table), position(source));
    }

    #[test]
    fn a_cancelled_live_region_move_bakes_the_restored_map() {
        let (mut session, table, source) = baked_table_on_the_example_map();
        let before = session.map().unwrap().clone();

        assert!(session.move_selected_region_live(Coord::new(3, 3, 1)));
        assert!(session.move_selected_region_live(Coord::new(2, 3, 1)));
        session.cancel_live_move();

        assert!(!is_bake_held(&session));
        assert_eq!(*session.map().unwrap(), before);
        assert_eq!(baked_position(&session, table), position(source));
        assert_render_cache_matches_rebuild(&session);

        session.cancel_live_move();
        assert!(!is_bake_held(&session), "an idle cancel leaves the bake alone");
    }
}
