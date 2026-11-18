use base::path::TreePath;

use dmm::{Map, Size};
use prefab::Prefab;

use super::*;

/// A map whose every tile names its own coordinate, so a copy can be told
/// apart from the tile it landed on.
fn map(size: Size) -> Map {
    let mut map = Map::new(size);
    for y in 1..=size.y {
        for x in 1..=size.x {
            let key = map.intern_tile(vec![
                Prefab::new(TreePath::parse(&format!("/turf/t{x}_{y}"))),
                Prefab::new(TreePath::parse("/area")),
            ]);
            for level in &mut map.grid {
                level[(size.y - y) as usize][(x - 1) as usize] = key;
            }
        }
    }

    map
}

fn selection(min: (u32, u32), max: (u32, u32)) -> Selection {
    Selection {
        min: Coord::new(min.0, min.1, 1),
        max: Coord::new(max.0, max.1, 1),
    }
}

#[test]
fn a_copied_region_keeps_its_tiles_in_row_major_order() {
    let document = MapDocument::new(map(Size { x: 4, y: 4, z: 1 }), 1);
    let region = copy_region(
        (&document.map).into(),
        selection((2, 2), (3, 4)),
        RegionSelectionMode::Full,
        HiddenTypes::default(),
    )
    .expect("region");

    assert_eq!((region.width(), region.height()), (2, 3));
    assert_eq!(region.tile(0, 0).expect("tile")[0].path.to_string(), "/turf/t2_2");
    assert_eq!(region.tile(1, 2).expect("tile")[0].path.to_string(), "/turf/t3_4");
    assert_eq!(region.filled().count(), 6);
}

#[test]
fn a_hollow_copy_leaves_its_middle_as_a_hole() {
    let document = MapDocument::new(map(Size { x: 5, y: 5, z: 1 }), 1);
    let region = copy_region(
        (&document.map).into(),
        selection((1, 1), (3, 3)),
        RegionSelectionMode::Hollow { line_width: 1 },
        HiddenTypes::default(),
    )
    .expect("region");

    assert_eq!(region.filled().count(), 8);
    assert_eq!(region.selection_mode(), RegionSelectionMode::Hollow { line_width: 1 });
    assert!(region.tile(1, 1).expect("tile").is_empty());
}

#[test]
fn a_copy_leaves_hidden_types_behind() {
    let document = MapDocument::new(map(Size { x: 2, y: 1, z: 1 }), 1);
    let hidden = [TreePath::parse("/area")].into_iter().collect();
    let region = copy_region(
        (&document.map).into(),
        selection((1, 1), (2, 1)),
        RegionSelectionMode::Full,
        hidden,
    )
    .expect("region");

    assert_eq!(
        region.tile(0, 0),
        Some(&vec![Prefab::new(TreePath::parse("/turf/t1_1"))])
    );
    assert_eq!(region.filled().count(), 2);
}

#[test]
fn a_paste_keeps_what_was_hidden_at_copy_time() {
    let source = MapDocument::new(map(Size { x: 1, y: 1, z: 1 }), 1);
    let mut destination = MapDocument::new(map(Size { x: 2, y: 1, z: 1 }), 1);
    let tree = ObjectTree::default();
    let hidden = [TreePath::parse("/area")].into_iter().collect();
    let region = copy_region(
        (&source.map).into(),
        selection((1, 1), (1, 1)),
        RegionSelectionMode::Full,
        hidden,
    )
    .expect("region");
    let area = destination.placed_tile(Coord::new(2, 1, 1)).expect("tile")[1].clone();

    let paste = paste_region(
        &mut destination,
        &tree,
        &region,
        Coord::new(2, 1, 1),
        SelectionRotation::Original,
    )
    .expect("paste");
    assert!(destination.apply(paste.action.edit));

    let pasted = destination.placed_tile(Coord::new(2, 1, 1)).expect("tile");
    assert_eq!(pasted.len(), 2);
    assert!(pasted.contains(&area), "the hidden area keeps its placement");
    assert!(
        pasted
            .iter()
            .any(|placed| placed.prefab().path.to_string() == "/turf/t1_1")
    );
}

#[test]
fn a_quarter_turn_swaps_the_footprint() {
    let document = MapDocument::new(map(Size { x: 4, y: 4, z: 1 }), 1);
    let region = copy_region(
        (&document.map).into(),
        selection((1, 1), (2, 4)),
        RegionSelectionMode::Full,
        HiddenTypes::default(),
    )
    .expect("region");
    let min = Coord::new(1, 1, 1);

    assert_eq!(
        region.footprint(min, SelectionRotation::Original),
        Some(selection((1, 1), (2, 4)))
    );
    assert_eq!(
        region.footprint(min, SelectionRotation::Clockwise),
        Some(selection((1, 1), (4, 2)))
    );
}

#[test]
fn a_region_pasted_into_another_map_reproduces_the_tiles_with_fresh_ids() {
    let source = MapDocument::new(map(Size { x: 4, y: 4, z: 1 }), 1);
    let mut destination = MapDocument::new(map(Size { x: 4, y: 4, z: 1 }), 1);
    let tree = ObjectTree::default();
    let region = copy_region(
        (&source.map).into(),
        selection((1, 1), (2, 2)),
        RegionSelectionMode::Full,
        HiddenTypes::default(),
    )
    .expect("region");

    let before = destination
        .placed_tile(Coord::new(3, 3, 1))
        .expect("tile")
        .iter()
        .map(PlacedPrefab::id)
        .collect::<Vec<_>>();
    let paste = paste_region(
        &mut destination,
        &tree,
        &region,
        Coord::new(3, 3, 1),
        SelectionRotation::Original,
    )
    .expect("paste");
    assert_eq!(paste.target, selection((3, 3), (4, 4)));
    assert!(destination.apply(paste.action.edit));

    let pasted = destination.placed_tile(Coord::new(3, 3, 1)).expect("tile");
    assert_eq!(pasted[0].prefab().path.to_string(), "/turf/t1_1");
    assert!(
        pasted.iter().all(|placed| !before.contains(&placed.id())),
        "the paste mints its own instance ids"
    );
    // The map the region came from is untouched.
    assert_eq!(
        source.placed_tile(Coord::new(1, 1, 1)).expect("tile")[0]
            .prefab()
            .path
            .to_string(),
        "/turf/t1_1"
    );
}

#[test]
fn a_paste_that_would_run_off_the_map_is_refused() {
    let source = MapDocument::new(map(Size { x: 4, y: 4, z: 1 }), 1);
    let mut destination = MapDocument::new(map(Size { x: 4, y: 4, z: 1 }), 1);
    let tree = ObjectTree::default();
    let region = copy_region(
        (&source.map).into(),
        selection((1, 1), (3, 3)),
        RegionSelectionMode::Full,
        HiddenTypes::default(),
    )
    .expect("region");

    assert!(
        paste_region(
            &mut destination,
            &tree,
            &region,
            Coord::new(3, 3, 1),
            SelectionRotation::Original
        )
        .is_none()
    );
}

#[test]
fn pasting_a_path_the_tree_does_not_know_still_writes_the_prefab() {
    let mut destination = MapDocument::new(map(Size { x: 2, y: 2, z: 1 }), 1);
    let tree = ObjectTree::default();
    let region = TileRegion {
        width: 1,
        height: 1,
        selection_mode: RegionSelectionMode::Full,
        tiles: vec![vec![Prefab::new(TreePath::parse("/turf/from/another/codebase"))]],
        hidden: HiddenTypes::default(),
        merge: PasteMerge::Replace,
    };

    let paste = paste_region(
        &mut destination,
        &tree,
        &region,
        Coord::new(1, 1, 1),
        SelectionRotation::Original,
    )
    .expect("paste");
    assert!(destination.apply(paste.action.edit));
    assert_eq!(
        destination.placed_tile(Coord::new(1, 1, 1)).expect("tile")[0]
            .prefab()
            .path
            .to_string(),
        "/turf/from/another/codebase"
    );
}

fn ids_at(document: &MapDocument, x: u32, y: u32) -> Vec<PrefabInstanceId> {
    document.instance_ids_at(Coord::new(x, y, 1)).to_vec()
}

#[test]
fn copied_instances_keep_only_the_picked_prefabs_at_their_relative_positions() {
    let document = MapDocument::new(map(Size { x: 4, y: 4, z: 1 }), 1);
    let picked = [
        ids_at(&document, 3, 3)[1],
        ids_at(&document, 2, 2)[0],
        ids_at(&document, 2, 2)[1],
    ];
    let region = copy_instances((&document.map).into(), &picked).expect("region");

    assert_eq!((region.width(), region.height()), (2, 2));
    assert_eq!(region.merge(), PasteMerge::Stack);
    assert_eq!(region.tile(0, 0).expect("tile").len(), 2);
    assert_eq!(region.tile(1, 1), Some(&vec![Prefab::new(TreePath::parse("/area"))]));
    assert!(region.tile(1, 0).expect("tile").is_empty());
    assert!(region.tile(0, 1).expect("tile").is_empty());
    assert_eq!(copy_instances((&document.map).into(), &[]), None);
}

#[test]
fn a_stacked_paste_keeps_what_the_destination_had() {
    let mut source = map(Size { x: 1, y: 1, z: 1 });
    let key = source.intern_tile(vec![
        Prefab::new(TreePath::parse("/turf/t1_1")),
        Prefab::new(TreePath::parse("/area")),
    ]);
    source.grid[0][0][0] = key;
    let source = MapDocument::new(source, 1);
    let mut destination = MapDocument::new(map(Size { x: 2, y: 1, z: 1 }), 1);
    let tree = ObjectTree::default();
    let area = ids_at(&destination, 2, 1)[1];
    let picked = [ids_at(&source, 1, 1)[0]];
    let region = copy_instances((&source.map).into(), &picked).expect("region");

    let pasted = paste_region(
        &mut destination,
        &tree,
        &region,
        Coord::new(2, 1, 1),
        SelectionRotation::Original,
    )
    .expect("paste");
    assert_eq!(pasted.placed.len(), 1);
    assert!(destination.apply(pasted.action.edit));

    let tile = destination.placed_tile(Coord::new(2, 1, 1)).expect("tile");
    let paths = tile
        .iter()
        .map(|placed| placed.prefab().path.to_string())
        .collect::<Vec<_>>();
    assert_eq!(paths, ["/turf/t2_1", "/area", "/turf/t1_1"]);
    assert_eq!(tile[2].id(), pasted.placed[0]);
    assert_eq!(tile[1].id(), area, "the destination keeps what was not pasted");
}
