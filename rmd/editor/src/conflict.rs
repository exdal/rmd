use std::collections::{HashMap, HashSet, VecDeque};

use dmm::{
    Coord,
    Prefab,
    Tile,
    merge::{TileConflict, tiles_equal},
};

use crate::{
    bake::{HIGHLIGHT_ALWAYS, Highlight, highlight_tiles},
    command::{Edit, EditGroupId, History},
    document::MapDocument,
    git::Operation,
};

pub const OURS_LABEL: &str = "HEAD";

pub const UNRESOLVED_COLOR: [f32; 3] = [1.0, 0.25, 0.2];
pub const OURS_COLOR: [f32; 3] = [0.3, 0.6, 1.0];
pub const THEIRS_COLOR: [f32; 3] = [1.0, 0.6, 0.15];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Ours,
    Theirs,
}

/// What loading an unmerged map produced
#[derive(Debug, Clone)]
pub struct ConflictData {
    pub operation: Option<Operation>,
    pub conflicts: Vec<TileConflict>,
}

/// Connected unresolved tiles on one level
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub tiles: Vec<Coord>,
    /// north west corner of the bounding box
    pub anchor: Coord,
    pub min: Coord,
    pub max: Coord,
}

impl Region {
    /// Stable while the region keeps its tiles
    pub fn id(&self) -> u64 {
        let first = self.tiles.first().copied().unwrap_or(self.anchor);

        (u64::from(first.z) << 40) | (u64::from(first.y) << 20) | u64::from(first.x)
    }
}

/// Effective resolutions at one point in the history
#[derive(Debug, Clone, Default)]
pub struct Resolved(HashMap<Coord, Side>);

impl Resolved {
    pub fn side(&self, coord: Coord) -> Option<Side> { self.0.get(&coord).copied() }
}

/// One conflicting tile as the conflict table lists it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictRow {
    pub coord: Coord,
    /// 1 based, numbered over every conflict so it holds while tiles get resolved
    pub region: usize,
}

#[derive(Debug, Clone)]
pub struct ConflictState {
    pub operation: Option<Operation>,
    tiles: HashMap<Coord, TileConflict>,
    order: Vec<Coord>,
    rows: Vec<ConflictRow>,
    resolutions: HashMap<Coord, Vec<(Side, EditGroupId)>>,
    revision: u64,
}

pub fn describe_tile(tile: Option<&Tile>) -> String {
    match tile {
        None => String::from("(outside the map)"),
        Some(tile) if tile.is_empty() => String::from("(empty)"),
        Some(tile) => tile.iter().map(describe_prefab).collect::<Vec<_>>().join("\n"),
    }
}

pub fn describe_prefab(prefab: &Prefab) -> String {
    match prefab.vars.len() {
        0 => prefab.path.to_string(),
        count => format!("{} {{{count} vars}}", prefab.path),
    }
}

/// Replaces the tile at `coord` unless it already matches `target`
pub fn set_tile(edit: &mut Edit, document: &mut MapDocument, coord: Coord, target: Tile) {
    if document
        .map
        .tile_at(coord)
        .is_some_and(|current| tiles_equal(current, &target))
    {
        return;
    }

    let after = target.into_iter().map(|prefab| document.instantiate(prefab)).collect();
    edit.change(document, coord, after);
}

/// Groups touching tiles, diagonals included, sorted by level then from the north west
pub fn connected_regions(coords: impl IntoIterator<Item = Coord>) -> Vec<Region> {
    let order = coords.into_iter().collect::<Vec<_>>();
    let open = order.iter().copied().collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    let mut regions = Vec::new();

    for start in order {
        if !seen.insert(start) {
            continue;
        }

        let mut tiles = Vec::new();
        let mut queue = VecDeque::from([start]);
        while let Some(coord) = queue.pop_front() {
            tiles.push(coord);
            for dy in -1i64..=1 {
                for dx in -1i64..=1 {
                    let (Ok(x), Ok(y)) = (
                        u32::try_from(i64::from(coord.x) + dx),
                        u32::try_from(i64::from(coord.y) + dy),
                    ) else {
                        continue;
                    };
                    let next = Coord::new(x, y, coord.z);
                    if open.contains(&next) && seen.insert(next) {
                        queue.push_back(next);
                    }
                }
            }
        }

        tiles.sort_unstable_by_key(|coord| (std::cmp::Reverse(coord.y), coord.x));
        let min = Coord::new(
            tiles.iter().map(|coord| coord.x).min().unwrap_or(start.x),
            tiles.iter().map(|coord| coord.y).min().unwrap_or(start.y),
            start.z,
        );
        let max = Coord::new(
            tiles.iter().map(|coord| coord.x).max().unwrap_or(start.x),
            tiles.iter().map(|coord| coord.y).max().unwrap_or(start.y),
            start.z,
        );

        regions.push(Region {
            anchor: Coord::new(min.x, max.y, start.z),
            tiles,
            min,
            max,
        });
    }

    regions.sort_unstable_by_key(|region| (region.anchor.z, std::cmp::Reverse(region.anchor.y), region.anchor.x));
    regions
}

impl ConflictState {
    pub fn new(data: ConflictData) -> Self {
        let order = data.conflicts.iter().map(|conflict| conflict.coord).collect();
        let tiles = data
            .conflicts
            .into_iter()
            .map(|conflict| (conflict.coord, conflict))
            .collect();

        let mut state = Self {
            operation: data.operation,
            tiles,
            order,
            rows: Vec::new(),
            resolutions: HashMap::new(),
            revision: 0,
        };
        state.rows = state
            .regions_in(None, &Resolved::default())
            .into_iter()
            .enumerate()
            .flat_map(|(index, region)| {
                region.tiles.into_iter().map(move |coord| ConflictRow {
                    coord,
                    region: index + 1,
                })
            })
            .collect();

        state
    }

    /// Every conflict grouped by region, in reading order
    pub fn rows(&self) -> &[ConflictRow] { &self.rows }

    /// Changes whenever a resolution is recorded
    pub fn revision(&self) -> u64 { self.revision }

    pub fn side_label(&self, side: Side) -> &str {
        match side {
            Side::Ours => OURS_LABEL,
            Side::Theirs => self
                .operation
                .as_ref()
                .map_or("theirs", |operation| operation.theirs.short.as_str()),
        }
    }

    pub fn side_detail(&self, side: Side) -> String {
        match (side, self.operation.as_ref()) {
            (Side::Ours, Some(operation)) => format!(
                "Keep the current branch (HEAD)\n{} {}",
                operation.kind.label(),
                operation.theirs.describe()
            ),
            (Side::Ours, None) => String::from("Keep the current branch (HEAD)"),
            (Side::Theirs, Some(operation)) => format!("Take {}", operation.theirs.describe()),
            (Side::Theirs, None) => String::from("Take the incoming side"),
        }
    }

    pub fn len(&self) -> usize { self.order.len() }

    pub fn is_empty(&self) -> bool { self.order.is_empty() }

    pub fn conflict_at(&self, coord: Coord) -> Option<&TileConflict> { self.tiles.get(&coord) }

    pub fn coords(&self) -> &[Coord] { &self.order }

    /// The latest resolution that is still applied in the history
    pub fn resolution(&self, coord: Coord, history: &History) -> Option<Side> {
        self.resolutions
            .get(&coord)?
            .iter()
            .rev()
            .find(|(_, mark)| history.is_applied(*mark))
            .map(|(side, _)| *side)
    }

    /// Every tile's effective resolution, reading the history once
    pub fn resolved(&self, history: &History) -> Resolved {
        let applied = history.applied_groups();

        Resolved(
            self.resolutions
                .iter()
                .filter_map(|(coord, choices)| {
                    choices
                        .iter()
                        .rev()
                        .find(|(_, mark)| applied.contains(mark))
                        .map(|(side, _)| (*coord, *side))
                })
                .collect(),
        )
    }

    pub fn unresolved_in<'a>(&'a self, resolved: &'a Resolved) -> impl Iterator<Item = Coord> + 'a {
        self.order
            .iter()
            .copied()
            .filter(move |coord| !resolved.0.contains_key(coord))
    }

    pub fn unresolved_count(&self, history: &History) -> usize { self.unresolved_in(&self.resolved(history)).count() }

    pub fn side_tile(&self, coord: Coord, side: Side) -> Option<Option<&Tile>> {
        let conflict = self.tiles.get(&coord)?;

        Some(match side {
            Side::Ours => conflict.ours.as_ref(),
            Side::Theirs => conflict.theirs.as_ref(),
        })
    }

    /// Unresolved regions, 8 connected so diagonal neighbours share buttons
    pub fn regions(&self, z: Option<u32>, history: &History) -> Vec<Region> {
        self.regions_in(z, &self.resolved(history))
    }

    pub fn regions_in(&self, z: Option<u32>, resolved: &Resolved) -> Vec<Region> {
        connected_regions(
            self.unresolved_in(resolved)
                .filter(|coord| z.is_none_or(|z| coord.z == z)),
        )
    }

    /// Replaces the conflicting tiles among `coords` with one side. The edit is recorded even when
    /// nothing changes so the choice can be undone. Returns `None` when no coordinate conflicts.
    pub fn resolve_edit(&self, document: &mut MapDocument, coords: &[Coord], side: Side) -> Option<Edit> {
        let mut edit = Edit::new(format!("Take {}", self.side_label(side))).recorded_when_empty();
        let mut any = false;

        for coord in coords {
            let Some(target) = self.side_tile(*coord, side) else {
                continue;
            };
            any = true;

            set_tile(&mut edit, document, *coord, target.cloned().unwrap_or_default());
        }

        any.then_some(edit)
    }

    pub fn record(&mut self, coords: &[Coord], side: Side, mark: EditGroupId) {
        for coord in coords.iter().filter(|coord| self.tiles.contains_key(coord)) {
            self.resolutions.entry(*coord).or_default().push((side, mark));
        }
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn highlights(&self, z: u32, history: &History) -> Vec<Highlight> {
        self.highlights_in(z, &self.resolved(history))
    }

    pub fn highlights_in(&self, z: u32, resolved: &Resolved) -> Vec<Highlight> {
        let mut unresolved = HashSet::new();
        let mut ours = HashSet::new();
        let mut theirs = HashSet::new();

        for coord in self.order.iter().filter(|coord| coord.z == z) {
            let position = [coord.x as i32, coord.y as i32];
            match resolved.0.get(coord) {
                None => unresolved.insert(position),
                Some(Side::Ours) => ours.insert(position),
                Some(Side::Theirs) => theirs.insert(position),
            };
        }

        [
            (ours, OURS_COLOR, 0.1, false),
            (theirs, THEIRS_COLOR, 0.1, false),
            (unresolved, UNRESOLVED_COLOR, 0.2, true),
        ]
        .into_iter()
        .filter(|(tiles, ..)| !tiles.is_empty())
        .map(|(tiles, color, fill, outline)| Highlight {
            tiles: highlight_tiles(&tiles),
            z: z as i32,
            color,
            fill,
            outline,
            when: HIGHLIGHT_ALWAYS,
            label: None,
        })
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use core::path::TreePath;

    use dmm::{Coord, Map, Prefab, Size, merge::TileConflict};

    use super::{ConflictData, ConflictState, Side};
    use crate::{command::EditGroupId, document::MapDocument};

    fn tile(path: &str) -> Vec<Prefab> { vec![Prefab::new(TreePath::parse(path))] }

    fn setup(coords: &[(u32, u32)]) -> (MapDocument, ConflictState) {
        let mut map = Map::new(Size { x: 6, y: 6, z: 1 });
        let floor = map.intern_tile(tile("/turf/floor"));
        for row in &mut map.grid[0] {
            row.fill(floor);
        }

        let conflicts = coords
            .iter()
            .map(|(x, y)| TileConflict {
                coord: Coord::new(*x, *y, 1),
                base: Some(tile("/turf/floor")),
                ours: Some(tile("/turf/floor")),
                theirs: Some(tile("/turf/wall")),
            })
            .collect();

        (
            MapDocument::new(map, 1),
            ConflictState::new(ConflictData {
                operation: None,
                conflicts,
            }),
        )
    }

    fn resolve(document: &mut MapDocument, state: &mut ConflictState, coords: &[Coord], side: Side) {
        let edit = state.resolve_edit(document, coords, side).unwrap();
        let mark = EditGroupId::new();
        assert!(document.apply_grouped(edit, Some(mark)));
        state.record(coords, side, mark);
    }

    #[test]
    fn groups_touching_tiles_including_diagonals() {
        let (document, state) = setup(&[(1, 1), (2, 2), (5, 5), (5, 6)]);
        let regions = state.regions(Some(1), &document.history);

        assert_eq!(regions.len(), 2);
        assert_eq!(regions[0].tiles.len(), 2);
        assert_eq!(regions[0].anchor, Coord::new(5, 6, 1));
        assert_eq!(regions[1].anchor, Coord::new(1, 2, 1));
    }

    #[test]
    fn taking_theirs_replaces_tiles_and_undo_reopens_the_conflict() {
        let (mut document, mut state) = setup(&[(1, 1), (2, 1)]);
        let coords = [Coord::new(1, 1, 1), Coord::new(2, 1, 1)];

        resolve(&mut document, &mut state, &coords, Side::Theirs);

        assert_eq!(state.unresolved_count(&document.history), 0);
        assert_eq!(
            document.map.tile_at(coords[0]).map(|tile| tile[0].path.to_string()),
            Some("/turf/wall".to_owned())
        );

        assert!(document.undo());
        assert_eq!(state.unresolved_count(&document.history), 2);
        assert_eq!(
            document.map.tile_at(coords[0]).map(|tile| tile[0].path.to_string()),
            Some("/turf/floor".to_owned())
        );

        assert!(document.redo());
        assert_eq!(state.resolution(coords[1], &document.history), Some(Side::Theirs));
    }

    #[test]
    fn taking_head_changes_nothing_but_is_still_undoable() {
        let (mut document, mut state) = setup(&[(3, 3)]);
        let coord = Coord::new(3, 3, 1);
        let ids = document.instance_ids_at(coord).to_vec();

        resolve(&mut document, &mut state, &[coord], Side::Ours);

        assert_eq!(document.instance_ids_at(coord), ids.as_slice());
        assert_eq!(state.resolution(coord, &document.history), Some(Side::Ours));
        assert!(document.undo());
        assert_eq!(state.resolution(coord, &document.history), None);
    }

    #[test]
    fn a_later_choice_wins_until_it_is_undone() {
        let (mut document, mut state) = setup(&[(3, 3)]);
        let coord = Coord::new(3, 3, 1);

        resolve(&mut document, &mut state, &[coord], Side::Theirs);
        resolve(&mut document, &mut state, &[coord], Side::Ours);
        assert_eq!(state.resolution(coord, &document.history), Some(Side::Ours));

        assert!(document.undo());
        assert_eq!(state.resolution(coord, &document.history), Some(Side::Theirs));
    }

    #[test]
    fn highlights_split_by_state() {
        let (mut document, mut state) = setup(&[(1, 1), (4, 4)]);
        resolve(&mut document, &mut state, &[Coord::new(4, 4, 1)], Side::Theirs);

        let highlights = state.highlights(1, &document.history);

        assert_eq!(highlights.len(), 2);
        assert!(
            highlights
                .iter()
                .any(|highlight| highlight.outline && highlight.tiles.len() == 1)
        );
    }

    #[test]
    fn rows_keep_their_region_numbers_while_tiles_get_resolved() {
        let (mut document, mut state) = setup(&[(1, 1), (2, 2), (5, 5)]);
        let before = state.rows().to_vec();

        resolve(&mut document, &mut state, &[Coord::new(5, 5, 1)], Side::Ours);

        assert_eq!(state.rows(), before.as_slice());
        assert_eq!(
            before.iter().map(|row| (row.coord, row.region)).collect::<Vec<_>>(),
            [
                (Coord::new(5, 5, 1), 1),
                (Coord::new(2, 2, 1), 2),
                (Coord::new(1, 1, 1), 2),
            ]
        );
    }
}
