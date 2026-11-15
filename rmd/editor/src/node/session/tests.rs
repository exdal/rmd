use base::{path::TreePath, types::Value};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use dmm::{Coord, Map, Prefab, Size};

use crate::{
    document::MapDocument,
    focus::AreaFocus,
    session::{
        DocumentCache, Session,
        fixtures::{
            layered_node_session, node_environment, node_map, node_session, node_tile_has_group, oriented_dir,
            oriented_node_map, oriented_node_session,
        },
    },
};

#[test]
fn node_sprite_eligibility_tracks_type_edits_history_and_profile_rebakes() {
    let coord = Coord::new(1, 1, 1);
    let map = node_map(1, 1, &[(coord, vec!["/obj/cable/heavy", "/obj/not_node"])]);
    let (mut session, seed) = node_session(map, coord);
    let id = session.state.active().unwrap();
    session.textures.insert_missing_icon().unwrap();
    session.rebuild_instances(id);
    let eligibility = |session: &Session| session.instances().unwrap().sprite(seed).unwrap().node_pickable;
    assert!(eligibility(&session), "registered descendants are eligible");
    let other = *session
        .state
        .active_document()
        .unwrap()
        .instance_ids_at(coord)
        .last()
        .unwrap();
    assert!(!session.instances().unwrap().sprite(other).unwrap().node_pickable);
    assert!(session.replace_context_instance(seed, TreePath::parse("/obj/not_node")));
    assert!(!eligibility(&session));
    assert!(session.undo());
    assert!(eligibility(&session));
    assert!(session.redo());
    assert!(!eligibility(&session));
    assert!(session.undo());
    assert!(session.delete_context_instance(seed));
    assert!(session.instances().unwrap().sprite(seed).is_none());
    assert!(session.undo());
    assert!(eligibility(&session));

    session.state.choose_prefab(Prefab::new(TreePath::parse("/obj/cable")));
    session.set_tool(crate::tool::Tool::Place);
    let placed = session.place_at(coord, None).unwrap();
    assert!(session.instances().unwrap().sprite(placed).unwrap().node_pickable);

    let bake = session.caches.get_mut(&id).unwrap().bake.take();
    session.rebuild_instances(id);
    assert!(
        !eligibility(&session),
        "no stale membership while the profile is absent"
    );
    session.caches.get_mut(&id).unwrap().bake = bake;
    session.rebuild_instances(id);
    assert!(eligibility(&session));
    session.pick_instance(Some(other));
    let picked = [other];
    let view = session
        .map_view_frame(
            id,
            Default::default(),
            Default::default(),
            render::MapViewInteraction {
                mode: render::InteractionMode::Node { pick: None },
                ..Default::default()
            },
            &[],
            &[],
            &picked,
        )
        .unwrap();
    assert_eq!(view.picked, &[]);
}

#[test]
fn node_select_requires_a_supported_hit_and_preserves_the_network_on_a_miss() {
    let coord = Coord::new(1, 1, 1);
    let map = node_map(1, 1, &[(coord, vec!["/obj/cable/heavy", "/obj/not_node"])]);
    let (mut session, seed) = node_session(map, coord);
    session.set_tool(crate::tool::Tool::Node);
    let other = *session
        .state
        .active_document()
        .unwrap()
        .instance_ids_at(coord)
        .last()
        .unwrap();
    assert!(!session.select_node_from_pick(render::PickResult::Miss));
    assert!(session.node_overlay().is_none());
    assert!(session.select_node_from_pick(render::PickResult::Hit(seed)));
    let overlay = session.node_overlay().unwrap();
    assert!(!session.select_node_from_pick(render::PickResult::Hit(other)));
    assert!(!session.select_node_from_pick(render::PickResult::Miss));
    assert_eq!(session.picked_instance(), Some(seed));
    assert_eq!(session.node_overlay(), Some(overlay));
}

#[test]
fn node_candidate_uses_the_visible_pick_with_tile_fallbacks() {
    let coord = Coord::new(1, 1, 1);
    let floor_path = TreePath::parse("/turf/open/floor");
    let area_path = TreePath::parse("/area/station");
    let supply_path = TreePath::parse("/obj/pipe/supply");
    let scrubbers_path = TreePath::parse("/obj/pipe/scrubbers");
    let other_path = TreePath::parse("/obj/not_node");
    let map = node_map(
        1,
        1,
        &[(coord, vec!["/obj/pipe/supply", "/obj/pipe/scrubbers", "/obj/not_node"])],
    );
    let environment = node_environment();
    let document = MapDocument::new(map, 1);
    let candidates = document
        .instance_ids_at(coord)
        .iter()
        .filter_map(|id| {
            let prefab = document.prefab_instance(*id)?.prefab();

            [&floor_path, &area_path, &supply_path, &scrubbers_path, &other_path]
                .contains(&&prefab.path)
                .then_some((prefab.path.clone(), *id))
        })
        .collect::<HashMap<_, _>>();
    let floor = candidates[&floor_path];
    let area = candidates[&area_path];
    let supply = candidates[&supply_path];
    let scrubbers = candidates[&scrubbers_path];
    let other = candidates[&other_path];
    let bake = crate::bake::build(&environment, &document).expect("node profile bakes");
    let mut session = Session::new();
    session.state.environment = Some(Arc::new(environment));
    let document_id = session.state.open_document(document);
    session.caches.insert(
        document_id,
        DocumentCache {
            bake: Some(bake),
            ..Default::default()
        },
    );
    session.rebuild_instances(document_id);

    assert_eq!(session.node_candidate_from_pick(Some(supply), coord), Some(supply));
    assert_eq!(
        session.node_candidate_from_pick(Some(scrubbers), coord),
        Some(scrubbers)
    );
    assert_eq!(session.node_candidate_from_pick(Some(floor), coord), Some(scrubbers));
    assert_eq!(session.node_candidate_from_pick(Some(area), coord), Some(scrubbers));
    assert_eq!(session.node_candidate_from_pick(Some(other), coord), None);
    assert_eq!(session.node_candidate_from_pick(None, coord), Some(scrubbers));
}

#[test]
fn node_drag_clones_the_seed_live_and_is_one_undo_step() {
    let environment = node_environment();
    let start = Coord::new(1, 2, 1);
    let middle = Coord::new(3, 2, 1);
    let detour = Coord::new(3, 3, 1);
    let end = Coord::new(5, 2, 1);
    let mut seed = Prefab::new(TreePath::parse("/obj/cable/heavy"));
    seed.set_var("color".into(), Value::Text(String::from("#65aaff")));
    seed.set_var("dir".into(), Value::Num(4.0));
    let mut map = Map::new(Size { x: 5, y: 3, z: 1 });
    for y in 1..=map.size.y {
        for x in 1..=map.size.x {
            let mut tile = vec![
                Prefab::new(TreePath::parse("/turf/open/floor")),
                Prefab::new(TreePath::parse("/area/station")),
            ];
            if Coord::new(x, y, 1) == start {
                tile.push(seed.clone());
            }
            let key = map.intern_tile(tile);
            map.grid[0][(map.size.y - y) as usize][(x - 1) as usize] = key;
        }
    }
    let document = MapDocument::new(map, 1);
    let target = document
        .instance_ids_at(start)
        .iter()
        .copied()
        .find(|instance| {
            document
                .prefab_instance(*instance)
                .is_some_and(|instance| instance.prefab().path == seed.path)
        })
        .expect("seed cable");
    let bake = crate::bake::build(&environment, &document).expect("node profile bakes");
    assert_eq!(bake.node_groups().len(), 3);

    let mut session = Session::new();
    session.state.environment = Some(Arc::new(environment));
    let document_id = session.state.open_document(document);
    session.caches.insert(
        document_id,
        DocumentCache {
            bake: Some(bake),
            ..Default::default()
        },
    );
    session.rebuild_instances(document_id);

    assert!(session.node_tool_available());
    assert!(session.begin_node_edit(target));
    assert!(session.start_node_drag(start));
    assert!(session.update_node_drag(middle));
    session.cancel_node_drag();
    for x in 2..=3 {
        assert!(
            session
                .map()
                .unwrap()
                .tile_at(Coord::new(x, 2, 1))
                .unwrap()
                .iter()
                .all(|prefab| prefab.path != seed.path),
            "cancelling restores the route at x={x}"
        );
    }
    assert!(!session.undo(), "a cancelled route leaves no history entry");

    assert!(session.start_node_drag(start));
    assert!(session.update_node_drag(detour));
    assert!(session.update_node_drag(end));

    assert!(
        session
            .map()
            .unwrap()
            .tile_at(detour)
            .unwrap()
            .iter()
            .all(|prefab| prefab.path != seed.path),
        "rerouting removes the obsolete preview branch"
    );

    for x in 1..=5 {
        let tile = session.map().unwrap().tile_at(Coord::new(x, 2, 1)).unwrap();
        let routed = tile
            .iter()
            .filter(|prefab| prefab.path == seed.path)
            .collect::<Vec<_>>();
        assert_eq!(routed, vec![&seed], "exactly one cloned seed at x={x}");
    }
    assert!(session.finish_node_drag(true));

    assert!(session.undo());
    for x in 2..=5 {
        assert!(
            session
                .map()
                .unwrap()
                .tile_at(Coord::new(x, 2, 1))
                .unwrap()
                .iter()
                .all(|prefab| prefab.path != seed.path),
            "one undo removes the routed stroke at x={x}"
        );
    }
    assert!(!session.undo());

    assert!(session.redo());
    for x in 1..=5 {
        assert!(
            session
                .map()
                .unwrap()
                .tile_at(Coord::new(x, 2, 1))
                .unwrap()
                .iter()
                .any(|prefab| prefab == &seed),
            "redo restores the exact seed at x={x}"
        );
    }
    assert!(!session.redo());
}

#[test]
fn switching_maps_cancels_a_live_node_route_before_activation() {
    let start = Coord::new(1, 2, 1);
    let middle = Coord::new(2, 2, 1);
    let end = Coord::new(4, 2, 1);
    let first_map = node_map(4, 2, &[(start, vec!["/obj/cable"])]);
    let (mut session, target) = node_session(first_map, start);
    let first = session.state.active().unwrap();

    assert!(session.begin_node_edit(target));
    assert!(session.start_node_drag(start));
    assert!(session.update_node_drag(end));
    assert!(node_tile_has_group(&session, middle));

    let second_map = node_map(4, 2, &[(middle, vec!["/obj/not_node"])]);
    let second_before = second_map.tile_at(middle).cloned().unwrap();
    let second = session.activate_document(MapDocument::new(second_map, 1));

    assert_eq!(session.state.active(), Some(second));
    assert!(session.node_edit.is_none());
    assert_eq!(
        session.state.document(second).unwrap().map.tile_at(middle),
        Some(&second_before)
    );

    let first_document = session.state.document(first).unwrap();
    for x in 2..=4 {
        assert!(
            first_document
                .map
                .tile_at(Coord::new(x, 2, 1))
                .unwrap()
                .iter()
                .all(|prefab| prefab.path != TreePath::parse("/obj/cable")),
            "switching maps restores the preview route at x={x}"
        );
    }
    assert_eq!(
        first_document.undo_label(),
        None,
        "the cancelled preview leaves no history entry"
    );
}

#[test]
fn oriented_route_preview_reroute_cancel_and_undo_preserve_directions() {
    let start = Coord::new(1, 2, 1);
    let straight = Coord::new(2, 2, 1);
    let bend = Coord::new(3, 2, 1);
    let end = Coord::new(3, 3, 1);
    let map = oriented_node_map(5, 4, &[(start, "/obj/link/segment", 1)]);
    let (mut session, seed) = oriented_node_session(map, start);
    assert!(session.begin_node_edit(seed));
    assert!(session.start_node_drag(start));
    assert!(session.update_node_drag(end));
    assert_eq!(oriented_dir(&session, start), Some(4));
    assert_eq!(oriented_dir(&session, straight), Some(4));
    assert_eq!(oriented_dir(&session, bend), Some(9));
    assert_eq!(oriented_dir(&session, end), Some(1));

    assert!(session.update_node_drag(Coord::new(4, 2, 1)));
    assert_eq!(oriented_dir(&session, end), None);
    assert_eq!(oriented_dir(&session, bend), Some(4));
    session.cancel_node_drag();
    assert_eq!(oriented_dir(&session, start), Some(1));
    assert_eq!(oriented_dir(&session, straight), None);
    assert!(!session.undo());

    assert!(session.start_node_drag(start));
    assert!(session.update_node_drag(end));
    assert!(session.finish_node_drag(true));
    assert!(session.undo());
    assert_eq!(oriented_dir(&session, start), Some(1));
    assert_eq!(oriented_dir(&session, bend), None);
    assert!(session.redo());
    assert_eq!(oriented_dir(&session, start), Some(4));
    assert_eq!(oriented_dir(&session, bend), Some(9));
}

#[test]
fn oriented_route_reorients_safe_endpoint_and_rejects_three_way_segment() {
    let start = Coord::new(1, 2, 1);
    let middle = Coord::new(2, 2, 1);
    let endpoint = Coord::new(3, 2, 1);
    let north = Coord::new(3, 3, 1);
    let placements = [
        (start, "/obj/link/segment", 4),
        (endpoint, "/obj/link/segment", 1),
        (north, "/obj/link/segment", 1),
    ];
    let (mut session, seed) = oriented_node_session(oriented_node_map(4, 4, &placements), start);
    let original_id = session
        .state
        .active_document()
        .unwrap()
        .instance_ids_at(endpoint)
        .iter()
        .copied()
        .find(|id| {
            session
                .state
                .active_document()
                .unwrap()
                .prefab_instance(*id)
                .is_some_and(|instance| instance.prefab().path == TreePath::parse("/obj/link/segment"))
        })
        .unwrap();
    assert!(session.begin_node_edit(seed));
    assert!(session.start_node_drag(start));
    assert!(session.update_node_drag(endpoint));
    assert_eq!(oriented_dir(&session, endpoint), Some(9));
    session.update_node_drag(Coord::new(4, 2, 1));
    assert!(!session.node_overlay().unwrap().route_valid);
    assert_eq!(oriented_dir(&session, endpoint), Some(1));
    assert_eq!(oriented_dir(&session, middle), None);
    assert!(session.update_node_drag(endpoint));
    assert!(session.finish_node_drag(true));
    assert!(
        session
            .state
            .active_document()
            .unwrap()
            .prefab_instance(original_id)
            .is_some()
    );
    assert!(session.undo());
    assert_eq!(oriented_dir(&session, endpoint), Some(1));
    assert_eq!(oriented_dir(&session, middle), None);

    let south = Coord::new(3, 1, 1);
    let mut branched = placements.to_vec();
    branched.push((south, "/obj/link/segment", 1));
    let (mut session, seed) = oriented_node_session(oriented_node_map(4, 4, &branched), start);
    assert!(session.begin_node_edit(seed));
    assert!(session.start_node_drag(start));
    assert!(!session.update_node_drag(endpoint));
    assert!(!session.node_overlay().unwrap().route_valid);
    assert_eq!(oriented_dir(&session, middle), None);
    assert_eq!(oriented_dir(&session, endpoint), Some(1));
    assert!(!session.finish_node_drag(true));
    assert!(!session.undo());
}

#[test]
fn node_connection_deletion_prunes_orphaned_endpoints_and_is_one_undo_step() {
    let start = Coord::new(1, 2, 1);
    let end = Coord::new(5, 2, 1);
    let map = node_map(
        5,
        3,
        &[
            (start, vec!["/obj/cable"]),
            (
                Coord::new(2, 2, 1),
                vec!["/obj/structure/table", "/obj/cable", "/obj/cable/heavy"],
            ),
            (Coord::new(3, 2, 1), vec!["/obj/cable/heavy"]),
            (Coord::new(4, 2, 1), vec!["/obj/cable"]),
            (end, vec!["/obj/cable/heavy"]),
        ],
    );
    let (mut session, target) = node_session(map, start);

    assert!(session.begin_node_edit(target));
    let overlay = session.node_overlay().unwrap();
    assert_eq!(overlay.nodes, vec![start, end]);
    assert_eq!(overlay.connections.len(), 1);
    let connection = overlay.connections[0].clone();

    assert!(session.delete_node_connection(&connection));
    for x in 1..=5 {
        assert!(
            session
                .map()
                .unwrap()
                .tile_at(Coord::new(x, 2, 1))
                .unwrap()
                .iter()
                .all(|prefab| !prefab.path.to_string().starts_with("/obj/cable")),
            "all node-group placements were removed at x={x}"
        );
    }
    assert!(
        session
            .map()
            .unwrap()
            .tile_at(Coord::new(2, 2, 1))
            .unwrap()
            .iter()
            .any(|prefab| prefab.path == TreePath::parse("/obj/structure/table")),
        "unrelated placements remain"
    );
    assert!(session.node_overlay().is_none());

    assert!(session.undo());
    assert!(!session.undo());
    assert_eq!(session.node_overlay().unwrap().connections, vec![connection.clone()]);
    assert_eq!(
        session
            .map()
            .unwrap()
            .tile_at(Coord::new(2, 2, 1))
            .unwrap()
            .iter()
            .filter(|prefab| prefab.path.to_string().starts_with("/obj/cable"))
            .count(),
        2
    );

    assert!(session.redo());
    assert!(!session.redo());
    assert!(session.node_overlay().is_none());
}

fn pipes_at(session: &Session, coord: Coord, path: &str) -> usize {
    session
        .map()
        .unwrap()
        .tile_at(coord)
        .unwrap()
        .iter()
        .filter(|prefab| prefab.path == TreePath::parse(path))
        .count()
}

#[test]
fn node_drag_lays_its_own_layer_over_a_pipe_on_another_layer() {
    let start = Coord::new(1, 2, 1);
    let crossed = Coord::new(3, 2, 1);
    let end = Coord::new(5, 2, 1);
    let map = node_map(
        5,
        3,
        &[(start, vec!["/obj/pipe/layer2"]), (crossed, vec!["/obj/pipe/layer4"])],
    );
    let (mut session, target) = layered_node_session(map, start);

    assert!(session.begin_node_edit(target));
    assert_eq!(session.node_overlay().unwrap().nodes, vec![start]);
    assert!(session.start_node_drag(start));
    assert!(session.update_node_drag(end));
    assert!(session.finish_node_drag(true));

    for x in 1..=5 {
        assert_eq!(
            pipes_at(&session, Coord::new(x, 2, 1), "/obj/pipe/layer2"),
            1,
            "one layer 2 pipe at x={x}"
        );
    }
    assert_eq!(pipes_at(&session, crossed, "/obj/pipe/layer4"), 1);
}

#[test]
fn node_connection_deletion_keeps_a_stacked_pipe_on_another_layer() {
    let start = Coord::new(1, 2, 1);
    let stacked = Coord::new(2, 2, 1);
    let end = Coord::new(3, 2, 1);
    let map = node_map(
        3,
        3,
        &[
            (start, vec!["/obj/pipe/layer2"]),
            (stacked, vec!["/obj/pipe/layer2", "/obj/pipe/layer4"]),
            (end, vec!["/obj/pipe/layer2"]),
        ],
    );
    let (mut session, target) = layered_node_session(map, start);

    assert!(session.begin_node_edit(target));
    let overlay = session.node_overlay().unwrap();
    assert_eq!(overlay.nodes, vec![start, end]);
    assert_eq!(overlay.connections.len(), 1);

    assert!(session.delete_node_connection(&overlay.connections[0]));
    for x in 1..=3 {
        assert_eq!(pipes_at(&session, Coord::new(x, 2, 1), "/obj/pipe/layer2"), 0);
    }
    assert_eq!(pipes_at(&session, stacked, "/obj/pipe/layer4"), 1);
}

#[test]
#[ignore = "requires the local target/tgstation checkout"]
fn tgstation_pipes_connect_only_on_the_same_layer_and_color() {
    use std::path::PathBuf;

    use crate::{environment::progress::Progress, session::fixtures::settle_bake};

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/tgstation");
    let options = crate::environment::compile::BakeOptions {
        forced_profile: Some(String::from("tgstation")),
        ..Default::default()
    };
    let loaded = crate::environment::worker::load_codebase(&root.join("tgstation.dme"), &options, &Progress::new())
        .expect("tgstation codebase");
    assert!(loaded.diagnostics.profile.is_none(), "{:?}", loaded.diagnostics.profile);
    let mut session = Session::new();
    session.apply_codebase(loaded);

    let scrubbers = "/obj/machinery/atmospherics/pipe/smart/manifold4w/scrubbers";
    let layer2 = format!("{scrubbers}/hidden/layer2");
    let visible_layer2 = format!("{scrubbers}/visible/layer2");
    let simple_layer2 = "/obj/machinery/atmospherics/pipe/smart/simple/scrubbers/hidden/layer2";
    let supply_layer2 = "/obj/machinery/atmospherics/pipe/smart/manifold4w/supply/hidden/layer2";
    let layer4 = format!("{scrubbers}/hidden/layer4");
    let layer3 = format!("{scrubbers}/hidden");
    let tiles = (1..=5).map(|x| Coord::new(x, 1, 1)).collect::<Vec<_>>();
    let map = node_map(
        5,
        1,
        &[
            (tiles[0], vec![layer2.as_str()]),
            (tiles[1], vec![visible_layer2.as_str(), supply_layer2]),
            (tiles[2], vec![simple_layer2, layer4.as_str()]),
            (tiles[3], vec![layer4.as_str()]),
            (tiles[4], vec![layer3.as_str()]),
        ],
    );
    session.activate_document(MapDocument::new(map, 1));
    settle_bake(&mut session);

    let instance_at = |session: &Session, coord: Coord, path: &str| {
        let document = session.state.document(session.state.active().unwrap()).unwrap();
        document
            .instance_ids_at(coord)
            .iter()
            .copied()
            .find(|id| {
                document
                    .prefab_instance(*id)
                    .is_some_and(|instance| instance.prefab().path == TreePath::parse(path))
            })
            .unwrap()
    };

    let target = instance_at(&session, tiles[0], &layer2);
    assert!(session.begin_node_edit(target));
    assert_eq!(
        session.node_overlay().unwrap().segments,
        vec![(tiles[0], tiles[1]), (tiles[1], tiles[2])],
        "hidden, visible and simple scrubbers on layer 2 join; supply and layer 4 stay out"
    );

    let target = instance_at(&session, tiles[3], &layer4);
    assert!(session.begin_node_edit(target));
    assert_eq!(
        session.node_overlay().unwrap().segments,
        vec![(tiles[2], tiles[3])],
        "layer 4 ignores layer 3 next to it"
    );
}

#[test]
fn node_connection_deletion_keeps_endpoints_with_other_connections() {
    let start = Coord::new(1, 2, 1);
    let junction = Coord::new(3, 2, 1);
    let map = node_map(
        5,
        3,
        &[
            (start, vec!["/obj/cable"]),
            (Coord::new(2, 2, 1), vec!["/obj/cable"]),
            (junction, vec!["/obj/cable"]),
            (Coord::new(4, 2, 1), vec!["/obj/cable"]),
            (Coord::new(5, 2, 1), vec!["/obj/cable"]),
            (Coord::new(3, 3, 1), vec!["/obj/cable"]),
        ],
    );
    let (mut session, target) = node_session(map, start);

    assert!(session.begin_node_edit(target));
    let connection = session
        .node_overlay()
        .unwrap()
        .connections
        .into_iter()
        .find(|connection| connection.first() == Some(&start) && connection.last() == Some(&junction))
        .unwrap();
    assert!(session.delete_node_connection(&connection));

    for coord in [start, Coord::new(2, 2, 1)] {
        assert!(
            session
                .map()
                .unwrap()
                .tile_at(coord)
                .unwrap()
                .iter()
                .all(|prefab| !prefab.path.to_string().starts_with("/obj/cable"))
        );
    }
    for coord in [junction, Coord::new(4, 2, 1), Coord::new(5, 2, 1), Coord::new(3, 3, 1)] {
        assert!(
            session
                .map()
                .unwrap()
                .tile_at(coord)
                .unwrap()
                .iter()
                .any(|prefab| prefab.path.to_string().starts_with("/obj/cable"))
        );
    }
    let overlay = session.node_overlay().unwrap();
    assert!(overlay.nodes.contains(&junction));
    assert_eq!(overlay.connections.len(), 2);

    assert!(session.undo());
    assert_eq!(session.node_overlay().unwrap().connections.len(), 3);
}

#[test]
fn adjacent_non_leaf_nodes_do_not_block_connection_deletion() {
    let left = Coord::new(2, 2, 1);
    let right = Coord::new(3, 2, 1);
    let remaining = [
        Coord::new(1, 2, 1),
        Coord::new(2, 3, 1),
        Coord::new(4, 2, 1),
        Coord::new(3, 3, 1),
    ];
    let map = node_map(
        4,
        3,
        &[
            (remaining[0], vec!["/obj/cable"]),
            (left, vec!["/obj/cable"]),
            (remaining[1], vec!["/obj/cable"]),
            (right, vec!["/obj/cable"]),
            (remaining[2], vec!["/obj/cable"]),
            (remaining[3], vec!["/obj/cable"]),
        ],
    );
    let (mut session, target) = node_session(map, left);

    assert!(session.begin_node_edit(target));
    let connection = session
        .node_overlay()
        .unwrap()
        .connections
        .into_iter()
        .find(|connection| connection.as_slice() == [left, right])
        .expect("adjacent structural nodes have a connection with no interior tile");

    assert!(session.delete_node_connection(&connection));
    assert!(
        node_tile_has_group(&session, left),
        "the active seed is retained on a tie"
    );
    assert!(
        !node_tile_has_group(&session, right),
        "one endpoint is removed to sever the inferred edge"
    );
    for coord in remaining {
        assert!(
            node_tile_has_group(&session, coord),
            "other branch placements remain at {coord:?}"
        );
    }

    assert!(session.undo());
    assert!(node_tile_has_group(&session, right));
    assert_eq!(session.node_overlay().unwrap().connections.len(), 6);
}

#[test]
fn node_connection_deletion_removes_a_straight_span_between_two_elbows() {
    let lower = Coord::new(3, 1, 1);
    let middle = Coord::new(3, 2, 1);
    let upper = Coord::new(3, 3, 1);
    let placements = [
        (Coord::new(1, 1, 1), vec!["/obj/cable"]),
        (Coord::new(2, 1, 1), vec!["/obj/cable"]),
        (lower, vec!["/obj/cable"]),
        (middle, vec!["/obj/cable/heavy"]),
        (upper, vec!["/obj/cable"]),
        (Coord::new(4, 3, 1), vec!["/obj/cable"]),
        (Coord::new(5, 3, 1), vec!["/obj/cable"]),
    ];
    let map = node_map(5, 3, &placements);
    let (mut session, target) = node_session(map, lower);

    assert!(session.begin_node_edit(target));
    let overlay = session.node_overlay().unwrap();
    let connection = overlay
        .connections
        .iter()
        .find(|connection| connection.as_slice() == [lower, middle, upper])
        .cloned()
        .expect("the straight span between the elbows is a connection");
    let middle_owner = session
        .state
        .active_document()
        .unwrap()
        .instance_ids_at(middle)
        .iter()
        .copied()
        .find(|id| {
            session
                .state
                .active_document()
                .unwrap()
                .prefab_instance(*id)
                .is_some_and(|instance| instance.prefab().path.to_string().starts_with("/obj/cable"))
        })
        .unwrap();
    assert_eq!(
        session.node_connection_from_pick(Some(middle_owner), Coord::new(1, 3, 1)),
        Some(connection.clone()),
        "a visibly shifted prefab resolves through its anchor tile"
    );
    assert_eq!(
        session.node_connection_from_pick(None, middle),
        Some(connection.clone()),
        "an empty visibility hit falls back to the pointed tile"
    );

    assert!(session.delete_node_connection(&connection));
    assert!(!node_tile_has_group(&session, middle));
    for coord in [
        Coord::new(1, 1, 1),
        Coord::new(2, 1, 1),
        lower,
        upper,
        Coord::new(4, 3, 1),
        Coord::new(5, 3, 1),
    ] {
        assert!(
            node_tile_has_group(&session, coord),
            "the remaining branches keep {coord:?}"
        );
    }
}

#[test]
fn adjacent_terminal_nodes_are_removed_with_their_connection() {
    let start = Coord::new(1, 1, 1);
    let end = Coord::new(2, 1, 1);
    let map = node_map(2, 1, &[(start, vec!["/obj/cable"]), (end, vec!["/obj/cable/heavy"])]);
    let (mut session, target) = node_session(map, start);

    assert!(session.begin_node_edit(target));
    let connection = session.node_overlay().unwrap().connections[0].clone();
    assert_eq!(connection, vec![start, end]);
    assert!(!session.delete_standalone_node(start));
    assert!(session.delete_node_connection(&connection));
    for coord in [start, end] {
        assert!(
            session
                .map()
                .unwrap()
                .tile_at(coord)
                .unwrap()
                .iter()
                .all(|prefab| !prefab.path.to_string().starts_with("/obj/cable"))
        );
    }
    assert!(session.node_overlay().is_none());

    assert!(session.undo());
    assert!(!session.undo());
    assert_eq!(session.node_overlay().unwrap().connections, vec![connection]);
}

#[test]
fn standalone_node_deletion_removes_the_group_and_is_one_undo_step() {
    let coord = Coord::new(1, 1, 1);
    let map = node_map(
        1,
        1,
        &[(coord, vec!["/obj/structure/table", "/obj/cable", "/obj/cable/heavy"])],
    );
    let (mut session, target) = node_session(map, coord);

    assert!(session.begin_node_edit(target));
    let overlay = session.node_overlay().unwrap();
    assert_eq!(overlay.nodes, vec![coord]);
    assert!(overlay.segments.is_empty());

    assert!(session.delete_standalone_node(coord));
    let tile = session.map().unwrap().tile_at(coord).unwrap();
    assert!(
        tile.iter()
            .all(|prefab| !prefab.path.to_string().starts_with("/obj/cable"))
    );
    assert!(
        tile.iter()
            .any(|prefab| prefab.path == TreePath::parse("/obj/structure/table"))
    );
    assert!(session.node_overlay().is_none());

    assert!(session.undo());
    assert!(!session.undo());
    assert_eq!(
        session
            .map()
            .unwrap()
            .tile_at(coord)
            .unwrap()
            .iter()
            .filter(|prefab| prefab.path.to_string().starts_with("/obj/cable"))
            .count(),
        2
    );
    assert_eq!(session.node_overlay().unwrap().nodes, vec![coord]);

    assert!(session.redo());
    assert!(!session.redo());
    assert!(session.node_overlay().is_none());
}

#[test]
fn node_connection_deletion_is_rejected_whole_outside_the_focus() {
    let start = Coord::new(1, 1, 1);
    let outside = Coord::new(3, 1, 1);
    let end = Coord::new(4, 1, 1);
    let map = node_map(
        4,
        1,
        &[
            (start, vec!["/obj/cable"]),
            (Coord::new(2, 1, 1), vec!["/obj/cable"]),
            (outside, vec!["/obj/cable"]),
            (end, vec!["/obj/cable"]),
        ],
    );
    let (mut session, target) = node_session(map, start);
    let area = session
        .state
        .active_document()
        .unwrap()
        .instance_ids_at(start)
        .iter()
        .copied()
        .find(|id| {
            session
                .state
                .active_document()
                .unwrap()
                .prefab_instance(*id)
                .is_some_and(|instance| instance.prefab().path == TreePath::parse("/area/station"))
        })
        .unwrap();
    session
        .state
        .active_document_mut()
        .unwrap()
        .set_focus(Some(AreaFocus::new(
            start,
            Prefab::new(TreePath::parse("/area/station")),
            area,
            HashSet::from([start, Coord::new(2, 1, 1), end]),
        )));

    assert!(session.begin_node_edit(target));
    let connection = session.node_overlay().unwrap().connections[0].clone();
    assert!(!session.delete_node_connection(&connection));
    for x in 1..=4 {
        assert!(
            session
                .map()
                .unwrap()
                .tile_at(Coord::new(x, 1, 1))
                .unwrap()
                .iter()
                .any(|prefab| prefab.path.to_string().starts_with("/obj/cable"))
        );
    }
    assert!(!session.undo());
}

#[test]
fn node_drag_merges_a_nearby_group_and_exposes_its_far_endpoint() {
    let environment = node_environment();
    let start = Coord::new(1, 2, 1);
    let endpoint = Coord::new(3, 2, 1);
    let far_endpoint = Coord::new(5, 2, 1);
    let seed = Prefab::new(TreePath::parse("/obj/cable/heavy"));
    let mut map = Map::new(Size { x: 5, y: 3, z: 1 });
    for y in 1..=map.size.y {
        for x in 1..=map.size.x {
            let coord = Coord::new(x, y, 1);
            let mut tile = vec![
                Prefab::new(TreePath::parse("/turf/open/floor")),
                Prefab::new(TreePath::parse("/area/station")),
            ];
            if coord == start || matches!(x, 4 | 5) && y == 2 {
                tile.push(seed.clone());
            }
            let key = map.intern_tile(tile);
            map.grid[0][(map.size.y - y) as usize][(x - 1) as usize] = key;
        }
    }
    let document = MapDocument::new(map, 1);
    let target = document
        .instance_ids_at(start)
        .iter()
        .copied()
        .find(|instance| {
            document
                .prefab_instance(*instance)
                .is_some_and(|instance| instance.prefab().path == seed.path)
        })
        .expect("seed cable");
    let bake = crate::bake::build(&environment, &document).expect("node profile bakes");
    let mut session = Session::new();
    session.state.environment = Some(Arc::new(environment));
    let document_id = session.state.open_document(document);
    session.caches.insert(
        document_id,
        DocumentCache {
            bake: Some(bake),
            ..Default::default()
        },
    );
    session.rebuild_instances(document_id);

    assert!(session.begin_node_edit(target));
    assert_eq!(session.node_overlay().unwrap().nodes, vec![start]);
    assert!(session.start_node_drag(start));
    assert!(session.update_node_drag(endpoint));

    let live = session.node_overlay().unwrap();
    assert_eq!(
        live.segments.len(),
        4,
        "the routed line merged with the nearby component"
    );
    assert_eq!(live.nodes, vec![start, endpoint, far_endpoint]);

    assert!(session.finish_node_drag(true));
    let committed = session.node_overlay().unwrap();
    assert_eq!(committed.segments.len(), 4);
    assert_eq!(committed.nodes, vec![start, endpoint, far_endpoint]);
}

#[test]
fn node_drag_restores_an_abandoned_blocker_detour() {
    let environment = node_environment();
    let start = Coord::new(3, 2, 1);
    let first_target = Coord::new(5, 2, 1);
    let final_target = Coord::new(5, 1, 1);
    let blocker = Coord::new(4, 2, 1);
    let seed = Prefab::new(TreePath::parse("/obj/cable/heavy"));
    let mut map = Map::new(Size { x: 5, y: 3, z: 1 });
    for y in 1..=map.size.y {
        for x in 1..=map.size.x {
            let coord = Coord::new(x, y, 1);
            let turf = if coord == blocker {
                "/turf/closed/wall"
            } else {
                "/turf/open/floor"
            };
            let mut tile = vec![
                Prefab::new(TreePath::parse(turf)),
                Prefab::new(TreePath::parse("/area/station")),
            ];
            if coord == start {
                tile.push(seed.clone());
            }
            let key = map.intern_tile(tile);
            map.grid[0][(map.size.y - y) as usize][(x - 1) as usize] = key;
        }
    }
    let document = MapDocument::new(map, 1);
    let target = document
        .instance_ids_at(start)
        .iter()
        .copied()
        .find(|instance| {
            document
                .prefab_instance(*instance)
                .is_some_and(|instance| instance.prefab().path == seed.path)
        })
        .expect("seed cable");
    let bake = crate::bake::build(&environment, &document).expect("node profile bakes");
    let mut session = Session::new();
    session.state.environment = Some(Arc::new(environment));
    let document_id = session.state.open_document(document);
    session.caches.insert(
        document_id,
        DocumentCache {
            bake: Some(bake),
            ..Default::default()
        },
    );
    session.rebuild_instances(document_id);

    assert!(session.begin_node_edit(target));
    assert!(session.start_node_drag(start));
    assert!(session.update_node_drag(first_target));
    for coord in [Coord::new(3, 3, 1), Coord::new(4, 3, 1), Coord::new(5, 3, 1)] {
        assert!(
            session
                .map()
                .unwrap()
                .tile_at(coord)
                .unwrap()
                .iter()
                .any(|prefab| prefab.path == seed.path),
            "the initial route uses the upper detour at {coord:?}"
        );
    }

    assert!(session.update_node_drag(final_target));
    for coord in [Coord::new(3, 3, 1), Coord::new(4, 3, 1), Coord::new(5, 3, 1)] {
        assert!(
            session
                .map()
                .unwrap()
                .tile_at(coord)
                .unwrap()
                .iter()
                .all(|prefab| prefab.path != seed.path),
            "the abandoned upper detour was restored at {coord:?}"
        );
    }
    for coord in [Coord::new(3, 1, 1), Coord::new(4, 1, 1), Coord::new(5, 1, 1)] {
        assert!(
            session
                .map()
                .unwrap()
                .tile_at(coord)
                .unwrap()
                .iter()
                .any(|prefab| prefab.path == seed.path),
            "the final route uses the lower detour at {coord:?}"
        );
    }
}
