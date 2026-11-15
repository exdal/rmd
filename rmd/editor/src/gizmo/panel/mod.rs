use base::types::{Identifier, Value};

use dear_imgui_rs::{DrawListMut, MouseButton, MouseCursor, StyleColor, Ui};
use dmi::metadata::Dir;
use dmm::{Coord, Size};
use ui::widgets::dpi;

use crate::{
    document::{InstanceChange, PrefabInstanceId, Selection, VarMutation, command::EditGroupId},
    gizmo::{
        direction::{DirectionState, DirectionalTypes},
        transform::anchor_to_tile,
    },
    inspector::{
        panel::TransformMode,
        session::{GizmoMember, PickedTransform},
    },
    session::Session,
    settings::{Settings, keybind::KeybindAction},
    tool::SelectionRotation,
    viewport::camera::Controller,
};

const AXIS_LENGTH: f32 = 48.0;
const ARROW_LENGTH: f32 = 11.0;
const ARROW_HALF_WIDTH: f32 = 6.0;
const CENTER_HALF_SIZE: f32 = 5.0;
const HIT_RADIUS: f32 = 7.0;
const LINE_THICKNESS: f32 = 3.0;
const HOT_LINE_THICKNESS: f32 = 5.0;
const RESIZE_HANDLE_OFFSET: f32 = 7.0;
const RESIZE_HANDLE_HIT_HALF_SIZE: f32 = 5.0;
const RESIZE_HANDLE_HALF_SIZE: f32 = 3.5;
const DIRECTION_HOLD_SECONDS: f64 = 0.2;
const DIRECTION_TIME_EPSILON: f64 = 1.0e-9;
const DIRECTION_INNER_RADIUS: f32 = 22.0;
const DIRECTION_OUTER_RADIUS: f32 = 78.0;
const DIRECTION_TIP_RADIUS: f32 = 67.0;
const DIRECTION_ARROW_LENGTH: f32 = 14.0;
const DIRECTION_ARROW_HALF_WIDTH: f32 = 7.5;
const DIRECTION_SECTOR_GAP: f32 = 0.035;
const DIRECTION_SECTOR_SEGMENTS: usize = 8;
const FACING_OFFSET: f32 = 26.0;
const FACING_LENGTH: f32 = 20.0;
const FACING_ARROW_LENGTH: f32 = 9.0;
const FACING_ARROW_HALF_WIDTH: f32 = 6.0;
const FACING_LINE_THICKNESS: f32 = 3.0;

const CLOCKWISE_DIRECTIONS: [Dir; 8] = [
    Dir::North,
    Dir::Northeast,
    Dir::East,
    Dir::Southeast,
    Dir::South,
    Dir::Southwest,
    Dir::West,
    Dir::Northwest,
];
const ALL_DIRECTION_SLOTS: u8 = CLOCKWISE_DIRECTIONS.len() as u8;
const CARDINAL_DIRECTION_SLOTS: u8 = ALL_DIRECTION_SLOTS / 2;

const X_COLOR: [f32; 4] = [0.92, 0.24, 0.22, 1.0];
const Y_COLOR: [f32; 4] = [0.28, 0.82, 0.35, 1.0];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Handle {
    X,
    Y,
    XY,
}

impl Handle {
    const fn moves_x(self) -> bool {
        matches!(self, Self::X | Self::XY)
    }

    const fn moves_y(self) -> bool {
        matches!(self, Self::Y | Self::XY)
    }

    const fn cursor(self) -> MouseCursor {
        match self {
            Self::X => MouseCursor::ResizeEW,
            Self::Y => MouseCursor::ResizeNS,
            Self::XY => MouseCursor::ResizeAll,
        }
    }
}

#[derive(Debug, Clone)]
struct DragState {
    members: Vec<DragMember>,
    start_group_anchor: [f32; 2],
    mode: TransformMode,
    handle: Handle,
    start_mouse: [f32; 2],
    zoom: f32,
    tile_size: u32,
    group: EditGroupId,
    is_rebuilding: bool,
}

#[derive(Debug, Clone, Copy)]
struct DragMember {
    id: PrefabInstanceId,
    start_values: [i32; 2],
    start_anchor: [f32; 2],
    start_coord: Coord,
}

#[derive(Debug, Clone, Copy)]
struct RegionDragState {
    start_selection: Selection,
    action: RegionDragAction,
    start_mouse: [f32; 2],
    zoom: f32,
    tile_size: u32,
    map_size: Size,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegionDragAction {
    Move { copy: bool },
    Resize { x: i8, y: i8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DirectionSet {
    supported: [bool; 8],
    slots: u8,
}

impl DirectionSet {
    fn contains(self, direction: Dir) -> bool {
        self.supported[clockwise_index(direction)]
    }

    fn slot_directions(self) -> impl Iterator<Item = Dir> {
        CLOCKWISE_DIRECTIONS
            .into_iter()
            .filter(move |direction| self.slots == ALL_DIRECTION_SLOTS || !is_diagonal(*direction))
    }

    fn directions(self) -> impl Iterator<Item = Dir> {
        CLOCKWISE_DIRECTIONS
            .into_iter()
            .filter(move |direction| self.contains(*direction))
    }

    fn len(self) -> usize {
        self.supported.iter().filter(|supported| **supported).count()
    }
}

#[derive(Debug, Clone, Copy)]
struct DirectionGesture {
    target: DirectionGestureTarget,
    placement_coord: Option<Coord>,
    pressed_at: f64,
    action: DirectionAction,
    set: Option<DirectionSet>,
    phase: DirectionPhase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirectionGestureTarget {
    Selection(PrefabInstanceId),
    Placement,
}

#[derive(Debug, Clone, Copy)]
struct DirectionTarget {
    id: DirectionGestureTarget,
    state: DirectionState,
}

#[derive(Debug, Clone, Copy)]
struct DirectionUpdate {
    target: DirectionTarget,
    origin: Option<[f32; 2]>,
    placement_coord: Option<Coord>,
    map_view: GizmoMapView,
}

#[derive(Debug, Clone, Copy)]
enum DirectionPhase {
    Pending,
    Open {
        origin: [f32; 2],
        group: EditGroupId,
        last_hovered: Option<Dir>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingDirectionOutcome {
    Pending,
    Open,
    Tap,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirectionTap {
    RotateGroup,
    Face(Dir),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirectionAction {
    Face,
    RotateGroup,
}

#[derive(Debug, Clone, Copy)]
enum DirectionRequest {
    RotateGroup {
        rotation: SelectionRotation,
        group: Option<EditGroupId>,
    },
    Face {
        direction: Dir,
        group: Option<EditGroupId>,
    },
}

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

    fn update_direction(
        &mut self, ui: &Ui, session: &mut Session, settings: &Settings, update: DirectionUpdate,
    ) -> bool {
        let DirectionUpdate {
            target,
            origin,
            placement_coord,
            map_view,
        } = update;
        let rotation_key = settings.keybindings.get(KeybindAction::Rotate);
        if self.direction.is_some_and(|gesture| gesture.target != target.id) {
            self.direction = None;
        }
        let is_group =
            matches!(target.id, DirectionGestureTarget::Selection(_)) && session.picked_instances().len() > 1;
        let (action, set) = if is_group {
            (
                DirectionAction::RotateGroup,
                Some(group_direction_set(reference_direction(target.state))),
            )
        } else {
            (
                DirectionAction::Face,
                direction_set(
                    target.state.dmi_directions,
                    target.state.declared,
                    target.state.directional_types,
                ),
            )
        };
        if self.direction.is_none()
            && self.drag.is_none()
            && map_view.hovered
            && rotation_key.is_pressed(ui)
            && origin.is_some()
            && set.is_some()
        {
            self.direction = Some(DirectionGesture {
                target: target.id,
                placement_coord,
                pressed_at: ui.time(),
                action,
                set,
                phase: DirectionPhase::Pending,
            });
        }
        let was_direction_active = self.direction.is_some();
        let key_down = rotation_key.is_down(ui);
        let key_released = rotation_key.is_released(ui);
        let mut request = None;

        if let Some(mut gesture) = self.direction {
            if matches!(gesture.phase, DirectionPhase::Pending) {
                match pending_direction_outcome(gesture.pressed_at, ui.time(), key_down, key_released) {
                    PendingDirectionOutcome::Pending => {},
                    PendingDirectionOutcome::Open => {
                        if let Some(origin) = origin
                            && gesture.set.is_some()
                        {
                            gesture.phase = DirectionPhase::Open {
                                origin,
                                group: EditGroupId::new(),
                                last_hovered: None,
                            };
                        } else {
                            self.direction = None;
                        }
                    },
                    PendingDirectionOutcome::Tap => {
                        match direction_tap(gesture.action, gesture.set, target.state) {
                            Some(DirectionTap::RotateGroup) => {
                                request = Some(DirectionRequest::RotateGroup {
                                    rotation: SelectionRotation::Clockwise,
                                    group: None,
                                });
                            },
                            Some(DirectionTap::Face(direction)) => {
                                request = Some(DirectionRequest::Face { direction, group: None });
                            },
                            None => {},
                        }
                        self.direction = None;
                    },
                    PendingDirectionOutcome::Cancel => self.direction = None,
                }
            }

            if let DirectionPhase::Open {
                origin,
                group,
                mut last_hovered,
            } = gesture.phase
                && let Some(set) = gesture.set
            {
                if key_released || !key_down {
                    self.direction = None;
                } else {
                    let hovered = direction_at(ui.io().mouse_pos(), origin, map_view.min, map_view.max, set, dpi(ui));
                    if hovered != last_hovered {
                        last_hovered = hovered;
                        if let Some(direction) = hovered {
                            request = hover_request(gesture.action, target.state, set, direction, group);
                        }
                    }
                    gesture.phase = DirectionPhase::Open {
                        origin,
                        group,
                        last_hovered,
                    };
                    self.direction = Some(gesture);
                }
            } else if self.direction.is_some() {
                self.direction = Some(gesture);
            }
        }

        match request {
            Some(DirectionRequest::RotateGroup { rotation, group }) => {
                session.rotate_picked_group(rotation, group);
            },
            Some(DirectionRequest::Face { direction, group }) => apply_direction(session, target, direction, group),
            None => {},
        }

        was_direction_active
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

fn dragged_region_selection(drag: RegionDragState, mouse: [f32; 2]) -> Selection {
    let scale = drag.zoom * drag.tile_size as f32;

    let delta = [
        ((mouse[0] - drag.start_mouse[0]) / scale).round() as i64,
        (-(mouse[1] - drag.start_mouse[1]) / scale).round() as i64,
    ];

    if let RegionDragAction::Resize { x, y } = drag.action {
        let mut selection = drag.start_selection;
        for (edge, delta, min, max, limit) in [
            (x, delta[0], &mut selection.min.x, &mut selection.max.x, drag.map_size.x),
            (y, delta[1], &mut selection.min.y, &mut selection.max.y, drag.map_size.y),
        ] {
            if edge < 0 {
                *min = i64::from(*min).saturating_add(delta).clamp(1, i64::from(*max)) as u32;
            } else if edge > 0 {
                *max = i64::from(*max)
                    .saturating_add(delta)
                    .clamp(i64::from(*min), i64::from(limit)) as u32;
            }
        }
        return selection;
    }

    let max_x = drag
        .map_size
        .x
        .saturating_sub(drag.start_selection.width())
        .saturating_add(1)
        .max(1);
    let max_y = drag
        .map_size
        .y
        .saturating_sub(drag.start_selection.height())
        .saturating_add(1)
        .max(1);

    let min = Coord::new(
        i64::from(drag.start_selection.min.x)
            .saturating_add(delta[0])
            .clamp(1, i64::from(max_x)) as u32,
        i64::from(drag.start_selection.min.y)
            .saturating_add(delta[1])
            .clamp(1, i64::from(max_y)) as u32,
        drag.start_selection.min.z,
    );

    drag.start_selection.with_min(min).unwrap_or(drag.start_selection)
}

fn region_hit_at(
    camera: &Controller, view: GizmoMapView, selection: Selection, tile_size: u32, mouse: [f32; 2], scale: f32,
    copy: bool,
) -> Option<RegionDragAction> {
    if let Some((x, y)) = resize_handle_at(camera, view, selection, tile_size, mouse, scale) {
        return Some(RegionDragAction::Resize { x, y });
    }

    let size = tile_size.max(1) as f32;
    let lower = camera.map_to_screen([(selection.min.x - 1) as f32 * size, (selection.min.y - 1) as f32 * size]);
    let upper = camera.map_to_screen([selection.max.x as f32 * size, selection.max.y as f32 * size]);
    let min = [view.min[0] + lower[0], view.min[1] + upper[1]];
    let max = [view.min[0] + upper[0], view.min[1] + lower[1]];

    (contains(mouse, view.min, view.max) && contains(mouse, min, max)).then_some(RegionDragAction::Move { copy })
}

fn resize_cursor(x: i8, y: i8) -> MouseCursor {
    match (x, y) {
        (0, _) => MouseCursor::ResizeNS,
        (_, 0) => MouseCursor::ResizeEW,
        _ if x == y => MouseCursor::ResizeNESW,
        _ => MouseCursor::ResizeNWSE,
    }
}

fn resize_handle_positions(
    camera: &Controller, viewport_min: [f32; 2], selection: Selection, tile_size: u32, scale: f32,
) -> [(i8, i8, [f32; 2]); 8] {
    let size = tile_size.max(1) as f32;
    let offset = RESIZE_HANDLE_OFFSET * scale;
    let lower = camera.map_to_screen([(selection.min.x - 1) as f32 * size, (selection.min.y - 1) as f32 * size]);
    let upper = camera.map_to_screen([selection.max.x as f32 * size, selection.max.y as f32 * size]);
    let left = viewport_min[0] + lower[0] - offset;
    let right = viewport_min[0] + upper[0] + offset;
    let bottom = viewport_min[1] + lower[1] + offset;
    let top = viewport_min[1] + upper[1] - offset;
    let center = [(left + right) * 0.5, (top + bottom) * 0.5];
    [
        (-1, -1, [left, bottom]),
        (1, -1, [right, bottom]),
        (-1, 1, [left, top]),
        (1, 1, [right, top]),
        (-1, 0, [left, center[1]]),
        (1, 0, [right, center[1]]),
        (0, -1, [center[0], bottom]),
        (0, 1, [center[0], top]),
    ]
}

fn resize_handle_at(
    camera: &Controller, view: GizmoMapView, selection: Selection, tile_size: u32, mouse: [f32; 2], scale: f32,
) -> Option<(i8, i8)> {
    if !contains(mouse, view.min, view.max) {
        return None;
    }

    let reach = RESIZE_HANDLE_HIT_HALF_SIZE * scale;
    resize_handle_positions(camera, view.min, selection, tile_size, scale)
        .into_iter()
        .find(|(_, _, position)| {
            contains(
                mouse,
                [position[0] - reach, position[1] - reach],
                [position[0] + reach, position[1] + reach],
            )
        })
        .map(|(x, y, _)| (x, y))
}

fn draw_resize_handles(
    ui: &Ui, camera: &Controller, view: GizmoMapView, selection: Selection, tile_size: u32,
) -> Option<(i8, i8)> {
    let scale = dpi(ui);
    let hovered = view
        .hovered
        .then(|| resize_handle_at(camera, view, selection, tile_size, ui.io().mouse_pos(), scale))
        .flatten();
    let half_size = RESIZE_HANDLE_HALF_SIZE * scale;
    let draw = ui.get_window_draw_list();
    draw.with_clip_rect(view.min, view.max, || {
        for (x, y, position) in resize_handle_positions(camera, view.min, selection, tile_size, scale) {
            let min = [position[0] - half_size, position[1] - half_size];
            let max = [position[0] + half_size, position[1] + half_size];
            let color = if hovered == Some((x, y)) {
                [1.0, 0.85, 0.25, 1.0]
            } else {
                [0.5, 1.0, 0.65, 1.0]
            };
            draw.add_rect(min, max, color).filled(true).build();
            draw.add_rect(min, max, [0.0, 0.0, 0.0, 0.9]).build();
        }
    });
    hovered
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

fn direction_set(
    dmi_directions: Option<u32>, declared: Option<[bool; 8]>, directional_types: Option<DirectionalTypes>,
) -> Option<DirectionSet> {
    let mut set = DirectionSet {
        supported: [false; 8],
        slots: CARDINAL_DIRECTION_SLOTS,
    };

    if let Some(supported) = directional_types.map(|types| types.supported).or(declared) {
        for (direction, supported) in Dir::ORDER.into_iter().zip(supported) {
            set.supported[clockwise_index(direction)] = supported;
        }
    } else {
        match dmi_directions {
            Some(4) => {
                for direction in [Dir::North, Dir::East, Dir::South, Dir::West] {
                    set.supported[clockwise_index(direction)] = true;
                }
            },
            Some(8) => set.supported.fill(true),
            _ => return None,
        }
    }

    set.slots = if set.directions().any(is_diagonal) {
        ALL_DIRECTION_SLOTS
    } else {
        CARDINAL_DIRECTION_SLOTS
    };

    (set.len() > 1).then_some(set)
}

fn picked_direction_target(target: PickedTransform) -> DirectionTarget {
    DirectionTarget {
        id: DirectionGestureTarget::Selection(target.picked),
        state: DirectionState {
            dir: target.dir,
            dmi_directions: target.dmi_directions,
            declared: target.declared,
            directional_types: target.directional_types,
        },
    }
}

fn clockwise_index(direction: Dir) -> usize {
    CLOCKWISE_DIRECTIONS
        .iter()
        .position(|candidate| *candidate == direction)
        .unwrap_or_default()
}

const fn is_diagonal(direction: Dir) -> bool {
    matches!(
        direction,
        Dir::Northeast | Dir::Southeast | Dir::Southwest | Dir::Northwest
    )
}

fn facing_direction(state: DirectionState) -> Option<Dir> {
    direction_set(state.dmi_directions, state.declared, state.directional_types)
        .and_then(|set| current_direction(state, set))
}

fn current_direction(state: DirectionState, set: DirectionSet) -> Option<Dir> {
    state
        .directional_types
        .and_then(|types| types.current)
        .or_else(|| Dir::from_bits(state.dir))
        .filter(|direction| set.contains(*direction))
}

fn next_clockwise_direction(set: DirectionSet, current: Option<Dir>) -> Option<Dir> {
    let (start, include_start) = current
        .map(|direction| (clockwise_index(direction), false))
        .unwrap_or((clockwise_index(Dir::South), true));

    (usize::from(!include_start)..=CLOCKWISE_DIRECTIONS.len())
        .map(|offset| CLOCKWISE_DIRECTIONS[(start + offset) % CLOCKWISE_DIRECTIONS.len()])
        .find(|direction| set.contains(*direction))
}

fn direction_tap(action: DirectionAction, set: Option<DirectionSet>, state: DirectionState) -> Option<DirectionTap> {
    if action == DirectionAction::RotateGroup {
        return Some(DirectionTap::RotateGroup);
    }

    let set = set?;

    next_clockwise_direction(set, current_direction(state, set)).map(DirectionTap::Face)
}

fn reference_direction(state: DirectionState) -> Dir {
    state
        .directional_types
        .and_then(|types| types.current)
        .or_else(|| Dir::from_bits(state.dir))
        .unwrap_or(Dir::North)
}

fn group_direction_set(reference: Dir) -> DirectionSet {
    let mut set = DirectionSet {
        supported: [false; 8],
        slots: if is_diagonal(reference) {
            ALL_DIRECTION_SLOTS
        } else {
            CARDINAL_DIRECTION_SLOTS
        },
    };
    let start = clockwise_index(reference);
    for turn in 0..4 {
        set.supported[(start + turn * 2) % CLOCKWISE_DIRECTIONS.len()] = true;
    }

    set
}

fn quarter_turns(from: Dir, to: Dir) -> Option<SelectionRotation> {
    let steps = (clockwise_index(to) + CLOCKWISE_DIRECTIONS.len() - clockwise_index(from)) % CLOCKWISE_DIRECTIONS.len();

    match steps {
        0 => Some(SelectionRotation::Original),
        2 => Some(SelectionRotation::Clockwise),
        4 => Some(SelectionRotation::Half),
        6 => Some(SelectionRotation::CounterClockwise),
        _ => None,
    }
}

fn hover_request(
    action: DirectionAction, state: DirectionState, set: DirectionSet, direction: Dir, group: EditGroupId,
) -> Option<DirectionRequest> {
    match action {
        DirectionAction::Face => (current_direction(state, set) != Some(direction)).then_some(DirectionRequest::Face {
            direction,
            group: Some(group),
        }),
        DirectionAction::RotateGroup => {
            let rotation = quarter_turns(reference_direction(state), direction)
                .filter(|rotation| *rotation != SelectionRotation::Original)?;

            Some(DirectionRequest::RotateGroup {
                rotation,
                group: Some(group),
            })
        },
    }
}

fn pending_direction_outcome(pressed_at: f64, now: f64, key_down: bool, key_released: bool) -> PendingDirectionOutcome {
    let held_long_enough = now.is_finite()
        && pressed_at.is_finite()
        && now - pressed_at + DIRECTION_TIME_EPSILON >= DIRECTION_HOLD_SECONDS;
    if held_long_enough {
        if key_down {
            PendingDirectionOutcome::Open
        } else {
            PendingDirectionOutcome::Cancel
        }
    } else if key_released {
        PendingDirectionOutcome::Tap
    } else if key_down {
        PendingDirectionOutcome::Pending
    } else {
        PendingDirectionOutcome::Cancel
    }
}

fn apply_direction(session: &mut Session, target: DirectionTarget, direction: Dir, group: Option<EditGroupId>) {
    match target.id {
        DirectionGestureTarget::Selection(_) => {
            session.set_picked_direction(direction, group);
        },
        DirectionGestureTarget::Placement => {
            session.set_placement_direction(direction);
        },
    }
}

fn direction_vector(direction: Dir) -> [f32; 2] {
    const DIAGONAL: f32 = std::f32::consts::FRAC_1_SQRT_2;

    match direction {
        Dir::South => [0.0, 1.0],
        Dir::North => [0.0, -1.0],
        Dir::East => [1.0, 0.0],
        Dir::West => [-1.0, 0.0],
        Dir::Southeast => [DIAGONAL, DIAGONAL],
        Dir::Southwest => [-DIAGONAL, DIAGONAL],
        Dir::Northeast => [DIAGONAL, -DIAGONAL],
        Dir::Northwest => [-DIAGONAL, -DIAGONAL],
    }
}

fn direction_head(origin: [f32; 2], direction: Dir, expansion: f32, scale: f32) -> [[f32; 2]; 3] {
    let vector = direction_vector(direction);
    let perpendicular = [-vector[1], vector[0]];
    let tip_radius = (DIRECTION_TIP_RADIUS + expansion) * scale;
    let base_radius = (DIRECTION_TIP_RADIUS - DIRECTION_ARROW_LENGTH - expansion) * scale;
    let half_width = (DIRECTION_ARROW_HALF_WIDTH + expansion) * scale;
    let tip = [origin[0] + vector[0] * tip_radius, origin[1] + vector[1] * tip_radius];
    let base = [origin[0] + vector[0] * base_radius, origin[1] + vector[1] * base_radius];

    [
        tip,
        [
            base[0] + perpendicular[0] * half_width,
            base[1] + perpendicular[1] * half_width,
        ],
        [
            base[0] - perpendicular[0] * half_width,
            base[1] - perpendicular[1] * half_width,
        ],
    ]
}

fn direction_at(
    point: [f32; 2], origin: [f32; 2], viewport_min: [f32; 2], viewport_max: [f32; 2], set: DirectionSet, scale: f32,
) -> Option<Dir> {
    if !contains(point, viewport_min, viewport_max) {
        return None;
    }

    let point = unscaled(point, origin, scale);
    let delta = [point[0] - origin[0], point[1] - origin[1]];
    let radius_squared = delta[0] * delta[0] + delta[1] * delta[1];
    if !(DIRECTION_INNER_RADIUS * DIRECTION_INNER_RADIUS..=DIRECTION_OUTER_RADIUS * DIRECTION_OUTER_RADIUS)
        .contains(&radius_squared)
    {
        return None;
    }

    let radius = radius_squared.sqrt();
    let minimum_dot = (std::f32::consts::PI / set.slots as f32).cos();

    set.directions()
        .map(|direction| {
            let vector = direction_vector(direction);
            let dot = (delta[0] * vector[0] + delta[1] * vector[1]) / radius;

            (direction, dot)
        })
        .filter(|(_, dot)| *dot >= minimum_dot)
        .max_by(|left, right| left.1.total_cmp(&right.1))
        .map(|(direction, _)| direction)
}

#[derive(Debug, Clone, Copy)]
struct DraggedGroup {
    delta: [i32; 2],
    anchor: [f32; 2],
}

#[derive(Debug, Clone, Copy)]
struct DraggedTransform {
    values: [i32; 2],
    anchor: [f32; 2],
}

fn dragged_group(drag: &DragState, mouse: [f32; 2], snap: bool) -> DraggedGroup {
    let map_delta = [
        (mouse[0] - drag.start_mouse[0]) / drag.zoom,
        -(mouse[1] - drag.start_mouse[1]) / drag.zoom,
    ];
    let mut delta = [0; 2];
    let mut anchor = drag.start_group_anchor;

    for axis in 0..2 {
        let enabled = if axis == 0 {
            drag.handle.moves_x()
        } else {
            drag.handle.moves_y()
        };
        if !enabled {
            continue;
        }

        let desired = drag.start_group_anchor[axis] + map_delta[axis];
        anchor[axis] = if snap {
            let tile = drag.tile_size as f32;

            (desired / tile).round() * tile
        } else {
            desired
        };
        delta[axis] = rounded_i32(anchor[axis] - drag.start_group_anchor[axis]);
    }

    DraggedGroup { delta, anchor }
}

fn dragged_member(drag: &DragState, member: &DragMember, group: DraggedGroup) -> DraggedTransform {
    let offset = [
        member.start_anchor[0] - drag.start_group_anchor[0],
        member.start_anchor[1] - drag.start_group_anchor[1],
    ];

    DraggedTransform {
        values: [
            member.start_values[0].saturating_add(group.delta[0]),
            member.start_values[1].saturating_add(group.delta[1]),
        ],
        anchor: [group.anchor[0] + offset[0], group.anchor[1] + offset[1]],
    }
}

fn anchored_member(
    drag: &DragState, member: &DragMember, group: DraggedGroup, reanchor: bool, size: (u32, u32),
) -> (Coord, [i32; 2]) {
    let dragged = dragged_member(drag, member, group);
    if !reanchor {
        return (member.start_coord, dragged.values);
    }

    let anchored = anchor_to_tile(dragged.anchor, member.start_coord, drag.tile_size, size);
    let next = [
        dragged.values[0].saturating_add(anchored.adjust[0]),
        dragged.values[1].saturating_add(anchored.adjust[1]),
    ];

    (anchored.coord, next)
}

fn transform_mutations(mode: TransformMode, current: [i32; 2], next: [i32; 2]) -> Vec<VarMutation> {
    let (x_name, y_name) = mode.variables();
    let mut mutations = Vec::with_capacity(2);
    if next[0] != current[0] {
        mutations.push(VarMutation::Set(Identifier::from(x_name), Value::Num(next[0] as f32)));
    }
    if next[1] != current[1] {
        mutations.push(VarMutation::Set(Identifier::from(y_name), Value::Num(next[1] as f32)));
    }

    mutations
}

fn rounded_i32(value: f32) -> i32 {
    if value.is_finite() { value.round() as i32 } else { 0 }
}

fn themed_color(ui: &Ui, color: StyleColor, alpha: f32) -> [f32; 4] {
    let [red, green, blue, _] = ui.style_color(color);

    [red, green, blue, alpha]
}

fn draw_gizmo(ui: &Ui, origin: [f32; 2], map_view: GizmoMapView, hot: Option<Handle>, facing: Option<Dir>) {
    let center = themed_color(ui, StyleColor::Text, 1.0);
    let hot_color = themed_color(ui, StyleColor::ButtonHovered, 1.0);
    let shadow_color = themed_color(ui, StyleColor::BorderShadow, 0.85);
    let scale = dpi(ui);
    let axis_length = AXIS_LENGTH * scale;
    let arrow_length = ARROW_LENGTH * scale;
    let arrow_half_width = ARROW_HALF_WIDTH * scale;
    let center_half_size = CENTER_HALF_SIZE * scale;
    let draw_list = ui.get_window_draw_list();
    draw_list.with_clip_rect(map_view.min, map_view.max, || {
        if let Some(direction) = facing {
            let color = themed_color(ui, StyleColor::Text, 0.9);
            let center = [origin[0], origin[1] + FACING_OFFSET * scale];
            draw_facing_arrow(&draw_list, center, direction, color, shadow_color, scale);
        }

        let x_base = [origin[0] + axis_length - arrow_length, origin[1]];
        let x_tip = [origin[0] + axis_length, origin[1]];
        let y_base = [origin[0], origin[1] - axis_length + arrow_length];
        let y_tip = [origin[0], origin[1] - axis_length];
        let x_color = if hot == Some(Handle::X) { hot_color } else { X_COLOR };
        let y_color = if hot == Some(Handle::Y) { hot_color } else { Y_COLOR };
        let center_color = if hot == Some(Handle::XY) { hot_color } else { center };
        let x_thickness = scale
            * if hot == Some(Handle::X) {
                HOT_LINE_THICKNESS
            } else {
                LINE_THICKNESS
            };
        let y_thickness = scale
            * if hot == Some(Handle::Y) {
                HOT_LINE_THICKNESS
            } else {
                LINE_THICKNESS
            };

        draw_list
            .add_line(origin, x_base, shadow_color)
            .thickness(x_thickness + 2.0 * scale)
            .build();
        draw_list
            .add_line(origin, y_base, shadow_color)
            .thickness(y_thickness + 2.0 * scale)
            .build();
        let shadow_expansion = 1.5 * scale;
        draw_list
            .add_triangle(
                [x_tip[0] + shadow_expansion, x_tip[1]],
                [
                    x_base[0] - shadow_expansion,
                    x_base[1] - arrow_half_width - shadow_expansion,
                ],
                [
                    x_base[0] - shadow_expansion,
                    x_base[1] + arrow_half_width + shadow_expansion,
                ],
                shadow_color,
            )
            .filled(true)
            .build();
        draw_list
            .add_triangle(
                [y_tip[0], y_tip[1] - shadow_expansion],
                [
                    y_base[0] - arrow_half_width - shadow_expansion,
                    y_base[1] + shadow_expansion,
                ],
                [
                    y_base[0] + arrow_half_width + shadow_expansion,
                    y_base[1] + shadow_expansion,
                ],
                shadow_color,
            )
            .filled(true)
            .build();
        draw_list
            .add_line(origin, x_base, x_color)
            .thickness(x_thickness)
            .build();
        draw_list
            .add_line(origin, y_base, y_color)
            .thickness(y_thickness)
            .build();
        draw_list
            .add_triangle(
                x_tip,
                [x_base[0], x_base[1] - arrow_half_width],
                [x_base[0], x_base[1] + arrow_half_width],
                x_color,
            )
            .filled(true)
            .build();
        draw_list
            .add_triangle(
                y_tip,
                [y_base[0] - arrow_half_width, y_base[1]],
                [y_base[0] + arrow_half_width, y_base[1]],
                y_color,
            )
            .filled(true)
            .build();

        let center_min = [origin[0] - center_half_size, origin[1] - center_half_size];
        let center_max = [origin[0] + center_half_size, origin[1] + center_half_size];
        draw_list
            .add_rect(
                [center_min[0] - scale, center_min[1] - scale],
                [center_max[0] + scale, center_max[1] + scale],
                shadow_color,
            )
            .filled(true)
            .build();
        draw_list
            .add_rect(center_min, center_max, center_color)
            .filled(true)
            .build();
        draw_list.add_text([x_tip[0] + 3.0 * scale, x_tip[1] - 8.0 * scale], x_color, "X");
        draw_list.add_text([y_tip[0] - 4.0 * scale, y_tip[1] - 17.0 * scale], y_color, "Y");
    });
}

fn draw_facing_arrow(
    draw_list: &DrawListMut<'_>, center: [f32; 2], direction: Dir, color: [f32; 4], shadow_color: [f32; 4], scale: f32,
) {
    let vector = direction_vector(direction);
    let perpendicular = [-vector[1], vector[0]];
    let point = |radius: f32, side: f32| {
        [
            center[0] + (vector[0] * radius + perpendicular[0] * side) * scale,
            center[1] + (vector[1] * radius + perpendicular[1] * side) * scale,
        ]
    };
    let tip = FACING_LENGTH * 0.5;
    let base = tip - FACING_ARROW_LENGTH;

    for (color, expansion) in [(shadow_color, 1.5), (color, 0.0)] {
        draw_list
            .add_line(point(-tip - expansion, 0.0), point(base, 0.0), color)
            .thickness((FACING_LINE_THICKNESS + 2.0 * expansion) * scale)
            .build();
        draw_list
            .add_triangle(
                point(tip + expansion, 0.0),
                point(base - expansion, FACING_ARROW_HALF_WIDTH + expansion),
                point(base - expansion, -FACING_ARROW_HALF_WIDTH - expansion),
                color,
            )
            .filled(true)
            .build();
    }
}

fn direction_angle(direction: Dir) -> f32 {
    let vector = direction_vector(direction);

    vector[1].atan2(vector[0])
}

fn direction_sector(origin: [f32; 2], direction: Dir, slots: u8, scale: f32) -> Vec<[[f32; 2]; 4]> {
    let center = direction_angle(direction);
    let half_width = std::f32::consts::PI / slots as f32;
    let start = center - half_width + DIRECTION_SECTOR_GAP;
    let end = center + half_width - DIRECTION_SECTOR_GAP;
    let point = |angle: f32, radius: f32| {
        let radius = radius * scale;

        [origin[0] + angle.cos() * radius, origin[1] + angle.sin() * radius]
    };
    let mut quads = Vec::with_capacity(DIRECTION_SECTOR_SEGMENTS);

    for segment in 0..DIRECTION_SECTOR_SEGMENTS {
        let first = segment as f32 / DIRECTION_SECTOR_SEGMENTS as f32;
        let second = (segment + 1) as f32 / DIRECTION_SECTOR_SEGMENTS as f32;
        let first_angle = start + (end - start) * first;
        let second_angle = start + (end - start) * second;
        quads.push([
            point(first_angle, DIRECTION_INNER_RADIUS),
            point(first_angle, DIRECTION_OUTER_RADIUS),
            point(second_angle, DIRECTION_OUTER_RADIUS),
            point(second_angle, DIRECTION_INNER_RADIUS),
        ]);
    }

    quads
}

fn draw_direction_wheel(
    ui: &Ui, origin: [f32; 2], map_view: GizmoMapView, set: DirectionSet, current: Option<Dir>, hovered: Option<Dir>,
) {
    let direction_color = themed_color(ui, StyleColor::Text, 1.0);
    let wheel_color = themed_color(ui, StyleColor::WindowBg, 0.28);
    let sector_color = themed_color(ui, StyleColor::WindowBg, 0.42);
    let current_color = themed_color(ui, StyleColor::Tab, 0.56);
    let hot_direction_color = themed_color(ui, StyleColor::PlotHistogramHovered, 0.68);
    let shadow_color = themed_color(ui, StyleColor::BorderShadow, 0.85);
    let scale = dpi(ui);
    let draw_list = ui.get_window_draw_list();
    draw_list.with_clip_rect(map_view.min, map_view.max, || {
        for direction in set.slot_directions() {
            let color = if !set.contains(direction) {
                wheel_color
            } else if hovered == Some(direction) {
                hot_direction_color
            } else if current == Some(direction) {
                current_color
            } else {
                sector_color
            };

            for quad in direction_sector(origin, direction, set.slots, scale) {
                draw_list
                    .add_triangle(quad[0], quad[1], quad[2], color)
                    .filled(true)
                    .build();
                draw_list
                    .add_triangle(quad[0], quad[2], quad[3], color)
                    .filled(true)
                    .build();
            }
        }

        draw_list
            .add_circle(origin, DIRECTION_OUTER_RADIUS * scale, shadow_color)
            .thickness(1.5 * scale)
            .build();
        draw_list
            .add_circle(origin, DIRECTION_INNER_RADIUS * scale, wheel_color)
            .thickness(1.5 * scale)
            .build();

        for direction in set.directions() {
            let shadow = direction_head(origin, direction, 1.5, scale);
            draw_list
                .add_triangle(shadow[0], shadow[1], shadow[2], shadow_color)
                .filled(true)
                .build();

            let head = direction_head(origin, direction, 0.0, scale);
            draw_list
                .add_triangle(head[0], head[1], head[2], direction_color)
                .filled(true)
                .build();
        }
    });
}

#[cfg(test)]
mod tests;
