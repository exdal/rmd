use dear_imgui_rs::{MouseButton, MouseCursor, Ui};
use dmm::{Coord, Size};
use ui::widgets::dpi;

use self::{
    direction::{current_direction, facing_direction, picked_direction_target},
    drag::{anchored_member, dragged_group, transform_mutations},
    draw::{draw_direction_wheel, draw_gizmo},
    region::{dragged_region_selection, draw_resize_handles, region_hit_at, resize_cursor},
    state::{
        AXIS_LENGTH, CENTER_HALF_SIZE, DirectionGesture, DirectionGestureTarget, DirectionPhase, DirectionTarget,
        DirectionUpdate, DragMember, DragState, HIT_RADIUS, Handle, RegionDragAction, RegionDragState,
    },
};
use crate::{
    document::{command::EditGroupId, instance::InstanceChange, selection::Selection},
    inspector::{panel::TransformMode, session::GizmoMember},
    session::Session,
    settings::Settings,
    viewport::camera::Controller,
};

mod direction;
mod drag;
mod draw;
mod region;
mod state;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct GizmoResponse {
    pub captures_mouse: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RegionGizmoResponse {
    pub captures_mouse: bool,
    pub selection: Selection,
    pub drag: Option<RegionDrag>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RegionDrag {
    Move { copy: bool, released: bool },
    Resize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RegionGizmoTarget {
    pub selection: Selection,
    pub map_size: Size,
    pub tile_size: u32,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct GizmoMapView {
    pub min: [f32; 2],
    pub max: [f32; 2],
    pub hovered: bool,
}

#[derive(Default)]
pub(crate) struct GizmoState {
    drag: Option<DragState>,
    direction: Option<DirectionGesture>,
    region_drag: Option<RegionDragState>,
}

impl GizmoState {
    pub(crate) const fn is_interacting(&self) -> bool {
        self.drag.is_some() || self.direction.is_some() || self.region_drag.is_some()
    }

    pub(crate) fn is_handle_hovered(
        &self, ui: &Ui, session: &Session, camera: &Controller, mode: TransformMode, map_view: GizmoMapView,
    ) -> bool {
        if self.is_interacting() {
            return true;
        }

        let members = mode_members(&session.gizmo_members(), mode);
        let Some(bounds) = union_bounds(&members) else {
            return false;
        };
        let origin = gizmo_origin(camera, map_view.min, bounds);

        map_view.hovered && handle_at(ui.io().mouse_pos(), origin, map_view.min, map_view.max, dpi(ui)).is_some()
    }

    pub(crate) fn cancel(&mut self) {
        self.drag = None;
        self.direction = None;
        self.region_drag = None;
    }

    pub(crate) fn draw(
        &mut self, ui: &Ui, session: &mut Session, settings: &Settings, camera: &Controller, mode: TransformMode,
        map_view: GizmoMapView,
    ) -> GizmoResponse {
        self.region_drag = None;
        let all_members = session.gizmo_members();
        if all_members.is_empty() {
            self.drag = None;
            self.direction = None;

            return GizmoResponse::default();
        }

        let mut members = mode_members(&all_members, mode);
        let has_handles = !members.is_empty();
        if !has_handles {
            self.drag = None;
        }

        if self.drag.as_ref().is_some_and(|drag| {
            drag.mode != mode
                || !drag
                    .members
                    .iter()
                    .map(|member| member.id)
                    .eq(members.iter().map(|member| member.id))
        }) {
            self.drag = None;
        }

        let mouse = ui.io().mouse_pos();
        let scale = dpi(ui);
        let initial_origin = union_bounds(&members).map(|bounds| gizmo_origin(camera, map_view.min, bounds));
        let direction_origin = tiles_center(&all_members)
            .map(|center| tile_point_origin(camera, map_view.min, center, session.options.tile_size));
        let was_dragging = self.drag.is_some();
        let mut direction_target = session.gizmo_transform();
        let was_direction_active = match direction_target {
            Some(target) => self.update_direction(
                ui,
                session,
                settings,
                DirectionUpdate {
                    target: picked_direction_target(target),
                    origin: direction_origin,
                    placement_coord: None,
                    map_view,
                },
            ),
            None => {
                self.direction = None;

                false
            },
        };
        if let Some(updated) = session.picked_transform() {
            direction_target = Some(updated);
        }
        members = mode_members(&session.gizmo_members(), mode);

        let hovered_handle = initial_origin
            .filter(|_| map_view.hovered)
            .and_then(|origin| handle_at(mouse, origin, map_view.min, map_view.max, scale));

        if self.direction.is_none()
            && self.drag.is_none()
            && let Some(handle) = hovered_handle
            && ui.is_mouse_clicked(MouseButton::Left)
            && let Some(bounds) = union_bounds(&members)
        {
            self.drag = Some(DragState {
                members: members
                    .iter()
                    .map(|member| DragMember {
                        id: member.id,
                        start_values: member_values(member, mode),
                        start_anchor: [member.sprite.x, member.sprite.y],
                        start_coord: member.coord,
                    })
                    .collect(),
                start_group_anchor: [bounds[0], bounds[1]],
                mode,
                handle,
                start_mouse: mouse,
                zoom: camera.camera.zoom.max(f32::EPSILON),
                tile_size: session.options.tile_size.max(1),
                group: EditGroupId::new(),
                is_rebuilding: members.iter().any(|member| !member.is_movable),
            });
        }

        let active_handle = self.drag.as_ref().map(|drag| drag.handle);
        if let Some(drag) = &self.drag {
            if !ui.is_mouse_down(MouseButton::Left) {
                self.drag = None;
            } else if mouse != drag.start_mouse && mouse.iter().all(|value| value.is_finite()) {
                let size = session
                    .map()
                    .map_or((1, 1), |map| (map.size().x.max(1), map.size().y.max(1)));
                let shift_snap = ui.io().key_shift();
                let ctrl_reanchor = ui.io().key_ctrl();
                let group = dragged_group(drag, mouse, shift_snap);
                let changes = drag
                    .members
                    .iter()
                    .filter_map(|member| {
                        let current = members.iter().find(|current| current.id == member.id)?;
                        let (coord, next) = anchored_member(drag, member, group, ctrl_reanchor, size);
                        let mutations = transform_mutations(mode, member_values(current, mode), next);
                        let to_coord = (coord != current.coord).then_some(coord);

                        (to_coord.is_some() || !mutations.is_empty()).then_some(InstanceChange {
                            id: member.id,
                            to_coord,
                            path: None,
                            mutations,
                        })
                    })
                    .collect::<Vec<_>>();

                if !changes.is_empty() {
                    let label = format!("move {} offsets", mode.label().to_ascii_lowercase());
                    if drag.is_rebuilding {
                        let changes = drag
                            .members
                            .iter()
                            .map(|member| {
                                let (coord, next) = anchored_member(drag, member, group, ctrl_reanchor, size);

                                InstanceChange {
                                    id: member.id,
                                    to_coord: (coord != member.start_coord).then_some(coord),
                                    path: None,
                                    mutations: transform_mutations(mode, member.start_values, next),
                                }
                            })
                            .collect::<Vec<_>>();
                        session.rechange_instances(&changes, label, drag.group);
                    } else {
                        session.change_instances(&changes, label, Some(drag.group));
                    }

                    members = mode_members(&session.gizmo_members(), mode);
                }
            }
        }

        let origin = union_bounds(&members).map(|bounds| gizmo_origin(camera, map_view.min, bounds));
        let hovered = if self.drag.is_some() {
            None
        } else if map_view.hovered {
            origin.and_then(|origin| handle_at(mouse, origin, map_view.min, map_view.max, scale))
        } else {
            None
        };
        let hot = self.drag.as_ref().map(|drag| drag.handle).or(hovered).or(active_handle);
        let open_wheel = self.direction.and_then(|gesture| match gesture.phase {
            DirectionPhase::Open {
                origin, last_hovered, ..
            } => gesture.set.map(|set| (origin, set, last_hovered)),
            DirectionPhase::Pending => None,
        });
        let direction_state = direction_target.map(|target| picked_direction_target(target).state);

        if let Some(((wheel_origin, set, hovered_direction), state)) = open_wheel.zip(direction_state) {
            if hovered_direction.is_some() {
                ui.set_mouse_cursor(Some(MouseCursor::Hand));
            }
            draw_direction_wheel(
                ui,
                wheel_origin,
                map_view,
                set,
                current_direction(state, set),
                hovered_direction,
            );
        } else if let Some(origin) = origin {
            if let Some(handle) = hot {
                ui.set_mouse_cursor(Some(handle.cursor()));
            }
            let facing = direction_state.and_then(facing_direction);
            draw_gizmo(ui, origin, map_view, hot, facing);
        }

        GizmoResponse {
            captures_mouse: was_dragging
                || self.drag.is_some()
                || was_direction_active
                || self.direction.is_some()
                || hovered.is_some(),
        }
    }

    pub(crate) fn draw_region(
        &mut self, ui: &Ui, camera: &Controller, target: RegionGizmoTarget, map_view: GizmoMapView,
    ) -> RegionGizmoResponse {
        self.drag = None;
        self.direction = None;

        let RegionGizmoTarget {
            mut selection,
            map_size,
            tile_size,
        } = target;

        let mouse = ui.io().mouse_pos();
        let was_dragging = self.region_drag.is_some();
        let hovered = map_view
            .hovered
            .then(|| {
                region_hit_at(
                    camera,
                    map_view,
                    selection,
                    tile_size,
                    mouse,
                    dpi(ui),
                    ui.io().key_ctrl(),
                )
            })
            .flatten();
        if self.region_drag.is_none()
            && let Some(action) = hovered
            && ui.is_mouse_clicked(MouseButton::Left)
        {
            self.region_drag = Some(RegionDragState {
                start_selection: selection,
                action,
                start_mouse: mouse,
                zoom: camera.camera.zoom.max(f32::EPSILON),
                tile_size: tile_size.max(1),
                map_size,
            });
        }

        let drag = self.region_drag.map(|state| {
            if mouse.iter().all(|value| value.is_finite()) {
                selection = dragged_region_selection(state, mouse);
            }

            match state.action {
                RegionDragAction::Move { copy } => RegionDrag::Move {
                    copy,
                    released: !ui.is_mouse_down(MouseButton::Left),
                },
                RegionDragAction::Resize { .. } => RegionDrag::Resize,
            }
        });
        if !ui.is_mouse_down(MouseButton::Left) {
            self.region_drag = None;
        }

        RegionGizmoResponse {
            captures_mouse: was_dragging || self.region_drag.is_some() || hovered.is_some(),
            selection,
            drag,
        }
    }

    pub(crate) fn draw_region_overlay(
        &self, ui: &Ui, camera: &Controller, target: RegionGizmoTarget, map_view: GizmoMapView,
    ) {
        let dragging = self.region_drag.map(|drag| drag.action);
        if !matches!(dragging, Some(RegionDragAction::Move { .. })) {
            draw_resize_handles(ui, camera, map_view, target.selection, target.tile_size);
        }

        let action = dragging.or_else(|| {
            map_view
                .hovered
                .then(|| {
                    region_hit_at(
                        camera,
                        map_view,
                        target.selection,
                        target.tile_size,
                        ui.io().mouse_pos(),
                        dpi(ui),
                        ui.io().key_ctrl(),
                    )
                })
                .flatten()
        });

        match action {
            Some(RegionDragAction::Resize { x, y }) => ui.set_mouse_cursor(Some(resize_cursor(x, y))),
            Some(RegionDragAction::Move { .. }) => ui.set_mouse_cursor(Some(MouseCursor::ResizeAll)),
            None => {},
        }
    }

    pub(crate) fn placement_coord(&self) -> Option<Coord> {
        self.direction.and_then(|gesture| {
            (gesture.target == DirectionGestureTarget::Placement)
                .then_some(gesture.placement_coord)
                .flatten()
        })
    }

    pub(crate) fn draw_placement_direction(
        &mut self, ui: &Ui, session: &mut Session, settings: &Settings, camera: &Controller, coord: Option<Coord>,
        map_view: GizmoMapView,
    ) -> GizmoResponse {
        self.drag = None;
        self.region_drag = None;
        let Some(state) = session.placement_direction() else {
            self.direction = None;

            return GizmoResponse::default();
        };
        let coord = self.placement_coord().or(coord);
        let origin = coord.map(|coord| tile_center_origin(camera, map_view.min, coord, session.options.tile_size));
        let target = DirectionTarget {
            id: DirectionGestureTarget::Placement,
            state,
        };
        let was_direction_active = self.update_direction(
            ui,
            session,
            settings,
            DirectionUpdate {
                target,
                origin,
                placement_coord: coord,
                map_view,
            },
        );
        let state = session.placement_direction().unwrap_or(state);
        let open_wheel = self.direction.and_then(|gesture| match gesture.phase {
            DirectionPhase::Open {
                origin, last_hovered, ..
            } => gesture.set.map(|set| (origin, set, last_hovered)),
            DirectionPhase::Pending => None,
        });

        if let Some((wheel_origin, set, hovered_direction)) = open_wheel {
            if hovered_direction.is_some() {
                ui.set_mouse_cursor(Some(MouseCursor::Hand));
            }
            draw_direction_wheel(
                ui,
                wheel_origin,
                map_view,
                set,
                current_direction(state, set),
                hovered_direction,
            );
        }

        GizmoResponse {
            captures_mouse: was_direction_active || self.direction.is_some(),
        }
    }
}

fn mode_members(members: &[GizmoMember], mode: TransformMode) -> Vec<GizmoMember> {
    members
        .iter()
        .filter(|member| mode == TransformMode::Pixel || member.is_movable)
        .copied()
        .collect()
}

fn member_values(member: &GizmoMember, mode: TransformMode) -> [i32; 2] {
    match mode {
        TransformMode::Pixel => member.pixel,
        TransformMode::Step => member.step,
    }
}

fn union_bounds(members: &[GizmoMember]) -> Option<[f32; 4]> {
    members
        .iter()
        .map(|member| {
            [
                member.sprite.x,
                member.sprite.y,
                member.sprite.width,
                member.sprite.height,
            ]
        })
        .reduce(|union, bounds| {
            let x = union[0].min(bounds[0]);
            let y = union[1].min(bounds[1]);
            let right = (union[0] + union[2]).max(bounds[0] + bounds[2]);
            let top = (union[1] + union[3]).max(bounds[1] + bounds[3]);

            [x, y, right - x, top - y]
        })
}

fn tiles_center(members: &[GizmoMember]) -> Option<[f32; 2]> {
    let first = members.first()?.coord;
    let (min, max) = members.iter().fold((first, first), |(min, max), member| {
        (
            Coord::new(min.x.min(member.coord.x), min.y.min(member.coord.y), min.z),
            Coord::new(max.x.max(member.coord.x), max.y.max(member.coord.y), max.z),
        )
    });

    Some([(min.x + max.x) as f32 * 0.5, (min.y + max.y) as f32 * 0.5])
}

fn gizmo_origin(camera: &Controller, viewport_min: [f32; 2], bounds: [f32; 4]) -> [f32; 2] {
    let center = camera.map_to_screen([bounds[0] + bounds[2] * 0.5, bounds[1] + bounds[3] * 0.5]);

    [viewport_min[0] + center[0], viewport_min[1] + center[1]]
}

fn tile_center_origin(camera: &Controller, viewport_min: [f32; 2], coord: Coord, tile_size: u32) -> [f32; 2] {
    tile_point_origin(camera, viewport_min, [coord.x as f32, coord.y as f32], tile_size)
}

fn tile_point_origin(camera: &Controller, viewport_min: [f32; 2], tile: [f32; 2], tile_size: u32) -> [f32; 2] {
    let tile_size = tile_size.max(1) as f32;
    let center = camera.map_to_screen([(tile[0] - 0.5) * tile_size, (tile[1] - 0.5) * tile_size]);

    [viewport_min[0] + center[0], viewport_min[1] + center[1]]
}

fn handle_at(
    point: [f32; 2], origin: [f32; 2], viewport_min: [f32; 2], viewport_max: [f32; 2], scale: f32,
) -> Option<Handle> {
    if !contains(point, viewport_min, viewport_max) {
        return None;
    }

    let point = unscaled(point, origin, scale);

    let center_min = [origin[0] - CENTER_HALF_SIZE, origin[1] - CENTER_HALF_SIZE];
    let center_max = [origin[0] + CENTER_HALF_SIZE, origin[1] + CENTER_HALF_SIZE];
    if contains(point, center_min, center_max) {
        return Some(Handle::XY);
    }

    let on_x = point[0] >= origin[0] + CENTER_HALF_SIZE
        && point[0] <= origin[0] + AXIS_LENGTH + HIT_RADIUS
        && (point[1] - origin[1]).abs() <= HIT_RADIUS;
    if on_x {
        return Some(Handle::X);
    }

    let on_y = point[1] <= origin[1] - CENTER_HALF_SIZE
        && point[1] >= origin[1] - AXIS_LENGTH - HIT_RADIUS
        && (point[0] - origin[0]).abs() <= HIT_RADIUS;

    on_y.then_some(Handle::Y)
}

fn unscaled(point: [f32; 2], origin: [f32; 2], scale: f32) -> [f32; 2] {
    [
        origin[0] + (point[0] - origin[0]) / scale,
        origin[1] + (point[1] - origin[1]) / scale,
    ]
}

fn contains(point: [f32; 2], min: [f32; 2], max: [f32; 2]) -> bool {
    point.iter().all(|value| value.is_finite())
        && point[0] >= min[0]
        && point[0] <= max[0]
        && point[1] >= min[1]
        && point[1] <= max[1]
}

#[cfg(test)]
mod tests;
