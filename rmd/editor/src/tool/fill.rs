use base::path::TreePath;
use std::collections::{HashSet, VecDeque};

use dmm::{Coord, Prefab};
use objtree::{ObjectTree, TypeId};

use crate::document::{MapDocument, command::Edit, grid::Grid};

use super::{
    FillError, FillMode, PlacementKind, SelectionMask, ToolContext, ToolEdit,
    place::{placement_kind, replace_kind, would_replace_kind},
};

pub(super) fn fill(
    context: &mut ToolContext<'_>, max_tiles: Option<usize>, mask: Option<SelectionMask>,
) -> Result<Option<ToolEdit>, FillError> {
    if !coord_in_bounds(context.coord, context.document.map.size())
        || !context.document.allows_edit_at(context.coord)
        || mask.is_some_and(|mask| !mask.includes(context.coord))
    {
        return Ok(None);
    }

    let Some(prefab) = context.prefab.cloned() else {
        return Ok(None);
    };
    let Some(prefab_id) = context.tree.id_of(&prefab.path) else {
        return Ok(None);
    };
    let Some(kind) = placement_kind(context.tree, &prefab) else {
        return Ok(None);
    };
    if kind == PlacementKind::Atom {
        return Ok(None);
    }

    let floor = context.tree.id_of(&TreePath::parse("/turf/open/floor"));
    let wall = context.tree.id_of(&TreePath::parse("/turf/closed"));
    let coords = match context.fill_mode {
        FillMode::Wall => match wall {
            Some(wall) => boundary_region(context, &[wall], mask),
            None => return Ok(None),
        },
        FillMode::EntireArea => area_region(context, mask),
        FillMode::Custom => custom_region(context, mask),
    };

    if coords.is_empty() {
        return Ok(None);
    }

    let preserve_walls = kind == PlacementKind::Turf
        && context.fill_mode == FillMode::EntireArea
        && floor.is_some_and(|floor| context.tree.is_subtype_of(prefab_id, floor));
    let mut coords = coords
        .into_iter()
        .filter(|coord| {
            !preserve_walls || !wall.is_some_and(|wall| tile_has_subtype(context.tree, context.document, *coord, wall))
        })
        .filter(|coord| would_replace_kind(context.document, context.tree, *coord, &prefab, kind))
        .collect::<Vec<_>>();

    if let Some(limit) = max_tiles
        && coords.len() > limit
    {
        return Err(FillError::TooLarge { limit });
    }

    if coords.is_empty() {
        return Ok(None);
    }

    let mut edit = Edit::new(format!("fill {}", prefab.path));
    let mut affected = Vec::new();

    for coord in coords.drain(..) {
        let Some((after, tile_affected)) = replace_kind(context.document, context.tree, coord, &prefab, kind) else {
            continue;
        };

        edit.change(context.document, coord, after);
        affected.extend(tile_affected);
    }

    if edit.is_empty() {
        return Ok(None);
    }

    Ok(Some(ToolEdit {
        edit,
        picked: None,
        affected,
    }))
}

pub(crate) fn coord_in_bounds(coord: Coord, size: dmm::Size) -> bool {
    (1..=size.x).contains(&coord.x) && (1..=size.y).contains(&coord.y) && (1..=size.z).contains(&coord.z)
}

fn custom_region(context: &ToolContext<'_>, mask: Option<SelectionMask>) -> Vec<Coord> {
    let boundaries = context
        .custom_fill_boundaries
        .iter()
        .filter_map(|path| context.tree.id_of(path))
        .collect::<Vec<_>>();

    if boundaries.is_empty() {
        return Vec::new();
    }

    boundary_region(context, &boundaries, mask)
}

fn boundary_region(context: &ToolContext<'_>, boundaries: &[TypeId], mask: Option<SelectionMask>) -> Vec<Coord> {
    let is_boundary = |coord| tile_has_any_subtype(context.tree, context.document, coord, boundaries);

    if is_boundary(context.coord) {
        return Vec::new();
    }

    let mut tiles = Vec::new();
    let mut pending = VecDeque::from([context.coord]);
    let mut visited = HashSet::from([context.coord]);

    while let Some(coord) = pending.pop_front() {
        if !context.document.allows_edit_at(coord)
            || mask.is_some_and(|mask| !mask.includes(coord))
            || is_boundary(coord)
        {
            continue;
        }

        tiles.push(coord);
        for neighbor in cardinal_neighbors(coord, context.document.map.size()) {
            if visited.insert(neighbor) {
                pending.push_back(neighbor);
            }
        }
    }

    tiles
}

fn area_region(context: &ToolContext<'_>, mask: Option<SelectionMask>) -> Vec<Coord> {
    let area = match context.tree.roots().area {
        Some(area) => area,
        None => return Vec::new(),
    };
    let Some(seed) = prefab_of_subtype(context.tree, &context.document.map, context.coord, area).cloned() else {
        return Vec::new();
    };

    let mut tiles = Vec::new();
    let mut pending = VecDeque::from([context.coord]);
    let mut visited = HashSet::from([context.coord]);

    while let Some(coord) = pending.pop_front() {
        if !context.document.allows_edit_at(coord)
            || mask.is_some_and(|mask| !mask.includes(coord))
            || !prefab_of_subtype(context.tree, &context.document.map, coord, area)
                .is_some_and(|candidate| crate::frame::same_area(&seed, candidate))
        {
            continue;
        }

        tiles.push(coord);
        for neighbor in cardinal_neighbors(coord, context.document.map.size()) {
            if visited.insert(neighbor) {
                pending.push_back(neighbor);
            }
        }
    }

    tiles
}

fn cardinal_neighbors(coord: Coord, size: dmm::Size) -> impl Iterator<Item = Coord> {
    [
        (coord.x > 1).then(|| Coord::new(coord.x - 1, coord.y, coord.z)),
        (coord.x < size.x).then(|| Coord::new(coord.x + 1, coord.y, coord.z)),
        (coord.y > 1).then(|| Coord::new(coord.x, coord.y - 1, coord.z)),
        (coord.y < size.y).then(|| Coord::new(coord.x, coord.y + 1, coord.z)),
    ]
    .into_iter()
    .flatten()
}

fn tile_has_subtype(tree: &ObjectTree, document: &MapDocument, coord: Coord, ancestor: TypeId) -> bool {
    prefab_of_subtype(tree, &document.map, coord, ancestor).is_some()
}

fn tile_has_any_subtype(tree: &ObjectTree, document: &MapDocument, coord: Coord, ancestors: &[TypeId]) -> bool {
    document.map.tile_at(coord).is_some_and(|tile| {
        tile.iter().any(|prefab| {
            tree.id_of(&prefab.path)
                .is_some_and(|id| ancestors.iter().any(|ancestor| tree.is_subtype_of(id, *ancestor)))
        })
    })
}

fn prefab_of_subtype<'a>(tree: &ObjectTree, map: &'a Grid, coord: Coord, ancestor: TypeId) -> Option<&'a Prefab> {
    map.tile_at(coord)?.iter().find(|prefab| {
        tree.id_of(&prefab.path)
            .is_some_and(|id| tree.is_subtype_of(id, ancestor))
    })
}
