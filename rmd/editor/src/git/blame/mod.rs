pub(crate) mod session;

use std::collections::HashMap;

use dmm::{Coord, Map, Size, Tile, key::Key, tiles_equal, writer::format_value};

use crate::git::repo::{CommitInfo, FileVersion, GitError, Repo, RepoPath, parse_map};

pub const UNCOMMITTED: u32 = u32::MAX;
/// The change is older than the walked, parseable history
pub const BOUNDARY: u32 = u32::MAX - 1;
const PENDING: u32 = u32::MAX - 2;
const ABSENT: u32 = u32::MAX;

pub trait VersionSource {
    /// Newest first
    fn commits(&self) -> &[CommitInfo];

    /// Whether older commits exist beyond the last one listed
    fn truncated(&self) -> bool;

    /// The map as of commit `index`; an unparseable map stops the history walk.
    fn map_at(&mut self, index: usize) -> Result<MapVersion, GitError>;
}

pub enum MapVersion {
    Present(Map),
    Missing,
    Unparseable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlameCell<'a> {
    Commit(u32, &'a CommitInfo),
    Uncommitted,
    Boundary,
}

#[derive(Debug, Clone, Default)]
pub struct BlameResult {
    pub size: Size,
    pub commits: Vec<CommitInfo>,
    /// Index of the first unparseable revision reached while walking backwards.
    pub parse_boundary: Option<usize>,
    /// The number of revisions available for blame, capped by the slider and parse boundary.
    pub history_limit: usize,
    /// Revisions from HEAD through the oldest one that still owns a tile, what the heatmap palette spans
    pub heat_span: usize,
    cells: Vec<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlameCounts {
    // indexed like `BlameResult::commits`
    pub commits: Vec<usize>,
    pub uncommitted: usize,
    pub boundary: usize,
}

fn cell_index(size: Size, coord: Coord) -> Option<usize> {
    if coord.x == 0 || coord.y == 0 || coord.z == 0 || coord.x > size.x || coord.y > size.y || coord.z > size.z {
        return None;
    }

    let (x, y, z) = (coord.x as usize - 1, coord.y as usize - 1, coord.z as usize - 1);

    Some((z * size.y as usize + y) * size.x as usize + x)
}

impl BlameResult {
    pub fn at(&self, coord: Coord) -> Option<BlameCell<'_>> {
        match *self.cells.get(cell_index(self.size, coord)?)? {
            UNCOMMITTED => Some(BlameCell::Uncommitted),
            BOUNDARY => Some(BlameCell::Boundary),
            index => self
                .commits
                .get(index as usize)
                .map(|commit| BlameCell::Commit(index, commit)),
        }
    }

    pub fn level(&self, z: u32) -> impl Iterator<Item = (Coord, u32)> + '_ {
        let size = self.size;
        (1..=size.y).flat_map(move |y| {
            (1..=size.x).filter_map(move |x| {
                let coord = Coord::new(x, y, z);
                Some((coord, *self.cells.get(cell_index(size, coord)?)?))
            })
        })
    }

    pub fn counts(&self) -> BlameCounts {
        let mut counts = BlameCounts {
            commits: vec![0; self.commits.len()],
            ..BlameCounts::default()
        };

        for cell in &self.cells {
            match *cell {
                UNCOMMITTED => counts.uncommitted += 1,
                BOUNDARY => counts.boundary += 1,
                index => {
                    if let Some(count) = counts.commits.get_mut(index as usize) {
                        *count += 1;
                    }
                },
            }
        }

        counts
    }

    /// A tile owned by `cell`, on level `prefer_z` when it has one there
    pub fn first_tile(&self, cell: u32, prefer_z: u32) -> Option<Coord> {
        let find = |z| self.level(z).find(|(_, owner)| *owner == cell).map(|(coord, _)| coord);

        find(prefer_z).or_else(|| (1..=self.size.z).filter(|z| *z != prefer_z).find_map(find))
    }

    pub fn boundary_label(&self) -> String {
        self.parse_boundary
            .and_then(|index| self.commits.get(index))
            .map_or_else(
                || String::from("Older than blame depth"),
                |commit| format!("History stops at {}: map could not be parsed", commit.short),
            )
    }
}

#[derive(Default)]
struct Interner {
    buckets: HashMap<String, Vec<(Tile, u32)>>,
    next: u32,
}

impl Interner {
    fn id(&mut self, tile: &Tile) -> u32 {
        let mut text = String::new();
        for prefab in tile {
            text.push_str(&prefab.path.to_string());
            text.push('{');
            for (name, var) in &prefab.vars {
                text.push_str(name.as_str());
                text.push('=');
                text.push_str(&format_value(&var.value));
                text.push(';');
            }
            text.push('}');
        }

        let bucket = self.buckets.entry(text).or_default();
        if let Some((_, id)) = bucket.iter().find(|(candidate, _)| tiles_equal(candidate, tile)) {
            return *id;
        }

        let id = self.next;
        self.next = self.next.saturating_add(1);
        bucket.push((tile.clone(), id));

        id
    }

    fn grid(&mut self, map: Option<&Map>, size: Size) -> Vec<u32> {
        let mut ids = vec![ABSENT; cell_index(size, Coord::new(size.x, size.y, size.z)).map_or(0, |last| last + 1)];
        let Some(map) = map else {
            return ids;
        };

        let keys: HashMap<Key, u32> = map.dictionary.iter().map(|(key, tile)| (*key, self.id(tile))).collect();
        let empty = self.id(&Vec::new());
        for z in 1..=size.z.min(map.size.z) {
            for y in 1..=size.y.min(map.size.y) {
                for x in 1..=size.x.min(map.size.x) {
                    let coord = Coord::new(x, y, z);
                    if let (Some(index), Some(key)) = (cell_index(size, coord), map.key_at(coord)) {
                        ids[index] = keys.get(&key).copied().unwrap_or(empty);
                    }
                }
            }
        }

        ids
    }
}

pub fn blame(
    current: &Map, source: &mut impl VersionSource, cancelled: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(usize, usize),
) -> Result<BlameResult, GitError> {
    let size = current.size;
    let commits = source.commits().to_vec();
    let truncated = source.truncated();
    let listed_limit = commits.len().saturating_sub(usize::from(truncated));
    let mut interner = Interner::default();
    let now = interner.grid(Some(current), size);
    let mut cells = vec![PENDING; now.len()];

    progress(0, commits.len());

    if cancelled() {
        return Err(GitError(String::from("cancelled")));
    }

    let head = if commits.is_empty() {
        MapVersion::Missing
    } else {
        source.map_at(0)?
    };

    let mut parse_boundary = None;
    let mut newer = match head {
        MapVersion::Present(map) => interner.grid(Some(&map), size),
        MapVersion::Missing => interner.grid(None, size),
        MapVersion::Unparseable => {
            parse_boundary = Some(0);
            Vec::new()
        },
    };

    if parse_boundary.is_some() {
        cells.fill(BOUNDARY);
        return Ok(BlameResult {
            size,
            commits,
            parse_boundary,
            history_limit: 0,
            heat_span: 0,
            cells,
        });
    }

    let mut pending = 0;

    for ((cell, now), head) in cells.iter_mut().zip(&now).zip(&newer) {
        if now != head {
            *cell = UNCOMMITTED;
        } else {
            pending += 1;
        }
    }

    for index in 0..commits.len() {
        if pending == 0 {
            break;
        }

        if cancelled() {
            return Err(GitError(String::from("cancelled")));
        }

        let older = if index + 1 < commits.len() {
            match source.map_at(index + 1)? {
                MapVersion::Present(map) => interner.grid(Some(&map), size),
                MapVersion::Missing => interner.grid(None, size),
                MapVersion::Unparseable => {
                    parse_boundary = Some(index + 1);
                    break;
                },
            }
        } else if truncated {
            break;
        } else {
            // the commit that added the file
            vec![ABSENT; newer.len()]
        };

        for ((cell, newer), older) in cells.iter_mut().zip(&newer).zip(&older) {
            if *cell == PENDING && newer != older {
                *cell = index as u32;
                pending -= 1;
            }
        }

        newer = older;
        progress(index + 1, commits.len());
    }

    for cell in &mut cells {
        if *cell == PENDING {
            *cell = BOUNDARY;
        }
    }

    let history_limit = parse_boundary.map_or(listed_limit, |boundary| boundary.min(listed_limit));
    let heat_span = cells
        .iter()
        .filter(|cell| (**cell as usize) < history_limit)
        .max()
        .map_or(0, |oldest| *oldest as usize + 1);

    Ok(BlameResult {
        size,
        commits,
        parse_boundary,
        history_limit,
        heat_span,
        cells,
    })
}

pub struct GitVersions {
    repo: Repo,
    versions: Vec<FileVersion>,
    commits: Vec<CommitInfo>,
    truncated: bool,
}

impl GitVersions {
    #[cfg(test)]
    pub fn open(path: RepoPath, depth: usize) -> Result<Self, GitError> {
        Self::open_with_cancel(path, depth, &|| false)
    }

    pub fn open_with_cancel(path: RepoPath, depth: usize, cancelled: &dyn Fn() -> bool) -> Result<Self, GitError> {
        let repo = path.open()?;
        let versions = repo.file_history(depth.saturating_add(2), cancelled)?;

        if cancelled() {
            return Err(GitError(String::from("cancelled")));
        }

        let (versions, truncated) = history_window(versions, depth);
        let commits = versions.iter().map(|version| version.commit.clone()).collect();

        Ok(Self {
            repo,
            versions,
            commits,
            truncated,
        })
    }
}

fn history_window(mut versions: Vec<FileVersion>, depth: usize) -> (Vec<FileVersion>, bool) {
    let kept = depth.saturating_add(1);
    let truncated = versions.len() > kept;
    versions.truncate(kept);

    (versions, truncated)
}

impl VersionSource for GitVersions {
    fn commits(&self) -> &[CommitInfo] {
        &self.commits
    }

    fn truncated(&self) -> bool {
        self.truncated
    }

    fn map_at(&mut self, index: usize) -> Result<MapVersion, GitError> {
        let Some(version) = self.versions.get(index) else {
            return Ok(MapVersion::Missing);
        };
        let Some(blob) = version.blob else {
            return Ok(MapVersion::Missing);
        };
        Ok(parse_map(self.repo.blob(blob)?).map_or(MapVersion::Unparseable, MapVersion::Present))
    }
}

const DAY: i64 = 24 * 60 * 60;

pub fn pending_note(cell: BlameCell<'_>, edited_since_blame: bool) -> Option<&'static str> {
    if edited_since_blame {
        Some("Edited since blame ran")
    } else if cell == BlameCell::Uncommitted {
        Some("Not committed yet")
    } else {
        None
    }
}

pub fn relative_time(now: i64, time: i64) -> String {
    let age = now.saturating_sub(time).max(0);
    let (amount, unit) = match age {
        0..60 => return String::from("just now"),
        60..3600 => (age / 60, "minute"),
        3600..86_400 => (age / 3600, "hour"),
        86_400..2_678_400 => (age / DAY, "day"),
        2_678_400..31_536_000 => (age / (30 * DAY), "month"),
        _ => (age / (365 * DAY), "year"),
    };

    format!("{amount} {unit}{} ago", if amount == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests;
