use base::{path::TreePath, types::Value};

use dmm::{Coord, Map, MapFormat, Prefab, Size};

use crate::{
    document::{MapDocument, command::Edit, grid::TileView},
    session::{
        Session,
        fixtures::{assert_render_cache_matches_rebuild, examples, flat_session},
    },
    tool::Tool,
};

#[test]
fn session_undo_and_redo_update_the_active_render_cache() {
    let root = examples();
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).unwrap();
    session.open_map(&root.join("test.dmm"), 1).unwrap();
    let coord = Coord::new(2, 2, 1);
    let before = session.state.active_document().unwrap().placed_tile(coord).unwrap();
    let table = session
        .tree()
        .unwrap()
        .id_of(&TreePath::parse("/obj/structure/table"))
        .unwrap();

    assert_eq!(session.undo_label(), None);
    assert_eq!(session.redo_label(), None);
    assert!(session.choose_type(table));
    let placed = session.place_at(coord, None).unwrap();
    assert_eq!(session.undo_label(), Some("place /obj/structure/table"));
    assert!(session.state.active_document().unwrap().is_dirty());

    let edit_revision = session.revision();
    assert!(session.undo());
    assert_eq!(
        session.state.active_document().unwrap().placed_tile(coord),
        Some(before)
    );
    assert_eq!(session.redo_label(), Some("place /obj/structure/table"));
    assert!(!session.state.active_document().unwrap().is_dirty());
    assert_eq!(session.revision(), edit_revision.wrapping_add(1));
    assert_render_cache_matches_rebuild(&session);

    let undo_revision = session.revision();
    assert!(session.redo());
    assert_eq!(
        session
            .state
            .active_document()
            .unwrap()
            .instance_location(placed)
            .map(|location| location.coord),
        Some(coord)
    );
    assert_eq!(session.undo_label(), Some("place /obj/structure/table"));
    assert_eq!(session.redo_label(), None);
    assert!(session.state.active_document().unwrap().is_dirty());
    assert_eq!(session.revision(), undo_revision.wrapping_add(1));
    assert_render_cache_matches_rebuild(&session);
}

#[test]
fn session_history_commands_only_change_the_active_document() {
    let root = examples();
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).unwrap();
    session.open_map(&root.join("test.dmm"), 1).unwrap();
    let first = session.state.active().unwrap();
    let coord = Coord::new(2, 2, 1);
    let first_before = session.state.active_document().unwrap().placed_tile(coord).unwrap();
    let table = session
        .tree()
        .unwrap()
        .id_of(&TreePath::parse("/obj/structure/table"))
        .unwrap();
    assert!(session.choose_type(table));
    session.place_at(coord, None).unwrap();

    session.open_map(&root.join("test2.dmm"), 1).unwrap();
    let second = session.state.active().unwrap();
    session.place_at(coord, None).unwrap();
    let second_after = session.state.active_document().unwrap().placed_tile(coord).unwrap();

    session.state.set_active(first);
    assert!(session.undo());

    assert_eq!(
        session.state.document(first).unwrap().placed_tile(coord),
        Some(first_before)
    );
    assert_eq!(
        session.state.document(second).unwrap().placed_tile(coord),
        Some(second_after)
    );
    assert_render_cache_matches_rebuild(&session);
}

#[test]
fn reopening_an_open_map_focuses_it_instead_of_duplicating_it() {
    let root = examples();
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).expect("codebase");
    session.open_map(&root.join("test.dmm"), 1).expect("first map");
    let first = session.state.active().expect("first is active");
    session.open_map(&root.join("test2.dmm"), 1).expect("second map");

    session
        .open_map(&root.join("test.dmm"), 1)
        .expect("reopen the first map");

    assert_eq!(session.state.document_ids().len(), 2, "no duplicate document");
    assert_eq!(session.state.active(), Some(first), "the existing tab is focused");
}

#[test]
fn closing_a_map_drops_its_render_cache() {
    let root = examples();
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).expect("codebase");
    session.open_map(&root.join("test.dmm"), 1).expect("first map");
    let first = session.state.active().expect("first is active");
    session.open_map(&root.join("test2.dmm"), 1).expect("second map");
    let second = session.state.active().expect("second is active");

    assert!(session.close_map(first));

    assert!(!session.caches.contains_key(&first));
    assert!(session.caches.contains_key(&second));
    assert_eq!(session.state.active(), Some(second));
    assert!(
        session
            .map_view_frame(
                first,
                render::MapViewRect::default(),
                render::Camera::default(),
                Default::default(),
                &[],
                &[],
                &[],
            )
            .is_none()
    );
}

#[test]
fn a_new_map_uses_the_requested_size_and_codebase_tile_defaults() {
    let root = examples();
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).unwrap();
    let path = std::env::temp_dir().join(format!("rmd-new-map-{}.dmm", std::process::id()));
    let _ = std::fs::remove_file(&path);

    session
        .create_map(&path, Size { x: 3, y: 2, z: 2 }, MapFormat::Tgm)
        .unwrap();

    let document = session.state.active_document().unwrap();
    assert_eq!(document.path.as_deref(), Some(path.as_path()));
    assert_eq!(document.map.size(), Size { x: 3, y: 2, z: 2 });
    assert_eq!(document.format(), MapFormat::Tgm);
    assert_eq!(document.z, 1);
    assert!(document.is_dirty());
    assert_eq!(document.to_map().dictionary.len(), 1);
    for coord in [Coord::new(1, 1, 1), Coord::new(3, 2, 2)] {
        assert_eq!(
            document
                .map
                .tile_at(coord)
                .unwrap()
                .iter()
                .map(|prefab| prefab.path.to_string())
                .collect::<Vec<_>>(),
            ["/turf", "/area"]
        );
    }

    session.save_map_as(&path, MapFormat::Tgm).unwrap();
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .starts_with("//MAP CONVERTED BY dmm2tgm.py")
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn new_map_creation_accepts_dimensions_above_255() {
    let mut session = Session::new();
    session.load_environment(&examples().join("test.dme")).unwrap();
    let fill = session.default_fill().unwrap();

    for size in [
        Size { x: 256, y: 1, z: 1 },
        Size { x: 1, y: 256, z: 1 },
        Size { x: 1, y: 1, z: 256 },
    ] {
        let path = std::env::temp_dir().join(format!(
            "rmd-new-map-{}-{}-{}-{}.dmm",
            size.x,
            size.y,
            size.z,
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);

        session.create_map(&path, size, MapFormat::Tgm).unwrap();

        let document = session.state.active_document().unwrap();
        assert_eq!(document.map.size(), size);
        assert_eq!(document.z, 1);
        assert!(document.is_dirty());
        for coord in [Coord::new(1, 1, 1), Coord::new(size.x, size.y, size.z)] {
            assert_eq!(document.map.tile_at(coord).map(TileView::to_tile), Some(fill.clone()));
        }
    }
}

#[test]
fn sanitizing_on_save_leaves_default_values_out_of_the_file_only() {
    let mut table = Prefab::new(TreePath::parse("/obj/structure/table"));
    table.set_var("name".into(), Value::Text(String::from("table")));
    table.set_var("icon_state".into(), Value::Text(String::from("broken")));
    for sanitize in [false, true] {
        let mut session = flat_session(1, 1);
        session.state.choose_prefab(table.clone());
        session.set_tool(Tool::Place);
        assert!(session.place_at(Coord::new(1, 1, 1), None).is_some());
        let path = std::env::temp_dir().join(format!("rmd-sanitize-{sanitize}-{}.dmm", std::process::id()));

        session.set_sanitize_vars_on_save(sanitize);
        session.save_map_as(&path, MapFormat::Tgm).unwrap();

        let (saved, errors) = dmm::parser::parse(&std::fs::read_to_string(&path).unwrap());
        assert!(errors.is_empty(), "{errors:?}");
        let saved_table = saved
            .tile_at(Coord::new(1, 1, 1))
            .unwrap()
            .iter()
            .find(|prefab| prefab.path == table.path)
            .unwrap();
        assert_eq!(saved_table.var(&"name".into()).is_none(), sanitize);
        assert_eq!(
            saved_table.var(&"icon_state".into()),
            Some(&Value::Text(String::from("broken")))
        );
        assert!(
            session
                .map()
                .unwrap()
                .tile_at(Coord::new(1, 1, 1))
                .unwrap()
                .contains(&table),
            "the open map keeps every override"
        );
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn save_all_writes_every_changed_map_with_a_file_and_names_one_that_needs_a_path() {
    let saved = std::env::temp_dir().join(format!("rmd-save-all-{}.dmm", std::process::id()));
    let unsaved = std::env::temp_dir().join(format!("rmd-save-all-new-{}.dmm", std::process::id()));
    let _ = std::fs::remove_file(&saved);
    let mut session = Session::new();
    let first = session
        .state
        .open_document(MapDocument::create(&saved, Map::new(Size { x: 1, y: 1, z: 1 }), 1));
    session.save_map_as(&saved, MapFormat::Tgm).unwrap();
    assert!(session.state.document_mut(first).unwrap().resize(2, 1, &[]));
    let second = session
        .state
        .open_document(MapDocument::create(&unsaved, Map::new(Size { x: 1, y: 1, z: 1 }), 1));

    let outcome = session.save_all();

    assert_eq!(outcome.needs_path, Some(second));
    assert_eq!(outcome.error, None);
    assert!(!session.state.document(first).unwrap().is_dirty());
    assert!(session.state.document(second).unwrap().is_dirty());
    assert!(!unsaved.exists(), "a map that never had a file waits for Save As");
    let _ = std::fs::remove_file(saved);
}

#[test]
fn in_place_save_reuses_the_confirmed_path_and_format() {
    let path = std::env::temp_dir().join(format!("rmd-save-map-{}.dmm", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let mut session = Session::new();
    session
        .state
        .open_document(MapDocument::create(&path, Map::new(Size { x: 1, y: 1, z: 1 }), 1));

    assert!(!session.can_save_map_in_place());
    session.save_map_as(&path, MapFormat::Tgm).expect("initial save");
    assert!(session.can_save_map_in_place());

    std::fs::write(&path, "replace me").expect("replace target contents");
    session.save_map().expect("in-place save");
    assert!(
        std::fs::read_to_string(&path)
            .expect("written map")
            .starts_with("//MAP CONVERTED BY dmm2tgm.py")
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn new_map_creation_rejects_invalid_dimensions_and_existing_targets() {
    let mut session = Session::new();
    let path = std::env::temp_dir().join(format!("rmd-new-map-collision-{}.dmm", std::process::id()));

    for size in [
        Size { x: 0, y: 1, z: 1 },
        Size { x: 1, y: 0, z: 1 },
        Size { x: 1, y: 1, z: 0 },
    ] {
        let error = session.create_map(&path, size, MapFormat::Standard).unwrap_err();
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::InvalidInput
        );
        assert!(error.to_string().contains("at least 1"));
    }

    std::fs::write(&path, "existing").unwrap();
    let error = session
        .create_map(&path, Size { x: 1, y: 1, z: 1 }, MapFormat::Standard)
        .unwrap_err();
    assert!(error.to_string().contains("already exists"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn exporting_the_open_map_as_tgm_round_trips_through_the_parser() {
    let root = examples();
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).unwrap();
    let source = root.join("test.dmm");
    session.open_map(&source, 1).unwrap();

    let dir = std::env::temp_dir().join(format!("rmd-session-export-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let exported = dir.join("exported.dmm");

    let before = session.state.active_document().unwrap().to_map();
    session.save_map_as(&exported, dmm::MapFormat::Tgm).expect("export");

    assert_eq!(session.map_path(), Some(exported.as_path()));
    assert_eq!(session.map_format(), Some(dmm::MapFormat::Tgm));

    let (reparsed, errors) = dmm::parser::parse(&std::fs::read_to_string(&exported).expect("written map"));
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(reparsed.format, dmm::MapFormat::Tgm);
    assert_eq!(reparsed.grid, before.grid);
    assert_eq!(reparsed.dictionary, before.dictionary);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn map_prefabs_of_lists_each_placed_prefab_once_and_drops_removed_ones() {
    let path = TreePath::parse("/turf/floor");
    let floor = Prefab::new(path.clone());
    let mut east = floor.clone();
    east.set_var("dir".into(), Value::Num(4.0));
    let mut map = Map::new(Size { x: 3, y: 1, z: 1 });
    let plain = map.intern_tile(vec![floor.clone()]);
    let facing = map.intern_tile(vec![east.clone()]);
    map.grid[0][0] = vec![plain, facing, plain];
    let mut session = Session::new();
    let id = session.state.open_document(MapDocument::new(map, 1));
    session.rebuild_instances(id);

    assert_eq!(session.map_prefabs_of(id, &path), [floor.clone(), east]);

    let document = session.state.document_mut(id).unwrap();
    let placed = vec![document.instantiate(floor.clone())];
    let mut edit = Edit::new("replace");
    edit.change(document, Coord::new(2, 1, 1), placed);
    assert!(document.apply(edit));

    assert_eq!(session.map_prefabs_of(id, &path), [floor]);
}
