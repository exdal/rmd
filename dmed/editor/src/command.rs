use dmm::{Coord, Map, Tile};

#[derive(Debug, Clone)]
pub struct TileChange {
    pub coord: Coord,
    pub before: Tile,
    pub after: Tile,
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

    pub fn change(&mut self, map: &Map, coord: Coord, after: Tile) {
        let before = map.tile_at(coord).cloned().unwrap_or_default();
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

    pub fn apply(&mut self, map: &mut Map, edit: Edit) {
        if edit.is_empty() {
            return;
        }

        for change in &edit.changes {
            set_tile(map, change.coord, change.after.clone());
        }

        self.undo_stack.push(edit);
        self.redo_stack.clear();
    }

    pub fn undo(&mut self, map: &mut Map) -> Option<&Edit> {
        let edit = self.undo_stack.pop()?;
        for change in edit.changes.iter().rev() {
            set_tile(map, change.coord, change.before.clone());
        }

        self.redo_stack.push(edit);

        self.redo_stack.last()
    }

    pub fn redo(&mut self, map: &mut Map) -> Option<&Edit> {
        let edit = self.redo_stack.pop()?;
        for change in &edit.changes {
            set_tile(map, change.coord, change.after.clone());
        }

        self.undo_stack.push(edit);

        self.undo_stack.last()
    }

    pub fn mark_saved(&mut self) { self.saved_at = self.undo_stack.len(); }

    pub fn is_dirty(&self) -> bool { self.undo_stack.len() != self.saved_at }

    pub fn can_undo(&self) -> bool { !self.undo_stack.is_empty() }

    pub fn can_redo(&self) -> bool { !self.redo_stack.is_empty() }
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
