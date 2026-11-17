use base::path::TreePath;

use dmm::{Coord, Map, Prefab, Size};

use super::Stash;
use crate::{
    clipboard::{TileRegion, copy_region},
    document::{DocumentId, MapDocument, command::Edit, selection::Selection},
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

fn name_at(stash: &Stash, id: DocumentId, x: u32, y: u32) -> Option<String> {
    stash
        .document(id)?
        .map
        .tile_at(Coord::new(x, y, 1))
        .and_then(|tile| tile.first())
        .map(|prefab| prefab.path.to_string())
}

#[test]
fn an_entry_holds_its_region_from_the_origin() {
    let mut stash = Stash::default();
    let id = stash.push(region(2, 3, "a"));
    let size = stash.document(id).unwrap().map.size();

    assert_eq!((size.x, size.y, size.z), (2, 3, 1));
    assert_eq!(name_at(&stash, id, 1, 1).as_deref(), Some("/turf/a1_1"));
    assert_eq!(name_at(&stash, id, 2, 3).as_deref(), Some("/turf/a2_3"));
}

#[test]
fn removing_an_entry_leaves_the_others_untouched() {
    let mut stash = Stash::default();
    let first = stash.push(region(2, 3, "a"));
    let second = stash.push(region(3, 1, "b"));
    assert_ne!(first, second);
    let generation = stash.document(second).unwrap().generation();

    assert!(stash.remove(first));
    assert!(!stash.remove(first));
    assert_eq!(stash.ids(), [second]);
    assert!(stash.document(first).is_none());
    assert_eq!(stash.document(second).unwrap().generation(), generation);
    assert_eq!(name_at(&stash, second, 3, 1).as_deref(), Some("/turf/b3_1"));
}

#[test]
fn the_stash_document_selects_but_never_edits() {
    let mut stash = Stash::default();
    let id = stash.push(region(2, 3, "a"));
    let coord = Coord::new(1, 1, 1);
    let document = stash.document_mut(id).unwrap();

    assert!(document.is_stash());
    assert!(!document.allows_edit_at(coord));
    assert!(document.allows_select_at(coord));
    let instance = document.instance_ids_at(coord)[0];
    assert!(!document.allows_edit_instance(instance));

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
