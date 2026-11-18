use std::{
    collections::{HashMap, HashSet, hash_map::Entry},
    mem,
    sync::atomic::{AtomicU64, Ordering},
};

use base::coord::Coord;
use net::LevelChange;
use prefab::PrefabInstanceId;

use crate::{
    document::{
        MapDocument,
        grid::{Grid, LevelCells},
        instance::PlacedTile,
    },
    tool::SelectionMask,
};

#[derive(Debug, Clone, PartialEq)]
pub struct TileChange {
    pub coord: Coord,
    pub before: PlacedTile,
    pub after: PlacedTile,
}

/// Map width and height on each side of an edit
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resize {
    pub before: (u32, u32),
    pub after: (u32, u32),
}

/// Region selection on each side of an edit
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionChange {
    pub before: Option<SelectionMask>,
    pub after: Option<SelectionMask>,
}

#[derive(Debug, Clone)]
pub enum LevelEdit {
    Insert { z: u32, cells: LevelCells },
    Delete { z: u32, cells: LevelCells },
}

impl LevelEdit {
    pub fn z(&self) -> u32 {
        match self {
            Self::Insert { z, .. } | Self::Delete { z, .. } => *z,
        }
    }

    pub fn cells(&self) -> &LevelCells {
        match self {
            Self::Insert { cells, .. } | Self::Delete { cells, .. } => cells,
        }
    }

    pub fn change(&self) -> LevelChange {
        match *self {
            Self::Insert { z, .. } => LevelChange::Inserted(z),
            Self::Delete { z, .. } => LevelChange::Deleted(z),
        }
    }

    pub fn undo_change(&self) -> LevelChange {
        match *self {
            Self::Insert { z, .. } => LevelChange::Deleted(z),
            Self::Delete { z, .. } => LevelChange::Inserted(z),
        }
    }
}

/// IMPORTANT: An Edit is a list of tile snapshots, not operations!!!
#[derive(Debug, Clone)]
pub struct Edit {
    pub label: String,
    pub changes: Vec<TileChange>,
    record_empty: bool,
    resize: Option<Resize>,
    level: Option<LevelEdit>,
    selection: Option<SelectionChange>,
}

impl Edit {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            changes: Vec::new(),
            record_empty: false,
            resize: None,
            level: None,
            selection: None,
        }
    }

    /// The map takes the `after` size before the changes land, and the `before` size once they are undone
    pub(crate) fn resizing(mut self, resize: Resize) -> Self {
        self.resize = Some(resize);

        self
    }

    pub fn resize(&self) -> Option<Resize> {
        self.resize
    }

    pub(crate) fn changing_level(mut self, level: LevelEdit) -> Self {
        self.level = Some(level);
        self
    }

    pub fn level(&self) -> Option<&LevelEdit> {
        self.level.as_ref()
    }

    pub fn selecting(mut self, selection: SelectionChange) -> Self {
        self.selection = Some(selection);

        self
    }

    pub fn selection(&self) -> Option<SelectionChange> {
        self.selection
    }

    pub fn is_structural(&self) -> bool {
        self.resize.is_some() || self.level.is_some()
    }

    pub fn recorded_when_empty(mut self) -> Self {
        self.record_empty = true;

        self
    }

    pub fn change(&mut self, document: &MapDocument, coord: Coord, after: PlacedTile) {
        let before = document.placed_tile(coord).unwrap_or_default();
        self.changes.push(TileChange { coord, before, after });
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty() && self.level.is_none()
    }

    pub fn affected_instances(&self) -> Vec<PrefabInstanceId> {
        let mut affected = self
            .changes
            .iter()
            .flat_map(|change| change.before.iter().chain(&change.after))
            .map(|placed| placed.id())
            .chain(self.level.iter().flat_map(|level| level.cells().ids()))
            .collect::<Vec<_>>();
        affected.sort_unstable();
        affected.dedup();
        affected
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EditGroupId(u64);

impl EditGroupId {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);

        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl Default for EditGroupId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
struct HistoryEntry {
    edit: Edit,
    group: Option<EditGroupId>,
}

#[derive(Debug)]
struct Displaced {
    redo_stack: Vec<HistoryEntry>,
    saved_at: Option<usize>,
}

#[derive(Debug)]
pub struct History {
    undo_stack: Vec<HistoryEntry>,
    redo_stack: Vec<HistoryEntry>,
    /// Saved undo position
    saved_at: Option<usize>,
    /// What pushing the top entry discarded, kept until anything else touches the history
    displaced: Option<Displaced>,
}

impl Default for History {
    fn default() -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            saved_at: Some(0),
            displaced: None,
        }
    }
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn apply_grouped(&mut self, grid: &mut Grid, edit: Edit, group: Option<EditGroupId>) -> bool {
        if edit.is_empty() && !edit.record_empty {
            return true;
        }

        if !apply_edit(grid, &edit, ChangeSide::After) {
            return false;
        }

        let undo_len = self.undo_stack.len();
        if group.is_some()
            && !edit.is_structural()
            && let Some(previous) = self.undo_stack.last_mut()
            && previous.group == group
            && !previous.edit.is_structural()
        {
            if self.saved_at == Some(undo_len) {
                self.saved_at = None;
            }

            // we only keep changes that differ (before != after), if a stoke undid itself (ie dragging on)
            // then dragging back over a tile and restoring what was there, the merged entry has no changes
            // left, so we pop it
            merge_changes(&mut previous.edit.changes, edit.changes);

            if previous.edit.changes.is_empty() {
                self.undo_stack.pop();
                self.displaced = None;
            }

            if !self.redo_stack.is_empty() {
                self.displaced = None;
            }

            self.discard_redo();

            return true;
        }

        let displaced = Displaced {
            redo_stack: mem::take(&mut self.redo_stack),
            saved_at: self.saved_at,
        };
        self.discard_redo();
        self.displaced = Some(displaced);
        self.undo_stack.push(HistoryEntry { edit, group });
        true
    }

    pub(crate) fn undo(&mut self, grid: &mut Grid) -> Option<&Edit> {
        if !apply_edit(grid, &self.undo_stack.last()?.edit, ChangeSide::Before) {
            return None;
        }

        let entry = self.undo_stack.pop()?;

        self.displaced = None;
        self.redo_stack.push(entry);

        self.redo_stack.last().map(|entry| &entry.edit)
    }

    // an undo that leaves the redo stack as the edit found it
    pub(crate) fn retract(&mut self, grid: &mut Grid) -> Option<Edit> {
        if !apply_edit(grid, &self.undo_stack.last()?.edit, ChangeSide::Before) {
            return None;
        }

        let entry = self.undo_stack.pop()?;

        match self.displaced.take() {
            Some(displaced) => {
                self.redo_stack = displaced.redo_stack;
                self.saved_at = displaced.saved_at;
            },
            None => self.discard_redo(),
        }

        Some(entry.edit)
    }

    pub(crate) fn redo(&mut self, grid: &mut Grid) -> Option<&Edit> {
        if !apply_edit(grid, &self.redo_stack.last()?.edit, ChangeSide::After) {
            return None;
        }

        let entry = self.redo_stack.pop()?;

        self.displaced = None;
        self.undo_stack.push(entry);

        self.undo_stack.last().map(|entry| &entry.edit)
    }

    pub fn mark_saved(&mut self) {
        self.saved_at = Some(self.undo_stack.len());
        self.displaced = None;
    }

    pub fn clear(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.saved_at = None;
        self.displaced = None;
    }

    pub fn is_dirty(&self) -> bool {
        self.saved_at != Some(self.undo_stack.len())
    }

    pub fn is_applied(&self, group: EditGroupId) -> bool {
        self.undo_stack.iter().any(|entry| entry.group == Some(group))
    }

    pub fn applied_groups(&self) -> HashSet<EditGroupId> {
        self.undo_stack.iter().filter_map(|entry| entry.group).collect()
    }

    pub fn undo_group(&self) -> Option<EditGroupId> {
        self.undo_stack.last()?.group
    }

    pub fn undo_depth(&self) -> usize {
        self.undo_stack.len()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn next_undo(&self) -> Option<&Edit> {
        self.undo_stack.last().map(|entry| &entry.edit)
    }

    pub fn next_redo(&self) -> Option<&Edit> {
        self.redo_stack.last().map(|entry| &entry.edit)
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.undo_stack.last().map(|entry| entry.edit.label.as_str())
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.redo_stack.last().map(|entry| entry.edit.label.as_str())
    }

    fn discard_redo(&mut self) {
        if self.saved_at.is_some_and(|saved_at| saved_at > self.undo_stack.len()) {
            self.saved_at = None;
        }
        self.redo_stack.clear();
    }
}

fn merge_changes(previous: &mut Vec<TileChange>, next: Vec<TileChange>) {
    for change in next {
        match previous.iter_mut().find(|existing| existing.coord == change.coord) {
            Some(existing) => existing.after = change.after,
            None => previous.push(change),
        }
    }

    previous.retain(|change| change.before != change.after);
}

#[derive(Clone, Copy)]
enum ChangeSide {
    Before,
    After,
}

pub(crate) fn apply_unrecorded(grid: &mut Grid, edit: &Edit) -> bool {
    apply_edit(grid, edit, ChangeSide::After)
}

fn apply_edit(grid: &mut Grid, edit: &Edit, side: ChangeSide) -> bool {
    if let Some(level) = &edit.level {
        let is_inserting = matches!(
            (level, side),
            (LevelEdit::Insert { .. }, ChangeSide::After) | (LevelEdit::Delete { .. }, ChangeSide::Before)
        );

        return if is_inserting {
            grid.insert_level(level.z(), level.cells())
        } else {
            grid.delete_level(level.z())
        };
    }

    let size = edit.resize.map(|resize| match side {
        ChangeSide::Before => resize.before,
        ChangeSide::After => resize.after,
    });

    if let Some((width, height)) = size {
        let current = grid.size();
        grid.resize(width.max(current.x), height.max(current.y));
    }

    apply_changes(grid, &edit.changes, side);
    if let Some((width, height)) = size {
        grid.resize(width, height);
    }

    true
}

// a tile changed twice by one edit ends up as its last `after`, and goes back to its first `before`
fn apply_changes(grid: &mut Grid, changes: &[TileChange], side: ChangeSide) {
    let mut tiles = Vec::new();
    let mut slots = HashMap::new();
    for change in changes {
        let placed = match side {
            ChangeSide::Before => &change.before,
            ChangeSide::After => &change.after,
        };

        match slots.entry(change.coord) {
            Entry::Vacant(slot) => {
                slot.insert(tiles.len());
                tiles.push((change.coord, placed));
            },
            Entry::Occupied(slot) => {
                if matches!(side, ChangeSide::After) {
                    tiles[*slot.get()].1 = placed;
                }
            },
        }
    }

    for (coord, _) in &tiles {
        grid.clear(*coord);
    }

    for (coord, placed) in tiles {
        grid.place(coord, placed);
    }
}

#[cfg(test)]
mod tests;
