use base::{coord::Coord, path::TreePath, types::Value};

use prefab::{Prefab, PrefabInstanceId};

use vm::world::Position;

use crate::{
    document::{grid::TileView, selection::Selection},
    session::{
        BakeHold, FillOutcome, Session,
        fixtures::{assert_render_cache_matches_rebuild, examples, flat_session, focus_session, settle_bake},
    },
    tool::{
        FillMode, RegionSelectionMode, SelectionMask, SelectionPlacement, SelectionRotation, SelectionTransform, Tool,
    },
    viewport::preview::RegionPreviewSource,
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
        let untouched_before = session.map().unwrap().tile_at(untouched).unwrap().to_tile();
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
        assert_eq!(
            session.map().unwrap().tile_at(untouched).map(TileView::to_tile),
            Some(untouched_before.clone())
        );
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
    let untouched_before = session.map().unwrap().tile_at(untouched).unwrap().to_tile();
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
    assert_eq!(
        session.map().unwrap().tile_at(untouched).map(TileView::to_tile),
        Some(untouched_before.clone())
    );
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

#[derive(Debug, Clone, Copy)]
enum MoveExit {
    Finish,
    Cancel,
    Deselect,
    Reselect,
    Paste,
}

#[test]
fn every_way_out_of_a_live_move_releases_the_bake() {
    let paste_at = Coord::new(2, 1, 1);
    for is_floating in [false, true] {
        for exit in [
            MoveExit::Finish,
            MoveExit::Cancel,
            MoveExit::Deselect,
            MoveExit::Reselect,
            MoveExit::Paste,
        ] {
            let case = format!("{exit:?}, is_floating: {is_floating}");
            let (mut session, _) = table_beside_a_wall(Coord::new(3, 1, 1));
            assert!(session.copy_tile(Coord::new(1, 1, 1)));
            if is_floating {
                assert!(session.paste_clipboard(Coord::new(4, 1, 1), SelectionRotation::Original));
            }

            let start = session.selection().unwrap().min;
            assert!(
                session.move_selected_region_live(Coord::new(start.x + 1, 1, 1)),
                "{case}"
            );
            assert!(is_bake_held(&session), "{case}");

            let is_still_floating = match exit {
                MoveExit::Finish => {
                    session.finish_live_move();

                    is_floating
                },
                MoveExit::Cancel => {
                    session.cancel_live_move();

                    is_floating
                },
                MoveExit::Deselect => {
                    session.select_region(None);

                    false
                },
                MoveExit::Reselect => {
                    let selection = session.selection();
                    session.select_region(selection);

                    false
                },
                MoveExit::Paste => {
                    assert!(session.paste_clipboard(paste_at, SelectionRotation::Original));
                    assert_eq!(session.selection(), Some(Selection::from_drag(paste_at, paste_at)));

                    true
                },
            };

            assert!(!is_bake_held(&session), "{case}");
            assert_eq!(session.pending_move_source(), None, "{case}");
            assert_eq!(session.live_floating_paste().is_some(), is_still_floating, "{case}");
            assert_render_cache_matches_rebuild(&session);
        }
    }
}

#[test]
fn a_normal_move_drops_a_stale_float_so_undo_cannot_revive_it() {
    for is_moved in [false, true] {
        let (mut session, _) = table_beside_a_wall(Coord::new(3, 1, 1));
        assert!(session.copy_tile(Coord::new(1, 1, 1)));
        assert!(session.paste_clipboard(Coord::new(4, 1, 1), SelectionRotation::Original));
        session
            .state
            .choose_prefab(Prefab::new(TreePath::parse("/obj/structure/table")));
        session.set_tool(Tool::Place);
        assert!(session.place_at(Coord::new(6, 1, 1), None).is_some());
        session.set_tool(Tool::Select);
        assert!(
            session.live_floating_paste().is_none(),
            "another edit makes the paste stale"
        );

        if is_moved {
            assert!(session.move_selected_region_live(Coord::new(5, 1, 1)));
            session.finish_live_move();
            assert_eq!(session.undo_label(), Some("move region"));
            assert!(session.undo());
        }

        assert!(session.undo());
        assert_eq!(session.undo_label(), Some("paste region"));
        assert_eq!(
            session.live_floating_paste().is_some(),
            !is_moved,
            "undoing back to the paste revives it unless a normal move started in between"
        );
    }
}
