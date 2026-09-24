use std::collections::{HashMap, HashSet};

use dmm::{
    Coord,
    Map,
    Prefab,
    Tile,
    key::Key,
    merge::{prefabs_equal, tiles_equal},
};

use crate::{
    bake::{HIGHLIGHT_ALWAYS, Highlight, highlight_tiles},
    command::Edit,
    conflict::{connected_regions, describe_prefab, set_tile},
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

/// How one object changed between two versions of a tile
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Kept,
    Added,
    Removed,
    Modified,
}

impl LineKind {
    pub const fn color(self) -> Option<[f32; 3]> {
        match self {
            Self::Kept => None,
            Self::Added => Some(ADDED_COLOR),
            Self::Removed => Some(REMOVED_COLOR),
            Self::Modified => Some(MODIFIED_COLOR),
        }
    }
}

/// A var of a modified object, `None` on the side that doesn't set it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VarChange {
    pub name: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileLine {
    pub kind: LineKind,
    pub text: String,
    pub vars: Vec<VarChange>,
}

impl TileLine {
    fn new(kind: LineKind, prefab: &Prefab) -> Self {
        Self {
            kind,
            text: describe_prefab(prefab),
            vars: Vec::new(),
        }
    }
}

fn var_changes(before: &Prefab, after: &Prefab) -> Vec<VarChange> {
    let changed = before
        .vars
        .iter()
        .filter(|(name, old)| after.var(name) != Some(&old.value))
        .map(|(name, old)| VarChange {
            name: name.to_string(),
            before: Some(old.value.to_string()),
            after: after.var(name).map(ToString::to_string),
        });
    let set = after
        .vars
        .iter()
        .filter(|(name, _)| before.var(name).is_none())
        .map(|(name, new)| VarChange {
            name: name.to_string(),
            before: None,
            after: Some(new.value.to_string()),
        });

    changed.chain(set).collect()
}

/// Ends a run of changes: removals first, then additions, pairing objects that kept their path
fn flush(lines: &mut Vec<TileLine>, removed: &mut Vec<&Prefab>, added: &mut Vec<&Prefab>) {
    let mut used = vec![false; removed.len()];
    let pairs = added
        .iter()
        .map(|prefab| {
            let pair = (0..removed.len()).find(|index| !used[*index] && removed[*index].path == prefab.path);
            if let Some(index) = pair {
                used[index] = true;
            }

            pair
        })
        .collect::<Vec<_>>();

    lines.extend(
        removed
            .iter()
            .zip(&used)
            .filter(|(_, used)| !**used)
            .map(|(prefab, _)| TileLine::new(LineKind::Removed, prefab)),
    );
    lines.extend(added.iter().zip(pairs).map(|(prefab, pair)| match pair {
        Some(index) => TileLine {
            kind: LineKind::Modified,
            text: prefab.path.to_string(),
            vars: var_changes(removed[index], prefab),
        },
        None => TileLine::new(LineKind::Added, prefab),
    }));
    removed.clear();
    added.clear();
}

/// One line per object, like a unified diff of the two object stacks
pub fn tile_lines(from: Option<&Tile>, to: Option<&Tile>) -> Vec<TileLine> {
    let from = from.map_or(&[][..], Vec::as_slice);
    let to = to.map_or(&[][..], Vec::as_slice);
    // shared[i][j] is how many objects `from[i..]` and `to[j..]` have in common, in order
    let mut shared = vec![vec![0usize; to.len() + 1]; from.len() + 1];
    for i in (0..from.len()).rev() {
        for j in (0..to.len()).rev() {
            shared[i][j] = if prefabs_equal(&from[i], &to[j]) {
                shared[i + 1][j + 1] + 1
            } else {
                shared[i + 1][j].max(shared[i][j + 1])
            };
        }
    }

    let mut lines = Vec::new();
    let (mut removed, mut added) = (Vec::new(), Vec::new());
    let (mut i, mut j) = (0, 0);
    while i < from.len() || j < to.len() {
        if i < from.len() && j < to.len() && prefabs_equal(&from[i], &to[j]) {
            flush(&mut lines, &mut removed, &mut added);
            lines.push(TileLine::new(LineKind::Kept, &from[i]));
            i += 1;
            j += 1;
        } else if i < from.len() && (j == to.len() || shared[i + 1][j] >= shared[i][j + 1]) {
            removed.push(&from[i]);
            i += 1;
        } else {
            added.push(&to[j]);
            j += 1;
        }
    }
    flush(&mut lines, &mut removed, &mut added);

    lines
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
    use core::{path::TreePath, types::Value};

    use dmm::{Coord, Map, Prefab, Size, Tile};

    use super::{ChangeKind, LineKind, VarChange, diff, restore_edit, tile_lines};
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

    fn lines(from: Option<&Tile>, to: Option<&Tile>) -> Vec<(LineKind, String)> {
        tile_lines(from, to)
            .into_iter()
            .map(|line| (line.kind, line.text))
            .collect()
    }

    fn door(dir: f32) -> Prefab {
        let mut door = Prefab::new(TreePath::parse("/obj/door"));
        door.set_var("name".into(), Value::Text(String::from("door")));
        door.set_var("dir".into(), Value::Num(dir));

        door
    }

    #[test]
    fn tile_lines_marks_kept_added_and_removed_objects() {
        let old = tile(&["/turf/floor", "/obj/table"]);
        let new = tile(&["/turf/floor", "/obj/chair"]);

        assert_eq!(
            lines(Some(&old), Some(&new)),
            [
                (LineKind::Kept, String::from("/turf/floor")),
                (LineKind::Removed, String::from("/obj/table")),
                (LineKind::Added, String::from("/obj/chair")),
            ]
        );
    }

    #[test]
    fn identical_tiles_are_all_kept() {
        let old = tile(&["/turf/floor", "/obj/table"]);

        assert!(
            tile_lines(Some(&old), Some(&old))
                .iter()
                .all(|line| line.kind == LineKind::Kept)
        );
    }

    #[test]
    fn same_path_with_changed_vars_is_one_modified_line() {
        let mut old = tile(&["/turf/floor"]);
        old.push(door(2.0));
        let mut new = tile(&["/turf/floor"]);
        new.push(door(4.0));

        let result = tile_lines(Some(&old), Some(&new));

        assert_eq!(result.len(), 2);
        assert_eq!(result[1].kind, LineKind::Modified);
        assert_eq!(result[1].text, "/obj/door");
        assert_eq!(
            result[1].vars,
            [VarChange {
                name: String::from("dir"),
                before: Some(String::from("2")),
                after: Some(String::from("4")),
            }]
        );
    }

    #[test]
    fn a_missing_side_is_all_added_or_all_removed() {
        let old = tile(&["/turf/floor", "/obj/table"]);

        assert!(
            tile_lines(None, Some(&old))
                .iter()
                .all(|line| line.kind == LineKind::Added)
        );
        assert!(
            tile_lines(Some(&old), None)
                .iter()
                .all(|line| line.kind == LineKind::Removed)
        );
        assert_eq!(tile_lines(Some(&old), None).len(), 2);
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
