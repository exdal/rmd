use base::{coord::Coord, path::TreePath};

use dmm::{Map, Size};
use prefab::Prefab;

use super::validate_level;
use crate::{
    document::{MapDocument, grid::TileView, selection::Selection},
    session::{
        LevelChange, Session,
        fixtures::{assert_render_cache_matches_rebuild, examples, flat_session, focus_session, settle_bake},
    },
    tool::Tool,
};

#[test]
fn resizing_the_map_rebuilds_the_render_cache_and_undo_restores_it() {
    for (width, height) in [(5, 4), (256, 2), (3, 256)] {
        let mut session = flat_session(3, 2);
        let fill = session.default_fill().unwrap();
        let tile = session.options.tile_size as f32;

        assert_eq!(session.resize_map(width, height, &fill), Ok(()));
        let size = session.map().unwrap().size();
        assert_eq!((size.x, size.y), (width, height));
        assert_eq!(
            session
                .map()
                .unwrap()
                .tile_at(Coord::new(width, height, 1))
                .map(TileView::to_tile),
            Some(fill.clone())
        );
        assert_eq!(session.extent_px(), (width as f32 * tile, height as f32 * tile));
        assert_render_cache_matches_rebuild(&session);

        assert!(session.undo());
        let size = session.map().unwrap().size();
        assert_eq!((size.x, size.y), (3, 2));
        assert_render_cache_matches_rebuild(&session);

        assert!(session.redo());
        let size = session.map().unwrap().size();
        assert_eq!((size.x, size.y), (width, height));
        assert_eq!(
            session
                .map()
                .unwrap()
                .tile_at(Coord::new(width, height, 1))
                .map(TileView::to_tile),
            Some(fill.clone())
        );
        assert_render_cache_matches_rebuild(&session);
    }
}

#[test]
fn resizing_with_zero_dimensions_leaves_the_map_and_history_unchanged() {
    let mut session = flat_session(3, 2);
    let fill = session.default_fill().unwrap();
    assert_eq!(session.resize_map(4, 2, &fill), Ok(()));
    let map = session.map().unwrap().clone();
    let revision = session.revision();
    let undo_label = session.undo_label().map(str::to_owned);
    let redo_label = session.redo_label().map(str::to_owned);

    for (width, height) in [(0, 2), (4, 0), (0, 0)] {
        assert_eq!(session.resize_map(width, height, &fill), Ok(()));
        assert_eq!(session.map(), Some(&map));
        assert_eq!(session.revision(), revision);
        assert_eq!(session.undo_label(), undo_label.as_deref());
        assert_eq!(session.redo_label(), redo_label.as_deref());
    }
}

#[test]
fn resize_losses_count_tiles_with_more_than_the_fill() {
    let mut session = flat_session(3, 1);
    let fill = session.default_fill().unwrap();
    assert_eq!(session.resize_map(4, 1, &fill), Ok(()));

    assert_eq!(session.resize_losses(3, 1, &fill), 0, "only the added fill tile goes");
    assert_eq!(session.resize_losses(2, 1, &fill), 1);
    assert_eq!(session.resize_losses(4, 1, &fill), 0);
}

#[test]
fn tile_fill_accepts_only_turf_and_area_subtypes() {
    let session = flat_session(1, 1);
    let fill = session.default_fill().unwrap();
    let (turf, area) = (fill[0].path.to_string(), fill[1].path.to_string());

    assert_eq!(session.tile_fill(&format!(" {turf} "), &area), Ok(fill));
    assert!(session.tile_fill(&area, &area).is_err());
    assert!(session.tile_fill(&turf, "/obj").is_err());
    assert!(session.tile_fill("/turf/missing", &area).is_err());
    assert!(session.tile_fill("", &area).is_err());
}

#[test]
fn validates_one_based_map_levels() {
    assert!(validate_level(1, 3).is_ok());
    assert!(validate_level(3, 3).is_ok());
    assert!(validate_level(0, 3).is_err());
    assert!(validate_level(4, 3).is_err());
}

#[test]
fn changing_levels_keeps_focus_and_selection_on_their_level() {
    let mut session = focus_session();
    session.toggle_focus_at(Some(Coord::new(1, 1, 1)));
    session.set_tool(Tool::Select);
    let selection = Selection::from_drag(Coord::new(1, 1, 1), Coord::new(2, 1, 1));
    assert!(session.select_region(Some(selection)));
    let focus = session.focused_area();
    assert!(focus.is_some());
    assert!(!session.can_edit_at(Coord::new(4, 1, 1)));

    session.change_level(1);

    assert_eq!(session.z(), 2);
    assert_eq!(session.focused_area(), None);
    assert_eq!(session.selection(), None);
    assert!(session.can_edit_at(Coord::new(4, 1, 2)));

    session.change_level(-1);

    assert_eq!(session.focused_area(), focus);
    assert_eq!(session.selection(), Some(selection));
    assert!(!session.can_edit_at(Coord::new(4, 1, 1)));
}

fn baked_session(root: &std::path::Path) -> Session {
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).expect("codebase");
    session.open_map(&root.join("test.dmm"), 1).expect("map");
    settle_bake(&mut session);

    session
}

// by tile, a fresh bake numbers its instances anew
fn assert_bake_matches_the_whole_map(session: &Session) {
    let mut whole = Session::new();
    whole.load_environment(&examples().join("test.dme")).expect("codebase");
    whole.activate_document(MapDocument::new(session.state.active_document().unwrap().to_map(), 1));
    settle_bake(&mut whole);

    let baked = |session: &Session| {
        let document = session.state.active_document().unwrap();
        let bake = session.active_cache().bake.as_ref().expect("a bake");
        let size = document.map.size();
        let tiles = (1..=size.z)
            .flat_map(|z| (1..=size.y).flat_map(move |y| (1..=size.x).map(move |x| Coord::new(x, y, z))))
            .map(|coord| {
                document
                    .instance_ids_at(coord)
                    .iter()
                    .map(|id| (bake.position(id.get()), bake.appearances.get(&id.get()).cloned()))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        (tiles, bake.lighting.clone(), bake.runtime.world.size)
    };
    assert!(
        baked(session) == baked(&whole),
        "the bake diverged from baking the whole map"
    );
}

fn baked_levels() -> Session {
    let mut session = Session::new();
    session
        .load_environment(&examples().join("test.dme"))
        .expect("codebase");
    let mut map = Map::new(Size { x: 2, y: 1, z: 3 });
    let fill = session.default_fill().expect("fill types");
    for z in 1..=3 {
        let mut tile = fill.clone();
        let mut table = Prefab::new(TreePath::parse("/obj/structure/table"));
        table.set_var("name".into(), base::types::Value::Text(format!("level {z}")));
        tile.insert(0, table);
        let key = map.intern_tile(tile);
        map.grid[z - 1][0].fill(key);
    }

    session.activate_document(MapDocument::new(map, 3));
    settle_bake(&mut session);
    session
}

#[test]
fn loaded_conflicts_block_level_deletion_and_history_even_when_hidden() {
    let mut session = crate::session::fixtures::conflicted_session();
    let id = session.state.active().unwrap();
    let fill = session.map().unwrap().tile_at(Coord::new(1, 1, 1)).unwrap().to_tile();
    session.create_level(id, &fill).unwrap();
    session.git_enabled = false;
    assert!(!session.can_delete_level(id));
    assert!(session.delete_level(id, 1).is_err());
    assert_eq!(session.undo_label(), None);
    assert!(!session.undo());
    let conflicts = session
        .caches
        .get_mut(&id)
        .unwrap()
        .git
        .as_mut()
        .unwrap()
        .conflicts
        .take();
    assert!(session.undo());
    session.caches.get_mut(&id).unwrap().git.as_mut().unwrap().conflicts = conflicts;
    assert_eq!(session.redo_label(), None);
    assert!(!session.redo());
    assert_eq!(session.level_count(), 1);
}

#[test]
fn deleting_levels_rebuilds_the_bake_and_history_restores_source_ids() {
    for z in 1..=3 {
        let mut session = baked_levels();
        let id = session.state.active().expect("document");
        let original = session.state.document(id).expect("document").map.clone();
        session.set_level(z);
        session.delete_level(id, z).expect("delete level");
        settle_bake(&mut session);
        assert_eq!(session.level_count(), 2);
        assert_eq!(session.z(), z.min(2));
        assert_render_cache_matches_rebuild(&session);
        assert_bake_matches_the_whole_map(&session);

        assert!(session.undo());
        settle_bake(&mut session);
        assert_eq!(session.state.document(id).expect("document").map, original);
        assert_render_cache_matches_rebuild(&session);
        assert_bake_matches_the_whole_map(&session);

        assert!(session.redo());
        settle_bake(&mut session);
        assert_eq!(session.level_count(), 2);
        assert_render_cache_matches_rebuild(&session);
        assert_bake_matches_the_whole_map(&session);
    }
}

#[test]
fn level_deletion_and_history_discard_in_flight_builds_and_extensions() {
    for initial in [false, true] {
        let mut session = baked_levels();
        let id = session.state.active().expect("document");
        let original = session.state.document(id).expect("document").map.clone();
        let fill = session.default_fill().expect("fill types");
        assert_eq!(session.create_level(id, &fill), Ok(4));
        assert!(session.baker.extending(id));
        if initial {
            settle_bake(&mut session);
            session.rebake(id);
        }

        session.delete_level(id, 1).expect("delete level");
        assert!(session.baker.invalidated(id));
        assert!(session.undo());
        assert!(session.redo());
        assert!(session.undo());
        assert!(session.undo(), "undo level creation before the stale bake arrives");
        settle_bake(&mut session);
        assert_eq!(session.state.document(id).expect("document").map, original);
        assert_render_cache_matches_rebuild(&session);
        assert_bake_matches_the_whole_map(&session);

        assert!(session.redo());
        settle_bake(&mut session);
        assert_eq!(session.level_count(), 4);
        assert_render_cache_matches_rebuild(&session);
        assert_bake_matches_the_whole_map(&session);
    }
}

#[test]
fn a_new_level_bakes_in_the_background_like_the_whole_map_would() {
    let mut session = baked_session(&examples());
    let id = session.state.active().unwrap();
    let levels = session.level_count();
    session.set_level(levels);
    let fill = session.default_fill().unwrap();

    assert_eq!(session.create_level(id, &fill), Ok(levels + 1));

    assert!(session.baker.extending(id));
    assert_eq!(session.bake_view().map(|view| view.title), Some("Baking new level"));
    settle_bake(&mut session);
    assert!(session.bake_view().is_none());
    assert_render_cache_matches_rebuild(&session);
    assert_bake_matches_the_whole_map(&session);
}

#[test]
fn edits_made_while_a_new_level_bakes_are_baked_when_it_lands() {
    let mut session = baked_session(&examples());
    let id = session.state.active().unwrap();
    let levels = session.level_count();
    session.set_level(levels);
    let fill = session.default_fill().unwrap();
    assert_eq!(session.create_level(id, &fill), Ok(levels + 1));
    assert!(session.baker.extending(id));

    session
        .state
        .choose_prefab(Prefab::new(TreePath::parse("/obj/structure/table")));
    session.set_tool(Tool::Place);
    assert!(session.place_at(Coord::new(2, 2, levels + 1), None).is_some());
    session.set_level(levels);
    assert!(session.place_at(Coord::new(2, 2, levels), None).is_some());

    assert!(
        !session.baker.invalidated(id),
        "an edit sent the map back for a full bake"
    );
    settle_bake(&mut session);
    assert_render_cache_matches_rebuild(&session);
    assert_bake_matches_the_whole_map(&session);
}

#[test]
fn creating_a_level_clears_focus() {
    let mut session = focus_session();
    let id = session.state.active().unwrap();
    session.set_level(5);
    session.toggle_focus_at(Some(Coord::new(1, 1, 5)));
    assert!(session.focused_area().is_some());

    let fill = session.tile_fill("/turf", "/area").unwrap();
    assert_eq!(session.create_level(id, &fill), Ok(6));
    session.set_level(5);

    assert_eq!(session.focused_area(), None);
    assert!(session.can_edit_at(Coord::new(4, 1, 5)));
}

#[test]
fn changing_up_from_the_last_level_requests_and_uses_a_fill_type() {
    let root = examples();
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).expect("codebase");
    let mut map = Map::new(Size { x: 2, y: 1, z: 1 });
    let key = map.intern_tile(vec![
        Prefab::new(TreePath::parse("/turf")),
        Prefab::new(TreePath::parse("/area")),
    ]);
    for cell in map.grid.iter_mut().flatten().flatten() {
        *cell = key;
    }
    let id = session.activate_document(MapDocument::new(map, 1));
    let revision = session.revision();

    assert!(session.can_change_level(1));
    assert!(!session.can_change_level(-1));
    assert_eq!(session.change_level(1), LevelChange::NewLevelRequested);
    assert_eq!(session.level_count(), 1, "the request does not create a level yet");
    let fill = session.tile_fill("/turf", "/area").unwrap();
    assert_eq!(session.create_level(id, &fill), Ok(2));

    let document = session.state.active_document().expect("active document");
    assert_eq!(document.map.size().z, 2);
    assert_eq!(document.z, 2);
    assert!(document.is_dirty());
    for x in 1..=2 {
        assert_eq!(
            document.map.tile_at(Coord::new(x, 1, 2)).map(TileView::to_tile),
            Some(fill.clone()),
            "the chosen turf and area"
        );
    }
    assert_ne!(
        session.revision(),
        revision,
        "the appended level is rendered immediately"
    );
    assert_eq!(
        session
            .map_view_frame(
                id,
                Default::default(),
                Default::default(),
                Default::default(),
                &[],
                &[],
                &[]
            )
            .unwrap()
            .level_count,
        2
    );

    session.change_level(-1);
    assert_eq!(session.z(), 1);
    assert_eq!(
        session.level_count(),
        2,
        "moving down does not delete the appended level"
    );
}

#[test]
fn level_creation_can_exceed_255_levels() {
    let root = examples();
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).expect("codebase");
    let mut map = Map::new(Size { x: 1, y: 1, z: 255 });
    let key = map.intern_tile(vec![
        Prefab::new(TreePath::parse("/turf")),
        Prefab::new(TreePath::parse("/area")),
    ]);
    for cell in map.grid.iter_mut().flatten().flatten() {
        *cell = key;
    }
    let id = session.activate_document(MapDocument::new(map, 255));
    let fill = session.default_fill().unwrap();

    for z in [256, 257] {
        assert!(session.can_change_level(1));
        assert_eq!(session.change_level(1), LevelChange::NewLevelRequested);
        assert_eq!(session.z(), z - 1);
        assert_eq!(session.level_count(), z - 1);

        assert_eq!(session.create_level(id, &fill), Ok(z));
        assert_eq!(session.z(), z);
        assert_eq!(session.level_count(), z);
        assert_eq!(
            session
                .map()
                .unwrap()
                .tile_at(Coord::new(1, 1, z))
                .map(TileView::to_tile),
            Some(fill.clone())
        );
    }
}
