use crate::{
    document::{DocumentId, MapDocument},
    frame::options::HiddenTypes,
    region::session::RegionState,
    session::Session,
    tool::RegionSelectionMode,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StashMode {
    Copy,
    Move,
}

impl Session {
    pub fn is_stash_active(&self) -> bool {
        self.state.active_document().is_some_and(MapDocument::is_stash)
    }

    pub fn can_stash_selection(&self, mode: StashMode) -> bool {
        if self.is_stash_active() || self.selection_mode() != RegionSelectionMode::Full {
            return false;
        }

        let is_readable = self
            .state
            .active()
            .is_some_and(|active| self.effective_grid(active).is_some());

        self.selection_mask().is_some() && is_readable && (mode == StashMode::Copy || self.can_clear_selection())
    }

    pub fn stash_selection(&mut self, mode: StashMode) -> Option<DocumentId> {
        if !self.can_stash_selection(mode) {
            return None;
        }

        let selection = self.selection()?;
        let region = match mode {
            StashMode::Copy => self.read_region(selection, RegionSelectionMode::Full),
            StashMode::Move => self.take_region(selection, RegionSelectionMode::Full, "move to stash"),
        }?;

        let id = self.state.stash_mut().push(region);
        self.rebake(id);

        Some(id)
    }

    pub fn remove_stash_entry(&mut self, id: DocumentId) -> bool {
        if self.node_edit.as_ref().is_some_and(|edit| edit.document == id) {
            self.cancel_node_edit();
        }

        if self.state.active() == Some(id) {
            self.finish_live_move();
            self.region_state = RegionState::Idle;
        }

        if !self.state.remove_stash_entry(id) {
            return false;
        }

        self.caches.remove(&id);

        true
    }

    pub fn clear_stash(&mut self) {
        for id in self.state.stash().ids() {
            self.remove_stash_entry(id);
        }
    }

    pub fn stash_ids(&self) -> Vec<DocumentId> {
        self.state.stash().ids()
    }

    pub fn stash_len(&self) -> usize {
        self.state.stash().len()
    }

    pub(crate) fn active_stash_hidden(&self) -> Option<&HiddenTypes> {
        self.state.active().and_then(|id| self.state.stash().hidden(id))
    }

    pub(crate) fn rendered_document_ids(&self) -> Vec<DocumentId> {
        let mut ids = self.state.document_ids();
        ids.extend(self.state.stash().ids());

        ids
    }
}

#[cfg(test)]
mod tests;
