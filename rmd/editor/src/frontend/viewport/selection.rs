use dear_imgui_rs::Ui;
use dmm::Coord;

use super::ViewFrame;
use crate::{
    conflict::{Hunk, Side},
    document::{DocumentId, Selection},
    frontend::{
        PendingRegionPlacement, RectangleGesture, UiState, conflict_controls_layout, draw_conflict_controls,
        draw_region_outline, rectangle_drag_coord, restore_rectangle_gesture,
    },
    gizmo::{GizmoMapView, RegionDrag, RegionGizmoTarget},
    maploader::preview::ModuleRootKey,
    pacing::FrameDemand,
    session::{RegionPreviewSource, Session},
    tool::{SelectionMask, SelectionPlacement, SelectionRotation, Tool},
};

#[derive(Debug, Default)]
pub(in crate::frontend) struct ViewGestures {
    pub(in crate::frontend) module_popup: Option<(ModuleRootKey, Coord)>,
    pub(super) region_selection_anchor: Option<Coord>,
    pub(super) rectangle_gesture: Option<RectangleGesture>,
    pub(super) region_placement: Option<PendingRegionPlacement>,
    pub(super) fill_anchor: Option<Coord>,
}

impl ViewGestures {
    pub(in crate::frontend) fn cancel(&mut self, session: &mut Session, id: DocumentId) {
        if session.state.active() == Some(id) {
            session.cancel_live_move();
        }

        restore_rectangle_gesture(session, id, &mut self.rectangle_gesture);
        self.region_selection_anchor = None;
        self.region_placement = None;
        self.fill_anchor = None;
    }

    pub(super) const fn is_busy(&self) -> bool {
        self.rectangle_gesture.is_some() || self.region_placement.is_some() || self.fill_anchor.is_some()
    }

    pub(super) fn displayed(&self, source: Selection) -> Selection {
        self.region_placement.map_or(source, |placement| placement.target)
    }
}

pub(super) fn shows_region_selection(tool: Tool) -> bool {
    matches!(tool, Tool::Select | Tool::Fill)
}

pub(super) fn sync_gestures(session: &Session, gestures: &mut ViewGestures, tool: Tool) {
    if tool != Tool::Select {
        // we changed the tool, drop it
        gestures.region_selection_anchor = None;
        gestures.region_placement = None;
    } else {
        if gestures
            .region_selection_anchor
            .is_some_and(|anchor| anchor.z != session.z())
        {
            // we changed z level, drop it
            gestures.region_selection_anchor = None;
        }

        if gestures.region_placement.is_some_and(|placement| {
            let occupied = match placement.placement {
                SelectionPlacement::Move => placement.target,
                SelectionPlacement::Copy => placement.source,
            };

            Some(occupied) != session.selection()
        }) {
            // current source of placement is changed, drop it (mapper might have pasted from different map)
            gestures.region_placement = None;
        }
    }

    if tool != Tool::RectFill || gestures.fill_anchor.is_some_and(|anchor| anchor.z != session.z()) {
        gestures.fill_anchor = None;
    }
}

pub(super) fn conflict_controls_contain(ui: &Ui, session: &Session, frame: &ViewFrame<'_>, hunks: &[Hunk]) -> bool {
    let theirs_label = session
        .git_state(frame.id)
        .and_then(|git| git.conflicts.as_ref())
        .map_or(String::from("theirs"), |conflicts| {
            conflicts.side_label(Side::Theirs).to_owned()
        });

    hunks.iter().any(|hunk| {
        conflict_controls_layout(
            ui,
            frame.camera,
            hunk,
            session.options.tile_size,
            frame.layout.viewport.min,
            frame.layout.controls_area(),
            &theirs_label,
        )
        .is_some_and(|(_, bounds)| bounds.contains(frame.mouse))
    })
}

pub(super) fn prepare_region_preview(session: &mut Session, frame: &ViewFrame<'_>) {
    if let Some(placement) = frame.gestures.region_placement
        && placement.placement == SelectionPlacement::Copy
        && session.tool() == Tool::Select
    {
        session.prepare_region_preview(
            frame.id,
            RegionPreviewSource::Selection(SelectionMask {
                bounds: placement.source,
                mode: session.selection_mode(),
            }),
            placement.target.min,
            SelectionRotation::Original,
        );
    }
}

impl UiState {
    pub(super) fn draw_region_gizmo(
        &mut self, ui: &Ui, session: &mut Session, frame: &mut ViewFrame<'_>, gizmo_view: GizmoMapView,
    ) -> bool {
        let gestures = &mut *frame.gestures;
        let map_size = session.map().map(|map| map.size());
        if gestures.region_selection_anchor.is_some() {
            self.gizmo.cancel();
            session.cancel_live_move();

            return false;
        }

        let (Some(source), Some(size)) = (session.selection(), map_size) else {
            self.gizmo.cancel();
            session.cancel_live_move();

            return false;
        };

        let response = self.gizmo.draw_region(
            ui,
            frame.camera,
            RegionGizmoTarget {
                selection: source,
                map_size: size,
                tile_size: session.options.tile_size,
            },
            gizmo_view,
        );

        match response.drag {
            Some(RegionDrag::Resize) => {
                gestures.rectangle_gesture.get_or_insert(RectangleGesture {
                    start: session.selection_mask(),
                    z: session.z(),
                });
                session.try_select_region(SelectionMask {
                    bounds: response.selection,
                    mode: session.selection_mode(),
                });
            },
            Some(RegionDrag::Move { copy: true, released }) => {
                gestures.region_placement = (response.selection != source).then_some(PendingRegionPlacement {
                    source,
                    target: response.selection,
                    placement: SelectionPlacement::Copy,
                });
                if released {
                    if gestures.region_placement.take().is_some() {
                        session.place_selected_region_with_mode(
                            response.selection.min,
                            SelectionRotation::Original,
                            SelectionPlacement::Copy,
                            session.selection_mode(),
                        );
                    }

                    self.gizmo.cancel();
                }
            },
            Some(RegionDrag::Move { copy: false, released }) => {
                session.move_selected_region_live(response.selection.min);
                gestures.region_placement = session
                    .pending_move_source()
                    .zip(session.selection())
                    .filter(|(source, target)| source != target)
                    .map(|(source, target)| PendingRegionPlacement {
                        source,
                        target,
                        placement: SelectionPlacement::Move,
                    });
                if released {
                    session.finish_live_move();
                    gestures.region_placement = None;
                    self.gizmo.cancel();
                }
            },
            None => {
                gestures.region_placement = None;
                session.cancel_live_move();
            },
        }

        response.captures_mouse
    }

    pub(super) fn start_rectangle(
        &self, ui: &Ui, session: &mut Session, frame: &mut ViewFrame<'_>, coord: Coord, is_left_clicked: bool,
    ) {
        let gestures = &mut *frame.gestures;
        if gestures.region_placement.is_some() || !is_left_clicked || !session.can_select_at(coord) {
            return;
        }

        let start = session.selection_mask();
        gestures.rectangle_gesture = Some(RectangleGesture { start, z: session.z() });
        // clicking outside the selection clears it, dragging draws a new one
        if start.is_some() {
            session.select_region(None);
            gestures.region_selection_anchor = Some(coord);

            return;
        }

        if session.try_select_region(SelectionMask {
            bounds: Selection::from_drag(coord, coord),
            mode: self.region_selection_options.drawing_mode(ui.io().key_shift()),
        }) {
            gestures.region_selection_anchor = Some(coord);
        }
    }

    pub(super) fn drag_rectangle(&self, ui: &Ui, session: &mut Session, frame: &mut ViewFrame<'_>, is_left_down: bool) {
        if let Some(anchor) = frame.gestures.region_selection_anchor
            && let Some(coord) = drag_coord(session, frame, anchor.z)
            && (coord != anchor || session.selection().is_some())
        {
            session.try_select_region(SelectionMask {
                bounds: Selection::from_drag(anchor, coord),
                mode: if is_left_down {
                    self.region_selection_options.drawing_mode(ui.io().key_shift())
                } else {
                    session.selection_mode()
                },
            });
        }

        if !is_left_down {
            frame.gestures.region_selection_anchor = None;
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
            mode: self.region_selection_options.drawing_mode(ui.io().key_shift()),
        };
        if is_left_down {
            if !session.can_fill_rect(mask) {
                ui.tooltip_text(if session.palette().is_none() {
                    "Choose a brush to fill with"
                } else {
                    "This rectangle can't be filled here"
                });
            }

            draw_region_outline(ui, session, frame.camera, mask.bounds, mask.mode, frame.layout.viewport);
        } else {
            session.fill_rect(mask);
        }
    }

    pub(super) fn draw_region_overlays(
        &mut self, ui: &Ui, session: &Session, frame: &ViewFrame<'_>, tool: Tool, gizmo_view: GizmoMapView,
    ) {
        let viewport = frame.layout.viewport;
        if frame.gestures.fill_anchor.is_some() {
            self.raise_demand(FrameDemand::Throttled);
        }

        let Some(source) = session.selection().filter(|_| shows_region_selection(tool)) else {
            return;
        };

        let displayed = frame.gestures.displayed(source);
        draw_region_outline(ui, session, frame.camera, displayed, session.selection_mode(), viewport);
        self.raise_demand(FrameDemand::Throttled);

        if tool == Tool::Select
            && frame.gestures.region_selection_anchor.is_none()
            && let Some(map_size) = session.map().map(|map| map.size())
        {
            self.gizmo.draw_region_overlay(
                ui,
                frame.camera,
                RegionGizmoTarget {
                    selection: displayed,
                    map_size,
                    tile_size: session.options.tile_size,
                },
                gizmo_view,
            );
        }
    }

    pub(super) fn draw_conflict_overlay_controls(
        &mut self, ui: &Ui, session: &mut Session, frame: &ViewFrame<'_>, conflict_hunks: &[Hunk],
    ) {
        draw_conflict_controls(
            ui,
            session,
            frame.id,
            frame.camera,
            conflict_hunks,
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
