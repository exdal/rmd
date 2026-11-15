pub(crate) mod panel;
pub(crate) mod session;

use base::path::TreePath;
use std::collections::BTreeMap;

use dmm::{Coord, Prefab};
use objtree::ObjectTree;

use crate::{
    document::{MapDocument, PrefabInstanceId, command::Edit, instance::PrefabLocation, selection::Selection},
    environment::world::World,
    tool::{
        ToolEdit,
        place::{placement_kind, refill_missing_bases, same_prefabs},
    },
};

#[derive(Debug, Clone, PartialEq)]
pub enum SearchQuery {
    Prefab(Prefab),
    Type { path: TreePath, subtypes: bool },
}

impl SearchQuery {
    fn matcher<'a>(&'a self, tree: &'a ObjectTree) -> impl Fn(&Prefab) -> bool + 'a {
        let root = match self {
            Self::Type { path, subtypes: true } => tree.id_of(path),
            _ => None,
        };

        move |prefab| match self {
            Self::Prefab(target) => prefab == target,
            Self::Type { path, subtypes: false } => prefab.path.segments == path.segments,
            Self::Type { path, subtypes: true } => match (root, tree.id_of(&prefab.path)) {
                (Some(root), Some(id)) => tree.is_subtype_of(id, root),
                _ => prefab.path.segments.starts_with(&path.segments),
            },
        }
    }
}

pub fn find_instances(
    document: &MapDocument, tree: &ObjectTree, query: &SearchQuery, bounds: Option<Selection>,
) -> Vec<PrefabInstanceId> {
    let matches = query.matcher(tree);
    let mut found = document
        .prefab_instances()
        .filter(|instance| {
            bounds.is_none_or(|bounds| bounds.contains(instance.location.coord)) && matches(instance.prefab())
        })
        .collect::<Vec<_>>();
    found.sort_unstable_by_key(|instance| {
        let location = instance.location;
        (
            location.coord.z,
            location.coord.y,
            location.coord.x,
            location.prefab_index,
            instance.id.get(),
        )
    });

    found.into_iter().map(|instance| instance.id).collect::<Vec<_>>()
}

/// Drops the ones that no longer exist, and follows the ones that moved
pub fn resolve_instances(
    document: &MapDocument, instances: &[PrefabInstanceId],
) -> Vec<(PrefabInstanceId, PrefabLocation)> {
    instances
        .iter()
        .filter_map(|instance| {
            document
                .instance_location(*instance)
                .map(|location| (*instance, location))
        })
        .collect()
}

pub fn delete_instances(
    document: &mut MapDocument, tree: &ObjectTree, world: &World, instances: &[PrefabInstanceId],
) -> Option<ToolEdit> {
    let by_tile = editable_by_tile(document, instances);
    let mut edit = Edit::new(match instances {
        [instance] => format!("delete {}", document.prefab_instance(*instance)?.prefab().path),
        _ => format!("delete {} instances", by_tile.values().map(Vec::len).sum::<usize>()),
    });
    let mut affected = Vec::new();
    for (coord, targets) in by_tile {
        let before = document.placed_tile(coord)?;
        let mut after = before.clone();
        after.retain(|placed| !targets.contains(&placed.id()));
        let mut added = Vec::new();
        if let Some(base_tile) = &world.base_tile {
            added = refill_missing_bases(document, tree, base_tile, &mut after);
        }

        if same_prefabs(&before, &after) {
            continue;
        }

        affected.extend(targets);
        affected.extend(added);
        edit.change(document, coord, after);
    }

    finish(edit, affected)
}

/// Keeps each placement's id, and skips the ones a turf, area or movable can't become
pub fn replace_instances(
    document: &MapDocument, tree: &ObjectTree, instances: &[PrefabInstanceId], prefab: &Prefab,
) -> Option<ToolEdit> {
    let kind = placement_kind(tree, prefab)?;
    let mut edit = Edit::new(format!("replace with {}", prefab.path));
    let mut affected = Vec::new();
    for (coord, targets) in editable_by_tile(document, instances) {
        let mut after = document.placed_tile(coord)?;
        let mut changed = false;
        for placed in after.iter_mut().filter(|placed| targets.contains(&placed.id())) {
            if placement_kind(tree, placed.prefab()) != Some(kind) || placed.prefab() == prefab {
                continue;
            }
            *placed.prefab_mut() = prefab.clone();
            affected.push(placed.id());
            changed = true;
        }
        if changed {
            edit.change(document, coord, after);
        }
    }

    finish(edit, affected)
}

pub fn can_replace(document: &MapDocument, tree: &ObjectTree, instance: PrefabInstanceId, prefab: &Prefab) -> bool {
    document.prefab_instance(instance).is_some_and(|prefab_instance| {
        document.allows_edit_instance(instance)
            && prefab_instance.prefab() != prefab
            && placement_kind(tree, prefab_instance.prefab())
                .is_some_and(|kind| placement_kind(tree, prefab) == Some(kind))
    })
}

fn editable_by_tile(document: &MapDocument, instances: &[PrefabInstanceId]) -> BTreeMap<Coord, Vec<PrefabInstanceId>> {
    let mut by_tile = BTreeMap::<Coord, Vec<PrefabInstanceId>>::new();
    for instance in instances {
        if let Some(location) = document.instance_location(*instance)
            && document.allows_edit_instance(*instance)
        {
            by_tile.entry(location.coord).or_default().push(*instance);
        }
    }

    by_tile
}

fn finish(edit: Edit, mut affected: Vec<PrefabInstanceId>) -> Option<ToolEdit> {
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

#[cfg(test)]
mod tests;
