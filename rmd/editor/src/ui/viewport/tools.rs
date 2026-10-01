use dear_imgui_rs::{MouseButton, Ui};
use dmm::Coord;
use editor::{document::Selection, tool::Tool};
use render::{InteractionMode, MapViewInteraction, PickRequest};

use super::{
    ViewFrame,
    cursor_in_map_view,
    stroke::{ActivePlacementFlash, PickStroke, PlacementStroke},
};
use crate::{
    gizmo::GizmoMapView,
    session::{FillOutcome, Session},
    settings::{KeybindAction, Settings},
    ui::{
        FILL_LIMIT_WARNING_POPUP,
        FillWarningContext,
        NodeOverlayView,
        NodeRightClick,
        PendingFillWarning,
        UiState,
        context_menu::{NodeContext, POPUP as MAP_MENU_POPUP, Target as MenuTarget},
        coop::{COMMENT_POPUP, CommentDraft},
        draw_node_overlay,
        node::NodeOverlayHit,
        node_right_click,
    },
};

#[derive(Debug, Clone, Copy)]
pub(super) struct Clicks {
    pub(super) is_left_clicked: bool,
    pub(super) is_left_down: bool,
    pub(super) is_left_double_clicked: bool,
    pub(super) is_right_clicked: bool,
}

impl Clicks {
    pub(super) fn read(ui: &Ui) -> Self {
        Self {
            is_left_clicked: ui.is_mouse_clicked(MouseButton::Left),
            is_left_down: ui.is_mouse_down(MouseButton::Left),
            is_left_double_clicked: ui.is_mouse_double_clicked(MouseButton::Left),
            is_right_clicked: ui.is_mouse_clicked(MouseButton::Right),
        }
    }
}

pub(super) fn configure_tool_interaction(tool: Tool, interaction: &mut MapViewInteraction) {
    interaction.mode = match (tool, interaction.mode) {
        (Tool::Place | Tool::BlockSelect | Tool::Fill, _) => InteractionMode::Place,
        (Tool::Select | Tool::Replace, InteractionMode::Select { pick }) => InteractionMode::Select { pick },
        (Tool::Delete, InteractionMode::Delete { pick }) => InteractionMode::Delete { pick },
        (Tool::Select | Tool::Replace | Tool::Node | Tool::Comment, _) => InteractionMode::Select { pick: None },
        (Tool::Delete, _) => InteractionMode::Delete { pick: None },
    };
    match tool {
        Tool::Place | Tool::Node | Tool::BlockSelect | Tool::Fill | Tool::Comment => {
            interaction.cursor = None;
            interaction.selected = None;
        },
        Tool::Delete | Tool::Replace => {
            interaction.selected = None;
        },
        Tool::Select => {},
    }
}

pub(super) fn interaction_mode(tool: Tool) -> InteractionMode {
    match tool {
        Tool::Place | Tool::BlockSelect | Tool::Fill => InteractionMode::Place,
        Tool::Select | Tool::Replace | Tool::Node | Tool::Comment => InteractionMode::Select { pick: None },
        Tool::Delete => InteractionMode::Delete { pick: None },
    }
}

pub(super) fn request_pick(interaction: &mut MapViewInteraction, request: PickRequest) {
    match &mut interaction.mode {
        InteractionMode::Place => {},
        InteractionMode::Select { pick } | InteractionMode::Delete { pick } => *pick = Some(request),
    }
}

pub(super) fn draw_node_hit(ui: &Ui, session: &Session, frame: &ViewFrame<'_>) -> NodeOverlayHit {
    let is_interactive = frame.is_hovered && !session.node_dragging();

    (frame.is_active && session.tool() == Tool::Node)
        .then(|| session.node_overlay())
        .flatten()
        .map(|overlay| {
            draw_node_overlay(
                ui,
                &overlay,
                NodeOverlayView {
                    camera: frame.camera,
                    viewport_min: frame.layout.viewport.min,
                    viewport_max: frame.layout.viewport.max,
                    tile_size: session.options.tile_size,
                    hovered_tile: frame.coord,
                    interactive: is_interactive,
                },
            )
        })
        .unwrap_or_default()
}

pub(super) fn apply_right_click(
    ui: &Ui, session: &mut Session, frame: &mut ViewFrame<'_>, node_hit: &NodeOverlayHit, coord: Coord,
) {
    match node_right_click(session.tool(), ui.io().key_shift(), node_hit) {
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
                    frame
                        .cursor
                        .map(|cursor| NodeContext::Pick(coord, cursor_in_map_view(cursor, frame.layout.scale)))
                }
            } else {
                None
            };
            let is_block_selected = session.tool() == Tool::BlockSelect
                && session
                    .selection()
                    .is_some_and(|selection| session.selection_mode().includes(selection, coord));
            *frame.context = Some(MenuTarget {
                document: frame.id,
                coord,
                atoms,
                node,
                block_selected: is_block_selected,
                replace_for: None,
                replace_query: String::new(),
            });
            ui.open_popup(MAP_MENU_POPUP);
        },
    }
}

pub(super) fn drive_node_tool(
    session: &mut Session, frame: &mut ViewFrame<'_>, node_hit: &NodeOverlayHit, clicks: Clicks,
    is_blame_capturing: bool,
) {
    if !frame.is_focused {
        session.cancel_node_drag();
    }

    if !is_blame_capturing
        && clicks.is_left_double_clicked
        && let Some(coord) = frame.coord
        && let Some(cursor) = frame.cursor
    {
        frame.interaction.cursor = Some(cursor_in_map_view(cursor, frame.layout.scale));
        request_pick(frame.interaction, PickRequest::NodeSeed(coord));
    } else if !is_blame_capturing
        && clicks.is_left_clicked
        && let Some(coord) = node_hit.handle
    {
        session.start_node_drag(coord);
    }

    if session.node_dragging() {
        if clicks.is_left_down {
            if let Some(coord) = frame.coord {
                session.update_node_drag(coord);
            }
        } else {
            session.finish_node_drag(frame.is_hovered && frame.coord.is_some());
        }
    }
}

impl UiState {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_tool_gizmo(
        &mut self, ui: &Ui, session: &mut Session, settings: &Settings, frame: &mut ViewFrame<'_>, tool: Tool,
        paste_target: Option<Selection>, gizmo_view: GizmoMapView,
    ) -> bool {
        match tool {
            Tool::Select => {
                self.gizmo
                    .draw(
                        ui,
                        session,
                        settings,
                        frame.camera,
                        self.inspector.transform_mode(),
                        gizmo_view,
                    )
                    .captures_mouse
            },
            Tool::Place => {
                self.gizmo
                    .draw_placement_direction(ui, session, settings, frame.camera, frame.coord, gizmo_view)
                    .captures_mouse
            },
            Tool::BlockSelect => self.draw_block_gizmo(ui, session, settings, frame, paste_target, gizmo_view),
            Tool::Node | Tool::Comment | Tool::Delete | Tool::Replace | Tool::Fill => {
                self.gizmo.cancel();

                false
            },
        }
    }

    pub(super) fn end_strokes(&mut self, session: &Session, clicks: Clicks, is_gizmo_capturing: bool) {
        if clicks.is_left_clicked {
            self.placement_stroke = None;
            self.pick_stroke = None;
        }

        if !clicks.is_left_down || !matches!(session.tool(), Tool::Delete | Tool::Replace) {
            self.pick_stroke = None;
        }

        if self.placement_stroke.as_ref().is_some_and(|stroke| {
            !clicks.is_left_down
                || is_gizmo_capturing
                || !stroke.matches_context(session.tool(), session.palette(), session.z())
        }) {
            self.placement_stroke = None;
        }
    }

    pub(super) fn apply_tool_at(
        &mut self, ui: &Ui, session: &mut Session, settings: &Settings, frame: &mut ViewFrame<'_>, cursor: [f32; 2],
        clicks: Clicks,
    ) {
        let pixel = [cursor[0].floor() as u32, cursor[1].floor() as u32];
        frame.interaction.cursor = Some(cursor_in_map_view(cursor, frame.layout.scale));
        let is_alternate = settings.keybindings.get(KeybindAction::ToolAlternate).is_held(ui);

        let Some(coord) = frame.coord else {
            return;
        };

        let tool = session.tool();
        if !matches!(tool, Tool::Place | Tool::Node) {
            self.placement_stroke = None;
        }

        match tool {
            Tool::Place => self.place_at(ui, session, settings, frame, coord, clicks, is_alternate),
            Tool::Select => {
                if clicks.is_left_double_clicked {
                    request_pick(frame.interaction, PickRequest::NodeSeed(coord));
                } else if clicks.is_left_clicked {
                    request_pick(frame.interaction, PickRequest::Select);
                }
            },
            Tool::Node => {},
            Tool::Comment => {
                if clicks.is_left_clicked && session.coop_shared_map_path(frame.id).is_some() {
                    self.comment_draft = Some(CommentDraft::new(frame.id, frame.camera.screen_to_map(cursor)));
                    ui.open_popup(COMMENT_POPUP);
                }
            },
            Tool::BlockSelect => self.start_rectangle(ui, session, frame, coord, clicks.is_left_clicked),
            Tool::Delete | Tool::Replace => self.pick_at(ui, session, frame, coord, pixel, clicks, is_alternate),
            Tool::Fill => self.fill_at(ui, session, coord, clicks.is_left_clicked),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn place_at(
        &mut self, ui: &Ui, session: &mut Session, settings: &Settings, frame: &mut ViewFrame<'_>, coord: Coord,
        clicks: Clicks, is_alternate: bool,
    ) {
        if clicks.is_left_clicked {
            self.placement_stroke = session
                .palette()
                .cloned()
                .map(|prefab| PlacementStroke::new(prefab, session.z()));
        }

        let group = session
            .can_edit_at(coord)
            .then(|| self.placement_stroke.as_mut().and_then(|stroke| stroke.visit(coord)))
            .flatten();
        let placed = group.and_then(|group| {
            if is_alternate {
                session.place_replacing_objs_at(coord, Some(group))
            } else {
                session.place_at(coord, Some(group))
            }
        });
        if settings.tile_place_flash
            && let Some(owner) = placed
        {
            let flash = ActivePlacementFlash {
                owner,
                coord,
                started_at: ui.time(),
            };
            self.placement_flash = Some(flash);
            frame.interaction.placement_flash = flash.sample(ui.time());
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn pick_at(
        &mut self, ui: &Ui, session: &mut Session, frame: &mut ViewFrame<'_>, coord: Coord, pixel: [u32; 2],
        clicks: Clicks, is_alternate: bool,
    ) {
        let tool = session.tool();
        if tool == Tool::Replace && session.palette().is_none() {
            ui.tooltip_text("Choose a brush to replace atoms with");
        }

        if clicks.is_left_clicked {
            self.pick_stroke = Some(PickStroke::new(pixel, session.z()));
        }

        if tool == Tool::Delete && is_alternate {
            frame.interaction.cursor = None;
            if let Some(group) = self.pick_stroke.as_mut().and_then(|stroke| stroke.clear(coord)) {
                session.delete_tile(coord, Some(group));
            }
        } else if clicks.is_left_clicked
            || (clicks.is_left_down && self.pick_stroke.as_mut().is_some_and(|stroke| stroke.move_to(pixel)))
        {
            let request = match tool {
                Tool::Delete => PickRequest::Delete,
                _ => PickRequest::Replace,
            };
            request_pick(frame.interaction, request);
        }
    }

    fn fill_at(&mut self, ui: &Ui, session: &mut Session, coord: Coord, is_left_clicked: bool) {
        if session.selection_mask().is_some_and(|mask| !mask.includes(coord)) {
            ui.tooltip_text("Click a selected tile, or Clear selection to fill elsewhere");
        }

        if is_left_clicked
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
    }
}
