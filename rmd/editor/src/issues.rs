use core::path::TreePath;
use std::collections::BTreeMap;

use dmm::Prefab;
use objtree::ObjectTree;

use crate::{
    command::Edit,
    document::{MapDocument, PrefabInstanceId},
    search,
    tool::{PlacementKind, ToolEdit, placement_kind},
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

pub fn has_replacement_kind(tree: &ObjectTree, source: &TreePath) -> bool { unknown_kind(tree, source).is_some() }

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
    document: &MapDocument, tree: &ObjectTree, source: &TreePath, instances: &[PrefabInstanceId],
) -> Option<ToolEdit> {
    search::delete_instances(document, &editable_instances(document, tree, source, instances))
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
        selected: None,
        affected,
    })
}
