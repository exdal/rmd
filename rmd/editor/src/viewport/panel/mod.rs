use std::mem;

use dear_imgui_rs::{Condition, Ui, WindowFlags, WindowKey, WindowKeyError};
use dmm::Coord;
use render::{MapViewInteraction, MapViewRect, Renderer};
use ui::{
    icons::materialdesignicons::{ICON_ALERT, ICON_WEB},
    widgets::focus_window_on_hover,
};

use self::{
    context_menu::Target as MenuTarget,
    hover::draw_tile_hover,
    overlay::{OverlayRect, overlay_padding},
    overlays::{banner_contains, draw_recent_prefabs},
    selection::{ViewGestures, conflict_controls_contain, prepare_region_preview, sync_gestures},
    shortcuts::choose_recent_on_key,
    tools::{
        Clicks, apply_right_click, configure_tool_interaction, draw_node_hit, drive_node_tool, interaction_mode,
        shows_picked,
    },
};
use crate::{
    app::pacing::FrameDemand,
    coop::panel::presence::{CoopPresence, draw_coop_status, draw_tab_menu, hit_comment, share_presence},
    document::DocumentId,
    frontend::{CloseFocus, UiState, VisibleMapView},
    git::panel::blame::BlamePopup,
    gizmo::{guides::GuideBadge, panel::GizmoMapView},
    maploader::panel,
    node::panel::NodeOverlayHit,
    palette::panel::{
        history::recent_button_size,
        prefab_menu::{PrefabMenu, update_prefab_menu},
    },
    region::panel::restore_rectangle_gesture,
    session::Session,
    settings::{Settings, keybind::KeybindAction},
    tool::{Tool, panel::fill_warning::draw_fill_limit_warning},
    viewport::camera::Controller,
};

pub(crate) mod context_menu;
pub(crate) mod edit;
mod grid;
mod hover;
mod navigation;
pub(crate) mod overlay;
mod overlays;
pub(crate) mod selection;
pub(crate) mod shortcuts;
pub(crate) mod stroke;
#[cfg(test)]
mod tests;
mod tools;

pub(crate) struct MapViewState {
    window: WindowKey,
    pub(crate) camera: Controller,
    size: (u32, u32),
    pub(crate) rect: MapViewRect,
    visible: bool,
    placed: bool,
    pub(crate) refit: bool,
    pub(crate) focus: bool,
    hovered_coord: Option<Coord>,
    blame_popup: Option<BlamePopup>,
    prefab_menu: Option<PrefabMenu>,
    pub(crate) gestures: ViewGestures,
    context: Option<MenuTarget>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ViewKind {
    Map,
    StashEntry(OverlayRect),
}

pub(crate) struct ViewOutput<'a> {
    pub(crate) visible: &'a mut Vec<VisibleMapView>,
    pub(crate) coop: &'a mut CoopPresence,
}

struct ViewContents<'a> {
    id: DocumentId,
    kind: ViewKind,
    index: usize,
    name: &'a str,
    is_out_of_date: bool,
    view: &'a mut MapViewState,
    interaction: &'a mut MapViewInteraction,
    guide_badges: &'a [GuideBadge],
    coop: &'a mut CoopPresence,
}

pub(crate) struct MapViewDraw<'a> {
    pub(crate) id: DocumentId,
    pub(crate) index: usize,
    pub(crate) view: &'a mut MapViewState,
    pub(crate) interaction: &'a mut MapViewInteraction,
    pub(crate) guide_badges: &'a [GuideBadge],

    pub(crate) refit_requested: bool,
    pub(crate) keep_open: &'a mut bool,
    pub(crate) coop: &'a mut CoopPresence,
}

impl MapViewState {
    pub(crate) fn new(id: DocumentId) -> Result<Self, WindowKeyError> {
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
            prefab_menu: None,
            focus: false,
            gestures: ViewGestures::default(),
            context: None,
        })
    }

    pub(crate) const fn window(&self) -> &WindowKey {
        &self.window
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ViewLayout {
    extent: (u32, u32),
    scale: [f32; 2],
    pub(crate) viewport: OverlayRect,
    top_overlay: OverlayRect,
    bottom_overlay: OverlayRect,
}

impl ViewLayout {
    fn new(ui: &Ui, extent: (u32, u32), scale: [f32; 2], kind: ViewKind) -> Self {
        let viewport = OverlayRect {
            min: ui.item_rect_min(),
            max: ui.item_rect_max(),
        };
        let padding = overlay_padding(ui);
        let (top_height, bottom_height) = match kind {
            ViewKind::Map => (
                ui.frame_height() + padding * 2.0,
                recent_button_size(ui) + padding * 2.0,
            ),
            ViewKind::StashEntry(_) => (0.0, 0.0),
        };

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

    fn center(&self) -> [f32; 2] {
        [self.extent.0 as f32 * 0.5, self.extent.1 as f32 * 0.5]
    }

    pub(crate) fn controls_area(&self) -> OverlayRect {
        OverlayRect {
            min: [self.viewport.min[0], self.top_overlay.max[1]],
            max: [self.viewport.max[0], self.bottom_overlay.min[1]],
        }
    }

    pub(crate) fn local(&self, point: [f32; 2]) -> Option<[f32; 2]> {
        let local = [point[0] - self.viewport.min[0], point[1] - self.viewport.min[1]];

        local
            .iter()
            .all(|value| value.is_finite() && *value >= 0.0)
            .then_some(local)
            .filter(|local| local[0] < self.extent.0 as f32 && local[1] < self.extent.1 as f32)
    }
}

pub(crate) struct ViewFrame<'a> {
    pub(crate) id: DocumentId,
    kind: ViewKind,
    pub(crate) camera: &'a mut Controller,
    refit: &'a mut bool,
    pub(crate) gestures: &'a mut ViewGestures,
    blame_popup: &'a mut Option<BlamePopup>,
    context: &'a mut Option<MenuTarget>,
    interaction: &'a mut MapViewInteraction,
    pub(crate) layout: ViewLayout,
    pub(crate) mouse: [f32; 2],
    pub(crate) is_active: bool,
    is_focused: bool,
    pub(crate) is_pointed: bool,
    pub(crate) is_hovered: bool,
    cursor: Option<[f32; 2]>,
    coord: Option<Coord>,
}

impl ViewFrame<'_> {
    fn resolve_cursor(&mut self, session: &Session, is_over_blame_popup: bool) {
        self.cursor = (self.is_hovered && !is_over_blame_popup)
            .then_some(self.mouse)
            .and_then(|point| self.layout.local(point));
        self.coord = self.cursor.and_then(|cursor| {
            let size = session.map()?.size();

            self.camera
                .screen_to_tile(cursor, size, session.options.tile_size, session.z())
        });
    }
}

#[derive(Debug, Clone, Copy)]
struct MouseCapture {
    is_conflict_controls: bool,
    is_banner: bool,
    is_blame_popup: bool,
    is_escape: bool,
}

impl MouseCapture {
    const fn is_any(self) -> bool {
        self.is_conflict_controls || self.is_banner || self.is_blame_popup || self.is_escape
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

pub(crate) fn panel_extent(available: [f32; 2]) -> ([f32; 2], (u32, u32)) {
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
    pub(crate) fn draw_map_views(
        &mut self, ui: &Ui, session: &mut Session, settings: &mut Settings, refit_active: bool,
    ) -> (Vec<VisibleMapView>, Option<usize>, CoopPresence) {
        let mut closing = None;
        let mut visible = Vec::new();
        let mut coop = CoopPresence::default();

        for (&id, view) in &mut self.map_views {
            if view.gestures.rectangle_gesture.is_some_and(|gesture| {
                session.state.active() != Some(id)
                    || session.tool() != Tool::Select
                    || session
                        .state
                        .document(id)
                        .is_none_or(|document| document.z != gesture.z)
            }) {
                restore_rectangle_gesture(session, id, &mut view.gestures.rectangle_gesture);
                view.gestures.region_selection_anchor = None;
                self.gizmo.cancel();
            }
        }

        for id in session.state.document_ids() {
            if self.draw_document_view(
                ui,
                session,
                settings,
                id,
                refit_active,
                ViewKind::Map,
                ViewOutput {
                    visible: &mut visible,
                    coop: &mut coop,
                },
            ) == Some(false)
            {
                closing = Some(id);
            }
        }

        if mem::take(&mut self.is_stash_open_requested) {
            settings.panels.stash = true;
            self.stash_view.refit();
        }

        if settings.panels.stash {
            self.draw_stash(
                ui,
                session,
                settings,
                refit_active,
                ViewOutput {
                    visible: &mut visible,
                    coop: &mut coop,
                },
            );
        }

        if !settings.panels.stash {
            self.hide_stash(session);
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

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_document_view(
        &mut self, ui: &Ui, session: &mut Session, settings: &mut Settings, id: DocumentId, refit_active: bool,
        kind: ViewKind, output: ViewOutput<'_>,
    ) -> Option<bool> {
        let ViewOutput { visible, coop } = output;
        let mut view = match self.map_views.remove(&id) {
            Some(view) => view,
            None => match MapViewState::new(id) {
                Ok(view) => view,
                Err(e) => {
                    log::error!("could not create a map view window for document {}: {e}", id.get());

                    return None;
                },
            },
        };

        if self.close_focus == Some(CloseFocus::Map(id)) {
            self.close_focus = None;
            view.focus = true;
        }

        let refit = refit_active && session.state.active() == Some(id);
        let mut keep_open = true;
        let active = session.state.active() == Some(id);
        let guides = if active && settings.selection_guide_line && session.tool() == Tool::Pick {
            session.picked_guides()
        } else {
            Default::default()
        };
        let mut interaction = MapViewInteraction {
            highlight: settings.selection_highlight.style(),
            mode: interaction_mode(session.tool()),
            ..Default::default()
        };
        let map_view_index = visible.len();
        match kind {
            ViewKind::Map => self.draw_map_view(
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
                    coop,
                },
            ),
            ViewKind::StashEntry(_) => {
                view.visible = false;
                self.draw_view_contents(
                    ui,
                    session,
                    settings,
                    ViewContents {
                        id,
                        kind,
                        index: map_view_index,
                        name: "",
                        is_out_of_date: false,
                        view: &mut view,
                        interaction: &mut interaction,
                        guide_badges: &guides.badges,
                        coop,
                    },
                );
            },
        }

        if view.visible && !view.rect.is_empty() {
            let picked = if active && shows_picked(session.tool()) {
                session.picked_instances()
            } else {
                Vec::new()
            };
            visible.push(VisibleMapView {
                document: id,
                rect: view.rect,
                camera: view.camera.camera,
                interaction,
                guide_lines: guides.lines,
                connected: guides.connected,
                picked,
            });
        }

        self.map_views.insert(id, view);

        Some(keep_open)
    }

    pub(crate) fn draw_map_view(
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
        session.hide_region_preview(id);
        let Some(document) = session.state.document(id) else {
            return;
        };
        let name = document.title();
        let flags = if document.is_dirty() {
            WindowFlags::UNSAVED_DOCUMENT
        } else {
            WindowFlags::empty()
        };

        let is_out_of_date = session.coop_out_of_date(id).is_some();
        let title = map_view_title(session, id, &name, is_out_of_date);

        view.visible = false;
        view.refit |= refit_requested;

        if !mem::replace(&mut view.placed, true)
            && let Some(node) = self.central_node.or(self.dockspace_root)
        {
            ui.set_next_window_dock_id_with_cond(node, Condition::Always);
        }

        let window = view.window.clone();
        let map_window = ui
            .window(window.label(title.as_str()))
            .flags(flags)
            .opened(keep_open)
            .focused(mem::take(&mut view.focus));
        map_window.build(|| {
            draw_tab_menu(ui, session, id);
            self.draw_view_contents(
                ui,
                session,
                settings,
                ViewContents {
                    id,
                    kind: ViewKind::Map,
                    index,
                    name: &name,
                    is_out_of_date,
                    view,
                    interaction,
                    guide_badges,
                    coop,
                },
            );
        });
    }

    fn draw_view_contents(
        &mut self, ui: &Ui, session: &mut Session, settings: &mut Settings, contents: ViewContents<'_>,
    ) {
        let ViewContents {
            id,
            kind,
            index,
            name,
            is_out_of_date,
            view,
            interaction,
            guide_badges,
            coop,
        } = contents;
        let MapViewState {
            camera,
            size,
            rect,
            visible,
            refit,
            hovered_coord,
            blame_popup,
            prefab_menu,
            gestures,
            context,
            ..
        } = view;
        if kind == ViewKind::Map {
            self.activate_on_focus(ui, session, settings, id, gestures);
        }

        let is_receiving = draw_coop_status(ui, session, id, name, is_out_of_date);
        if is_receiving || session.coop_transfer(id).is_some() {
            self.raise_demand(FrameDemand::Full);
        }

        if is_receiving {
            return;
        }

        let (image_size, extent) = match kind {
            ViewKind::Map => panel_extent(ui.content_region_avail()),
            ViewKind::StashEntry(area) => {
                ui.set_cursor_screen_pos(area.min);
                panel_extent([area.max[0] - area.min[0], area.max[1] - area.min[1]])
            },
        };
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
        let layout = ViewLayout::new(ui, extent, scale, kind);
        let mouse = ui.io().mouse_pos();
        let mut frame = ViewFrame {
            id,
            kind,
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

        if kind == ViewKind::Map && frame.is_hovered && !ui.io().want_text_input() {
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

        let is_blame_capturing = draw_tile_hover(ui, session, &mut frame, is_over_blame_popup);
        update_prefab_menu(ui, session, settings, id, frame.is_focused, prefab_menu);
        let node_hit = draw_node_hit(ui, session, &frame);
        configure_tool_interaction(session.tool(), frame.interaction);
        self.toggle_focus_on_key(ui, session, settings, &frame);

        if frame.is_active {
            self.draw_active_map(ui, session, settings, &mut frame, &node_hit, is_blame_capturing);
        }
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
            gestures.region_selection_anchor = None;
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
        self.apply_edit_command(session, frame);

        let tool = session.tool();
        let gizmo_context = (frame.id, tool, session.z());
        if self.gizmo_context != Some(gizmo_context) {
            self.gizmo.cancel();
            self.gizmo_context = Some(gizmo_context);
        }

        if tool != Tool::Move {
            self.move_stroke = None;
        }

        if frame
            .gestures
            .rectangle_gesture
            .is_some_and(|gesture| tool != Tool::Select || gesture.z != session.z())
        {
            restore_rectangle_gesture(session, frame.id, &mut frame.gestures.rectangle_gesture);
            frame.gestures.region_selection_anchor = None;
            self.gizmo.cancel();
        }

        let is_escape = self.cancel_on_escape(ui, session, frame, tool);
        self.draw_place_preview(ui, session, settings, frame, tool);

        sync_gestures(session, frame.gestures, tool);
        let conflict_hunks = session.conflict_hunks(frame.id, Some(session.z()));
        let is_gizmo_hot = tool == Tool::Pick
            && self.gizmo.is_handle_hovered(
                ui,
                session,
                frame.camera,
                self.inspector.transform_mode(),
                GizmoMapView {
                    min: frame.layout.viewport.min,
                    max: frame.layout.viewport.max,
                    hovered: frame.is_hovered && !ui.io().want_text_input(),
                },
            );
        let module_controls = panel::draw(ui, session, frame, is_escape, is_gizmo_hot);
        if let Some(path) = module_controls.open {
            self.open_map = Some(path);
        }

        let capture = MouseCapture {
            is_conflict_controls: conflict_controls_contain(ui, session, frame, &conflict_hunks),
            is_banner: banner_contains(ui, session, frame),
            is_blame_popup: is_blame_capturing || module_controls.is_capturing,
            is_escape,
        };
        let gizmo_view = GizmoMapView {
            min: frame.layout.viewport.min,
            max: frame.layout.viewport.max,
            hovered: frame.is_hovered && !capture.is_any() && !ui.io().want_text_input(),
        };
        let is_gizmo_capturing = self.draw_tool_gizmo(ui, session, settings, frame, tool, gizmo_view);
        session.set_module_bake_held(
            self.gizmo.is_interacting() || self.move_stroke.is_some_and(|stroke| stroke.is_dragging()),
        );

        if tool == Tool::Select
            && gizmo_view.hovered
            && !frame.gestures.is_busy()
            && settings.keybindings.get(KeybindAction::Rotate).is_pressed(ui)
        {
            session.rotate_selected_region();
        }

        let clicks = Clicks::read(ui);
        if clicks.is_right_clicked
            && !is_blame_capturing
            && !module_controls.is_capturing
            && let Some(coord) = frame.coord
        {
            apply_right_click(ui, session, frame, node_hit, coord);
        }

        if session.tool() == Tool::Node {
            drive_node_tool(
                session,
                frame,
                node_hit,
                clicks,
                capture.is_any() || ui.io().want_text_input(),
            );
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
        self.drag_fill_rect(ui, session, frame, clicks.is_left_down);
        self.draw_region_overlays(ui, session, frame, tool, gizmo_view);
        self.apply_map_menu(ui, session, settings, frame);

        self.draw_conflict_overlay_controls(ui, session, frame, &conflict_hunks);
        if frame.kind == ViewKind::Map {
            self.draw_top_overlays(ui, session, settings, frame);
            draw_recent_prefabs(ui, session, settings, frame);
        }

        configure_tool_interaction(session.tool(), frame.interaction);
        draw_fill_limit_warning(
            ui,
            session,
            &mut self.pending_fill_warning,
            self.fill_mode,
            &self.custom_fill_boundaries,
        );
        prepare_region_preview(session, frame);
    }

    fn cancel_view_edits(&mut self, session: &mut Session, frame: &mut ViewFrame<'_>) {
        frame.gestures.cancel(session, frame.id);
        self.cancel_tool_gestures();
        session.cancel_node_drag();
    }
}
