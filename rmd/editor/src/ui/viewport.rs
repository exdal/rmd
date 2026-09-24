use std::collections::HashSet;

use dear_imgui_rs::{Condition, Key, MouseButton, Ui, WindowKey, WindowKeyError};
use dmm::{Coord, Prefab};
use editor::{
    command::EditGroupId,
    conflict::Side,
    document::{DocumentId, MapDocument, Selection},
    icons::materialdesignicons::ICON_CIRCLE_SMALL,
    tool::{SelectionMask, SelectionPlacement, SelectionRotation, Tool, rotated_selection_at},
};
use render::{InteractionMode, MapViewInteraction, MapViewRect, PickRequest, PlacementFlash, Renderer};

use super::{
    BlamePopup,
    BlockPlacementAction,
    CLOSE_MAP_POPUP,
    FILL_LIMIT_WARNING_POPUP,
    FillWarningContext,
    NodeOverlayView,
    NodeRightClick,
    OVERLAY_PADDING,
    OverlayRect,
    PasteAction,
    PendingBlockPlacement,
    PendingFillWarning,
    PendingPaste,
    PlacementControls,
    RectangleGesture,
    TopOverlayState,
    UiState,
    VisibleMapView,
    blame_tooltip_position,
    block_controls_placement,
    block_placement_controls_layout,
    centered_paste_min,
    common::focus_window_on_hover,
    conflict_controls_layout,
    context_menu::{
        Action as MenuAction,
        NodeContext,
        POPUP as MAP_MENU_POPUP,
        Target as MenuTarget,
        draw_popup as draw_map_menu,
    },
    draw_blame_popup,
    draw_block_outline,
    draw_block_placement_controls,
    draw_conflict_controls,
    draw_conflict_tooltip,
    draw_diff_tooltip,
    draw_fill_limit_warning,
    draw_guide_badges,
    draw_highlights,
    draw_history_overlay,
    draw_node_overlay,
    draw_paste_controls,
    draw_placement_preview,
    draw_selected_pixel_grid,
    draw_tile_grid,
    draw_top_overlay,
    inspector::JumpTarget,
    node_right_click,
    paste_controls,
    recent_button_size,
    rectangle_drag_coord,
    request_level_change,
    restore_rectangle_gesture,
};
use crate::{
    camera::Controller,
    gizmo::{BlockGizmoKind, BlockGizmoTarget, GizmoMapView},
    session::{BlockPreviewSource, FillOutcome, GuideBadge, Session},
    settings::{KeybindAction, Settings},
};

const PLACEMENT_FLASH_DURATION: f64 = 0.25;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ActivePlacementFlash {
    owner: dmm::PrefabInstanceId,
    coord: Coord,
    started_at: f64,
}

impl ActivePlacementFlash {
    fn sample(self, now: f64) -> Option<PlacementFlash> {
        let elapsed = (now - self.started_at).max(0.0);
        if !elapsed.is_finite() || elapsed >= PLACEMENT_FLASH_DURATION {
            return None;
        }

        Some(PlacementFlash {
            owner: self.owner,
            strength: (1.0 - elapsed / PLACEMENT_FLASH_DURATION) as f32,
        })
    }
}

#[derive(Debug)]
pub(super) struct PlacementStroke {
    prefab: Prefab,
    z: u32,
    group: EditGroupId,
    visited: HashSet<Coord>,
}

impl PlacementStroke {
    fn new(prefab: Prefab, z: u32) -> Self {
        Self {
            prefab,
            z,
            group: EditGroupId::new(),
            visited: HashSet::new(),
        }
    }

    fn matches_context(&self, tool: Tool, prefab: Option<&Prefab>, z: u32) -> bool {
        tool == Tool::Place && prefab == Some(&self.prefab) && z == self.z
    }

    fn visit(&mut self, coord: Coord) -> Option<EditGroupId> {
        (coord.z == self.z && self.visited.insert(coord)).then_some(self.group)
    }
}

#[derive(Debug)]
pub(super) struct DeletionStroke {
    cursor: [u32; 2],
}

impl DeletionStroke {
    const fn new(cursor: [u32; 2]) -> Self { Self { cursor } }

    fn move_to(&mut self, cursor: [u32; 2]) -> bool {
        let moved = self.cursor != cursor;
        self.cursor = cursor;

        moved
    }
}

pub(super) struct MapViewState {
    window: WindowKey,
    pub(super) camera: Controller,
    size: (u32, u32),
    pub(super) rect: MapViewRect,
    visible: bool,
    pub(super) refit: bool,
    pub(super) focus: bool,
    hovered_coord: Option<Coord>,
    blame_popup: Option<BlamePopup>,
    pub(super) block_selection_anchor: Option<Coord>,
    pub(super) rectangle_gesture: Option<RectangleGesture>,
    pub(super) block_placement: Option<PendingBlockPlacement>,
    pub(super) paste: Option<PendingPaste>,
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
}

impl MapViewState {
    pub(super) fn new(id: DocumentId) -> Result<Self, WindowKeyError> {
        Ok(Self {
            window: WindowKey::new(format!("viewport-{}", id.get()), "Map View")?,
            camera: Controller::new(),
            size: (1, 1),
            rect: MapViewRect::default(),
            visible: false,
            refit: true,
            hovered_coord: None,
            blame_popup: None,
            focus: false,
            block_selection_anchor: None,
            rectangle_gesture: None,
            block_placement: None,
            paste: None,
            context: None,
        })
    }
}

fn active_placement_flash(
    active: &mut Option<ActivePlacementFlash>, now: f64, enabled: bool,
) -> Option<(Coord, PlacementFlash)> {
    if !enabled {
        *active = None;

        return None;
    }

    let placement = (*active)?;
    let Some(flash) = placement.sample(now) else {
        *active = None;

        return None;
    };

    Some((placement.coord, flash))
}

fn configure_tool_interaction(tool: Tool, interaction: &mut MapViewInteraction) {
    interaction.mode = match (tool, interaction.mode) {
        (Tool::Place | Tool::BlockSelect | Tool::Fill, _) => InteractionMode::Place,
        (Tool::Select, InteractionMode::Select { pick }) => InteractionMode::Select { pick },
        (Tool::Delete, InteractionMode::Delete { pick }) => InteractionMode::Delete { pick },
        (Tool::Select, _) => InteractionMode::Select { pick: None },
        (Tool::Delete, _) => InteractionMode::Delete { pick: None },
        (Tool::Node, _) => InteractionMode::Select { pick: None },
    };
    match tool {
        Tool::Place | Tool::Node | Tool::BlockSelect | Tool::Fill => {
            interaction.cursor = None;
            interaction.hovered_area = None;
            interaction.selected = None;
        },
        Tool::Delete => {
            interaction.selected = None;
        },
        Tool::Select => {},
    }
}

fn interaction_mode(tool: Tool) -> InteractionMode {
    match tool {
        Tool::Place | Tool::BlockSelect | Tool::Fill => InteractionMode::Place,
        Tool::Select => InteractionMode::Select { pick: None },
        Tool::Delete => InteractionMode::Delete { pick: None },
        Tool::Node => InteractionMode::Select { pick: None },
    }
}

fn request_pick(interaction: &mut MapViewInteraction, request: PickRequest) {
    match &mut interaction.mode {
        InteractionMode::Place => {},
        InteractionMode::Select { pick } | InteractionMode::Delete { pick } => *pick = Some(request),
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

fn has_modifiers(ui: &Ui) -> bool {
    let io = ui.io();

    io.key_ctrl() || io.key_shift() || io.key_alt() || io.key_super()
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

impl UiState {
    pub(super) fn draw_map_views(
        &mut self, ui: &Ui, session: &mut Session, settings: &mut Settings, refit_active: bool,
    ) -> (Vec<VisibleMapView>, Option<usize>) {
        let mut closing = None;
        let mut visible = Vec::new();

        for (&id, view) in &mut self.map_views {
            if view.rectangle_gesture.is_some_and(|gesture| {
                session.state.active() != Some(id)
                    || session.tool() != Tool::BlockSelect
                    || session
                        .state
                        .document(id)
                        .is_none_or(|document| document.z != gesture.z)
            }) {
                restore_rectangle_gesture(session, id, &mut view.rectangle_gesture);
                view.block_selection_anchor = None;
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
            if session.state.document(id).is_some_and(MapDocument::is_dirty) {
                self.pending_close = Some(id);
                ui.open_popup(CLOSE_MAP_POPUP);
            } else {
                session.close_map(id);
            }
        }
        self.draw_close_confirmation(ui, session);

        self.map_views.retain(|id, _| session.state.document(*id).is_some());

        let picking = visible.iter().position(|view| view.interaction.cursor.is_some());

        (visible, picking)
    }

    pub(super) fn draw_map_view(
        &mut self, ui: &Ui, session: &mut Session, settings: &mut Settings, draw: MapViewDraw<'_>,
    ) {
        let MapViewDraw {
            id,
            index: map_view_index,
            view,
            interaction,
            guide_badges,

            refit_requested,
            keep_open,
        } = draw;
        session.hide_block_preview(id);
        let Some(title) = session.state.document(id).map(MapDocument::title) else {
            return;
        };
        let MapViewState {
            window,
            camera,
            size: view_size,
            rect: view_rect,
            visible: view_visible,
            refit: view_refit,
            focus: view_focus,
            hovered_coord,
            blame_popup,
            block_selection_anchor,
            rectangle_gesture,
            block_placement,
            paste,
            context,
        } = view;
        *view_visible = false;
        *view_refit |= refit_requested;
        let refit = view_refit;

        if let Some(node) = self.central_node.or(self.dockspace_root) {
            ui.set_next_window_dock_id_with_cond(node, Condition::FirstUseEver);
        }
        let map_window = ui
            .window(window.label(title.as_str()))
            .opened(keep_open)
            .focused(std::mem::take(view_focus));
        map_window.build(|| {
            if settings.focus_windows_on_hover {
                focus_window_on_hover(ui);
            }
            if ui.is_window_focused() {
                if session.state.active() != Some(id) {
                    self.cancel_edit_gestures(session, session.state.active());
                }
                session.set_active_document(id);
            }
            let is_active = session.state.active() == Some(id);
            if !is_active && rectangle_gesture.is_some() {
                restore_rectangle_gesture(session, id, rectangle_gesture);
                *block_selection_anchor = None;
                self.gizmo.cancel();
            }
            let dock = ui.get_window_dock_id();
            if dock.raw() != 0 {
                self.central_node = Some(dock);
            }

            let (image_size, viewport) = panel_extent(ui.content_region_avail());
            *view_size = viewport;
            camera.resize(viewport.0, viewport.1);
            if *refit {
                let (width, height) = session.extent_px_of(id);
                camera.frame_map(width, height);
                *refit = false;
            }

            *view_visible = true;

            let origin = ui.cursor_screen_pos();
            let scale = ui.io().display_framebuffer_scale();
            let target = ui.io().display_size();
            *view_rect = framebuffer_rect(origin, image_size, scale, target);
            ui.image(Renderer::map_view_texture(map_view_index), image_size);

            let image_hovered = ui.is_item_hovered();
            let viewport_min = ui.item_rect_min();
            let viewport_max = ui.item_rect_max();

            let top_overlay_height = ui.frame_height() + OVERLAY_PADDING * 2.0;
            let history_overlay_height = recent_button_size(ui) + OVERLAY_PADDING * 2.0;
            let top_overlay = OverlayRect {
                min: viewport_min,
                max: [
                    viewport_max[0],
                    (viewport_min[1] + top_overlay_height).min(viewport_max[1]),
                ],
            };
            let bottom_overlay = OverlayRect {
                min: [
                    viewport_min[0],
                    (viewport_max[1] - history_overlay_height).max(viewport_min[1]),
                ],
                max: viewport_max,
            };

            let mouse = ui.io().mouse_pos();
            let over_overlay = top_overlay.contains(mouse) || bottom_overlay.contains(mouse);
            let hovered = image_hovered && !over_overlay && is_active;
            let focused = ui.is_window_focused();
            if focused
                && !ui.io().want_text_input()
                && let Some(index) = settings.keybindings.pressed_recent(ui)
            {
                session.choose_recent(index);
            }

            if hovered && !ui.io().want_text_input() {
                let io = ui.io();
                if !self.gizmo.is_interacting() && ui.is_mouse_down(MouseButton::Middle) {
                    camera.pan_by(io.mouse_delta());
                }

                let wheel = io.mouse_wheel();
                if !self.gizmo.is_interacting() && wheel != 0.0 {
                    let mouse = io.mouse_pos();
                    camera.zoom_by(wheel, [mouse[0] - viewport_min[0], mouse[1] - viewport_min[1]]);
                }

                if settings.keybindings.get(KeybindAction::ShowAreas).is_pressed(ui) {
                    session.toggle_areas();
                }
                if settings.keybindings.get(KeybindAction::ShowAreaOutlines).is_pressed(ui) {
                    session.toggle_area_outlines();
                }
                if settings.keybindings.get(KeybindAction::ShowLighting).is_pressed(ui) {
                    session.toggle_lighting();
                }
                if settings.keybindings.get(KeybindAction::ShowTileGrid).is_pressed(ui) {
                    settings.show_tile_grid = !settings.show_tile_grid;
                }
                if settings.keybindings.get(KeybindAction::ShowPixelGrid).is_pressed(ui) {
                    settings.show_selected_pixel_grid = !settings.show_selected_pixel_grid;
                }
                if settings.keybindings.get(KeybindAction::LevelUp).is_pressed(ui) {
                    request_level_change(session, 1, &mut self.new_level_dialog, &self.new_level_type_path);
                }
                if settings.keybindings.get(KeybindAction::LevelDown).is_pressed(ui) {
                    request_level_change(session, -1, &mut self.new_level_dialog, &self.new_level_type_path);
                }
                if settings.keybindings.get(KeybindAction::Refit).is_pressed(ui) {
                    *refit = true;
                }
                if settings.keybindings.get(KeybindAction::PlaceTool).is_pressed(ui) {
                    session.set_tool(Tool::Place);
                }
                if settings.keybindings.get(KeybindAction::SelectTool).is_pressed(ui) {
                    session.set_tool(Tool::Select);
                }
                if settings.keybindings.get(KeybindAction::NodeTool).is_pressed(ui) {
                    session.set_tool(Tool::Node);
                }
                if settings.keybindings.get(KeybindAction::BlockSelectTool).is_pressed(ui) {
                    session.set_tool(Tool::BlockSelect);
                }
                if settings.keybindings.get(KeybindAction::DeleteTool).is_pressed(ui) {
                    session.set_tool(Tool::Delete);
                }
                if settings.keybindings.get(KeybindAction::FillTool).is_pressed(ui) {
                    session.set_tool(Tool::Fill);
                }

                if *refit {
                    let (width, height) = session.extent_px();
                    camera.frame_map(width, height);
                    *refit = false;
                }
            }

            if settings.show_tile_grid {
                draw_tile_grid(
                    ui,
                    session,
                    camera,
                    settings.tile_grid_min_pixels,
                    settings.show_tile_grid_axis,
                    viewport_min,
                    viewport_max,
                );
            }
            if is_active
                && settings.show_selected_pixel_grid
                && let Some(transform) = session.selected_transform()
                && transform.sprite.z == session.z()
            {
                draw_selected_pixel_grid(
                    ui,
                    camera,
                    &transform,
                    session.options.tile_size,
                    settings.selected_pixel_grid_min_pixels,
                    settings.show_pixel_grid_axis,
                    viewport_min,
                    viewport_max,
                );
            }

            {
                // The hover comes from the previous frame, this runs before the cursor is resolved
                let highlights = session.highlights(id, *hovered_coord);
                let highlights = highlights.iter().collect::<Vec<_>>();
                draw_highlights(
                    ui,
                    camera,
                    OverlayRect {
                        min: viewport_min,
                        max: viewport_max,
                    },
                    &highlights,
                    session.options.tile_size,
                );
            }

            draw_guide_badges(ui, camera, viewport_min, viewport_max, guide_badges);

            let in_viewport = |point: [f32; 2]| {
                let local = [point[0] - viewport_min[0], point[1] - viewport_min[1]];

                local
                    .iter()
                    .all(|value| value.is_finite() && *value >= 0.0)
                    .then_some(local)
                    .filter(|local| local[0] < viewport.0 as f32 && local[1] < viewport.1 as f32)
            };
            let mouse_over_blame_popup = blame_popup
                .as_ref()
                .and_then(|popup| popup.bounds)
                .is_some_and(|bounds| bounds.contains(mouse));
            let cursor = (hovered && !mouse_over_blame_popup)
                .then(|| ui.io().mouse_pos())
                .and_then(in_viewport);
            let pointed_coord = cursor.and_then(|cursor| {
                let size = session.map()?.size;

                camera.screen_to_tile(cursor, size, session.options.tile_size, session.z())
            });
            *hovered_coord = pointed_coord;
            let hovered_conflict = pointed_coord.and_then(|coord| {
                session
                    .git_state(id)
                    .and_then(|git| git.conflicts.as_ref())
                    .and_then(|state| state.conflict_at(coord))
            });
            let hovered_diff = pointed_coord
                .filter(|_| hovered_conflict.is_none() && session.git_state(id).is_some_and(|git| git.show_diff))
                .and_then(|coord| session.diff_at(id, coord).map(|change| (coord, change)));
            let show_blame = session.git_state(id).is_some_and(|git| git.show_blame);
            let hovered_blame = pointed_coord
                .filter(|_| show_blame && hovered_conflict.is_none() && hovered_diff.is_none())
                .and_then(|coord| session.blame_at(id, coord).map(|cell| (coord, cell)));
            let hovered_commit = match hovered_blame {
                Some((coord, (editor::blame::BlameCell::Commit(..), false))) => Some(coord),
                _ => None,
            };
            let mut tooltip_position = None;
            if blame_popup.is_none() {
                if let (Some(coord), Some(conflict)) = (pointed_coord, hovered_conflict) {
                    ui.tooltip(|| draw_conflict_tooltip(ui, coord, conflict));
                } else if let Some((coord, (kind, before, after))) = hovered_diff
                    && let Some(state) = session.git_state(id).and_then(|git| git.diff.as_ref())
                {
                    let (from, to) = (state.from.label(), state.to.label());
                    ui.tooltip(|| draw_diff_tooltip(ui, coord, kind, (&from, before), (&to, after)));
                } else if let Some((_, (cell, changed))) = hovered_blame {
                    match cell {
                        editor::blame::BlameCell::Commit(_, commit) if !changed => {
                            let now = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map_or(0, |time| time.as_secs() as i64);
                            let detail = format!(
                                "{} {ICON_CIRCLE_SMALL} {} {ICON_CIRCLE_SMALL} {}\n{}\n",
                                commit.short,
                                commit.author,
                                editor::blame::relative_time(now, commit.time),
                                commit.summary
                            );
                            ui.tooltip(|| {
                                ui.text(detail);
                                tooltip_position = Some(ui.window_pos());
                            });
                        },
                        editor::blame::BlameCell::Boundary if !changed => {
                            if let Some(label) = session
                                .git_state(id)
                                .and_then(|git| git.blame.as_ref())
                                .map(|blame| blame.result.boundary_label())
                            {
                                ui.tooltip_text(label);
                            }
                        },
                        _ => {
                            if let Some(note) = editor::blame::pending_note(cell, changed) {
                                ui.tooltip_text(note);
                            }
                        },
                    }
                }
            }

            if !show_blame {
                *blame_popup = None;
            }

            let blame_clicked = ui.is_mouse_clicked(MouseButton::Left) && hovered_commit.is_some();
            if let Some(coord) = hovered_commit.filter(|_| blame_clicked) {
                let position = blame_popup
                    .as_ref()
                    .filter(|popup| popup.coord == coord)
                    .map(|popup| popup.position)
                    .or(tooltip_position)
                    .unwrap_or_else(|| blame_tooltip_position(ui, mouse));
                *blame_popup = Some(BlamePopup {
                    coord,
                    position,
                    bounds: None,
                });
            } else if ui.is_mouse_clicked(MouseButton::Left) && !mouse_over_blame_popup {
                *blame_popup = None;
            }

            let blame_popup_captures_mouse = blame_clicked || mouse_over_blame_popup;
            if let Some(popup) = blame_popup.as_mut() {
                let commit = match session.blame_at(id, popup.coord) {
                    Some((editor::blame::BlameCell::Commit(_, commit), false)) => Some(commit),
                    _ => None,
                };

                if let Some(commit) = commit {
                    let web = session.git_state(id).and_then(|git| git.web.as_ref());
                    if let Some((bounds, close)) = draw_blame_popup(ui, id, popup, commit, web) {
                        popup.bounds = Some(bounds);
                        if close {
                            *blame_popup = None;
                        }
                    }
                } else {
                    *blame_popup = None;
                }
            }

            let node_interactive = hovered && !session.node_dragging();
            let node_hit = (is_active && session.tool() == Tool::Node)
                .then(|| session.node_overlay())
                .flatten()
                .map(|overlay| {
                    draw_node_overlay(
                        ui,
                        &overlay,
                        NodeOverlayView {
                            camera,
                            viewport_min,
                            viewport_max,
                            tile_size: session.options.tile_size,
                            hovered_tile: pointed_coord,
                            interactive: node_interactive,
                        },
                    )
                })
                .unwrap_or_default();

            configure_tool_interaction(session.tool(), interaction);

            if hovered
                && !ui.io().want_text_input()
                && !has_modifiers(ui)
                && !settings.keybindings.is_any_pressed(ui)
                && ui.is_key_pressed(Key::F)
            {
                session.toggle_focus_at(pointed_coord);
                self.placement_stroke = None;
            }

            if is_active {
                if focused && !ui.io().want_text_input() {
                    let undo = settings.keybindings.get(KeybindAction::Undo).is_pressed_repeating(ui);
                    let redo = settings.keybindings.get(KeybindAction::Redo).is_pressed_repeating(ui);
                    if undo || redo {
                        restore_rectangle_gesture(session, id, rectangle_gesture);
                        self.gizmo.cancel();
                        self.placement_flash = None;
                        self.placement_stroke = None;
                        self.deletion_stroke = None;
                        *block_selection_anchor = None;
                        *block_placement = None;
                        *paste = None;
                        session.cancel_node_drag();

                        if undo {
                            session.undo();
                        } else {
                            session.redo();
                        }
                    }
                    if settings.keybindings.get(KeybindAction::Copy).is_pressed(ui) {
                        session.copy_selection(session.selection_mode());
                    }
                    if settings.keybindings.get(KeybindAction::Paste).is_pressed(ui)
                        && let Some(block) = session.clipboard()
                    {
                        let (width, height) = (block.width(), block.height());
                        restore_rectangle_gesture(session, id, rectangle_gesture);
                        let anchor = pointed_coord.or_else(|| {
                            let size = session.map()?.size;
                            let center = [viewport.0 as f32 * 0.5, viewport.1 as f32 * 0.5];

                            camera.screen_to_tile(center, size, session.options.tile_size, session.z())
                        });
                        if let (Some(anchor), Some(size)) = (anchor, session.map().map(|map| map.size)) {
                            session.set_tool(Tool::BlockSelect);
                            self.gizmo.cancel();
                            *block_placement = None;
                            *block_selection_anchor = None;
                            *paste = Some(PendingPaste {
                                min: centered_paste_min(width, height, anchor, size),
                                rotation: SelectionRotation::Original,
                            });
                        }
                    }
                }

                let tool = session.tool();
                let gizmo_context = (id, tool, session.z());
                if self.gizmo_context != Some(gizmo_context) {
                    self.gizmo.cancel();
                    self.gizmo_context = Some(gizmo_context);
                }
                if rectangle_gesture.is_some_and(|gesture| tool != Tool::BlockSelect || gesture.z != session.z()) {
                    restore_rectangle_gesture(session, id, rectangle_gesture);
                    *block_selection_anchor = None;
                    self.gizmo.cancel();
                }
                let escape = focused
                    && !ui.io().want_text_input()
                    && !self.popup_was_open
                    && !ui.is_popup_open_with_flags("", dear_imgui_rs::PopupQueryFlags::ANY_POPUP)
                    && ui.is_key_pressed(Key::Escape);
                if escape {
                    if rectangle_gesture.is_some() {
                        restore_rectangle_gesture(session, id, rectangle_gesture);
                        *block_selection_anchor = None;
                    } else if block_placement.is_some() || paste.is_some() {
                        *block_placement = None;
                        *paste = None;
                    } else if matches!(tool, Tool::BlockSelect | Tool::Fill) {
                        session.select_block(None);
                    }
                    self.gizmo.cancel();
                    session.cancel_node_drag();
                }
                let preview_coord = (tool == Tool::Place)
                    .then(|| self.gizmo.placement_coord().or(pointed_coord))
                    .flatten();
                let active_flash =
                    active_placement_flash(&mut self.placement_flash, ui.time(), settings.tile_place_flash);
                interaction.placement_flash = active_flash.map(|(_, flash)| flash);

                if let Some(coord) = preview_coord
                    && session.can_edit_at(coord)
                    && active_flash.is_none_or(|(flash_coord, _)| flash_coord != coord)
                {
                    draw_placement_preview(ui, session, camera, coord, viewport_min, viewport_max);
                }

                let gizmo_map_view = GizmoMapView {
                    min: viewport_min,
                    max: viewport_max,
                    hovered,
                };

                if tool != Tool::BlockSelect {
                    *block_selection_anchor = None;
                    *block_placement = None;
                    *paste = None;
                } else {
                    if block_selection_anchor.is_some_and(|anchor| anchor.z != session.z()) {
                        *block_selection_anchor = None;
                    }

                    if block_placement.is_some_and(|placement| Some(placement.source) != session.selection()) {
                        *block_placement = None;
                    }

                    if let Some(pending) = paste.as_mut() {
                        pending.min.z = session.z();
                    }
                }

                let paste_mask =
                    paste.and_then(|pending| session.clipboard_selection_mask(pending.min, pending.rotation));
                let paste_target = paste_mask.map(|mask| mask.bounds);
                if paste.is_some() && paste_target.is_none() {
                    *paste = None;
                }

                let block_controls_area = OverlayRect {
                    min: [viewport_min[0], top_overlay.max[1]],
                    max: [viewport_max[0], bottom_overlay.min[1]],
                };
                let conflict_regions = session.conflict_regions(id, Some(session.z()));
                let theirs_label = session
                    .git_state(id)
                    .and_then(|git| git.conflicts.as_ref())
                    .map_or(String::from("theirs"), |conflicts| {
                        conflicts.side_label(Side::Theirs).to_owned()
                    });
                let conflict_controls_capture_mouse = conflict_regions.iter().any(|region| {
                    conflict_controls_layout(
                        ui,
                        camera,
                        region,
                        session.options.tile_size,
                        viewport_min,
                        block_controls_area,
                        &theirs_label,
                    )
                    .is_some_and(|(_, bounds)| bounds.contains(mouse))
                });
                let banner_capture_mouse = session.git_state(id).is_some_and(|git| git.pending_load)
                    && mouse[0] >= viewport_min[0] + 8.0
                    && mouse[0] <= viewport_min[0] + 320.0
                    && mouse[1] >= top_overlay.max[1] + 4.0
                    && mouse[1] <= top_overlay.max[1] + ui.frame_height() + 16.0;

                let controls_hit_test = match *paste {
                    Some(_) => paste_controls(self.gizmo.block_rotation_open(), paste_target)
                        .map(|target| (PlacementControls::Paste, target)),
                    None => block_controls_placement(
                        tool,
                        rectangle_gesture.is_some(),
                        self.gizmo.block_rotation_open(),
                        session.selection(),
                        *block_placement,
                    )
                    .map(|placement| (PlacementControls::Block, placement.target)),
                };
                let block_controls_capture_mouse = controls_hit_test.is_some_and(|(controls, target)| {
                    let (_, bounds) = block_placement_controls_layout(
                        ui,
                        camera,
                        controls,
                        target,
                        session.options.tile_size,
                        viewport_min,
                        block_controls_area,
                    );

                    bounds.contains(mouse)
                });

                let gizmo_map_view = GizmoMapView {
                    hovered: hovered
                        && !block_controls_capture_mouse
                        && !conflict_controls_capture_mouse
                        && !banner_capture_mouse
                        && !blame_popup_captures_mouse
                        && !escape
                        && !ui.io().want_text_input(),
                    ..gizmo_map_view
                };

                let gizmo_captures_mouse = match tool {
                    Tool::Select => {
                        self.gizmo
                            .draw(
                                ui,
                                session,
                                settings,
                                camera,
                                self.inspector.transform_mode(),
                                gizmo_map_view,
                            )
                            .captures_mouse
                    },
                    Tool::Place => {
                        self.gizmo
                            .draw_placement_direction(ui, session, settings, camera, pointed_coord, gizmo_map_view)
                            .captures_mouse
                    },
                    Tool::Node => {
                        self.gizmo.cancel();

                        false
                    },
                    Tool::BlockSelect => {
                        if let (Some(pending), Some(target), Some(size)) =
                            (*paste, paste_target, session.map().map(|map| map.size))
                        {
                            let response = self.gizmo.draw_block(
                                ui,
                                settings,
                                camera,
                                BlockGizmoTarget {
                                    selection: target,
                                    rotation: pending.rotation,
                                    map_size: size,
                                    tile_size: session.options.tile_size,
                                    kind: BlockGizmoKind::Clipboard,
                                },
                                gizmo_map_view,
                            );

                            if response.selection.min != target.min || response.rotation != pending.rotation {
                                *paste = Some(PendingPaste {
                                    min: response.selection.min,
                                    rotation: response.rotation,
                                });
                            }

                            response.captures_mouse
                        } else if block_selection_anchor.is_some() {
                            self.gizmo.cancel();

                            false
                        } else if let (Some(source), Some(size)) =
                            (session.selection(), session.map().map(|map| map.size))
                        {
                            let (displayed, rotation) = block_placement
                                .map_or((source, SelectionRotation::Original), |placement| {
                                    (placement.target, placement.rotation)
                                });

                            let response = self.gizmo.draw_block(
                                ui,
                                settings,
                                camera,
                                BlockGizmoTarget {
                                    selection: displayed,
                                    rotation,
                                    map_size: size,
                                    tile_size: session.options.tile_size,
                                    kind: if block_placement.is_some() {
                                        BlockGizmoKind::Placement
                                    } else {
                                        BlockGizmoKind::Selection
                                    },
                                },
                                gizmo_map_view,
                            );

                            if response.resizing {
                                rectangle_gesture.get_or_insert(RectangleGesture {
                                    start: session.selection_mask(),
                                    z: session.z(),
                                });
                                session.try_select_block(SelectionMask {
                                    bounds: response.selection,
                                    mode: session.selection_mode(),
                                });
                            } else if response.selection != displayed || response.rotation != rotation {
                                *block_selection_anchor = None;
                                *block_placement =
                                    rotated_selection_at(source, response.selection.min, response.rotation)
                                        .filter(|target| {
                                            *target != source || response.rotation != SelectionRotation::Original
                                        })
                                        .map(|target| PendingBlockPlacement {
                                            source,
                                            target,
                                            rotation: response.rotation,
                                        });
                            }

                            response.captures_mouse
                        } else {
                            self.gizmo.cancel();

                            false
                        }
                    },
                    Tool::Delete => {
                        self.gizmo.cancel();

                        false
                    },
                    Tool::Fill => {
                        self.gizmo.cancel();

                        false
                    },
                };
                let left_clicked = ui.is_mouse_clicked(MouseButton::Left);
                let left_down = ui.is_mouse_down(MouseButton::Left);
                let left_double_clicked = ui.is_mouse_double_clicked(MouseButton::Left);
                let right_clicked = ui.is_mouse_clicked(MouseButton::Right);
                if right_clicked
                    && !blame_popup_captures_mouse
                    && let Some(coord) = pointed_coord
                {
                    match node_right_click(session.tool(), ui.io().key_shift(), &node_hit) {
                        NodeRightClick::DeleteStandalone(node) => {
                            session.delete_standalone_node(node);
                        },
                        NodeRightClick::DeleteConnection(connection) => {
                            session.delete_node_connection(connection);
                        },
                        NodeRightClick::Menu => {
                            let atoms = session
                                .state
                                .active_document()
                                .map(|document| document.instance_ids_at(coord).to_vec())
                                .unwrap_or_default();
                            let node = if session.tool() == Tool::Node {
                                if let Some(standalone) = node_hit.standalone {
                                    Some(NodeContext::Standalone(standalone))
                                } else if let Some(connection) = node_hit.connection.clone() {
                                    Some(NodeContext::Connection(connection))
                                } else if node_hit.handle.is_some() {
                                    None
                                } else {
                                    cursor.map(|cursor| NodeContext::Pick(coord, cursor_in_map_view(cursor, scale)))
                                }
                            } else {
                                None
                            };
                            let block_selected = session.tool() == Tool::BlockSelect
                                && session
                                    .selection()
                                    .is_some_and(|selection| session.selection_mode().includes(selection, coord));
                            *context = Some(MenuTarget {
                                document: id,
                                coord,
                                atoms,
                                node,
                                block_selected,
                                replace_for: None,
                                replace_query: String::new(),
                            });
                            ui.open_popup(MAP_MENU_POPUP);
                        },
                    }
                }

                if session.tool() == Tool::Node {
                    if !focused {
                        session.cancel_node_drag();
                    }
                    if !blame_popup_captures_mouse
                        && left_double_clicked
                        && let Some(coord) = pointed_coord
                        && let Some(cursor) = cursor
                    {
                        interaction.cursor = Some(cursor_in_map_view(cursor, scale));
                        request_pick(interaction, PickRequest::NodeSeed(coord));
                    } else if !blame_popup_captures_mouse
                        && left_clicked
                        && let Some(coord) = node_hit.handle
                    {
                        session.start_node_drag(coord);
                    }

                    if session.node_dragging() {
                        if left_down {
                            if let Some(coord) = pointed_coord {
                                session.update_node_drag(coord);
                            }
                        } else {
                            session.finish_node_drag(hovered && pointed_coord.is_some());
                        }
                    }
                }
                if left_clicked {
                    self.placement_stroke = None;
                    self.deletion_stroke = None;
                }
                if !left_down || session.tool() != Tool::Delete {
                    self.deletion_stroke = None;
                }
                if self.placement_stroke.as_ref().is_some_and(|stroke| {
                    !left_down
                        || gizmo_captures_mouse
                        || !stroke.matches_context(session.tool(), session.palette(), session.z())
                }) {
                    self.placement_stroke = None;
                }
                if !escape
                    && !gizmo_captures_mouse
                    && !block_controls_capture_mouse
                    && !conflict_controls_capture_mouse
                    && !banner_capture_mouse
                    && !blame_popup_captures_mouse
                    && session.tool() != Tool::Node
                    && let Some(cursor) = cursor
                {
                    let pixel = [cursor[0].floor() as u32, cursor[1].floor() as u32];
                    interaction.cursor = Some(cursor_in_map_view(cursor, scale));

                    if let Some(coord) = pointed_coord {
                        interaction.hovered_area = session.area_at(id, coord);
                        match session.tool() {
                            Tool::Place => {
                                if left_clicked {
                                    self.placement_stroke = session
                                        .palette()
                                        .cloned()
                                        .map(|prefab| PlacementStroke::new(prefab, session.z()));
                                }
                                let group = session
                                    .can_edit_at(coord)
                                    .then(|| self.placement_stroke.as_mut().and_then(|stroke| stroke.visit(coord)))
                                    .flatten();
                                let placed = group.and_then(|group| session.place_at(coord, Some(group)));
                                if settings.tile_place_flash
                                    && let Some(owner) = placed
                                {
                                    let flash = ActivePlacementFlash {
                                        owner,
                                        coord,
                                        started_at: ui.time(),
                                    };
                                    self.placement_flash = Some(flash);
                                    interaction.placement_flash = flash.sample(ui.time());
                                }
                            },
                            Tool::Select => {
                                self.placement_stroke = None;
                                if left_double_clicked {
                                    request_pick(interaction, PickRequest::NodeSeed(coord));
                                } else if left_clicked {
                                    request_pick(interaction, PickRequest::Select);
                                }
                            },
                            Tool::Node => {},
                            Tool::BlockSelect => {
                                self.placement_stroke = None;
                                if block_placement.is_none()
                                    && paste.is_none()
                                    && left_clicked
                                    && session.can_edit_at(coord)
                                {
                                    *block_placement = None;
                                    let selection = Selection::from_drag(coord, coord);
                                    *rectangle_gesture = Some(RectangleGesture {
                                        start: session.selection_mask(),
                                        z: session.z(),
                                    });
                                    if session.try_select_block(SelectionMask {
                                        bounds: selection,
                                        mode: self.block_selection_options.drawing_mode(ui.io().key_shift()),
                                    }) {
                                        *block_selection_anchor = Some(coord);
                                    }
                                }
                            },
                            Tool::Delete => {
                                self.placement_stroke = None;
                                let requests_pick = if left_clicked {
                                    self.deletion_stroke = Some(DeletionStroke::new(pixel));

                                    true
                                } else {
                                    left_down
                                        && self
                                            .deletion_stroke
                                            .as_mut()
                                            .is_some_and(|stroke| stroke.move_to(pixel))
                                };
                                if requests_pick {
                                    request_pick(interaction, PickRequest::Delete);
                                }
                            },
                            Tool::Fill => {
                                self.placement_stroke = None;
                                if session.selection_mask().is_some_and(|mask| !mask.includes(coord)) {
                                    ui.tooltip_text("Click a selected tile, or Clear selection to fill elsewhere");
                                }
                                if left_clicked
                                    && let FillOutcome::TooLarge { limit } =
                                        session.fill_at(coord, self.fill_mode, &self.custom_fill_boundaries)
                                {
                                    self.pending_fill_warning = Some(PendingFillWarning {
                                        context: FillWarningContext::capture(session),
                                        coord,
                                        fill_mode: self.fill_mode,
                                        custom_fill_boundaries: self.custom_fill_boundaries.clone(),
                                        limit,
                                    });
                                    ui.open_popup(FILL_LIMIT_WARNING_POPUP);
                                }
                            },
                        }
                    }
                }

                if let Some(anchor) = *block_selection_anchor
                    && let Some(size) = session.map().map(|map| map.size)
                    && let Some(coord) =
                        rectangle_drag_coord(camera, mouse, viewport_min, size, session.options.tile_size, anchor.z)
                {
                    session.try_select_block(SelectionMask {
                        bounds: Selection::from_drag(anchor, coord),
                        mode: if left_down {
                            self.block_selection_options.drawing_mode(ui.io().key_shift())
                        } else {
                            session.selection_mode()
                        },
                    });
                }

                if !left_down {
                    *block_selection_anchor = None;
                    *rectangle_gesture = None;
                }

                let overlay_viewport = OverlayRect {
                    min: viewport_min,
                    max: viewport_max,
                };

                let block_overlay = match (*paste, paste_mask) {
                    (Some(pending), Some(mask)) => {
                        let target = mask.bounds;
                        draw_block_outline(ui, session, camera, target, mask.mode, overlay_viewport);

                        Some((target, pending.rotation))
                    },
                    _ if matches!(session.tool(), Tool::BlockSelect | Tool::Fill) => {
                        session.selection().map(|source| {
                            let (displayed, rotation) = block_placement
                                .map_or((source, SelectionRotation::Original), |placement| {
                                    (placement.target, placement.rotation)
                                });
                            draw_block_outline(
                                ui,
                                session,
                                camera,
                                displayed,
                                session.selection_mode(),
                                overlay_viewport,
                            );

                            (displayed, rotation)
                        })
                    },
                    _ => None,
                };

                if let Some((displayed, rotation)) = block_overlay
                    && tool == Tool::BlockSelect
                    && block_selection_anchor.is_none()
                    && let Some(map_size) = session.map().map(|map| map.size)
                {
                    self.gizmo.draw_block_overlay(
                        ui,
                        camera,
                        BlockGizmoTarget {
                            selection: displayed,
                            rotation,
                            map_size,
                            tile_size: session.options.tile_size,
                            kind: if paste.is_some() {
                                BlockGizmoKind::Clipboard
                            } else if block_placement.is_some() {
                                BlockGizmoKind::Placement
                            } else {
                                BlockGizmoKind::Selection
                            },
                        },
                        gizmo_map_view,
                    );
                }

                if let Some(target) = context.as_mut()
                    && let Some(action) = draw_map_menu(ui, session, settings, target)
                {
                    if matches!(
                        action,
                        MenuAction::Undo
                            | MenuAction::Redo
                            | MenuAction::Paste(_)
                            | MenuAction::Cut(_)
                            | MenuAction::Delete(_)
                    ) {
                        restore_rectangle_gesture(session, id, rectangle_gesture);
                        self.gizmo.cancel();
                        self.placement_flash = None;
                        self.placement_stroke = None;
                        self.deletion_stroke = None;
                        *block_selection_anchor = None;
                        *block_placement = None;
                        *paste = None;
                        session.cancel_node_drag();
                    }
                    match action {
                        MenuAction::Conflict(coords, side) => {
                            session.resolve_conflict(id, &coords, side);
                        },
                        MenuAction::BlameCopy(hash) => {
                            self.copy_to_clipboard = Some(hash);
                        },
                        MenuAction::BlamePin(commit) => {
                            session.pin_blame(id, commit);
                        },
                        MenuAction::BlameRun => {
                            session.run_blame(id, settings.blame_depth as usize);
                        },
                        MenuAction::Restore(coords) => {
                            session.restore_diff(id, &coords);
                        },
                        MenuAction::Undo => {
                            session.undo();
                        },
                        MenuAction::Redo => {
                            session.redo();
                        },
                        MenuAction::Copy(coord) => {
                            session.copy_tile(coord);
                        },
                        MenuAction::Paste(coord) => {
                            session.paste_clipboard(coord, SelectionRotation::Original);
                        },
                        MenuAction::Cut(coord) => {
                            session.cut_tile(coord);
                        },
                        MenuAction::Delete(coord) => {
                            session.delete_tile(coord);
                        },
                        MenuAction::Select(instance) => {
                            session.select_instance(Some(instance));
                            self.reveal_selected_instance(session);
                        },
                        MenuAction::DeleteAtom(instance) => {
                            session.delete_context_instance(instance);
                        },
                        MenuAction::Reorder(instance, to_top) => {
                            session.reorder_instance(instance, to_top);
                        },
                        MenuAction::Reset(instance) => {
                            session.reset_instance_to_default(instance);
                        },
                        MenuAction::Replace(instance, path) => {
                            session.replace_context_instance(instance, path);
                        },
                        MenuAction::Search(instance, kind) => {
                            self.inspector.open_similar_instances_for(session, id, instance, kind);
                        },
                        MenuAction::Node(node) => match node {
                            NodeContext::Standalone(coord) => {
                                session.delete_standalone_node(coord);
                            },
                            NodeContext::Connection(connection) => {
                                session.delete_node_connection(&connection);
                            },
                            NodeContext::Pick(coord, pixel) => {
                                interaction.cursor = Some(pixel);
                                request_pick(interaction, PickRequest::NodeDelete(coord));
                            },
                        },
                        MenuAction::Mirror(transform) => {
                            session.transform_selected_block_with_mode(transform, session.selection_mode());
                        },
                    }
                }
                let paste_action = paste
                    .zip(paste_controls(self.gizmo.block_rotation_open(), paste_target))
                    .and_then(|(pending, target)| {
                        let can_paste = session.can_paste_clipboard(pending.min, pending.rotation);

                        draw_paste_controls(
                            ui,
                            camera,
                            target,
                            session.options.tile_size,
                            viewport_min,
                            block_controls_area,
                            can_paste,
                        )
                        .map(|action| (action, pending))
                    });
                draw_conflict_controls(
                    ui,
                    session,
                    id,
                    camera,
                    &conflict_regions,
                    viewport_min,
                    block_controls_area,
                );

                let controls_placement = paste
                    .is_none()
                    .then(|| {
                        block_controls_placement(
                            tool,
                            rectangle_gesture.is_some(),
                            self.gizmo.block_rotation_open(),
                            session.selection(),
                            *block_placement,
                        )
                    })
                    .flatten();
                let placement_action = controls_placement.and_then(|placement| {
                    draw_block_placement_controls(
                        ui,
                        camera,
                        placement,
                        viewport_min,
                        block_controls_area,
                        session,
                        &mut self.block_selection_options,
                    )
                    .map(|action| (action, placement))
                });
                draw_top_overlay(
                    ui,
                    session,
                    top_overlay,
                    TopOverlayState {
                        keybindings: settings.keybindings,
                        selection_busy: rectangle_gesture.is_some() || block_placement.is_some() || paste.is_some(),
                        block_selection_options: &mut self.block_selection_options,
                        fill_mode: &mut self.fill_mode,
                        custom_fill_boundaries: &mut self.custom_fill_boundaries,
                        custom_fill_search: &mut self.custom_fill_search,
                        new_level_dialog: &mut self.new_level_dialog,
                        new_level_type_path: &self.new_level_type_path,
                    },
                );
                if is_active && session.git_state(id).is_some_and(|git| git.pending_load) {
                    ui.set_cursor_screen_pos([viewport_min[0] + 12.0, top_overlay.max[1] + 8.0]);
                    ui.text("Merge conflicts on disk");
                    ui.same_line();
                    if ui.small_button("Load conflicts") {
                        self.pending_conflict_reload = Some(id);
                    }
                }
                if let Some((action, pending)) = paste_action {
                    if action == PasteAction::Paste {
                        session.paste_clipboard(pending.min, pending.rotation);
                    }
                    *paste = None;
                    self.gizmo.cancel();
                }
                if let Some((action, placement)) = placement_action {
                    let finished = match action {
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
                        BlockPlacementAction::Fill => session.fill_selected_block(
                            placement.target.min,
                            placement.rotation,
                            session.selection_mode(),
                        ),
                        BlockPlacementAction::Cancel => {
                            if block_placement.is_none() {
                                session.select_block(None);
                            }

                            true
                        },
                    };
                    if finished {
                        *block_placement = None;
                        self.gizmo.cancel();
                    }
                }
                let recent_prefabs = session.recent_prefabs();
                if !recent_prefabs.is_empty() {
                    draw_history_overlay(
                        ui,
                        session,
                        settings.keybindings,
                        bottom_overlay,
                        recent_prefabs.to_vec(),
                    );
                }
                configure_tool_interaction(session.tool(), interaction);
                if session.focused_area().is_some() {
                    interaction.hovered_area = None;
                }

                draw_fill_limit_warning(
                    ui,
                    session,
                    &mut self.pending_fill_warning,
                    self.fill_mode,
                    &self.custom_fill_boundaries,
                );

                if let Some(pending) = *paste {
                    session.prepare_block_preview(id, BlockPreviewSource::Clipboard, pending.min, pending.rotation);
                } else if let Some(placement) = *block_placement
                    && session.tool() == Tool::BlockSelect
                {
                    session.prepare_block_preview(
                        id,
                        BlockPreviewSource::Selection(SelectionMask {
                            bounds: placement.source,
                            mode: session.selection_mode(),
                        }),
                        placement.target.min,
                        placement.rotation,
                    );
                }
            }
        });
    }

    pub(super) fn jump_to_instance(&mut self, session: &mut Session, target: JumpTarget) {
        let Some(location) = session
            .state
            .document(target.document)
            .and_then(|document| document.instance_location(target.instance))
        else {
            return;
        };

        self.cancel_edit_gestures(session, session.state.active());
        session.set_active_document(target.document);
        session.set_level(location.coord.z);
        if let Some(document) = session.state.active_document_mut() {
            document.set_focus(None);
            document.selection = None;
        }
        session.select_instance(Some(target.instance));

        let view = match self.map_views.get_mut(&target.document) {
            Some(view) => view,
            None => {
                let Ok(view) = MapViewState::new(target.document) else {
                    log::error!(
                        "could not create a map view window for document {}",
                        target.document.get()
                    );

                    return;
                };
                self.map_views.insert(target.document, view);
                self.map_views
                    .get_mut(&target.document)
                    .expect("the inserted map view is available")
            },
        };
        view.camera.center_on_tile(location.coord, session.options.tile_size);
        view.refit = false;
        view.focus = true;
    }
}

#[cfg(test)]
mod tests {
    use core::path::TreePath;
    use std::path::PathBuf;

    use dear_imgui_rs::{Condition, DockLayout, Key, MouseButton};
    use dmm::{Coord, Prefab, PrefabInstanceId, Size};
    use editor::{
        conflict::Side,
        document::{DocumentId, MapDocument, Selection},
        tool::{BlockSelectionMode, SelectionMask, SelectionRotation, Tool},
    };
    use render::{HighlightStyle, InteractionMode, MapViewInteraction, MapViewRect, PickRequest, PlacementFlash};

    use super::{
        ActivePlacementFlash,
        DeletionStroke,
        MapViewState,
        PlacementStroke,
        active_placement_flash,
        configure_tool_interaction,
        cursor_in_map_view,
        framebuffer_rect,
        panel_extent,
        request_pick,
    };
    use crate::{
        camera::Controller,
        gizmo::{BlockGizmoKind, BlockGizmoTarget, GizmoMapView, GizmoState},
        session::{
            LevelChange,
            Session,
            fixtures::{install_blame, node_map, node_session, node_tile_has_group},
        },
        settings::{KeyBinding, KeybindAction, KeybindPreset, Settings},
        ui::{
            IMGUI_CONTEXT,
            RectangleGesture,
            block::block_selection_bounds,
            context_menu::NodeContext,
            fixtures::{RectangleUiHarness, rectangle_context},
            inspector::TransformMode,
            restore_rectangle_gesture,
        },
    };

    #[test]
    fn clicking_a_blamed_tile_opens_a_popup_without_placing_a_tile() {
        struct OneVersion {
            commits: Vec<editor::git::CommitInfo>,
            map: dmm::Map,
        }
        impl editor::blame::VersionSource for OneVersion {
            fn commits(&self) -> &[editor::git::CommitInfo] { &self.commits }

            fn truncated(&self) -> bool { false }

            fn map_at(&mut self, _: usize) -> Result<editor::blame::MapVersion, editor::git::GitError> {
                Ok(editor::blame::MapVersion::Present(self.map.clone()))
            }
        }

        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut session = Session::new();
        session
            .load_environment(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env/test.dme"))
            .unwrap();
        let mut map = dmm::Map::new(Size { x: 20, y: 20, z: 1 });
        let floor = Prefab::new(TreePath::parse("/turf/open/floor"));
        let key = map.intern_tile(vec![floor.clone()]);
        for row in &mut map.grid[0] {
            row.fill(key);
        }
        let mut source = OneVersion {
            commits: vec![editor::git::CommitInfo {
                hash: String::from("a").repeat(40),
                short: String::from("aaaaaaa"),
                author: String::from("Map author"),
                time: 1,
                summary: String::from("Paint the floor"),
                pull_request: None,
            }],
            map: map.clone(),
        };
        let blame = editor::blame::blame(&map, &mut source, &|| false, &mut |_, _| {}).unwrap();
        session.apply_map(crate::loader::LoadedMap {
            path: PathBuf::from("blame-ui-test.dmm"),
            map,
            z: 1,
            errors: vec![],
            repo: Some(editor::git::RepoPath {
                root: PathBuf::from("."),
                git_dir: PathBuf::from(".git"),
                rel: String::from("blame-ui-test.dmm"),
            }),
            conflict: None,
        });
        let id = session.state.active().unwrap();
        install_blame(&mut session, id, blame);
        session.set_tool(Tool::Place);
        session
            .state
            .choose_prefab(Prefab::new(TreePath::parse("/turf/closed/wall")));

        let mut app = RectangleUiHarness::with_session(session);
        let tile = app.tile(5, 8);
        app.pointer(tile, false);
        assert!(app.view.blame_popup.is_none(), "hovering only shows the normal tooltip");
        app.click(tile);

        let popup = app.view.blame_popup.as_ref().expect("clickable popup");
        assert_eq!(popup.coord, Coord::new(5, 8, 1));
        assert_eq!(app.session.undo_label(), None);
        assert_eq!(
            app.session.map().unwrap().tile_at(Coord::new(5, 8, 1)).unwrap()[0].path,
            floor.path
        );
    }

    #[test]
    fn conflict_button_takes_the_incoming_tile_without_reaching_the_place_tool() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut session = Session::new();
        session
            .load_environment(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env/test.dme"))
            .unwrap();
        let mut map = dmm::Map::new(Size { x: 20, y: 20, z: 1 });
        let floor = Prefab::new(TreePath::parse("/turf/open/floor"));
        let key = map.intern_tile(vec![floor.clone()]);
        for row in &mut map.grid[0] {
            row.fill(key);
        }
        let coord = Coord::new(5, 8, 1);
        let incoming = Prefab::new(TreePath::parse("/turf/closed/wall"));
        session.apply_map(crate::loader::LoadedMap {
            path: PathBuf::from("conflict-ui-test.dmm"),
            map,
            z: 1,
            errors: vec![],
            repo: Some(editor::git::RepoPath {
                root: PathBuf::from("."),
                git_dir: PathBuf::from(".git"),
                rel: String::from("conflict-ui-test.dmm"),
            }),
            conflict: Some(editor::conflict::ConflictData {
                operation: None,
                conflicts: vec![dmm::merge::TileConflict {
                    coord,
                    base: Some(vec![floor.clone()]),
                    ours: Some(vec![floor]),
                    theirs: Some(vec![incoming.clone()]),
                }],
            }),
        });
        session.set_tool(Tool::Place);
        session
            .state
            .choose_prefab(Prefab::new(TreePath::parse("/turf/closed/wall")));
        let mut app = RectangleUiHarness::with_session(session);
        let button = app.conflict_button.expect("incoming control is on screen");
        app.click(button);
        assert_eq!(
            app.session.state.active_document().unwrap().map.tile_at(coord).unwrap()[0].path,
            incoming.path
        );
        assert_eq!(
            app.session
                .git_state(app.id)
                .unwrap()
                .conflicts
                .as_ref()
                .unwrap()
                .resolution(coord, &app.session.state.active_document().unwrap().history),
            Some(Side::Theirs)
        );
        assert_eq!(app.session.undo_label(), Some("Take theirs"));
    }

    #[test]
    fn right_click_captures_the_clicked_tile_in_every_tool() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        for tool in [
            Tool::Select,
            Tool::Place,
            Tool::Node,
            Tool::BlockSelect,
            Tool::Delete,
            Tool::Fill,
        ] {
            let mut app = RectangleUiHarness::new();
            app.session.set_tool(tool);
            let point = app.tile(5, 8);
            app.context.io_mut().add_mouse_pos_event(point);
            app.context.io_mut().add_mouse_button_event(MouseButton::Right, false);
            app.step();
            app.context.io_mut().add_mouse_button_event(MouseButton::Right, true);
            app.step();
            let target = app.view.context.as_ref().expect("right click opens the map menu");
            assert_eq!(target.coord, Coord::new(5, 8, 1), "{tool:?}");
            assert_eq!(target.document, app.id);
            app.context.io_mut().add_mouse_button_event(MouseButton::Right, false);
            let next_tile = app.tile(6, 8);
            app.context.io_mut().add_mouse_pos_event(next_tile);
            app.step();
            assert_eq!(app.view.context.as_ref().unwrap().coord, Coord::new(5, 8, 1));
        }
    }

    #[test]
    fn middle_click_outside_the_map_menu_closes_it_and_pans() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut app = RectangleUiHarness::new();
        let point = app.tile(5, 8);
        app.context.io_mut().add_mouse_pos_event(point);
        app.context.io_mut().add_mouse_button_event(MouseButton::Right, true);
        app.step();
        app.context.io_mut().add_mouse_button_event(MouseButton::Right, false);
        app.step();
        assert!(app.view.context.is_some(), "right click opens the map menu");

        // the menu opens to the right of the cursor, so a tile to the left is outside it
        let outside = app.tile(2, 8);
        app.context.io_mut().add_mouse_pos_event(outside);
        app.context.io_mut().add_mouse_button_event(MouseButton::Middle, true);
        app.step();
        let before = app.tile(5, 8);
        app.context
            .io_mut()
            .add_mouse_pos_event([outside[0] + 40.0, outside[1] + 20.0]);
        app.step();

        assert_ne!(app.tile(5, 8), before, "the map pans once the menu is gone");
    }

    #[test]
    fn node_right_click_deletes_on_the_first_unfocused_click_and_shift_opens_the_menu() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let start = Coord::new(5, 8, 1);
        let middle = Coord::new(6, 8, 1);
        let end = Coord::new(7, 8, 1);
        let map = node_map(
            20,
            20,
            &[
                (start, vec!["/obj/cable"]),
                (middle, vec!["/obj/cable"]),
                (end, vec!["/obj/cable"]),
            ],
        );
        let (mut session, seed) = node_session(map, start);
        assert!(session.begin_node_edit(seed));
        let mut app = RectangleUiHarness::with_session(session);
        app.settings.focus_windows_on_hover = false;
        app.focus_other_window = true;
        app.view.focus = false;
        let point = app.tile(6, 8);
        app.context.io_mut().add_mouse_pos_event(point);
        app.context.io_mut().add_mouse_button_event(MouseButton::Right, false);
        app.step();

        app.context.io_mut().add_mouse_button_event(MouseButton::Right, true);
        app.step();
        assert!(!node_tile_has_group(&app.session, middle));
        assert!(app.view.context.is_none());
        assert_eq!(app.session.undo_label(), Some("delete node connection"));

        app.context.io_mut().add_mouse_button_event(MouseButton::Right, false);
        app.step();
        assert!(app.session.undo());
        assert!(app.session.begin_node_edit(seed));
        app.key(Key::ModShift, true);
        app.context.io_mut().add_mouse_button_event(MouseButton::Right, true);
        app.step();
        assert!(node_tile_has_group(&app.session, middle));
        assert!(matches!(
            app.view.context.as_ref().and_then(|target| target.node.as_ref()),
            Some(NodeContext::Connection(connection)) if connection.contains(&middle)
        ));
    }

    #[test]
    fn rectangle_ui_supports_drawing_resizing_filling_and_switching_to_the_bucket() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut app = RectangleUiHarness::new();
        app.pointer(app.tile(5, 8), false);
        app.key(Key::ModShift, true);
        app.key(Key::S, true);
        assert_eq!(app.session.tool(), Tool::BlockSelect);
        assert!(app.session.selection().is_none());
        app.key(Key::S, false);

        // Shift is still held from the shortcut, so the drag draws a border.
        app.pointer(app.tile(5, 8), true);
        app.pointer(app.tile(7, 10), true);
        app.pointer(app.tile(7, 10), false);
        app.key(Key::ModShift, false);
        let border = SelectionMask {
            bounds: Selection::from_drag(Coord::new(5, 8, 1), Coord::new(7, 10, 1)),
            mode: BlockSelectionMode::Hollow { line_width: 1 },
        };
        assert_eq!(app.session.selection_mask(), Some(border));
        assert_eq!(app.session.undo_label(), None);

        // Explicit mode controls change the current selection without painting.
        app.click(app.mode_buttons.unwrap().1);
        assert_eq!(app.session.selection_mode(), BlockSelectionMode::Full);
        app.click(app.mode_buttons.unwrap().0);
        assert_eq!(app.session.selection_mask(), Some(border));

        // A Shift-gizmo drag resizes bounds; releasing Shift mid-drag keeps the border.
        app.pointer(app.tile(6, 9), false);
        app.key(Key::ModShift, true);
        app.pointer(app.tile(6, 9), true);
        app.pointer(app.tile(8, 11), true);
        app.key(Key::ModShift, false);
        app.pointer(app.tile(8, 11), false);
        let resized = SelectionMask {
            bounds: Selection::from_drag(border.bounds.min, Coord::new(9, 12, 1)),
            ..border
        };
        assert_eq!(app.session.selection_mask(), Some(resized));
        assert!(app.view.block_placement.is_none());
        assert_eq!(app.session.undo_label(), None);

        let mut red = Prefab::new(TreePath::parse("/turf/open/floor"));
        red.set_var("color".into(), core::types::Value::Text("#ff0000".into()));
        app.session.state.choose_prefab(red.clone());
        app.step();
        app.click(app.fill_button.unwrap());
        assert!(
            app.session
                .map()
                .unwrap()
                .tile_at(resized.bounds.min)
                .unwrap()
                .contains(&red)
        );
        assert!(
            !app.session
                .map()
                .unwrap()
                .tile_at(Coord::new(7, 10, 1))
                .unwrap()
                .contains(&red)
        );

        app.pointer(app.tile(5, 8), false);
        app.key(Key::Q, true);
        app.key(Key::Q, false);
        assert_eq!(app.session.tool(), Tool::Fill);
        assert_eq!(app.session.selection_mask(), Some(resized));
        let mut blue = red.clone();
        blue.set_var("color".into(), core::types::Value::Text("#0000ff".into()));
        app.session.state.choose_prefab(blue.clone());
        app.click(app.tile(7, 10));
        assert!(
            !app.session
                .map()
                .unwrap()
                .tile_at(resized.bounds.min)
                .unwrap()
                .contains(&blue)
        );
        app.click(app.tile(5, 8));
        assert!(
            app.session
                .map()
                .unwrap()
                .tile_at(resized.bounds.min)
                .unwrap()
                .contains(&blue)
        );
        assert!(
            !app.session
                .map()
                .unwrap()
                .tile_at(Coord::new(4, 8, 1))
                .unwrap()
                .contains(&blue)
        );
        app.key(Key::Escape, true);
        app.key(Key::Escape, false);
        assert_eq!(app.session.selection_mask(), None);
        app.key(Key::ModCtrl, true);
        app.key(Key::Z, true);
        app.key(Key::Z, false);
        app.key(Key::ModCtrl, false);
        assert!(
            app.session
                .map()
                .unwrap()
                .tile_at(resized.bounds.min)
                .unwrap()
                .contains(&red)
        );
    }

    #[test]
    fn grid_overlay_shortcuts_toggle_settings_while_hovered() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut app = RectangleUiHarness::new();
        app.pointer(app.tile(5, 8), false);

        assert!(app.settings.show_tile_grid);
        assert!(app.settings.show_selected_pixel_grid);

        app.key(Key::G, true);
        assert!(!app.settings.show_tile_grid);
        assert!(
            app.settings.show_selected_pixel_grid,
            "plain G should not affect the pixel grid"
        );
        app.key(Key::G, false);

        app.key(Key::ModShift, true);
        app.key(Key::G, true);
        assert!(!app.settings.show_selected_pixel_grid);
        assert!(
            !app.settings.show_tile_grid,
            "shift+g should not toggle the tile grid again"
        );
        app.key(Key::G, false);
        app.key(Key::ModShift, false);
    }

    #[test]
    fn rectangle_ui_keeps_shift_draw_mode_and_restores_cancelled_gestures() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut app = RectangleUiHarness::new();
        app.session.set_tool(Tool::BlockSelect);
        app.pointer(app.tile(5, 8), true);
        app.pointer(app.tile(7, 10), true);
        assert_eq!(app.session.selection_mode(), BlockSelectionMode::Full);
        app.key(Key::ModShift, true);
        assert_eq!(
            app.session.selection_mode(),
            BlockSelectionMode::Hollow { line_width: 1 }
        );
        app.context.io_mut().add_key_event(Key::ModShift, false);
        app.pointer(app.tile(7, 10), false);
        let original = app.session.selection_mask().unwrap();
        assert_eq!(original.mode, BlockSelectionMode::Hollow { line_width: 1 });
        assert_eq!(app.state.block_selection_options.mode(), BlockSelectionMode::Full);

        app.pointer(app.tile(12, 12), true);
        app.pointer(app.tile(14, 14), true);
        assert_eq!(app.session.selection_mode(), BlockSelectionMode::Full);
        app.key(Key::Escape, true);
        assert_eq!(app.session.selection_mask(), Some(original));
        app.key(Key::Escape, false);
        app.pointer(app.tile(14, 14), false);
        assert_eq!(app.session.selection_mask(), Some(original));

        // A corner handle resizes without Shift, and tool changes cancel the active resize.
        let corner = app.tile(7, 10);
        let corner = [corner[0] + 23.0, corner[1] - 23.0];
        app.pointer(corner, false);
        app.pointer(corner, true);
        app.pointer([corner[0] + 32.0, corner[1] - 32.0], true);
        assert_eq!(app.session.selection().unwrap().max, Coord::new(8, 11, 1));
        assert!(app.view.block_placement.is_none());
        app.key(Key::Q, true);
        app.key(Key::Q, false);
        assert_eq!(app.session.tool(), Tool::Fill);
        assert_eq!(app.session.selection_mask(), Some(original));
        app.pointer(corner, false);
        assert_eq!(app.session.undo_label(), None);
        assert!(app.view.rectangle_gesture.is_none());
        app.click(app.mode_buttons.unwrap().0);
        assert_eq!(
            app.state.block_selection_options.mode(),
            BlockSelectionMode::Hollow { line_width: 1 }
        );
    }

    #[test]
    fn rectangle_ui_copies_a_border_to_another_map_and_keeps_it_hollow_after_paste() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut app = RectangleUiHarness::new();
        let source_id = app.id;
        app.session.set_tool(Tool::BlockSelect);
        app.key(Key::ModShift, true);
        app.pointer(app.tile(5, 8), true);
        app.pointer(app.tile(9, 14), true);
        app.pointer(app.tile(9, 14), false);
        app.key(Key::ModShift, false);
        let source = app.session.selection_mask().unwrap();
        assert_eq!(source.mode, BlockSelectionMode::Hollow { line_width: 1 });
        let mut red = Prefab::new(TreePath::parse("/turf/open/floor"));
        red.set_var("color".into(), core::types::Value::Text("#ff0000".into()));
        app.session.state.choose_prefab(red.clone());
        app.step();
        app.click(app.fill_button.unwrap());
        app.pointer(app.tile(5, 8), false);
        app.key(Key::ModCtrl, true);
        app.key(Key::C, true);
        app.key(Key::C, false);
        app.key(Key::ModCtrl, false);
        let clipboard = app.session.clipboard().unwrap().clone();
        assert_eq!(clipboard.filled().count(), source.mode.tiles(source.bounds).count());
        let source_grid = app.session.map().unwrap().grid.clone();

        let mut destination = dmm::Map::new(Size { x: 20, y: 20, z: 1 });
        let blue = Prefab::new(TreePath::parse("/turf/open/floor"));
        let key = destination.intern_tile(vec![blue.clone(), Prefab::new(TreePath::parse("/area/station"))]);
        for row in &mut destination.grid[0] {
            row.fill(key);
        }
        let destination_grid = destination.grid.clone();
        app.session.apply_map(crate::loader::LoadedMap {
            path: PathBuf::from("rectangle-ui-destination.dmm"),
            map: destination,
            z: 1,
            errors: vec![],
            repo: None,
            conflict: None,
        });
        app.id = app.session.state.active().unwrap();
        let mut destination_view = MapViewState::new(app.id).unwrap();
        destination_view.refit = false;
        destination_view.focus = true;
        destination_view.camera.camera.x = 320.0;
        destination_view.camera.camera.y = 320.0;
        let source_view = std::mem::replace(&mut app.view, destination_view);
        app.state.map_views.insert(source_id, source_view);
        // Different destination settings and palette must not change what was copied.
        app.state.block_selection_options.full_rectangle = false;
        app.state.block_selection_options.line_width = 3;
        app.session.state.choose_prefab(blue.clone());
        for _ in 0..3 {
            app.step();
        }
        app.pointer(app.tile(12, 10), false);
        app.key(Key::ModCtrl, true);
        app.key(Key::V, true);
        app.key(Key::V, false);
        app.key(Key::ModCtrl, false);
        let pending = app.view.paste.expect("Ctrl+V starts a preview in the destination");
        let target = app
            .session
            .clipboard_selection_mask(pending.min, pending.rotation)
            .unwrap();
        assert_eq!(target.mode, source.mode);
        assert_eq!(
            app.session
                .clipboard_preview_sprites(target.bounds, pending.rotation)
                .len(),
            clipboard.filled().count()
        );
        assert_eq!(
            app.session.map().unwrap().grid,
            destination_grid,
            "preview does not paint"
        );
        app.key(Key::Enter, true);
        app.key(Key::Enter, false);
        assert!(app.view.paste.is_none());
        assert_eq!(app.session.selection_mask(), Some(target));
        for coord in target.bounds.iter() {
            let tile = app.session.map().unwrap().tile_at(coord).unwrap();
            assert_eq!(tile.contains(&red), target.includes(coord));
            assert_eq!(tile.contains(&blue), !target.includes(coord));
        }
        assert_eq!(app.session.state.document(source_id).unwrap().map.grid, source_grid);
        app.key(Key::ModCtrl, true);
        app.key(Key::Z, true);
        app.key(Key::Z, false);
        app.key(Key::ModCtrl, false);
        assert_eq!(app.session.map().unwrap().grid, destination_grid);
        assert_eq!(app.session.state.document(source_id).unwrap().map.grid, source_grid);
        app.key(Key::ModCtrl, true);
        app.key(Key::Y, true);
        app.key(Key::Y, false);
        app.key(Key::ModCtrl, false);
        assert_eq!(app.session.selection_mask(), Some(target));

        // The next Fill selection must still act on the copied border only.
        let mut green = blue.clone();
        green.set_var("color".into(), core::types::Value::Text("#00ff00".into()));
        app.session.state.choose_prefab(green.clone());
        app.step();
        app.click(app.fill_button.unwrap());
        for coord in target.bounds.iter() {
            assert_eq!(
                app.session.map().unwrap().tile_at(coord).unwrap().contains(&green),
                target.includes(coord)
            );
        }
    }

    fn gizmo_frame(
        context: &mut dear_imgui_rs::Context, gizmo: &mut GizmoState, camera: &Controller, settings: &Settings,
        target: BlockGizmoTarget, hovered: bool,
    ) -> crate::gizmo::BlockGizmoResponse {
        let ui = context.frame();
        let view = GizmoMapView {
            min: [0.0; 2],
            max: [800.0, 600.0],
            hovered,
        };
        let response = ui
            .window("rectangle-test")
            .position([0.0; 2], Condition::Always)
            .size([800.0, 600.0], Condition::Always)
            .build(|| {
                let response = gizmo.draw_block(ui, settings, camera, target, view);
                gizmo.draw_block_overlay(
                    ui,
                    camera,
                    BlockGizmoTarget {
                        selection: response.selection,
                        ..target
                    },
                    view,
                );
                response
            })
            .unwrap();
        assert!(context.render_legacy().valid());
        response
    }

    fn object_gizmo_frame(
        context: &mut dear_imgui_rs::Context, gizmo: &mut GizmoState, session: &mut Session, camera: &Controller,
        settings: &Settings,
    ) -> (crate::gizmo::GizmoResponse, usize) {
        let ui = context.frame();
        let view = GizmoMapView {
            min: [0.0; 2],
            max: [800.0, 600.0],
            hovered: true,
        };
        let response = ui
            .window("object-gizmo-z-level-test")
            .position([0.0; 2], Condition::Always)
            .size([800.0, 600.0], Condition::Always)
            .build(|| gizmo.draw(ui, session, settings, camera, TransformMode::Pixel, view))
            .unwrap();
        let draw_data = context.render_legacy();
        assert!(draw_data.valid());

        (response, draw_data.total_vtx_count())
    }

    #[test]
    fn object_gizmo_only_appears_on_the_selected_instances_z_level() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut context = rectangle_context();
        let mut session = Session::new();
        let examples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env");
        session.load_environment(&examples.join("test.dme")).unwrap();
        let coord = Coord::new(6, 3, 1);
        let mut map = dmm::Map::new(Size { x: 8, y: 6, z: 2 });
        let base = map.intern_tile(vec![
            Prefab::new(TreePath::parse("/turf/open/floor")),
            Prefab::new(TreePath::parse("/area/station")),
        ]);
        let selected_tile = map.intern_tile(vec![
            Prefab::new(TreePath::parse("/obj/structure/table")),
            Prefab::new(TreePath::parse("/turf/open/floor")),
            Prefab::new(TreePath::parse("/area/station")),
        ]);
        for level in &mut map.grid {
            for row in level {
                row.fill(base);
            }
        }
        map.grid[0][2][5] = selected_tile;
        session.apply_map(crate::loader::LoadedMap {
            path: PathBuf::from("object-gizmo-z-level-test.dmm"),
            map,
            z: 1,
            errors: vec![],
            repo: None,
            conflict: None,
        });
        let selected = session.state.active_document().unwrap().instance_ids_at(coord)[0];
        session.select_instance(Some(selected));

        let mut camera = Controller::new();
        camera.resize(800, 600);
        camera.center_on_tile(coord, session.options.tile_size);
        let sprite = session.selected_transform().unwrap().sprite;
        let origin = camera.map_to_screen([sprite.x + sprite.width * 0.5, sprite.y + sprite.height * 0.5]);
        let mut gizmo = GizmoState::default();
        let settings = Settings::default();
        context.io_mut().add_mouse_pos_event(origin);
        context.io_mut().add_mouse_button_event(MouseButton::Left, true);

        let (visible, visible_vertices) =
            object_gizmo_frame(&mut context, &mut gizmo, &mut session, &camera, &settings);
        assert!(visible.captures_mouse);
        assert!(gizmo.is_interacting());

        assert_eq!(session.change_level(1), LevelChange::Changed);
        assert_eq!(session.selected_instance(), Some(selected));
        let (hidden, hidden_vertices) = object_gizmo_frame(&mut context, &mut gizmo, &mut session, &camera, &settings);
        assert!(!hidden.captures_mouse);
        assert!(!gizmo.is_interacting());
        assert!(visible_vertices > hidden_vertices);

        assert_eq!(session.change_level(-1), LevelChange::Changed);
        let (visible_again, restored_vertices) =
            object_gizmo_frame(&mut context, &mut gizmo, &mut session, &camera, &settings);
        assert!(visible_again.captures_mouse);
        assert!(restored_vertices > hidden_vertices);
    }

    #[test]
    fn resize_and_move_gestures_keep_their_mouse_down_action_when_shift_changes() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        for resize in [true, false] {
            let mut context = rectangle_context();
            let mut camera = Controller::new();
            camera.resize(800, 600);
            camera.center_on_tile(Coord::new(5, 5, 1), 32);
            let selection = Selection::from_drag(Coord::new(3, 3, 1), Coord::new(5, 5, 1));
            let bounds = block_selection_bounds(&camera, [0.0; 2], selection, 32);
            let origin = [
                (bounds.min[0] + bounds.max[0]) * 0.5,
                (bounds.min[1] + bounds.max[1]) * 0.5,
            ];
            let mut target = BlockGizmoTarget {
                selection,
                rotation: SelectionRotation::Original,
                map_size: Size { x: 10, y: 10, z: 1 },
                tile_size: 32,
                kind: BlockGizmoKind::Selection,
            };
            let mut gizmo = GizmoState::default();
            let settings = Settings::default();
            context.io_mut().add_mouse_pos_event(origin);
            context.io_mut().add_key_event(Key::ModShift, resize);
            context.io_mut().add_mouse_button_event(MouseButton::Left, true);
            let start = gizmo_frame(&mut context, &mut gizmo, &camera, &settings, target, true);
            assert_eq!(start.resizing, resize);
            context.io_mut().add_key_event(Key::ModShift, !resize);
            context
                .io_mut()
                .add_mouse_pos_event([origin[0] + 32.0, origin[1] - 32.0]);
            let dragged = gizmo_frame(&mut context, &mut gizmo, &camera, &settings, target, true);
            assert_eq!(dragged.resizing, resize);
            assert_eq!(dragged.selection.width(), if resize { 4 } else { 3 });
            assert_eq!(
                dragged.selection.min,
                if resize { selection.min } else { Coord::new(4, 4, 1) }
            );
            target.selection = dragged.selection;
            context.io_mut().add_mouse_button_event(MouseButton::Left, false);
            context.io_mut().add_mouse_pos_event([2000.0, -2000.0]);
            let released = gizmo_frame(&mut context, &mut gizmo, &camera, &settings, target, false);
            assert_eq!(released.resizing, resize);
            assert!(released.captures_mouse);
            assert!(!gizmo.is_interacting());
            assert_eq!(released.selection.max, Coord::new(10, 10, 1));
        }
    }

    #[test]
    fn rotation_shortcuts_work_with_both_presets_and_a_custom_modifier_binding() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        for (preset, modifier, key, custom) in [
            (KeybindPreset::Default, None, Key::R, false),
            (KeybindPreset::StrongDmm, Some(Key::ModShift), Key::R, false),
            (KeybindPreset::Default, Some(Key::ModCtrl), Key::T, true),
        ] {
            let mut context = rectangle_context();
            let mut settings = Settings {
                keybindings: preset.bindings(),
                ..Settings::default()
            };
            if custom {
                settings
                    .keybindings
                    .rebind(KeybindAction::Rotate, KeyBinding::with_ctrl(Key::T));
            }
            let mut camera = Controller::new();
            camera.resize(800, 600);
            let mut gizmo = GizmoState::default();
            let target = BlockGizmoTarget {
                selection: Selection::from_drag(Coord::new(2, 2, 1), Coord::new(4, 5, 1)),
                rotation: SelectionRotation::Original,
                map_size: Size { x: 10, y: 10, z: 1 },
                tile_size: 32,
                kind: BlockGizmoKind::Selection,
            };
            context.io_mut().add_mouse_pos_event([400.0, 300.0]);
            if let Some(modifier) = modifier {
                context.io_mut().add_key_event(modifier, true);
            }
            context.io_mut().add_key_event(key, true);
            let pressed = gizmo_frame(&mut context, &mut gizmo, &camera, &settings, target, true);
            assert!(!pressed.resizing);
            context.io_mut().add_key_event(key, false);
            let released = gizmo_frame(&mut context, &mut gizmo, &camera, &settings, target, true);
            assert_eq!(released.rotation, SelectionRotation::Clockwise);
            assert_eq!(released.selection, target.selection);
        }
    }

    #[test]
    fn cancelling_rectangle_gestures_restores_only_the_originating_document_and_level() {
        let mut session = Session::new();
        let id = session
            .state
            .open_document(MapDocument::new(dmm::Map::new(Size { x: 10, y: 10, z: 2 }), 1));
        let start = SelectionMask {
            bounds: Selection::from_drag(Coord::new(2, 2, 1), Coord::new(4, 4, 1)),
            mode: BlockSelectionMode::Hollow { line_width: 1 },
        };
        session.try_select_block(start);
        let mut gesture = Some(RectangleGesture {
            start: Some(start),
            z: 1,
        });
        session.try_select_block(SelectionMask {
            bounds: Selection::from_drag(start.bounds.min, Coord::new(8, 8, 1)),
            mode: BlockSelectionMode::Full,
        });
        let other = session
            .state
            .open_document(MapDocument::new(dmm::Map::new(Size { x: 3, y: 3, z: 1 }), 1));
        restore_rectangle_gesture(&mut session, id, &mut gesture);
        assert_eq!(session.state.document(id).unwrap().selection_mask(), Some(start));
        assert_eq!(session.state.document(other).unwrap().selection_mask(), None);
        assert!(gesture.is_none());
        session.state.set_active(id);
        gesture = Some(RectangleGesture {
            start: Some(start),
            z: 1,
        });
        session.set_level(2);
        restore_rectangle_gesture(&mut session, id, &mut gesture);
        assert_eq!(session.selection_mask(), None);
    }

    #[test]
    fn a_map_view_rect_is_measured_in_framebuffer_pixels() {
        // ImGui lays out in logical units; the render target is physical pixels.
        let rect = framebuffer_rect([100.0, 50.0], [400.0, 300.0], [1.5, 1.5], [1000.0, 800.0]);

        assert_eq!(rect.x, 150);
        assert_eq!(rect.y, 75);
        assert_eq!(rect.width, 600);
        assert_eq!(rect.height, 450);
    }

    #[test]
    fn a_map_view_rect_is_the_same_at_unit_scale() {
        let rect = framebuffer_rect([0.0, 22.0], [640.0, 480.0], [1.0, 1.0], [1280.0, 720.0]);

        assert_eq!(
            rect,
            MapViewRect {
                x: 0,
                y: 22,
                width: 640,
                height: 480,
            }
        );
    }

    #[test]
    fn a_map_view_rect_is_clamped_to_the_target() {
        // Position stays inside the main framebuffer, while the independent map
        // attachment keeps its full size when the ImGui window is clipped.
        let rect = framebuffer_rect([-40.0, -10.0], [200.0, 100.0], [1.0, 1.0], [120.0, 60.0]);

        assert_eq!(rect.x, 0);
        assert_eq!(rect.y, 0);
        assert_eq!(rect.width, 200);
        assert_eq!(rect.height, 100);

        let collapsed = framebuffer_rect([10.0, 10.0], [0.0, 0.0], [1.0, 1.0], [100.0, 100.0]);
        assert!(collapsed.is_empty(), "a zero-size split contributes no draw");
    }

    #[test]
    fn a_cursor_is_local_to_its_map_view() {
        // Picking chooses a visibility attachment by hovered ImGui window, so
        // its cursor stays local regardless of where that window is placed.
        assert_eq!(cursor_in_map_view([10.0, 20.0], [1.0, 1.0]), [10, 20]);
    }

    #[test]
    fn a_cursor_scales_with_the_display() {
        assert_eq!(cursor_in_map_view([30.0, 40.0], [2.0, 2.0]), [60, 80]);
    }

    #[test]
    fn every_document_gets_its_own_map_view_window_key() {
        let first = DocumentId::new();
        let second = DocumentId::new();
        let first = MapViewState::new(first).expect("valid window key");
        let second = MapViewState::new(second).expect("valid window key");

        // Two windows sharing a key would be one window, and the dock layout
        // compiler rejects the duplicate outright.
        assert_ne!(first.window.stable_id(), second.window.stable_id());
        assert!(DockLayout::tabs([&first.window, &second.window]).validate().is_ok());
    }

    #[test]
    fn a_new_map_view_starts_out_wanting_to_frame_its_map() {
        let view = MapViewState::new(DocumentId::new()).expect("valid window key");

        assert!(view.refit);
        assert!(!view.focus);
        assert!(view.block_selection_anchor.is_none());
        assert!(view.block_placement.is_none());
    }

    #[test]
    fn panel_extent_rejects_non_finite_or_empty_sizes() {
        assert_eq!(panel_extent([0.0, f32::NAN]), ([1.0, 1.0], (1, 1)));
        assert_eq!(panel_extent([320.75, 200.25]), ([320.75, 200.25], (320, 200)));
    }

    #[test]
    fn placement_flash_fades_once_over_a_quarter_second() {
        let owner = PrefabInstanceId::from_raw(7).unwrap();
        let flash = ActivePlacementFlash {
            owner,
            coord: Coord::new(2, 3, 1),
            started_at: 10.0,
        };

        assert_eq!(flash.sample(10.0), Some(PlacementFlash { owner, strength: 1.0 }));
        assert_eq!(flash.sample(10.125), Some(PlacementFlash { owner, strength: 0.5 }));
        assert_eq!(flash.sample(10.25), None);
        assert_eq!(flash.sample(11.0), None);
    }

    #[test]
    fn disabling_placement_flash_clears_an_active_effect() {
        let owner = PrefabInstanceId::from_raw(7).unwrap();
        let mut active = Some(ActivePlacementFlash {
            owner,
            coord: Coord::new(2, 3, 1),
            started_at: 10.0,
        });

        assert_eq!(active_placement_flash(&mut active, 10.0, false), None);
        assert_eq!(active, None);
    }

    #[test]
    fn placement_strokes_process_each_tile_once_until_a_new_stroke_begins() {
        let prefab = Prefab::new(TreePath::parse("/obj/table"));
        let first = Coord::new(2, 3, 1);
        let second = Coord::new(3, 3, 1);
        let mut stroke = PlacementStroke::new(prefab.clone(), 1);

        let group = stroke.visit(first).expect("first tile in stroke");
        assert_eq!(stroke.visit(first), None);
        assert_eq!(stroke.visit(second), Some(group));
        assert_eq!(stroke.visit(first), None);
        assert_eq!(stroke.visit(Coord::new(2, 3, 2)), None);

        let mut next_stroke = PlacementStroke::new(prefab, 1);
        assert!(next_stroke.visit(first).is_some());
    }

    #[test]
    fn placement_strokes_only_match_their_starting_context() {
        let prefab = Prefab::new(TreePath::parse("/obj/table"));
        let other = Prefab::new(TreePath::parse("/obj/chair"));
        let stroke = PlacementStroke::new(prefab.clone(), 2);

        assert!(stroke.matches_context(Tool::Place, Some(&prefab), 2));
        assert!(!stroke.matches_context(Tool::Select, Some(&prefab), 2));
        assert!(!stroke.matches_context(Tool::Place, Some(&other), 2));
        assert!(!stroke.matches_context(Tool::Place, Some(&prefab), 1));
        assert!(!stroke.matches_context(Tool::Place, None, 2));
    }

    #[test]
    fn tool_interaction_configures_highlights_and_pick_requests() {
        let owner = PrefabInstanceId::from_raw(7).unwrap();
        let interaction = MapViewInteraction {
            cursor: Some([10, 20]),
            hovered_area: Some(owner),
            selected: Some(owner),
            placement_flash: Some(PlacementFlash { owner, strength: 0.5 }),
            highlight: HighlightStyle::Tint,
            mode: InteractionMode::Select {
                pick: Some(PickRequest::Select),
            },
        };
        let mut selected = interaction;

        configure_tool_interaction(Tool::Select, &mut selected);
        assert_eq!(selected, interaction);

        let mut delete = interaction;
        configure_tool_interaction(Tool::Delete, &mut delete);
        assert_eq!(
            delete,
            MapViewInteraction {
                selected: None,
                mode: InteractionMode::Delete { pick: None },
                ..interaction
            }
        );

        configure_tool_interaction(Tool::Place, &mut selected);
        assert_eq!(
            selected,
            MapViewInteraction {
                placement_flash: interaction.placement_flash,
                highlight: interaction.highlight,
                ..Default::default()
            }
        );

        let mut node = interaction;
        configure_tool_interaction(Tool::Node, &mut node);
        assert_eq!(
            node,
            MapViewInteraction {
                placement_flash: interaction.placement_flash,
                highlight: interaction.highlight,
                mode: InteractionMode::Select { pick: None },
                ..Default::default()
            }
        );
        let coord = Coord::new(3, 4, 1);
        request_pick(&mut node, PickRequest::NodeSeed(coord));
        assert_eq!(
            node.mode,
            InteractionMode::Select {
                pick: Some(PickRequest::NodeSeed(coord)),
            }
        );
        request_pick(&mut node, PickRequest::NodeDelete(coord));
        assert_eq!(
            node.mode,
            InteractionMode::Select {
                pick: Some(PickRequest::NodeDelete(coord)),
            }
        );

        let mut fill = interaction;
        configure_tool_interaction(Tool::Fill, &mut fill);
        assert_eq!(
            fill,
            MapViewInteraction {
                placement_flash: interaction.placement_flash,
                highlight: interaction.highlight,
                ..Default::default()
            }
        );

        let mut block = interaction;
        configure_tool_interaction(Tool::BlockSelect, &mut block);
        assert_eq!(
            block,
            MapViewInteraction {
                placement_flash: interaction.placement_flash,
                highlight: interaction.highlight,
                ..Default::default()
            }
        );
    }

    #[test]
    fn deletion_strokes_only_continue_when_the_cursor_moves() {
        let mut stroke = DeletionStroke::new([10, 20]);

        assert!(!stroke.move_to([10, 20]));
        assert!(stroke.move_to([11, 20]));
        assert!(!stroke.move_to([11, 20]));
        assert!(stroke.move_to([10, 20]));
    }
}
