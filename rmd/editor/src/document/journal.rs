use std::{collections::BTreeSet, mem};

use dmm::Coord;
use net::LevelChange;

use super::command::Edit;

#[derive(Debug, Default)]
pub struct Journal {
    pub coords: BTreeSet<Coord>,
    pub reshaped: bool,
}

impl Journal {
    pub(super) fn record(&mut self, edit: &Edit) {
        self.coords.extend(edit.changes.iter().map(|change| change.coord));
        self.reshaped |= edit.is_structural();
    }

    pub(super) fn extend(&mut self, other: Journal) {
        self.coords.extend(other.coords);
        self.reshaped |= other.reshaped;
    }

    pub fn is_empty(&self) -> bool {
        self.coords.is_empty() && !self.reshaped
    }

    pub(super) fn shift(&mut self, change: LevelChange) {
        self.coords = mem::take(&mut self.coords)
            .into_iter()
            .filter_map(|coord| change.level(coord.z).map(|z| Coord { z, ..coord }))
            .collect();
    }
}
