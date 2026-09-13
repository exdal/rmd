use std::collections::{HashMap, HashSet};

use dmm::{Coord, Tile};
use objtree::ObjectTree;

use crate::{
    command::Edit,
    document::{MapDocument, PlacedPrefab, PlacedTile, Selection},
    tool::{BlockSelectionMode, SelectionRotation, ToolEdit, coord_in_bounds, rotate_point, rotate_prefab},
};

#[derive(Debug, Clone, PartialEq)]
pub struct TileBlock {
    width: u32,
    height: u32,
    selection_mode: BlockSelectionMode,
    /// row major, `tiles[y * width + x]`, `y` ascending = dmm north
    tiles: Vec<Tile>,
}

impl TileBlock {
    pub fn width(&self) -> u32 { self.width }

    pub fn height(&self) -> u32 { self.height }

    pub fn selection_mode(&self) -> BlockSelectionMode { self.selection_mode }

    pub fn tile(&self, x: u32, y: u32) -> Option<&Tile> {
        (x < self.width && y < self.height).then(|| &self.tiles[(y * self.width + x) as usize])
    }

    pub fn filled(&self) -> impl Iterator<Item = (u32, u32, &Tile)> {
        self.tiles.iter().enumerate().filter_map(|(index, tile)| {
            let index = index as u32;

            (!tile.is_empty()).then(|| (index % self.width, index / self.width, tile))
        })
    }

    pub fn footprint(&self, min: Coord, rotation: SelectionRotation) -> Option<Selection> {
        let (width, height) = match rotation {
            SelectionRotation::Original | SelectionRotation::Half => (self.width, self.height),
            SelectionRotation::Clockwise | SelectionRotation::CounterClockwise => (self.height, self.width),
        };
        let selection = Selection {
            min,
            max: Coord::new(
                min.x.checked_add(width.checked_sub(1)?)?,
                min.y.checked_add(height.checked_sub(1)?)?,
                min.z,
            ),
        };

        selection.is_well_formed().then_some(selection)
    }

    pub fn destinations(&self, target: Selection, rotation: SelectionRotation) -> impl Iterator<Item = (Coord, &Tile)> {
        self.filled().map(move |(x, y, tile)| {
            let (offset_x, offset_y) = rotate_point((x, y), self.width, self.height, rotation);

            (
                Coord::new(target.min.x + offset_x, target.min.y + offset_y, target.min.z),
                tile,
            )
        })
    }
}

pub fn copy_block(document: &MapDocument, selection: Selection, mode: BlockSelectionMode) -> Option<TileBlock> {
    if !selection.is_well_formed() {
        return None;
    }

    let (width, height) = (selection.width(), selection.height());
    let included = mode.tiles(selection).collect::<HashSet<_>>();
    let mut tiles = Vec::with_capacity((width as usize).checked_mul(height as usize)?);
    for y in 0..height {
        for x in 0..width {
            let coord = Coord::new(selection.min.x + x, selection.min.y + y, selection.min.z);
            let tile = included
                .contains(&coord)
                .then(|| document.map.tile_at(coord).cloned())
                .flatten()
                .unwrap_or_default();

            tiles.push(tile);
        }
    }

    tiles.iter().any(|tile| !tile.is_empty()).then_some(TileBlock {
        width,
        height,
        selection_mode: mode,
        tiles,
    })
}

pub fn placements<'a>(
    document: &MapDocument, block: &'a TileBlock, min: Coord, rotation: SelectionRotation,
) -> Option<(Selection, Vec<(Coord, &'a Tile)>)> {
    let target = block.footprint(min, rotation)?;

    can_paste(document, block, min, rotation).then(|| (target, block.destinations(target, rotation).collect()))
}

pub fn can_paste(document: &MapDocument, block: &TileBlock, min: Coord, rotation: SelectionRotation) -> bool {
    let Some(target) = block.footprint(min, rotation) else {
        return false;
    };
    let size = document.map.size;

    block
        .destinations(target, rotation)
        .all(|(coord, _)| coord_in_bounds(coord, size) && document.allows_edit_at(coord))
}

pub fn paste_block(
    document: &mut MapDocument, tree: &ObjectTree, block: &TileBlock, min: Coord, rotation: SelectionRotation,
) -> Option<(ToolEdit, Selection)> {
    let (target, placements) = placements(document, block, min, rotation)?;
    let staged = placements
        .into_iter()
        .map(|(destination, tile)| {
            let placed = tile
                .iter()
                .map(|prefab| {
                    let mut prefab = prefab.clone();
                    rotate_prefab(tree, &mut prefab, rotation);

                    document.instantiate(prefab)
                })
                .collect::<Vec<_>>();

            (destination, placed)
        })
        .collect::<Vec<_>>();
    let mut staged = staged.into_iter().collect::<HashMap<Coord, PlacedTile>>();

    let mut coords = staged.keys().copied().collect::<Vec<_>>();
    coords.sort_unstable_by_key(|coord| (coord.z, coord.y, coord.x));

    let mut edit = Edit::new("paste block");
    let mut affected = Vec::new();
    for coord in coords {
        let after = staged.remove(&coord)?;
        let before = document.placed_tile(coord)?;
        if before == after {
            continue;
        }
        affected.extend(before.iter().map(PlacedPrefab::id));
        affected.extend(after.iter().map(PlacedPrefab::id));
        edit.change(document, coord, after);
    }
    if edit.is_empty() {
        return None;
    }
    affected.sort_unstable_by_key(|id| id.get());
    affected.dedup();

    Some((
        ToolEdit {
            edit,
            selected: None,
            affected,
        },
        target,
    ))
}

#[cfg(test)]
mod tests {
    use core::path::TreePath;

    use dmm::{Map, Prefab, Size};

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
    fn a_copied_block_keeps_its_tiles_in_row_major_order() {
        let document = MapDocument::new(map(Size { x: 4, y: 4, z: 1 }), 1);
        let block = copy_block(&document, selection((2, 2), (3, 4)), BlockSelectionMode::Full).expect("block");

        assert_eq!((block.width(), block.height()), (2, 3));
        assert_eq!(block.tile(0, 0).expect("tile")[0].path.to_string(), "/turf/t2_2");
        assert_eq!(block.tile(1, 2).expect("tile")[0].path.to_string(), "/turf/t3_4");
        assert_eq!(block.filled().count(), 6);
    }

    #[test]
    fn a_hollow_copy_leaves_its_middle_as_a_hole() {
        let document = MapDocument::new(map(Size { x: 5, y: 5, z: 1 }), 1);
        let block = copy_block(
            &document,
            selection((1, 1), (3, 3)),
            BlockSelectionMode::Hollow { line_width: 1 },
        )
        .expect("block");

        assert_eq!(block.filled().count(), 8);
        assert_eq!(block.selection_mode(), BlockSelectionMode::Hollow { line_width: 1 });
        assert!(block.tile(1, 1).expect("tile").is_empty());
    }

    #[test]
    fn a_quarter_turn_swaps_the_footprint() {
        let document = MapDocument::new(map(Size { x: 4, y: 4, z: 1 }), 1);
        let block = copy_block(&document, selection((1, 1), (2, 4)), BlockSelectionMode::Full).expect("block");
        let min = Coord::new(1, 1, 1);

        assert_eq!(
            block.footprint(min, SelectionRotation::Original),
            Some(selection((1, 1), (2, 4)))
        );
        assert_eq!(
            block.footprint(min, SelectionRotation::Clockwise),
            Some(selection((1, 1), (4, 2)))
        );
    }

    #[test]
    fn a_block_pasted_into_another_map_reproduces_the_tiles_with_fresh_ids() {
        let source = MapDocument::new(map(Size { x: 4, y: 4, z: 1 }), 1);
        let mut destination = MapDocument::new(map(Size { x: 4, y: 4, z: 1 }), 1);
        let tree = ObjectTree::default();
        let block = copy_block(&source, selection((1, 1), (2, 2)), BlockSelectionMode::Full).expect("block");

        let before = destination
            .placed_tile(Coord::new(3, 3, 1))
            .expect("tile")
            .iter()
            .map(PlacedPrefab::id)
            .collect::<Vec<_>>();
        let (edit, target) = paste_block(
            &mut destination,
            &tree,
            &block,
            Coord::new(3, 3, 1),
            SelectionRotation::Original,
        )
        .expect("paste");
        assert_eq!(target, selection((3, 3), (4, 4)));
        assert!(destination.apply(edit.edit));

        let pasted = destination.placed_tile(Coord::new(3, 3, 1)).expect("tile");
        assert_eq!(pasted[0].prefab().path.to_string(), "/turf/t1_1");
        assert!(
            pasted.iter().all(|placed| !before.contains(&placed.id())),
            "the paste mints its own instance ids"
        );
        // The map the block came from is untouched.
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
        let block = copy_block(&source, selection((1, 1), (3, 3)), BlockSelectionMode::Full).expect("block");

        assert!(
            paste_block(
                &mut destination,
                &tree,
                &block,
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
        let block = TileBlock {
            width: 1,
            height: 1,
            selection_mode: BlockSelectionMode::Full,
            tiles: vec![vec![Prefab::new(TreePath::parse("/turf/from/another/codebase"))]],
        };

        let (edit, _) = paste_block(
            &mut destination,
            &tree,
            &block,
            Coord::new(1, 1, 1),
            SelectionRotation::Original,
        )
        .expect("paste");
        assert!(destination.apply(edit.edit));
        assert_eq!(
            destination.placed_tile(Coord::new(1, 1, 1)).expect("tile")[0]
                .prefab()
                .path
                .to_string(),
            "/turf/from/another/codebase"
        );
    }
}
