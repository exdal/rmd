pub(crate) mod session;

use std::collections::{HashMap, HashSet};

use base::coord::Coord;
use dmm::{Map, Size, Tile, key::Key, tiles_equal};
use prefab::{Prefab, prefabs_equal};

use crate::{
    bake::{HIGHLIGHT_ALWAYS, Highlight, highlight_tiles},
    document::{
        MapDocument,
        command::Edit,
        grid::{Grid, SharedTile, TileView},
    },
    git::conflict::{connected_hunks, describe_prefab, set_tile},
};

pub const ADDED_COLOR: [f32; 3] = [0.3, 0.85, 0.4];
pub const REMOVED_COLOR: [f32; 3] = [1.0, 0.3, 0.3];
pub const MODIFIED_COLOR: [f32; 3] = [1.0, 0.75, 0.2];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffSource {
    Working,
    Revision(String),
}

impl DiffSource {
    pub fn head() -> Self {
        Self::Revision(String::from("HEAD"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeKind {
    Added,
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
    pub hunk: usize,
}

/// Every tile that differs between two versions of a map, grouped by hunk
#[derive(Debug, Clone, Default)]
pub struct MapDiff {
    changes: Vec<ChangedTile>,
    index: HashMap<Coord, usize>,
    counts: [usize; 3],
}

impl MapDiff {
    pub fn changes(&self) -> &[ChangedTile] {
        &self.changes
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.changes.len()
    }

    pub fn at(&self, coord: Coord) -> Option<ChangeKind> {
        Some(self.changes[*self.index.get(&coord)?].kind)
    }

    pub fn count(&self, kind: ChangeKind) -> usize {
        self.counts[kind.index()]
    }

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

#[derive(Debug, Clone, Copy)]
pub enum Version<'a> {
    File(&'a Map),
    Working(&'a Grid),
}

/// Tiles with the same identity hold the same prefabs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Identity {
    Key(Key),
    Shared(SharedTile),
}

impl<'a> Version<'a> {
    pub fn size(self) -> Size {
        match self {
            Self::File(map) => map.size,
            Self::Working(grid) => grid.size(),
        }
    }

    pub fn tile_at(self, coord: Coord) -> Option<TileView<'a>> {
        match self {
            Self::File(map) => map.tile_at(coord).map(TileView::from),
            Self::Working(grid) => grid.tile_at(coord),
        }
    }

    /// `None` outside the map
    fn cell(self, coord: Coord) -> Option<(Identity, TileView<'a>)> {
        match self {
            Self::File(map) => {
                let key = map.key_at(coord)?;

                Some((
                    Identity::Key(key),
                    TileView::from(map.dictionary.get(&key).unwrap_or(&EMPTY)),
                ))
            },
            Self::Working(grid) => {
                let (shared, tile) = grid.shared_tile_at(coord)?;

                Some((Identity::Shared(shared), tile))
            },
        }
    }
}

/// Whether `whole` holds every prefab of `part`, duplicates counted
fn contains(whole: TileView<'_>, part: TileView<'_>) -> bool {
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

fn classify(from: Option<TileView<'_>>, to: Option<TileView<'_>>) -> Option<ChangeKind> {
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
pub fn diff(from: Option<Version>, to: Option<Version>) -> MapDiff {
    let size = |version: Option<Version>| version.map_or(Default::default(), Version::size);
    let (from_size, to_size) = (size(from), size(to));
    let mut kinds = HashMap::new();
    // one comparison per pair of distinct tiles, not per coordinate
    let mut compared: HashMap<(Option<Identity>, Option<Identity>), Option<ChangeKind>> = HashMap::new();

    for z in 1..=from_size.z.max(to_size.z) {
        for y in (1..=from_size.y.max(to_size.y)).rev() {
            for x in 1..=from_size.x.max(to_size.x) {
                let coord = Coord::new(x, y, z);
                let (before, after) = (from.and_then(|from| from.cell(coord)), to.and_then(|to| to.cell(coord)));
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
    for (hunk, tiles) in connected_hunks(order).into_iter().enumerate() {
        for coord in tiles.tiles {
            let kind = kinds[&coord];
            result.index.insert(coord, result.changes.len());
            result.counts[kind.index()] += 1;
            result.changes.push(ChangedTile {
                coord,
                kind,
                hunk: hunk + 1,
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
pub fn tile_lines(from: Option<TileView<'_>>, to: Option<TileView<'_>>) -> Vec<TileLine> {
    let from = from.unwrap_or_default();
    let to = to.unwrap_or_default();
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
        if document.map.tile_at(*coord).is_none() {
            continue;
        }

        let target = from.and_then(|map| map.tile_at(*coord)).cloned().unwrap_or_default();
        set_tile(&mut edit, document, *coord, target);
    }

    (!edit.is_empty()).then_some(edit)
}

#[cfg(test)]
mod tests;
