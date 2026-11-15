use base::{location::Location, types::Value};

use dmm::{Map, Size};

use super::*;
use crate::focus::AreaFocus;

fn map() -> (Map, Prefab) {
    let mut target = Prefab::new(TreePath::parse("/obj/table"));
    target.set_var("name".into(), Value::Text(String::from("Conference")));
    let mut different_override = target.clone();
    different_override.set_var("name".into(), Value::Text(String::from("Coffee")));
    let different_path = Prefab::new(TreePath::parse("/obj/chair"));
    let subtype = Prefab::new(TreePath::parse("/obj/table/glass"));

    let mut map = Map::new(Size { x: 2, y: 1, z: 2 });
    let first = map.intern_tile(vec![target.clone(), different_override.clone()]);
    let second = map.intern_tile(vec![different_path, target.clone()]);
    let third = map.intern_tile(vec![target.clone()]);
    let fourth = map.intern_tile(vec![different_override, subtype]);
    map.grid[0][0][0] = first;
    map.grid[0][0][1] = second;
    map.grid[1][0][0] = third;
    map.grid[1][0][1] = fourth;

    (map, target)
}

fn table(subtypes: bool) -> SearchQuery {
    SearchQuery::Type {
        path: TreePath::parse("/obj/table"),
        subtypes,
    }
}

#[test]
fn a_prefab_search_matches_the_exact_prefab_across_levels_in_tile_order() {
    let (map, target) = map();
    let document = MapDocument::new(map, 1);

    assert_eq!(
        find_instances(&document, &ObjectTree::default(), &SearchQuery::Prefab(target), None),
        [
            document.instance_ids_at(Coord::new(1, 1, 1))[0],
            document.instance_ids_at(Coord::new(2, 1, 1))[1],
            document.instance_ids_at(Coord::new(1, 1, 2))[0],
        ]
    );
}

#[test]
fn a_type_search_includes_prefab_overrides_and_optionally_subtypes() {
    let (map, _) = map();
    let document = MapDocument::new(map, 1);
    let tree = ObjectTree::default();
    let exact = [
        document.instance_ids_at(Coord::new(1, 1, 1))[0],
        document.instance_ids_at(Coord::new(1, 1, 1))[1],
        document.instance_ids_at(Coord::new(2, 1, 1))[1],
        document.instance_ids_at(Coord::new(1, 1, 2))[0],
        document.instance_ids_at(Coord::new(2, 1, 2))[0],
    ];

    assert_eq!(find_instances(&document, &tree, &table(false), None), exact);
    assert_eq!(
        find_instances(&document, &tree, &table(true), None),
        [&exact[..], &[document.instance_ids_at(Coord::new(2, 1, 2))[1]]].concat()
    );
}

#[test]
fn a_search_can_be_limited_to_a_selection() {
    let (map, _) = map();
    let document = MapDocument::new(map, 1);
    let bounds = Selection::from_drag(Coord::new(2, 1, 1), Coord::new(2, 1, 1));

    assert_eq!(
        find_instances(&document, &ObjectTree::default(), &table(false), Some(bounds)),
        [document.instance_ids_at(Coord::new(2, 1, 1))[1]]
    );
}

#[test]
fn snapshots_follow_moves_and_drop_deleted_placements() {
    let (map, target) = map();
    let mut document = MapDocument::new(map, 1);
    let matches = find_instances(&document, &ObjectTree::default(), &SearchQuery::Prefab(target), None);
    let moved = matches[0];
    let deleted = matches[2];

    assert_eq!(
        document.move_instance(
            &ObjectTree::default(),
            moved,
            Coord::new(2, 1, 1),
            "move table",
            &[],
            None
        ),
        Some(true)
    );
    let action = delete_instances(&mut document, &ObjectTree::default(), &World::default(), &[deleted]).unwrap();
    assert!(document.apply(action.edit));

    let rows = resolve_instances(&document, &matches);
    assert_eq!(rows.len(), matches.len() - 1);
    assert_eq!(
        rows.iter()
            .find(|(instance, _)| *instance == moved)
            .map(|(_, location)| location.coord),
        Some(Coord::new(2, 1, 1))
    );
    assert!(rows.iter().all(|(instance, _)| *instance != deleted));
}

#[test]
fn deleting_every_match_is_one_undo() {
    let (map, target) = map();
    let mut document = MapDocument::new(map, 1);
    let grid = document.map.clone();
    let matches = find_instances(&document, &ObjectTree::default(), &SearchQuery::Prefab(target), None);

    let action = delete_instances(&mut document, &ObjectTree::default(), &World::default(), &matches).unwrap();
    assert_eq!(action.edit.label, "delete 3 instances");
    assert!(document.apply(action.edit));
    assert!(resolve_instances(&document, &matches).is_empty());
    assert_eq!(document.instance_ids_at(Coord::new(1, 1, 1)).len(), 1);

    assert!(document.undo());
    assert_eq!(document.map, grid);
    assert_eq!(resolve_instances(&document, &matches).len(), 3);
}

#[test]
fn nothing_is_deleted_outside_the_focused_area_on_its_level() {
    let (map, target) = map();
    let mut document = MapDocument::new(map, 1);
    let matches = find_instances(
        &document,
        &ObjectTree::default(),
        &SearchQuery::Prefab(target.clone()),
        None,
    );
    let seed = Coord::new(1, 1, 1);
    document.set_focus(Some(AreaFocus::new(seed, target, matches[0], [seed].into())));

    let action = delete_instances(&mut document, &ObjectTree::default(), &World::default(), &matches).unwrap();
    assert_eq!(action.affected, [matches[0], matches[2]]);
}

#[test]
fn deleting_turfs_leaves_the_default_turf() {
    let mut tree = ObjectTree::new();
    for path in ["/atom", "/turf", "/turf/floor", "/area", "/area/station"] {
        tree.register(&TreePath::parse(path), Location::default());
    }
    for (path, parent) in [("/turf", "/atom"), ("/area", "/atom")] {
        let id = tree.id_of(&TreePath::parse(path)).unwrap();
        tree.get_mut(id).unwrap().parent_type = Some(TreePath::parse(parent));
    }
    tree.resolve_parent_types();

    let floor = Prefab::new(TreePath::parse("/turf/floor"));
    let mut map = Map::new(Size { x: 2, y: 1, z: 1 });
    let tile = map.intern_tile(vec![floor.clone(), Prefab::new(TreePath::parse("/area/station"))]);
    map.grid[0][0] = vec![tile, tile];
    let mut document = MapDocument::new(map, 1);
    let before = document.map.clone();

    let matches = find_instances(&document, &tree, &SearchQuery::Prefab(floor), None);
    assert_eq!(matches.len(), 2);
    let action = delete_instances(&mut document, &tree, &World::new(&tree), &matches).unwrap();
    assert!(document.apply(action.edit));
    for x in 1..=2 {
        let paths = document
            .map
            .tile_at(Coord::new(x, 1, 1))
            .unwrap()
            .iter()
            .map(|prefab| prefab.path.to_string())
            .collect::<Vec<_>>();
        assert_eq!(paths, ["/turf", "/area/station"]);
    }

    assert!(document.undo());
    assert_eq!(document.map, before);
}
