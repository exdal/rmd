use base::{
    coord::Coord,
    location::Location,
    path::TreePath,
    types::{Identifier, Value},
};
use std::{path::PathBuf, sync::Arc};

use dmi::{
    IconInfo,
    metadata::{Dir, IconState, Metadata},
};
use objtree::ObjectTree;
use prefab::Prefab;

use super::{direction_mutation, directional_types_for};
use crate::{
    document::instance::VarMutation,
    environment::Environment,
    inspector::session::EditScope,
    session::Session,
    tool::{Tool, transform::directional_type_target},
};

#[test]
fn an_identical_scope_rotation_moves_every_matching_placement_to_the_new_subtype() {
    let mut tree = ObjectTree::new();
    tree.register(&TreePath::parse("/obj/alarm/directional/north"), Location::default());
    tree.register(&TreePath::parse("/obj/alarm/directional/east"), Location::default());
    let north = Prefab::new(TreePath::parse("/obj/alarm/directional/north"));
    let mut map = dmm::Map::new(dmm::Size { x: 3, y: 1, z: 1 });
    let key = map.intern_tile(vec![north.clone()]);
    let empty = map.intern_tile(vec![]);
    map.grid[0][0] = vec![key, empty, key];
    let mut session = Session::new();
    session.state.environment = Some(Arc::new(Environment::new(".", tree)));
    let id = session.state.open_document(crate::document::MapDocument::new(map, 1));
    session.rebuild_instances(id);
    let first = session
        .state
        .active_document()
        .unwrap()
        .instance_ids_at(Coord::new(1, 1, 1))[0];
    session.pick_instance(Some(first));

    assert_eq!(
        session.set_directional_type_in(EditScope::Identical, Dir::East, None),
        Some(true)
    );

    let east = Prefab::new(TreePath::parse("/obj/alarm/directional/east"));
    let document = session.state.active_document().unwrap();
    assert_eq!(document.identical_instances(&east).len(), 2);
    assert!(document.identical_instances(&north).is_empty());
}

#[test]
fn directional_type_groups_are_discovered_from_base_group_and_direction_paths() {
    let mut tree = ObjectTree::new();
    let base = tree.register(&TreePath::parse("/obj/alarm"), Location::default());
    let group = tree.register(&TreePath::parse("/obj/alarm/directional"), Location::default());
    let north = tree.register(&TreePath::parse("/obj/alarm/directional/north"), Location::default());
    tree.register(&TreePath::parse("/obj/alarm/directional/east"), Location::default());
    tree.register(
        &TreePath::parse("/obj/alarm/directional/northwest"),
        Location::default(),
    );
    let unrelated = tree.register(&TreePath::parse("/obj/alarm/party"), Location::default());

    let base_types = directional_types_for(&tree, base).unwrap();
    assert_eq!(base_types.current, None);
    assert_eq!(
        base_types.supported,
        [false, true, true, false, false, false, false, true]
    );
    assert_eq!(directional_types_for(&tree, group), Some(base_types));

    let north_types = directional_types_for(&tree, north).unwrap();
    assert_eq!(north_types.current, Some(Dir::North));
    assert_eq!(north_types.supported, base_types.supported);
    assert_eq!(
        directional_type_target(&tree, north, Dir::Northwest),
        Some(TreePath::parse("/obj/alarm/directional/northwest"))
    );
    assert_eq!(directional_types_for(&tree, unrelated), None);
}

#[test]
fn placement_rotation_replaces_directional_paths_in_one_recent_slot() {
    let mut tree = ObjectTree::new();
    tree.register(&TreePath::parse("/obj/alarm/directional/north"), Location::default());
    tree.register(&TreePath::parse("/obj/alarm/directional/east"), Location::default());
    let mut session = Session::new();
    session.state.environment = Some(Arc::new(Environment::new(".", tree)));
    let mut prefab = Prefab::new(TreePath::parse("/obj/alarm/directional/north"));
    prefab.set_var("dir".into(), Value::Num(Dir::South.to_bits() as f32));
    session.state.choose_prefab(prefab);
    session.set_tool(Tool::Place);

    let before = session.placement_direction().unwrap();
    assert_eq!(before.directional_types.unwrap().current, Some(Dir::North));
    assert_eq!(session.set_placement_direction(Dir::East), Some(true));

    let rotated = session.palette().unwrap();
    assert_eq!(rotated.path, TreePath::parse("/obj/alarm/directional/east"));
    assert_eq!(rotated.var(&"dir".into()), None);
    assert_eq!(session.recent_prefabs(), std::slice::from_ref(rotated));
    assert_eq!(
        session
            .placement_direction()
            .unwrap()
            .directional_types
            .unwrap()
            .current,
        Some(Dir::East)
    );
    assert_eq!(session.set_placement_direction(Dir::East), Some(false));
}

#[test]
fn direction_mutations_use_byond_direction_bits() {
    for direction in Dir::ORDER {
        assert_eq!(
            direction_mutation(direction),
            VarMutation::Set(Identifier::from("dir"), Value::Num(direction.to_bits() as f32))
        );
    }
}

#[test]
fn facing_several_picks_changes_the_path_of_directional_types_and_the_dir_of_the_rest() {
    let mut tree = ObjectTree::new();
    tree.register(&TreePath::parse("/obj/alarm/directional/north"), Location::default());
    tree.register(&TreePath::parse("/obj/alarm/directional/east"), Location::default());
    tree.register(&TreePath::parse("/obj/plain"), Location::default());
    let mut map = dmm::Map::new(dmm::Size { x: 2, y: 1, z: 1 });
    let alarm = map.intern_tile(vec![Prefab::new(TreePath::parse("/obj/alarm/directional/north"))]);
    let plain = map.intern_tile(vec![Prefab::new(TreePath::parse("/obj/plain"))]);
    map.grid[0][0] = vec![alarm, plain];
    let mut session = Session::new();
    session.state.environment = Some(Arc::new(Environment::new(".", tree)));
    let id = session.state.open_document(crate::document::MapDocument::new(map, 1));
    session.rebuild_instances(id);
    let ids = (1..=2)
        .map(|x| {
            session
                .state
                .active_document()
                .unwrap()
                .instance_ids_at(Coord::new(x, 1, 1))[0]
        })
        .collect::<Vec<_>>();
    session.toggle_pick(ids[0]);
    session.toggle_pick(ids[1]);

    assert_eq!(session.set_picked_direction(Dir::East, None), Some(true));

    let document = session.state.active_document().unwrap();
    let alarm = document.prefab_instance(ids[0]).unwrap().prefab();
    assert_eq!(alarm.path, TreePath::parse("/obj/alarm/directional/east"));
    assert_eq!(alarm.var(&"dir".into()), None);
    let plain = document.prefab_instance(ids[1]).unwrap().prefab();
    assert_eq!(plain.path, TreePath::parse("/obj/plain"));
    assert_eq!(plain.var(&"dir".into()), Some(&Value::Num(Dir::East.to_bits() as f32)));

    assert!(session.undo());
    let document = session.state.active_document().unwrap();
    let alarm = document.prefab_instance(ids[0]).unwrap().prefab();
    assert_eq!(alarm.path, TreePath::parse("/obj/alarm/directional/north"));
    assert_eq!(
        document.prefab_instance(ids[1]).unwrap().prefab().var(&"dir".into()),
        None
    );
}

fn four_direction_session(paths: &[&str]) -> Session {
    let mut tree = ObjectTree::new();
    for path in paths {
        tree.register(&TreePath::parse(path), Location::default());
    }
    let environment = Environment::new(".", tree);
    let mut session = Session::new();
    session
        .textures
        .insert_info(
            "four.dmi",
            &IconInfo {
                path: PathBuf::new(),
                metadata: Metadata {
                    width: 32,
                    height: 32,
                    states: vec![IconState {
                        name: String::from("four"),
                        dirs: 4,
                        frames: 1,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                sheet_width: 0,
                sheet_height: 0,
            },
        )
        .unwrap();
    session.state.environment = Some(Arc::new(environment));

    session
}

fn four_direction_prefab(path: &str) -> Prefab {
    let mut prefab = Prefab::new(TreePath::parse(path));
    prefab.set_var("icon".into(), Value::Resource(String::from("four.dmi")));
    prefab.set_var("icon_state".into(), Value::Text(String::from("four")));

    prefab
}

fn facing(prefab: &Prefab, direction: Dir) -> Prefab {
    let mut variant = prefab.clone();
    variant.set_var("dir".into(), Value::Num(direction.to_bits() as f32));

    variant
}

#[test]
fn direction_variants_follow_the_icon_state_direction_count() {
    let session = four_direction_session(&["/obj/sign"]);
    let prefab = four_direction_prefab("/obj/sign");

    assert_eq!(
        session.direction_variants(&prefab),
        Dir::ORDER[..4]
            .iter()
            .map(|direction| facing(&prefab, *direction))
            .collect::<Vec<_>>()
    );
}

#[test]
fn directional_map_helpers_have_no_direction_variants() {
    let session = four_direction_session(&["/obj/alarm/directional/north", "/obj/alarm/directional/east"]);
    let prefab = four_direction_prefab("/obj/alarm/directional/north");

    assert!(session.direction_variants(&prefab).is_empty());
}
