use std::mem;

use dear_imgui_rs::{Condition, Ui, WindowKey, WindowKeyError};
use dmm::Coord;
use editor::{
    document::{DocumentId, MapDocument},
    icons::materialdesignicons::{ICON_ALERT, ICON_WEB},
    tool::Tool,
};
use render::{MapViewInteraction, MapViewRect, Renderer};

pub(super) use self::{
    edit::EditCommand,
    presence::{CoopPresence, FollowedView},
    shortcuts::MomentaryTool,
    stroke::{ActivePlacementFlash, PickStroke, PlacementStroke},
};
use self::{
    hover::draw_git_hover,
    overlays::{banner_contains, draw_recent_prefabs},
    presence::{draw_coop_status, hit_comment, share_presence},
    selection::{
        ViewGestures,
        block_controls_contain,
        conflict_controls_contain,
        prepare_block_preview,
        sync_gestures,
    },
    shortcuts::choose_recent_on_key,
    tools::{Clicks, apply_right_click, configure_tool_interaction, draw_node_hit, drive_node_tool, interaction_mode},
};
use super::{
    BlamePopup,
    OverlayRect,
    UiState,
    VisibleMapView,
    common::focus_window_on_hover,
    context_menu::Target as MenuTarget,
    draw_fill_limit_warning,
    node::NodeOverlayHit,
    overlay_padding,
    recent_button_size,
    restore_rectangle_gesture,
};
use crate::{
    camera::Controller,
    gizmo::GizmoMapView,
    session::{GuideBadge, Session},
    settings::{KeybindAction, Settings},
};

mod edit;
mod hover;
mod navigation;
mod overlays;
mod presence;
mod selection;
mod shortcuts;
mod stroke;
#[cfg(test)]
mod tests;
mod tools;

pub(super) struct MapViewState {
    window: WindowKey,
    pub(super) camera: Controller,
    size: (u32, u32),
    pub(super) rect: MapViewRect,
    visible: bool,
    placed: bool,
    pub(super) refit: bool,
    pub(super) focus: bool,
    hovered_coord: Option<Coord>,
    blame_popup: Option<BlamePopup>,
    pub(super) gestures: ViewGestures,
    context: Option<MenuTarget>,
}

pub(super) struct MapViewDraw<'a> {
    pub(super) id: DocumentId,
    pub(super) index: usize,
    pub(super) view: &'a mut MapViewState,
    pub(super) interaction: &'a mut MapViewInteraction,
    pub(super) guide_badges: &'a [GuideBadge],

    pub(super) refit_requested: bool,
    pub(super) keep_open: &'a mut bool,
    pub(super) coop: &'a mut CoopPresence,
}

impl MapViewState {
    pub(super) fn new(id: DocumentId) -> Result<Self, WindowKeyError> {
        Ok(Self {
            window: WindowKey::new(format!("viewport-{}", id.get()), "Map View")?,
            camera: Controller::new(),
            size: (1, 1),
            rect: MapViewRect::default(),
            visible: false,
            placed: false,
            refit: true,
            hovered_coord: None,
            blame_popup: None,
            focus: false,
            gestures: ViewGestures::default(),
            context: None,
        })
    }

    pub(super) const fn window(&self) -> &WindowKey { &self.window }
}

#[derive(Debug, Clone, Copy)]
struct ViewLayout {
    extent: (u32, u32),
    scale: [f32; 2],
    viewport: OverlayRect,
    top_overlay: OverlayRect,
    bottom_overlay: OverlayRect,
}

impl ViewLayout {
    fn new(ui: &Ui, extent: (u32, u32), scale: [f32; 2]) -> Self {
        let viewport = OverlayRect {
            min: ui.item_rect_min(),
            max: ui.item_rect_max(),
        };
        let padding = overlay_padding(ui);
        let top_height = ui.frame_height() + padding * 2.0;
        let bottom_height = recent_button_size(ui) + padding * 2.0;

        Self {
            extent,
            scale,
            viewport,
            top_overlay: OverlayRect {
                min: viewport.min,
                max: [viewport.max[0], (viewport.min[1] + top_height).min(viewport.max[1])],
            },
            bottom_overlay: OverlayRect {
                min: [viewport.min[0], (viewport.max[1] - bottom_height).max(viewport.min[1])],
                max: viewport.max,
            },
        }
    }

    fn center(&self) -> [f32; 2] { [self.extent.0 as f32 * 0.5, self.extent.1 as f32 * 0.5] }

    fn controls_area(&self) -> OverlayRect {
        OverlayRect {
            min: [self.viewport.min[0], self.top_overlay.max[1]],
            max: [self.viewport.max[0], self.bottom_overlay.min[1]],
        }
    }

    fn local(&self, point: [f32; 2]) -> Option<[f32; 2]> {
        let local = [point[0] - self.viewport.min[0], point[1] - self.viewport.min[1]];

        local
            .iter()
            .all(|value| value.is_finite() && *value >= 0.0)
            .then_some(local)
            .filter(|local| local[0] < self.extent.0 as f32 && local[1] < self.extent.1 as f32)
    }
}

struct ViewFrame<'a> {
    id: DocumentId,
    camera: &'a mut Controller,
    refit: &'a mut bool,
    gestures: &'a mut ViewGestures,
    blame_popup: &'a mut Option<BlamePopup>,
    context: &'a mut Option<MenuTarget>,
    interaction: &'a mut MapViewInteraction,
    layout: ViewLayout,
    mouse: [f32; 2],
    is_active: bool,
    is_focused: bool,
    is_pointed: bool,
    is_hovered: bool,
    cursor: Option<[f32; 2]>,
    coord: Option<Coord>,
}

impl ViewFrame<'_> {
    fn resolve_cursor(&mut self, session: &Session, is_over_blame_popup: bool) {
        self.cursor = (self.is_hovered && !is_over_blame_popup)
            .then_some(self.mouse)
            .and_then(|point| self.layout.local(point));
        self.coord = self.cursor.and_then(|cursor| {
            let size = session.map()?.size;

            self.camera
                .screen_to_tile(cursor, size, session.options.tile_size, session.z())
        });
    }
}

#[derive(Debug, Clone, Copy)]
struct MouseCapture {
    is_block_controls: bool,
    is_conflict_controls: bool,
    is_banner: bool,
    is_blame_popup: bool,
    is_escape: bool,
}

impl MouseCapture {
    const fn is_any(self) -> bool {
        self.is_block_controls || self.is_conflict_controls || self.is_banner || self.is_blame_popup || self.is_escape
    }
}

fn framebuffer_rect(origin: [f32; 2], size: [f32; 2], scale: [f32; 2], target: [f32; 2]) -> MapViewRect {
    let limit = |value: f32, max: f32| value.max(0.0).min(max.max(0.0)).floor() as u32;
    let (max_x, max_y) = (target[0] * scale[0], target[1] * scale[1]);
    MapViewRect {
        x: limit(origin[0] * scale[0], max_x),
        y: limit(origin[1] * scale[1], max_y),
        width: limit(size[0] * scale[0], u32::MAX as f32),
        height: limit(size[1] * scale[1], u32::MAX as f32),
    }
}

fn cursor_in_map_view(local: [f32; 2], scale: [f32; 2]) -> [u32; 2] {
    [
        (local[0] * scale[0]).max(0.0) as u32,
        (local[1] * scale[1]).max(0.0) as u32,
    ]
}

fn panel_extent(available: [f32; 2]) -> ([f32; 2], (u32, u32)) {
    let width = if available[0].is_finite() {
        available[0].max(1.0)
    } else {
        1.0
    };
    let height = if available[1].is_finite() {
        available[1].max(1.0)
    } else {
        1.0
    };

    ([width, height], (width.floor() as u32, height.floor() as u32))
}

fn map_view_title(session: &Session, id: DocumentId, name: &str, is_out_of_date: bool) -> String {
    let is_shared = session
        .state
        .document(id)
        .and_then(|document| document.path.as_deref())
        .is_some_and(|path| session.is_coop_shared_file(path));

    if is_out_of_date {
        format!("{ICON_ALERT} {name}")
    } else if is_shared {
        format!("{ICON_WEB} {name}")
    } else {
        name.to_owned()
    }
}

impl UiState {
    pub(super) fn draw_map_views(
        &mut self, ui: &Ui, session: &mut Session, settings: &mut Settings, refit_active: bool,
    ) -> (Vec<VisibleMapView>, Option<usize>, CoopPresence) {
        let mut closing = None;
        let mut visible = Vec::new();
        let mut coop = CoopPresence::default();

        for (&id, view) in &mut self.map_views {
            if view.gestures.rectangle_gesture.is_some_and(|gesture| {
                session.state.active() != Some(id)
                    || session.tool() != Tool::BlockSelect
                    || session
                        .state
                        .document(id)
                        .is_none_or(|document| document.z != gesture.z)
            }) {
                restore_rectangle_gesture(session, id, &mut view.gestures.rectangle_gesture);
                view.gestures.block_selection_anchor = None;
                self.gizmo.cancel();
            }
        }

        for id in session.state.document_ids() {
            let mut view = match self.map_views.remove(&id) {
                Some(view) => view,
                None => match MapViewState::new(id) {
                    Ok(view) => view,
                    Err(e) => {
                        log::error!("could not create a map view window for document {}: {e}", id.get());

                        continue;
                    },
                },
            };

            let refit = refit_active && session.state.active() == Some(id);
            let mut keep_open = true;
            let active = session.state.active() == Some(id);
            let guides = if active && settings.selection_guide_line && session.tool() == Tool::Select {
                session.selected_guides()
            } else {
                Default::default()
            };
            let mut interaction = MapViewInteraction {
                selected: session.selected_instance_of(id),
                highlight: settings.selection_highlight.style(),
                mode: interaction_mode(session.tool()),
                ..Default::default()
            };
            let map_view_index = visible.len();
            self.draw_map_view(
                ui,
                session,
                settings,
                MapViewDraw {
                    id,
                    index: map_view_index,
                    view: &mut view,
                    interaction: &mut interaction,
                    guide_badges: &guides.badges,

                    refit_requested: refit,
                    keep_open: &mut keep_open,
                    coop: &mut coop,
                },
            );

            if view.visible && !view.rect.is_empty() {
                visible.push(VisibleMapView {
                    document: id,
                    rect: view.rect,
                    camera: view.camera.camera,
                    interaction,
                    guide_lines: guides.lines,
                    connected: guides.connected,
                });
            }

            self.map_views.insert(id, view);

            if !keep_open {
                closing = Some(id);
            }
        }

        if let Some(id) = closing {
            self.request_close(session, [id]);
        }

        self.draw_close_confirmation(ui, session);

        self.map_views.retain(|id, _| session.state.document(*id).is_some());
        if settings.mirror_camera {
            self.mirror_active_camera(session);
        }

        let picking = visible.iter().position(|view| view.interaction.cursor.is_some());

        (visible, picking, coop)
    }

    pub(super) fn draw_map_view(
        &mut self, ui: &Ui, session: &mut Session, settings: &mut Settings, draw: MapViewDraw<'_>,
    ) {
        let MapViewDraw {
            id,
            index,
            view,
            interaction,
            guide_badges,

            refit_requested,
            keep_open,
            coop,
        } = draw;
        session.hide_block_preview(id);
        let Some(name) = session.state.document(id).map(MapDocument::title) else {
            return;
        };

        let is_out_of_date = session.coop_out_of_date(id).is_some();
        let title = map_view_title(session, id, &name, is_out_of_date);

        // the window label borrows the key for the whole window, so the rest of the view is borrowed field by field
        let MapViewState {
            window,
            camera,
            size,
            rect,
            visible,
            placed,
            refit,
            focus,
            hovered_coord,
            blame_popup,
            gestures,
            context,
        } = view;
        *visible = false;
        *refit |= refit_requested;

        if !mem::replace(placed, true)
            && let Some(node) = self.central_node.or(self.dockspace_root)
        {
            ui.set_next_window_dock_id_with_cond(node, Condition::Always);
        }

        let map_window = ui
            .window(window.label(title.as_str()))
            .opened(keep_open)
            .focused(mem::take(focus));
        map_window.build(|| {
            self.activate_on_focus(ui, session, settings, id, gestures);

            let is_receiving = draw_coop_status(ui, session, id, &name, is_out_of_date);
            if is_receiving {
                return;
            }

            let (image_size, extent) = panel_extent(ui.content_region_avail());
            *size = extent;
            camera.resize(extent.0, extent.1);
            if *refit {
                let (width, height) = session.extent_px_of(id);
                camera.frame_map(width, height);
                *refit = false;
            }

            *visible = true;

            let scale = ui.io().display_framebuffer_scale();
            *rect = framebuffer_rect(ui.cursor_screen_pos(), image_size, scale, ui.io().display_size());
            ui.image(Renderer::map_view_texture(index), image_size);

            let is_image_hovered = ui.is_item_hovered();
            let layout = ViewLayout::new(ui, extent, scale);
            let mouse = ui.io().mouse_pos();
            let mut frame = ViewFrame {
                id,
                camera,
                refit,
                gestures,
                blame_popup,
                context,
                interaction,
                layout,
                mouse,
                is_active: session.state.active() == Some(id),
                is_focused: ui.is_window_focused(),
                is_pointed: is_image_hovered
                    && !layout.top_overlay.contains(mouse)
                    && !layout.bottom_overlay.contains(mouse),
                is_hovered: false,
                cursor: None,
                coord: None,
            };

            let comment_hit = hit_comment(ui, session, &frame);
            frame.is_hovered = frame.is_pointed && comment_hit.is_none() && frame.is_active;
            let is_drag_panning = frame.is_hovered && settings.keybindings.get(KeybindAction::PanDrag).is_held(ui);
            self.release_momentary_tool(ui, session, frame.is_hovered && !is_drag_panning);
            choose_recent_on_key(ui, session, settings, frame.is_focused);

            if frame.is_hovered && !ui.io().want_text_input() {
                self.pan_and_zoom(ui, settings, &mut frame, is_drag_panning);
                self.apply_view_shortcuts(ui, session, settings, frame.refit);
                if *frame.refit {
                    let (width, height) = session.extent_px();
                    frame.camera.frame_map(width, height);
                    *frame.refit = false;
                }
            }

            frame.is_hovered &= !is_drag_panning;
            self.draw_view_overlays(ui, session, settings, &frame, *hovered_coord, guide_badges);
            self.draw_coop_overlays(ui, session, &frame, comment_hit);

            let is_over_blame_popup = frame
                .blame_popup
                .as_ref()
                .and_then(|popup| popup.bounds)
                .is_some_and(|bounds| bounds.contains(mouse));
            frame.resolve_cursor(session, is_over_blame_popup);
            *hovered_coord = frame.coord;
            share_presence(session, &frame, coop);

            let is_blame_capturing = draw_git_hover(ui, session, &mut frame, is_over_blame_popup);
            let node_hit = draw_node_hit(ui, session, &frame);
            configure_tool_interaction(session.tool(), frame.interaction);
            self.toggle_focus_on_key(ui, session, settings, &frame);

            if frame.is_active {
                self.draw_active_map(ui, session, settings, &mut frame, &node_hit, is_blame_capturing);
            }
        });
    }

    fn activate_on_focus(
        &mut self, ui: &Ui, session: &mut Session, settings: &Settings, id: DocumentId, gestures: &mut ViewGestures,
    ) {
        if settings.focus_windows_on_hover && !self.panel_focus_requested {
            focus_window_on_hover(ui);
        }

        if ui.is_window_focused() {
            if session.state.active() != Some(id) {
                self.cancel_edit_gestures(session, session.state.active());
            }

            session.set_active_document(id);
        }

        if session.state.active() != Some(id) && gestures.rectangle_gesture.is_some() {
            restore_rectangle_gesture(session, id, &mut gestures.rectangle_gesture);
            gestures.block_selection_anchor = None;
            self.gizmo.cancel();
        }

        let dock = ui.get_window_dock_id();
        if dock.raw() != 0 {
            self.central_node = Some(dock);
        }
    }

    fn draw_active_map(
        &mut self, ui: &Ui, session: &mut Session, settings: &Settings, frame: &mut ViewFrame<'_>,
        node_hit: &NodeOverlayHit, is_blame_capturing: bool,
    ) {
        self.apply_edit_keys(ui, session, settings, frame);
        self.apply_edit_command(session, frame);

        let tool = session.tool();
        let gizmo_context = (frame.id, tool, session.z());
        if self.gizmo_context != Some(gizmo_context) {
            self.gizmo.cancel();
            self.gizmo_context = Some(gizmo_context);
        }

        if frame
            .gestures
            .rectangle_gesture
            .is_some_and(|gesture| tool != Tool::BlockSelect || gesture.z != session.z())
        {
            restore_rectangle_gesture(session, frame.id, &mut frame.gestures.rectangle_gesture);
            frame.gestures.block_selection_anchor = None;
            self.gizmo.cancel();
        }

        let is_escape = self.cancel_on_escape(ui, session, frame, tool);
        self.draw_place_preview(ui, session, settings, frame, tool);

        let paste_mask = sync_gestures(session, frame.gestures, tool);
        let paste_target = paste_mask.map(|mask| mask.bounds);
        let conflict_regions = session.conflict_regions(frame.id, Some(session.z()));
        let capture = MouseCapture {
            is_block_controls: block_controls_contain(
                ui,
                session,
                frame,
                tool,
                paste_target,
                self.gizmo.block_rotation_open(),
            ),
            is_conflict_controls: conflict_controls_contain(ui, session, frame, &conflict_regions),
            is_banner: banner_contains(ui, session, frame),
            is_blame_popup: is_blame_capturing,
            is_escape,
        };
        let gizmo_view = GizmoMapView {
            min: frame.layout.viewport.min,
            max: frame.layout.viewport.max,
            hovered: frame.is_hovered && !capture.is_any() && !ui.io().want_text_input(),
        };
        let is_gizmo_capturing = self.draw_tool_gizmo(ui, session, settings, frame, tool, paste_target, gizmo_view);

        let clicks = Clicks::read(ui);
        if clicks.is_right_clicked
            && !is_blame_capturing
            && let Some(coord) = frame.coord
        {
            apply_right_click(ui, session, frame, node_hit, coord);
        }

        if session.tool() == Tool::Node {
            drive_node_tool(session, frame, node_hit, clicks, is_blame_capturing);
        }

        self.end_strokes(session, clicks, is_gizmo_capturing);
        if !capture.is_any()
            && !is_gizmo_capturing
            && session.tool() != Tool::Node
            && let Some(cursor) = frame.cursor
        {
            self.apply_tool_at(ui, session, settings, frame, cursor, clicks);
        }

        self.drag_rectangle(ui, session, frame, clicks.is_left_down);
        self.draw_block_overlays(ui, session, frame, tool, paste_mask, gizmo_view);
        self.apply_map_menu(ui, session, settings, frame);

        let actions = self.draw_block_controls(ui, session, frame, tool, paste_target, &conflict_regions);
        self.draw_top_overlays(ui, session, settings, frame);
        self.apply_block_actions(session, frame.gestures, actions);
        draw_recent_prefabs(ui, session, settings, frame);
        configure_tool_interaction(session.tool(), frame.interaction);
        draw_fill_limit_warning(
            ui,
            session,
            &mut self.pending_fill_warning,
            self.fill_mode,
            &self.custom_fill_boundaries,
        );
        prepare_block_preview(session, frame);
    }

    fn cancel_view_edits(&mut self, session: &mut Session, frame: &mut ViewFrame<'_>) {
        frame.gestures.cancel(session, frame.id);
        self.cancel_tool_gestures();
        session.cancel_node_drag();
    }
}
