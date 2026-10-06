use super::*;

#[test]
fn rectangle_ui_supports_drawing_resizing_filling_and_switching_to_the_bucket() {
    let _guard = IMGUI_CONTEXT.lock().unwrap();
    let mut app = RectangleUiHarness::new();
    app.pointer(app.tile(5, 8), false);
    app.key(Key::ModShift, true);
    app.key(Key::S, true);
    assert_eq!(app.session.tool(), Tool::BlockSelect);
    assert!(app.session.selection().is_none());
    app.key(Key::S, false);

    // Shift is still held from the shortcut, so the drag draws a border.
    app.pointer(app.tile(5, 8), true);
    app.pointer(app.tile(7, 10), true);
    app.pointer(app.tile(7, 10), false);
    app.key(Key::ModShift, false);
    let border = SelectionMask {
        bounds: Selection::from_drag(Coord::new(5, 8, 1), Coord::new(7, 10, 1)),
        mode: BlockSelectionMode::Hollow { line_width: 1 },
    };
    assert_eq!(app.session.selection_mask(), Some(border));
    assert_eq!(app.session.undo_label(), None);

    // Explicit mode controls change the current selection without painting.
    app.click(app.mode_buttons.unwrap().1);
    assert_eq!(app.session.selection_mode(), BlockSelectionMode::Full);
    app.click(app.mode_buttons.unwrap().0);
    assert_eq!(app.session.selection_mask(), Some(border));

    // A Shift-gizmo drag resizes bounds; releasing Shift mid-drag keeps the border.
    app.pointer(app.tile(6, 9), false);
    app.key(Key::ModShift, true);
    app.pointer(app.tile(6, 9), true);
    app.pointer(app.tile(8, 11), true);
    app.key(Key::ModShift, false);
    app.pointer(app.tile(8, 11), false);
    let resized = SelectionMask {
        bounds: Selection::from_drag(border.bounds.min, Coord::new(9, 12, 1)),
        ..border
    };
    assert_eq!(app.session.selection_mask(), Some(resized));
    assert!(app.view.gestures.block_placement.is_none());
    assert_eq!(app.session.undo_label(), None);

    let mut red = Prefab::new(TreePath::parse("/turf/open/floor"));
    red.set_var("color".into(), core::types::Value::Text("#ff0000".into()));
    app.session.state.choose_prefab(red.clone());
    app.step();
    app.click(app.fill_button.unwrap());
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
            .tile_at(Coord::new(7, 10, 1))
            .unwrap()
            .contains(&red)
    );

    app.pointer(app.tile(5, 8), false);
    app.key(Key::Q, true);
    app.key(Key::Q, false);
    assert_eq!(app.session.tool(), Tool::Fill);
    assert_eq!(app.session.selection_mask(), Some(resized));
    let mut blue = red.clone();
    blue.set_var("color".into(), core::types::Value::Text("#0000ff".into()));
    app.session.state.choose_prefab(blue.clone());
    app.click(app.tile(7, 10));
    assert!(
        !app.session
            .map()
            .unwrap()
            .tile_at(resized.bounds.min)
            .unwrap()
            .contains(&blue)
    );
    app.click(app.tile(5, 8));
    assert!(
        app.session
            .map()
            .unwrap()
            .tile_at(resized.bounds.min)
            .unwrap()
            .contains(&blue)
    );
    assert!(
        !app.session
            .map()
            .unwrap()
            .tile_at(Coord::new(4, 8, 1))
            .unwrap()
            .contains(&blue)
    );
    app.key(Key::Escape, true);
    app.key(Key::Escape, false);
    assert_eq!(app.session.selection_mask(), None);
    app.key(Key::ModCtrl, true);
    app.key(Key::Z, true);
    app.key(Key::Z, false);
    app.key(Key::ModCtrl, false);
    assert!(
        app.session
            .map()
            .unwrap()
            .tile_at(resized.bounds.min)
            .unwrap()
            .contains(&red)
    );
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

    // A corner handle resizes without Shift, and tool changes cancel the active resize.
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
    app.click(app.mode_buttons.unwrap().0);
    assert_eq!(
        app.state.block_selection_options.mode(),
        BlockSelectionMode::Hollow { line_width: 1 }
    );
}

#[test]
fn rectangle_ui_copies_a_border_to_another_map_and_keeps_it_hollow_after_paste() {
    let _guard = IMGUI_CONTEXT.lock().unwrap();
    let mut app = RectangleUiHarness::new();
    let source_id = app.id;
    app.session.set_tool(Tool::BlockSelect);
    app.key(Key::ModShift, true);
    app.pointer(app.tile(5, 8), true);
    app.pointer(app.tile(9, 14), true);
    app.pointer(app.tile(9, 14), false);
    app.key(Key::ModShift, false);
    let source = app.session.selection_mask().unwrap();
    assert_eq!(source.mode, BlockSelectionMode::Hollow { line_width: 1 });
    let mut red = Prefab::new(TreePath::parse("/turf/open/floor"));
    red.set_var("color".into(), core::types::Value::Text("#ff0000".into()));
    app.session.state.choose_prefab(red.clone());
    app.step();
    app.click(app.fill_button.unwrap());
    app.pointer(app.tile(5, 8), false);
    app.key(Key::ModCtrl, true);
    app.key(Key::C, true);
    app.key(Key::C, false);
    app.key(Key::ModCtrl, false);
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
    // Different destination settings and palette must not change what was copied.
    app.state.block_selection_options.full_rectangle = false;
    app.state.block_selection_options.line_width = 3;
    app.session.state.choose_prefab(blue.clone());
    for _ in 0..3 {
        app.step();
    }
    app.pointer(app.tile(12, 10), false);
    app.key(Key::ModCtrl, true);
    app.key(Key::V, true);
    app.key(Key::V, false);
    app.key(Key::ModCtrl, false);
    let pending = app
        .view
        .gestures
        .paste
        .expect("Ctrl+V starts a preview in the destination");
    let target = app
        .session
        .clipboard_selection_mask(pending.min, pending.rotation)
        .unwrap();
    assert_eq!(target.mode, source.mode);
    assert_eq!(
        app.session
            .clipboard_preview_sprites(target.bounds, pending.rotation)
            .len(),
        clipboard.filled().count()
    );
    assert_eq!(*app.session.map().unwrap(), destination_grid, "preview does not paint");
    app.key(Key::Enter, true);
    app.key(Key::Enter, false);
    assert!(app.view.gestures.paste.is_none());
    assert_eq!(app.session.selection_mask(), Some(target));
    for coord in target.bounds.iter() {
        let tile = app.session.map().unwrap().tile_at(coord).unwrap();
        assert_eq!(tile.contains(&red), target.includes(coord));
        assert_eq!(tile.contains(&blue), !target.includes(coord));
    }
    assert_eq!(app.session.state.document(source_id).unwrap().map, source_grid);
    app.key(Key::ModCtrl, true);
    app.key(Key::Z, true);
    app.key(Key::Z, false);
    app.key(Key::ModCtrl, false);
    assert_eq!(*app.session.map().unwrap(), destination_grid);
    assert_eq!(app.session.state.document(source_id).unwrap().map, source_grid);
    app.key(Key::ModCtrl, true);
    app.key(Key::Y, true);
    app.key(Key::Y, false);
    app.key(Key::ModCtrl, false);
    assert_eq!(app.session.selection_mask(), Some(target));

    // The next Fill selection must still act on the copied border only.
    let mut green = blue.clone();
    green.set_var("color".into(), core::types::Value::Text("#00ff00".into()));
    app.session.state.choose_prefab(green.clone());
    app.step();
    app.click(app.fill_button.unwrap());
    for coord in target.bounds.iter() {
        assert_eq!(
            app.session.map().unwrap().tile_at(coord).unwrap().contains(&green),
            target.includes(coord)
        );
    }
}

fn gizmo_frame(
    context: &mut dear_imgui_rs::Context, gizmo: &mut GizmoState, camera: &Controller, settings: &Settings,
    target: BlockGizmoTarget, hovered: bool,
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
            let response = gizmo.draw_block(ui, settings, camera, target, view);
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
fn resize_and_move_gestures_keep_their_mouse_down_action_when_shift_changes() {
    let _guard = IMGUI_CONTEXT.lock().unwrap();
    for resize in [true, false] {
        let mut context = rectangle_context();
        let mut camera = Controller::new();
        camera.resize(800, 600);
        camera.center_on_tile(Coord::new(5, 5, 1), 32);
        let selection = Selection::from_drag(Coord::new(3, 3, 1), Coord::new(5, 5, 1));
        let bounds = block_selection_bounds(&camera, [0.0; 2], selection, 32);
        let origin = [
            (bounds.min[0] + bounds.max[0]) * 0.5,
            (bounds.min[1] + bounds.max[1]) * 0.5,
        ];
        let mut target = BlockGizmoTarget {
            selection,
            rotation: SelectionRotation::Original,
            map_size: Size { x: 10, y: 10, z: 1 },
            tile_size: 32,
            kind: BlockGizmoKind::Selection,
        };
        let mut gizmo = GizmoState::default();
        let settings = Settings::default();
        context.io_mut().add_mouse_pos_event(origin);
        context.io_mut().add_key_event(Key::ModShift, resize);
        context.io_mut().add_mouse_button_event(MouseButton::Left, true);
        let start = gizmo_frame(&mut context, &mut gizmo, &camera, &settings, target, true);
        assert_eq!(start.resizing, resize);
        context.io_mut().add_key_event(Key::ModShift, !resize);
        context
            .io_mut()
            .add_mouse_pos_event([origin[0] + 32.0, origin[1] - 32.0]);
        let dragged = gizmo_frame(&mut context, &mut gizmo, &camera, &settings, target, true);
        assert_eq!(dragged.resizing, resize);
        assert_eq!(dragged.selection.width(), if resize { 4 } else { 3 });
        assert_eq!(
            dragged.selection.min,
            if resize { selection.min } else { Coord::new(4, 4, 1) }
        );
        target.selection = dragged.selection;
        context.io_mut().add_mouse_button_event(MouseButton::Left, false);
        context.io_mut().add_mouse_pos_event([2000.0, -2000.0]);
        let released = gizmo_frame(&mut context, &mut gizmo, &camera, &settings, target, false);
        assert_eq!(released.resizing, resize);
        assert!(released.captures_mouse);
        assert!(!gizmo.is_interacting());
        assert_eq!(released.selection.max, Coord::new(10, 10, 1));
    }
}

#[test]
fn rotation_shortcuts_work_with_both_presets_and_a_custom_modifier_binding() {
    let _guard = IMGUI_CONTEXT.lock().unwrap();
    for (preset, modifier, key, custom) in [
        (KeybindPreset::Default, None, Key::R, false),
        (KeybindPreset::StrongDmm, Some(Key::ModShift), Key::R, false),
        (KeybindPreset::Default, Some(Key::ModCtrl), Key::T, true),
    ] {
        let mut context = rectangle_context();
        let mut settings = Settings {
            keybindings: preset.bindings(),
            ..Settings::default()
        };
        if custom {
            settings
                .keybindings
                .rebind(KeybindAction::Rotate, KeyBinding::with_ctrl(Key::T));
        }
        let mut camera = Controller::new();
        camera.resize(800, 600);
        let mut gizmo = GizmoState::default();
        let target = BlockGizmoTarget {
            selection: Selection::from_drag(Coord::new(2, 2, 1), Coord::new(4, 5, 1)),
            rotation: SelectionRotation::Original,
            map_size: Size { x: 10, y: 10, z: 1 },
            tile_size: 32,
            kind: BlockGizmoKind::Selection,
        };
        context.io_mut().add_mouse_pos_event([400.0, 300.0]);
        if let Some(modifier) = modifier {
            context.io_mut().add_key_event(modifier, true);
        }
        context.io_mut().add_key_event(key, true);
        let pressed = gizmo_frame(&mut context, &mut gizmo, &camera, &settings, target, true);
        assert!(!pressed.resizing);
        context.io_mut().add_key_event(key, false);
        let released = gizmo_frame(&mut context, &mut gizmo, &camera, &settings, target, true);
        assert_eq!(released.rotation, SelectionRotation::Clockwise);
        assert_eq!(released.selection, target.selection);
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

        app.pointer(app.tile(5, 8), false);
        app.pointer(app.tile(5, 8), true);
        app.pointer(app.tile(6, 9), true);
        app.pointer(app.tile(6, 9), false);
        let selection = Selection::from_drag(Coord::new(5, 8, z), Coord::new(6, 9, z));
        assert_eq!(app.session.selection(), Some(selection), "created: {created}");

        let center = [
            (app.tile(5, 8)[0] + app.tile(6, 9)[0]) * 0.5,
            (app.tile(5, 8)[1] + app.tile(6, 9)[1]) * 0.5,
        ];
        app.pointer(center, false);
        app.pointer(center, true);
        app.pointer([center[0] + 64.0, center[1]], true);
        app.pointer([center[0] + 64.0, center[1]], false);
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

        app.key(Key::Enter, true);
        app.key(Key::Enter, false);
        let location = |app: &RectangleUiHarness| {
            app.session
                .state
                .active_document()
                .unwrap()
                .instance_location(table)
                .map(|location| location.coord)
        };
        assert_eq!(location(&app), Some(Coord::new(7, 8, z)), "created: {created}");

        app.key(Key::ModCtrl, true);
        app.key(Key::X, true);
        app.key(Key::X, false);
        app.key(Key::ModCtrl, false);
        assert_eq!(location(&app), None, "created: {created}");
        assert!(app.session.clipboard().is_some(), "created: {created}");
    }
}
