use std::{
    collections::{HashMap, HashSet},
    sync::atomic::{AtomicU64, Ordering},
};

use dmm::{Coord, Map, Tile, key::Key};

use crate::document::{MapDocument, PlacedTile, PrefabInstances};

#[derive(Debug, Clone, PartialEq)]
pub struct TileChange {
    pub coord: Coord,
    pub before: PlacedTile,
    pub after: PlacedTile,
}

#[derive(Debug, Clone)]
pub struct Edit {
    pub label: String,
    pub changes: Vec<TileChange>,
    record_empty: bool,
}

impl Edit {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            changes: Vec::new(),
            record_empty: false,
        }
    }

    pub fn recorded_when_empty(mut self) -> Self {
        self.record_empty = true;

        self
    }

    pub fn change(&mut self, document: &MapDocument, coord: Coord, after: PlacedTile) {
        let before = document.placed_tile(coord).unwrap_or_default();
        self.changes.push(TileChange { coord, before, after });
    }

    pub fn is_empty(&self) -> bool { self.changes.is_empty() }

    pub fn affected_instances(&self) -> Vec<dmm::PrefabInstanceId> {
        let mut affected = self
            .changes
            .iter()
            .flat_map(|change| change.before.iter().chain(&change.after))
            .map(|placed| placed.id())
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

    pub const fn get(self) -> u64 { self.0 }
}

impl Default for EditGroupId {
    fn default() -> Self { Self::new() }
}

#[derive(Debug)]
struct HistoryEntry {
    edit: Edit,
    group: Option<EditGroupId>,
}

#[derive(Debug)]
pub struct History {
    undo_stack: Vec<HistoryEntry>,
    redo_stack: Vec<HistoryEntry>,
    /// Saved undo position
    saved_at: Option<usize>,
}

impl Default for History {
    fn default() -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            saved_at: Some(0),
        }
    }
}

impl History {
    pub fn new() -> Self { Self::default() }

    pub(crate) fn apply_grouped(
        &mut self, map: &mut Map, instances: &mut PrefabInstances, key_usage: &mut HashMap<Key, usize>, edit: Edit,
        group: Option<EditGroupId>,
    ) {
        if edit.is_empty() && !edit.record_empty {
            return;
        }

        apply_changes(map, instances, key_usage, &edit.changes, ChangeSide::After);

        let undo_len = self.undo_stack.len();
        if group.is_some()
            && let Some(previous) = self.undo_stack.last_mut()
            && previous.group == group
        {
            if self.saved_at == Some(undo_len) {
                self.saved_at = None;
            }
            merge_changes(&mut previous.edit.changes, edit.changes);
            if previous.edit.changes.is_empty() {
                self.undo_stack.pop();
            }
            self.discard_redo();

            return;
        }

        self.discard_redo();
        self.undo_stack.push(HistoryEntry { edit, group });
    }

    pub(crate) fn undo(
        &mut self, map: &mut Map, instances: &mut PrefabInstances, key_usage: &mut HashMap<Key, usize>,
    ) -> Option<&Edit> {
        let entry = self.undo_stack.pop()?;
        apply_changes(map, instances, key_usage, &entry.edit.changes, ChangeSide::Before);

        self.redo_stack.push(entry);

        self.redo_stack.last().map(|entry| &entry.edit)
    }

    pub(crate) fn redo(
        &mut self, map: &mut Map, instances: &mut PrefabInstances, key_usage: &mut HashMap<Key, usize>,
    ) -> Option<&Edit> {
        let entry = self.redo_stack.pop()?;
        apply_changes(map, instances, key_usage, &entry.edit.changes, ChangeSide::After);

        self.undo_stack.push(entry);

        self.undo_stack.last().map(|entry| &entry.edit)
    }

    pub fn mark_saved(&mut self) { self.saved_at = Some(self.undo_stack.len()); }

    pub fn is_dirty(&self) -> bool { self.saved_at != Some(self.undo_stack.len()) }

    pub fn is_applied(&self, group: EditGroupId) -> bool {
        self.undo_stack.iter().any(|entry| entry.group == Some(group))
    }

    pub fn applied_groups(&self) -> HashSet<EditGroupId> {
        self.undo_stack.iter().filter_map(|entry| entry.group).collect()
    }

    pub fn undo_depth(&self) -> usize { self.undo_stack.len() }

    pub fn can_undo(&self) -> bool { !self.undo_stack.is_empty() }

    pub fn can_redo(&self) -> bool { !self.redo_stack.is_empty() }

    pub fn undo_label(&self) -> Option<&str> { self.undo_stack.last().map(|entry| entry.edit.label.as_str()) }

    pub fn redo_label(&self) -> Option<&str> { self.redo_stack.last().map(|entry| entry.edit.label.as_str()) }

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

fn apply_changes(
    map: &mut Map, instances: &mut PrefabInstances, key_usage: &mut HashMap<Key, usize>, changes: &[TileChange],
    side: ChangeSide,
) {
    for change in changes {
        instances.remove(change.coord);
    }

    for change in changes {
        let placed = match side {
            ChangeSide::Before => &change.before,
            ChangeSide::After => &change.after,
        };
        let tile = placed.iter().map(|entry| entry.prefab().clone()).collect();
        let ids = placed.iter().map(|entry| entry.id()).collect();

        set_tile(map, key_usage, change.coord, tile);
        instances.insert(change.coord, ids);
    }
}

fn set_tile(map: &mut Map, key_usage: &mut HashMap<Key, usize>, coord: Coord, tile: Tile) {
    let Some(z) = coord.z.checked_sub(1).map(|z| z as usize) else {
        return;
    };

    let Some(row_index) = map.size.y.checked_sub(coord.y).map(|y| y as usize) else {
        return;
    };

    let Some(column) = coord.x.checked_sub(1).map(|x| x as usize) else {
        return;
    };

    let Some(previous) = map
        .grid
        .get(z)
        .and_then(|level| level.get(row_index))
        .and_then(|row| row.get(column))
        .copied()
    else {
        return;
    };

    let remove_previous = match key_usage.get_mut(&previous) {
        Some(count) if *count > 1 => {
            *count -= 1;
            false
        },
        Some(_) => true,
        None => false,
    };
    if remove_previous {
        key_usage.remove(&previous);
        map.dictionary.remove(&previous);
    }

    let key = map.intern_tile(tile);
    *key_usage.entry(key).or_insert(0) += 1;
    map.grid[z][row_index][column] = key;
}

#[cfg(test)]
mod tests {
    use core::{path::TreePath, types::Value};

    use dmm::{Coord, Map, Prefab, Size};

    use super::{Edit, EditGroupId};
    use crate::document::MapDocument;

    fn document() -> MapDocument {
        let mut map = Map::new(Size { x: 3, y: 1, z: 1 });
        let occupied = map.intern_tile(vec![
            Prefab::new(TreePath::parse("/turf/floor")),
            Prefab::new(TreePath::parse("/obj/table")),
        ]);
        let empty = map.intern_tile(Vec::new());
        map.grid[0][0] = vec![occupied, occupied, empty];

        MapDocument::new(map, 1)
    }

    #[test]
    fn changing_one_prefab_preserves_its_id_and_only_changes_its_cell() {
        let mut document = document();
        let source = Coord::new(1, 1, 1);
        let untouched = Coord::new(2, 1, 1);
        let id = document.instance_ids_at(source)[1];
        let untouched_before = document.map.tile_at(untouched).cloned();
        let mut after = document.placed_tile(source).unwrap();
        after[1]
            .prefab_mut()
            .set_var("name".into(), Value::Text(String::from("moved table")));

        let mut edit = Edit::new("change table");
        edit.change(&document, source, after);
        document.apply(edit);

        assert_eq!(document.instance_ids_at(source)[1], id);
        assert_eq!(document.instance_location(id).unwrap().coord, source);
        assert_eq!(document.map.tile_at(untouched).cloned(), untouched_before);
    }

    #[test]
    fn moving_a_prefab_transfers_its_id_and_undo_restores_it() {
        let mut document = document();
        let source = Coord::new(1, 1, 1);
        let destination = Coord::new(3, 1, 1);
        let mut source_after = document.placed_tile(source).unwrap();
        let moved = source_after.remove(1);
        let id = moved.id();
        let mut destination_after = document.placed_tile(destination).unwrap();
        destination_after.push(moved);

        let mut edit = Edit::new("move table");
        edit.change(&document, source, source_after);
        edit.change(&document, destination, destination_after);
        document.apply(edit);

        assert_eq!(document.instance_location(id).unwrap().coord, destination);
        assert!(document.undo());
        assert_eq!(document.instance_location(id).unwrap().coord, source);
        assert!(document.redo());
        assert_eq!(document.instance_location(id).unwrap().coord, destination);
    }

    #[test]
    fn copies_get_new_ids_and_deleted_ids_return_on_undo() {
        let mut document = document();
        let source = Coord::new(1, 1, 1);
        let destination = Coord::new(3, 1, 1);
        let original = document.placed_tile(source).unwrap()[1].clone();
        let copy = document.instantiate(original.prefab().clone());
        let copied_id = copy.id();
        assert_ne!(copied_id, original.id());

        let mut edit = Edit::new("copy table");
        edit.change(&document, destination, vec![copy]);
        document.apply(edit);
        assert_eq!(document.instance_location(copied_id).unwrap().coord, destination);

        let mut delete = Edit::new("delete table");
        delete.change(&document, destination, Vec::new());
        document.apply(delete);
        assert_eq!(document.instance_location(copied_id), None);
        assert!(document.undo());
        assert_eq!(document.instance_location(copied_id).unwrap().coord, destination);
    }

    #[test]
    fn undo_and_redo_move_through_the_saved_position() {
        let mut document = document();
        let coord = Coord::new(1, 1, 1);
        let mut edit = Edit::new("clear tile");
        edit.change(&document, coord, Vec::new());
        document.apply(edit);
        document.history.mark_saved();

        assert!(!document.is_dirty());
        assert_eq!(document.undo_label(), Some("clear tile"));
        assert!(document.undo());
        assert!(document.is_dirty());
        assert_eq!(document.redo_label(), Some("clear tile"));
        assert!(document.redo());
        assert!(!document.is_dirty());
    }

    #[test]
    fn replacing_the_saved_redo_branch_stays_dirty() {
        let mut document = document();
        let mut saved = Edit::new("clear first tile");
        saved.change(&document, Coord::new(1, 1, 1), Vec::new());
        document.apply(saved);
        document.history.mark_saved();
        assert!(document.undo());

        let mut replacement = Edit::new("clear second tile");
        replacement.change(&document, Coord::new(2, 1, 1), Vec::new());
        document.apply(replacement);

        assert!(document.is_dirty());
        assert_eq!(document.undo_label(), Some("clear second tile"));
        assert_eq!(document.redo_label(), None);
    }

    #[test]
    fn extending_a_saved_edit_group_stays_dirty() {
        let mut document = document();
        let coord = Coord::new(1, 1, 1);
        let group = EditGroupId::new();
        let mut first = document.placed_tile(coord).unwrap();
        first.pop();
        let mut edit = Edit::new("change tile");
        edit.change(&document, coord, first);
        document.apply_grouped(edit, Some(group));
        document.history.mark_saved();

        let mut edit = Edit::new("change tile");
        edit.change(&document, coord, Vec::new());
        document.apply_grouped(edit, Some(group));

        assert!(document.is_dirty());
        assert!(document.undo());
        assert!(!document.undo());
    }

    #[test]
    fn an_empty_recorded_edit_is_undoable_and_tracked_by_group() {
        let mut document = document();
        let group = EditGroupId::new();

        document.apply_grouped(Edit::new("keep tile"), Some(EditGroupId::new()));
        assert!(!document.history.can_undo());

        document.apply_grouped(Edit::new("keep tile").recorded_when_empty(), Some(group));
        assert!(document.history.is_applied(group));
        assert_eq!(document.undo_label(), Some("keep tile"));

        assert!(document.undo());
        assert!(!document.history.is_applied(group));
        assert!(document.redo());
        assert!(document.history.is_applied(group));
    }
}
