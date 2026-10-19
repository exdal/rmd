use super::*;

fn red_floor() -> Prefab {
    let mut red = Prefab::new(TreePath::parse("/turf/open/floor"));
    red.set_var("color".into(), core::types::Value::Text("#ff0000".into()));

    red
}

fn center_of(app: &RectangleUiHarness, selection: Selection) -> [f32; 2] {
    let (min, max) = (
        app.tile(selection.min.x, selection.min.y),
        app.tile(selection.max.x, selection.max.y),
    );

    [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5]
}

fn ctrl_key(app: &mut RectangleUiHarness, key: Key) {
    app.key(Key::ModCtrl, true);
    app.key(key, true);
    app.key(key, false);
    app.key(Key::ModCtrl, false);
}

#[test]
fn rectangle_ui_draws_borders_resizes_from_the_edges_and_shares_the_mask_with_the_bucket() {
    let _guard = IMGUI_CONTEXT.lock().unwrap();
    let mut app = RectangleUiHarness::new();
    app.pointer(app.tile(5, 8), false);
    app.key(Key::ModShift, true);
    app.key(Key::S, true);
    assert_eq!(app.session.tool(), Tool::BlockSelect);
    assert!(app.session.selection().is_none());
    app.key(Key::S, false);

    // shift is still held from the shortcut, so the drag draws a border
    app.drag(app.tile(5, 8), app.tile(7, 10));
    app.key(Key::ModShift, false);
    let border = SelectionMask {
        bounds: Selection::from_drag(Coord::new(5, 8, 1), Coord::new(7, 10, 1)),
        mode: BlockSelectionMode::Hollow { line_width: 1 },
    };
    assert_eq!(app.session.selection_mask(), Some(border));
    assert_eq!(app.session.undo_label(), None);

    let corner = app.tile(7, 10);
    let corner = [corner[0] + 23.0, corner[1] - 23.0];
    app.drag(corner, [corner[0] + 64.0, corner[1] - 64.0]);
    let resized = SelectionMask {
        bounds: Selection::from_drag(border.bounds.min, Coord::new(9, 12, 1)),
        ..border
    };
    assert_eq!(app.session.selection_mask(), Some(resized));
    assert!(app.view.gestures.block_placement.is_none());
    assert_eq!(app.session.undo_label(), None);

    app.pointer(app.tile(5, 8), false);
    app.key(Key::Q, true);
    app.key(Key::Q, false);
    assert_eq!(app.session.tool(), Tool::Fill);
    assert_eq!(app.session.selection_mask(), Some(resized));
    let red = red_floor();
    app.session.state.choose_prefab(red.clone());
    app.click(app.tile(7, 10));
    assert!(
        !app.session
            .map()
            .unwrap()
            .tile_at(resized.bounds.min)
            .unwrap()
            .contains(&red)
    );
    app.click(app.tile(5, 8));
    assert!(
        app.session
            .map()
            .unwrap()
            .tile_at(resized.bounds.min)
            .unwrap()
            .contains(&red)
    );
    assert!(
        !app.session
            .map()
            .unwrap()
            .tile_at(Coord::new(4, 8, 1))
            .unwrap()
            .contains(&red)
    );
    app.key(Key::Escape, true);
    app.key(Key::Escape, false);
    assert_eq!(app.session.selection_mask(), None);
}

#[test]
fn rectangle_ui_keeps_shift_draw_mode_and_restores_cancelled_gestures() {
    let _guard = IMGUI_CONTEXT.lock().unwrap();
    let mut app = RectangleUiHarness::new();
    app.session.set_tool(Tool::BlockSelect);
    app.pointer(app.tile(5, 8), true);
    app.pointer(app.tile(7, 10), true);
    assert_eq!(app.session.selection_mode(), BlockSelectionMode::Full);
    app.key(Key::ModShift, true);
    assert_eq!(
        app.session.selection_mode(),
        BlockSelectionMode::Hollow { line_width: 1 }
    );
    app.context.io_mut().add_key_event(Key::ModShift, false);
    app.pointer(app.tile(7, 10), false);
    let original = app.session.selection_mask().unwrap();
    assert_eq!(original.mode, BlockSelectionMode::Hollow { line_width: 1 });
    assert_eq!(app.state.block_selection_options.mode(), BlockSelectionMode::Full);

    app.pointer(app.tile(12, 12), true);
    app.pointer(app.tile(14, 14), true);
    assert_eq!(app.session.selection_mode(), BlockSelectionMode::Full);
    app.key(Key::Escape, true);
    assert_eq!(app.session.selection_mask(), Some(original));
    app.key(Key::Escape, false);
    app.pointer(app.tile(14, 14), false);
    assert_eq!(app.session.selection_mask(), Some(original));

    // a corner handle resizes, and tool changes cancel the active resize
    let corner = app.tile(7, 10);
    let corner = [corner[0] + 23.0, corner[1] - 23.0];
    app.pointer(corner, false);
    app.pointer(corner, true);
    app.pointer([corner[0] + 32.0, corner[1] - 32.0], true);
    assert_eq!(app.session.selection().unwrap().max, Coord::new(8, 11, 1));
    assert!(app.view.gestures.block_placement.is_none());
    app.key(Key::Q, true);
    app.key(Key::Q, false);
    assert_eq!(app.session.tool(), Tool::Fill);
    assert_eq!(app.session.selection_mask(), Some(original));
    app.pointer(corner, false);
    assert_eq!(app.session.undo_label(), None);
    assert!(app.view.gestures.rectangle_gesture.is_none());
}

#[test]
fn rectangle_ui_clears_on_a_click_outside_and_selects_one_tile_without_a_selection() {
    let _guard = IMGUI_CONTEXT.lock().unwrap();
    let mut app = RectangleUiHarness::new();
    app.session.set_tool(Tool::BlockSelect);
    app.click(app.tile(4, 4));
    let one = Selection::from_drag(Coord::new(4, 4, 1), Coord::new(4, 4, 1));
    assert_eq!(app.session.selection(), Some(one));
    assert_eq!(app.session.undo_label(), None);

    app.drag(app.tile(10, 10), app.tile(12, 11));
    assert_eq!(
        app.session.selection(),
        Some(Selection::from_drag(Coord::new(10, 10, 1), Coord::new(12, 11, 1)))
    );
    app.click(app.tile(3, 8));
    assert_eq!(app.session.selection(), None);
    assert_eq!(app.session.undo_label(), None);
}

#[test]
fn rectangle_ui_moves_on_release_copies_with_ctrl_and_cancels_with_escape() {
    let _guard = IMGUI_CONTEXT.lock().unwrap();
    let mut app = RectangleUiHarness::new();
    app.session
        .state
        .choose_prefab(Prefab::new(TreePath::parse("/obj/structure/table")));
    app.session.set_tool(Tool::Place);
    let table = app.session.place_at(Coord::new(5, 8, 1), None).unwrap();
    let tables = |app: &RectangleUiHarness| {
        Selection::from_drag(Coord::new(1, 1, 1), Coord::new(20, 20, 1))
            .iter()
            .filter(|coord| {
                app.session
                    .map()
                    .unwrap()
                    .tile_at(*coord)
                    .unwrap()
                    .iter()
                    .any(|prefab| prefab.path == TreePath::parse("/obj/structure/table"))
            })
            .collect::<Vec<_>>()
    };
    app.session.set_tool(Tool::BlockSelect);
    app.step();
    app.drag(app.tile(5, 8), app.tile(6, 9));
    let source = Selection::from_drag(Coord::new(5, 8, 1), Coord::new(6, 9, 1));
    let center = center_of(&app, source);
    let before = app.session.map().unwrap().clone();

    // escape mid-move drops the drag without an edit
    app.pointer(center, false);
    app.pointer(center, true);
    app.pointer([center[0] + 64.0, center[1]], true);
    assert!(app.view.gestures.block_placement.is_some());
    app.key(Key::Escape, true);
    app.key(Key::Escape, false);
    app.pointer([center[0] + 64.0, center[1]], false);
    assert_eq!(app.session.selection(), Some(source));
    assert_eq!(*app.session.map().unwrap(), before);

    app.key(Key::ModCtrl, true);
    app.pointer(center, true);
    app.key(Key::ModCtrl, false);
    app.pointer([center[0], center[1] - 64.0], true);
    app.pointer([center[0], center[1] - 64.0], false);
    let copied = Selection::from_drag(Coord::new(5, 10, 1), Coord::new(6, 11, 1));
    assert_eq!(app.session.selection(), Some(copied));
    assert_eq!(tables(&app), [Coord::new(5, 8, 1), Coord::new(5, 10, 1)]);

    let center = center_of(&app, copied);
    app.drag(center, [center[0] + 64.0, center[1]]);
    assert_eq!(
        app.session.selection(),
        Some(Selection::from_drag(Coord::new(7, 10, 1), Coord::new(8, 11, 1)))
    );
    assert_eq!(tables(&app), [Coord::new(5, 8, 1), Coord::new(7, 10, 1)]);
    let location = app.session.state.active_document().unwrap().instance_location(table);
    assert_eq!(location.map(|location| location.coord), Some(Coord::new(5, 8, 1)));
}

#[test]
fn rotate_shortcut_rotates_the_selection_in_place_with_both_presets_and_a_custom_binding() {
    let _guard = IMGUI_CONTEXT.lock().unwrap();
    for (preset, modifier, key, custom) in [
        (KeybindPreset::Default, None, Key::R, false),
        (KeybindPreset::StrongDmm, Some(Key::ModShift), Key::R, false),
        (KeybindPreset::Default, Some(Key::ModCtrl), Key::T, true),
    ] {
        let mut app = RectangleUiHarness::new();
        app.settings.keybindings = preset.bindings();
        if custom {
            app.settings
                .keybindings
                .rebind(KeybindAction::Rotate, KeyBinding::with_ctrl(Key::T));
        }
        app.session.set_tool(Tool::BlockSelect);
        app.drag(app.tile(2, 8), app.tile(4, 11));
        app.pointer(app.tile(10, 10), false);
        if let Some(modifier) = modifier {
            app.key(modifier, true);
        }
        app.key(key, true);
        app.key(key, false);
        if let Some(modifier) = modifier {
            app.key(modifier, false);
        }
        assert_eq!(
            app.session.selection(),
            Some(Selection::from_drag(Coord::new(2, 8, 1), Coord::new(5, 10, 1))),
            "{preset:?}"
        );
        assert!(app.session.undo_label().is_some());
    }
}

#[test]
fn rectangle_ui_pastes_a_border_into_another_map_immediately() {
    let _guard = IMGUI_CONTEXT.lock().unwrap();
    let mut app = RectangleUiHarness::new();
    let source_id = app.id;
    app.session.set_tool(Tool::BlockSelect);
    app.key(Key::ModShift, true);
    app.drag(app.tile(5, 8), app.tile(9, 14));
    app.key(Key::ModShift, false);
    let source = app.session.selection_mask().unwrap();
    assert_eq!(source.mode, BlockSelectionMode::Hollow { line_width: 1 });
    let red = red_floor();
    app.session.state.choose_prefab(red.clone());
    assert!(app.session.fill_rect(source));
    app.pointer(app.tile(5, 8), false);
    ctrl_key(&mut app, Key::C);
    let clipboard = app.session.clipboard().unwrap().clone();
    assert_eq!(clipboard.filled().count(), source.mode.tiles(source.bounds).count());
    let source_grid = app.session.map().unwrap().clone();

    let mut destination = dmm::Map::new(Size { x: 20, y: 20, z: 1 });
    let blue = Prefab::new(TreePath::parse("/turf/open/floor"));
    let key = destination.intern_tile(vec![blue.clone(), Prefab::new(TreePath::parse("/area/station"))]);
    for row in &mut destination.grid[0] {
        row.fill(key);
    }
    app.session.apply_map(crate::loader::LoadedMap {
        path: PathBuf::from("rectangle-ui-destination.dmm"),
        map: destination,
        z: 1,
        errors: vec![],
        repo: None,
        conflict: None,
    });
    let destination_grid = app.session.map().unwrap().clone();
    app.id = app.session.state.active().unwrap();
    let mut destination_view = MapViewState::new(app.id).unwrap();
    destination_view.refit = false;
    destination_view.focus = true;
    destination_view.camera.camera.x = 320.0;
    destination_view.camera.camera.y = 320.0;
    let source_view = std::mem::replace(&mut app.view, destination_view);
    app.state.map_views.insert(source_id, source_view);
    // destination settings and palette must not change what was copied
    app.state.block_selection_options.full_rectangle = false;
    app.state.block_selection_options.line_width = 3;
    app.session.state.choose_prefab(blue.clone());
    for _ in 0..3 {
        app.step();
    }
    app.pointer(app.tile(12, 10), false);
    ctrl_key(&mut app, Key::V);
    let target = SelectionMask {
        bounds: Selection::from_drag(Coord::new(10, 7, 1), Coord::new(14, 13, 1)),
        mode: source.mode,
    };
    assert_eq!(app.session.tool(), Tool::BlockSelect);
    assert_eq!(app.session.selection_mask(), Some(target));
    for coord in target.bounds.iter() {
        let tile = app.session.map().unwrap().tile_at(coord).unwrap();
        assert_eq!(tile.contains(&red), target.includes(coord));
        assert_eq!(tile.contains(&blue), !target.includes(coord));
    }
    assert_eq!(app.session.state.document(source_id).unwrap().map, source_grid);
    ctrl_key(&mut app, Key::Z);
    assert_eq!(*app.session.map().unwrap(), destination_grid);
    assert_eq!(app.session.state.document(source_id).unwrap().map, source_grid);
    ctrl_key(&mut app, Key::Y);
    assert_eq!(app.session.selection_mask(), Some(target));
}

#[test]
fn rect_fill_tool_fills_on_release_and_leaves_the_selection_alone() {
    let _guard = IMGUI_CONTEXT.lock().unwrap();
    let mut app = RectangleUiHarness::new();
    app.session.set_tool(Tool::BlockSelect);
    app.drag(app.tile(15, 15), app.tile(16, 16));
    let selection = app.session.selection_mask();
    assert!(selection.is_some());
    let red = red_floor();
    app.session.state.choose_prefab(red.clone());
    app.pointer(app.tile(8, 8), false);
    app.key(Key::ModShift, true);
    app.key(Key::Q, true);
    app.key(Key::Q, false);
    app.key(Key::ModShift, false);
    assert_eq!(app.session.tool(), Tool::RectFill);
    let is_red = |app: &RectangleUiHarness, x, y| {
        app.session
            .map()
            .unwrap()
            .tile_at(Coord::new(x, y, 1))
            .unwrap()
            .contains(&red)
    };

    app.pointer(app.tile(6, 8), true);
    app.pointer(app.tile(8, 10), true);
    assert!(!is_red(&app, 6, 8), "nothing is painted until release");
    app.pointer(app.tile(8, 10), false);
    assert!(is_red(&app, 6, 8) && is_red(&app, 7, 9) && is_red(&app, 8, 10));
    assert!(!is_red(&app, 9, 11));

    app.key(Key::ModShift, true);
    app.drag(app.tile(11, 8), app.tile(13, 10));
    app.key(Key::ModShift, false);
    assert!(is_red(&app, 11, 9) && !is_red(&app, 12, 9));

    let before = app.session.map().unwrap().clone();
    app.pointer(app.tile(6, 12), true);
    app.pointer(app.tile(7, 13), true);
    app.key(Key::Escape, true);
    app.key(Key::Escape, false);
    app.pointer(app.tile(7, 13), false);
    assert_eq!(*app.session.map().unwrap(), before);
    assert_eq!(app.session.selection_mask(), selection);
}

fn gizmo_frame(
    context: &mut dear_imgui_rs::Context, gizmo: &mut GizmoState, camera: &Controller, target: BlockGizmoTarget,
    hovered: bool,
) -> crate::gizmo::BlockGizmoResponse {
    let ui = context.frame();
    let view = GizmoMapView {
        min: [0.0; 2],
        max: [800.0, 600.0],
        hovered,
    };
    let response = ui
        .window("rectangle-test")
        .position([0.0; 2], Condition::Always)
        .size([800.0, 600.0], Condition::Always)
        .build(|| {
            let response = gizmo.draw_block(ui, camera, target, view);
            gizmo.draw_block_overlay(
                ui,
                camera,
                BlockGizmoTarget {
                    selection: response.selection,
                    ..target
                },
                view,
            );
            response
        })
        .unwrap();
    assert!(context.render_legacy().valid());
    response
}

#[test]
fn block_drags_keep_their_mouse_down_action_when_modifiers_change() {
    let _guard = IMGUI_CONTEXT.lock().unwrap();
    for (resize, copy) in [(true, false), (false, false), (false, true)] {
        let mut context = rectangle_context();
        let mut camera = Controller::new();
        camera.resize(800, 600);
        camera.center_on_tile(Coord::new(5, 5, 1), 32);
        let selection = Selection::from_drag(Coord::new(3, 3, 1), Coord::new(5, 5, 1));
        let bounds = block_selection_bounds(&camera, [0.0; 2], selection, 32);
        let start = if resize {
            [bounds.max[0] + 7.0, bounds.min[1] - 7.0]
        } else {
            [
                (bounds.min[0] + bounds.max[0]) * 0.5,
                (bounds.min[1] + bounds.max[1]) * 0.5,
            ]
        };
        let mut target = BlockGizmoTarget {
            selection,
            map_size: Size { x: 10, y: 10, z: 1 },
            tile_size: 32,
        };
        let mut gizmo = GizmoState::default();
        let expected = |released| {
            if resize {
                BlockDrag::Resize
            } else {
                BlockDrag::Move { copy, released }
            }
        };
        context.io_mut().add_mouse_pos_event(start);
        context.io_mut().add_key_event(Key::ModCtrl, copy);
        context.io_mut().add_mouse_button_event(MouseButton::Left, true);
        let pressed = gizmo_frame(&mut context, &mut gizmo, &camera, target, true);
        assert_eq!(pressed.drag, Some(expected(false)));
        context.io_mut().add_key_event(Key::ModCtrl, !copy);
        context.io_mut().add_mouse_pos_event([start[0] + 32.0, start[1] - 32.0]);
        let dragged = gizmo_frame(&mut context, &mut gizmo, &camera, target, true);
        assert_eq!(dragged.drag, Some(expected(false)));
        assert_eq!(dragged.selection.width(), if resize { 4 } else { 3 });
        assert_eq!(
            dragged.selection.min,
            if resize { selection.min } else { Coord::new(4, 4, 1) }
        );
        if resize {
            target.selection = dragged.selection;
        }
        context.io_mut().add_mouse_button_event(MouseButton::Left, false);
        let released = gizmo_frame(&mut context, &mut gizmo, &camera, target, false);
        assert_eq!(released.drag, Some(expected(true)));
        assert!(released.captures_mouse);
        assert!(!gizmo.is_interacting());
    }
}

#[test]
fn cancelling_rectangle_gestures_restores_only_the_originating_document_and_level() {
    let mut session = Session::new();
    let id = session
        .state
        .open_document(MapDocument::new(dmm::Map::new(Size { x: 10, y: 10, z: 2 }), 1));
    let start = SelectionMask {
        bounds: Selection::from_drag(Coord::new(2, 2, 1), Coord::new(4, 4, 1)),
        mode: BlockSelectionMode::Hollow { line_width: 1 },
    };
    session.try_select_block(start);
    let mut gesture = Some(RectangleGesture {
        start: Some(start),
        z: 1,
    });
    session.try_select_block(SelectionMask {
        bounds: Selection::from_drag(start.bounds.min, Coord::new(8, 8, 1)),
        mode: BlockSelectionMode::Full,
    });
    let other = session
        .state
        .open_document(MapDocument::new(dmm::Map::new(Size { x: 3, y: 3, z: 1 }), 1));
    restore_rectangle_gesture(&mut session, id, &mut gesture);
    assert_eq!(session.state.document(id).unwrap().selection_mask(), Some(start));
    assert_eq!(session.state.document(other).unwrap().selection_mask(), None);
    assert!(gesture.is_none());
    session.state.set_active(id);
    gesture = Some(RectangleGesture {
        start: Some(start),
        z: 1,
    });
    session.set_level(2);
    restore_rectangle_gesture(&mut session, id, &mut gesture);
    assert_eq!(session.selection_mask(), None);

    session.set_level(1);
    assert_eq!(session.selection_mask(), Some(start));
}

#[test]
fn rectangle_ui_moves_and_cuts_on_a_created_level() {
    let _guard = IMGUI_CONTEXT.lock().unwrap();
    for created in [false, true] {
        let mut app = RectangleUiHarness::new();
        let z = if created {
            let fill = app.session.tile_fill("/turf/open/floor", "/area/station").unwrap();
            assert_eq!(app.session.create_level(app.id, &fill), Ok(2));
            app.step();
            2
        } else {
            1
        };
        app.session
            .state
            .choose_prefab(Prefab::new(TreePath::parse("/obj/structure/table")));
        app.session.set_tool(Tool::Place);
        let table = app.session.place_at(Coord::new(5, 8, z), None).unwrap();
        app.session.set_tool(Tool::BlockSelect);
        app.step();

        app.drag(app.tile(5, 8), app.tile(6, 9));
        let selection = Selection::from_drag(Coord::new(5, 8, z), Coord::new(6, 9, z));
        assert_eq!(app.session.selection(), Some(selection), "created: {created}");

        let center = center_of(&app, selection);
        app.pointer(center, false);
        app.pointer(center, true);
        app.pointer([center[0] + 64.0, center[1]], true);
        let pending = app.view.gestures.block_placement.map(|placement| placement.target);
        assert_eq!(
            pending,
            Some(Selection::from_drag(Coord::new(7, 8, z), Coord::new(8, 9, z))),
            "created: {created}"
        );
        assert!(
            app.session
                .map_view_frame(
                    app.id,
                    Default::default(),
                    Default::default(),
                    Default::default(),
                    &[],
                    &[]
                )
                .and_then(|frame| frame.preview)
                .is_some_and(|preview| !preview.sprites.is_empty()),
            "created: {created}"
        );

        app.pointer([center[0] + 64.0, center[1]], false);
        assert!(app.view.gestures.block_placement.is_none());
        let location = |app: &RectangleUiHarness| {
            app.session
                .state
                .active_document()
                .unwrap()
                .instance_location(table)
                .map(|location| location.coord)
        };
        assert_eq!(location(&app), Some(Coord::new(7, 8, z)), "created: {created}");

        ctrl_key(&mut app, Key::X);
        assert_eq!(location(&app), None, "created: {created}");
        assert!(app.session.clipboard().is_some(), "created: {created}");
    }
}

#[test]
fn only_the_active_view_outlines_its_selected_instance() {
    let _guard = IMGUI_CONTEXT.lock().unwrap();
    let mut session = Session::new();
    session
        .load_environment(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env/test.dme"))
        .unwrap();
    let mut select_table_in = |name: &str| {
        let mut map = dmm::Map::new(Size { x: 4, y: 4, z: 1 });
        let key = map.intern_tile(vec![
            Prefab::new(TreePath::parse("/obj/structure/table")),
            Prefab::new(TreePath::parse("/turf/open/floor")),
            Prefab::new(TreePath::parse("/area/station")),
        ]);
        for row in &mut map.grid[0] {
            row.fill(key);
        }

        session.apply_map(crate::loader::LoadedMap {
            path: std::env::temp_dir().join(name),
            map,
            z: 1,
            errors: vec![],
            repo: None,
            conflict: None,
        });
        let id = session.state.active().unwrap();
        let table = session.state.document(id).unwrap().instance_ids_at(Coord::new(1, 1, 1))[0];
        session.select_instance(Some(table));

        (id, table)
    };
    let (left, _) = select_table_in("rmd-selection-left.dmm");
    let (right, table) = select_table_in("rmd-selection-right.dmm");
    assert!(session.selected_instance_of(left).is_some());
    session.set_tool(Tool::Select);

    let mut context = rectangle_context();
    let mut state = UiState::new(false).unwrap();
    let mut settings = Settings::default();
    let mut frame = |state: &mut UiState, session: &mut Session| {
        let ui = context.frame();
        for (index, id) in [left, right].into_iter().enumerate() {
            let name = format!("###viewport-{}", id.get());
            ui.set_window_pos_by_name(&name, [index as f32 * 400.0, 0.0]);
            ui.set_window_size_by_name(&name, [400.0, 600.0]);
        }

        let (views, ..) = state.draw_map_views(ui, session, &mut settings, false);
        assert!(context.render_legacy().valid());

        views
    };
    frame(&mut state, &mut session);
    state.map_views.get_mut(&right).unwrap().focus = true;
    let views = (0..3).map(|_| frame(&mut state, &mut session)).last().unwrap();
    assert_eq!(session.state.active(), Some(right));

    let outlined = views
        .iter()
        .map(|view| (view.document, view.interaction.selected))
        .collect::<Vec<_>>();
    assert_eq!(outlined, [(left, None), (right, Some(table))]);
}
