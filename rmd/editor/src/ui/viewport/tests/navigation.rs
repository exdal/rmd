use super::*;

#[test]
fn middle_click_outside_the_map_menu_closes_it_and_pans() {
    let _guard = IMGUI_CONTEXT.lock().unwrap();
    let mut app = RectangleUiHarness::new();
    let point = app.tile(5, 8);
    app.context.io_mut().add_mouse_pos_event(point);
    app.context.io_mut().add_mouse_button_event(MouseButton::Right, true);
    app.step();
    app.context.io_mut().add_mouse_button_event(MouseButton::Right, false);
    app.step();
    assert!(app.view.context.is_some(), "right click opens the map menu");

    // the menu opens to the right of the cursor, so a tile to the left is outside it
    let outside = app.tile(2, 8);
    app.context.io_mut().add_mouse_pos_event(outside);
    app.context.io_mut().add_mouse_button_event(MouseButton::Middle, true);
    app.step();
    let before = app.tile(5, 8);
    app.context
        .io_mut()
        .add_mouse_pos_event([outside[0] + 40.0, outside[1] + 20.0]);
    app.step();

    assert_ne!(app.tile(5, 8), before, "the map pans once the menu is gone");
}

#[test]
fn grid_overlay_shortcuts_toggle_settings_while_hovered() {
    let _guard = IMGUI_CONTEXT.lock().unwrap();
    let mut app = RectangleUiHarness::new();
    app.pointer(app.tile(5, 8), false);

    assert!(app.settings.show_tile_grid);
    assert!(app.settings.show_selected_pixel_grid);

    app.key(Key::G, true);
    assert!(!app.settings.show_tile_grid);
    assert!(
        app.settings.show_selected_pixel_grid,
        "plain G should not affect the pixel grid"
    );
    app.key(Key::G, false);

    app.key(Key::ModShift, true);
    app.key(Key::G, true);
    assert!(!app.settings.show_selected_pixel_grid);
    assert!(
        !app.settings.show_tile_grid,
        "shift+g should not toggle the tile grid again"
    );
    app.key(Key::G, false);
    app.key(Key::ModShift, false);
}

#[test]
fn going_to_a_tile_opens_a_view_on_its_level_and_flashes_its_turf() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env");
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).unwrap();
    let mut map = dmm::Map::new(Size { x: 3, y: 3, z: 2 });
    let tile = map.intern_tile(vec![
        Prefab::new(TreePath::parse("/obj/structure/table")),
        Prefab::new(TreePath::parse("/turf/open/floor")),
        Prefab::new(TreePath::parse("/area/station")),
    ]);
    for cell in map.grid.iter_mut().flatten().flatten() {
        *cell = tile;
    }
    let document = session.state.open_document(MapDocument::new(map, 1));
    let target = Coord::new(2, 3, 2);
    let turf = session.state.document(document).unwrap().instance_ids_at(target)[1];
    let mut state = UiState::new(false).expect("valid window keys");

    state.go_to_tile(&mut session, document, target, 10.0);

    assert_eq!(session.state.active(), Some(document));
    assert_eq!(session.z(), 2);
    let mut expected = MapViewState::new(document).unwrap().camera;
    expected.center_on_tile(target, session.options.tile_size);
    let view = state.map_views.get(&document).expect("a view opens for the map");
    assert_eq!(
        (view.camera.camera.x, view.camera.camera.y),
        (expected.camera.x, expected.camera.y)
    );
    assert!(view.focus);
    assert_eq!(
        state.placement_flash,
        Some(ActivePlacementFlash {
            owner: turf,
            coord: target,
            started_at: 10.0,
        })
    );
}

#[test]
fn mirroring_copies_the_active_camera_and_the_levels_other_maps_have() {
    let mut session = Session::new();
    let mut ui = UiState::new(false).unwrap();
    let ids = [3, 2, 3].map(|z| {
        session
            .state
            .open_document(MapDocument::new(dmm::Map::new(Size { x: 4, y: 4, z }), 1))
    });
    for id in ids {
        ui.map_views.insert(id, MapViewState::new(id).unwrap());
    }
    let [active, shallow, deep] = ids;
    session.set_active_document(active);
    session.set_level(3);
    let camera = &mut ui.map_views.get_mut(&active).unwrap().camera.camera;
    camera.x = 48.0;
    camera.y = 80.0;
    camera.zoom = 2.5;
    camera.viewport_width = 640;
    ui.map_views.get_mut(&deep).unwrap().camera.camera.viewport_width = 320;

    ui.mirror_active_camera(&mut session);

    let mirrored = ui.map_views[&deep].camera.camera;
    assert_eq!([mirrored.x, mirrored.y, mirrored.zoom], [48.0, 80.0, 2.5]);
    assert_eq!(mirrored.viewport_width, 320, "each view keeps its own size");
    assert_eq!(session.state.document(deep).unwrap().z, 3);
    assert_eq!(
        session.state.document(shallow).unwrap().z,
        1,
        "a map without level 3 stays put"
    );
    assert_eq!(ui.map_views[&shallow].camera.camera.zoom, 2.5);
}

#[test]
fn arrow_keys_pan_shift_pans_faster_and_space_drags_the_map_without_placing() {
    let _guard = IMGUI_CONTEXT.lock().unwrap();
    let mut app = RectangleUiHarness::new();
    let table = Prefab::new(TreePath::parse("/obj/structure/table"));
    app.session.state.choose_prefab(table.clone());
    app.session.set_tool(Tool::Place);
    app.pointer(app.tile(5, 8), false);

    let pan_right = |app: &mut RectangleUiHarness| {
        let before = app.view.camera.camera.x;
        app.key(Key::RightArrow, true);
        app.key(Key::RightArrow, false);
        app.view.camera.camera.x - before
    };
    let slow = pan_right(&mut app);
    app.key(Key::ModShift, true);
    let fast = pan_right(&mut app);
    app.key(Key::ModShift, false);
    assert!(slow > 0.0, "right pans the view right");
    assert!(fast > slow * 2.0, "shift pans faster");

    let zoom = app.view.camera.camera.zoom;
    app.key(Key::Equal, true);
    app.key(Key::Equal, false);
    assert!(app.view.camera.camera.zoom > zoom);

    let start = app.tile(5, 8);
    app.pointer(start, false);
    let camera = app.view.camera.camera;
    app.key(Key::Space, true);
    app.pointer(start, true);
    app.pointer([start[0] + 64.0, start[1]], true);
    app.pointer([start[0] + 64.0, start[1]], false);
    app.key(Key::Space, false);
    assert_ne!(app.view.camera.camera.x, camera.x, "space and drag pans");
    assert!(
        !app.session
            .state
            .active_document()
            .unwrap()
            .prefab_instances()
            .any(|instance| *instance.prefab() == table),
        "the drag never reaches the place tool"
    );
}
