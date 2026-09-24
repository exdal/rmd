use dmm::Coord;
use editor::{
    document::Selection,
    tool::{
        BlockSelectionMode,
        SelectionMask,
        SelectionPlacement,
        SelectionRotation,
        SelectionTransform,
        Tool,
        fill_selection as build_selection_fill,
        is_placeable,
        place_selection_with_mode as build_selection_placement,
        rotated_selection_at,
        transform_selection_with_mode as build_selection_transform,
        transformed_selection,
    },
};

use super::Session;

impl Session {
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
}

#[cfg(test)]
mod tests {
    use core::{path::TreePath, types::Value};

    use dmm::{Coord, Prefab};
    use editor::{
        document::Selection,
        tool::{BlockSelectionMode, FillMode, SelectionMask, SelectionPlacement, SelectionRotation, Tool},
    };

    use crate::session::{
        FillOutcome,
        fixtures::{assert_render_cache_matches_rebuild, flat_session, focus_session},
    };

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
}
