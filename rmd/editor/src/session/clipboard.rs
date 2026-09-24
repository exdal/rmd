use dmm::Coord;
use editor::{
    clipboard::{self, TileBlock},
    document::Selection,
    frame::HiddenTypes,
    tool::{BlockSelectionMode, SelectionMask, SelectionRotation, ToolEdit},
};

use super::Session;

impl Session {
    fn hidden_types(&self) -> HiddenTypes {
        self.tree()
            .map(|tree| self.type_visibility.hidden_types(tree))
            .unwrap_or_default()
    }

    pub fn copy_selection(&mut self, mode: BlockSelectionMode) -> bool {
        let Some(selection) = self.selection() else {
            return false;
        };

        self.copy_block(selection, mode)
    }

    pub fn copy_tile(&mut self, coord: Coord) -> bool {
        self.copy_block(Selection::from_drag(coord, coord), BlockSelectionMode::Full)
    }

    fn copy_block(&mut self, selection: Selection, mode: BlockSelectionMode) -> bool {
        let hidden = self.hidden_types();
        let Some(block) = self
            .state
            .active_document()
            .and_then(|document| clipboard::copy_block(document, selection, mode, hidden))
        else {
            return false;
        };
        self.state.set_clipboard(block);

        true
    }

    pub fn can_clear_tile(&self, coord: Coord) -> bool {
        let Some((environment, document)) = self.state.active_pair() else {
            return false;
        };

        document.allows_edit_at(coord)
            && document
                .map
                .tile_at(coord)
                .is_some_and(|tile| clipboard::can_clear(&environment.tree, tile, &self.hidden_types()))
    }

    fn build_clear(&mut self, selection: Selection, mode: BlockSelectionMode, label: &str) -> Option<ToolEdit> {
        let hidden = self.hidden_types();
        let (environment, document) = self.state.active_pair_mut()?;

        clipboard::clear_block(document, &environment.tree, selection, mode, &hidden, label)
    }

    fn cut_block(&mut self, selection: Selection, mode: BlockSelectionMode, label: &str) -> bool {
        let Some(action) = self.build_clear(selection, mode, label) else {
            return false;
        };
        let hidden = self.hidden_types();
        let Some(block) = self
            .state
            .active_document()
            .and_then(|document| clipboard::copy_block(document, selection, mode, hidden))
        else {
            return false;
        };

        if !self.commit(action, None) {
            return false;
        }

        self.state.set_clipboard(block);

        true
    }

    pub fn delete_tile(&mut self, coord: Coord) -> bool {
        self.build_clear(
            Selection::from_drag(coord, coord),
            BlockSelectionMode::Full,
            "delete tile",
        )
        .is_some_and(|action| self.commit(action, None))
    }

    pub fn cut_tile(&mut self, coord: Coord) -> bool {
        self.cut_block(Selection::from_drag(coord, coord), BlockSelectionMode::Full, "cut tile")
    }

    pub fn delete_selection(&mut self) -> bool {
        let Some(selection) = self.selection() else {
            return false;
        };

        self.build_clear(selection, self.selection_mode(), "delete block")
            .is_some_and(|action| self.commit(action, None))
    }

    pub fn cut_selection(&mut self) -> bool {
        let Some(selection) = self.selection() else {
            return false;
        };

        self.cut_block(selection, self.selection_mode(), "cut block")
    }

    pub fn clipboard(&self) -> Option<&TileBlock> { self.state.clipboard() }

    pub fn clipboard_footprint(&self, min: Coord, rotation: SelectionRotation) -> Option<Selection> {
        self.state.clipboard()?.footprint(min, rotation)
    }

    pub fn clipboard_selection_mask(&self, min: Coord, rotation: SelectionRotation) -> Option<SelectionMask> {
        let block = self.state.clipboard()?;
        Some(SelectionMask {
            bounds: self.clipboard_footprint(min, rotation)?,
            mode: block.selection_mode(),
        })
    }

    pub fn can_paste_clipboard(&self, min: Coord, rotation: SelectionRotation) -> bool {
        let Some(block) = self.state.clipboard() else {
            return false;
        };
        let Some(document) = self.state.active_document() else {
            return false;
        };

        clipboard::can_paste(document, block, min, rotation)
    }

    pub fn paste_clipboard(&mut self, min: Coord, rotation: SelectionRotation) -> bool {
        let built = {
            let Some((environment, document, block)) = self.state.active_paste_mut() else {
                return false;
            };

            clipboard::paste_block(document, &environment.tree, block, min, rotation)
                .map(|built| (built, block.selection_mode()))
        };

        let Some(((action, target), mode)) = built else {
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

        true
    }
}

#[cfg(test)]
mod tests {
    use core::{path::TreePath, types::Value};

    use dmm::{Coord, Map, Prefab, Size};
    use editor::{
        document::{MapDocument, PlacedPrefab, Selection},
        tool::{BlockSelectionMode, SelectionMask, SelectionRotation, Tool},
    };

    use crate::session::{
        Session,
        fixtures::{assert_render_cache_matches_rebuild, examples, flat_session},
    };

    #[test]
    fn a_block_copied_from_one_map_pastes_into_another() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).expect("codebase");
        session.open_map(&root.join("test.dmm"), 1).expect("first map");
        let first = session.state.active().expect("first is active");

        session.set_tool(Tool::BlockSelect);
        session.choose_type(
            session
                .tree()
                .and_then(|tree| tree.id_of(&TreePath::parse("/obj/structure/table")))
                .expect("a placeable type"),
        );
        session.set_tool(Tool::Place);
        assert!(
            session.place_at(Coord::new(1, 1, 1), None).is_some(),
            "something to copy"
        );
        session.set_tool(Tool::BlockSelect);

        let source = Selection::from_drag(Coord::new(1, 1, 1), Coord::new(2, 2, 1));
        assert!(session.select_block(Some(source)));
        assert!(session.copy_selection(BlockSelectionMode::Full));
        let copied = session
            .state
            .active_document()
            .and_then(|document| document.placed_tile(Coord::new(1, 1, 1)))
            .expect("source tile");

        session.open_map(&root.join("test2.dmm"), 1).expect("second map");
        let second = session.state.active().expect("second is active");
        assert_ne!(first, second);

        let target_min = Coord::new(3, 3, 1);
        assert!(session.can_paste_clipboard(target_min, SelectionRotation::Original));
        assert!(session.paste_clipboard(target_min, SelectionRotation::Original));

        let pasted = session
            .state
            .document(second)
            .and_then(|document| document.placed_tile(target_min))
            .expect("pasted tile");
        assert_eq!(
            pasted.iter().map(PlacedPrefab::prefab).collect::<Vec<_>>(),
            copied.iter().map(PlacedPrefab::prefab).collect::<Vec<_>>(),
            "the prefabs survive the trip"
        );
        assert_eq!(
            session.selection(),
            Some(Selection::from_drag(target_min, Coord::new(4, 4, 1))),
            "what landed is selected"
        );

        // The block is still on the clipboard, and the map it came from never moved.
        assert!(session.clipboard().is_some());
        assert_eq!(
            session
                .state
                .document(first)
                .and_then(|document| document.placed_tile(Coord::new(1, 1, 1))),
            Some(copied)
        );
    }

    #[test]
    fn a_paste_is_one_undo_in_the_map_it_landed_in() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).expect("codebase");
        session.open_map(&root.join("test.dmm"), 1).expect("first map");
        session.set_tool(Tool::BlockSelect);
        assert!(session.select_block(Some(Selection::from_drag(Coord::new(1, 1, 1), Coord::new(2, 2, 1)))));
        assert!(session.copy_selection(BlockSelectionMode::Full));

        session.open_map(&root.join("test2.dmm"), 1).expect("second map");
        let second = session.state.active().expect("second is active");
        let before = session
            .state
            .document(second)
            .and_then(|document| document.placed_tile(Coord::new(3, 3, 1)))
            .expect("tile");

        assert!(session.paste_clipboard(Coord::new(3, 3, 1), SelectionRotation::Original));
        assert!(session.state.document(second).is_some_and(MapDocument::is_dirty));

        // `History` is per document and the paste went in as a single edit, so
        // one undo takes all of it back.
        assert!(
            session.state.document_mut(second).is_some_and(MapDocument::undo),
            "the paste is undoable"
        );
        assert_eq!(
            session
                .state
                .document(second)
                .and_then(|document| document.placed_tile(Coord::new(3, 3, 1))),
            Some(before)
        );
    }

    #[test]
    fn cross_map_paste_preserves_the_copied_mask_contents_and_destination_hole() {
        for mode in [
            BlockSelectionMode::Full,
            BlockSelectionMode::Hollow { line_width: 1 },
            BlockSelectionMode::Hollow { line_width: 2 },
        ] {
            for rotation in [
                SelectionRotation::Original,
                SelectionRotation::Clockwise,
                SelectionRotation::Half,
                SelectionRotation::CounterClockwise,
            ] {
                let mut session = flat_session(9, 11);
                let first = session.state.active().unwrap();
                let source = Selection::from_drag(Coord::new(2, 2, 1), Coord::new(6, 8, 1));
                let mut red = Prefab::new(TreePath::parse("/turf/open/floor"));
                red.set_var("color".into(), Value::Text("#ff0000".into()));
                session.state.choose_prefab(red.clone());
                session.set_tool(Tool::BlockSelect);
                assert!(session.select_block_with_mode(Some(source), mode));
                assert!(session.fill_selected_block(source.min, SelectionRotation::Original, mode));
                assert!(session.copy_selection(session.selection_mode()));
                let copied = session.clipboard().unwrap().clone();
                let source_grid = session.map().unwrap().grid.clone();
                let source_revision = session.caches[&first].revision;

                let mut map = Map::new(Size { x: 15, y: 15, z: 2 });
                let base = map.intern_tile(vec![
                    Prefab::new(TreePath::parse("/turf/open/floor")),
                    Prefab::new(TreePath::parse("/area/station")),
                ]);
                for row in map.grid.iter_mut().flatten() {
                    row.fill(base);
                }
                let second = session.activate_document(MapDocument::new(map, 2));
                let destination_grid = session.map().unwrap().grid.clone();
                let min = Coord::new(4, 4, 2);
                let target = session.clipboard_footprint(min, rotation).unwrap();
                let before = target
                    .iter()
                    .map(|coord| {
                        (
                            coord,
                            session.state.active_document().unwrap().placed_tile(coord).unwrap(),
                        )
                    })
                    .collect::<std::collections::HashMap<_, _>>();
                assert_eq!(session.selection_mask(), None);
                assert_eq!(
                    session.clipboard_preview_sprites(target, rotation).len(),
                    mode.tiles(source).count()
                );
                assert!(session.paste_clipboard(min, rotation));
                for coord in target.iter() {
                    let document = session.state.active_document().unwrap();
                    if mode.includes(target, coord) {
                        let mut expected = copied
                            .destinations(target, rotation)
                            .find(|(destination, _)| *destination == coord)
                            .unwrap()
                            .1
                            .clone();
                        for prefab in &mut expected {
                            editor::tool::rotate_prefab(session.tree().unwrap(), prefab, rotation);
                        }
                        assert_eq!(
                            document.map.tile_at(coord).unwrap(),
                            &expected,
                            "{mode:?}, {rotation:?}, {coord:?}"
                        );
                        assert_ne!(document.placed_tile(coord).unwrap()[0].id(), before[&coord][0].id());
                    } else {
                        assert_eq!(
                            document.placed_tile(coord).unwrap(),
                            before[&coord],
                            "the hole must preserve the destination's contents and IDs"
                        );
                    }
                }
                assert_eq!(session.selection_mask(), Some(SelectionMask { bounds: target, mode }));
                assert_render_cache_matches_rebuild(&session);
                assert!(session.undo());
                assert_eq!(session.map().unwrap().grid, destination_grid);
                assert!(!session.undo());
                assert!(session.redo());
                assert_render_cache_matches_rebuild(&session);
                assert_eq!(session.state.document(first).unwrap().map.grid, source_grid);
                assert_eq!(session.caches[&first].revision, source_revision);
                assert_eq!(session.state.active(), Some(second));
            }
        }
    }

    fn paths_at(session: &Session, coord: Coord) -> Vec<String> {
        session
            .map()
            .unwrap()
            .tile_at(coord)
            .unwrap()
            .iter()
            .map(|prefab| prefab.path.to_string())
            .collect()
    }

    fn place(session: &mut Session, prefab: Prefab, coord: Coord) {
        session.state.choose_prefab(prefab);
        session.set_tool(Tool::Place);
        assert!(session.place_at(coord, None).is_some());
    }

    #[test]
    fn a_block_cut_and_paste_leaves_hidden_areas_where_they_were() {
        let mut session = flat_session(4, 4);
        let source = Coord::new(1, 1, 1);
        let destination = Coord::new(3, 3, 1);
        let mut engineering = Prefab::new(TreePath::parse("/area/station"));
        engineering.set_var("name".into(), Value::Text("Engineering".into()));
        place(
            &mut session,
            Prefab::new(TreePath::parse("/obj/structure/table")),
            source,
        );
        place(&mut session, engineering.clone(), destination);
        let source_area = session.state.active_document().unwrap().placed_tile(source).unwrap()[2].clone();
        let destination_area = session
            .state
            .active_document()
            .unwrap()
            .placed_tile(destination)
            .unwrap()[1]
            .clone();
        let area = session.tree().unwrap().id_of(&TreePath::parse("/area")).unwrap();
        assert!(session.toggle_type_visibility(area));

        session.set_tool(Tool::BlockSelect);
        assert!(session.select_block(Some(Selection::from_drag(source, source))));
        assert!(session.cut_selection());
        let cut = session.state.active_document().unwrap().placed_tile(source).unwrap();
        let defaults = editor::tool::default_tile_paths(session.tree().unwrap()).unwrap();
        assert_eq!(cut.len(), 2);
        assert_eq!(cut[0].prefab().path, defaults.0);
        assert_eq!(cut[1], source_area, "the hidden area is untouched");
        assert_render_cache_matches_rebuild(&session);

        assert!(session.paste_clipboard(destination, SelectionRotation::Original));
        let pasted = session
            .state
            .active_document()
            .unwrap()
            .placed_tile(destination)
            .unwrap();
        assert_eq!(
            paths_at(&session, destination),
            ["/obj/structure/table", "/turf/open/floor", "/area/station"]
        );
        assert_eq!(pasted[2], destination_area, "the destination keeps its own area");
        assert_eq!(pasted[2].prefab(), &engineering);
        assert_render_cache_matches_rebuild(&session);

        assert!(session.undo());
        assert!(session.undo());
        assert_eq!(
            paths_at(&session, source),
            ["/obj/structure/table", "/turf/open/floor", "/area/station"]
        );
    }

    #[test]
    fn a_filtered_paste_never_leaves_a_tile_without_a_turf() {
        let mut session = flat_session(4, 1);
        let source = Coord::new(1, 1, 1);
        let destination = Coord::new(3, 1, 1);
        place(
            &mut session,
            Prefab::new(TreePath::parse("/turf/closed/wall")),
            destination,
        );
        let floor = session
            .tree()
            .unwrap()
            .id_of(&TreePath::parse("/turf/open/floor"))
            .unwrap();
        assert!(session.toggle_type_visibility(floor));

        assert!(session.copy_tile(source));
        assert_eq!(
            session.clipboard().unwrap().tile(0, 0).unwrap().len(),
            1,
            "only the area is copied"
        );
        assert!(session.paste_clipboard(destination, SelectionRotation::Original));

        assert_eq!(paths_at(&session, destination), ["/turf/closed/wall", "/area/station"]);
    }

    #[test]
    fn deleting_a_block_with_nothing_visible_to_remove_does_nothing() {
        let mut session = flat_session(4, 4);
        let turf = session.tree().unwrap().id_of(&TreePath::parse("/turf")).unwrap();
        let area = session.tree().unwrap().id_of(&TreePath::parse("/area")).unwrap();
        assert!(session.toggle_type_visibility(turf));
        assert!(session.toggle_type_visibility(area));
        session.set_tool(Tool::BlockSelect);
        assert!(session.select_block(Some(Selection::from_drag(Coord::new(1, 1, 1), Coord::new(4, 4, 1)))));

        assert!(!session.can_clear_tile(Coord::new(1, 1, 1)));
        assert!(!session.delete_selection());
        assert!(!session.state.active_document().unwrap().is_dirty());
    }

    #[test]
    fn context_tile_cut_paste_and_delete_are_undoable() {
        let mut session = flat_session(4, 4);
        let source = Coord::new(2, 2, 1);
        let destination = Coord::new(3, 2, 1);
        let table = TreePath::parse("/obj/structure/table");
        let type_id = session.tree().unwrap().id_of(&table).unwrap();
        assert!(session.choose_type(type_id));
        assert!(session.place_at(source, None).is_some());
        let before = session.map().unwrap().tile_at(source).unwrap().clone();

        assert!(session.copy_tile(source));
        assert_eq!(session.clipboard().unwrap().tile(0, 0), Some(&before));
        assert!(session.cut_tile(source));
        assert_render_cache_matches_rebuild(&session);
        let defaults = editor::tool::default_tile_paths(session.tree().unwrap()).unwrap();
        let cleared = session.map().unwrap().tile_at(source).unwrap();
        assert_eq!(
            cleared.iter().map(|prefab| &prefab.path).collect::<Vec<_>>(),
            vec![&defaults.0, &defaults.1]
        );
        assert!(session.undo());
        assert_eq!(session.map().unwrap().tile_at(source), Some(&before));
        assert!(session.redo());

        assert!(session.paste_clipboard(destination, SelectionRotation::Original));
        assert_eq!(session.map().unwrap().tile_at(destination), Some(&before));
        assert!(session.delete_tile(destination));
        assert_render_cache_matches_rebuild(&session);
        assert!(!session.can_clear_tile(destination));
        assert!(session.undo());
        assert_eq!(session.map().unwrap().tile_at(destination), Some(&before));
    }
}
