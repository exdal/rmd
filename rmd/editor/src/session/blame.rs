use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

use dmm::{Coord, Map};
use editor::{
    blame::{self, BlameCell, BlameCounts, BlameResult},
    document::DocumentId,
};

use super::{Session, SharedHighlights};

pub(crate) struct BlameState {
    pub result: BlameResult,
    pub snapshot: Map,
    pub revision: u64,
    pub pinned: Option<u32>,
    pub counts: BlameCounts,
    /// Built once per level, the result never changes
    heatmap: RefCell<HashMap<u32, SharedHighlights>>,
    /// Outline of one commit's tiles, keyed by level and commit index
    outlines: RefCell<HashMap<(u32, u32), SharedHighlights>>,
}

impl BlameState {
    pub(super) fn new(result: BlameResult, snapshot: Map, revision: u64) -> Self {
        Self {
            counts: result.counts(),
            result,
            snapshot,
            revision,
            pinned: None,
            heatmap: RefCell::new(HashMap::new()),
            outlines: RefCell::new(HashMap::new()),
        }
    }

    pub(super) fn heatmap(&self, z: u32) -> SharedHighlights {
        self.heatmap
            .borrow_mut()
            .entry(z)
            .or_insert_with(|| blame_heatmap(&self.result, z).into())
            .clone()
    }

    pub(super) fn outline(&self, z: u32, commit: u32) -> SharedHighlights {
        self.outlines
            .borrow_mut()
            .entry((z, commit))
            .or_insert_with(|| {
                let covered = self
                    .result
                    .level(z)
                    .filter(|(_, cell)| *cell == commit)
                    .map(|(coord, _)| [coord.x as i32, coord.y as i32])
                    .collect::<HashSet<_>>();
                if covered.is_empty() {
                    return Arc::new([]);
                }

                Arc::new([editor::bake::Highlight {
                    tiles: editor::bake::highlight_tiles(&covered),
                    z: z as i32,
                    color: [1.0, 1.0, 1.0],
                    fill: 0.0,
                    outline: true,
                    when: editor::bake::HIGHLIGHT_ALWAYS,
                    label: self
                        .result
                        .commits
                        .get(commit as usize)
                        .map(|commit| commit.short.clone()),
                }])
            })
            .clone()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum BlameHeat {
    Commit(u32),
    Worktree,
    Boundary,
}

const BLAME_BASE_FILL: f32 = 0.55;

const BLAME_OVERLAY_OPACITY_REDUCTION: f32 = 0.15;

const BLAME_HOT_OPACITY_REDUCTION: f32 = 0.30;

const BLAME_COLDEST_HEAT: f32 = 0.12;

fn blame_heatmap(result: &BlameResult, z: u32) -> Vec<editor::bake::Highlight> {
    let mut buckets: BTreeMap<BlameHeat, HashSet<[i32; 2]>> = BTreeMap::new();
    for (coord, cell) in result.level(z) {
        let source = blame_heat(result, cell);
        buckets
            .entry(source)
            .or_default()
            .insert([coord.x as i32, coord.y as i32]);
    }
    buckets
        .into_iter()
        .map(|(source, covered)| editor::bake::Highlight {
            tiles: editor::bake::highlight_tiles(&covered),
            z: z as i32,
            color: blame_heatmap_color(source, result.heat_span),
            fill: blame_heatmap_fill(source, result.heat_span),
            outline: false,
            when: editor::bake::HIGHLIGHT_ALWAYS,
            label: None,
        })
        .collect()
}

fn blame_heatmap_color(source: BlameHeat, heat_span: usize) -> [f32; 3] {
    let index = match source {
        BlameHeat::Commit(index) => index as usize,
        BlameHeat::Worktree => return [0.14, 0.9, 0.32],
        BlameHeat::Boundary => return [0.4, 0.2, 0.8],
    };
    let color = inferno(blame_revision_heat(index, heat_span));
    let peak = color.into_iter().fold(0.0f32, f32::max);
    let gain = (0.85 / peak.max(f32::EPSILON)).max(1.0);
    color.map(|channel| (channel * gain).clamp(0.0, 1.0))
}

fn blame_heat(result: &BlameResult, cell: u32) -> BlameHeat {
    match cell {
        blame::UNCOMMITTED => BlameHeat::Worktree,
        blame::BOUNDARY => BlameHeat::Boundary,
        index if (index as usize) < result.history_limit => BlameHeat::Commit(index),
        _ => BlameHeat::Boundary,
    }
}

pub(crate) fn blame_color(result: &BlameResult, cell: u32) -> [f32; 3] {
    blame_heatmap_color(blame_heat(result, cell), result.heat_span)
}

fn blame_heatmap_fill(source: BlameHeat, heat_span: usize) -> f32 {
    let base = BLAME_BASE_FILL * (1.0 - BLAME_OVERLAY_OPACITY_REDUCTION);
    match source {
        BlameHeat::Commit(index) => {
            base * (1.0 - BLAME_HOT_OPACITY_REDUCTION * blame_revision_heat(index as usize, heat_span))
        },
        BlameHeat::Worktree | BlameHeat::Boundary => base,
    }
}

/// 1.0 at HEAD
fn blame_revision_heat(index: usize, heat_span: usize) -> f32 {
    let age = (index as f32 / (heat_span.max(2) - 1) as f32).min(1.0);

    BLAME_COLDEST_HEAT + (1.0 - BLAME_COLDEST_HEAT) * (1.0 - age)
}

fn inferno(t: f32) -> [f32; 3] {
    const C0: [f32; 3] = [0.000_218_940_37, 0.001_651_004_7, -0.019_480_899];
    const C1: [f32; 3] = [0.106_513_42, 0.563_956_44, 3.932_712_3];
    const C2: [f32; 3] = [11.602_493, -3.972_854, -15.942_394];
    const C3: [f32; 3] = [-41.703_995, 17.436_398, 44.354_145];
    const C4: [f32; 3] = [77.162_93, -33.402_36, -81.807_31];
    const C5: [f32; 3] = [-71.319_43, 32.626_064, 73.209_52];
    const C6: [f32; 3] = [25.131_126, -12.242_669, -23.070_326];

    let t = t.clamp(0.0, 1.0);
    std::array::from_fn(|channel| {
        C0[channel]
            + t * (C1[channel]
                + t * (C2[channel] + t * (C3[channel] + t * (C4[channel] + t * (C5[channel] + t * C6[channel])))))
    })
}

impl Session {
    pub(crate) fn toggle_blame(&mut self, id: DocumentId, depth: usize) {
        let Some(git) = self.caches.get_mut(&id).and_then(|cache| cache.git.as_mut()) else {
            return;
        };

        git.show_blame = !git.show_blame;
        if git.show_blame {
            git.show_diff = false;
        }

        if git.show_blame && git.blame.is_none() {
            self.run_blame(id, depth);
        }
    }

    pub(crate) fn run_blame(&mut self, id: DocumentId, depth: usize) {
        if let Some(git) = self.caches.get_mut(&id).and_then(|cache| cache.git.as_mut()) {
            git.show_blame = true;
            git.show_diff = false;
        }

        let Some(document) = self.state.document(id) else {
            return;
        };

        let Some(cache) = self.caches.get(&id) else { return };
        let Some(git) = cache.git.as_ref() else { return };

        self.git_worker
            .blame(id, git.repo.clone(), document.map.clone(), cache.map_revision, depth);
    }

    pub(crate) fn blame_progress(&self, id: DocumentId) -> Option<usize> { self.git_worker.blame_progress(id) }

    pub(crate) fn blame_at(&self, id: DocumentId, coord: Coord) -> Option<(BlameCell<'_>, bool)> {
        let cache = self.caches.get(&id)?;
        let blame = cache.git.as_ref()?.blame.as_ref()?;
        let current = self.state.document(id)?.map.tile_at(coord);
        let old = blame.snapshot.tile_at(coord);
        let changed = match (current, old) {
            (Some(current), Some(old)) => !dmm::merge::tiles_equal(current, old),
            (None, None) => false,
            _ => true,
        };

        Some((blame.result.at(coord)?, changed))
    }

    pub(crate) fn blame_stale(&self, id: DocumentId) -> bool {
        self.caches
            .get(&id)
            .and_then(|cache| {
                cache
                    .git
                    .as_ref()
                    .and_then(|git| git.blame.as_ref())
                    .map(|blame| blame.revision != cache.map_revision)
            })
            .unwrap_or(false)
    }

    pub(crate) fn pin_blame(&mut self, id: DocumentId, commit: u32) {
        if let Some(git) = self.caches.get_mut(&id).and_then(|cache| cache.git.as_mut()) {
            git.show_blame = true;
            git.show_diff = false;
        }

        if let Some(blame) = self
            .caches
            .get_mut(&id)
            .and_then(|cache| cache.git.as_mut())
            .and_then(|git| git.blame.as_mut())
        {
            blame.pinned = (blame.pinned != Some(commit)).then_some(commit);
        }
    }

    pub(crate) fn blame_state(&self, id: DocumentId) -> Option<&BlameState> { self.git_state(id)?.blame.as_ref() }
}

#[cfg(test)]
mod tests {
    use super::{BLAME_COLDEST_HEAT, BlameHeat, blame_heatmap_color, blame_revision_heat};

    #[test]
    fn blame_palette_uses_revision_positions_and_keeps_worktree_green() {
        let heat_span = 500;
        let boundary = blame_heatmap_color(BlameHeat::Boundary, heat_span);
        let cold = blame_heatmap_color(BlameHeat::Commit(499), heat_span);
        let middle = blame_heatmap_color(BlameHeat::Commit(250), heat_span);
        let hot = blame_heatmap_color(BlameHeat::Commit(0), heat_span);
        let worktree = blame_heatmap_color(BlameHeat::Worktree, heat_span);
        let short_history = blame_heatmap_color(BlameHeat::Commit(1), 2);
        let long_history = blame_heatmap_color(BlameHeat::Commit(1), heat_span);

        assert!(boundary[2] >= 0.8 && boundary[2] > boundary[0]);
        assert!(cold[2] >= 0.8 && cold[2] > cold[0]);
        assert_ne!(middle, cold);
        assert_ne!(middle, hot);
        assert_ne!(short_history, long_history);
        assert!(hot[0] >= 0.9 && hot[1] >= 0.6);
        assert!(worktree[1] >= 0.8 && worktree[1] > worktree[0] * 3.0 && worktree[1] > worktree[2] * 2.0);
    }

    #[test]
    fn blame_heat_spans_head_to_the_oldest_owning_revision() {
        let heat = |index| blame_revision_heat(index, 347);

        assert_eq!(heat(0), 1.0);
        assert_eq!(heat(346), BLAME_COLDEST_HEAT);
        assert_eq!(heat(400), BLAME_COLDEST_HEAT, "listed commits past the span stay cold");
        assert!((heat(173) - (1.0 + BLAME_COLDEST_HEAT) / 2.0).abs() < 1e-3);
        assert!((1..347).all(|index| heat(index) < heat(index - 1)));
        assert_eq!(blame_revision_heat(0, 1), 1.0);
        assert_eq!(blame_revision_heat(0, 0), 1.0);
    }
}
