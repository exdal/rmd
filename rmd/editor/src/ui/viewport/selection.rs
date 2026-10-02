use dear_imgui_rs::Ui;
use dmm::Coord;
use editor::{
    conflict::{Region, Side},
    document::{DocumentId, Selection},
    tool::{SelectionMask, SelectionPlacement, SelectionRotation, Tool, rotated_selection_at},
};

use super::ViewFrame;
use crate::{
    gizmo::{BlockGizmoKind, BlockGizmoTarget, GizmoMapView},
    session::{BlockPreviewSource, Session},
    settings::Settings,
    ui::{
        BlockPlacementAction,
        PasteAction,
        PendingBlockPlacement,
        PendingPaste,
        PlacementControls,
        RectangleGesture,
        UiState,
        block_controls_placement,
        block_placement_controls_layout,
        conflict_controls_layout,
        draw_block_outline,
        draw_block_placement_controls,
        draw_conflict_controls,
        draw_paste_controls,
        paste_controls,
        rectangle_drag_coord,
        restore_rectangle_gesture,
    },
};

#[derive(Debug, Default)]
pub(in crate::ui) struct ViewGestures {
    pub(super) block_selection_anchor: Option<Coord>,
    pub(super) rectangle_gesture: Option<RectangleGesture>,
    pub(super) block_placement: Option<PendingBlockPlacement>,
    pub(super) paste: Option<PendingPaste>,
}

impl ViewGestures {
    pub(in crate::ui) fn cancel(&mut self, session: &mut Session, id: DocumentId) {
        restore_rectangle_gesture(session, id, &mut self.rectangle_gesture);
        self.block_selection_anchor = None;
        self.block_placement = None;
        self.paste = None;
    }

    pub(super) const fn is_busy(&self) -> bool {
        self.rectangle_gesture.is_some() || self.block_placement.is_some() || self.paste.is_some()
    }

    pub(super) fn displayed(&self, source: Selection) -> (Selection, SelectionRotation) {
        self.block_placement
            .map_or((source, SelectionRotation::Original), |placement| {
                (placement.target, placement.rotation)
            })
    }
}

pub(super) struct BlockActions {
    paste: Option<(PasteAction, PendingPaste)>,
    placement: Option<(BlockPlacementAction, PendingBlockPlacement)>,
}

pub(super) fn shows_block_selection(tool: Tool) -> bool { matches!(tool, Tool::BlockSelect | Tool::Fill) }

pub(super) fn sync_gestures(session: &Session, gestures: &mut ViewGestures, tool: Tool) -> Option<SelectionMask> {
    if tool != Tool::BlockSelect {
        gestures.block_selection_anchor = None;
        gestures.block_placement = None;
        gestures.paste = None;
    } else {
        if gestures
            .block_selection_anchor
            .is_some_and(|anchor| anchor.z != session.z())
        {
            gestures.block_selection_anchor = None;
        }

        if gestures
            .block_placement
            .is_some_and(|placement| Some(placement.source) != session.selection())
        {
            gestures.block_placement = None;
        }

        if let Some(pending) = gestures.paste.as_mut() {
            pending.min.z = session.z();
        }
    }

    let paste_mask = gestures
        .paste
        .and_then(|pending| session.clipboard_selection_mask(pending.min, pending.rotation));
    if gestures.paste.is_some() && paste_mask.is_none() {
        gestures.paste = None;
    }

    paste_mask
}

pub(super) fn block_controls_contain(
    ui: &Ui, session: &Session, frame: &ViewFrame<'_>, tool: Tool, paste_target: Option<Selection>,
    is_rotation_open: bool,
) -> bool {
    let hit = match frame.gestures.paste {
        Some(_) => paste_controls(is_rotation_open, paste_target).map(|target| (PlacementControls::Paste, target)),
        None => block_controls_placement(
            tool,
            frame.gestures.rectangle_gesture.is_some(),
            is_rotation_open,
            session.selection(),
            frame.gestures.block_placement,
        )
        .map(|placement| (PlacementControls::Block, placement.target)),
    };

    hit.is_some_and(|(controls, target)| {
        let (_, bounds) = block_placement_controls_layout(
            ui,
            frame.camera,
            controls,
            target,
            session.options.tile_size,
            frame.layout.viewport.min,
            frame.layout.controls_area(),
        );

        bounds.contains(frame.mouse)
    })
}

pub(super) fn conflict_controls_contain(ui: &Ui, session: &Session, frame: &ViewFrame<'_>, regions: &[Region]) -> bool {
    let theirs_label = session
        .git_state(frame.id)
        .and_then(|git| git.conflicts.as_ref())
        .map_or(String::from("theirs"), |conflicts| {
            conflicts.side_label(Side::Theirs).to_owned()
        });

    regions.iter().any(|region| {
        conflict_controls_layout(
            ui,
            frame.camera,
            region,
            session.options.tile_size,
            frame.layout.viewport.min,
            frame.layout.controls_area(),
            &theirs_label,
        )
        .is_some_and(|(_, bounds)| bounds.contains(frame.mouse))
    })
}

pub(super) fn prepare_block_preview(session: &mut Session, frame: &ViewFrame<'_>) {
    if let Some(pending) = frame.gestures.paste {
        session.prepare_block_preview(frame.id, BlockPreviewSource::Clipboard, pending.min, pending.rotation);
    } else if let Some(placement) = frame.gestures.block_placement
        && session.tool() == Tool::BlockSelect
    {
        session.prepare_block_preview(
            frame.id,
            BlockPreviewSource::Selection(SelectionMask {
                bounds: placement.source,
                mode: session.selection_mode(),
            }),
            placement.target.min,
            placement.rotation,
        );
    }
}

impl UiState {
    pub(super) fn draw_block_gizmo(
        &mut self, ui: &Ui, session: &mut Session, settings: &Settings, frame: &mut ViewFrame<'_>,
        paste_target: Option<Selection>, gizmo_view: GizmoMapView,
    ) -> bool {
        let gestures = &mut *frame.gestures;
        let map_size = session.map().map(|map| map.size);
        if let (Some(pending), Some(target), Some(size)) = (gestures.paste, paste_target, map_size) {
            let response = self.gizmo.draw_block(
                ui,
                settings,
                frame.camera,
                BlockGizmoTarget {
                    selection: target,
                    rotation: pending.rotation,
                    map_size: size,
                    tile_size: session.options.tile_size,
                    kind: BlockGizmoKind::Clipboard,
                },
                gizmo_view,
            );

            if response.selection.min != target.min || response.rotation != pending.rotation {
                gestures.paste = Some(PendingPaste {
                    min: response.selection.min,
                    rotation: response.rotation,
                });
            }

            return response.captures_mouse;
        }

        if gestures.block_selection_anchor.is_some() {
            self.gizmo.cancel();

            return false;
        }

        let (Some(source), Some(size)) = (session.selection(), map_size) else {
            self.gizmo.cancel();

            return false;
        };

        let (displayed, rotation) = gestures.displayed(source);
        let response = self.gizmo.draw_block(
            ui,
            settings,
            frame.camera,
            BlockGizmoTarget {
                selection: displayed,
                rotation,
                map_size: size,
                tile_size: session.options.tile_size,
                kind: if gestures.block_placement.is_some() {
                    BlockGizmoKind::Placement
                } else {
                    BlockGizmoKind::Selection
                },
            },
            gizmo_view,
        );

        if response.resizing {
            gestures.rectangle_gesture.get_or_insert(RectangleGesture {
                start: session.selection_mask(),
                z: session.z(),
            });
            session.try_select_block(SelectionMask {
                bounds: response.selection,
                mode: session.selection_mode(),
            });
        } else if response.selection != displayed || response.rotation != rotation {
            gestures.block_selection_anchor = None;
            gestures.block_placement = rotated_selection_at(source, response.selection.min, response.rotation)
                .filter(|target| *target != source || response.rotation != SelectionRotation::Original)
                .map(|target| PendingBlockPlacement {
                    source,
                    target,
                    rotation: response.rotation,
                });
        }

        response.captures_mouse
    }

    pub(super) fn start_rectangle(
        &self, ui: &Ui, session: &mut Session, frame: &mut ViewFrame<'_>, coord: Coord, is_left_clicked: bool,
    ) {
        let gestures = &mut *frame.gestures;
        if gestures.block_placement.is_some()
            || gestures.paste.is_some()
            || !is_left_clicked
            || !session.can_edit_at(coord)
        {
            return;
        }

        gestures.rectangle_gesture = Some(RectangleGesture {
            start: session.selection_mask(),
            z: session.z(),
        });
        if session.try_select_block(SelectionMask {
            bounds: Selection::from_drag(coord, coord),
            mode: self.block_selection_options.drawing_mode(ui.io().key_shift()),
        }) {
            gestures.block_selection_anchor = Some(coord);
        }
    }

    pub(super) fn drag_rectangle(&self, ui: &Ui, session: &mut Session, frame: &mut ViewFrame<'_>, is_left_down: bool) {
        if let Some(anchor) = frame.gestures.block_selection_anchor
            && let Some(size) = session.map().map(|map| map.size)
            && let Some(coord) = rectangle_drag_coord(
                frame.camera,
                frame.mouse,
                frame.layout.viewport.min,
                size,
                session.options.tile_size,
                anchor.z,
            )
        {
            session.try_select_block(SelectionMask {
                bounds: Selection::from_drag(anchor, coord),
                mode: if is_left_down {
                    self.block_selection_options.drawing_mode(ui.io().key_shift())
                } else {
                    session.selection_mode()
                },
            });
        }

        if !is_left_down {
            frame.gestures.block_selection_anchor = None;
            frame.gestures.rectangle_gesture = None;
        }
    }

    pub(super) fn draw_block_overlays(
        &mut self, ui: &Ui, session: &Session, frame: &ViewFrame<'_>, tool: Tool, paste_mask: Option<SelectionMask>,
        gizmo_view: GizmoMapView,
    ) {
        let viewport = frame.layout.viewport;
        let block_overlay = match (frame.gestures.paste, paste_mask) {
            (Some(pending), Some(mask)) => {
                draw_block_outline(ui, session, frame.camera, mask.bounds, mask.mode, viewport);

                Some((mask.bounds, pending.rotation))
            },
            _ if shows_block_selection(session.tool()) => session.selection().map(|source| {
                let (displayed, rotation) = frame.gestures.displayed(source);
                draw_block_outline(ui, session, frame.camera, displayed, session.selection_mode(), viewport);

                (displayed, rotation)
            }),
            _ => None,
        };

        if let Some((displayed, rotation)) = block_overlay
            && tool == Tool::BlockSelect
            && frame.gestures.block_selection_anchor.is_none()
            && let Some(map_size) = session.map().map(|map| map.size)
        {
            self.gizmo.draw_block_overlay(
                ui,
                frame.camera,
                BlockGizmoTarget {
                    selection: displayed,
                    rotation,
                    map_size,
                    tile_size: session.options.tile_size,
                    kind: if frame.gestures.paste.is_some() {
                        BlockGizmoKind::Clipboard
                    } else if frame.gestures.block_placement.is_some() {
                        BlockGizmoKind::Placement
                    } else {
                        BlockGizmoKind::Selection
                    },
                },
                gizmo_view,
            );
        }
    }

    pub(super) fn draw_block_controls(
        &mut self, ui: &Ui, session: &mut Session, frame: &ViewFrame<'_>, tool: Tool, paste_target: Option<Selection>,
        conflict_regions: &[Region],
    ) -> BlockActions {
        let viewport_min = frame.layout.viewport.min;
        let controls_area = frame.layout.controls_area();
        let is_rotation_open = self.gizmo.block_rotation_open();
        let paste = frame
            .gestures
            .paste
            .zip(paste_controls(is_rotation_open, paste_target))
            .and_then(|(pending, target)| {
                let can_paste = session.can_paste_clipboard(pending.min, pending.rotation);

                draw_paste_controls(
                    ui,
                    frame.camera,
                    target,
                    session.options.tile_size,
                    viewport_min,
                    controls_area,
                    can_paste,
                )
                .map(|action| (action, pending))
            });
        draw_conflict_controls(
            ui,
            session,
            frame.id,
            frame.camera,
            conflict_regions,
            viewport_min,
            controls_area,
        );

        let placement = frame
            .gestures
            .paste
            .is_none()
            .then(|| {
                block_controls_placement(
                    tool,
                    frame.gestures.rectangle_gesture.is_some(),
                    is_rotation_open,
                    session.selection(),
                    frame.gestures.block_placement,
                )
            })
            .flatten()
            .and_then(|placement| {
                draw_block_placement_controls(
                    ui,
                    frame.camera,
                    placement,
                    viewport_min,
                    controls_area,
                    session,
                    &mut self.block_selection_options,
                )
                .map(|action| (action, placement))
            });

        BlockActions { paste, placement }
    }

    pub(super) fn apply_block_actions(
        &mut self, session: &mut Session, gestures: &mut ViewGestures, actions: BlockActions,
    ) {
        if let Some((action, pending)) = actions.paste {
            if action == PasteAction::Paste {
                session.paste_clipboard(pending.min, pending.rotation);
            }

            gestures.paste = None;
            self.gizmo.cancel();
        }

        let Some((action, placement)) = actions.placement else {
            return;
        };

        let is_finished = match action {
            BlockPlacementAction::Move => session.place_selected_block_with_mode(
                placement.target.min,
                placement.rotation,
                SelectionPlacement::Move,
                session.selection_mode(),
            ),
            BlockPlacementAction::Copy => session.place_selected_block_with_mode(
                placement.target.min,
                placement.rotation,
                SelectionPlacement::Copy,
                session.selection_mode(),
            ),
            BlockPlacementAction::Fill => {
                session.fill_selected_block(placement.target.min, placement.rotation, session.selection_mode())
            },
            BlockPlacementAction::Cancel => {
                if gestures.block_placement.is_none() {
                    session.select_block(None);
                }

                true
            },
        };
        if is_finished {
            gestures.block_placement = None;
            self.gizmo.cancel();
        }
    }
}
