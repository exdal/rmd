use core::path::TreePath;
use std::collections::BTreeMap;

use dmm::{Coord, Prefab};
use objtree::ObjectTree;

use crate::{
    command::Edit,
    document::{MapDocument, PrefabInstanceId},
    search,
    tool::{PlacementKind, ToolEdit, is_missing_base, placement_kind, refill_missing_bases},
    world::World,
};

fn unknown_kind(tree: &ObjectTree, path: &TreePath) -> Option<PlacementKind> {
    if tree.id_of(path).is_some() {
        return None;
    }

    for length in (1..path.segments.len()).rev() {
        let ancestor = TreePath::new(path.segments[..length].to_vec(), true);
        if tree.id_of(&ancestor).is_some() {
            return placement_kind(tree, &Prefab::new(ancestor));
        }
    }

    None
}

pub fn has_replacement_kind(tree: &ObjectTree, source: &TreePath) -> bool {
    unknown_kind(tree, source).is_some()
}

pub fn can_replace(tree: &ObjectTree, source: &TreePath, target: &TreePath) -> bool {
    unknown_kind(tree, source).is_some_and(|kind| placement_kind(tree, &Prefab::new(target.clone())) == Some(kind))
}

pub fn editable_instances(
    document: &MapDocument, tree: &ObjectTree, source: &TreePath, instances: &[PrefabInstanceId],
) -> Vec<PrefabInstanceId> {
    if tree.id_of(source).is_some() {
        return Vec::new();
    }

    let mut editable = instances
        .iter()
        .copied()
        .filter(|id| {
            document.prefab_instance(*id).is_some_and(|instance| {
                instance.prefab().path.segments == source.segments && document.allows_edit_at(instance.location.coord)
            })
        })
        .collect::<Vec<_>>();

    editable.sort_unstable();
    editable.dedup();

    editable
}

pub fn delete_instances(
    document: &mut MapDocument, tree: &ObjectTree, world: &World, source: &TreePath, instances: &[PrefabInstanceId],
) -> Option<ToolEdit> {
    let editable = editable_instances(document, tree, source, instances);

    search::delete_instances(document, tree, world, &editable)
}

pub fn missing_base_tiles(document: &MapDocument) -> Vec<Coord> {
    let size = document.map.size();

    (1..=size.z)
        .flat_map(|z| (1..=size.y).flat_map(move |y| (1..=size.x).map(move |x| Coord::new(x, y, z))))
        .filter(|coord| document.map.tile_at(*coord).is_some_and(|tile| is_missing_base(tile)))
        .collect()
}

pub fn fill_missing_bases(
    document: &mut MapDocument, tree: &ObjectTree, world: &World, tiles: &[Coord],
) -> Option<ToolEdit> {
    let base_tile = world.base_tile.as_ref()?;
    let mut edit = Edit::new("fill missing turfs and areas");
    let mut affected = Vec::new();
    for coord in tiles.iter().copied() {
        if !document.allows_edit_at(coord) {
            continue;
        }

        let mut after = document.placed_tile(coord)?;
        let added = refill_missing_bases(document, tree, base_tile, &mut after);
        if added.is_empty() {
            continue;
        }

        affected.extend(added);
        edit.change(document, coord, after);
    }

    (!edit.is_empty()).then_some(ToolEdit {
        edit,
        picked: None,
        affected,
    })
}

/// This func changes __ONLY__ the paths, placements IDs, overrides and orders are retained.
pub fn replace_instances(
    document: &MapDocument, tree: &ObjectTree, source: &TreePath, instances: &[PrefabInstanceId], target: &TreePath,
) -> Option<ToolEdit> {
    if !can_replace(tree, source, target) {
        return None;
    }

    let affected = editable_instances(document, tree, source, instances);
    if affected.is_empty() {
        return None;
    }

    let mut by_tile = BTreeMap::<_, Vec<_>>::new();
    for id in &affected {
        by_tile
            .entry(document.instance_location(*id)?.coord)
            .or_default()
            .push(*id);
    }

    let mut edit = Edit::new(format!(
        "replace {} instances of {source} with {target}",
        affected.len()
    ));

    for (coord, targets) in by_tile {
        let mut after = document.placed_tile(coord)?;
        for placed in after.iter_mut().filter(|placed| targets.contains(&placed.id())) {
            placed.prefab_mut().path = target.clone();
        }

        edit.change(document, coord, after);
    }

    Some(ToolEdit {
        edit,
        picked: None,
        affected,
    })
}
