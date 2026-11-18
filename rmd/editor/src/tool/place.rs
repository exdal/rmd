use base::{coord::Coord, path::TreePath};
use std::collections::BTreeMap;

use objtree::{ObjectTree, TypeId};
use prefab::{Prefab, PrefabInstanceId};

use crate::{
    document::{MapDocument, command::Edit, grid::TileView, instance::PlacedPrefab, instance::PlacedTile},
    environment::world::{BaseTile, World},
    frame::options::HiddenTypes,
};

use super::{PlacementKind, ToolContext, ToolEdit};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BaseKind {
    Turf,
    Area,
}

impl BaseKind {
    fn placement(self) -> PlacementKind {
        match self {
            Self::Turf => PlacementKind::Turf,
            Self::Area => PlacementKind::Area,
        }
    }

    fn root(self) -> &'static str {
        match self {
            Self::Turf => "turf",
            Self::Area => "area",
        }
    }

    fn is_prefab(self, prefab: &Prefab) -> bool {
        prefab
            .path
            .segments
            .first()
            .is_some_and(|segment| segment.as_str() == self.root())
    }

    fn is_in(self, tile: &[PlacedPrefab]) -> bool {
        tile.iter().any(|placed| self.is_prefab(placed.prefab()))
    }

    fn refill(
        self, document: &mut MapDocument, tree: &ObjectTree, path: &TreePath, tile: &mut PlacedTile,
    ) -> Option<PrefabInstanceId> {
        if self.is_in(tile) {
            return None;
        }

        let placed = document.instantiate(Prefab::new(path.clone()));
        let id = placed.id();
        tile.insert(insertion_index(tree, tile, self.placement()), placed);

        Some(id)
    }
}

pub fn is_missing_base(tile: TileView<'_>) -> bool {
    [BaseKind::Turf, BaseKind::Area]
        .into_iter()
        .any(|kind| !tile.iter().any(|prefab| kind.is_prefab(prefab)))
}

pub fn same_prefabs(left: &[PlacedPrefab], right: &[PlacedPrefab]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| left.prefab() == right.prefab())
}

pub fn refill_missing_bases(
    document: &mut MapDocument, tree: &ObjectTree, base_tile: &BaseTile, tile: &mut PlacedTile,
) -> Vec<PrefabInstanceId> {
    [(BaseKind::Turf, &base_tile.turf), (BaseKind::Area, &base_tile.area)]
        .into_iter()
        .filter_map(|(kind, path)| kind.refill(document, tree, path, tile))
        .collect()
}

pub(super) fn place(context: &mut ToolContext<'_>) -> Option<ToolEdit> {
    let prefab = context.prefab?.clone();

    place_on_tile(context.document, context.tree, context.coord, &prefab, None)
}

pub fn place_replacing_objs(
    document: &mut MapDocument, tree: &ObjectTree, coord: Coord, prefab: &Prefab, hidden: &HiddenTypes,
) -> Option<ToolEdit> {
    place_on_tile(document, tree, coord, prefab, Some(hidden))
}

fn place_on_tile(
    document: &mut MapDocument, tree: &ObjectTree, coord: Coord, prefab: &Prefab, replaced_objs: Option<&HiddenTypes>,
) -> Option<ToolEdit> {
    let kind = placement_kind(tree, prefab)?;
    let mut tile = document.placed_tile(coord)?;
    let mut affected = Vec::new();
    if let Some(hidden) = replaced_objs
        && is_obj(tree, &prefab.path)
    {
        tile.retain(|placed| {
            let replaced = is_obj(tree, &placed.prefab().path) && !hidden.hides(placed.prefab());
            if replaced {
                affected.push(placed.id());
            }
            !replaced
        });
    }
    let (after, picked, placed) = place_prefab_into(document, tree, coord, tile, prefab, kind)?;
    affected.extend(placed);

    let mut edit = Edit::new(format!("place {}", prefab.path));
    edit.change(document, coord, after);

    Some(ToolEdit {
        edit,
        picked: Some(picked),
        affected,
    })
}

fn is_obj(tree: &ObjectTree, path: &TreePath) -> bool {
    tree.roots()
        .obj
        .zip(tree.id_of(path))
        .is_some_and(|(obj, id)| tree.is_subtype_of(id, obj))
}

pub(super) fn place_prefab_on_tile(
    document: &mut MapDocument, tree: &ObjectTree, coord: Coord, prefab: &Prefab, kind: PlacementKind,
) -> Option<(PlacedTile, PrefabInstanceId, Vec<PrefabInstanceId>)> {
    let tile = document.placed_tile(coord)?;

    place_prefab_into(document, tree, coord, tile, prefab, kind)
}

fn place_prefab_into(
    document: &mut MapDocument, tree: &ObjectTree, coord: Coord, mut after: PlacedTile, prefab: &Prefab,
    kind: PlacementKind,
) -> Option<(PlacedTile, PrefabInstanceId, Vec<PrefabInstanceId>)> {
    let mut affected = Vec::new();

    let picked = match kind {
        PlacementKind::Turf | PlacementKind::Area => {
            let matching = after
                .iter()
                .enumerate()
                .filter_map(|(index, placed)| (placement_kind(tree, placed.prefab()) == Some(kind)).then_some(index))
                .collect::<Vec<_>>();

            if let Some(first) = matching.first().copied() {
                let picked = after[first].id();
                *after[first].prefab_mut() = prefab.clone();
                affected.push(picked);
                for index in matching.into_iter().skip(1).rev() {
                    affected.push(after.remove(index).id());
                }

                picked
            } else {
                let placed = document.instantiate(prefab.clone());
                let picked = placed.id();
                let index = insertion_index(tree, &after, kind);
                after.insert(index, placed);
                affected.push(picked);

                picked
            }
        },
        PlacementKind::Atom => {
            let placed = document.instantiate(prefab.clone());
            let picked = placed.id();
            let index = insertion_index(tree, &after, kind);
            after.insert(index, placed);
            affected.push(picked);

            picked
        },
    };

    (document.placed_tile(coord).as_ref() != Some(&after)).then_some((after, picked, affected))
}

pub(super) fn delete(context: &mut ToolContext<'_>) -> Option<ToolEdit> {
    let target = context.target?;

    delete_instances(context.document, context.tree, context.world, &[target], "delete")
}

pub fn delete_instances(
    document: &mut MapDocument, tree: &ObjectTree, world: &World, targets: &[PrefabInstanceId], verb: &str,
) -> Option<ToolEdit> {
    let mut targets_by_tile = BTreeMap::<Coord, Vec<PrefabInstanceId>>::new();
    for target in targets {
        let Some(location) = document.instance_location(*target) else {
            continue;
        };

        if !document.allows_edit_instance(*target) {
            continue;
        }

        targets_by_tile.entry(location.coord).or_default().push(*target);
    }

    let mut changes = Vec::new();
    let mut affected = Vec::new();
    let mut removed = Vec::new();
    for (coord, ids) in targets_by_tile {
        let Some(before) = document.placed_tile(coord) else {
            continue;
        };

        let mut after = before.clone();
        let mut removed_on_tile = Vec::new();
        for id in ids {
            if let Some(index) = after.iter().position(|placed| placed.id() == id) {
                removed_on_tile.push(after.remove(index));
            }
        }

        if removed_on_tile.is_empty() {
            continue;
        }

        let mut affected_on_tile = removed_on_tile.iter().map(PlacedPrefab::id).collect::<Vec<_>>();
        if let Some(base_tile) = &world.base_tile {
            affected_on_tile.extend(refill_missing_bases(document, tree, base_tile, &mut after));
        }

        if same_prefabs(&before, &after) {
            continue;
        }

        affected.extend(affected_on_tile);
        removed.extend(removed_on_tile);
        changes.push((coord, after));
    }

    let label = match removed.as_slice() {
        [] => return None,
        [placed] => format!("{verb} {}", placed.prefab().path),
        _ => format!("{verb} {} atoms", removed.len()),
    };

    let mut edit = Edit::new(label);
    for (coord, after) in changes {
        edit.change(document, coord, after);
    }

    Some(ToolEdit {
        edit,
        picked: None,
        affected,
    })
}

pub(super) fn would_replace_kind(
    document: &MapDocument, tree: &ObjectTree, coord: Coord, prefab: &Prefab, kind: PlacementKind,
) -> bool {
    let Some(tile) = document.map.tile_at(coord) else {
        return false;
    };
    let mut matching = tile.iter().filter(|placed| placement_kind(tree, placed) == Some(kind));
    let Some(first) = matching.next() else {
        return true;
    };

    first != prefab || matching.next().is_some()
}

pub(super) fn replace_kind(
    document: &mut MapDocument, tree: &ObjectTree, coord: Coord, prefab: &Prefab, kind: PlacementKind,
) -> Option<(crate::document::instance::PlacedTile, Vec<PrefabInstanceId>)> {
    let mut after = document.placed_tile(coord)?;
    let matching = after
        .iter()
        .enumerate()
        .filter_map(|(index, placed)| (placement_kind(tree, placed.prefab()) == Some(kind)).then_some(index))
        .collect::<Vec<_>>();
    let mut affected = Vec::new();

    if let Some(first) = matching.first().copied() {
        let selected = after[first].id();
        *after[first].prefab_mut() = prefab.clone();
        affected.push(selected);
        for index in matching.into_iter().skip(1).rev() {
            affected.push(after.remove(index).id());
        }
    } else {
        let placed = document.instantiate(prefab.clone());
        let selected = placed.id();
        let index = insertion_index(tree, &after, kind);
        after.insert(index, placed);
        affected.push(selected);
    }

    (document.placed_tile(coord).as_ref() != Some(&after)).then_some((after, affected))
}

pub(crate) fn insertion_index(
    tree: &ObjectTree, tile: &[crate::document::instance::PlacedPrefab], kind: PlacementKind,
) -> usize {
    let rank = placement_rank(kind);

    tile.iter()
        .position(|placed| {
            placement_kind(tree, placed.prefab())
                .map(placement_rank)
                .is_some_and(|candidate| candidate > rank)
        })
        .unwrap_or(tile.len())
}

fn placement_rank(kind: PlacementKind) -> u8 {
    match kind {
        PlacementKind::Atom => 0,
        PlacementKind::Turf => 1,
        PlacementKind::Area => 2,
    }
}

pub fn placement_kind(tree: &ObjectTree, prefab: &Prefab) -> Option<PlacementKind> {
    let id = tree.id_of(&prefab.path)?;
    let roots = tree.roots();
    let atom = roots.atom?;
    if !tree.is_subtype_of(id, atom) {
        return None;
    }
    if roots.turf.is_some_and(|turf| tree.is_subtype_of(id, turf)) {
        return Some(PlacementKind::Turf);
    }
    if roots.area.is_some_and(|area| tree.is_subtype_of(id, area)) {
        return Some(PlacementKind::Area);
    }

    Some(PlacementKind::Atom)
}

pub fn is_placeable(tree: &ObjectTree, id: TypeId) -> bool {
    tree.roots().atom.is_some_and(|atom| tree.is_subtype_of(id, atom))
}
