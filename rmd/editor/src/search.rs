use core::path::TreePath;
use std::collections::BTreeMap;

use dmm::{Coord, Prefab};
use objtree::ObjectTree;

use crate::{
    command::Edit,
    document::{MapDocument, PrefabInstanceId, PrefabLocation, Selection},
    tool::{ToolEdit, placement_kind},
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
        .filter(|(_, prefab, location)| bounds.is_none_or(|bounds| bounds.contains(location.coord)) && matches(prefab))
        .map(|(instance, _, location)| (instance, location))
        .collect::<Vec<_>>();
    found.sort_unstable_by_key(|(instance, location)| {
        (
            location.coord.z,
            location.coord.y,
            location.coord.x,
            location.prefab_index,
            instance.get(),
        )
    });

    found.into_iter().map(|(instance, _)| instance).collect()
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

pub fn delete_instances(document: &MapDocument, instances: &[PrefabInstanceId]) -> Option<ToolEdit> {
    let by_tile = editable_by_tile(document, instances);
    let mut edit = Edit::new(match instances {
        [instance] => format!("delete {}", document.prefab_instance(*instance)?.0.path),
        _ => format!("delete {} instances", by_tile.values().map(Vec::len).sum::<usize>()),
    });
    let mut affected = Vec::new();
    for (coord, targets) in by_tile {
        let mut after = document.placed_tile(coord)?;
        after.retain(|placed| !targets.contains(&placed.id()));
        affected.extend(targets);
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
    document.prefab_instance(instance).is_some_and(|(placed, location)| {
        document.allows_edit_at(location.coord)
            && placed != prefab
            && placement_kind(tree, placed).is_some_and(|kind| placement_kind(tree, prefab) == Some(kind))
    })
}

fn editable_by_tile(document: &MapDocument, instances: &[PrefabInstanceId]) -> BTreeMap<Coord, Vec<PrefabInstanceId>> {
    let mut by_tile = BTreeMap::<Coord, Vec<PrefabInstanceId>>::new();
    for instance in instances {
        if let Some(location) = document.instance_location(*instance)
            && document.allows_edit_at(location.coord)
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
        selected: None,
        affected,
    })
}

#[cfg(test)]
mod tests {
    use core::types::Value;

    use dmm::{Map, Size};

    use super::*;
    use crate::focus::AreaFocus;

    fn map() -> (Map, Prefab) {
        let mut target = Prefab::new(TreePath::parse("/obj/table"));
        target.set_var("name".into(), Value::Text(String::from("Conference")));
        let mut different_override = target.clone();
        different_override.set_var("name".into(), Value::Text(String::from("Coffee")));
        let different_path = Prefab::new(TreePath::parse("/obj/chair"));
        let subtype = Prefab::new(TreePath::parse("/obj/table/glass"));

        let mut map = Map::new(Size { x: 2, y: 1, z: 2 });
        let first = map.intern_tile(vec![target.clone(), different_override.clone()]);
        let second = map.intern_tile(vec![different_path, target.clone()]);
        let third = map.intern_tile(vec![target.clone()]);
        let fourth = map.intern_tile(vec![different_override, subtype]);
        map.grid[0][0][0] = first;
        map.grid[0][0][1] = second;
        map.grid[1][0][0] = third;
        map.grid[1][0][1] = fourth;

        (map, target)
    }

    fn table(subtypes: bool) -> SearchQuery {
        SearchQuery::Type {
            path: TreePath::parse("/obj/table"),
            subtypes,
        }
    }

    #[test]
    fn a_prefab_search_matches_the_exact_prefab_across_levels_in_tile_order() {
        let (map, target) = map();
        let document = MapDocument::new(map, 1);

        assert_eq!(
            find_instances(&document, &ObjectTree::default(), &SearchQuery::Prefab(target), None),
            [
                document.instance_ids_at(Coord::new(1, 1, 1))[0],
                document.instance_ids_at(Coord::new(2, 1, 1))[1],
                document.instance_ids_at(Coord::new(1, 1, 2))[0],
            ]
        );
    }

    #[test]
    fn a_type_search_includes_prefab_overrides_and_optionally_subtypes() {
        let (map, _) = map();
        let document = MapDocument::new(map, 1);
        let tree = ObjectTree::default();
        let exact = [
            document.instance_ids_at(Coord::new(1, 1, 1))[0],
            document.instance_ids_at(Coord::new(1, 1, 1))[1],
            document.instance_ids_at(Coord::new(2, 1, 1))[1],
            document.instance_ids_at(Coord::new(1, 1, 2))[0],
            document.instance_ids_at(Coord::new(2, 1, 2))[0],
        ];

        assert_eq!(find_instances(&document, &tree, &table(false), None), exact);
        assert_eq!(
            find_instances(&document, &tree, &table(true), None),
            [&exact[..], &[document.instance_ids_at(Coord::new(2, 1, 2))[1]]].concat()
        );
    }

    #[test]
    fn a_search_can_be_limited_to_a_selection() {
        let (map, _) = map();
        let document = MapDocument::new(map, 1);
        let bounds = Selection::from_drag(Coord::new(2, 1, 1), Coord::new(2, 1, 1));

        assert_eq!(
            find_instances(&document, &ObjectTree::default(), &table(false), Some(bounds)),
            [document.instance_ids_at(Coord::new(2, 1, 1))[1]]
        );
    }

    #[test]
    fn snapshots_follow_moves_and_drop_deleted_placements() {
        let (map, target) = map();
        let mut document = MapDocument::new(map, 1);
        let matches = find_instances(&document, &ObjectTree::default(), &SearchQuery::Prefab(target), None);
        let moved = matches[0];
        let deleted = matches[2];

        assert_eq!(
            document.move_instance(moved, Coord::new(2, 1, 1), "move table", &[], None),
            Some(true)
        );
        assert!(document.apply(delete_instances(&document, &[deleted]).unwrap().edit));

        let rows = resolve_instances(&document, &matches);
        assert_eq!(rows.len(), matches.len() - 1);
        assert_eq!(
            rows.iter()
                .find(|(instance, _)| *instance == moved)
                .map(|(_, location)| location.coord),
            Some(Coord::new(2, 1, 1))
        );
        assert!(rows.iter().all(|(instance, _)| *instance != deleted));
    }

    #[test]
    fn deleting_every_match_is_one_undo() {
        let (map, target) = map();
        let mut document = MapDocument::new(map, 1);
        let grid = document.map.grid.clone();
        let matches = find_instances(&document, &ObjectTree::default(), &SearchQuery::Prefab(target), None);

        let action = delete_instances(&document, &matches).unwrap();
        assert_eq!(action.edit.label, "delete 3 instances");
        assert!(document.apply(action.edit));
        assert!(resolve_instances(&document, &matches).is_empty());
        assert_eq!(document.instance_ids_at(Coord::new(1, 1, 1)).len(), 1);

        assert!(document.undo());
        assert_eq!(document.map.grid, grid);
        assert_eq!(resolve_instances(&document, &matches).len(), 3);
    }

    #[test]
    fn nothing_is_deleted_outside_the_focused_area() {
        let (map, target) = map();
        let mut document = MapDocument::new(map, 1);
        let matches = find_instances(
            &document,
            &ObjectTree::default(),
            &SearchQuery::Prefab(target.clone()),
            None,
        );
        let seed = Coord::new(1, 1, 1);
        document.set_focus(Some(AreaFocus::new(seed, target, matches[0], [seed].into())));

        let action = delete_instances(&document, &matches).unwrap();
        assert_eq!(action.affected, [matches[0]]);
    }
}
