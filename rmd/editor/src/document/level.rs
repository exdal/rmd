use dmm::{Coord, Prefab};
use net::{LevelChange, LevelContents};

use super::{
    MapDocument, command,
    command::{Edit, LevelEdit, Resize},
    grid::LevelCells,
    instance::PlacedTile,
    patch,
};

impl MapDocument {
    /// Keeps the bottom-left corner in place, tiles that appear hold `fill`. One undo step, which drops the
    /// area focus and any selection that no longer fits.
    pub fn resize(&mut self, width: u32, height: u32, fill: &[Prefab]) -> bool {
        let size = self.map.size();
        let (old_width, old_height) = (size.x, size.y);
        if width == 0 || height == 0 || (width, height) == (old_width, old_height) {
            return false;
        }

        let mut edit = Edit::new(format!("resize map to {width}x{height}")).resizing(Resize {
            before: (old_width, old_height),
            after: (width, height),
        });
        for z in 1..=size.z {
            for y in 1..=height.max(old_height) {
                for x in 1..=width.max(old_width) {
                    let kept = x <= width && y <= height;
                    let existed = x <= old_width && y <= old_height;
                    let coord = Coord::new(x, y, z);
                    if existed && !kept {
                        edit.change(self, coord, PlacedTile::new());
                    } else if kept && !existed {
                        let placed = fill.iter().map(|prefab| self.instantiate(prefab.clone())).collect();
                        edit.change(self, coord, placed);
                    }
                }
            }
        }

        self.focus = None;
        if self
            .selection
            .is_some_and(|selection| selection.max.x > width || selection.max.y > height)
        {
            self.selection = None;
        }

        self.apply(edit)
    }

    pub fn append_level(&mut self, tile: &[Prefab]) -> Option<u32> {
        if self.read_only {
            return None;
        }

        let edit = self.new_level_edit(tile)?;
        let z = edit.level()?.z();
        self.apply(edit).then_some(z)
    }

    // in this map's instance ids
    pub fn level_cells(&mut self, contents: &LevelContents) -> LevelCells {
        patch::decode_level(&mut self.map, contents)
    }

    pub fn insert_requested_level(&mut self, z: u32, cells: LevelCells) -> bool {
        if self.read_only {
            return false;
        }

        let edit = Edit::new(format!("create Z level {z}")).changing_level(LevelEdit::Insert { z, cells });
        if !self.history.apply_grouped(&mut self.map, edit, None) {
            return false;
        }

        self.requested_level_changed(LevelChange::Inserted(z));

        true
    }

    pub fn insert_remote_level(&mut self, z: u32, cells: LevelCells) -> bool {
        if self.read_only {
            return false;
        }

        let is_appended = z > self.map.size().z;
        let edit = Edit::new(format!("create Z level {z}")).changing_level(LevelEdit::Insert { z, cells });
        if !command::apply_unrecorded(&mut self.map, &edit) {
            return false;
        }

        // a level on top renumbers nothing, one in between moves the levels our history points at
        if !is_appended {
            let change = LevelChange::Inserted(z);
            self.history.clear();
            self.shift_journal(change);
            self.level_changed(change.level(self.z).unwrap_or(z));
        }

        self.pending_write = true;
        self.generation += 1;

        true
    }

    // our own level undo, back from the server like a peer's change would be
    pub fn undo_requested_level(&mut self, change: LevelChange) -> bool {
        if self.read_only {
            return false;
        }

        if self
            .history
            .next_undo()
            .and_then(Edit::level)
            .map(LevelEdit::undo_change)
            != Some(change)
            || self.history.undo(&mut self.map).is_none()
        {
            return false;
        }

        self.requested_level_changed(change);

        true
    }

    pub fn redo_requested_level(&mut self, change: LevelChange) -> bool {
        if self.read_only {
            return false;
        }

        if self.history.next_redo().and_then(Edit::level).map(LevelEdit::change) != Some(change)
            || self.history.redo(&mut self.map).is_none()
        {
            return false;
        }

        self.requested_level_changed(change);

        true
    }

    fn requested_level_changed(&mut self, change: LevelChange) {
        self.shift_journal(change);
        let (LevelChange::Appended(z) | LevelChange::Inserted(z) | LevelChange::Deleted(z)) = change;
        self.level_changed(z);
        self.generation += 1;
    }

    fn shift_journal(&mut self, change: LevelChange) {
        if let Some(journal) = self.journal.as_mut() {
            journal.shift(change);
        }
    }

    fn new_level_edit(&mut self, tile: &[Prefab]) -> Option<Edit> {
        let z = self.map.size().z.checked_add(1)?;
        let cells = self.map.filled_level(tile);

        Some(Edit::new(format!("create Z level {z}")).changing_level(LevelEdit::Insert { z, cells }))
    }

    fn delete_level_edit(&self, z: u32) -> Option<Edit> {
        if self.map.size().z <= 1 {
            return None;
        }

        let cells = self.map.level(z)?;

        Some(Edit::new(format!("delete Z level {z}")).changing_level(LevelEdit::Delete { z, cells }))
    }

    pub fn delete_level(&mut self, z: u32) -> bool {
        if self.read_only {
            return false;
        }

        self.delete_level_edit(z).is_some_and(|edit| self.apply(edit))
    }

    pub fn delete_requested_level(&mut self, z: u32) -> bool {
        if self.read_only {
            return false;
        }

        let Some(edit) = self.delete_level_edit(z) else {
            return false;
        };

        if !self.history.apply_grouped(&mut self.map, edit, None) {
            return false;
        }

        self.requested_level_changed(LevelChange::Deleted(z));

        true
    }

    pub fn delete_remote_level(&mut self, z: u32) -> bool {
        if self.read_only {
            return false;
        }

        let Some(edit) = self.delete_level_edit(z) else {
            return false;
        };

        if !command::apply_unrecorded(&mut self.map, &edit) {
            return false;
        }

        // every entry points at levels by their old numbers
        let change = LevelChange::Deleted(z);
        self.history.clear();
        self.shift_journal(change);
        self.level_changed(change.level(self.z).unwrap_or(z));
        self.pending_write = true;
        self.generation += 1;

        true
    }

    pub(super) fn level_changed(&mut self, z: u32) {
        self.z = z.min(self.map.size().z).max(1);
        self.selection = None;
        self.picked.clear();
        self.focus = None;
    }
}
