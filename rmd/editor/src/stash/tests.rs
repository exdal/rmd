use base::path::TreePath;

use dmm::{Coord, Map, Prefab, Size};

use super::{GAP, Stash};
use crate::{
    clipboard::{TileRegion, copy_region},
    document::{MapDocument, command::Edit, selection::Selection},
    frame::options::HiddenTypes,
    tool::RegionSelectionMode,
};

fn region(width: u32, height: u32, name: &str) -> TileRegion {
    let mut map = Map::new(Size {
        x: width,
        y: height,
        z: 1,
    });
    for y in 1..=height {
        for x in 1..=width {
            let key = map.intern_tile(vec![Prefab::new(TreePath::parse(&format!("/turf/{name}{x}_{y}")))]);
            map.grid[0][(height - y) as usize][(x - 1) as usize] = key;
        }
    }

    let document = MapDocument::new(map, 1);
    let selection = Selection {
        min: Coord::new(1, 1, 1),
        max: Coord::new(width, height, 1),
    };

    copy_region(
        (&document.map).into(),
        selection,
        RegionSelectionMode::Full,
        HiddenTypes::default(),
    )
    .expect("region")
}

fn name_at(stash: &Stash, x: u32, y: u32) -> Option<String> {
    stash
        .document()
        .map
        .tile_at(Coord::new(x, y, 1))
        .and_then(|tile| tile.first())
        .map(|prefab| prefab.path.to_string())
}

fn two_entries() -> Stash {
    let mut stash = Stash::new();
    stash.push(region(2, 3, "a"));
    stash.push(region(3, 1, "b"));

    stash
}

#[test]
fn entries_lay_out_left_to_right_bottom_aligned() {
    let stash = two_entries();
    let size = stash.document().map.size();

    assert_eq!((size.x, size.y, size.z), (2 + GAP + 3, 3, 1));
    assert_eq!(name_at(&stash, 1, 1).as_deref(), Some("/turf/a1_1"));
    assert_eq!(name_at(&stash, 2, 3).as_deref(), Some("/turf/a2_3"));
    assert_eq!(name_at(&stash, 5, 1).as_deref(), Some("/turf/b1_1"));
    assert_eq!(name_at(&stash, 7, 1).as_deref(), Some("/turf/b3_1"));

    for coord in [(3, 1), (4, 3), (5, 2), (7, 3)] {
        assert_eq!(name_at(&stash, coord.0, coord.1), None, "{coord:?}");
    }
}

#[test]
fn entry_bounds_cover_only_the_entry() {
    let stash = two_entries();

    let first = stash.entry_bounds(Coord::new(2, 2, 1)).expect("first");
    assert_eq!((first.min, first.max), (Coord::new(1, 1, 1), Coord::new(2, 3, 1)));

    let second = stash.entry_bounds(Coord::new(6, 1, 1)).expect("second");
    assert_eq!((second.min, second.max), (Coord::new(5, 1, 1), Coord::new(7, 1, 1)));

    assert!(stash.entry_bounds(Coord::new(3, 1, 1)).is_none());
    assert!(stash.entry_bounds(Coord::new(5, 2, 1)).is_none());
    assert!(stash.entry_bounds(Coord::new(9, 9, 1)).is_none());
}

#[test]
fn removing_an_entry_repacks_the_rest() {
    let mut stash = two_entries();

    assert!(!stash.remove_at(Coord::new(3, 1, 1)));
    assert_eq!(stash.len(), 2);

    assert!(stash.remove_at(Coord::new(1, 1, 1)));
    let size = stash.document().map.size();
    assert_eq!((size.x, size.y), (3, 1));
    assert_eq!(name_at(&stash, 1, 1).as_deref(), Some("/turf/b1_1"));
    assert_eq!(name_at(&stash, 3, 1).as_deref(), Some("/turf/b3_1"));
}

#[test]
fn clearing_returns_to_an_empty_single_tile() {
    let mut stash = two_entries();
    let id = stash.id();
    stash.clear();

    let size = stash.document().map.size();
    assert_eq!((size.x, size.y, size.z), (1, 1, 1));
    assert!(stash.is_empty());
    assert_eq!(name_at(&stash, 1, 1), None);
    assert_eq!(stash.id(), id);
}

#[test]
fn the_stash_document_selects_but_never_edits() {
    let mut stash = two_entries();
    let coord = Coord::new(1, 1, 1);
    let document = stash.document_mut();

    assert!(document.is_stash());
    assert!(!document.allows_edit_at(coord));
    assert!(document.allows_select_at(coord));
    let id = document.instance_ids_at(coord)[0];
    assert!(!document.allows_edit_instance(id));

    let mut edit = Edit::new("edit");
    edit.change(document, coord, Vec::new());
    assert!(!document.apply(edit));
    assert!(!document.undo());
    assert!(!document.redo());
    assert!(!document.resize(9, 9, &[]));
    assert!(document.append_level(&[]).is_none());
    assert!(!document.delete_level(1));
    assert!(!document.is_dirty());
}
