use base::{path::TreePath, types::Value};

use dmi::metadata::Dir;
use dmm::{Coord, Prefab};

use super::EditScope;
use crate::{
    document::{PrefabInstanceId, command::EditGroupId, instance::InstanceChange, instance::VarMutation},
    session::{
        Session,
        fixtures::{assert_render_cache_matches_rebuild, examples, flat_session, focus_session},
    },
    tool::{SelectionRotation, Tool},
};

fn place(session: &mut Session, placements: &[(u32, &Prefab)]) -> Vec<PrefabInstanceId> {
    session.set_tool(Tool::Place);
    placements
        .iter()
        .map(|(x, prefab)| {
            session.state.choose_prefab((*prefab).clone());
            session.place_at(Coord::new(*x, 1, 1), None).unwrap()
        })
        .collect()
}

fn names(session: &Session, ids: &[PrefabInstanceId]) -> Vec<Option<Value>> {
    let document = session.state.active_document().unwrap();

    ids.iter()
        .filter_map(|id| document.prefab_instance(*id))
        .map(|instance| instance.prefab().var(&"name".into()).cloned())
        .collect()
}

fn set_name(name: &str) -> [VarMutation; 1] {
    [VarMutation::Set("name".into(), Value::Text(name.to_string()))]
}

fn three_tables() -> (Session, Vec<PrefabInstanceId>) {
    let mut session = flat_session(3, 1);
    let table = Prefab::new(TreePath::parse("/obj/structure/table"));
    let ids = place(&mut session, &[(1, &table), (2, &table), (3, &table)]);
    session.set_tool(Tool::Pick);
    session.pick_instance(None);

    (session, ids)
}

#[test]
fn toggling_adds_removes_and_hands_the_primary_back() {
    let (mut session, ids) = three_tables();

    session.toggle_pick(ids[0]);
    session.toggle_pick(ids[1]);
    assert_eq!(session.picked_instances(), vec![ids[0], ids[1]]);
    assert_eq!(session.picked_instance(), Some(ids[1]));

    session.toggle_pick(ids[1]);
    assert_eq!(session.picked_instances(), vec![ids[0]]);
    assert_eq!(session.picked_instance(), Some(ids[0]));

    session.toggle_pick(ids[0]);
    assert!(session.picked_instances().is_empty());
    assert_eq!(session.picked_instance(), None);
}

#[test]
fn picking_replaces_or_clears_the_whole_set() {
    let (mut session, ids) = three_tables();
    session.toggle_pick(ids[0]);
    session.toggle_pick(ids[1]);

    session.pick_instance(Some(ids[2]));
    assert_eq!(session.picked_instances(), vec![ids[2]]);

    session.toggle_pick(ids[0]);
    session.pick_instance(None);
    assert!(session.picked_instances().is_empty());
}

#[test]
fn a_level_change_clears_the_set() {
    let (mut session, ids) = three_tables();
    session.toggle_pick(ids[0]);
    session.toggle_pick(ids[1]);
    let document = session.state.active().unwrap();

    session.create_level(document, &[]).unwrap();

    assert!(session.picked_instances().is_empty());
}

#[test]
fn deleting_a_picked_atom_prunes_it_from_the_set() {
    let (mut session, ids) = three_tables();
    session.toggle_pick(ids[0]);
    session.toggle_pick(ids[1]);

    assert!(session.delete_context_instance(ids[1]));
    assert_eq!(session.picked_instances(), vec![ids[0]]);
    assert_eq!(session.picked_instance(), Some(ids[0]));

    assert!(session.undo());
    assert_eq!(session.picked_instances(), vec![ids[0]]);

    assert!(session.delete_context_instance(ids[0]));
    assert!(session.undo());
    assert!(session.picked_instances().is_empty());
}

#[test]
fn a_selected_scope_edit_changes_every_picked_atom_in_one_undo() {
    let (mut session, ids) = three_tables();
    for id in &ids {
        session.toggle_pick(*id);
    }
    let before = names(&session, &ids);

    assert_eq!(
        session.edit_instance_vars_in(EditScope::Selected, "set name", &set_name("Oak"), None),
        Some(true)
    );
    assert_eq!(names(&session, &ids), vec![Some(Value::Text(String::from("Oak"))); 3]);

    assert!(session.undo());
    assert_eq!(names(&session, &ids), before);
}

#[test]
fn an_identical_scope_edit_changes_every_matching_placement_in_one_undo() {
    let mut session = flat_session(4, 1);
    let table = Prefab::new(TreePath::parse("/obj/structure/table"));
    let mut other = table.clone();
    other.set_var("name".into(), Value::Text(String::from("Other")));
    let ids = place(&mut session, &[(1, &table), (2, &table), (3, &other), (4, &table)]);
    session.pick_instance(Some(ids[0]));
    session.refresh_identical();
    assert_eq!(session.identical().len(), 3);

    assert_eq!(
        session.edit_instance_vars_in(EditScope::Identical, "set name", &set_name("Oak"), None),
        Some(true)
    );

    let oak = Some(Value::Text(String::from("Oak")));
    let other_name = Some(Value::Text(String::from("Other")));
    assert_eq!(
        names(&session, &ids),
        [oak.clone(), oak.clone(), other_name.clone(), oak]
    );
    assert_eq!(
        session.state.active_document().unwrap().undo_label(),
        Some("set name on 3 instances")
    );
    assert_render_cache_matches_rebuild(&session);

    session.undo();
    assert_eq!(names(&session, &ids), [None, None, other_name, None]);
}

#[test]
fn identical_placements_refresh_as_the_map_and_focus_change() {
    let mut session = focus_session();
    let table = Prefab::new(TreePath::parse("/obj/structure/table"));
    let ids = place(&mut session, &[(1, &table), (4, &table)]);
    session.pick_instance(Some(ids[0]));
    session.refresh_identical();
    assert_eq!(session.identical(), [ids[0], ids[1]]);

    let added = place(&mut session, &[(2, &table)]);
    session.pick_instance(Some(ids[0]));
    session.refresh_identical();
    assert_eq!(session.identical().len(), 3, "a new placement joins");

    session.toggle_focus_at(Some(Coord::new(1, 1, 1)));
    session.refresh_identical();
    assert_eq!(
        session.identical(),
        [ids[0], added[0]],
        "focus leaves out what it protects"
    );

    session.edit_instance_vars_in(EditScope::Selected, "set name", &set_name("Oak"), None);
    session.refresh_identical();
    assert_eq!(
        session.identical(),
        [ids[0]],
        "the edited one no longer matches the others"
    );
}

#[test]
fn identical_bounds_cover_the_other_placements_on_the_current_level() {
    let mut session = flat_session(3, 1);
    let table = Prefab::new(TreePath::parse("/obj/structure/table"));
    let ids = place(&mut session, &[(1, &table), (3, &table)]);
    session.pick_instance(Some(ids[0]));
    session.refresh_identical();

    let tile = session.options.tile_size as f32;
    assert_eq!(session.identical_bounds(), [[tile * 2.0, 0.0, tile, tile]]);
}

#[test]
fn a_selected_scope_edit_leaves_identical_placements_alone() {
    let mut session = flat_session(2, 1);
    let table = Prefab::new(TreePath::parse("/obj/structure/table"));
    let ids = place(&mut session, &[(1, &table), (2, &table)]);
    session.pick_instance(Some(ids[0]));

    session.edit_instance_vars_in(EditScope::Selected, "set name", &set_name("Oak"), None);

    assert_eq!(names(&session, &ids), [Some(Value::Text(String::from("Oak"))), None]);
}

#[test]
fn an_identical_scope_edit_skips_placements_outside_the_focused_area() {
    let mut session = focus_session();
    let table = Prefab::new(TreePath::parse("/obj/structure/table"));
    let ids = place(&mut session, &[(1, &table), (2, &table), (4, &table)]);
    session.toggle_focus_at(Some(Coord::new(1, 1, 1)));
    session.pick_instance(Some(ids[0]));

    assert_eq!(
        session.edit_instance_vars_in(EditScope::Identical, "set name", &set_name("Oak"), None),
        Some(true)
    );

    let oak = Some(Value::Text(String::from("Oak")));
    assert_eq!(names(&session, &ids), [oak.clone(), oak, None]);
}

fn coords(session: &Session, ids: &[PrefabInstanceId]) -> Vec<Coord> {
    let document = session.state.active_document().unwrap();

    ids.iter()
        .map(|id| document.instance_location(*id).unwrap().coord)
        .collect()
}

fn change(id: PrefabInstanceId, to_coord: Option<Coord>, mutations: &[VarMutation]) -> InstanceChange {
    InstanceChange {
        id,
        to_coord,
        path: None,
        mutations: mutations.to_vec(),
    }
}

#[test]
fn a_batched_change_moves_and_edits_several_atoms_in_one_undo() {
    let mut session = flat_session(4, 1);
    let table = Prefab::new(TreePath::parse("/obj/structure/table"));
    let ids = place(&mut session, &[(1, &table), (1, &table), (3, &table)]);
    assert_eq!(
        coords(&session, &ids),
        [Coord::new(1, 1, 1), Coord::new(1, 1, 1), Coord::new(3, 1, 1)]
    );
    let revision = session.revision();

    let changed = session.change_instances(
        &[
            change(ids[0], Some(Coord::new(2, 1, 1)), &[]),
            change(ids[1], Some(Coord::new(4, 1, 1)), &[]),
            change(ids[2], None, &set_name("Oak")),
        ],
        "move",
        None,
    );

    assert_eq!(changed, Some(true));
    assert_eq!(
        coords(&session, &ids),
        [Coord::new(2, 1, 1), Coord::new(4, 1, 1), Coord::new(3, 1, 1)]
    );
    assert_eq!(
        names(&session, &ids),
        [None, None, Some(Value::Text(String::from("Oak")))]
    );
    assert_ne!(session.revision(), revision);
    assert_render_cache_matches_rebuild(&session);

    assert!(session.undo());
    assert_eq!(
        coords(&session, &ids),
        [Coord::new(1, 1, 1), Coord::new(1, 1, 1), Coord::new(3, 1, 1)]
    );
    assert_eq!(names(&session, &ids), [None, None, None]);
    assert_render_cache_matches_rebuild(&session);
}

#[test]
fn a_batched_change_outside_the_map_edits_in_place() {
    let mut session = flat_session(2, 1);
    let table = Prefab::new(TreePath::parse("/obj/structure/table"));
    let ids = place(&mut session, &[(1, &table)]);

    for to_coord in [Coord::new(9, 1, 1), Coord::new(2, 1, 2), Coord::new(1, 1, 1)] {
        assert_eq!(
            session.change_instances(&[change(ids[0], Some(to_coord), &set_name("Oak"))], "edit", None),
            Some(true)
        );
        assert_eq!(coords(&session, &ids), [Coord::new(1, 1, 1)]);
        assert!(session.undo());
    }

    assert_eq!(
        session.change_instances(&[change(ids[0], Some(Coord::new(1, 1, 1)), &[])], "edit", None),
        Some(false)
    );
}

fn turf_at(session: &Session, coord: Coord) -> PrefabInstanceId {
    let document = session.state.active_document().unwrap();
    let turfs = document
        .instance_ids_at(coord)
        .iter()
        .copied()
        .filter(|id| {
            let path = &document.prefab_instance(*id).unwrap().prefab().path;

            path.to_string().starts_with("/turf")
        })
        .collect::<Vec<_>>();
    assert_eq!(turfs.len(), 1, "{coord:?} holds one turf");

    turfs[0]
}

#[test]
fn a_batched_turf_move_replaces_the_destination_turf_and_refills_the_source() {
    let mut session = flat_session(2, 1);
    let (from, to) = (Coord::new(1, 1, 1), Coord::new(2, 1, 1));
    let turf = turf_at(&session, from);
    let replaced = turf_at(&session, to);

    let changed = session.change_instances(&[change(turf, Some(to), &set_name("Moved"))], "move", None);

    assert_eq!(changed, Some(true));
    assert_eq!(turf_at(&session, to), turf);
    assert_eq!(names(&session, &[turf]), [Some(Value::Text(String::from("Moved")))]);
    let refilled = turf_at(&session, from);
    assert_ne!(refilled, turf);
    let document = session.state.active_document().unwrap();
    assert!(document.prefab_instance(replaced).is_none());
    assert_eq!(
        document.prefab_instance(refilled).unwrap().prefab().path,
        session.world().unwrap().base_tile.clone().unwrap().turf
    );
    assert_render_cache_matches_rebuild(&session);

    assert!(session.undo());
    assert_eq!(turf_at(&session, from), turf);
    assert_eq!(turf_at(&session, to), replaced);
    assert_render_cache_matches_rebuild(&session);
}

#[test]
fn rechanging_a_dragged_turf_puts_back_the_turf_it_passed_over() {
    let mut session = flat_session(3, 1);
    let tiles = [Coord::new(1, 1, 1), Coord::new(2, 1, 1), Coord::new(3, 1, 1)];
    let originals = tiles.map(|coord| turf_at(&session, coord));
    let group = EditGroupId::new();

    assert!(session.rechange_instances(&[change(originals[0], Some(tiles[1]), &[])], "drag", group));
    assert!(session.rechange_instances(&[change(originals[0], Some(tiles[2]), &[])], "drag", group));

    assert_ne!(turf_at(&session, tiles[0]), originals[0]);
    assert_eq!(turf_at(&session, tiles[1]), originals[1]);
    assert_eq!(turf_at(&session, tiles[2]), originals[0]);
    assert_render_cache_matches_rebuild(&session);

    assert!(session.undo());
    assert_eq!(tiles.map(|coord| turf_at(&session, coord)), originals);
    assert_render_cache_matches_rebuild(&session);
}

#[test]
fn rechanging_back_to_the_start_leaves_nothing_to_undo() {
    let mut session = flat_session(2, 1);
    let tiles = [Coord::new(1, 1, 1), Coord::new(2, 1, 1)];
    let originals = tiles.map(|coord| turf_at(&session, coord));
    let group = EditGroupId::new();

    assert!(session.rechange_instances(&[change(originals[0], Some(tiles[1]), &[])], "drag", group));
    assert!(session.rechange_instances(&[change(originals[0], None, &[])], "drag", group));

    assert_eq!(tiles.map(|coord| turf_at(&session, coord)), originals);
    assert!(!session.undo());
    assert_render_cache_matches_rebuild(&session);
}

#[test]
fn the_gizmo_members_cover_every_editable_pick_on_the_level() {
    let (mut session, ids) = three_tables();
    session.toggle_pick(ids[0]);
    session.toggle_pick(ids[2]);

    let members = session.gizmo_members();

    assert_eq!(
        members.iter().map(|member| member.id).collect::<Vec<_>>(),
        [ids[0], ids[2]]
    );
    assert_eq!(
        members.iter().map(|member| member.coord).collect::<Vec<_>>(),
        [Coord::new(1, 1, 1), Coord::new(3, 1, 1)]
    );
    assert!(members[0].sprite.x < members[1].sprite.x);

    session.set_tool(Tool::Place);
    assert!(session.gizmo_members().is_empty());
}

#[test]
fn the_gizmo_only_shows_for_the_pick_tool() {
    let root = examples();
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).unwrap();
    session.open_map(&root.join("test.dmm"), 1).unwrap();
    let picked = session
        .state
        .active_document()
        .and_then(|document| document.instance_ids_at(Coord::new(6, 3, 1)).first())
        .copied()
        .unwrap();
    session.pick_instance(Some(picked));
    assert_eq!(session.gizmo_transform(), session.picked_transform());

    for tool in [Tool::Place, Tool::Delete, Tool::Fill] {
        session.set_tool(tool);
        assert_eq!(session.gizmo_transform(), None, "{tool:?}");
    }

    session.set_tool(Tool::Pick);
    assert!(session.gizmo_transform().is_some());
}

#[test]
fn reanchoring_the_selected_instance_keeps_its_rendered_position() {
    let root = examples();
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).unwrap();
    session.open_map(&root.join("test.dmm"), 1).unwrap();
    let coord = Coord::new(6, 3, 1);
    let picked = session
        .state
        .active_document()
        .and_then(|document| document.instance_ids_at(coord).first())
        .copied()
        .unwrap();
    session.pick_instance(Some(picked));
    let before = session.picked_transform().unwrap();
    let destination = Coord::new(coord.x + 1, coord.y, coord.z);

    let moved = session.move_picked_instance(
        destination,
        "re-anchor object",
        &[crate::document::instance::VarMutation::Set(
            "pixel_x".into(),
            Value::Num((before.pixel[0] - 32) as f32),
        )],
        None,
    );

    assert_eq!(moved, Some(true));
    assert_eq!(session.picked_location().unwrap().coord, destination);
    let after = session.picked_transform().unwrap();
    assert_eq!(after.pixel, [before.pixel[0] - 32, before.pixel[1]]);
    assert_eq!([after.sprite.x, after.sprite.y], [before.sprite.x, before.sprite.y]);
}

#[test]
fn editing_the_selected_prefab_updates_only_its_cached_sprite() {
    let root = examples();
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).unwrap();
    session.open_map(&root.join("test.dmm"), 1).unwrap();
    let coord = Coord::new(6, 3, 1);
    let picked = session
        .state
        .active_document()
        .and_then(|document| document.instance_ids_at(coord).first())
        .copied()
        .unwrap();
    assert_eq!(session.picked_prefab(), None);
    session.pick_instance(Some(picked));
    let transform = session.picked_transform().unwrap();
    assert_eq!(transform.picked, picked);
    assert!(transform.is_movable);
    assert_eq!(transform.pixel, [0, 0]);
    assert_eq!(transform.step, [0, 0]);
    assert_eq!(transform.dir, 2);
    assert_eq!(transform.dmi_directions, Some(1));
    assert_eq!(transform.directional_types, None);
    assert_eq!(transform.sprite.owner, picked);
    let revision = session.revision();
    let before = session.instances().unwrap().sprites.clone();

    assert_eq!(
        session.set_picked_instance_var("pixel_x".into(), Value::Num(7.0)),
        Some(true),
    );
    assert_eq!(session.revision(), revision.wrapping_add(1));
    let update = session.frame_update().unwrap();
    assert_eq!(update.previous_revision, revision);
    let [sprites] = update.sprites[..] else {
        panic!("one changed span: {:?}", update.sprites);
    };
    assert!(
        session.instances().unwrap().sprites[sprites.start..sprites.end]
            .iter()
            .any(|sprite| sprite.owner == picked)
    );
    assert_eq!(session.picked_transform().unwrap().pixel, [7, 0]);
    assert_eq!(
        session
            .instances()
            .unwrap()
            .sprites
            .iter()
            .zip(before)
            .filter(|(after, before)| *after != before)
            .count(),
        1,
    );

    let rendered_revision = session.revision();
    assert_eq!(
        session.set_picked_instance_var("name".into(), Value::Text("edited".into())),
        Some(true),
    );
    assert_eq!(session.revision(), rendered_revision);
    assert_eq!(
        session.picked_prefab().and_then(|prefab| prefab.var(&"name".into())),
        Some(&Value::Text("edited".into())),
    );
}

#[test]
fn editing_a_hidden_type_keeps_it_hidden_and_showing_it_uses_the_latest_appearance() {
    let root = examples();
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).unwrap();
    session.open_map(&root.join("test.dmm"), 1).unwrap();
    let table = session
        .tree()
        .unwrap()
        .id_of(&TreePath::parse("/obj/structure/table"))
        .unwrap();
    let light = session
        .tree()
        .unwrap()
        .id_of(&TreePath::parse("/obj/machinery/light"))
        .unwrap();
    let coord = Coord::new(6, 3, 1);
    let picked = session.state.active_document().unwrap().instance_ids_at(coord)[0];
    session.pick_instance(Some(picked));
    let light_texture = session.type_thumbnail(light).unwrap().texture;
    let before_revision = session.revision();

    assert!(session.toggle_type_visibility(table));
    assert!(!session.is_type_visible(table));
    assert_eq!(session.revision(), before_revision.wrapping_add(1));
    assert!(session.instances().unwrap().sprite(picked).is_none());
    assert_eq!(
        session.set_picked_instance_var("icon_state".into(), Value::Text(String::from("light"))),
        Some(true),
    );
    assert!(session.instances().unwrap().sprite(picked).is_none());

    assert!(session.toggle_type_visibility(table));
    assert!(session.is_type_visible(table));
    assert_eq!(
        session.instances().unwrap().sprite(picked).unwrap().texture,
        light_texture
    );
    assert_render_cache_matches_rebuild(&session);
}

fn tables_north_and_south() -> (Session, PrefabInstanceId, Prefab) {
    let mut session = flat_session(1, 2);
    let table = Prefab::new(TreePath::parse("/obj/structure/table"));
    session.set_tool(Tool::Place);
    session.state.choose_prefab(table.clone());
    let north = session.place_at(Coord::new(1, 2, 1), None).unwrap();
    session.place_at(Coord::new(1, 1, 1), None).unwrap();

    (session, north, table)
}

#[test]
fn a_moved_placement_draws_in_the_order_of_its_new_tile() {
    let (mut session, north, _) = tables_north_and_south();
    session.pick_instance(Some(north));

    assert_eq!(
        session.move_picked_instance(Coord::new(1, 1, 1), "move", &[], None),
        Some(true)
    );

    assert_render_cache_matches_rebuild(&session);
}

#[test]
fn a_peers_nudge_draws_in_the_order_of_a_clean_build() {
    let (mut session, _, table) = tables_north_and_south();
    let id = session.state.active().unwrap();
    let north = Coord::new(1, 2, 1);
    let mut north_tile = session.map().unwrap().tile_at(north).unwrap().clone();
    // its instance comes back new, and must still draw under the south table
    let nudged = north_tile.iter_mut().find(|prefab| **prefab == table).unwrap();
    nudged.set_var("pixel_y".into(), Value::Num(-16.0));

    assert!(session.apply_remote_tiles(id, vec![(north, north_tile)]));

    assert_render_cache_matches_rebuild(&session);
}

fn place_row(session: &mut Session, prefab: &Prefab, row: u32, xs: &[u32]) -> Vec<PrefabInstanceId> {
    session.set_tool(Tool::Place);
    xs.iter()
        .map(|x| {
            session.state.choose_prefab(prefab.clone());
            session.place_at(Coord::new(*x, row, 1), None).unwrap()
        })
        .collect::<Vec<_>>()
}

fn pick_all(session: &mut Session, ids: &[PrefabInstanceId]) {
    session.set_tool(Tool::Pick);
    session.pick_instances(ids);
}

fn at(x: u32, y: u32) -> Coord {
    Coord::new(x, y, 1)
}

#[test]
fn rotating_a_row_of_picks_turns_it_into_a_centered_column_in_one_undo() {
    let mut session = flat_session(3, 3);
    let table = Prefab::new(TreePath::parse("/obj/structure/table"));
    let ids = place_row(&mut session, &table, 2, &[1, 2, 3]);
    pick_all(&mut session, &ids);

    assert!(session.rotate_picked_group(SelectionRotation::Clockwise, None));

    // 3x1 at min (1, 2) becomes 1x3 at min (2, 1)
    assert_eq!(coords(&session, &ids), [at(2, 3), at(2, 2), at(2, 1)]);
    assert_render_cache_matches_rebuild(&session);

    assert!(session.undo());
    assert_eq!(coords(&session, &ids), [at(1, 2), at(2, 2), at(3, 2)]);
    assert_render_cache_matches_rebuild(&session);
}

#[test]
fn four_rotations_return_every_pick_to_its_tile_and_prefab() {
    let mut session = flat_session(3, 3);
    let table = Prefab::new(TreePath::parse("/obj/structure/table"));
    let ids = place_row(&mut session, &table, 2, &[1, 2, 3]);
    pick_all(&mut session, &ids);
    let prefabs = |session: &Session| {
        let document = session.state.active_document().unwrap();

        ids.iter()
            .map(|id| document.prefab_instance(*id).unwrap().prefab().clone())
            .collect::<Vec<Prefab>>()
    };
    let original = prefabs(&session);

    for _ in 0..4 {
        assert!(session.rotate_picked_group(SelectionRotation::Clockwise, None));
    }

    assert_eq!(coords(&session, &ids), [at(1, 2), at(2, 2), at(3, 2)]);
    assert_eq!(prefabs(&session), original);
}

#[test]
fn rotating_picks_turns_each_atoms_direction() {
    let mut session = flat_session(3, 3);
    let mut table = Prefab::new(TreePath::parse("/obj/structure/table"));
    table.set_var("dir".into(), Value::Num(Dir::North.to_bits() as f32));
    let ids = place_row(&mut session, &table, 1, &[1, 2]);
    pick_all(&mut session, &ids);

    assert!(session.rotate_picked_group(SelectionRotation::Clockwise, None));

    // 2x1 at min (1, 1) becomes 1x2 at min (1, 1)
    assert_eq!(coords(&session, &ids), [at(1, 2), at(1, 1)]);
    let east = Some(Value::Num(Dir::East.to_bits() as f32));
    let document = session.state.active_document().unwrap();
    for id in &ids {
        assert_eq!(
            document.prefab_instance(*id).unwrap().prefab().var(&"dir".into()),
            east.as_ref()
        );
    }

    assert!(session.undo());
    let document = session.state.active_document().unwrap();
    let north = Some(Value::Num(Dir::North.to_bits() as f32));
    assert_eq!(
        document.prefab_instance(ids[0]).unwrap().prefab().var(&"dir".into()),
        north.as_ref()
    );
}

#[test]
fn rotating_picks_is_refused_when_a_destination_leaves_the_map() {
    let mut session = flat_session(3, 1);
    let table = Prefab::new(TreePath::parse("/obj/structure/table"));
    let ids = place_row(&mut session, &table, 1, &[1, 2, 3]);
    pick_all(&mut session, &ids);
    let revision = session.revision();

    assert!(!session.rotate_picked_group(SelectionRotation::Clockwise, None));

    assert_eq!(coords(&session, &ids), [at(1, 1), at(2, 1), at(3, 1)]);
    assert_eq!(session.revision(), revision);
}

#[test]
fn rotating_picked_turfs_keeps_one_turf_on_every_tile() {
    let mut session = flat_session(3, 3);
    let turfs = [turf_at(&session, at(1, 2)), turf_at(&session, at(2, 2))];
    pick_all(&mut session, &turfs);

    assert!(session.rotate_picked_group(SelectionRotation::Clockwise, None));

    // 2x1 at min (1, 2) becomes 1x2 at min (1, 2)
    assert_eq!(coords(&session, &turfs), [at(1, 3), at(1, 2)]);
    for x in 1..=3 {
        for y in 1..=3 {
            turf_at(&session, at(x, y));
        }
    }
    assert_render_cache_matches_rebuild(&session);

    assert!(session.undo());
    assert_eq!(coords(&session, &turfs), [at(1, 2), at(2, 2)]);
    assert_render_cache_matches_rebuild(&session);
}

#[test]
fn rotating_a_single_pick_does_nothing() {
    let mut session = flat_session(3, 3);
    let table = Prefab::new(TreePath::parse("/obj/structure/table"));
    let ids = place_row(&mut session, &table, 2, &[1, 2]);
    pick_all(&mut session, &ids[..1]);

    assert!(!session.rotate_picked_group(SelectionRotation::Clockwise, None));
    assert_eq!(coords(&session, &ids), [at(1, 2), at(2, 2)]);
}

#[test]
fn a_clockwise_then_counter_clockwise_rotation_returns_every_pick() {
    let mut session = flat_session(3, 3);
    let table = Prefab::new(TreePath::parse("/obj/structure/table"));
    let ids = place_row(&mut session, &table, 2, &[1, 2, 3]);
    pick_all(&mut session, &ids);

    assert!(session.rotate_picked_group(SelectionRotation::Clockwise, None));
    assert!(session.rotate_picked_group(SelectionRotation::CounterClockwise, None));

    assert_eq!(coords(&session, &ids), [at(1, 2), at(2, 2), at(3, 2)]);
}

#[test]
fn a_half_rotation_swaps_a_pair_and_original_is_refused() {
    let mut session = flat_session(3, 3);
    let table = Prefab::new(TreePath::parse("/obj/structure/table"));
    let ids = place_row(&mut session, &table, 1, &[1, 2]);
    pick_all(&mut session, &ids);

    assert!(!session.rotate_picked_group(SelectionRotation::Original, None));
    assert!(session.rotate_picked_group(SelectionRotation::Half, None));

    assert_eq!(coords(&session, &ids), [at(2, 1), at(1, 1)]);
}

#[test]
fn rotations_sharing_a_group_undo_together() {
    let mut session = flat_session(3, 3);
    let table = Prefab::new(TreePath::parse("/obj/structure/table"));
    let ids = place_row(&mut session, &table, 2, &[1, 2, 3]);
    pick_all(&mut session, &ids);
    let group = Some(EditGroupId::new());

    assert!(session.rotate_picked_group(SelectionRotation::Clockwise, group));
    assert!(session.rotate_picked_group(SelectionRotation::Half, group));
    assert!(session.undo());

    assert_eq!(coords(&session, &ids), [at(1, 2), at(2, 2), at(3, 2)]);
}
