use base::{coord::Coord, path::TreePath, types::Value};

use prefab::{Prefab, PrefabInstanceId};

use crate::{
    document::{command::EditGroupId, grid::TileView},
    session::{
        FillOutcome, Session,
        fixtures::{area_at, examples, flat_session, focus_session},
    },
    tool::{FillMode, SelectionRotation, Tool},
};

#[test]
fn the_replace_tool_swaps_a_picked_atom_for_a_brush_of_the_same_kind() -> Result<(), &'static str> {
    let mut session = flat_session(2, 1);
    let document = session.state.active().unwrap();
    let coord = Coord::new(1, 1, 1);
    session
        .state
        .choose_prefab(Prefab::new(TreePath::parse("/obj/structure/table")));
    session.set_tool(Tool::Place);
    let table = session.place_at(coord, None).unwrap();
    let light = Prefab::new(TreePath::parse("/obj/machinery/light"));
    session.state.choose_prefab(light.clone());

    assert!(
        !session.replace_instance(table),
        "only the Replace tool replaces what it picks"
    );
    session.set_tool(Tool::Replace);
    assert!(!session.replace_instance(session.turf_at(document, coord).unwrap()));
    assert!(session.replace_instance(table));
    let prefab = session
        .state
        .active_document()
        .ok_or("active document is missing")?
        .prefab_instance(table)
        .ok_or("prefab instance is missing")?
        .prefab();
    assert_eq!(prefab, &light, "the placement keeps its id");
    Ok(())
}

#[test]
fn area_edits_retain_the_seed_component_or_clear_an_invalid_seed() {
    let mut session = focus_session();
    let seed = Coord::new(1, 1, 1);
    let neighbor = Coord::new(2, 1, 1);
    session.toggle_focus_at(Some(seed));

    let neighbor_area = area_at(&session, neighbor).unwrap();
    session.pick_instance(Some(neighbor_area));
    assert_eq!(
        session.set_picked_instance_var("name".into(), Value::Text(String::from("Engineering"))),
        Some(true),
    );
    assert!(session.focused_area().is_some());
    assert!(session.can_edit_at(seed));
    assert!(!session.can_edit_at(neighbor));

    let seed_area = area_at(&session, seed).unwrap();
    session.pick_instance(Some(seed_area));
    assert_eq!(
        session.set_picked_instance_var("name".into(), Value::Text(String::from("Engineering"))),
        Some(true),
    );
    assert_eq!(session.focused_area(), None);
}

#[test]
fn editing_an_area_updates_component_membership_incrementally() {
    let root = examples();
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).unwrap();
    session.open_map(&root.join("test.dmm"), 1).unwrap();
    let area = area_at(&session, Coord::new(3, 3, 1)).unwrap();
    session.pick_instance(Some(area));
    let revision = session.revision();

    assert_eq!(
        session.set_picked_instance_var("name".into(), Value::Text(String::from("Engineering"))),
        Some(true),
    );
    assert_eq!(session.revision(), revision.wrapping_add(1));
    let update = session.frame_update().expect("area edit must remain incremental");
    assert_eq!(update.previous_revision, revision);
    assert!(!update.sprites.is_empty());
}

#[test]
fn fill_commits_through_the_session_and_updates_rendered_turfs() {
    let root = examples();
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).unwrap();
    session.open_map(&root.join("test.dmm"), 1).unwrap();
    let coord = Coord::new(2, 2, 1);
    let boundary = Coord::new(1, 1, 1);
    let turf = session.tree().unwrap().roots().turf.unwrap();
    let turf_id = session
        .state
        .active_document()
        .unwrap()
        .map
        .tile_at(coord)
        .unwrap()
        .iter()
        .zip(session.state.active_document().unwrap().instance_ids_at(coord))
        .find_map(|(prefab, id)| {
            let candidate = session.tree().unwrap().id_of(&prefab.path)?;

            session.tree().unwrap().is_subtype_of(candidate, turf).then_some(*id)
        })
        .unwrap();
    let before_sprite = *session.instances().unwrap().sprite(turf_id).unwrap();
    let mut floor = Prefab::new(TreePath::parse("/turf/open/floor"));
    floor.set_var("color".into(), Value::Text("#ff0000".into()));
    session.state.choose_prefab(floor.clone());

    assert_eq!(session.fill_at(coord, FillMode::Wall, &[]), FillOutcome::NoChange);
    session.set_tool(Tool::Fill);
    let revision = session.revision();
    assert_eq!(session.fill_at(coord, FillMode::Wall, &[]), FillOutcome::Applied);

    assert_eq!(session.revision(), revision.wrapping_add(1));
    assert!(session.frame_update().is_some());
    assert_ne!(*session.instances().unwrap().sprite(turf_id).unwrap(), before_sprite);
    assert_eq!(
        session
            .map()
            .unwrap()
            .tile_at(coord)
            .unwrap()
            .iter()
            .find(|prefab| prefab.path == floor.path)
            .unwrap(),
        &floor
    );
    assert_eq!(
        session
            .map()
            .unwrap()
            .tile_at(boundary)
            .unwrap()
            .iter()
            .find(|prefab| prefab.path.to_string().starts_with("/turf"))
            .unwrap()
            .path,
        TreePath::parse("/turf/closed/wall")
    );
    assert_eq!(session.palette(), Some(&floor));
}

#[test]
fn delete_tool_removes_the_target_instance_and_its_cached_sprites() {
    let root = examples();
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).unwrap();
    session.open_map(&root.join("test.dmm"), 1).unwrap();
    let coord = Coord::new(6, 3, 1);
    let target = session
        .state
        .active_document()
        .and_then(|document| document.instance_ids_at(coord).first())
        .copied()
        .unwrap();
    let before_len = session.map().unwrap().tile_at(coord).unwrap().len();
    session.pick_instance(Some(target));

    assert!(!session.delete_instance(target));
    assert_eq!(session.picked_instance(), Some(target));

    session.set_tool(Tool::Delete);
    let revision = session.revision();
    assert!(session.delete_instance(target));
    assert_eq!(session.map().unwrap().tile_at(coord).unwrap().len(), before_len - 1);
    assert_eq!(session.picked_instance(), None);
    assert_eq!(session.state.active_document().unwrap().instance_location(target), None);
    assert!(
        session
            .instances()
            .unwrap()
            .sprites
            .iter()
            .all(|sprite| sprite.owner != target)
    );
    assert_eq!(session.revision(), revision.wrapping_add(1));
    assert!(!session.delete_instance(target));
}

#[test]
fn grouped_placements_are_undone_and_redone_as_one_stroke() {
    let root = examples();
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).unwrap();
    session.open_map(&root.join("test.dmm"), 1).unwrap();
    let table = session
        .tree()
        .unwrap()
        .id_of(&TreePath::parse("/obj/structure/table"))
        .unwrap();
    let first = Coord::new(2, 2, 1);
    let second = Coord::new(3, 2, 1);
    let first_before = session.state.active_document().unwrap().placed_tile(first).unwrap();
    let second_before = session.state.active_document().unwrap().placed_tile(second).unwrap();

    assert!(session.choose_type(table));
    let group = EditGroupId::new();
    session.place_at(first, Some(group)).unwrap();
    session.place_at(second, Some(group)).unwrap();
    let first_after = session.state.active_document().unwrap().placed_tile(first).unwrap();
    let second_after = session.state.active_document().unwrap().placed_tile(second).unwrap();

    let document = session.state.active_document_mut().unwrap();
    assert!(document.undo());
    assert_eq!(document.placed_tile(first), Some(first_before));
    assert_eq!(document.placed_tile(second), Some(second_before));
    assert!(!document.undo());

    assert!(document.redo());
    assert_eq!(document.placed_tile(first), Some(first_after));
    assert_eq!(document.placed_tile(second), Some(second_after));
    assert!(!document.redo());
}

#[test]
fn turf_placement_reuses_the_existing_id_and_picks_preserve_overrides() {
    let root = examples();
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).unwrap();
    session.open_map(&root.join("test.dmm"), 1).unwrap();
    let coord = Coord::new(2, 2, 1);
    let turf_root = session.tree().unwrap().roots().turf.unwrap();
    let turf_id = session
        .state
        .active_document()
        .unwrap()
        .map
        .tile_at(coord)
        .unwrap()
        .iter()
        .zip(session.state.active_document().unwrap().instance_ids_at(coord))
        .find_map(|(prefab, id)| {
            let candidate = session.tree().unwrap().id_of(&prefab.path)?;

            session
                .tree()
                .unwrap()
                .is_subtype_of(candidate, turf_root)
                .then_some(*id)
        })
        .unwrap();
    let wall = session
        .tree()
        .unwrap()
        .id_of(&TreePath::parse("/turf/closed/wall"))
        .unwrap();

    assert!(session.choose_type(wall));
    assert_eq!(session.place_at(coord, None), Some(turf_id));
    assert_eq!(session.place_at(coord, None), None);
    assert_eq!(session.picked_instance(), Some(turf_id));
    assert_eq!(
        session.picked_prefab().unwrap().path,
        TreePath::parse("/turf/closed/wall")
    );

    assert_eq!(
        session.set_picked_instance_var("name".into(), Value::Text("custom wall".into())),
        Some(true)
    );
    session.set_tool(Tool::Pick);
    session.pick_instance(Some(turf_id));
    assert_eq!(session.tool(), Tool::Pick);
    assert_eq!(
        session.palette().unwrap().var(&"name".into()),
        Some(&Value::Text("custom wall".into()))
    );
    assert_eq!(session.recent_prefabs().len(), 2);
}

#[test]
fn context_atom_actions_preserve_ids_and_undo() -> Result<(), &'static str> {
    let mut session = flat_session(4, 4);
    let coord = Coord::new(2, 2, 1);
    let table = TreePath::parse("/obj/structure/table");
    let light = TreePath::parse("/obj/machinery/light");
    let table_type = session.tree().unwrap().id_of(&table).unwrap();
    let light_type = session.tree().unwrap().id_of(&light).unwrap();
    assert!(session.choose_type(table_type));
    let first = session.place_at(coord, None).unwrap();
    assert!(session.choose_type(light_type));
    let second = session.place_at(coord, None).unwrap();
    assert_eq!(
        session.state.active_document().unwrap().instance_ids_at(coord)[0..2],
        [first, second]
    );

    assert!(session.reorder_instance(first, true));
    assert_eq!(
        session.state.active_document().unwrap().instance_ids_at(coord)[0..2],
        [second, first]
    );
    assert!(session.undo());
    assert_eq!(
        session.state.active_document().unwrap().instance_ids_at(coord)[0..2],
        [first, second]
    );

    session.pick_instance(Some(first));
    assert_eq!(
        session.set_picked_instance_var("name".into(), Value::Text("custom table".into())),
        Some(true)
    );
    assert!(session.reset_instance_to_default(first));
    assert!(
        session
            .state
            .active_document()
            .unwrap()
            .prefab_instance(first)
            .ok_or("prefab instance is missing")?
            .prefab()
            .vars
            .is_empty()
    );
    assert!(session.undo());
    assert_eq!(
        session
            .state
            .active_document()
            .unwrap()
            .prefab_instance(first)
            .ok_or("prefab instance is missing")?
            .prefab()
            .var(&"name".into()),
        Some(&Value::Text("custom table".into()))
    );
    assert!(session.replace_context_instance(first, light.clone()));
    let prefab = session
        .state
        .active_document()
        .ok_or("active document is missing")?
        .prefab_instance(first)
        .ok_or("prefab instance is missing")?
        .prefab();
    assert_eq!(prefab.path, light);
    assert!(prefab.vars.is_empty());
    assert!(!session.replace_context_instance(first, TreePath::parse("/turf/open/floor")));
    assert!(session.undo());
    assert_eq!(
        session
            .state
            .active_document()
            .unwrap()
            .prefab_instance(first)
            .ok_or("prefab instance is missing")?
            .prefab()
            .path,
        table
    );
    assert!(session.delete_context_instance(first));
    assert!(
        session
            .state
            .active_document()
            .unwrap()
            .instance_location(first)
            .is_none()
    );
    assert!(session.undo());
    assert!(
        session
            .state
            .active_document()
            .unwrap()
            .instance_location(first)
            .is_some()
    );
    Ok(())
}

#[test]
fn context_reorder_changes_equal_layer_draw_order_through_undo_and_redo() {
    let mut session = flat_session(2, 2);
    let coord = Coord::new(1, 1, 1);
    let table = session
        .tree()
        .unwrap()
        .id_of(&TreePath::parse("/obj/structure/table"))
        .unwrap();
    assert!(session.choose_type(table));
    let first = session.place_at(coord, None).unwrap();
    let second = session.place_at(coord, None).unwrap();
    let drawn = |session: &Session| {
        session
            .instances()
            .unwrap()
            .sprites
            .iter()
            .filter(|sprite| sprite.owner == first || sprite.owner == second)
            .map(|sprite| sprite.owner)
            .collect::<Vec<_>>()
    };
    assert_eq!(drawn(&session), vec![first, second]);
    let revision = session.revision();
    assert!(session.reorder_instance(first, true));
    assert_eq!(session.revision(), revision.wrapping_add(1));
    assert_eq!(drawn(&session), vec![second, first]);
    let revision = session.revision();
    assert!(session.undo());
    assert_eq!(session.revision(), revision.wrapping_add(1));
    assert_eq!(drawn(&session), vec![first, second]);
    let revision = session.revision();
    assert!(session.redo());
    assert_eq!(session.revision(), revision.wrapping_add(1));
    assert_eq!(drawn(&session), vec![second, first]);
}

#[test]
fn context_edits_obey_area_focus_without_blocking_copy() {
    let mut session = focus_session();
    let inside = Coord::new(1, 1, 1);
    let outside = Coord::new(4, 1, 1);
    let table = session
        .tree()
        .unwrap()
        .id_of(&TreePath::parse("/obj/structure/table"))
        .unwrap();
    assert!(session.choose_type(table));
    let target = session.place_at(outside, None).unwrap();
    let before = session.map().unwrap().tile_at(outside).unwrap().to_tile();
    session.toggle_focus_at(Some(inside));
    assert!(session.copy_tile(outside));
    assert!(!session.can_clear_tile(outside));
    assert!(!session.cut_tile(outside));
    assert!(!session.delete_tile(outside, None));
    assert!(!session.delete_context_instance(target));
    assert!(!session.reset_instance_to_default(target));
    assert!(!session.replace_context_instance(target, TreePath::parse("/obj/machinery/light")));
    assert!(!session.can_paste_clipboard(outside, SelectionRotation::Original));
    assert_eq!(
        session.map().unwrap().tile_at(outside).map(TileView::to_tile),
        Some(before.clone())
    );
}

fn table() -> Prefab {
    Prefab::new(TreePath::parse("/obj/structure/table"))
}

fn place(session: &mut Session, prefab: Prefab, coord: Coord) {
    session.state.choose_prefab(prefab);
    session.set_tool(Tool::Place);
    assert!(session.place_at(coord, None).is_some());
}

fn atom_at(session: &Session, coord: Coord) -> PrefabInstanceId {
    session.state.active_document().unwrap().instance_ids_at(coord)[0]
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

#[test]
fn deleting_picks_is_one_undo_and_labels_the_count() {
    let mut session = flat_session(3, 1);
    place(&mut session, table(), Coord::new(1, 1, 1));
    place(
        &mut session,
        Prefab::new(TreePath::parse("/obj/machinery/light")),
        Coord::new(1, 1, 1),
    );
    place(&mut session, table(), Coord::new(3, 1, 1));
    let ids = session
        .state
        .active_document()
        .unwrap()
        .instance_ids_at(Coord::new(1, 1, 1))
        .to_vec();
    let far = atom_at(&session, Coord::new(3, 1, 1));
    let before = session.map().unwrap().clone();
    session.set_tool(Tool::Pick);
    session.pick_instance(None);
    for id in [ids[0], ids[1], far] {
        session.toggle_pick(id);
    }

    assert!(session.delete_picked_instances());
    assert_eq!(session.undo_label(), Some("delete 3 atoms"));
    assert_eq!(
        paths_at(&session, Coord::new(1, 1, 1)),
        ["/turf/open/floor", "/area/station"]
    );
    assert_eq!(
        paths_at(&session, Coord::new(3, 1, 1)),
        ["/turf/open/floor", "/area/station"]
    );

    assert!(session.undo());
    assert_eq!(*session.map().unwrap(), before);
}

#[test]
fn deleting_one_pick_keeps_its_path_label() {
    let mut session = flat_session(2, 1);
    place(&mut session, table(), Coord::new(1, 1, 1));
    let id = atom_at(&session, Coord::new(1, 1, 1));
    session.set_tool(Tool::Pick);
    session.pick_instance(Some(id));

    assert!(session.delete_picked_instances());
    assert_eq!(session.undo_label(), Some("delete /obj/structure/table"));
}

#[test]
fn deleting_picks_skips_what_the_area_focus_protects() {
    let mut session = focus_session();
    place(&mut session, table(), Coord::new(1, 1, 1));
    place(&mut session, table(), Coord::new(3, 1, 1));
    let near = atom_at(&session, Coord::new(1, 1, 1));
    let far = atom_at(&session, Coord::new(3, 1, 1));
    session.toggle_focus_at(Some(Coord::new(1, 1, 1)));
    session.set_tool(Tool::Pick);
    session.pick_instance(None);
    session.toggle_pick(near);
    session.toggle_pick(far);

    assert!(session.delete_picked_instances());
    assert_eq!(
        paths_at(&session, Coord::new(1, 1, 1)),
        ["/turf/open/floor", "/area/station"]
    );
    assert_eq!(
        paths_at(&session, Coord::new(3, 1, 1)),
        ["/obj/structure/table", "/turf/open/floor", "/area/station"]
    );
    assert_eq!(session.undo_label(), Some("delete /obj/structure/table"));
}
