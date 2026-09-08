use dmm::{Coord, Prefab};
use objtree::{ObjectTree, TypeId};

use crate::{
    command::Edit,
    document::{MapDocument, PrefabInstanceId},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    Place,
    #[default]
    Select,
    Delete,
}

pub struct ToolContext<'a> {
    pub document: &'a mut MapDocument,
    pub tree: &'a ObjectTree,
    pub prefab: Option<&'a Prefab>,
    pub coord: Coord,
    pub anchor: Option<Coord>,
}

pub struct ToolEdit {
    pub edit: Edit,
    pub selected: PrefabInstanceId,
    pub affected: Vec<PrefabInstanceId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlacementKind {
    Atom,
    Turf,
    Area,
}

impl Tool {
    pub fn label(self) -> &'static str {
        match self {
            Tool::Place => "Place",
            Tool::Select => "Select",
            Tool::Delete => "Delete",
        }
    }

    pub fn build_edit(self, context: &mut ToolContext<'_>) -> Option<ToolEdit> {
        match self {
            Self::Place => place(context),
            Self::Select | Self::Delete => None,
        }
    }
}

fn place(context: &mut ToolContext<'_>) -> Option<ToolEdit> {
    let prefab = context.prefab?.clone();
    let kind = placement_kind(context.tree, &prefab)?;
    let mut after = context.document.placed_tile(context.coord)?;
    let mut affected = Vec::new();

    let selected = match kind {
        PlacementKind::Turf | PlacementKind::Area => {
            let matching = after
                .iter()
                .enumerate()
                .filter_map(|(index, placed)| {
                    (placement_kind(context.tree, placed.prefab()) == Some(kind)).then_some(index)
                })
                .collect::<Vec<_>>();

            if let Some(first) = matching.first().copied() {
                let selected = after[first].id();
                *after[first].prefab_mut() = prefab.clone();
                affected.push(selected);
                for index in matching.into_iter().skip(1).rev() {
                    affected.push(after.remove(index).id());
                }

                selected
            } else {
                let placed = context.document.instantiate(prefab.clone());
                let selected = placed.id();
                let index = insertion_index(context.tree, &after, kind);
                after.insert(index, placed);
                affected.push(selected);

                selected
            }
        },
        PlacementKind::Atom => {
            let placed = context.document.instantiate(prefab.clone());
            let selected = placed.id();
            let index = insertion_index(context.tree, &after, kind);
            after.insert(index, placed);
            affected.push(selected);

            selected
        },
    };

    if context.document.placed_tile(context.coord).as_ref() == Some(&after) {
        return None;
    }

    let mut edit = Edit::new(format!("place {}", prefab.path));
    edit.change(context.document, context.coord, after);

    Some(ToolEdit {
        edit,
        selected,
        affected,
    })
}

fn insertion_index(tree: &ObjectTree, tile: &[crate::document::PlacedPrefab], kind: PlacementKind) -> usize {
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

fn placement_kind(tree: &ObjectTree, prefab: &Prefab) -> Option<PlacementKind> {
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

#[cfg(test)]
mod tests {
    use core::{location::Location, path::TreePath, types::Value};

    use dmm::{Map, Prefab, Size};

    use super::{Tool, ToolContext};
    use crate::document::MapDocument;

    fn tree() -> objtree::ObjectTree {
        let mut tree = objtree::ObjectTree::new();
        for path in [
            "/atom",
            "/atom/movable",
            "/obj",
            "/obj/table",
            "/obj/chair",
            "/turf",
            "/turf/floor",
            "/turf/wall",
            "/area",
            "/area/station",
            "/area/space",
            "/datum",
        ] {
            tree.register(&TreePath::parse(path), Location::default());
        }
        for (path, parent) in [
            ("/atom/movable", "/atom"),
            ("/obj", "/atom/movable"),
            ("/turf", "/atom"),
            ("/area", "/atom"),
        ] {
            let id = tree.id_of(&TreePath::parse(path)).unwrap();
            tree.get_mut(id).unwrap().parent_type = Some(TreePath::parse(parent));
        }
        tree.resolve_parent_types();

        tree
    }

    fn map_document(paths: &[&str]) -> MapDocument {
        let mut map = Map::new(Size { x: 1, y: 1, z: 1 });
        let key = map.intern_tile(paths.iter().map(|path| Prefab::new(TreePath::parse(path))).collect());
        map.grid[0][0][0] = key;

        MapDocument::new(map, 1)
    }

    fn place(document: &mut MapDocument, tree: &objtree::ObjectTree, prefab: &Prefab) -> Option<super::ToolEdit> {
        Tool::Place.build_edit(&mut ToolContext {
            document,
            tree,
            prefab: Some(prefab),
            coord: dmm::Coord::new(1, 1, 1),
            anchor: None,
        })
    }

    #[test]
    fn objects_append_before_turf_and_area_with_exact_overrides() {
        let tree = tree();
        let mut document = map_document(&["/obj/table", "/turf/floor", "/area/station"]);
        let mut chair = Prefab::new(TreePath::parse("/obj/chair"));
        chair.set_var("name".into(), Value::Text("custom".into()));
        let action = place(&mut document, &tree, &chair).unwrap();
        let selected = action.selected;
        document.apply(action.edit);

        let paths = document
            .map
            .tile_at(dmm::Coord::new(1, 1, 1))
            .unwrap()
            .iter()
            .map(|prefab| prefab.path.to_string())
            .collect::<Vec<_>>();
        assert_eq!(paths, ["/obj/table", "/obj/chair", "/turf/floor", "/area/station"]);
        assert_eq!(
            document.prefab_instance(selected).unwrap().0.var(&"name".into()),
            Some(&Value::Text("custom".into()))
        );
    }

    #[test]
    fn turf_and_area_replace_their_category_and_keep_the_first_id() {
        let tree = tree();
        let mut document = map_document(&["/turf/floor", "/turf/wall", "/area/station"]);
        let turf_id = document.instance_ids_at(dmm::Coord::new(1, 1, 1))[0];
        let before = document.placed_tile(dmm::Coord::new(1, 1, 1)).unwrap();
        let action = place(&mut document, &tree, &Prefab::new(TreePath::parse("/turf/wall"))).unwrap();
        assert_eq!(action.selected, turf_id);
        assert_eq!(action.affected.len(), 2);
        document.apply(action.edit);

        let tile = document.map.tile_at(dmm::Coord::new(1, 1, 1)).unwrap();
        assert_eq!(tile.len(), 2);
        assert_eq!(tile[0].path, TreePath::parse("/turf/wall"));
        assert_eq!(document.instance_ids_at(dmm::Coord::new(1, 1, 1))[0], turf_id);
        assert!(document.undo());
        assert_eq!(document.placed_tile(dmm::Coord::new(1, 1, 1)).unwrap(), before);

        let mut document = map_document(&["/obj/table", "/turf/floor", "/area/station"]);
        let area_id = document.instance_ids_at(dmm::Coord::new(1, 1, 1))[2];
        let action = place(&mut document, &tree, &Prefab::new(TreePath::parse("/area/space"))).unwrap();
        assert_eq!(action.selected, area_id);
        document.apply(action.edit);
        let tile = document.map.tile_at(dmm::Coord::new(1, 1, 1)).unwrap();
        assert_eq!(tile[2].path, TreePath::parse("/area/space"));
        assert_eq!(document.instance_ids_at(dmm::Coord::new(1, 1, 1))[2], area_id);
    }

    #[test]
    fn identical_category_replacement_is_a_noop_and_datums_are_rejected() {
        let tree = tree();
        let mut document = map_document(&["/turf/floor", "/area/station"]);

        assert!(place(&mut document, &tree, &Prefab::new(TreePath::parse("/turf/floor"))).is_none());
        assert!(place(&mut document, &tree, &Prefab::new(TreePath::parse("/datum"))).is_none());
    }
}
