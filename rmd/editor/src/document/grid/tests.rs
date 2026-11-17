use base::{path::TreePath, types::Value};

use dmm::{Coord, Map, Prefab, PrefabInstanceId, key::Key, parser, writer};

use super::Grid;
use crate::document::{MapDocument, command::Edit, instance::PlacedTile};

// "b" holds the same prefabs as "a"
const MAP: &str = r#""a" = (/turf/open/floor,/area/station)
"b" = (/turf/open/floor,/area/station)
"q" = (/obj/machinery/power/smes{charge = 2e+005},/turf/open/floor,/area/station)
"z" = (/turf/closed/wall,/area/station)

(1,1,1) = {"
aqz
bza
"}
"#;

fn map() -> Map {
    let (map, errors) = parser::parse(MAP);
    assert!(errors.is_empty(), "{errors:?}");

    map
}

fn key(name: &str) -> Key {
    Key::parse(name).unwrap()
}

fn replace(document: &mut MapDocument, coord: Coord, tile: Vec<Prefab>) {
    let placed = tile
        .into_iter()
        .map(|prefab| document.instantiate(prefab))
        .collect::<PlacedTile>();
    let mut edit = Edit::new("replace");
    edit.change(document, coord, placed);
    assert!(document.apply(edit));
}

#[test]
fn an_untouched_map_exports_byte_for_byte() {
    let map = map();
    let grid = Grid::from_map(&map);

    assert_eq!(writer::write(&grid.to_map(&map, map.format)), writer::write(&map));
}

#[test]
fn tiles_of_one_key_share_an_allocation() {
    let grid = Grid::from_map(&map());
    let shared = |x, y| grid.shared_tile_at(Coord::new(x, y, 1)).unwrap().0;

    assert_eq!(shared(1, 2), shared(3, 1));
    assert_ne!(shared(1, 2), shared(1, 1));
}

#[test]
fn ids_count_up_from_the_bottom_row() {
    let grid = Grid::from_map(&map());
    let ids = |x, y| {
        grid.ids_at(Coord::new(x, y, 1))
            .iter()
            .map(|id| id.get())
            .collect::<Vec<_>>()
    };

    assert_eq!(ids(1, 1), [1, 2]);
    assert_eq!(ids(2, 1), [3, 4]);
    assert_eq!(ids(3, 1), [5, 6]);
    assert_eq!(ids(1, 2), [7, 8]);
    assert_eq!(ids(2, 2), [9, 10, 11]);
    let id = PrefabInstanceId::from_raw(9).unwrap();
    assert_eq!(grid.location(id).unwrap().coord, Coord::new(2, 2, 1));
}

#[test]
fn an_edited_tile_takes_a_new_key_and_the_rest_keep_theirs() {
    let original = map();
    let mut document = MapDocument::new(original.clone(), 1);
    let edited = Coord::new(3, 2, 1);
    replace(
        &mut document,
        edited,
        vec![Prefab::new(TreePath::parse("/turf/open/lava"))],
    );

    let exported = document.to_map();
    for y in 1..=2 {
        for x in 1..=3 {
            let coord = Coord::new(x, y, 1);
            if coord != edited {
                assert_eq!(exported.key_at(coord), original.key_at(coord), "{coord:?}");
            }
        }
    }

    let new = exported.key_at(edited).unwrap();
    assert!(!original.dictionary.contains_key(&new));
    assert_eq!(
        exported.tile_at(edited).unwrap()[0].path,
        TreePath::parse("/turf/open/lava")
    );
}

#[test]
fn a_tile_edited_back_gets_its_key_and_spelling_back() {
    let mut document = MapDocument::new(map(), 1);
    let coord = Coord::new(2, 2, 1);
    let before = document.map.tile_at(coord).unwrap().to_tile();
    replace(&mut document, coord, Vec::new());

    let mut smes = Prefab::new(TreePath::parse("/obj/machinery/power/smes"));
    smes.set_var("charge".into(), Value::Num(200000.0));
    let mut restored = before.clone();
    restored[0] = smes;
    replace(&mut document, coord, restored);

    assert_eq!(writer::write(&document.to_map()), writer::write(&map()));
}

#[test]
fn a_copied_tile_reuses_the_key_it_was_copied_from() {
    let mut document = MapDocument::new(map(), 1);
    let smes = document.map.tile_at(Coord::new(2, 2, 1)).unwrap().to_tile();
    replace(&mut document, Coord::new(1, 1, 1), smes);

    let exported = document.to_map();
    assert_eq!(exported.key_at(Coord::new(1, 1, 1)), Some(key("q")));
    assert!(!exported.dictionary.contains_key(&key("b")));
}

#[test]
fn a_tile_matching_several_keys_always_takes_the_lowest() {
    // each parse seeds a new hash order
    for _ in 0..32 {
        let mut document = MapDocument::new(map(), 1);
        let floor = document.map.tile_at(Coord::new(1, 2, 1)).unwrap().to_tile();
        replace(&mut document, Coord::new(3, 2, 1), floor);

        assert_eq!(document.to_map().key_at(Coord::new(3, 2, 1)), Some(key("a")));
    }
}

#[test]
fn a_fill_shares_one_tile() {
    let mut document = MapDocument::new(map(), 1);
    let mut edit = Edit::new("fill");
    for x in 1..=3 {
        let lava = vec![document.instantiate(Prefab::new(TreePath::parse("/turf/open/lava")))];
        edit.change(&document, Coord::new(x, 1, 1), lava);
    }
    assert!(document.apply(edit));

    let shared = |x| document.map.shared_tile_at(Coord::new(x, 1, 1)).unwrap().0;
    assert_eq!(shared(1), shared(2));
    assert_eq!(shared(2), shared(3));
}

#[test]
fn painting_next_to_an_equal_tile_shares_it() {
    let mut document = MapDocument::new(map(), 1);
    for x in 1..=3 {
        replace(
            &mut document,
            Coord::new(x, 1, 1),
            vec![Prefab::new(TreePath::parse("/turf/open/lava"))],
        );
    }

    let shared = |x| document.map.shared_tile_at(Coord::new(x, 1, 1)).unwrap().0;
    assert_eq!(shared(1), shared(2));
    assert_eq!(shared(2), shared(3));
}

#[test]
fn placement_counts_match_the_cells_through_every_kind_of_edit() {
    let mut document = MapDocument::new(map(), 1);
    let floor = || Prefab::new(TreePath::parse("/turf/open/floor"));
    document.map.assert_placement_counts();

    replace(&mut document, Coord::new(1, 1, 1), vec![floor()]);
    document.map.assert_placement_counts();

    let smes = document.instance_ids_at(Coord::new(2, 2, 1))[0];
    assert_eq!(
        document.set_instance_var(smes, "name".into(), Value::Text("battery".into())),
        Some(true)
    );
    document.map.assert_placement_counts();

    assert!(document.resize(4, 3, &[floor()]));
    document.map.assert_placement_counts();

    assert!(document.resize(2, 1, &[floor()]));
    document.map.assert_placement_counts();

    assert_eq!(document.append_level(&[floor()]), Some(2));
    document.map.assert_placement_counts();

    assert!(document.delete_level(1));
    document.map.assert_placement_counts();

    while document.undo() {
        document.map.assert_placement_counts();
    }

    while document.redo() {
        document.map.assert_placement_counts();
    }

    document.apply_remote(vec![(Coord::new(1, 1, 1), vec![floor(), floor()])]);
    document.map.assert_placement_counts();
}

#[test]
fn identical_matches_prefabs_written_differently() {
    let grid = Grid::from_map(&map());
    let mut smes = Prefab::new(TreePath::parse("/obj/machinery/power/smes"));
    smes.set_var("charge".into(), Value::Num(2e5));

    assert_eq!(grid.identical(&smes), grid.ids_at(Coord::new(2, 2, 1))[..1]);
}
