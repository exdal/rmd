use core::path::TreePath;
use std::collections::{HashSet, VecDeque};

use dmm::{Coord, Prefab};
use objtree::{ObjectTree, TypeId};

use crate::{
    command::Edit,
    document::{MapDocument, PrefabInstanceId},
};

pub const MAX_FILL_TILES: usize = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    Place,
    #[default]
    Select,
    Delete,
    Fill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FillMode {
    #[default]
    Wall,
    EntireArea,
    Custom,
}

impl FillMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Wall => "Wall",
            Self::EntireArea => "Entire Area",
            Self::Custom => "Custom",
        }
    }
}

pub struct ToolContext<'a> {
    pub document: &'a mut MapDocument,
    pub tree: &'a ObjectTree,
    pub prefab: Option<&'a Prefab>,
    pub target: Option<PrefabInstanceId>,
    pub coord: Coord,
    pub anchor: Option<Coord>,
    pub fill_mode: FillMode,
    pub custom_fill_boundaries: &'a [TreePath],
}

pub struct ToolEdit {
    pub edit: Edit,
    pub selected: Option<PrefabInstanceId>,
    pub affected: Vec<PrefabInstanceId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillError {
    TooLarge { limit: usize },
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
            Tool::Fill => "Fill",
        }
    }

    pub fn build_edit(self, context: &mut ToolContext<'_>) -> Option<ToolEdit> {
        match self {
            Self::Place => place(context),
            Self::Delete => delete(context),
            Self::Select => None,
            Self::Fill => fill(context, Some(MAX_FILL_TILES)).ok().flatten(),
        }
    }

    pub fn build_fill_edit(
        self, context: &mut ToolContext<'_>, max_tiles: Option<usize>,
    ) -> Result<Option<ToolEdit>, FillError> {
        if self == Self::Fill {
            fill(context, max_tiles)
        } else {
            Ok(self.build_edit(context))
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
        selected: Some(selected),
        affected,
    })
}

fn delete(context: &mut ToolContext<'_>) -> Option<ToolEdit> {
    let target = context.target?;
    let location = context.document.instance_location(target)?;
    if !context.document.allows_edit_at(location.coord) {
        return None;
    }
    let mut after = context.document.placed_tile(location.coord)?;
    if after
        .get(location.prefab_index)
        .is_none_or(|placed| placed.id() != target)
    {
        return None;
    }
    let placed = after.remove(location.prefab_index);

    let mut edit = Edit::new(format!("delete {}", placed.prefab().path));
    edit.change(context.document, location.coord, after);

    Some(ToolEdit {
        edit,
        selected: None,
        affected: vec![target],
    })
}

fn fill(context: &mut ToolContext<'_>, max_tiles: Option<usize>) -> Result<Option<ToolEdit>, FillError> {
    if !coord_in_bounds(context.coord, context.document.map.size) || !context.document.allows_edit_at(context.coord) {
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
            Some(wall) => wall_region(context, wall),
            None => return Ok(None),
        },
        FillMode::EntireArea => area_region(context),
        FillMode::Custom => custom_region(context),
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
        selected: None,
        affected,
    }))
}

fn coord_in_bounds(coord: Coord, size: dmm::Size) -> bool {
    (1..=size.x).contains(&coord.x) && (1..=size.y).contains(&coord.y) && (1..=size.z).contains(&coord.z)
}

fn wall_region(context: &ToolContext<'_>, wall: TypeId) -> Vec<Coord> { boundary_region(context, &[wall]) }

fn custom_region(context: &ToolContext<'_>) -> Vec<Coord> {
    let boundaries = context
        .custom_fill_boundaries
        .iter()
        .filter_map(|path| context.tree.id_of(path))
        .collect::<Vec<_>>();

    if boundaries.is_empty() {
        return Vec::new();
    }

    boundary_region(context, &boundaries)
}

fn boundary_region(context: &ToolContext<'_>, boundaries: &[TypeId]) -> Vec<Coord> {
    let is_boundary = |coord| tile_has_any_subtype(context.tree, context.document, coord, boundaries);

    if is_boundary(context.coord) {
        return Vec::new();
    }

    let mut region = Vec::new();
    let mut pending = VecDeque::from([context.coord]);
    let mut visited = HashSet::from([context.coord]);

    while let Some(coord) = pending.pop_front() {
        if !context.document.allows_edit_at(coord) || is_boundary(coord) {
            continue;
        }

        region.push(coord);
        for neighbor in cardinal_neighbors(coord, context.document.map.size) {
            if visited.insert(neighbor) {
                pending.push_back(neighbor);
            }
        }
    }

    region
}

fn area_region(context: &ToolContext<'_>) -> Vec<Coord> {
    let area = match context.tree.roots().area {
        Some(area) => area,
        None => return Vec::new(),
    };
    let Some(seed) = prefab_of_subtype(context.tree, &context.document.map, context.coord, area).cloned() else {
        return Vec::new();
    };

    let mut region = Vec::new();
    let mut pending = VecDeque::from([context.coord]);
    let mut visited = HashSet::from([context.coord]);

    while let Some(coord) = pending.pop_front() {
        if !context.document.allows_edit_at(coord)
            || !prefab_of_subtype(context.tree, &context.document.map, coord, area)
                .is_some_and(|candidate| crate::frame::same_area(&seed, candidate))
        {
            continue;
        }

        region.push(coord);
        for neighbor in cardinal_neighbors(coord, context.document.map.size) {
            if visited.insert(neighbor) {
                pending.push_back(neighbor);
            }
        }
    }

    region
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

fn prefab_of_subtype<'a>(tree: &ObjectTree, map: &'a dmm::Map, coord: Coord, ancestor: TypeId) -> Option<&'a Prefab> {
    map.tile_at(coord)?.iter().find(|prefab| {
        tree.id_of(&prefab.path)
            .is_some_and(|id| tree.is_subtype_of(id, ancestor))
    })
}

fn would_replace_kind(
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

fn replace_kind(
    document: &mut MapDocument, tree: &ObjectTree, coord: Coord, prefab: &Prefab, kind: PlacementKind,
) -> Option<(crate::document::PlacedTile, Vec<PrefabInstanceId>)> {
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
    use std::collections::HashSet;

    use dmm::{Coord, Map, Prefab, Size};

    use super::{FillError, FillMode, MAX_FILL_TILES, Tool, ToolContext};
    use crate::{document::MapDocument, focus::AreaFocus};

    fn tree() -> objtree::ObjectTree {
        let mut tree = objtree::ObjectTree::new();
        for path in [
            "/atom",
            "/atom/movable",
            "/obj",
            "/obj/table",
            "/obj/chair",
            "/obj/window",
            "/obj/window/reinforced",
            "/obj/machinery/door/airlock",
            "/turf",
            "/turf/floor",
            "/turf/wall",
            "/turf/open/floor",
            "/turf/open/floor/blue",
            "/turf/open/space",
            "/turf/closed/wall",
            "/turf/closed/wall/reinforced",
            "/turf/closed/reinforced",
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

    fn grid_document(width: u32, height: u32, mut tile_at: impl FnMut(Coord) -> Vec<Prefab>) -> MapDocument {
        let mut map = Map::new(Size {
            x: width,
            y: height,
            z: 1,
        });
        for y in 1..=height {
            for x in 1..=width {
                let coord = Coord::new(x, y, 1);
                let key = map.intern_tile(tile_at(coord));
                map.grid[0][(height - y) as usize][(x - 1) as usize] = key;
            }
        }

        MapDocument::new(map, 1)
    }

    fn prefabs(paths: &[&str]) -> Vec<Prefab> { paths.iter().map(|path| Prefab::new(TreePath::parse(path))).collect() }

    fn turf_at(document: &MapDocument, coord: Coord) -> &Prefab {
        document
            .map
            .tile_at(coord)
            .unwrap()
            .iter()
            .find(|prefab| prefab.path.to_string().starts_with("/turf"))
            .unwrap()
    }

    fn area_at(document: &MapDocument, coord: Coord) -> &Prefab {
        document
            .map
            .tile_at(coord)
            .unwrap()
            .iter()
            .find(|prefab| prefab.path.to_string().starts_with("/area"))
            .unwrap()
    }

    fn fill(
        document: &mut MapDocument, tree: &objtree::ObjectTree, prefab: &Prefab, coord: Coord, fill_mode: FillMode,
    ) -> Option<super::ToolEdit> {
        fill_with_boundaries(document, tree, prefab, coord, fill_mode, &[])
    }

    fn fill_with_boundaries(
        document: &mut MapDocument, tree: &objtree::ObjectTree, prefab: &Prefab, coord: Coord, fill_mode: FillMode,
        custom_fill_boundaries: &[TreePath],
    ) -> Option<super::ToolEdit> {
        Tool::Fill.build_edit(&mut ToolContext {
            document,
            tree,
            prefab: Some(prefab),
            target: None,
            coord,
            anchor: None,
            fill_mode,
            custom_fill_boundaries,
        })
    }

    fn fill_with_limit(
        document: &mut MapDocument, tree: &objtree::ObjectTree, prefab: &Prefab, coord: Coord, fill_mode: FillMode,
        max_tiles: Option<usize>,
    ) -> Result<Option<super::ToolEdit>, FillError> {
        Tool::Fill.build_fill_edit(
            &mut ToolContext {
                document,
                tree,
                prefab: Some(prefab),
                target: None,
                coord,
                anchor: None,
                fill_mode,
                custom_fill_boundaries: &[],
            },
            max_tiles,
        )
    }

    fn place(document: &mut MapDocument, tree: &objtree::ObjectTree, prefab: &Prefab) -> Option<super::ToolEdit> {
        Tool::Place.build_edit(&mut ToolContext {
            document,
            tree,
            prefab: Some(prefab),
            target: None,
            coord: dmm::Coord::new(1, 1, 1),
            anchor: None,
            fill_mode: FillMode::default(),
            custom_fill_boundaries: &[],
        })
    }

    #[test]
    fn objects_append_before_turf_and_area_with_exact_overrides() {
        let tree = tree();
        let mut document = map_document(&["/obj/table", "/turf/floor", "/area/station"]);
        let mut chair = Prefab::new(TreePath::parse("/obj/chair"));
        chair.set_var("name".into(), Value::Text("custom".into()));
        let action = place(&mut document, &tree, &chair).unwrap();
        let selected = action.selected.unwrap();
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
        assert_eq!(action.selected, Some(turf_id));
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
        assert_eq!(action.selected, Some(area_id));
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

    #[test]
    fn delete_removes_only_the_target_and_undo_restores_its_id() {
        let tree = tree();
        let coord = dmm::Coord::new(1, 1, 1);
        let mut document = map_document(&["/obj/table", "/turf/floor", "/area/station"]);
        let target = document.instance_ids_at(coord)[0];
        document.select_instance(Some(target));

        let action = Tool::Delete
            .build_edit(&mut ToolContext {
                document: &mut document,
                tree: &tree,
                prefab: None,
                target: Some(target),
                coord,
                anchor: None,
                fill_mode: FillMode::default(),
                custom_fill_boundaries: &[],
            })
            .unwrap();
        assert_eq!(action.selected, None);
        assert_eq!(action.affected, [target]);
        document.apply(action.edit);

        let paths = document
            .map
            .tile_at(coord)
            .unwrap()
            .iter()
            .map(|prefab| prefab.path.to_string())
            .collect::<Vec<_>>();
        assert_eq!(paths, ["/turf/floor", "/area/station"]);
        assert_eq!(document.selected_instance(), None);
        assert_eq!(document.instance_location(target), None);

        assert!(
            Tool::Delete
                .build_edit(&mut ToolContext {
                    document: &mut document,
                    tree: &tree,
                    prefab: None,
                    target: Some(target),
                    coord,
                    anchor: None,
                    fill_mode: FillMode::default(),
                    custom_fill_boundaries: &[],
                })
                .is_none()
        );

        assert!(document.undo());
        assert_eq!(document.instance_ids_at(coord)[0], target);
        assert!(document.redo());
        assert_eq!(document.instance_location(target), None);
    }

    #[test]
    fn wall_fill_replaces_the_interior_and_preserves_boundaries_objects_areas_and_ids() {
        let tree = tree();
        let mut document = grid_document(5, 5, |coord| {
            let boundary = coord.x == 1 || coord.x == 5 || coord.y == 1 || coord.y == 5;
            let mut paths = if coord == Coord::new(3, 3, 1) {
                vec!["/obj/table"]
            } else {
                Vec::new()
            };
            paths.push(if boundary {
                "/turf/closed/wall"
            } else {
                "/turf/open/floor"
            });
            if coord == Coord::new(3, 3, 1) {
                paths.push("/turf/open/space");
            }
            paths.push("/area/station");

            prefabs(&paths)
        });
        let interior = (2..=4)
            .flat_map(|y| (2..=4).map(move |x| Coord::new(x, y, 1)))
            .collect::<Vec<_>>();
        let before = interior
            .iter()
            .map(|coord| (*coord, document.placed_tile(*coord).unwrap()))
            .collect::<Vec<_>>();
        let turf_ids = interior
            .iter()
            .map(|coord| {
                let tile = document.map.tile_at(*coord).unwrap();
                tile.iter()
                    .zip(document.instance_ids_at(*coord))
                    .find_map(|(prefab, id)| prefab.path.to_string().starts_with("/turf").then_some(*id))
                    .unwrap()
            })
            .collect::<Vec<_>>();
        let mut blue = Prefab::new(TreePath::parse("/turf/open/floor/blue"));
        blue.set_var("name".into(), Value::Text("custom blue".into()));

        let action = fill(&mut document, &tree, &blue, Coord::new(3, 3, 1), FillMode::Wall).unwrap();
        assert_eq!(action.selected, None);
        assert_eq!(action.edit.changes.len(), 9);
        assert_eq!(action.affected.len(), 10);
        document.apply(action.edit);

        for (index, coord) in interior.iter().enumerate() {
            assert_eq!(turf_at(&document, *coord), &blue);
            assert!(document.instance_ids_at(*coord).contains(&turf_ids[index]));
        }
        assert_eq!(
            turf_at(&document, Coord::new(1, 3, 1)).path,
            TreePath::parse("/turf/closed/wall")
        );
        let center_paths = document
            .map
            .tile_at(Coord::new(3, 3, 1))
            .unwrap()
            .iter()
            .map(|prefab| prefab.path.to_string())
            .collect::<Vec<_>>();
        assert_eq!(center_paths, ["/obj/table", "/turf/open/floor/blue", "/area/station"]);

        assert!(document.undo());
        for (coord, tile) in before {
            assert_eq!(document.placed_tile(coord), Some(tile));
        }
        assert!(!document.undo());
        assert!(document.redo());
        assert_eq!(turf_at(&document, Coord::new(3, 3, 1)), &blue);
    }

    #[test]
    fn wall_fill_may_reach_the_map_edge() {
        let tree = tree();
        let mut document = grid_document(3, 2, |coord| {
            if coord == Coord::new(2, 1, 1) {
                prefabs(&["/area/space"])
            } else {
                prefabs(&["/turf/open/space", "/area/space"])
            }
        });
        let floor = Prefab::new(TreePath::parse("/turf/open/floor"));

        let action = fill(&mut document, &tree, &floor, Coord::new(1, 1, 1), FillMode::Wall).unwrap();
        assert_eq!(action.edit.changes.len(), 6);
        document.apply(action.edit);

        for y in 1..=2 {
            for x in 1..=3 {
                assert_eq!(turf_at(&document, Coord::new(x, y, 1)), &floor);
            }
        }
    }

    #[test]
    fn wall_fill_only_uses_closed_turfs_as_boundaries() {
        let tree = tree();
        let window = Coord::new(3, 1, 1);
        let airlock = Coord::new(5, 3, 1);
        let solid_override = Coord::new(3, 5, 1);
        let mut document = grid_document(5, 5, |coord| {
            let boundary = coord.x == 1 || coord.x == 5 || coord.y == 1 || coord.y == 5;
            let mut tile = Vec::new();
            if coord == window {
                tile.push(Prefab::new(TreePath::parse("/obj/window")));
            } else if coord == airlock {
                tile.push(Prefab::new(TreePath::parse("/obj/machinery/door/airlock")));
            } else if coord == solid_override {
                let mut table = Prefab::new(TreePath::parse("/obj/table"));
                table.set_var("density".into(), Value::Num(1.0));
                tile.push(table);
            }
            tile.push(Prefab::new(TreePath::parse(if boundary && tile.is_empty() {
                "/turf/closed/wall"
            } else {
                "/turf/open/floor"
            })));
            tile.push(Prefab::new(TreePath::parse("/area/station")));

            tile
        });
        let blue = Prefab::new(TreePath::parse("/turf/open/floor/blue"));

        let action = fill(&mut document, &tree, &blue, Coord::new(3, 3, 1), FillMode::Wall).unwrap();
        assert_eq!(action.edit.changes.len(), 12);
        document.apply(action.edit);

        for coord in [window, airlock, solid_override] {
            assert_eq!(turf_at(&document, coord), &blue);
        }
    }

    #[test]
    fn custom_fill_uses_configured_types_and_their_subtypes_as_boundaries() {
        let tree = tree();
        let window = Coord::new(3, 1, 1);
        let mut document = grid_document(5, 5, |coord| {
            let boundary = coord.x == 1 || coord.x == 5 || coord.y == 1 || coord.y == 5;
            if coord == window {
                prefabs(&["/obj/window/reinforced", "/turf/open/floor", "/area/station"])
            } else {
                prefabs(&[
                    if boundary {
                        "/turf/closed/wall/reinforced"
                    } else {
                        "/turf/open/floor"
                    },
                    "/area/station",
                ])
            }
        });
        let blue = Prefab::new(TreePath::parse("/turf/open/floor/blue"));
        let boundaries = [TreePath::parse("/turf/closed/wall"), TreePath::parse("/obj/window")];

        let action = fill_with_boundaries(
            &mut document,
            &tree,
            &blue,
            Coord::new(3, 3, 1),
            FillMode::Custom,
            &boundaries,
        )
        .unwrap();
        assert_eq!(action.edit.changes.len(), 9);
        document.apply(action.edit);

        assert_eq!(turf_at(&document, Coord::new(3, 3, 1)), &blue);
        assert_eq!(
            turf_at(&document, Coord::new(1, 3, 1)).path,
            TreePath::parse("/turf/closed/wall/reinforced")
        );
        assert_eq!(turf_at(&document, window).path, TreePath::parse("/turf/open/floor"));

        let unresolved = [TreePath::parse("/obj/missing")];
        assert!(
            fill_with_boundaries(
                &mut document,
                &tree,
                &blue,
                Coord::new(3, 3, 1),
                FillMode::Custom,
                &unresolved,
            )
            .is_none()
        );
    }

    #[test]
    fn wall_and_entire_area_modes_can_replace_areas_without_changing_turfs() {
        let tree = tree();
        let station = Prefab::new(TreePath::parse("/area/station"));
        let mut document = grid_document(5, 5, |coord| {
            let boundary = coord.x == 1 || coord.x == 5 || coord.y == 1 || coord.y == 5;
            prefabs(&[
                if boundary {
                    "/turf/closed/wall"
                } else {
                    "/turf/open/floor"
                },
                "/area/space",
            ])
        });
        let center = Coord::new(3, 3, 1);
        let center_turf = turf_at(&document, center).clone();

        let action = fill(&mut document, &tree, &station, center, FillMode::Wall).unwrap();
        assert_eq!(action.edit.changes.len(), 9);
        document.apply(action.edit);

        assert_eq!(turf_at(&document, center), &center_turf);
        for y in 2..=4 {
            for x in 2..=4 {
                assert_eq!(area_at(&document, Coord::new(x, y, 1)), &station);
            }
        }
        assert_eq!(
            area_at(&document, Coord::new(1, 3, 1)).path,
            TreePath::parse("/area/space")
        );

        let mut document = grid_document(4, 1, |coord| {
            let mut area = Prefab::new(TreePath::parse("/area/space"));
            if coord.x == 4 {
                area.set_var("name".into(), Value::Text("separate".into()));
            }

            vec![Prefab::new(TreePath::parse("/turf/open/floor")), area]
        });
        let turfs = (1..=4)
            .map(|x| turf_at(&document, Coord::new(x, 1, 1)).clone())
            .collect::<Vec<_>>();

        let action = fill(
            &mut document,
            &tree,
            &station,
            Coord::new(1, 1, 1),
            FillMode::EntireArea,
        )
        .unwrap();
        assert_eq!(action.edit.changes.len(), 3);
        document.apply(action.edit);

        for (index, turf) in turfs.iter().enumerate() {
            let coord = Coord::new(index as u32 + 1, 1, 1);
            assert_eq!(turf_at(&document, coord), turf);
            assert_eq!(
                area_at(&document, coord).path,
                TreePath::parse(if coord.x == 4 { "/area/space" } else { "/area/station" })
            );
        }
    }

    #[test]
    fn entire_area_uses_the_connected_exact_area_and_floor_fill_preserves_walls() {
        let tree = tree();
        let mut document = grid_document(4, 1, |coord| {
            let turf = match coord.x {
                1 | 4 => "/turf/open/floor",
                2 => "/turf/closed/wall",
                _ => "/turf/open/space",
            };
            let mut area = Prefab::new(TreePath::parse("/area/station"));
            if coord.x == 4 {
                area.set_var("name".into(), Value::Text("Engineering".into()));
            }

            vec![Prefab::new(TreePath::parse(turf)), area]
        });
        let mut blue = Prefab::new(TreePath::parse("/turf/open/floor/blue"));
        blue.set_var("name".into(), Value::Text("blue".into()));

        let action = fill(&mut document, &tree, &blue, Coord::new(1, 1, 1), FillMode::EntireArea).unwrap();
        assert_eq!(action.edit.changes.len(), 2);
        document.apply(action.edit);

        assert_eq!(turf_at(&document, Coord::new(1, 1, 1)), &blue);
        assert_eq!(
            turf_at(&document, Coord::new(2, 1, 1)).path,
            TreePath::parse("/turf/closed/wall")
        );
        assert_eq!(turf_at(&document, Coord::new(3, 1, 1)), &blue);
        assert_eq!(
            turf_at(&document, Coord::new(4, 1, 1)).path,
            TreePath::parse("/turf/open/floor")
        );

        assert!(document.undo());
        let space = Prefab::new(TreePath::parse("/turf/open/space"));
        let action = fill(&mut document, &tree, &space, Coord::new(1, 1, 1), FillMode::EntireArea).unwrap();
        assert_eq!(action.edit.changes.len(), 2);
        document.apply(action.edit);
        for x in 1..=3 {
            assert_eq!(turf_at(&document, Coord::new(x, 1, 1)), &space);
        }
        assert_eq!(
            turf_at(&document, Coord::new(4, 1, 1)).path,
            TreePath::parse("/turf/open/floor")
        );
    }

    #[test]
    fn fill_rejects_non_turfs_and_clips_to_the_active_focus() {
        let tree = tree();
        let mut document = grid_document(3, 1, |_| prefabs(&["/turf/open/space", "/area/space"]));
        let seed = Coord::new(1, 1, 1);
        assert!(
            fill(
                &mut document,
                &tree,
                &Prefab::new(TreePath::parse("/turf/open/floor")),
                Coord::new(0, 1, 1),
                FillMode::Wall,
            )
            .is_none()
        );
        assert!(
            fill(
                &mut document,
                &tree,
                &Prefab::new(TreePath::parse("/obj/table")),
                Coord::new(1, 1, 1),
                FillMode::Wall,
            )
            .is_none()
        );

        let mut no_area = map_document(&["/turf/open/space"]);
        assert!(
            fill(
                &mut no_area,
                &tree,
                &Prefab::new(TreePath::parse("/turf/open/floor")),
                Coord::new(1, 1, 1),
                FillMode::EntireArea,
            )
            .is_none()
        );

        let mut sparse_tree = objtree::ObjectTree::new();
        for path in ["/atom", "/turf/open/space"] {
            sparse_tree.register(&TreePath::parse(path), Location::default());
        }
        let turf = sparse_tree.id_of(&TreePath::parse("/turf")).unwrap();
        sparse_tree.get_mut(turf).unwrap().parent_type = Some(TreePath::parse("/atom"));
        sparse_tree.resolve_parent_types();
        let mut sparse_document = map_document(&["/turf/open/space"]);
        let space = Prefab::new(TreePath::parse("/turf/open/space"));
        assert!(fill(&mut sparse_document, &sparse_tree, &space, seed, FillMode::Wall).is_none());

        let area_id = document.instance_ids_at(seed)[1];
        document.set_focus(Some(AreaFocus::new(
            seed,
            Prefab::new(TreePath::parse("/area/space")),
            area_id,
            HashSet::from([seed, Coord::new(2, 1, 1)]),
        )));
        let floor = Prefab::new(TreePath::parse("/turf/open/floor"));
        let action = fill(&mut document, &tree, &floor, seed, FillMode::Wall).unwrap();
        assert_eq!(action.edit.changes.len(), 2);
        document.apply(action.edit);

        assert_eq!(turf_at(&document, seed), &floor);
        assert_eq!(turf_at(&document, Coord::new(2, 1, 1)), &floor);
        assert_eq!(
            turf_at(&document, Coord::new(3, 1, 1)).path,
            TreePath::parse("/turf/open/space")
        );
    }

    #[test]
    fn fill_limit_counts_only_tiles_that_would_change() {
        let tree = tree();
        let blue = Prefab::new(TreePath::parse("/turf/open/floor/blue"));
        let mut document = grid_document(4, 1, |coord| {
            let turf = match coord.x {
                2 => "/turf/closed/wall",
                3 => "/turf/open/floor/blue",
                _ => "/turf/open/space",
            };

            prefabs(&[turf, "/area/station"])
        });

        assert_eq!(
            fill_with_limit(
                &mut document,
                &tree,
                &blue,
                Coord::new(1, 1, 1),
                FillMode::EntireArea,
                Some(1),
            )
            .err(),
            Some(FillError::TooLarge { limit: 1 })
        );
        let action = fill_with_limit(
            &mut document,
            &tree,
            &blue,
            Coord::new(1, 1, 1),
            FillMode::EntireArea,
            Some(2),
        )
        .unwrap()
        .unwrap();
        assert_eq!(action.edit.changes.len(), 2);
    }

    #[test]
    fn oversized_fill_is_rejected_at_the_default_limit_and_can_be_built_without_it() {
        let tree = tree();
        let width = MAX_FILL_TILES as u32 + 1;
        let mut document = grid_document(width, 1, |_| prefabs(&["/turf/open/space", "/area/space"]));
        let floor = Prefab::new(TreePath::parse("/turf/open/floor"));

        assert_eq!(
            fill_with_limit(
                &mut document,
                &tree,
                &floor,
                Coord::new(1, 1, 1),
                FillMode::Wall,
                Some(MAX_FILL_TILES),
            )
            .err(),
            Some(FillError::TooLarge { limit: MAX_FILL_TILES })
        );
        assert_eq!(
            turf_at(&document, Coord::new(width, 1, 1)).path,
            TreePath::parse("/turf/open/space")
        );
        assert!(!document.undo());

        let action = fill_with_limit(&mut document, &tree, &floor, Coord::new(1, 1, 1), FillMode::Wall, None)
            .unwrap()
            .unwrap();
        assert_eq!(action.edit.changes.len(), MAX_FILL_TILES + 1);
        document.apply(action.edit);
        assert_eq!(turf_at(&document, Coord::new(width, 1, 1)), &floor);
        assert!(document.undo());
        assert_eq!(
            turf_at(&document, Coord::new(width, 1, 1)).path,
            TreePath::parse("/turf/open/space")
        );
        assert!(!document.undo());
    }
}
