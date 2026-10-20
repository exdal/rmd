use core::types::Value;

use dmm::{Map, MapFormat, writer::MapWriter};

use super::*;
use crate::loader::LoadedMap;

#[test]
fn node_tool_hover_and_single_click_survive_the_full_view_frame() {
    let _guard = IMGUI_CONTEXT.lock().unwrap();
    let coord = Coord::new(5, 8, 1);
    let (mut session, seed) = node_session(node_map(20, 20, &[(coord, vec!["/obj/cable"])]), coord);
    session.set_tool(Tool::Node);
    let mut app = RectangleUiHarness::with_session(session);
    let point = app.tile(coord.x, coord.y);
    app.pointer(point, false);
    assert!(app.interaction.cursor.is_some());
    assert_eq!(app.interaction.mode, InteractionMode::Node { pick: None });
    assert!(app.session.node_overlay().is_none());

    app.pointer(point, true);
    assert!(app.interaction.cursor.is_some());
    assert_eq!(app.interaction.mode.pick(), Some(PickRequest::NodeSelect));
    assert!(!app.session.node_dragging());
    assert!(app.session.select_node_from_pick(render::PickResult::Hit(seed)));
    assert_eq!(app.session.selected_instance(), Some(seed));
    assert!(app.session.node_overlay().is_some());
}

#[test]
fn selection_keeps_its_double_click_node_shortcut() {
    let _guard = IMGUI_CONTEXT.lock().unwrap();
    let coord = Coord::new(5, 8, 1);
    let (mut session, _) = node_session(node_map(20, 20, &[(coord, vec!["/obj/cable"])]), coord);
    session.set_tool(Tool::Select);
    let mut app = RectangleUiHarness::with_session(session);
    let point = app.tile(coord.x, coord.y);
    app.pointer(point, false);
    app.pointer(point, true);
    assert_eq!(app.interaction.mode.pick(), Some(PickRequest::Select));
    app.pointer(point, false);
    app.pointer(point, true);
    assert_eq!(app.interaction.mode.pick(), Some(PickRequest::NodeSeed(coord)));
}

#[test]
fn node_handles_start_a_drag_instead_of_requesting_a_new_pick() {
    let _guard = IMGUI_CONTEXT.lock().unwrap();
    let coord = Coord::new(5, 8, 1);
    let (mut session, seed) = node_session(node_map(20, 20, &[(coord, vec!["/obj/cable"])]), coord);
    assert!(session.begin_node_edit(seed));
    let mut app = RectangleUiHarness::with_session(session);
    let point = app.tile(coord.x, coord.y);
    app.pointer(point, false);
    app.pointer(point, true);
    assert!(app.session.node_dragging());
    assert_eq!(app.interaction.mode.pick(), None);
    app.pointer(point, false);
    assert!(!app.session.node_dragging());
}

#[test]
fn right_click_captures_the_clicked_tile_in_every_tool() {
    let _guard = IMGUI_CONTEXT.lock().unwrap();
    for tool in [
        Tool::Select,
        Tool::Place,
        Tool::Node,
        Tool::BlockSelect,
        Tool::Delete,
        Tool::Fill,
    ] {
        let mut app = RectangleUiHarness::new();
        app.session.set_tool(tool);
        let point = app.tile(5, 8);
        app.context.io_mut().add_mouse_pos_event(point);
        app.context.io_mut().add_mouse_button_event(MouseButton::Right, false);
        app.step();
        app.context.io_mut().add_mouse_button_event(MouseButton::Right, true);
        app.step();
        let target = app.view.context.as_ref().expect("right click opens the map menu");
        assert_eq!(target.coord, Coord::new(5, 8, 1), "{tool:?}");
        assert_eq!(target.document, app.id);
        app.context.io_mut().add_mouse_button_event(MouseButton::Right, false);
        let next_tile = app.tile(6, 8);
        app.context.io_mut().add_mouse_pos_event(next_tile);
        app.step();
        assert_eq!(app.view.context.as_ref().unwrap().coord, Coord::new(5, 8, 1));
    }
}

#[test]
fn node_right_click_deletes_on_the_first_unfocused_click_and_shift_opens_the_menu() {
    let _guard = IMGUI_CONTEXT.lock().unwrap();
    let start = Coord::new(5, 8, 1);
    let middle = Coord::new(6, 8, 1);
    let end = Coord::new(7, 8, 1);
    let map = node_map(
        20,
        20,
        &[
            (start, vec!["/obj/cable"]),
            (middle, vec!["/obj/cable"]),
            (end, vec!["/obj/cable"]),
        ],
    );
    let (mut session, seed) = node_session(map, start);
    assert!(session.begin_node_edit(seed));
    let mut app = RectangleUiHarness::with_session(session);
    app.settings.focus_windows_on_hover = false;
    app.focus_other_window = true;
    app.view.focus = false;
    let point = app.tile(6, 8);
    app.context.io_mut().add_mouse_pos_event(point);
    app.context.io_mut().add_mouse_button_event(MouseButton::Right, false);
    app.step();

    app.context.io_mut().add_mouse_button_event(MouseButton::Right, true);
    app.step();
    assert!(!node_tile_has_group(&app.session, middle));
    assert!(app.view.context.is_none());
    assert_eq!(app.session.undo_label(), Some("delete node connection"));

    app.context.io_mut().add_mouse_button_event(MouseButton::Right, false);
    app.step();
    assert!(app.session.undo());
    assert!(app.session.begin_node_edit(seed));
    app.key(Key::ModShift, true);
    app.context.io_mut().add_mouse_button_event(MouseButton::Right, true);
    app.step();
    assert!(node_tile_has_group(&app.session, middle));
    assert!(matches!(
        app.view.context.as_ref().and_then(|target| target.node.as_ref()),
        Some(NodeContext::Connection(connection)) if connection.contains(&middle)
    ));
}

fn object_gizmo_frame(
    context: &mut dear_imgui_rs::Context, gizmo: &mut GizmoState, session: &mut Session, camera: &Controller,
    settings: &Settings,
) -> (crate::gizmo::GizmoResponse, usize) {
    let ui = context.frame();
    let view = GizmoMapView {
        min: [0.0; 2],
        max: [800.0, 600.0],
        hovered: true,
    };
    let response = ui
        .window("object-gizmo-z-level-test")
        .position([0.0; 2], Condition::Always)
        .size([800.0, 600.0], Condition::Always)
        .build(|| gizmo.draw(ui, session, settings, camera, TransformMode::Pixel, view))
        .unwrap();
    let draw_data = context.render_legacy();
    assert!(draw_data.valid());

    (response, draw_data.total_vtx_count())
}

#[test]
fn object_gizmo_only_appears_on_the_selected_instances_z_level() {
    let _guard = IMGUI_CONTEXT.lock().unwrap();
    let mut context = rectangle_context();
    let mut session = Session::new();
    let examples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env");
    session.load_environment(&examples.join("test.dme")).unwrap();
    let coord = Coord::new(6, 3, 1);
    let mut map = dmm::Map::new(Size { x: 8, y: 6, z: 2 });
    let base = map.intern_tile(vec![
        Prefab::new(TreePath::parse("/turf/open/floor")),
        Prefab::new(TreePath::parse("/area/station")),
    ]);
    let selected_tile = map.intern_tile(vec![
        Prefab::new(TreePath::parse("/obj/structure/table")),
        Prefab::new(TreePath::parse("/turf/open/floor")),
        Prefab::new(TreePath::parse("/area/station")),
    ]);
    for level in &mut map.grid {
        for row in level {
            row.fill(base);
        }
    }
    map.grid[0][2][5] = selected_tile;
    session.apply_map(crate::loader::LoadedMap {
        path: PathBuf::from("object-gizmo-z-level-test.dmm"),
        map,
        z: 1,
        errors: vec![],
        repo: None,
        conflict: None,
    });
    let selected = session.state.active_document().unwrap().instance_ids_at(coord)[0];
    session.select_instance(Some(selected));

    let mut camera = Controller::new();
    camera.resize(800, 600);
    camera.center_on_tile(coord, session.options.tile_size);
    let sprite = session.selected_transform().unwrap().sprite;
    let origin = camera.map_to_screen([sprite.x + sprite.width * 0.5, sprite.y + sprite.height * 0.5]);
    let mut gizmo = GizmoState::default();
    let settings = Settings::default();
    context.io_mut().add_mouse_pos_event(origin);
    context.io_mut().add_mouse_button_event(MouseButton::Left, true);

    let (visible, visible_vertices) = object_gizmo_frame(&mut context, &mut gizmo, &mut session, &camera, &settings);
    assert!(visible.captures_mouse);
    assert!(gizmo.is_interacting());

    assert_eq!(session.change_level(1), LevelChange::Changed);
    assert_eq!(session.selected_instance(), Some(selected));
    let (hidden, hidden_vertices) = object_gizmo_frame(&mut context, &mut gizmo, &mut session, &camera, &settings);
    assert!(!hidden.captures_mouse);
    assert!(!gizmo.is_interacting());
    assert!(visible_vertices > hidden_vertices);

    assert_eq!(session.change_level(-1), LevelChange::Changed);
    let (visible_again, restored_vertices) =
        object_gizmo_frame(&mut context, &mut gizmo, &mut session, &camera, &settings);
    assert!(visible_again.captures_mouse);
    assert!(restored_vertices > hidden_vertices);
}

#[test]
fn ctrl_dragging_an_object_onto_another_tile_keeps_its_area_last() {
    let _guard = IMGUI_CONTEXT.lock().unwrap();
    let mut context = rectangle_context();
    let mut session = Session::new();
    let examples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env");
    session.load_environment(&examples.join("test.dme")).unwrap();
    let start = Coord::new(6, 3, 1);
    let above = Coord::new(6, 4, 1);
    let floor = Prefab::new(TreePath::parse("/turf/open/floor"));
    let area = Prefab::new(TreePath::parse("/area/station"));
    let mut light = Prefab::new(TreePath::parse("/obj/machinery/light"));
    light.set_var("pixel_y".into(), Value::Num(32.0));
    let mut map = Map::new(Size { x: 8, y: 6, z: 1 });
    map.format = MapFormat::Tgm;
    let base = map.intern_tile(vec![floor.clone(), area.clone()]);
    let lit = map.intern_tile(vec![light, floor, area]);
    for row in &mut map.grid[0] {
        row.fill(base);
    }

    // DM counts `y` from the bottom
    map.grid[0][(map.size.y - start.y) as usize][(start.x - 1) as usize] = lit;
    session.apply_map(LoadedMap {
        path: PathBuf::from("object-gizmo-reanchor-test.dmm"),
        map,
        z: 1,
        errors: vec![],
        repo: None,
        conflict: None,
    });
    let selected = session.state.active_document().unwrap().instance_ids_at(start)[0];
    session.select_instance(Some(selected));

    let mut camera = Controller::new();
    camera.resize(800, 600);
    camera.center_on_tile(start, session.options.tile_size);
    let sprite = session.selected_transform().unwrap().sprite;
    let center = [sprite.x + sprite.width * 0.5, sprite.y + sprite.height * 0.5];
    let mut gizmo = GizmoState::default();
    let settings = Settings::default();
    context.io_mut().add_mouse_pos_event(camera.map_to_screen(center));
    context.io_mut().add_mouse_button_event(MouseButton::Left, true);
    object_gizmo_frame(&mut context, &mut gizmo, &mut session, &camera, &settings);
    assert!(gizmo.is_interacting());

    // ctrl rehomes the light onto the tile its sprite is drawn over
    context.io_mut().add_key_event(Key::ModCtrl, true);
    context
        .io_mut()
        .add_mouse_pos_event(camera.map_to_screen([center[0] + 1.0, center[1]]));
    object_gizmo_frame(&mut context, &mut gizmo, &mut session, &camera, &settings);
    context.io_mut().add_key_event(Key::ModCtrl, false);
    context.io_mut().add_mouse_button_event(MouseButton::Left, false);
    object_gizmo_frame(&mut context, &mut gizmo, &mut session, &camera, &settings);

    let document = session.state.active_document().unwrap();
    assert_eq!(
        document.instance_location(selected).map(|location| location.coord),
        Some(above)
    );
    let paths = document
        .map
        .tile_at(above)
        .unwrap()
        .iter()
        .map(|prefab| prefab.path.to_string())
        .collect::<Vec<_>>();
    assert_eq!(paths, ["/obj/machinery/light", "/turf/open/floor", "/area/station"]);

    // maplint only ends a var edit block with `\t},`, so a var edited atom can never close the tile
    let written = MapWriter::new(&document.to_map()).write();
    assert!(!written.contains("\t})"), "{written}");
}

#[test]
fn edit_menu_commands_reach_the_active_map_view() {
    let _guard = IMGUI_CONTEXT.lock().unwrap();
    let mut app = RectangleUiHarness::new();
    let table = Prefab::new(TreePath::parse("/obj/structure/table"));
    app.session.state.choose_prefab(table.clone());
    app.session.set_tool(Tool::Place);
    let coord = Coord::new(5, 8, 1);
    assert!(app.session.place_at(coord, None).is_some());
    app.session.set_tool(Tool::BlockSelect);
    assert!(
        app.session
            .select_block(Some(Selection::from_drag(coord, Coord::new(6, 9, 1))))
    );

    app.state.edit_command = Some(EditCommand::Copy);
    app.step();
    assert!(app.session.clipboard().is_some());

    app.state.edit_command = Some(EditCommand::Delete);
    app.step();
    assert!(!app.session.map().unwrap().tile_at(coord).unwrap().contains(&table));
    assert!(app.state.edit_command.is_none());

    app.state.edit_command = Some(EditCommand::Deselect);
    app.step();
    assert_eq!(app.session.selection(), None);
}

#[test]
fn edit_keys_work_while_another_panel_has_focus() {
    let _guard = IMGUI_CONTEXT.lock().unwrap();
    let mut app = RectangleUiHarness::new();
    app.settings.focus_windows_on_hover = false;
    app.focus_other_window = true;
    app.view.focus = false;
    let table = Prefab::new(TreePath::parse("/obj/structure/table"));
    app.session.state.choose_prefab(table.clone());
    app.session.set_tool(Tool::Place);
    let coord = Coord::new(5, 8, 1);
    assert!(app.session.place_at(coord, None).is_some());
    app.step();

    app.key(Key::ModCtrl, true);
    app.key(Key::Z, true);
    app.key(Key::Z, false);
    assert!(!app.session.map().unwrap().tile_at(coord).unwrap().contains(&table));

    app.key(Key::Y, true);
    app.key(Key::Y, false);
    app.key(Key::ModCtrl, false);
    assert!(app.session.map().unwrap().tile_at(coord).unwrap().contains(&table));
}

#[test]
fn edit_keys_leave_the_map_alone_while_a_modal_is_open() {
    let _guard = IMGUI_CONTEXT.lock().unwrap();
    let mut app = RectangleUiHarness::new();
    let table = Prefab::new(TreePath::parse("/obj/structure/table"));
    app.session.state.choose_prefab(table.clone());
    app.session.set_tool(Tool::Place);
    let coord = Coord::new(5, 8, 1);
    assert!(app.session.place_at(coord, None).is_some());
    app.is_modal_open = true;
    app.step();
    app.step();

    app.key(Key::ModCtrl, true);
    app.key(Key::Z, true);
    app.key(Key::Z, false);
    app.key(Key::ModCtrl, false);
    assert!(app.session.map().unwrap().tile_at(coord).unwrap().contains(&table));

    app.session.set_tool(Tool::BlockSelect);
    assert!(app.session.select_block(Some(Selection::from_drag(coord, coord))));
    app.key(Key::Delete, true);
    app.key(Key::Delete, false);
    assert!(app.session.map().unwrap().tile_at(coord).unwrap().contains(&table));
}

#[test]
fn a_tapped_tool_key_switches_tools_and_a_held_one_hands_the_old_tool_back() {
    let _guard = IMGUI_CONTEXT.lock().unwrap();
    let mut app = RectangleUiHarness::new();
    let table = Prefab::new(TreePath::parse("/obj/structure/table"));
    app.session.state.choose_prefab(table.clone());
    app.session.set_tool(Tool::Select);
    let tile = app.tile(5, 8);
    app.pointer(tile, false);

    app.key(Key::W, true);
    app.key(Key::W, false);
    assert_eq!(app.session.tool(), Tool::Place, "a tap keeps the new tool");

    app.session.set_tool(Tool::Select);
    app.key(Key::W, true);
    assert_eq!(app.session.tool(), Tool::Place);
    app.pointer(tile, true);
    app.pointer(tile, false);
    app.key(Key::W, false);
    assert_eq!(app.session.tool(), Tool::Select, "releasing after a click goes back");
    assert!(
        app.session
            .map()
            .unwrap()
            .tile_at(Coord::new(5, 8, 1))
            .unwrap()
            .contains(&table)
    );
}

#[test]
fn the_alternate_delete_drag_clears_each_tile_in_one_undo_step_with_any_bound_key() {
    let _guard = IMGUI_CONTEXT.lock().unwrap();
    for (bound, held, clears) in [
        (None, Key::ModAlt, true),
        (Some(Key::ModShift), Key::ModShift, true),
        (Some(Key::ModShift), Key::ModAlt, false),
    ] {
        let mut app = RectangleUiHarness::new();
        if let Some(key) = bound {
            app.settings
                .keybindings
                .rebind(KeybindAction::ToolAlternate, KeyBinding::new(key));
        }
        let table = Prefab::new(TreePath::parse("/obj/structure/table"));
        app.session.state.choose_prefab(table.clone());
        app.session.set_tool(Tool::Place);
        let coords = [Coord::new(5, 8, 1), Coord::new(6, 8, 1)];
        for coord in coords {
            assert!(app.session.place_at(coord, None).is_some());
        }
        let holds_table =
            |app: &RectangleUiHarness, coord| app.session.map().unwrap().tile_at(coord).unwrap().contains(&table);
        app.session.set_tool(Tool::Delete);

        app.key(held, true);
        app.pointer(app.tile(5, 8), true);
        app.pointer(app.tile(6, 8), true);
        app.pointer(app.tile(6, 8), false);
        app.key(held, false);
        assert_eq!(
            coords.iter().all(|coord| !holds_table(&app, *coord)),
            clears,
            "{bound:?} holding {held:?}"
        );

        if clears {
            assert!(app.session.undo());
            assert!(coords.iter().all(|coord| holds_table(&app, *coord)));
        }
    }
}
