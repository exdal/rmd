use std::collections::{HashMap, HashSet};

use dmm::{Coord, Prefab, Tile};
use objtree::ObjectTree;

use crate::{
    command::Edit,
    document::{MapDocument, PlacedPrefab, PlacedTile, PrefabInstanceId, Selection},
    frame::HiddenTypes,
    maploader::preview::MapView,
    tool::{
        PlacementKind, RegionSelectionMode, SelectionRotation, ToolEdit, coord_in_bounds, insertion_index,
        placement_kind, rotate_point, rotate_prefab,
    },
    world::{BaseTile, World},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteMerge {
    Replace,
    Stack,
}

pub struct PastedRegion {
    pub action: ToolEdit,
    pub target: Selection,
    pub placed: Vec<PrefabInstanceId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TileRegion {
    width: u32,
    height: u32,
    selection_mode: RegionSelectionMode,
    /// row major, `tiles[y * width + x]`, `y` ascending = dmm north
    tiles: Vec<Tile>,
    /// A paste leaves these on the destination
    hidden: HiddenTypes,
    merge: PasteMerge,
}

impl TileRegion {
    pub fn merge(&self) -> PasteMerge {
        self.merge
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn selection_mode(&self) -> RegionSelectionMode {
        self.selection_mode
    }

    pub fn tile(&self, x: u32, y: u32) -> Option<&Tile> {
        (x < self.width && y < self.height).then(|| &self.tiles[(y * self.width + x) as usize])
    }

    pub fn filled(&self) -> impl Iterator<Item = (u32, u32, &Tile)> {
        let bounds = Selection {
            min: Coord::new(1, 1, 1),
            max: Coord::new(self.width, self.height, 1),
        };

        self.tiles.iter().enumerate().filter_map(move |(index, tile)| {
            let (x, y) = (index as u32 % self.width, index as u32 / self.width);

            self.selection_mode
                .includes(bounds, Coord::new(x + 1, y + 1, 1))
                .then_some((x, y, tile))
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

pub fn copy_region(
    map: MapView<'_>, selection: Selection, mode: RegionSelectionMode, hidden: HiddenTypes,
) -> Option<TileRegion> {
    if !selection.is_well_formed() {
        return None;
    }

    let (width, height) = (selection.width(), selection.height());
    let included = mode.tiles(selection).collect::<HashSet<_>>();
    let mut tiles: Vec<Tile> = Vec::with_capacity((width as usize).checked_mul(height as usize)?);
    for y in 0..height {
        for x in 0..width {
            let coord = Coord::new(selection.min.x + x, selection.min.y + y, selection.min.z);
            let tile = included
                .contains(&coord)
                .then(|| map.tile_at(coord))
                .flatten()
                .map(|tile| tile.iter().filter(|prefab| !hidden.hides(prefab)).cloned().collect())
                .unwrap_or_default();

            tiles.push(tile);
        }
    }

    tiles.iter().any(|tile| !tile.is_empty()).then_some(TileRegion {
        width,
        height,
        selection_mode: mode,
        tiles,
        hidden,
        merge: PasteMerge::Replace,
    })
}

pub fn copy_instances(map: MapView<'_>, ids: &[PrefabInstanceId]) -> Option<TileRegion> {
    let located = ids
        .iter()
        .filter_map(|id| Some((*id, map.location(*id)?.coord)))
        .collect::<Vec<_>>();
    let bounds = Selection::enclosing(located.iter().map(|(_, coord)| *coord))?;
    let (width, height) = (bounds.width(), bounds.height());
    let mut tiles = vec![Tile::new(); (width as usize).checked_mul(height as usize)?];
    let picked = located.iter().map(|(id, _)| *id).collect::<HashSet<_>>();
    for (_, coord) in &located {
        let index = ((coord.y - bounds.min.y) * width + (coord.x - bounds.min.x)) as usize;
        if !tiles[index].is_empty() {
            continue;
        }

        tiles[index] = map
            .ids_at(*coord)
            .iter()
            .zip(map.tile_at(*coord)?)
            .filter(|(id, _)| picked.contains(*id))
            .map(|(_, prefab)| prefab.clone())
            .collect();
    }

    Some(TileRegion {
        width,
        height,
        selection_mode: RegionSelectionMode::Full,
        tiles,
        hidden: HiddenTypes::default(),
        merge: PasteMerge::Stack,
    })
}

pub fn placements<'a>(
    document: &MapDocument, region: &'a TileRegion, min: Coord, rotation: SelectionRotation,
) -> Option<(Selection, Vec<(Coord, &'a Tile)>)> {
    let target = region.footprint(min, rotation)?;

    can_paste(document, region, min, rotation).then(|| (target, region.destinations(target, rotation).collect()))
}

pub fn can_paste(document: &MapDocument, region: &TileRegion, min: Coord, rotation: SelectionRotation) -> bool {
    let Some(target) = region.footprint(min, rotation) else {
        return false;
    };
    let size = document.map.size();

    region
        .destinations(target, rotation)
        .all(|(coord, _)| coord_in_bounds(coord, size) && document.allows_edit_at(coord))
}

pub fn paste_region(
    document: &mut MapDocument, tree: &ObjectTree, region: &TileRegion, min: Coord, rotation: SelectionRotation,
) -> Option<PastedRegion> {
    let (target, placements) = placements(document, region, min, rotation)?;
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

    let mut edit = Edit::new("paste region");
    let mut affected = Vec::new();
    let mut placed = Vec::new();
    for coord in coords {
        let before = document.placed_tile(coord)?;
        let pasted = staged.remove(&coord)?;
        let pasted_ids = pasted.iter().map(PlacedPrefab::id).collect::<Vec<_>>();
        let after = match region.merge {
            PasteMerge::Replace => merge_pasted(tree, &before, pasted, &region.hidden),
            PasteMerge::Stack => stack_pasted(tree, &before, pasted),
        };
        if before == after {
            continue;
        }

        placed.extend(after.iter().map(PlacedPrefab::id).filter(|id| pasted_ids.contains(id)));
        affected.extend(before.iter().map(PlacedPrefab::id));
        affected.extend(after.iter().map(PlacedPrefab::id));
        edit.change(document, coord, after);
    }
    if edit.is_empty() {
        return None;
    }
    affected.sort_unstable_by_key(|id| id.get());
    affected.dedup();

    Some(PastedRegion {
        action: ToolEdit {
            edit,
            picked: None,
            affected,
        },
        target,
        placed,
    })
}

fn stack_pasted(tree: &ObjectTree, before: &PlacedTile, pasted: PlacedTile) -> PlacedTile {
    let mut after = before.clone();
    for placed in pasted {
        let kind = kind_of(tree, placed.prefab());
        if kind != PlacementKind::Atom {
            after.retain(|existing| kind_of(tree, existing.prefab()) != kind);
        }

        after.insert(insertion_index(tree, &after, kind), placed);
    }

    after
}

pub(crate) fn merge_pasted(
    tree: &ObjectTree, before: &PlacedTile, pasted: PlacedTile, hidden: &HiddenTypes,
) -> PlacedTile {
    if hidden.is_empty() {
        return pasted;
    }

    let (mut after, replaced): (PlacedTile, PlacedTile) =
        before.iter().cloned().partition(|placed| hidden.hides(placed.prefab()));
    for placed in pasted {
        let kind = kind_of(tree, placed.prefab());
        // hidden turfs and areas are left alone, and a tile only has room for one
        if kind != PlacementKind::Atom && has_kind(tree, &after, kind) {
            continue;
        }

        after.insert(insertion_index(tree, &after, kind), placed);
    }

    for kind in [PlacementKind::Turf, PlacementKind::Area] {
        if !has_kind(tree, &after, kind)
            && let Some(placed) = replaced.iter().find(|placed| kind_of(tree, placed.prefab()) == kind)
        {
            after.insert(insertion_index(tree, &after, kind), placed.clone());
        }
    }

    after
}

/// Removes what is not hidden, leaving the default turf and area behind
pub fn clear_region(
    document: &mut MapDocument, tree: &ObjectTree, world: &World, selection: Selection, mode: RegionSelectionMode,
    hidden: &HiddenTypes, label: &str,
) -> Option<ToolEdit> {
    if !selection.is_well_formed()
        || !coord_in_bounds(selection.min, document.map.size())
        || !coord_in_bounds(selection.max, document.map.size())
        || mode.tiles(selection).any(|coord| !document.allows_edit_at(coord))
    {
        return None;
    }

    let base_tile = world.base_tile.as_ref()?;
    let mut edit = Edit::new(label);
    let mut affected = Vec::new();
    for coord in mode.tiles(selection) {
        if !document
            .map
            .tile_at(coord)
            .is_some_and(|tile| clears_anything(tile, hidden, base_tile))
        {
            continue;
        }

        let before = document.placed_tile(coord)?;
        let mut after = before
            .iter()
            .filter(|placed| hidden.hides(placed.prefab()) || is_base(placed.prefab(), base_tile))
            .cloned()
            .collect::<PlacedTile>();

        for (kind, path) in [
            (PlacementKind::Turf, &base_tile.turf),
            (PlacementKind::Area, &base_tile.area),
        ] {
            if !has_kind(tree, &after, kind) {
                let placed = document.instantiate(Prefab::new(path.clone()));
                after.insert(insertion_index(tree, &after, kind), placed);
            }
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

    Some(ToolEdit {
        edit,
        picked: None,
        affected,
    })
}

pub fn can_clear(world: &World, tile: &Tile, hidden: &HiddenTypes) -> bool {
    world
        .base_tile
        .as_ref()
        .is_some_and(|base_tile| clears_anything(tile, hidden, base_tile))
}

fn clears_anything(tile: &[Prefab], hidden: &HiddenTypes, base_tile: &BaseTile) -> bool {
    tile.iter()
        .any(|prefab| !hidden.hides(prefab) && !is_base(prefab, base_tile))
}

fn is_base(prefab: &Prefab, base_tile: &BaseTile) -> bool {
    prefab.vars.is_empty() && (prefab.path == base_tile.turf || prefab.path == base_tile.area)
}

fn kind_of(tree: &ObjectTree, prefab: &Prefab) -> PlacementKind {
    placement_kind(tree, prefab).unwrap_or(PlacementKind::Atom)
}

pub(crate) fn has_kind(tree: &ObjectTree, tile: &[PlacedPrefab], kind: PlacementKind) -> bool {
    tile.iter().any(|placed| kind_of(tree, placed.prefab()) == kind)
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
}
