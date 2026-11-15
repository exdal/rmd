pub(crate) mod panel;
pub(crate) mod session;

use std::collections::{HashMap, HashSet};

use dmm::{Coord, Prefab, Tile};
use objtree::ObjectTree;

use crate::{
    document::{MapDocument, PlacedPrefab, PlacedTile, PrefabInstanceId, Selection, command::Edit},
    environment::world::{BaseTile, World},
    frame::HiddenTypes,
    maploader::preview::MapView,
    tool::{
        PlacementKind, RegionSelectionMode, SelectionRotation, ToolEdit,
        fill::coord_in_bounds,
        place::{insertion_index, placement_kind},
        transform::{rotate_point, rotate_prefab},
    },
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

    #[cfg(test)]
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
mod tests;
