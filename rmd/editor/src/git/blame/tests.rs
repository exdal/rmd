use base::coord::Coord;
use dmm::parser::parse;

use super::{
    BlameCell, BlameCounts, MapVersion, UNCOMMITTED, VersionSource, blame, history_window, pending_note, relative_time,
};
use crate::git::{
    fixtures::{commit, git_available, temp_repo},
    repo::{CommitInfo, FileVersion, GitError, discover},
};

fn file_versions(count: usize) -> Vec<FileVersion> {
    (0..count)
        .map(|index| FileVersion {
            commit: CommitInfo {
                hash: format!("c{index}"),
                short: format!("c{index}"),
                author: String::new(),
                time: 0,
                summary: String::new(),
                pull_request: None,
            },
            blob: None,
        })
        .collect()
}

#[test]
fn a_history_that_ends_at_the_depth_is_not_truncated() {
    // Two blamed commits plus the commit that added the file
    let (versions, truncated) = history_window(file_versions(3), 2);
    assert_eq!(versions.len(), 3);
    assert!(!truncated, "the oldest version added the file and gets blamed");

    let (versions, truncated) = history_window(file_versions(4), 2);
    assert_eq!(versions.len(), 3, "the third version is kept as the baseline");
    assert!(truncated);

    let (versions, truncated) = history_window(file_versions(1), 2);
    assert_eq!(versions.len(), 1);
    assert!(!truncated);
}

#[test]
fn pending_notes_tell_edits_apart_from_uncommitted_tiles() {
    assert_eq!(pending_note(BlameCell::Uncommitted, false), Some("Not committed yet"));
    assert_eq!(
        pending_note(BlameCell::Uncommitted, true),
        Some("Edited since blame ran")
    );
    assert_eq!(pending_note(BlameCell::Boundary, true), Some("Edited since blame ran"));
    assert_eq!(pending_note(BlameCell::Boundary, false), None);
}

struct Versions {
    commits: Vec<CommitInfo>,
    maps: Vec<Option<&'static str>>,
    truncated: bool,
    reads: usize,
}

impl Versions {
    fn new(maps: &[Option<&'static str>], truncated: bool) -> Self {
        Self {
            commits: (0..maps.len())
                .map(|index| CommitInfo {
                    hash: format!("c{index}"),
                    short: format!("c{index}"),
                    author: String::from("someone"),
                    time: 1000 - index as i64,
                    summary: String::new(),
                    pull_request: None,
                })
                .collect(),
            maps: maps.to_vec(),
            truncated,
            reads: 0,
        }
    }
}

impl VersionSource for Versions {
    fn commits(&self) -> &[CommitInfo] {
        &self.commits
    }

    fn truncated(&self) -> bool {
        self.truncated
    }

    fn map_at(&mut self, index: usize) -> Result<MapVersion, GitError> {
        self.reads += 1;
        Ok(match self.maps[index] {
            Some(source) => {
                let (map, errors) = parse(source);
                if errors.is_empty() {
                    MapVersion::Present(map)
                } else {
                    MapVersion::Unparseable
                }
            },
            None => MapVersion::Missing,
        })
    }
}

fn grid(rows: &str) -> String {
    format!("\"a\" = (/turf/floor)\n\"b\" = (/turf/wall)\n\n(1,1,1) = {{\"\n{rows}\n\"}}\n")
}

fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

fn commit_of(result: &super::BlameResult, x: u32, y: u32) -> Option<String> {
    match result.at(Coord::new(x, y, 1))? {
        BlameCell::Commit(_, commit) => Some(commit.short.clone()),
        BlameCell::Uncommitted => Some(String::from("uncommitted")),
        BlameCell::Boundary => Some(String::from("boundary")),
    }
}

#[test]
fn blames_the_newest_commit_that_changed_each_tile() {
    // c0 is HEAD, c2 added the map
    let mut versions = Versions::new(
        &[
            Some(leak(grid("bab"))),
            Some(leak(grid("baa"))),
            Some(leak(grid("aaa"))),
        ],
        false,
    );
    let current = parse(&grid("bbb")).0;

    let result = blame(&current, &mut versions, &|| false, &mut |_, _| {}).unwrap();

    assert_eq!(commit_of(&result, 1, 1).as_deref(), Some("c1"));
    assert_eq!(commit_of(&result, 2, 1).as_deref(), Some("uncommitted"));
    assert_eq!(commit_of(&result, 3, 1).as_deref(), Some("c0"));

    assert_eq!(
        result.counts(),
        BlameCounts {
            commits: vec![1, 1, 0],
            uncommitted: 1,
            boundary: 0,
        }
    );
    assert_eq!(result.first_tile(1, 1), Some(Coord::new(1, 1, 1)));
    assert_eq!(result.first_tile(UNCOMMITTED, 2), Some(Coord::new(2, 1, 1)));
    assert_eq!(result.first_tile(2, 1), None);
}

#[test]
fn untouched_tiles_belong_to_the_commit_that_added_the_map() {
    let mut versions = Versions::new(&[Some(leak(grid("ab"))), Some(leak(grid("aa")))], false);
    let current = parse(&grid("ab")).0;

    let result = blame(&current, &mut versions, &|| false, &mut |_, _| {}).unwrap();

    assert_eq!(commit_of(&result, 1, 1).as_deref(), Some("c1"));
    assert_eq!(commit_of(&result, 2, 1).as_deref(), Some("c0"));
    assert_eq!(result.history_limit, 2);
}

#[test]
fn a_truncated_history_leaves_old_tiles_at_the_boundary() {
    let mut versions = Versions::new(&[Some(leak(grid("ab"))), Some(leak(grid("aa")))], true);
    let current = parse(&grid("ab")).0;

    let result = blame(&current, &mut versions, &|| false, &mut |_, _| {}).unwrap();

    assert_eq!(commit_of(&result, 1, 1).as_deref(), Some("boundary"));
    assert_eq!(commit_of(&result, 2, 1).as_deref(), Some("c0"));
    assert_eq!(result.boundary_label(), "Older than blame depth");
    assert_eq!(result.history_limit, 1);
    assert_eq!(result.heat_span, 1);
}

#[test]
fn the_heat_span_ends_at_the_oldest_commit_that_owns_a_tile() {
    let mut versions = Versions::new(
        &[
            Some(leak(grid("ab"))),
            Some(leak(grid("aa"))),
            Some(leak(grid("aa"))),
            Some(leak(grid("aa"))),
        ],
        false,
    );
    let current = parse(&grid("ab")).0;

    let result = blame(&current, &mut versions, &|| false, &mut |_, _| {}).unwrap();

    assert_eq!(result.history_limit, 4);
    assert_eq!(result.heat_span, 4, "the tile untouched since the map was added");

    let mut versions = Versions::new(&[Some(leak(grid("ab"))), Some(leak(grid("aa")))], true);
    let result = blame(&current, &mut versions, &|| false, &mut |_, _| {}).unwrap();

    assert_eq!(result.heat_span, 1, "boundary tiles don't stretch the palette");
}

#[test]
fn a_parse_failure_limits_history_but_preserves_newer_blame() {
    let mut versions = Versions::new(
        &[
            Some(leak(grid("ab"))),
            Some(leak(grid("aa"))),
            Some("not a map"),
            Some(leak(grid("bb"))),
        ],
        false,
    );
    let current = parse(&grid("ab")).0;

    let result = blame(&current, &mut versions, &|| false, &mut |_, _| {}).unwrap();

    assert_eq!(commit_of(&result, 1, 1).as_deref(), Some("boundary"));
    assert_eq!(commit_of(&result, 2, 1).as_deref(), Some("c0"));
    assert_eq!(result.parse_boundary, Some(2));
    assert_eq!(result.history_limit, 2);
    assert_eq!(result.boundary_label(), "History stops at c2: map could not be parsed");
    assert_eq!(versions.reads, 3);
}

#[test]
fn slider_before_parse_failure_remains_the_boundary() {
    let mut versions = Versions::new(&[Some(leak(grid("ab"))), Some(leak(grid("aa")))], true);
    let current = parse(&grid("ab")).0;

    let result = blame(&current, &mut versions, &|| false, &mut |_, _| {}).unwrap();

    assert_eq!(result.parse_boundary, None);
    assert_eq!(result.history_limit, 1);
    assert_eq!(result.boundary_label(), "Older than blame depth");
    assert_eq!(versions.reads, 2);
}

#[test]
fn an_unparseable_head_leaves_all_tiles_at_the_boundary() {
    let mut versions = Versions::new(&[Some("not a map"), Some(leak(grid("ab")))], false);
    let current = parse(&grid("ab")).0;

    let result = blame(&current, &mut versions, &|| false, &mut |_, _| {}).unwrap();

    assert_eq!(commit_of(&result, 1, 1).as_deref(), Some("boundary"));
    assert_eq!(commit_of(&result, 2, 1).as_deref(), Some("boundary"));
    assert_eq!(result.parse_boundary, Some(0));
    assert_eq!(result.history_limit, 0);
    assert_eq!(versions.reads, 1);
}

#[test]
fn stops_reading_once_every_tile_is_blamed() {
    let mut versions = Versions::new(
        &[
            Some(leak(grid("bb"))),
            Some(leak(grid("aa"))),
            Some(leak(grid("aa"))),
            Some(leak(grid("aa"))),
        ],
        false,
    );
    let current = parse(&grid("bb")).0;

    let result = blame(&current, &mut versions, &|| false, &mut |_, _| {}).unwrap();

    assert_eq!(commit_of(&result, 1, 1).as_deref(), Some("c0"));
    assert_eq!(versions.reads, 2);
}

#[test]
fn a_grown_map_blames_new_tiles_on_the_growth() {
    let mut versions = Versions::new(&[Some(leak(grid("aab"))), Some(leak(grid("aa")))], false);
    let current = parse(&grid("aab")).0;

    let result = blame(&current, &mut versions, &|| false, &mut |_, _| {}).unwrap();

    assert_eq!(commit_of(&result, 3, 1).as_deref(), Some("c0"));
    assert_eq!(commit_of(&result, 1, 1).as_deref(), Some("c1"));
}

#[test]
fn a_map_without_history_is_uncommitted() {
    let mut versions = Versions::new(&[], false);
    let current = parse(&grid("ab")).0;

    let result = blame(&current, &mut versions, &|| false, &mut |_, _| {}).unwrap();

    assert_eq!(commit_of(&result, 1, 1).as_deref(), Some("uncommitted"));
}

#[test]
fn describes_relative_time() {
    assert_eq!(relative_time(7200, 0), "2 hours ago");
    assert_eq!(relative_time(86_400, 0), "1 day ago");
}

#[test]
fn git_versions_attribute_exact_tiles_to_file_commits() {
    if !git_available() {
        return;
    }
    let dir = temp_repo("tile-blame");
    let file = "map.dmm";
    commit(&dir, file, &grid("aa"), "add map");
    commit(&dir, file, &grid("ba"), "change first tile");
    commit(&dir, file, &grid("bb"), "change second tile");
    let path = discover(&dir.join(file)).unwrap();
    let mut versions = super::GitVersions::open(path, 500).unwrap();
    let current = parse(&grid("bb")).0;
    let result = blame(&current, &mut versions, &|| false, &mut |_, _| {}).unwrap();
    let summary = |coord| match result.at(coord) {
        Some(BlameCell::Commit(_, commit)) => commit.summary.as_str(),
        _ => panic!("expected a commit"),
    };
    assert_eq!(summary(Coord::new(1, 1, 1)), "change first tile");
    assert_eq!(summary(Coord::new(2, 1, 1)), "change second tile");
}

#[test]
fn git_versions_stop_at_unparseable_map_revision() {
    if !git_available() {
        return;
    }
    let dir = temp_repo("tile-blame-parse-boundary");
    let file = "map.dmm";
    commit(&dir, file, &grid("bb"), "original map");
    commit(&dir, file, "not a map", "unparseable map");
    commit(&dir, file, &grid("aa"), "restored map");
    commit(&dir, file, &grid("ab"), "change second tile");
    let path = discover(&dir.join(file)).unwrap();
    let current = parse(&grid("ab")).0;

    let mut versions = super::GitVersions::open(path.clone(), 500).unwrap();
    let result = blame(&current, &mut versions, &|| false, &mut |_, _| {}).unwrap();
    assert_eq!(commit_of(&result, 1, 1).as_deref(), Some("boundary"));
    let Some(BlameCell::Commit(_, commit)) = result.at(Coord::new(2, 1, 1)) else {
        panic!("expected a commit for the second tile");
    };
    assert_eq!(commit.summary, "change second tile");
    assert!(result.boundary_label().contains("map could not be parsed"));

    let mut shallow = super::GitVersions::open(path, 1).unwrap();
    let result = blame(&current, &mut shallow, &|| false, &mut |_, _| {}).unwrap();
    assert_eq!(result.parse_boundary, None);
    assert_eq!(commit_of(&result, 1, 1).as_deref(), Some("boundary"));
    assert_eq!(result.boundary_label(), "Older than blame depth");
}
