use dmm::{Coord, Map, Tile};

use crate::document::{MapDocument, PlacedTile, PrefabInstances};

#[derive(Debug, Clone)]
pub struct TileChange {
    pub coord: Coord,
    pub before: PlacedTile,
    pub after: PlacedTile,
}

#[derive(Debug, Clone)]
pub struct Edit {
    pub label: String,
    pub changes: Vec<TileChange>,
}

impl Edit {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            changes: Vec::new(),
        }
    }

    pub fn change(&mut self, document: &MapDocument, coord: Coord, after: PlacedTile) {
        let before = document.placed_tile(coord).unwrap_or_default();
        self.changes.push(TileChange { coord, before, after });
    }

    pub fn is_empty(&self) -> bool { self.changes.is_empty() }
}

#[derive(Debug, Default)]
pub struct History {
    undo_stack: Vec<Edit>,
    redo_stack: Vec<Edit>,
    /// Saved undo position
    saved_at: usize,
}

impl History {
    pub fn new() -> Self { Self::default() }

    pub(crate) fn apply(&mut self, map: &mut Map, instances: &mut PrefabInstances, edit: Edit) {
        if edit.is_empty() {
            return;
        }

        apply_changes(map, instances, &edit.changes, ChangeSide::After);

        self.undo_stack.push(edit);
        self.redo_stack.clear();
    }

    pub(crate) fn undo(&mut self, map: &mut Map, instances: &mut PrefabInstances) -> Option<&Edit> {
        let edit = self.undo_stack.pop()?;
        apply_changes(map, instances, &edit.changes, ChangeSide::Before);

        self.redo_stack.push(edit);

        self.redo_stack.last()
    }

    pub(crate) fn redo(&mut self, map: &mut Map, instances: &mut PrefabInstances) -> Option<&Edit> {
        let edit = self.redo_stack.pop()?;
        apply_changes(map, instances, &edit.changes, ChangeSide::After);

        self.undo_stack.push(edit);

        self.undo_stack.last()
    }

    pub fn mark_saved(&mut self) { self.saved_at = self.undo_stack.len(); }

    pub fn is_dirty(&self) -> bool { self.undo_stack.len() != self.saved_at }

    pub fn can_undo(&self) -> bool { !self.undo_stack.is_empty() }

    pub fn can_redo(&self) -> bool { !self.redo_stack.is_empty() }
}

#[derive(Clone, Copy)]
enum ChangeSide {
    Before,
    After,
}

fn apply_changes(map: &mut Map, instances: &mut PrefabInstances, changes: &[TileChange], side: ChangeSide) {
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

        set_tile(map, change.coord, tile);
        instances.insert(change.coord, ids);
    }
}

/// Dictionary pruning happens on save
fn set_tile(map: &mut Map, coord: Coord, tile: Tile) {
    let key = map.intern_tile(tile);

    let Some(z) = coord.z.checked_sub(1).map(|z| z as usize) else {
        return;
    };

    let Some(row_index) = map.size.y.checked_sub(coord.y).map(|y| y as usize) else {
        return;
    };

    let Some(column) = coord.x.checked_sub(1).map(|x| x as usize) else {
        return;
    };

    if let Some(slot) = map
        .grid
        .get_mut(z)
        .and_then(|level| level.get_mut(row_index))
        .and_then(|row| row.get_mut(column))
    {
        *slot = key;
    }
}

#[cfg(test)]
mod tests {
    use core::{path::TreePath, types::Value};

    use dmm::{Coord, Map, Prefab, Size};

    use super::Edit;
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
}
