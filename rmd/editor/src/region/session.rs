use std::mem;

use dmm::Coord;

use crate::{clipboard::session::FloatingPaste, session::Session};
use crate::{
    document::{DocumentId, PrefabInstanceId, command::EditGroupId, selection::Selection},
    tool::{
        RegionSelectionMode, SelectionMask, SelectionPlacement, SelectionRotation, SelectionTransform, Tool,
        place::is_placeable,
        selection::{
            fill_selection as build_selection_fill, place_selection_with_mode as build_selection_placement,
            transform_selection_with_mode as build_selection_transform,
        },
        transform::{rotated_selection_at, transformed_selection},
    },
};

#[derive(Debug)]
pub(crate) struct PendingMove {
    document: DocumentId,
    source: SelectionMask,
    state: PendingMoveState,
}

#[derive(Default)]
pub(crate) enum RegionState {
    #[default]
    Idle,
    Floating(FloatingPaste),
    Moving(PendingMove),
}

// god have a mercy
impl RegionState {
    pub(crate) fn floating(&self) -> Option<&FloatingPaste> {
        match self {
            RegionState::Floating(floating)
            | RegionState::Moving(PendingMove {
                state: PendingMoveState::Floating(floating),
                ..
            }) => Some(floating),
            _ => None,
        }
    }

    pub(crate) fn take_floating(&mut self) -> Option<FloatingPaste> {
        match mem::take(self) {
            RegionState::Floating(floating) => Some(floating),
            RegionState::Moving(mut pending) => {
                let floating = match mem::replace(&mut pending.state, PendingMoveState::Unmoved) {
                    PendingMoveState::Floating(floating_paste) => Some(floating_paste),
                    state => {
                        pending.state = state;

                        None
                    },
                };

                *self = RegionState::Moving(pending);

                floating
            },
            other => {
                *self = other;

                None
            },
        }
    }

    pub(crate) fn put_floating(&mut self, floating: FloatingPaste) {
        match self {
            RegionState::Moving(pending) => pending.state = PendingMoveState::Floating(floating),
            RegionState::Idle | RegionState::Floating(_) => *self = RegionState::Floating(floating),
        }
    }
}

#[derive(Debug)]
enum PendingMoveState {
    Unmoved,
    Moved { group: EditGroupId, target: SelectionMask },
    // a fresh paste follows by pasting again, which puts back what it covered
    Floating(FloatingPaste),
}

impl Session {
    fn live_pending_move(&self) -> Option<&PendingMove> {
        let RegionState::Moving(pending) = &self.region_state else {
            // we only care about moving state
            return None;
        };

        let document = self.state.active_document()?;
        if document.id() != pending.document {
            // handled in different doc
            return None;
        }

        let is_live = match &pending.state {
            PendingMoveState::Unmoved => self.selection_mask() == Some(pending.source),
            PendingMoveState::Moved { group, target } => {
                document.history.undo_group() == Some(*group) && self.selection_mask() == Some(*target)
            },
            PendingMoveState::Floating(floating) => self.is_floating_live(floating),
        };

        is_live.then_some(pending)
    }

    pub(crate) fn pending_move_source(&self) -> Option<Selection> {
        self.live_pending_move().map(|pending| pending.source.bounds)
    }

    pub fn move_selected_region_live(&mut self, target_min: Coord) -> bool {
        if !matches!(self.region_state, RegionState::Moving(_)) {
            // force current region state into pending move

            let (Some(document), Some(source)) = (self.state.active(), self.selection_mask()) else {
                return false;
            };

            let state = self
                .live_floating_paste()
                .is_some()
                .then(|| self.region_state.take_floating())
                .flatten()
                .map_or(PendingMoveState::Unmoved, PendingMoveState::Floating);

            self.region_state = RegionState::Moving(PendingMove {
                document,
                source,
                state,
            });
            self.hold_bake(document);
        }

        let Some(pending) = self.live_pending_move() else {
            return false;
        };

        let (document, source) = (pending.document, pending.source);
        if self.selection().is_some_and(|selection| selection.min == target_min) {
            return true;
        }

        match pending.state {
            PendingMoveState::Unmoved => self.commit_pending_move(document, source, target_min, &[]),
            PendingMoveState::Moved { group, .. } => {
                let is_back = target_min == pending.source.bounds.min;
                if !is_back
                    && !self.can_place_region_with_mode(
                        pending.source.bounds,
                        target_min,
                        SelectionRotation::Original,
                        pending.source.mode,
                    )
                {
                    return false;
                }

                // moving from the untouched map puts back whatever the region passed over
                let Some(retracted) = self
                    .state
                    .active_document_mut()
                    .and_then(|document| document.retract_with_affected(group))
                else {
                    return false;
                };

                if !is_back && self.commit_pending_move(document, source, target_min, &retracted) {
                    return true;
                }

                self.region_state = RegionState::Moving(PendingMove {
                    document,
                    source,
                    state: PendingMoveState::Unmoved,
                });
                self.settle_retracted(&retracted);

                is_back
            },
            PendingMoveState::Floating(_) => {
                let Some(rotation) = self.live_floating_paste().map(|floating| floating.rotation) else {
                    return false;
                };

                if !self.can_place_selected_region_with_mode(
                    target_min,
                    SelectionRotation::Original,
                    SelectionPlacement::Move,
                    pending.source.mode,
                ) {
                    return false;
                }

                let Some(floating) = self.region_state.take_floating() else {
                    return false;
                };

                self.store_pasted_floating(floating, target_min, rotation)
            },
        }
    }

    fn commit_pending_move(
        &mut self, document: DocumentId, source: SelectionMask, target_min: Coord, retracted: &[PrefabInstanceId],
    ) -> bool {
        let mode = source.mode;
        if !self.can_place_selected_region_with_mode(
            target_min,
            SelectionRotation::Original,
            SelectionPlacement::Move,
            mode,
        ) {
            return false;
        }

        let group = EditGroupId::new();
        if !self.commit_region_placement(
            target_min,
            SelectionRotation::Original,
            SelectionPlacement::Move,
            mode,
            Some(group),
            retracted,
        ) {
            return false;
        }

        let Some(target) = self.selection_mask() else {
            return false;
        };

        self.region_state = RegionState::Moving(PendingMove {
            document,
            source,
            state: PendingMoveState::Moved { group, target },
        });

        true
    }

    pub(crate) fn settle_retracted(&mut self, retracted: &[PrefabInstanceId]) {
        self.update_instances(retracted);
        if let Some(id) = self.state.active()
            && let Some(cache) = self.caches.get_mut(&id)
        {
            cache.map_revision = cache.map_revision.wrapping_add(1);
        }
    }

    pub fn finish_live_move(&mut self) {
        match mem::take(&mut self.region_state) {
            RegionState::Moving(pending_move) => {
                self.release_bake(pending_move.document);

                if let PendingMoveState::Floating(floating_paste) = pending_move.state {
                    self.region_state = RegionState::Floating(floating_paste)
                }
            },
            state => {
                self.region_state = state;
            },
        }
    }

    pub fn cancel_live_move(&mut self) {
        if let Some(pending) = self.live_pending_move() {
            let source = pending.source;
            match pending.state {
                PendingMoveState::Unmoved => {},
                PendingMoveState::Moved { group, .. } => {
                    if let Some(retracted) = self
                        .state
                        .active_document_mut()
                        .and_then(|document| document.retract_with_affected(group))
                    {
                        self.settle_retracted(&retracted);
                    }
                },
                PendingMoveState::Floating(_) => {
                    if let Some(floating) = self.live_floating_paste()
                        && floating.target.bounds.min != source.bounds.min
                        && let Some(floating) = self.region_state.take_floating()
                    {
                        let rotation = floating.rotation;
                        self.store_pasted_floating(floating, source.bounds.min, rotation);
                    }
                },
            }
        }

        self.finish_live_move();
    }

    pub fn selection(&self) -> Option<Selection> {
        self.selection_mask().map(|mask| mask.bounds)
    }

    pub fn selection_mask(&self) -> Option<SelectionMask> {
        self.state.active_document()?.selection_mask()
    }

    pub fn selection_mode(&self) -> RegionSelectionMode {
        self.selection_mask()
            .map_or(RegionSelectionMode::Full, |mask| mask.mode)
    }

    pub(crate) fn edit_revision(&self) -> Option<u64> {
        Some(self.caches.get(&self.state.active()?)?.revision)
    }

    pub fn can_select_region(&self, mask: SelectionMask) -> bool {
        self.state.active_document().is_some_and(|document| {
            mask.bounds.is_well_formed()
                && mask.bounds.min.z == document.z
                && mask.bounds.max.x <= document.map.size().x
                && mask.bounds.max.y <= document.map.size().y
                && mask
                    .mode
                    .tiles(mask.bounds)
                    .all(|coord| document.allows_select_at(coord))
        })
    }

    pub fn try_select_region(&mut self, mask: SelectionMask) -> bool {
        self.can_select_region(mask) && self.select_region_with_mode(Some(mask.bounds), mask.mode)
    }

    pub fn select_region(&mut self, selection: Option<Selection>) -> bool {
        self.select_region_with_mode(selection, RegionSelectionMode::Full)
    }

    pub fn select_region_with_mode(
        &mut self, selection: Option<Selection>, selection_mode: RegionSelectionMode,
    ) -> bool {
        let Some(document) = self.state.active_document_mut() else {
            return false;
        };

        let selection = selection.filter(|selection| {
            selection.is_well_formed()
                && selection.min.z == document.z
                && selection.max.x <= document.map.size().x
                && selection.max.y <= document.map.size().y
                && selection_mode
                    .tiles(*selection)
                    .all(|coord| document.allows_select_at(coord))
        });
        document.set_selection(selection, selection_mode);
        document.pick_instance(None);
        self.finish_live_move();
        self.region_state = RegionState::Idle;

        selection.is_some()
    }

    pub fn place_selected_region_with_mode(
        &mut self, target_min: Coord, rotation: SelectionRotation, placement: SelectionPlacement,
        selection_mode: RegionSelectionMode,
    ) -> bool {
        if !self.can_place_selected_region_with_mode(target_min, rotation, placement, selection_mode) {
            return false;
        }

        if placement == SelectionPlacement::Move
            && rotation == SelectionRotation::Original
            && let Some(floating) = self.live_floating_paste()
            && floating.target.mode == selection_mode
            && let Some(floating) = self.region_state.take_floating()
        {
            let rotation = floating.rotation;

            return self.store_pasted_floating(floating, target_min, rotation);
        }

        self.commit_region_placement(target_min, rotation, placement, selection_mode, None, &[])
    }

    fn commit_region_placement(
        &mut self, target_min: Coord, rotation: SelectionRotation, placement: SelectionPlacement,
        selection_mode: RegionSelectionMode, group: Option<EditGroupId>, retracted: &[PrefabInstanceId],
    ) -> bool {
        let hidden = self.hidden_types();
        let built = {
            let Some((environment, document)) = self.state.active_pair_mut() else {
                return false;
            };
            let Some(selection) = document.selection() else {
                return false;
            };

            build_selection_placement(
                document,
                &environment.tree,
                &environment.world,
                *selection,
                target_min,
                rotation,
                placement,
                selection_mode,
                &hidden,
            )
        };

        let Some((mut action, selection)) = built else {
            return false;
        };

        if !retracted.is_empty() {
            action.affected.extend_from_slice(retracted);
            action.affected.sort_unstable();
            action.affected.dedup();
        }

        self.commit_selecting(
            action,
            SelectionMask {
                bounds: selection,
                mode: selection_mode,
            },
            group,
        )
    }

    pub fn can_place_selected_region_with_mode(
        &self, target_min: Coord, rotation: SelectionRotation, _placement: SelectionPlacement,
        mode: RegionSelectionMode,
    ) -> bool {
        self.selection()
            .is_some_and(|selection| self.can_place_region_with_mode(selection, target_min, rotation, mode))
    }

    fn can_place_region_with_mode(
        &self, selection: Selection, target_min: Coord, rotation: SelectionRotation, mode: RegionSelectionMode,
    ) -> bool {
        if self.tool() != Tool::Select {
            return false;
        }

        let Some(environment) = self.state.environment.as_ref() else {
            return false;
        };

        let roots = environment.tree.roots();
        if roots.turf.is_none() || roots.area.is_none() {
            return false;
        }

        let Some(document) = self.state.active_document() else {
            return false;
        };

        let Some(target) = rotated_selection_at(selection, target_min, rotation) else {
            return false;
        };

        if target == selection && rotation == SelectionRotation::Original {
            // why do you want to select and place? get help
            return false;
        }

        target.max.x <= document.map.size().x
            && target.max.y <= document.map.size().y
            && mode
                .tiles(selection)
                .chain(mode.tiles(target))
                .all(|coord| document.allows_edit_at(coord))
    }

    pub fn fill_rect(&mut self, mask: SelectionMask) -> bool {
        if !self.can_fill_rect(mask) {
            return false;
        }

        let Some(prefab) = self.state.palette.clone() else {
            return false;
        };
        let built = {
            let Some((environment, document)) = self.state.active_pair_mut() else {
                return false;
            };

            build_selection_fill(document, &environment.tree, mask.bounds, &prefab, mask.mode)
        };

        let Some(action) = built else {
            return false;
        };

        if !self.commit(action, None) {
            return false;
        }

        self.state.choose_prefab(prefab);

        true
    }

    pub fn can_fill_rect(&self, mask: SelectionMask) -> bool {
        let Some(environment) = self.state.environment.as_ref() else {
            return false;
        };
        let Some(prefab) = self.state.palette.as_ref() else {
            return false;
        };
        let Some(prefab_id) = environment.tree.id_of(&prefab.path) else {
            return false;
        };

        is_placeable(&environment.tree, prefab_id) && self.can_select_region(mask)
    }

    pub fn rotate_selected_region(&mut self) -> bool {
        self.transform_selected_region_with_mode(SelectionTransform::RotateClockwise, self.selection_mode())
    }

    pub fn transform_selected_region_with_mode(
        &mut self, transform: SelectionTransform, selection_mode: RegionSelectionMode,
    ) -> bool {
        if self.tool() != Tool::Select || !self.can_transform_selected_region_with_mode(transform, selection_mode) {
            return false;
        }

        if let Some(floating) = self.live_floating_paste()
            && floating.target.mode == selection_mode
        {
            let rotation = match transform {
                SelectionTransform::RotateClockwise => Some(floating.rotation.clockwise()),
                SelectionTransform::RotateCounterClockwise => Some(floating.rotation.counter_clockwise()),
                SelectionTransform::MirrorHorizontal | SelectionTransform::MirrorVertical => None,
            };
            if let Some(rotation) = rotation
                && let Some(floating) = self.region_state.take_floating()
            {
                let min = floating.target.bounds.min;

                return self.store_pasted_floating(floating, min, rotation);
            }
        }

        let hidden = self.hidden_types();
        let built = {
            let Some((environment, document)) = self.state.active_pair_mut() else {
                return false;
            };
            let Some(selection) = document.selection().copied() else {
                return false;
            };

            build_selection_transform(
                document,
                &environment.tree,
                &environment.world,
                selection,
                transform,
                selection_mode,
                &hidden,
            )
        };

        let Some((action, selection)) = built else {
            return false;
        };

        self.commit_selecting(
            action,
            SelectionMask {
                bounds: selection,
                mode: selection_mode,
            },
            None,
        )
    }

    pub fn can_transform_selected_region_with_mode(
        &self, transform: SelectionTransform, mode: RegionSelectionMode,
    ) -> bool {
        // holy fuck we need a better solution to this, just copy pasting  same shit over and over

        if self.tool() != Tool::Select {
            return false;
        }

        let Some(environment) = self.state.environment.as_ref() else {
            return false;
        };

        let roots = environment.tree.roots();
        if roots.turf.is_none() || roots.area.is_none() {
            return false;
        }

        let Some(document) = self.state.active_document() else {
            return false;
        };

        let Some(selection) = self.selection() else {
            return false;
        };

        let Some(target) = transformed_selection(selection, transform) else {
            return false;
        };

        target.max.x <= document.map.size().x
            && target.max.y <= document.map.size().y
            && mode
                .tiles(selection)
                .chain(mode.tiles(target))
                .all(|coord| document.allows_edit_at(coord))
    }
}

#[cfg(test)]
mod tests;
