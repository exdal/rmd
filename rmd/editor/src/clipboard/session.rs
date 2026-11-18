use base::coord::Coord;
use prefab::PrefabInstanceId;

use crate::{
    clipboard::{self, PasteMerge, TileRegion},
    document::{DocumentId, command::EditGroupId, selection::Selection},
    tool::{RegionSelectionMode, SelectionMask, SelectionRotation, Tool, ToolEdit},
};
use crate::{region::session::RegionState, session::Session};

#[derive(Debug)]
pub(crate) struct FloatingPaste {
    document: DocumentId,
    group: EditGroupId,
    region: TileRegion,
    pub(crate) rotation: SelectionRotation,
    pub(crate) target: SelectionMask,
}

impl Session {
    pub(crate) fn read_region(&self, selection: Selection, mode: RegionSelectionMode) -> Option<TileRegion> {
        let hidden = self.hidden_types();
        let hidden = match self.active_stash_hidden() {
            Some(captured) => hidden.union(captured),
            None => hidden,
        };

        self.state
            .active()
            .and_then(|active| self.effective_grid(active))
            .and_then(|map| clipboard::copy_region(map, selection, mode, hidden))
    }

    pub fn copy_selection(&mut self, mode: RegionSelectionMode) -> bool {
        let Some(selection) = self.selection() else {
            return false;
        };

        self.copy_region(selection, mode)
    }

    pub fn copy_tile(&mut self, coord: Coord) -> bool {
        self.copy_region(Selection::from_drag(coord, coord), RegionSelectionMode::Full)
    }

    fn copy_region(&mut self, selection: Selection, mode: RegionSelectionMode) -> bool {
        let Some(region) = self.read_region(selection, mode) else {
            return false;
        };

        self.state.set_clipboard(region);

        true
    }

    fn read_instances(&self, ids: &[PrefabInstanceId]) -> Option<TileRegion> {
        self.state
            .active()
            .and_then(|active| self.effective_grid(active))
            .and_then(|map| clipboard::copy_instances(map, ids))
    }

    pub fn copy_picked(&mut self) -> bool {
        let Some(region) = self.read_instances(&self.picked_instances()) else {
            return false;
        };

        self.state.set_clipboard(region);

        true
    }

    pub fn cut_picked(&mut self) -> bool {
        let Some(document) = self.state.active_document() else {
            return false;
        };
        let targets = self
            .picked_instances()
            .into_iter()
            .filter(|id| document.allows_edit_instance(*id))
            .collect::<Vec<_>>();
        let Some(region) = self.read_instances(&targets) else {
            return false;
        };

        let Some(action) = self.build_delete_instances(&targets, "cut") else {
            return false;
        };

        if !self.commit(action, None) {
            return false;
        }

        self.state.set_clipboard(region);

        true
    }

    pub fn can_clear_tile(&self, coord: Coord) -> bool {
        let Some((environment, document)) = self.state.active_pair() else {
            return false;
        };

        document.allows_edit_at(coord)
            && document
                .map
                .tile_at(coord)
                .is_some_and(|tile| clipboard::can_clear(&environment.world, tile, &self.hidden_types()))
    }

    pub fn can_clear_selection(&self) -> bool {
        let Some(selection) = self.selection() else {
            return false;
        };
        let Some((environment, document)) = self.state.active_pair() else {
            return false;
        };

        let mode = self.selection_mode();
        let hidden = self.hidden_types();

        mode.tiles(selection).all(|coord| document.allows_edit_at(coord))
            && mode.tiles(selection).any(|coord| {
                document
                    .map
                    .tile_at(coord)
                    .is_some_and(|tile| clipboard::can_clear(&environment.world, tile, &hidden))
            })
    }

    fn build_clear(&mut self, selection: Selection, mode: RegionSelectionMode, label: &str) -> Option<ToolEdit> {
        let hidden = self.hidden_types();
        let (environment, document) = self.state.active_pair_mut()?;

        clipboard::clear_region(
            document,
            &environment.tree,
            &environment.world,
            selection,
            mode,
            &hidden,
            label,
        )
    }

    pub(crate) fn take_region(
        &mut self, selection: Selection, mode: RegionSelectionMode, label: &str,
    ) -> Option<TileRegion> {
        let region = self.read_region(selection, mode)?;
        let action = self.build_clear(selection, mode, label)?;

        if !self.commit(action, None) {
            return None;
        }

        Some(region)
    }

    fn cut_region(&mut self, selection: Selection, mode: RegionSelectionMode, label: &str) -> bool {
        let Some(region) = self.take_region(selection, mode, label) else {
            return false;
        };

        self.state.set_clipboard(region);

        true
    }

    pub fn delete_tile(&mut self, coord: Coord, group: Option<EditGroupId>) -> bool {
        self.build_clear(
            Selection::from_drag(coord, coord),
            RegionSelectionMode::Full,
            "delete tile",
        )
        .is_some_and(|action| self.commit(action, group))
    }

    pub fn cut_tile(&mut self, coord: Coord) -> bool {
        self.cut_region(
            Selection::from_drag(coord, coord),
            RegionSelectionMode::Full,
            "cut tile",
        )
    }

    pub fn delete_selection(&mut self) -> bool {
        let Some(selection) = self.selection() else {
            return false;
        };

        self.build_clear(selection, self.selection_mode(), "delete region")
            .is_some_and(|action| self.commit(action, None))
    }

    pub fn cut_selection(&mut self) -> bool {
        let Some(selection) = self.selection() else {
            return false;
        };

        self.cut_region(selection, self.selection_mode(), "cut region")
    }

    pub fn clipboard(&self) -> Option<&TileRegion> {
        self.state.clipboard()
    }

    pub fn can_paste_clipboard(&self, min: Coord, rotation: SelectionRotation) -> bool {
        let Some(region) = self.state.clipboard() else {
            return false;
        };
        let Some(document) = self.state.active_document() else {
            return false;
        };

        clipboard::can_paste(document, region, min, rotation)
    }

    pub fn paste_clipboard(&mut self, min: Coord, rotation: SelectionRotation) -> bool {
        let Some(region) = self.state.clipboard().cloned() else {
            return false;
        };

        if region.merge() == PasteMerge::Stack {
            return self.paste_stacked(&region, min, rotation);
        }

        let Some(floating) = self.paste_floating(region, min, rotation) else {
            return false;
        };

        self.finish_live_move();
        self.region_state = RegionState::Floating(floating);

        true
    }

    fn paste_stacked(&mut self, region: &TileRegion, min: Coord, rotation: SelectionRotation) -> bool {
        let built = {
            let Some((environment, document)) = self.state.active_pair_mut() else {
                return false;
            };

            clipboard::paste_region(document, &environment.tree, region, min, rotation)
        };
        let Some(pasted) = built else {
            return false;
        };

        if !self.commit(pasted.action, None) {
            return false;
        }

        self.finish_live_move();
        self.set_tool(Tool::Pick);
        self.pick_instances(&pasted.placed);

        true
    }

    fn paste_floating(&mut self, region: TileRegion, min: Coord, rotation: SelectionRotation) -> Option<FloatingPaste> {
        let built = {
            let (environment, document) = self.state.active_pair_mut()?;

            clipboard::paste_region(document, &environment.tree, &region, min, rotation)
        };

        let (Some(pasted), Some(document)) = (built, self.state.active()) else {
            return None;
        };

        let group = EditGroupId::new();
        let target = SelectionMask {
            bounds: pasted.target,
            mode: region.selection_mode(),
        };
        if !self.commit_selecting(pasted.action, target, Some(group)) {
            return None;
        }

        Some(FloatingPaste {
            document,
            group,
            region,
            rotation,
            target,
        })
    }

    pub(crate) fn is_floating_live(&self, floating: &FloatingPaste) -> bool {
        let Some(document) = self.state.active_document() else {
            return false;
        };

        document.id() == floating.document
            && document.history.undo_group() == Some(floating.group)
            && self.selection_mask() == Some(floating.target)
    }

    pub(crate) fn live_floating_paste(&self) -> Option<&FloatingPaste> {
        let floating = self.region_state.floating()?;

        self.is_floating_live(floating).then_some(floating)
    }

    /// This function returns paste depending on the paste result:
    /// Old paste on err (aka paste failed), new paste on success (aka new paste returned from paste_floating)
    #[allow(clippy::result_large_err)]
    pub(crate) fn repaste_floating(
        &mut self, floating: FloatingPaste, min: Coord, rotation: SelectionRotation,
    ) -> Result<FloatingPaste, FloatingPaste> {
        let Some(document) = self.state.active_document() else {
            return Err(floating);
        };

        if !self.is_floating_live(&floating) || !clipboard::can_paste(document, &floating.region, min, rotation) {
            return Err(floating);
        }

        if !self.undo() {
            return Err(floating);
        }

        // TODO: This deep copies entire region, get rid of it
        let region = floating.region.clone();
        if let Some(new_floating) = self.paste_floating(region, min, rotation) {
            return Ok(new_floating);
        }

        self.redo();

        Err(floating)
    }

    pub(crate) fn store_pasted_floating(
        &mut self, floating: FloatingPaste, min: Coord, rotation: SelectionRotation,
    ) -> bool {
        let paste = self.repaste_floating(floating, min, rotation);
        let is_pasted = paste.is_ok();
        let (Ok(floating) | Err(floating)) = paste;
        self.region_state.put_floating(floating);

        is_pasted
    }
}

#[cfg(test)]
mod tests;
