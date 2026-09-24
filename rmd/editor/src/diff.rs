use std::collections::{HashMap, HashSet};

use dmm::{
    Coord,
    Map,
    Tile,
    key::Key,
    merge::{prefabs_equal, tiles_equal},
};

use crate::{
    bake::{HIGHLIGHT_ALWAYS, Highlight, highlight_tiles},
    command::Edit,
    conflict::{connected_regions, set_tile},
    document::MapDocument,
};

pub const ADDED_COLOR: [f32; 3] = [0.3, 0.85, 0.4];
pub const REMOVED_COLOR: [f32; 3] = [1.0, 0.3, 0.3];
pub const MODIFIED_COLOR: [f32; 3] = [1.0, 0.75, 0.2];

/// One side of a comparison
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffSource {
    /// The map open in the editor, unsaved edits included
    Working,
    /// Anything `git rev-parse` understands: `HEAD~3`, a branch, a tag or a hash
    Revision(String),
}

impl DiffSource {
    pub fn head() -> Self { Self::Revision(String::from("HEAD")) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeKind {
    /// The newer tile keeps everything the older one had and adds to it
    Added,
    /// The newer tile lost objects and gained none
    Removed,
    Modified,
}

impl ChangeKind {
    pub const ALL: [Self; 3] = [Self::Added, Self::Removed, Self::Modified];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Added => "Added",
            Self::Removed => "Removed",
            Self::Modified => "Modified",
        }
    }

    pub const fn color(self) -> [f32; 3] {
        match self {
            Self::Added => ADDED_COLOR,
            Self::Removed => REMOVED_COLOR,
            Self::Modified => MODIFIED_COLOR,
        }
    }

    const fn index(self) -> usize {
        match self {
            Self::Added => 0,
            Self::Removed => 1,
            Self::Modified => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChangedTile {
    pub coord: Coord,
    pub kind: ChangeKind,
    /// 1 based, touching changes share one
    pub region: usize,
}

/// Every tile that differs between two versions of a map, grouped by region
#[derive(Debug, Clone, Default)]
pub struct MapDiff {
    changes: Vec<ChangedTile>,
    index: HashMap<Coord, usize>,
    counts: [usize; 3],
}

impl MapDiff {
    pub fn changes(&self) -> &[ChangedTile] { &self.changes }

    pub fn is_empty(&self) -> bool { self.changes.is_empty() }

    pub fn len(&self) -> usize { self.changes.len() }

    pub fn at(&self, coord: Coord) -> Option<ChangeKind> { Some(self.changes[*self.index.get(&coord)?].kind) }

    pub fn count(&self, kind: ChangeKind) -> usize { self.counts[kind.index()] }

    pub fn highlights(&self, z: u32) -> Vec<Highlight> {
        let mut covered: [HashSet<[i32; 2]>; 3] = Default::default();
        for change in self.changes.iter().filter(|change| change.coord.z == z) {
            covered[change.kind.index()].insert([change.coord.x as i32, change.coord.y as i32]);
        }

        ChangeKind::ALL
            .into_iter()
            .zip(covered)
            .filter(|(_, tiles)| !tiles.is_empty())
            .map(|(kind, tiles)| Highlight {
                tiles: highlight_tiles(&tiles),
                z: z as i32,
                color: kind.color(),
                fill: 0.3,
                outline: true,
                when: HIGHLIGHT_ALWAYS,
                label: None,
            })
            .collect()
    }
}

static EMPTY: Tile = Vec::new();

/// `None` outside the map
fn cell(map: Option<&Map>, coord: Coord) -> Option<(Key, &Tile)> {
    let map = map?;
    let key = map.key_at(coord)?;

    Some((key, map.dictionary.get(&key).unwrap_or(&EMPTY)))
}

/// Whether `whole` holds every prefab of `part`, duplicates counted
fn contains(whole: &Tile, part: &Tile) -> bool {
    let mut used = vec![false; whole.len()];

    part.iter().all(|prefab| {
        let found = whole
            .iter()
            .enumerate()
            .position(|(index, candidate)| !used[index] && prefabs_equal(candidate, prefab));
        if let Some(index) = found {
            used[index] = true;
        }

        found.is_some()
    })
}

fn classify(from: Option<&Tile>, to: Option<&Tile>) -> Option<ChangeKind> {
    match (from, to) {
        (None, None) => None,
        (None, Some(_)) => Some(ChangeKind::Added),
        (Some(_), None) => Some(ChangeKind::Removed),
        (Some(from), Some(to)) if tiles_equal(from, to) => None,
        (Some(from), Some(to)) if contains(to, from) => Some(ChangeKind::Added),
        (Some(from), Some(to)) if contains(from, to) => Some(ChangeKind::Removed),
        _ => Some(ChangeKind::Modified),
    }
}

/// Compares two versions of a map, `None` for a version where the file doesn't exist
pub fn diff(from: Option<&Map>, to: Option<&Map>) -> MapDiff {
    let size = |map: Option<&Map>| map.map_or(Default::default(), |map| map.size);
    let (from_size, to_size) = (size(from), size(to));
    let mut kinds = HashMap::new();
    // one comparison per pair of dictionary entries, not per tile
    let mut compared: HashMap<(Option<Key>, Option<Key>), Option<ChangeKind>> = HashMap::new();

    for z in 1..=from_size.z.max(to_size.z) {
        for y in (1..=from_size.y.max(to_size.y)).rev() {
            for x in 1..=from_size.x.max(to_size.x) {
                let coord = Coord::new(x, y, z);
                let (before, after) = (cell(from, coord), cell(to, coord));
                let kind = *compared
                    .entry((before.map(|(key, _)| key), after.map(|(key, _)| key)))
                    .or_insert_with(|| classify(before.map(|(_, tile)| tile), after.map(|(_, tile)| tile)));
                if let Some(kind) = kind {
                    kinds.insert(coord, kind);
                }
            }
        }
    }

    let mut result = MapDiff::default();
    let order = {
        let mut order = kinds.keys().copied().collect::<Vec<_>>();
        order.sort_unstable_by_key(|coord| (coord.z, std::cmp::Reverse(coord.y), coord.x));
        order
    };
    for (region, tiles) in connected_regions(order).into_iter().enumerate() {
        for coord in tiles.tiles {
            let kind = kinds[&coord];
            result.index.insert(coord, result.changes.len());
            result.counts[kind.index()] += 1;
            result.changes.push(ChangedTile {
                coord,
                kind,
                region: region + 1,
            });
        }
    }

    result
}

/// Puts the `from` tiles back at `coords`. Tiles past the edge of the current map are skipped,
/// ones past the edge of `from` are cleared. `None` when nothing changes.
pub fn restore_edit(document: &mut MapDocument, from: Option<&Map>, coords: &[Coord], label: &str) -> Option<Edit> {
    let mut edit = Edit::new(label);

    for coord in coords {
        if document.map.key_at(*coord).is_none() {
            continue;
        }

        let target = from.and_then(|map| map.tile_at(*coord)).cloned().unwrap_or_default();
        set_tile(&mut edit, document, *coord, target);
    }

    (!edit.is_empty()).then_some(edit)
}

#[cfg(test)]
mod tests {
    use core::path::TreePath;

    use dmm::{Coord, Map, Prefab, Size, Tile};

    use super::{ChangeKind, diff, restore_edit};
    use crate::document::MapDocument;

    fn tile(paths: &[&str]) -> Tile { paths.iter().map(|path| Prefab::new(TreePath::parse(path))).collect() }

    fn map(size: (u32, u32), edits: &[((u32, u32), &[&str])]) -> Map {
        let mut map = Map::new(Size {
            x: size.0,
            y: size.1,
            z: 1,
        });
        let floor = map.intern_tile(tile(&["/turf/floor"]));
        for row in &mut map.grid[0] {
            row.fill(floor);
        }

        for ((x, y), paths) in edits {
            let key = map.intern_tile(tile(paths));
            let row = (map.size.y - y) as usize;
            map.grid[0][row][*x as usize - 1] = key;
        }

        map
    }

    fn kinds(from: Option<&Map>, to: Option<&Map>) -> Vec<(u32, u32, ChangeKind)> {
        diff(from, to)
            .changes()
            .iter()
            .map(|change| (change.coord.x, change.coord.y, change.kind))
            .collect()
    }

    #[test]
    fn identical_maps_have_no_changes() {
        let old = map((4, 4), &[((2, 2), &["/turf/floor", "/obj/table"])]);

        assert!(diff(Some(&old), Some(&old.clone())).is_empty());
    }

    #[test]
    fn classifies_added_removed_and_swapped_objects() {
        let old = map((4, 4), &[((3, 3), &["/turf/floor", "/obj/table"])]);
        let new = map(
            (4, 4),
            &[
                ((1, 1), &["/turf/floor", "/obj/chair"]),
                ((3, 3), &["/turf/floor"]),
                ((4, 4), &["/turf/wall"]),
            ],
        );
        let result = diff(Some(&old), Some(&new));

        assert_eq!(result.at(Coord::new(1, 1, 1)), Some(ChangeKind::Added));
        assert_eq!(result.at(Coord::new(3, 3, 1)), Some(ChangeKind::Removed));
        assert_eq!(result.at(Coord::new(4, 4, 1)), Some(ChangeKind::Modified));
        assert_eq!(result.at(Coord::new(2, 2, 1)), None);
        assert_eq!(result.len(), 3);
        assert_eq!(result.count(ChangeKind::Added), 1);
    }

    #[test]
    fn a_resized_map_adds_or_removes_its_edges() {
        let small = map((2, 1), &[]);
        let large = map((3, 1), &[]);

        assert_eq!(kinds(Some(&small), Some(&large)), [(3, 1, ChangeKind::Added)]);
        assert_eq!(kinds(Some(&large), Some(&small)), [(3, 1, ChangeKind::Removed)]);
        assert_eq!(kinds(None, Some(&small)).len(), 2);
    }

    #[test]
    fn touching_changes_share_a_region() {
        let old = map((5, 5), &[]);
        let new = map(
            (5, 5),
            &[
                ((1, 1), &["/turf/wall"]),
                ((2, 2), &["/turf/wall"]),
                ((5, 5), &["/turf/wall"]),
            ],
        );
        let regions = diff(Some(&old), Some(&new))
            .changes()
            .iter()
            .map(|change| (change.coord.x, change.region))
            .collect::<Vec<_>>();

        assert_eq!(regions, [(5, 1), (2, 2), (1, 2)]);
    }

    #[test]
    fn restoring_puts_the_old_tile_back_until_undone() {
        let old = map((3, 3), &[((2, 2), &["/turf/floor", "/obj/table"])]);
        let mut document = MapDocument::new(map((3, 3), &[]), 1);
        let coord = Coord::new(2, 2, 1);
        let edit = restore_edit(&mut document, Some(&old), &[coord, Coord::new(9, 9, 1)], "Restore").unwrap();

        assert!(document.apply_grouped(edit, None));
        assert!(diff(Some(&old), Some(&document.map)).is_empty());
        assert!(restore_edit(&mut document, Some(&old), &[coord], "Restore").is_none());

        assert!(document.undo());
        assert_eq!(kinds(Some(&old), Some(&document.map)), [(2, 2, ChangeKind::Removed)]);
    }
}
