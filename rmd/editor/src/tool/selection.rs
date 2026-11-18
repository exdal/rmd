use std::collections::{HashMap, HashSet};

use base::coord::Coord;
use objtree::ObjectTree;
use prefab::Prefab;

use crate::{
    clipboard::{has_kind, merge_pasted},
    document::{MapDocument, command::Edit, instance::PlacedPrefab, instance::PlacedTile, selection::Selection},
    environment::world::World,
    frame::options::HiddenTypes,
};

use super::{
    PlacementKind, RegionSelectionMode, SelectionPlacement, SelectionRotation, SelectionTransform, ToolEdit,
    fill::coord_in_bounds,
    place::{insertion_index, place_prefab_on_tile, placement_kind},
    transform::{rotated_selection_at, transform_point, transform_prefab, transformed_selection},
};

#[cfg(test)]
pub fn move_selection(
    document: &mut MapDocument, tree: &ObjectTree, world: &World, selection: Selection, target_min: Coord,
) -> Option<(ToolEdit, Selection)> {
    place_selection(
        document,
        tree,
        world,
        selection,
        target_min,
        SelectionRotation::Original,
        SelectionPlacement::Move,
    )
}

#[cfg(test)]
pub fn copy_selection(
    document: &mut MapDocument, tree: &ObjectTree, world: &World, selection: Selection, target_min: Coord,
) -> Option<(ToolEdit, Selection)> {
    place_selection(
        document,
        tree,
        world,
        selection,
        target_min,
        SelectionRotation::Original,
        SelectionPlacement::Copy,
    )
}

pub fn fill_selection(
    document: &mut MapDocument, tree: &ObjectTree, selection: Selection, prefab: &Prefab, mode: RegionSelectionMode,
) -> Option<ToolEdit> {
    if !selection.is_well_formed()
        || !coord_in_bounds(selection.min, document.map.size())
        || !coord_in_bounds(selection.max, document.map.size())
        || mode.tiles(selection).any(|coord| !document.allows_edit_at(coord))
    {
        return None;
    }

    let kind = placement_kind(tree, prefab)?;
    let mut edit = Edit::new(format!("fill region with {}", prefab.path));
    let mut affected = Vec::new();
    for coord in mode.tiles(selection) {
        let Some((after, _, tile_affected)) = place_prefab_on_tile(document, tree, coord, prefab, kind) else {
            continue;
        };

        edit.change(document, coord, after);
        affected.extend(tile_affected);
    }

    if edit.is_empty() {
        return None;
    }

    affected.sort_unstable_by_key(|id| id.get());

    Some(ToolEdit {
        edit,
        picked: None,
        affected,
    })
}

#[cfg(test)]
pub fn place_selection(
    document: &mut MapDocument, tree: &ObjectTree, world: &World, selection: Selection, target_min: Coord,
    rotation: SelectionRotation, placement: SelectionPlacement,
) -> Option<(ToolEdit, Selection)> {
    place_selection_with_mode(
        document,
        tree,
        world,
        selection,
        target_min,
        rotation,
        placement,
        RegionSelectionMode::Full,
        &HiddenTypes::default(),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn place_selection_with_mode(
    document: &mut MapDocument, tree: &ObjectTree, world: &World, selection: Selection, target_min: Coord,
    rotation: SelectionRotation, placement: SelectionPlacement, mode: RegionSelectionMode, hidden: &HiddenTypes,
) -> Option<(ToolEdit, Selection)> {
    let target = rotated_selection_at(selection, target_min, rotation)?;
    if target == selection && rotation == SelectionRotation::Original {
        return None;
    }
    let label = match placement {
        SelectionPlacement::Move => "move region",
        SelectionPlacement::Copy => "copy region",
    };

    build_selection_edit(
        document,
        tree,
        world,
        SelectionEditRequest {
            source: selection,
            target,
            transforms: rotation.transforms(),
            placement,
            mode,
            label,
        },
        hidden,
    )
}

#[cfg(test)]
pub fn transform_selection(
    document: &mut MapDocument, tree: &ObjectTree, world: &World, selection: Selection, transform: SelectionTransform,
) -> Option<(ToolEdit, Selection)> {
    transform_selection_with_mode(
        document,
        tree,
        world,
        selection,
        transform,
        RegionSelectionMode::Full,
        &HiddenTypes::default(),
    )
}

pub fn transform_selection_with_mode(
    document: &mut MapDocument, tree: &ObjectTree, world: &World, selection: Selection, transform: SelectionTransform,
    mode: RegionSelectionMode, hidden: &HiddenTypes,
) -> Option<(ToolEdit, Selection)> {
    if !selection.is_well_formed() {
        return None;
    }
    let target = transformed_selection(selection, transform)?;
    let label = match transform {
        SelectionTransform::RotateClockwise => "rotate region clockwise",
        SelectionTransform::RotateCounterClockwise => "rotate region counterclockwise",
        SelectionTransform::MirrorHorizontal => "mirror region horizontally",
        SelectionTransform::MirrorVertical => "mirror region vertically",
    };

    build_selection_edit(
        document,
        tree,
        world,
        SelectionEditRequest {
            source: selection,
            target,
            transforms: std::slice::from_ref(&transform),
            placement: SelectionPlacement::Move,
            mode,
            label,
        },
        hidden,
    )
}

struct SelectionEditRequest<'a> {
    source: Selection,
    target: Selection,
    transforms: &'a [SelectionTransform],
    placement: SelectionPlacement,
    mode: RegionSelectionMode,
    label: &'a str,
}

impl SelectionEditRequest<'_> {
    fn source_tiles(&self) -> impl Iterator<Item = Coord> {
        self.mode.tiles(self.source)
    }

    fn touched_tiles(&self) -> impl Iterator<Item = Coord> {
        self.source_tiles().chain(self.mode.tiles(self.target))
    }

    fn destination_of(&self, coord: Coord) -> Coord {
        let mut point = (coord.x - self.source.min.x, coord.y - self.source.min.y);
        let (mut width, mut height) = (self.source.width(), self.source.height());
        for transform in self.transforms {
            point = transform_point(point, width, height, *transform);
            if matches!(
                transform,
                SelectionTransform::RotateClockwise | SelectionTransform::RotateCounterClockwise
            ) {
                (width, height) = (height, width);
            }
        }

        Coord::new(
            self.target.min.x + point.0,
            self.target.min.y + point.1,
            self.target.min.z,
        )
    }
}

fn build_selection_edit(
    document: &mut MapDocument, tree: &ObjectTree, world: &World, request: SelectionEditRequest<'_>,
    hidden: &HiddenTypes,
) -> Option<(ToolEdit, Selection)> {
    let size = document.map.size();
    let touched = request.touched_tiles().collect::<HashSet<_>>();
    if touched
        .iter()
        .any(|coord| !coord_in_bounds(*coord, size) || !document.allows_edit_at(*coord))
    {
        return None;
    }

    let base_tile = if request.placement == SelectionPlacement::Move {
        Some(world.base_tile.as_ref()?)
    } else {
        None
    };
    let mut payload = Vec::with_capacity(touched.len());
    for coord in request.source_tiles() {
        let destination = request.destination_of(coord);
        let mut tile = document
            .placed_tile(coord)?
            .into_iter()
            .filter(|placed| !hidden.hides(placed.prefab()))
            .collect::<PlacedTile>();
        if request.placement == SelectionPlacement::Copy {
            tile = tile
                .into_iter()
                .map(|placed| document.instantiate(placed.prefab().clone()))
                .collect();
        }
        for transform in request.transforms {
            for placed in &mut tile {
                transform_prefab(tree, placed.prefab_mut(), *transform);
            }
        }
        payload.push((destination, tile));
    }

    let mut staged = HashMap::<Coord, PlacedTile>::new();
    if let Some(base_tile) = base_tile {
        for coord in request.source_tiles() {
            let mut left = document
                .placed_tile(coord)?
                .into_iter()
                .filter(|placed| hidden.hides(placed.prefab()))
                .collect::<PlacedTile>();
            for (kind, path) in [
                (PlacementKind::Turf, &base_tile.turf),
                (PlacementKind::Area, &base_tile.area),
            ] {
                if !has_kind(tree, &left, kind) {
                    let placed = document.instantiate(Prefab::new(path.clone()));
                    left.insert(insertion_index(tree, &left, kind), placed);
                }
            }

            staged.insert(coord, left);
        }
    }
    for (coord, incoming) in payload {
        let before = match staged.remove(&coord) {
            Some(before) => before,
            None => document.placed_tile(coord)?,
        };
        staged.insert(coord, merge_pasted(tree, &before, incoming, hidden));
    }

    let mut coords = staged.keys().copied().collect::<Vec<_>>();
    coords.sort_unstable_by_key(|coord| (coord.z, coord.y, coord.x));
    let mut edit = Edit::new(request.label);
    let mut affected = HashSet::new();
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

    let mut affected = affected.into_iter().collect::<Vec<_>>();
    affected.sort_unstable_by_key(|id| id.get());

    Some((
        ToolEdit {
            edit,
            picked: None,
            affected,
        },
        request.target,
    ))
}
