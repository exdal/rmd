use dear_imgui_rs::Ui;
use dmm::Coord;
use editor::{
    conflict::{Region, Side},
    document::{DocumentId, Selection},
    maploader::preview::ModuleRootKey,
    tool::{SelectionMask, SelectionPlacement, SelectionRotation, Tool},
};

use super::ViewFrame;
use crate::{
    gizmo::{BlockDrag, BlockGizmoTarget, GizmoMapView},
    pacing::FrameDemand,
    session::{BlockPreviewSource, Session},
    ui::{
        PendingBlockPlacement,
        RectangleGesture,
        UiState,
        conflict_controls_layout,
        draw_block_outline,
        draw_conflict_controls,
        rectangle_drag_coord,
        restore_rectangle_gesture,
    },
};

#[derive(Debug, Default)]
pub(in crate::ui) struct ViewGestures {
    pub(in crate::ui) module_popup: Option<(ModuleRootKey, Coord)>,
    pub(super) block_selection_anchor: Option<Coord>,
    pub(super) rectangle_gesture: Option<RectangleGesture>,
    pub(super) block_placement: Option<PendingBlockPlacement>,
    pub(super) fill_anchor: Option<Coord>,
}

impl ViewGestures {
    pub(in crate::ui) fn cancel(&mut self, session: &mut Session, id: DocumentId) {
        restore_rectangle_gesture(session, id, &mut self.rectangle_gesture);
        self.module_popup = None;
        self.block_selection_anchor = None;
        self.block_placement = None;
        self.fill_anchor = None;
    }

    pub(super) const fn is_busy(&self) -> bool {
        self.rectangle_gesture.is_some() || self.block_placement.is_some() || self.fill_anchor.is_some()
    }

    pub(super) fn displayed(&self, source: Selection) -> Selection {
        self.block_placement.map_or(source, |placement| placement.target)
    }
}

pub(super) fn shows_block_selection(tool: Tool) -> bool { matches!(tool, Tool::BlockSelect | Tool::Fill) }

pub(super) fn sync_gestures(session: &Session, gestures: &mut ViewGestures, tool: Tool) {
    if tool != Tool::BlockSelect {
        gestures.block_selection_anchor = None;
        gestures.block_placement = None;
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
    }

    if tool != Tool::RectFill || gestures.fill_anchor.is_some_and(|anchor| anchor.z != session.z()) {
        gestures.fill_anchor = None;
    }
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
    if let Some(placement) = frame.gestures.block_placement
        && session.tool() == Tool::BlockSelect
    {
        session.prepare_block_preview(
            frame.id,
            BlockPreviewSource::Selection(SelectionMask {
                bounds: placement.source,
                mode: session.selection_mode(),
            }),
            placement.target.min,
            SelectionRotation::Original,
        );
    }
}

impl UiState {
    pub(super) fn draw_block_gizmo(
        &mut self, ui: &Ui, session: &mut Session, frame: &mut ViewFrame<'_>, gizmo_view: GizmoMapView,
    ) -> bool {
        let gestures = &mut *frame.gestures;
        let map_size = session.map().map(|map| map.size());
        if gestures.block_selection_anchor.is_some() {
            self.gizmo.cancel();

            return false;
        }

        let (Some(source), Some(size)) = (session.selection(), map_size) else {
            self.gizmo.cancel();

            return false;
        };

        let response = self.gizmo.draw_block(
            ui,
            frame.camera,
            BlockGizmoTarget {
                selection: source,
                map_size: size,
                tile_size: session.options.tile_size,
            },
            gizmo_view,
        );

        match response.drag {
            Some(BlockDrag::Resize) => {
                gestures.rectangle_gesture.get_or_insert(RectangleGesture {
                    start: session.selection_mask(),
                    z: session.z(),
                });
                session.try_select_block(SelectionMask {
                    bounds: response.selection,
                    mode: session.selection_mode(),
                });
            },
            Some(BlockDrag::Move { copy, released }) => {
                gestures.block_placement = (response.selection != source).then_some(PendingBlockPlacement {
                    source,
                    target: response.selection,
                });
                if released {
                    if gestures.block_placement.take().is_some() {
                        let placement = if copy {
                            SelectionPlacement::Copy
                        } else {
                            SelectionPlacement::Move
                        };
                        session.place_selected_block_with_mode(
                            response.selection.min,
                            SelectionRotation::Original,
                            placement,
                            session.selection_mode(),
                        );
                    }

                    self.gizmo.cancel();
                }
            },
            None => gestures.block_placement = None,
        }

        response.captures_mouse
    }

    pub(super) fn start_rectangle(
        &self, ui: &Ui, session: &mut Session, frame: &mut ViewFrame<'_>, coord: Coord, is_left_clicked: bool,
    ) {
        let gestures = &mut *frame.gestures;
        if gestures.block_placement.is_some() || !is_left_clicked || !session.can_edit_at(coord) {
            return;
        }

        let start = session.selection_mask();
        gestures.rectangle_gesture = Some(RectangleGesture { start, z: session.z() });
        // clicking outside the selection clears it, dragging draws a new one
        if start.is_some() {
            session.select_block(None);
            gestures.block_selection_anchor = Some(coord);

            return;
        }

        if session.try_select_block(SelectionMask {
            bounds: Selection::from_drag(coord, coord),
            mode: self.block_selection_options.drawing_mode(ui.io().key_shift()),
        }) {
            gestures.block_selection_anchor = Some(coord);
        }
    }

    pub(super) fn drag_rectangle(&self, ui: &Ui, session: &mut Session, frame: &mut ViewFrame<'_>, is_left_down: bool) {
        if let Some(anchor) = frame.gestures.block_selection_anchor
            && let Some(coord) = drag_coord(session, frame, anchor.z)
            && (coord != anchor || session.selection().is_some())
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

    pub(super) fn start_fill_rect(
        &self, session: &Session, frame: &mut ViewFrame<'_>, coord: Coord, is_left_clicked: bool,
    ) {
        if is_left_clicked && session.can_edit_at(coord) {
            frame.gestures.fill_anchor = Some(coord);
        }
    }

    pub(super) fn drag_fill_rect(&self, ui: &Ui, session: &mut Session, frame: &mut ViewFrame<'_>, is_left_down: bool) {
        let Some(anchor) = frame.gestures.fill_anchor else {
            return;
        };

        if !is_left_down {
            frame.gestures.fill_anchor = None;
        }

        let Some(coord) = drag_coord(session, frame, anchor.z) else {
            return;
        };

        let mask = SelectionMask {
            bounds: Selection::from_drag(anchor, coord),
            mode: self.block_selection_options.drawing_mode(ui.io().key_shift()),
        };
        if is_left_down {
            if !session.can_fill_rect(mask) {
                ui.tooltip_text(if session.palette().is_none() {
                    "Choose a brush to fill with"
                } else {
                    "This rectangle can't be filled here"
                });
            }

            draw_block_outline(ui, session, frame.camera, mask.bounds, mask.mode, frame.layout.viewport);
        } else {
            session.fill_rect(mask);
        }
    }

    pub(super) fn draw_block_overlays(
        &mut self, ui: &Ui, session: &Session, frame: &ViewFrame<'_>, tool: Tool, gizmo_view: GizmoMapView,
    ) {
        let viewport = frame.layout.viewport;
        if frame.gestures.fill_anchor.is_some() {
            self.raise_demand(FrameDemand::Throttled);
        }

        let Some(source) = session.selection().filter(|_| shows_block_selection(tool)) else {
            return;
        };

        let displayed = frame.gestures.displayed(source);
        draw_block_outline(ui, session, frame.camera, displayed, session.selection_mode(), viewport);
        self.raise_demand(FrameDemand::Throttled);

        if tool == Tool::BlockSelect
            && frame.gestures.block_selection_anchor.is_none()
            && let Some(map_size) = session.map().map(|map| map.size())
        {
            self.gizmo.draw_block_overlay(
                ui,
                frame.camera,
                BlockGizmoTarget {
                    selection: displayed,
                    map_size,
                    tile_size: session.options.tile_size,
                },
                gizmo_view,
            );
        }
    }

    pub(super) fn draw_conflict_overlay_controls(
        &mut self, ui: &Ui, session: &mut Session, frame: &ViewFrame<'_>, conflict_regions: &[Region],
    ) {
        draw_conflict_controls(
            ui,
            session,
            frame.id,
            frame.camera,
            conflict_regions,
            frame.layout.viewport.min,
            frame.layout.controls_area(),
        );
    }
}

fn drag_coord(session: &Session, frame: &ViewFrame<'_>, z: u32) -> Option<Coord> {
    let size = session.map()?.size();

    rectangle_drag_coord(
        frame.camera,
        frame.mouse,
        frame.layout.viewport.min,
        size,
        session.options.tile_size,
        z,
    )
}
