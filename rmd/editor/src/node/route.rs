use base::{
    coord::Coord,
    types::{Identifier, Value},
    vars,
};
use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashMap, HashSet, VecDeque},
};

use bitflags::bitflags;
use objtree::{ObjectTree, TypeId};
use prefab::{Prefab, PrefabInstanceId};
use vm::bake::{NodeGroup, NodePortRule};

use crate::{
    document::{MapDocument, instance::PlacedTile},
    frame::visual,
};

bitflags! {
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct NodePorts: u32 {
        const NORTH = 1;
        const SOUTH = 2;
        const EAST = 4;
        const WEST = 8;
        const VERTICAL = Self::NORTH.bits() | Self::SOUTH.bits();
        const HORIZONTAL = Self::EAST.bits() | Self::WEST.bits();
        const NORTHEAST = Self::NORTH.bits() | Self::EAST.bits();
        const SOUTHEAST = Self::SOUTH.bits() | Self::EAST.bits();
        const NORTHWEST = Self::NORTH.bits() | Self::WEST.bits();
        const SOUTHWEST = Self::SOUTH.bits() | Self::WEST.bits();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Component {
    pub tiles: HashSet<Coord>,
    pub nodes: Vec<Coord>,
    pub segments: Vec<(Coord, Coord)>,
}

pub type Connection = Vec<Coord>;

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedGroup {
    definition: NodeGroup,
    members: HashSet<TypeId>,
    key: Vec<(Identifier, Option<Value>)>,
}

impl ResolvedGroup {
    pub fn shapes(&self, tree: &ObjectTree, prefab: &Prefab) -> bool {
        self.definition
            .orientable_subtype
            .is_some_and(|root| tree.id_of(&prefab.path).is_some_and(|ty| tree.is_subtype_of(ty, root)))
    }

    pub fn matches(&self, tree: &ObjectTree, prefab: &Prefab) -> bool {
        tree.id_of(&prefab.path).is_some_and(|ty| {
            self.members.contains(&ty)
                && self
                    .key
                    .iter()
                    .all(|(name, value)| resolved_var(tree, ty, prefab, name) == value.as_ref())
        })
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

pub fn resolve_group(tree: &ObjectTree, groups: &[NodeGroup], index: usize, seed: &Prefab) -> Option<ResolvedGroup> {
    let definition = groups.get(index)?.clone();
    let members = tree
        .descendants(definition.subtype)
        .into_iter()
        .filter(|ty| group_for_type(tree, groups, *ty) == Some(index))
        .collect();
    let seed_ty = tree.id_of(&seed.path)?;
    let key = definition
        .matching
        .iter()
        .map(|name| (name.clone(), resolved_var(tree, seed_ty, seed, name).cloned()))
        .collect();

    Some(ResolvedGroup {
        definition,
        members,
        key,
    })
}

fn resolved_var<'a>(tree: &'a ObjectTree, ty: TypeId, prefab: &'a Prefab, name: &Identifier) -> Option<&'a Value> {
    visual::resolve_value(tree, ty, prefab, name).map(|resolved| resolved.value)
}

pub fn eligible_instance_at(
    document: &MapDocument, tree: &ObjectTree, group: &ResolvedGroup, coord: Coord,
) -> Option<PrefabInstanceId> {
    document.instance_ids_at(coord).iter().rev().find_map(|id| {
        let prefab = document.prefab_instance(*id)?.prefab();

        group.matches(tree, prefab).then_some(*id)
    })
}

pub fn component(document: &MapDocument, tree: &ObjectTree, group: &ResolvedGroup, seed: Coord) -> Option<Component> {
    let tiles = component_tiles(document, tree, group, seed)?;

    let mut segments = Vec::new();
    for coord in &tiles {
        for direction in [Direction::East, Direction::North] {
            if let Some(neighbor) = step(*coord, direction, document.map.size().x, document.map.size().y)
                && tiles.contains(&neighbor)
                && connected(document, tree, group, *coord, neighbor)
            {
                segments.push((*coord, neighbor));
            }
        }
    }
    segments.sort_unstable_by_key(|(from, to)| (from.y, from.x, to.y, to.x));
    let mut adjacent = HashMap::<Coord, Vec<Direction>>::new();
    for (from, to) in &segments {
        adjacent.entry(*from).or_default().push(direction_between(*from, *to)?);
        adjacent.entry(*to).or_default().push(direction_between(*to, *from)?);
    }

    let mut nodes = tiles
        .iter()
        .copied()
        .filter(|coord| {
            let directions = adjacent.get(coord).map(Vec::as_slice).unwrap_or_default();

            match directions {
                [left, right] => left.opposite() != *right,
                _ => true,
            }
        })
        .collect::<Vec<_>>();
    nodes.sort_unstable_by_key(|coord| (coord.y, coord.x));

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

const fn coord_key(coord: Coord) -> (u32, u32, u32) {
    (coord.z, coord.y, coord.x)
}

#[cfg(test)]
pub fn route(
    document: &MapDocument, tree: &ObjectTree, group: &ResolvedGroup, start: Coord, target: Coord,
) -> Option<Vec<Coord>> {
    route_with_context(document, tree, group, start, target, &HashSet::new())
}

pub fn route_with_context(
    document: &MapDocument, tree: &ObjectTree, group: &ResolvedGroup, start: Coord, target: Coord,
    transient: &HashSet<Coord>,
) -> Option<Vec<Coord>> {
    let size = document.map.size();
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
        .is_some_and(|instance| group.matches(tree, instance.prefab()))
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

        for neighbor in neighbors(coord, document.map.size().x, document.map.size().y) {
            if !tiles.contains(&neighbor)
                && occupied(document, tree, group, neighbor)
                && connected(document, tree, group, coord, neighbor)
            {
                queue.push_back(neighbor);
            }
        }
    }

    Some(tiles)
}

fn group_prefab_at<'a>(
    document: &'a MapDocument, tree: &ObjectTree, group: &ResolvedGroup, coord: Coord,
    original: Option<&'a HashMap<Coord, PlacedTile>>,
) -> Option<&'a Prefab> {
    if let Some(tile) = original.and_then(|original| original.get(&coord)) {
        return tile
            .iter()
            .rev()
            .find_map(|placed| group.matches(tree, placed.prefab()).then_some(placed.prefab()));
    }

    let id = eligible_instance_at(document, tree, group, coord)?;

    document.prefab_instance(id).map(|instance| instance.prefab())
}

fn connected(document: &MapDocument, tree: &ObjectTree, group: &ResolvedGroup, from: Coord, to: Coord) -> bool {
    if group.definition.orientable_subtype.is_none() {
        return true;
    }

    let Some(direction) = direction_between(from, to) else {
        return false;
    };
    let Some(from_prefab) = group_prefab_at(document, tree, group, from, None) else {
        return false;
    };
    let Some(to_prefab) = group_prefab_at(document, tree, group, to, None) else {
        return false;
    };

    ports(tree, group, from_prefab).contains(direction.port())
        && ports(tree, group, to_prefab).contains(direction.opposite().port())
}

fn resolved_number(tree: &ObjectTree, prefab: &Prefab, name: &str) -> Option<u32> {
    let id = tree.id_of(&prefab.path)?;
    visual::resolve_value(tree, id, prefab, &Identifier::from(name))?
        .value
        .as_num()
        .filter(|number| number.is_finite() && *number >= 0.0)
        .map(|number| number as u32)
}

fn effective_direction(tree: &ObjectTree, prefab: &Prefab) -> u32 {
    resolved_number(tree, prefab, vars::DIR).unwrap_or_else(|| visual::resolve(tree, prefab).dir)
}

fn ports(tree: &ObjectTree, group: &ResolvedGroup, prefab: &Prefab) -> NodePorts {
    let Some(ty) = tree.id_of(&prefab.path) else {
        return NodePorts::empty();
    };
    if group.definition.ports == NodePortRule::AllSides {
        return NodePorts::all();
    }

    let direction = effective_direction(tree, prefab);

    group
        .definition
        .orientations
        .iter()
        .filter(|rule| rule.direction == direction && tree.is_subtype_of(ty, rule.subtype))
        .max_by_key(|rule| tree.ancestors(rule.subtype).count())
        .and_then(|rule| NodePorts::from_bits(rule.openings))
        .unwrap_or_default()
}

fn icon_direction(tree: &ObjectTree, group: &ResolvedGroup, prefab: &Prefab, required: NodePorts) -> Option<u32> {
    let ty = tree.id_of(&prefab.path)?;
    let subtype = group
        .definition
        .orientations
        .iter()
        .filter(|rule| tree.is_subtype_of(ty, rule.subtype))
        .max_by_key(|rule| tree.ancestors(rule.subtype).count())?
        .subtype;

    group.definition.orientations.iter().find_map(|rule| {
        let openings = NodePorts::from_bits(rule.openings)?;
        (rule.subtype == subtype && openings.bits().count_ones() == 2 && openings.contains(required))
            .then_some(rule.direction)
    })
}

fn direction_between(from: Coord, to: Coord) -> Option<Direction> {
    if from.z != to.z {
        return None;
    }

    match (to.x as i64 - from.x as i64, to.y as i64 - from.y as i64) {
        (0, 1) => Some(Direction::North),
        (0, -1) => Some(Direction::South),
        (1, 0) => Some(Direction::East),
        (-1, 0) => Some(Direction::West),
        _ => None,
    }
}

/// Orient the configured two-port segment along the candidate path. Previously
/// connected neighbors keep their openings. A third required opening is unsafe.
pub fn oriented_route_directions(
    document: &MapDocument, tree: &ObjectTree, group: &ResolvedGroup, brush: &Prefab, path: &[Coord],
    original: &HashMap<Coord, PlacedTile>,
) -> Option<HashMap<Coord, u32>> {
    if !group.shapes(tree, brush) {
        return Some(HashMap::new());
    }

    let mut required = HashMap::<Coord, NodePorts>::new();
    for pair in path.windows(2) {
        let direction = direction_between(pair[0], pair[1])?;
        for (coord, port) in [(pair[0], direction.port()), (pair[1], direction.opposite().port())] {
            if let Some(prefab) = group_prefab_at(document, tree, group, coord, Some(original))
                && !group.shapes(tree, prefab)
                && !ports(tree, group, prefab).contains(port)
            {
                return None;
            }
            *required.entry(coord).or_default() |= port;
        }
    }

    let mut directions = HashMap::new();
    for coord in path.iter().copied() {
        let existing = group_prefab_at(document, tree, group, coord, Some(original));
        if existing.is_some_and(|prefab| !group.shapes(tree, prefab)) {
            continue;
        }

        let mut mask = required.get(&coord).copied().unwrap_or_default();
        if let Some(prefab) = existing {
            let old_ports = ports(tree, group, prefab);
            for direction in [Direction::North, Direction::South, Direction::East, Direction::West] {
                let Some(neighbor) = step(coord, direction, document.map.size().x, document.map.size().y) else {
                    continue;
                };
                if let Some(other) = group_prefab_at(document, tree, group, neighbor, Some(original))
                    && old_ports.contains(direction.port())
                    && ports(tree, group, other).contains(direction.opposite().port())
                {
                    mask |= direction.port();
                }
            }
        }

        let is_all_sides = group.definition.ports == NodePortRule::AllSides;
        let template = existing.unwrap_or(brush);
        let icon = if mask.bits().count_ones() > 2 {
            None
        } else {
            icon_direction(tree, group, template, mask)
        };

        let dir = match icon {
            Some(dir) => dir,
            None if is_all_sides => {
                if existing.is_some() {
                    continue;
                }

                match icon_direction(tree, group, brush, required.get(&coord).copied().unwrap_or_default()) {
                    Some(dir) => dir,
                    None => continue,
                }
            },
            None if mask.bits().count_ones() > 2 => return None,
            None if mask.is_empty() => continue,
            None => return None,
        };

        if existing.is_some_and(|prefab| effective_direction(tree, prefab) != dir) && !document.allows_edit_at(coord) {
            return None;
        }

        directions.insert(coord, dir);
    }

    Some(directions)
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
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
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

    const fn port(self) -> NodePorts {
        match self {
            Self::None => NodePorts::empty(),
            Self::North => NodePorts::NORTH,
            Self::South => NodePorts::SOUTH,
            Self::East => NodePorts::EAST,
            Self::West => NodePorts::WEST,
        }
    }

    fn is_turn(self, next: Self) -> bool {
        !matches!(self, Self::None) && self != next
    }
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
mod tests;
