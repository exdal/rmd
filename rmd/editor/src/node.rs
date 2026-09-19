use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashMap, HashSet, VecDeque},
};

use dmm::{Coord, Prefab, PrefabInstanceId};
use objtree::{ObjectTree, TypeId};
use vm::bake::NodeGroup;

use crate::document::MapDocument;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Component {
    pub tiles: HashSet<Coord>,
    pub nodes: Vec<Coord>,
    pub segments: Vec<(Coord, Coord)>,
}

pub type Connection = Vec<Coord>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedGroup {
    definition: NodeGroup,
    members: HashSet<TypeId>,
}

impl ResolvedGroup {
    pub fn subtype(&self) -> TypeId { self.definition.subtype }

    fn matches(&self, tree: &ObjectTree, prefab: &Prefab) -> bool {
        tree.id_of(&prefab.path).is_some_and(|ty| self.members.contains(&ty))
    }
}

pub fn group_for_prefab(tree: &ObjectTree, groups: &[NodeGroup], prefab: &Prefab) -> Option<usize> {
    let ty = tree.id_of(&prefab.path)?;

    group_for_type(tree, groups, ty)
}

fn group_for_type(tree: &ObjectTree, groups: &[NodeGroup], ty: TypeId) -> Option<usize> {
    let mut selected: Option<usize> = None;
    for (index, group) in groups.iter().enumerate() {
        if !tree.is_subtype_of(ty, group.subtype) {
            continue;
        }
        if selected.is_none_or(|current| {
            group.subtype != groups[current].subtype && tree.is_subtype_of(group.subtype, groups[current].subtype)
        }) {
            selected = Some(index);
        }
    }

    selected
}

pub fn resolve_group(tree: &ObjectTree, groups: &[NodeGroup], index: usize) -> Option<ResolvedGroup> {
    let definition = groups.get(index)?.clone();
    let members = tree
        .descendants(definition.subtype)
        .into_iter()
        .filter(|ty| group_for_type(tree, groups, *ty) == Some(index))
        .collect();

    Some(ResolvedGroup { definition, members })
}

pub fn eligible_instance_at(
    document: &MapDocument, tree: &ObjectTree, group: &ResolvedGroup, coord: Coord,
) -> Option<PrefabInstanceId> {
    document.instance_ids_at(coord).iter().rev().find_map(|id| {
        let (prefab, _) = document.prefab_instance(*id)?;

        group.matches(tree, prefab).then_some(*id)
    })
}

pub fn component(document: &MapDocument, tree: &ObjectTree, group: &ResolvedGroup, seed: Coord) -> Option<Component> {
    let tiles = component_tiles(document, tree, group, seed)?;

    let mut nodes = tiles
        .iter()
        .copied()
        .filter(|coord| {
            let directions = neighbor_directions(*coord, &tiles);

            match directions.as_slice() {
                [left, right] => left.opposite() != *right,
                _ => true,
            }
        })
        .collect::<Vec<_>>();
    nodes.sort_unstable_by_key(|coord| (coord.y, coord.x));

    let mut segments = Vec::new();
    for coord in &tiles {
        for direction in [Direction::East, Direction::North] {
            if let Some(neighbor) = step(*coord, direction, document.map.size.x, document.map.size.y)
                && tiles.contains(&neighbor)
            {
                segments.push((*coord, neighbor));
            }
        }
    }
    segments.sort_unstable_by_key(|(from, to)| (from.y, from.x, to.y, to.x));

    Some(Component { tiles, nodes, segments })
}

pub fn connections(component: &Component, manual: &HashSet<Coord>) -> Vec<Connection> {
    let mut boundaries = component.nodes.iter().copied().collect::<HashSet<_>>();
    boundaries.extend(manual.iter().copied().filter(|coord| component.tiles.contains(coord)));

    let mut adjacent = HashMap::<Coord, Vec<Coord>>::new();
    for (from, to) in &component.segments {
        adjacent.entry(*from).or_default().push(*to);
        adjacent.entry(*to).or_default().push(*from);
    }
    for neighbors in adjacent.values_mut() {
        neighbors.sort_unstable_by_key(|coord| (coord.z, coord.y, coord.x));
    }

    let mut starts = boundaries.iter().copied().collect::<Vec<_>>();
    starts.sort_unstable_by_key(|coord| (coord.z, coord.y, coord.x));
    let mut visited = HashSet::new();
    let mut result = Vec::new();
    for start in starts {
        let Some(neighbors) = adjacent.get(&start) else {
            continue;
        };
        for neighbor in neighbors.iter().copied() {
            if !visited.insert(edge(start, neighbor)) {
                continue;
            }

            let mut path = vec![start, neighbor];
            let mut previous = start;
            let mut current = neighbor;
            while !boundaries.contains(&current) {
                let Some(next) = adjacent
                    .get(&current)
                    .and_then(|neighbors| neighbors.iter().copied().find(|coord| *coord != previous))
                else {
                    break;
                };
                visited.insert(edge(current, next));
                path.push(next);
                previous = current;
                current = next;
            }

            if coord_key(*path.last().unwrap()) < coord_key(path[0]) {
                path.reverse();
            }
            result.push(path);
        }
    }
    result.sort_unstable_by(|left, right| {
        coord_key(left[0])
            .cmp(&coord_key(right[0]))
            .then_with(|| coord_key(*left.last().unwrap()).cmp(&coord_key(*right.last().unwrap())))
            .then_with(|| left.len().cmp(&right.len()))
    });

    result
}

pub fn connection_at_tile(connections: &[Connection], coord: Coord) -> Option<&Connection> {
    let mut interior = connections
        .iter()
        .filter(|connection| connection.len() > 2 && connection[1..connection.len() - 1].contains(&coord));
    let first = interior.next();
    if first.is_some() && interior.next().is_none() {
        return first;
    }

    let mut containing = connections.iter().filter(|connection| connection.contains(&coord));
    let first = containing.next();

    (first.is_some() && containing.next().is_none())
        .then_some(first)
        .flatten()
}

fn edge(left: Coord, right: Coord) -> (Coord, Coord) {
    if coord_key(left) <= coord_key(right) {
        (left, right)
    } else {
        (right, left)
    }
}

const fn coord_key(coord: Coord) -> (u32, u32, u32) { (coord.z, coord.y, coord.x) }

pub fn route(
    document: &MapDocument, tree: &ObjectTree, group: &ResolvedGroup, start: Coord, target: Coord,
) -> Option<Vec<Coord>> {
    route_with_context(document, tree, group, start, target, &HashSet::new())
}

pub fn route_with_context(
    document: &MapDocument, tree: &ObjectTree, group: &ResolvedGroup, start: Coord, target: Coord,
    transient: &HashSet<Coord>,
) -> Option<Vec<Coord>> {
    let size = document.map.size;
    if start.z != target.z
        || start.x == 0
        || start.y == 0
        || target.x == 0
        || target.y == 0
        || start.x > size.x
        || start.y > size.y
        || target.x > size.x
        || target.y > size.y
        || !traversable(document, tree, group, start, start, transient)
        || !traversable(document, tree, group, target, start, transient)
    {
        return None;
    }

    let start_state = State {
        coord: start,
        direction: Direction::None,
    };
    let mut sequence = 0_u64;
    let mut queue = BinaryHeap::from([QueueEntry {
        state: start_state,
        estimate: manhattan(start, target),
        steps: 0,
        bends: 0,
        sequence,
    }]);
    let mut best = HashMap::from([(start_state, Cost { steps: 0, bends: 0 })]);
    let mut previous = HashMap::new();
    let mut finish = None;

    while let Some(entry) = queue.pop() {
        let cost = Cost {
            steps: entry.steps,
            bends: entry.bends,
        };
        if best.get(&entry.state).is_some_and(|known| *known != cost) {
            continue;
        }
        if entry.state.coord == target {
            finish = Some(entry.state);
            break;
        }

        for direction in ordered_directions(entry.state.direction, entry.state.coord, target) {
            let Some(coord) = step(entry.state.coord, direction, size.x, size.y) else {
                continue;
            };
            if !traversable(document, tree, group, coord, start, transient) {
                continue;
            }

            let state = State { coord, direction };
            let next = Cost {
                steps: cost.steps + 1,
                bends: cost.bends + usize::from(entry.state.direction.is_turn(direction)),
            };
            if best.get(&state).is_some_and(|known| *known <= next) {
                continue;
            }
            best.insert(state, next);
            previous.insert(state, entry.state);
            sequence = sequence.wrapping_add(1);
            queue.push(QueueEntry {
                state,
                estimate: next.steps + manhattan(coord, target),
                steps: next.steps,
                bends: next.bends,
                sequence,
            });
        }
    }

    let mut current = finish?;
    let mut path = vec![current.coord];
    while current != start_state {
        current = *previous.get(&current)?;
        path.push(current.coord);
    }
    path.reverse();

    Some(erase_loops(path))
}

pub fn occupied(document: &MapDocument, tree: &ObjectTree, group: &ResolvedGroup, coord: Coord) -> bool {
    document
        .placed_tile(coord)
        .is_some_and(|tile| tile.iter().any(|placed| group.matches(tree, placed.prefab())))
}

pub fn instance_matches(
    document: &MapDocument, tree: &ObjectTree, group: &ResolvedGroup, id: PrefabInstanceId,
) -> bool {
    document
        .prefab_instance(id)
        .is_some_and(|(prefab, _)| group.matches(tree, prefab))
}

fn traversable(
    document: &MapDocument, tree: &ObjectTree, group: &ResolvedGroup, coord: Coord, start: Coord,
    transient: &HashSet<Coord>,
) -> bool {
    if coord != start
        && document.placed_tile(coord).is_some_and(|tile| {
            tile.iter().any(|placed| {
                group
                    .definition
                    .blockers
                    .iter()
                    .any(|blocker| prefab_matches(tree, placed.prefab(), *blocker))
            })
        })
    {
        return false;
    }

    if occupied_ignoring(document, tree, group, coord, transient) {
        return true;
    }

    document.allows_edit_at(coord)
}

fn component_tiles(
    document: &MapDocument, tree: &ObjectTree, group: &ResolvedGroup, seed: Coord,
) -> Option<HashSet<Coord>> {
    if !occupied(document, tree, group, seed) {
        return None;
    }

    let mut tiles = HashSet::new();
    let mut queue = VecDeque::from([seed]);
    while let Some(coord) = queue.pop_front() {
        if !tiles.insert(coord) {
            continue;
        }
        for neighbor in neighbors(coord, document.map.size.x, document.map.size.y) {
            if !tiles.contains(&neighbor) && occupied(document, tree, group, neighbor) {
                queue.push_back(neighbor);
            }
        }
    }

    Some(tiles)
}

fn occupied_ignoring(
    document: &MapDocument, tree: &ObjectTree, group: &ResolvedGroup, coord: Coord, ignored: &HashSet<Coord>,
) -> bool {
    !ignored.contains(&coord) && occupied(document, tree, group, coord)
}

fn prefab_matches(tree: &ObjectTree, prefab: &Prefab, root: TypeId) -> bool {
    tree.id_of(&prefab.path).is_some_and(|ty| tree.is_subtype_of(ty, root))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct State {
    coord: Coord,
    direction: Direction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Cost {
    steps: usize,
    bends: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QueueEntry {
    state: State,
    estimate: usize,
    steps: usize,
    bends: usize,
    sequence: u64,
}

impl Ord for QueueEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .estimate
            .cmp(&self.estimate)
            .then_with(|| other.bends.cmp(&self.bends))
            .then_with(|| other.steps.cmp(&self.steps))
            .then_with(|| other.sequence.cmp(&self.sequence))
    }
}

impl PartialOrd for QueueEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.cmp(other)) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Direction {
    None,
    North,
    South,
    East,
    West,
}

impl Direction {
    const fn opposite(self) -> Self {
        match self {
            Self::None => Self::None,
            Self::North => Self::South,
            Self::South => Self::North,
            Self::East => Self::West,
            Self::West => Self::East,
        }
    }

    fn is_turn(self, next: Self) -> bool { !matches!(self, Self::None) && self != next }
}

fn ordered_directions(current: Direction, coord: Coord, target: Coord) -> [Direction; 4] {
    let horizontal = if target.x >= coord.x {
        Direction::East
    } else {
        Direction::West
    };
    let vertical = if target.y >= coord.y {
        Direction::North
    } else {
        Direction::South
    };
    let preferred = if current == Direction::None {
        horizontal
    } else {
        current
    };
    let second = if preferred == horizontal { vertical } else { horizontal };

    [preferred, second, second.opposite(), preferred.opposite()]
}

fn erase_loops(path: Vec<Coord>) -> Vec<Coord> {
    let mut result = Vec::with_capacity(path.len());
    let mut positions = HashMap::new();
    for coord in path {
        if let Some(index) = positions.get(&coord).copied() {
            for removed in result.drain(index + 1..) {
                positions.remove(&removed);
            }
        } else {
            positions.insert(coord, result.len());
            result.push(coord);
        }
    }

    result
}

fn neighbor_directions(coord: Coord, tiles: &HashSet<Coord>) -> Vec<Direction> {
    [Direction::North, Direction::South, Direction::East, Direction::West]
        .into_iter()
        .filter(|direction| {
            let (dx, dy) = delta(*direction);
            let x = coord.x as i64 + dx;
            let y = coord.y as i64 + dy;
            x > 0 && y > 0 && tiles.contains(&Coord::new(x as u32, y as u32, coord.z))
        })
        .collect()
}

fn neighbors(coord: Coord, width: u32, height: u32) -> impl Iterator<Item = Coord> {
    [Direction::North, Direction::South, Direction::East, Direction::West]
        .into_iter()
        .filter_map(move |direction| step(coord, direction, width, height))
}

fn step(coord: Coord, direction: Direction, width: u32, height: u32) -> Option<Coord> {
    let (dx, dy) = delta(direction);
    let x = coord.x as i64 + dx;
    let y = coord.y as i64 + dy;
    (x > 0 && y > 0 && x <= width as i64 && y <= height as i64).then(|| Coord::new(x as u32, y as u32, coord.z))
}

const fn delta(direction: Direction) -> (i64, i64) {
    match direction {
        Direction::None => (0, 0),
        Direction::North => (0, 1),
        Direction::South => (0, -1),
        Direction::East => (1, 0),
        Direction::West => (-1, 0),
    }
}

fn manhattan(left: Coord, right: Coord) -> usize {
    left.x.abs_diff(right.x) as usize + left.y.abs_diff(right.y) as usize
}

#[cfg(test)]
mod tests {
    use core::{location::Location, path::TreePath};

    use dmm::{Map, Size};

    use super::*;

    fn tree() -> ObjectTree {
        let mut tree = ObjectTree::new();
        for path in [
            "/atom",
            "/atom/movable",
            "/obj",
            "/obj/cable",
            "/obj/cable/heavy",
            "/obj/pipe",
            "/turf",
            "/turf/open",
            "/turf/closed",
            "/turf/closed/wall",
            "/area",
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

    fn document(width: u32, height: u32, placements: &[(Coord, &str)]) -> MapDocument {
        let mut map = Map::new(Size {
            x: width,
            y: height,
            z: 1,
        });
        for y in 1..=height {
            for x in 1..=width {
                let coord = Coord::new(x, y, 1);
                let turf = placements
                    .iter()
                    .find_map(|(placed, path)| (*placed == coord && path.starts_with("/turf")).then_some(*path))
                    .unwrap_or("/turf/open");
                let mut tile = vec![
                    Prefab::new(TreePath::parse(turf)),
                    Prefab::new(TreePath::parse("/area")),
                ];
                tile.extend(
                    placements
                        .iter()
                        .filter(|(placed, path)| *placed == coord && !path.starts_with("/turf"))
                        .map(|(_, path)| Prefab::new(TreePath::parse(path))),
                );
                let key = map.intern_tile(tile);
                map.grid[0][(height - y) as usize][(x - 1) as usize] = key;
            }
        }

        MapDocument::new(map, 1)
    }

    fn group_definition(tree: &ObjectTree) -> NodeGroup {
        NodeGroup {
            subtype: tree.id_of(&TreePath::parse("/obj/cable")).unwrap(),
            blockers: vec![tree.id_of(&TreePath::parse("/turf/closed")).unwrap()],
        }
    }

    fn group(tree: &ObjectTree) -> ResolvedGroup { resolve_group(tree, &[group_definition(tree)], 0).unwrap() }

    #[test]
    fn most_specific_registered_group_wins() {
        let tree = tree();
        let groups = [
            NodeGroup {
                subtype: tree.id_of(&TreePath::parse("/obj")).unwrap(),
                blockers: Vec::new(),
            },
            group_definition(&tree),
        ];
        let prefab = Prefab::new(TreePath::parse("/obj/cable/heavy"));

        assert_eq!(group_for_prefab(&tree, &groups, &prefab), Some(1));
    }

    #[test]
    fn a_broad_group_excludes_types_claimed_by_a_more_specific_group() {
        let tree = tree();
        let groups = [
            NodeGroup {
                subtype: tree.id_of(&TreePath::parse("/obj")).unwrap(),
                blockers: Vec::new(),
            },
            group_definition(&tree),
        ];
        let broad = resolve_group(&tree, &groups, 0).unwrap();
        let cable = resolve_group(&tree, &groups, 1).unwrap();
        let pipe_coord = Coord::new(1, 1, 1);
        let cable_coord = Coord::new(2, 1, 1);
        let document = document(3, 1, &[(pipe_coord, "/obj/pipe"), (cable_coord, "/obj/cable/heavy")]);

        let broad_component = component(&document, &tree, &broad, pipe_coord).unwrap();
        assert_eq!(broad_component.tiles, HashSet::from([pipe_coord]));
        let cable_component = component(&document, &tree, &cable, cable_coord).unwrap();
        assert_eq!(cable_component.tiles, HashSet::from([cable_coord]));
    }

    #[test]
    fn component_nodes_include_endpoints_elbows_and_junctions() {
        let tree = tree();
        let cable = group(&tree);
        let placements = [
            (Coord::new(1, 1, 1), "/obj/cable"),
            (Coord::new(2, 1, 1), "/obj/cable"),
            (Coord::new(3, 1, 1), "/obj/cable"),
            (Coord::new(3, 2, 1), "/obj/cable"),
            (Coord::new(3, 3, 1), "/obj/cable"),
            (Coord::new(4, 2, 1), "/obj/cable/heavy"),
        ];
        let document = document(5, 4, &placements);
        let component = component(&document, &tree, &cable, Coord::new(1, 1, 1)).unwrap();

        assert_eq!(component.tiles.len(), placements.len());
        assert_eq!(component.segments.len(), placements.len() - 1);
        assert!(component.nodes.contains(&Coord::new(1, 1, 1)), "endpoint");
        assert!(!component.nodes.contains(&Coord::new(2, 1, 1)));
        assert!(component.nodes.contains(&Coord::new(3, 1, 1)), "elbow");
        assert!(component.nodes.contains(&Coord::new(3, 2, 1)), "junction");
        assert!(component.nodes.contains(&Coord::new(3, 3, 1)), "endpoint");
        assert!(component.nodes.contains(&Coord::new(4, 2, 1)), "endpoint");
    }

    #[test]
    fn isolated_component_is_a_node() {
        let tree = tree();
        let cable = group(&tree);
        let coord = Coord::new(2, 2, 1);
        let document = document(3, 3, &[(coord, "/obj/cable")]);

        let component = component(&document, &tree, &cable, coord).unwrap();

        assert_eq!(component.nodes, vec![coord]);
        assert!(connections(&component, &HashSet::new()).is_empty());
    }

    #[test]
    fn connections_are_bounded_by_structural_and_manual_nodes() {
        let tree = tree();
        let cable = group(&tree);
        let placements = [
            (Coord::new(1, 2, 1), "/obj/cable"),
            (Coord::new(2, 2, 1), "/obj/cable"),
            (Coord::new(3, 2, 1), "/obj/cable"),
            (Coord::new(4, 2, 1), "/obj/cable"),
            (Coord::new(5, 2, 1), "/obj/cable"),
            (Coord::new(3, 3, 1), "/obj/cable"),
        ];
        let document = document(5, 3, &placements);
        let component = component(&document, &tree, &cable, Coord::new(1, 2, 1)).unwrap();
        let manual = HashSet::from([Coord::new(2, 2, 1)]);

        let connections = connections(&component, &manual);
        assert_eq!(
            connections,
            vec![
                vec![Coord::new(1, 2, 1), Coord::new(2, 2, 1)],
                vec![Coord::new(2, 2, 1), Coord::new(3, 2, 1)],
                vec![Coord::new(3, 2, 1), Coord::new(4, 2, 1), Coord::new(5, 2, 1)],
                vec![Coord::new(3, 2, 1), Coord::new(3, 3, 1)],
            ]
        );
        assert_eq!(
            connection_at_tile(&connections, Coord::new(4, 2, 1)),
            Some(&connections[2]),
            "an interior tile identifies its connection"
        );
        assert_eq!(
            connection_at_tile(&connections, Coord::new(1, 2, 1)),
            Some(&connections[0]),
            "an endpoint belonging to one connection is unambiguous"
        );
        assert_eq!(
            connection_at_tile(&connections, Coord::new(3, 2, 1)),
            None,
            "a shared node needs the precise screen-space hit test"
        );
    }

    #[test]
    fn route_takes_a_short_cardinal_detour_around_blockers() {
        let tree = tree();
        let cable = group(&tree);
        let start = Coord::new(1, 2, 1);
        let target = Coord::new(5, 2, 1);
        let document = document(
            5,
            3,
            &[(start, "/obj/cable"), (Coord::new(3, 2, 1), "/turf/closed/wall")],
        );
        let path = route(&document, &tree, &cable, start, target).unwrap();

        assert_eq!(path.first(), Some(&start));
        assert_eq!(path.last(), Some(&target));
        assert_eq!(path.len(), 7);
        assert!(!path.contains(&Coord::new(3, 2, 1)));
        assert!(path.windows(2).all(|step| {
            let dx = step[0].x.abs_diff(step[1].x);
            let dy = step[0].y.abs_diff(step[1].y);

            dx + dy == 1
        }));
    }

    #[test]
    fn route_can_begin_away_from_the_target_to_escape_blockers() {
        let tree = tree();
        let cable = group(&tree);
        let start = Coord::new(3, 2, 1);
        let target = Coord::new(5, 2, 1);
        let document = document(
            5,
            4,
            &[
                (start, "/obj/cable"),
                (Coord::new(4, 2, 1), "/turf/closed/wall"),
                (Coord::new(3, 1, 1), "/turf/closed/wall"),
                (Coord::new(3, 3, 1), "/turf/closed/wall"),
            ],
        );

        let path = route(&document, &tree, &cable, start, target).expect("the westward escape reaches the target");

        assert_eq!(path.first(), Some(&start));
        assert_eq!(path.get(1), Some(&Coord::new(2, 2, 1)));
        assert_eq!(path.last(), Some(&target));
        assert_eq!(path.len(), 9);
        assert!(
            path.windows(2)
                .all(|step| step[0].x.abs_diff(step[1].x) + step[0].y.abs_diff(step[1].y) == 1)
        );
    }

    #[test]
    fn route_loop_erasure_removes_a_reversed_spur() {
        let elbow = Coord::new(3, 2, 1);
        let west = Coord::new(2, 2, 1);
        let east = Coord::new(4, 2, 1);

        assert_eq!(erase_loops(vec![elbow, west, elbow, east]), vec![elbow, east]);
    }

    #[test]
    fn route_reuses_group_tiles_and_reports_a_sealed_target_unreachable() {
        let tree = tree();
        let cable = group(&tree);
        let start = Coord::new(1, 2, 1);
        let existing = Coord::new(2, 2, 1);
        let target = Coord::new(4, 2, 1);
        let open = document(4, 3, &[(start, "/obj/cable"), (existing, "/obj/cable/heavy")]);
        let path = route(&open, &tree, &cable, start, target).unwrap();
        assert!(path.contains(&existing));

        let sealed = document(
            4,
            3,
            &[
                (start, "/obj/cable"),
                (Coord::new(2, 1, 1), "/turf/closed/wall"),
                (Coord::new(2, 2, 1), "/turf/closed/wall"),
                (Coord::new(2, 3, 1), "/turf/closed/wall"),
            ],
        );
        assert_eq!(route(&sealed, &tree, &cable, start, target), None);
    }

    #[test]
    fn route_can_pass_beside_another_same_group_component() {
        let tree = tree();
        let cable = group(&tree);
        let start = Coord::new(3, 3, 1);
        let target = Coord::new(5, 1, 1);
        let unrelated = HashSet::from([Coord::new(1, 1, 1), Coord::new(2, 1, 1)]);
        let document = document(
            5,
            4,
            &[
                (start, "/obj/cable"),
                (Coord::new(4, 3, 1), "/turf/closed/wall"),
                (Coord::new(1, 1, 1), "/obj/cable"),
                (Coord::new(2, 1, 1), "/obj/cable"),
            ],
        );
        let path = route(&document, &tree, &cable, start, target).unwrap();

        assert_eq!(path.first(), Some(&start));
        assert_eq!(path.last(), Some(&target));
        assert!(path.iter().all(|coord| !unrelated.contains(coord)));
    }

    #[test]
    fn route_can_intentionally_connect_to_an_occupied_target() {
        let tree = tree();
        let cable = group(&tree);
        let start = Coord::new(1, 2, 1);
        let target = Coord::new(4, 2, 1);
        let document = document(4, 3, &[(start, "/obj/cable"), (target, "/obj/cable/heavy")]);
        let path = route(&document, &tree, &cable, start, target).unwrap();

        assert_eq!(path.first(), Some(&start));
        assert_eq!(path.last(), Some(&target));
    }

    #[test]
    fn route_merges_when_the_target_is_next_to_the_same_group() {
        let tree = tree();
        let cable = group(&tree);
        let start = Coord::new(1, 2, 1);
        let target = Coord::new(3, 2, 1);
        let neighbor = Coord::new(4, 2, 1);
        let document = document(4, 3, &[(start, "/obj/cable"), (neighbor, "/obj/cable/heavy")]);
        let path = route(&document, &tree, &cable, start, target).unwrap();

        assert_eq!(path.first(), Some(&start));
        assert_eq!(path.last(), Some(&target));
        assert!(neighbors(target, 4, 3).any(|coord| coord == neighbor));
    }
}
