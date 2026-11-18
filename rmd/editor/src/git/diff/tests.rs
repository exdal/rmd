use base::{coord::Coord, path::TreePath, types::Value};

use dmm::{Map, Size, Tile};
use prefab::Prefab;

use super::{ChangeKind, LineKind, VarChange, Version, diff, restore_edit, tile_lines};
use crate::document::{MapDocument, grid::TileView};

fn tile(paths: &[&str]) -> Tile {
    paths.iter().map(|path| Prefab::new(TreePath::parse(path))).collect()
}

fn map(size: (u32, u32), edits: &[((u32, u32), &[&str])]) -> Map {
    let mut map = Map::new(Size {
        x: size.0,
        y: size.1,
        z: 1,
    });
    let floor = map.intern_tile(tile(&["/turf/floor"]));
    for row in &mut map.grid[0] {
        row.fill(floor);
    }

    for ((x, y), paths) in edits {
        let key = map.intern_tile(tile(paths));
        let row = (map.size.y - y) as usize;
        map.grid[0][row][*x as usize - 1] = key;
    }

    map
}

fn kinds(from: Option<Version>, to: Option<Version>) -> Vec<(u32, u32, ChangeKind)> {
    diff(from, to)
        .changes()
        .iter()
        .map(|change| (change.coord.x, change.coord.y, change.kind))
        .collect()
}

#[test]
fn identical_maps_have_no_changes() {
    let old = map((4, 4), &[((2, 2), &["/turf/floor", "/obj/table"])]);

    assert!(diff(Some(Version::File(&old)), Some(Version::File(&old.clone()))).is_empty());
}

#[test]
fn classifies_added_removed_and_swapped_objects() {
    let old = map((4, 4), &[((3, 3), &["/turf/floor", "/obj/table"])]);
    let new = map(
        (4, 4),
        &[
            ((1, 1), &["/turf/floor", "/obj/chair"]),
            ((3, 3), &["/turf/floor"]),
            ((4, 4), &["/turf/wall"]),
        ],
    );
    let result = diff(Some(Version::File(&old)), Some(Version::File(&new)));

    assert_eq!(result.at(Coord::new(1, 1, 1)), Some(ChangeKind::Added));
    assert_eq!(result.at(Coord::new(3, 3, 1)), Some(ChangeKind::Removed));
    assert_eq!(result.at(Coord::new(4, 4, 1)), Some(ChangeKind::Modified));
    assert_eq!(result.at(Coord::new(2, 2, 1)), None);
    assert_eq!(result.len(), 3);
    assert_eq!(result.count(ChangeKind::Added), 1);
}

#[test]
fn a_resized_map_adds_or_removes_its_edges() {
    let small = map((2, 1), &[]);
    let large = map((3, 1), &[]);

    assert_eq!(
        kinds(Some(Version::File(&small)), Some(Version::File(&large))),
        [(3, 1, ChangeKind::Added)]
    );
    assert_eq!(
        kinds(Some(Version::File(&large)), Some(Version::File(&small))),
        [(3, 1, ChangeKind::Removed)]
    );
    assert_eq!(kinds(None, Some(Version::File(&small))).len(), 2);
}

#[test]
fn touching_changes_share_a_hunk() {
    let old = map((5, 5), &[]);
    let new = map(
        (5, 5),
        &[
            ((1, 1), &["/turf/wall"]),
            ((2, 2), &["/turf/wall"]),
            ((5, 5), &["/turf/wall"]),
        ],
    );
    let hunks = diff(Some(Version::File(&old)), Some(Version::File(&new)))
        .changes()
        .iter()
        .map(|change| (change.coord.x, change.hunk))
        .collect::<Vec<_>>();

    assert_eq!(hunks, [(5, 1), (2, 2), (1, 2)]);
}

fn lines(from: Option<&Tile>, to: Option<&Tile>) -> Vec<(LineKind, String)> {
    tile_lines(from.map(TileView::from), to.map(TileView::from))
        .into_iter()
        .map(|line| (line.kind, line.text))
        .collect()
}

fn door(dir: f32) -> Prefab {
    let mut door = Prefab::new(TreePath::parse("/obj/door"));
    door.set_var("name".into(), Value::Text(String::from("door")));
    door.set_var("dir".into(), Value::Num(dir));

    door
}

#[test]
fn tile_lines_marks_kept_added_and_removed_objects() {
    let old = tile(&["/turf/floor", "/obj/table"]);
    let new = tile(&["/turf/floor", "/obj/chair"]);

    assert_eq!(
        lines(Some(&old), Some(&new)),
        [
            (LineKind::Kept, String::from("/turf/floor")),
            (LineKind::Removed, String::from("/obj/table")),
            (LineKind::Added, String::from("/obj/chair")),
        ]
    );
}

#[test]
fn identical_tiles_are_all_kept() {
    let old = tile(&["/turf/floor", "/obj/table"]);

    assert!(
        tile_lines(Some(TileView::from(&old)), Some(TileView::from(&old)))
            .iter()
            .all(|line| line.kind == LineKind::Kept)
    );
}

#[test]
fn same_path_with_changed_vars_is_one_modified_line() {
    let mut old = tile(&["/turf/floor"]);
    old.push(door(2.0));
    let mut new = tile(&["/turf/floor"]);
    new.push(door(4.0));

    let result = tile_lines(Some(TileView::from(&old)), Some(TileView::from(&new)));

    assert_eq!(result.len(), 2);
    assert_eq!(result[1].kind, LineKind::Modified);
    assert_eq!(result[1].text, "/obj/door");
    assert_eq!(
        result[1].vars,
        [VarChange {
            name: String::from("dir"),
            before: Some(String::from("2")),
            after: Some(String::from("4")),
        }]
    );
}

#[test]
fn a_missing_side_is_all_added_or_all_removed() {
    let old = tile(&["/turf/floor", "/obj/table"]);

    assert!(
        tile_lines(None, Some(TileView::from(&old)))
            .iter()
            .all(|line| line.kind == LineKind::Added)
    );
    assert!(
        tile_lines(Some(TileView::from(&old)), None)
            .iter()
            .all(|line| line.kind == LineKind::Removed)
    );
    assert_eq!(tile_lines(Some(TileView::from(&old)), None).len(), 2);
}

#[test]
fn restoring_puts_the_old_tile_back_until_undone() {
    let old = map((3, 3), &[((2, 2), &["/turf/floor", "/obj/table"])]);
    let mut document = MapDocument::new(map((3, 3), &[]), 1);
    let coord = Coord::new(2, 2, 1);
    let edit = restore_edit(&mut document, Some(&old), &[coord, Coord::new(9, 9, 1)], "Restore").unwrap();

    assert!(document.apply_grouped(edit, None));
    assert!(diff(Some(Version::File(&old)), Some(Version::Working(&document.map))).is_empty());
    assert!(restore_edit(&mut document, Some(&old), &[coord], "Restore").is_none());

    assert!(document.undo());
    assert_eq!(
        kinds(Some(Version::File(&old)), Some(Version::Working(&document.map))),
        [(2, 2, ChangeKind::Removed)]
    );
}
