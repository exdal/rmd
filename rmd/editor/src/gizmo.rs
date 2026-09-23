use core::types::{Identifier, Value};

use dear_imgui_rs::{MouseButton, MouseCursor, StyleColor, Ui};
use dmi::metadata::Dir;
use dmm::{Coord, Size};
use editor::{
    command::EditGroupId,
    document::{PrefabInstanceId, Selection, VarMutation},
    tool::SelectionRotation,
};

use crate::{
    camera::Controller,
    session::{DirectionState, DirectionalTypes, SelectedTransform, Session},
    settings::{KeybindAction, Settings},
    transform::anchor_to_tile,
    ui::TransformMode,
};

const AXIS_LENGTH: f32 = 48.0;
const ARROW_LENGTH: f32 = 11.0;
const ARROW_HALF_WIDTH: f32 = 6.0;
const CENTER_HALF_SIZE: f32 = 5.0;
const HIT_RADIUS: f32 = 7.0;
const LINE_THICKNESS: f32 = 3.0;
const HOT_LINE_THICKNESS: f32 = 5.0;
const DIRECTION_HOLD_SECONDS: f64 = 0.2;
const DIRECTION_TIME_EPSILON: f64 = 1.0e-9;
const DIRECTION_INNER_RADIUS: f32 = 22.0;
const DIRECTION_OUTER_RADIUS: f32 = 78.0;
const DIRECTION_TIP_RADIUS: f32 = 67.0;
const DIRECTION_ARROW_LENGTH: f32 = 14.0;
const DIRECTION_ARROW_HALF_WIDTH: f32 = 7.5;
const DIRECTION_SECTOR_GAP: f32 = 0.035;
const DIRECTION_SECTOR_SEGMENTS: usize = 8;

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

const X_COLOR: [f32; 4] = [0.92, 0.24, 0.22, 1.0];
const Y_COLOR: [f32; 4] = [0.28, 0.82, 0.35, 1.0];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Handle {
    X,
    Y,
    XY,
}

impl Handle {
    const fn moves_x(self) -> bool { matches!(self, Self::X | Self::XY) }

    const fn moves_y(self) -> bool { matches!(self, Self::Y | Self::XY) }

    const fn cursor(self) -> MouseCursor {
        match self {
            Self::X => MouseCursor::ResizeEW,
            Self::Y => MouseCursor::ResizeNS,
            Self::XY => MouseCursor::ResizeAll,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct DragState {
    selected: PrefabInstanceId,
    mode: TransformMode,
    handle: Handle,
    start_mouse: [f32; 2],
    start_values: [i32; 2],
    start_anchor: [f32; 2],
    start_coord: Coord,
    zoom: f32,
    tile_size: u32,
    group: EditGroupId,
}

#[derive(Debug, Clone, Copy)]
struct BlockDragState {
    start_selection: Selection,
    handle: Handle,
    action: BlockDragAction,
    start_mouse: [f32; 2],
    zoom: f32,
    tile_size: u32,
    map_size: Size,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockDragAction {
    Move,
    Resize { x: i8, y: i8 },
}

impl BlockDragAction {
    fn is_resize(self) -> bool { matches!(self, Self::Resize { .. }) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockGizmoKind {
    Selection,
    Placement,
    Clipboard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DirectionSet {
    supported: [bool; 8],
    slots: u8,
}

impl DirectionSet {
    fn contains(self, direction: Dir) -> bool { self.supported[clockwise_index(direction)] }

    fn slot_directions(self) -> impl Iterator<Item = Dir> {
        CLOCKWISE_DIRECTIONS
            .into_iter()
            .filter(move |direction| self.slots == 8 || !is_diagonal(*direction))
    }

    fn directions(self) -> impl Iterator<Item = Dir> {
        CLOCKWISE_DIRECTIONS
            .into_iter()
            .filter(move |direction| self.contains(*direction))
    }

    fn len(self) -> usize { self.supported.iter().filter(|supported| **supported).count() }
}

#[derive(Debug, Clone, Copy)]
struct DirectionGesture {
    target: DirectionGestureTarget,
    placement_coord: Option<Coord>,
    pressed_at: f64,
    set: DirectionSet,
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

#[derive(Debug, Clone, Copy)]
struct BlockDirectionGesture {
    pressed_at: f64,
    phase: BlockDirectionPhase,
}

#[derive(Debug, Clone, Copy)]
enum BlockDirectionPhase {
    Pending,
    Open {
        origin: [f32; 2],
        last_hovered: Option<Dir>,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct GizmoResponse {
    pub captures_mouse: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BlockGizmoResponse {
    pub captures_mouse: bool,
    pub selection: Selection,
    pub rotation: SelectionRotation,
    pub resizing: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BlockGizmoTarget {
    pub selection: Selection,
    pub rotation: SelectionRotation,
    pub map_size: Size,
    pub tile_size: u32,
    pub kind: BlockGizmoKind,
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
    block_drag: Option<BlockDragState>,
    block_direction: Option<BlockDirectionGesture>,
}

impl GizmoState {
    pub(crate) const fn is_interacting(&self) -> bool {
        self.drag.is_some() || self.direction.is_some() || self.block_drag.is_some() || self.block_direction.is_some()
    }

    pub(crate) fn block_rotation_open(&self) -> bool {
        matches!(
            self.block_direction.map(|gesture| gesture.phase),
            Some(BlockDirectionPhase::Open { .. })
        )
    }

    pub(crate) fn cancel(&mut self) {
        self.drag = None;
        self.direction = None;
        self.block_drag = None;
        self.block_direction = None;
    }

    pub(crate) fn draw(
        &mut self, ui: &Ui, session: &mut Session, settings: &Settings, camera: &Controller, mode: TransformMode,
        map_view: GizmoMapView,
    ) -> GizmoResponse {
        self.block_drag = None;
        self.block_direction = None;
        let Some(mut target) = session
            .selected_transform()
            .filter(|target| target.sprite.z == session.z())
        else {
            self.drag = None;
            self.direction = None;

            return GizmoResponse::default();
        };
        let values = transform_values(target, mode);
        if values.is_none() {
            self.drag = None;
        }

        if self
            .drag
            .is_some_and(|drag| drag.selected != target.selected || drag.mode != mode)
        {
            self.drag = None;
        }

        let mouse = ui.io().mouse_pos();
        let initial_origin = gizmo_origin(camera, map_view.min, target.sprite);
        let direction_origin = session
            .selected_location()
            .map(|location| tile_center_origin(camera, map_view.min, location.coord, session.options.tile_size));
        let was_dragging = self.drag.is_some();
        let was_direction_active = self.update_direction(
            ui,
            session,
            settings,
            DirectionUpdate {
                target: selected_direction_target(target),
                origin: direction_origin,
                placement_coord: None,
                map_view,
            },
        );
        if let Some(updated) = session.selected_transform() {
            target = updated;
        }

        let hovered_handle = values
            .filter(|_| map_view.hovered)
            .and_then(|_| handle_at(mouse, initial_origin, map_view.min, map_view.max));

        if self.direction.is_none()
            && self.drag.is_none()
            && let Some(handle) = hovered_handle
            && ui.is_mouse_clicked(MouseButton::Left)
            && let Some(location) = session.selected_location()
            && let Some(values) = values
        {
            self.drag = Some(DragState {
                selected: target.selected,
                mode,
                handle,
                start_mouse: mouse,
                start_values: values,
                start_anchor: [target.sprite.x, target.sprite.y],
                start_coord: location.coord,
                zoom: camera.camera.zoom.max(f32::EPSILON),
                tile_size: session.options.tile_size.max(1),
                group: EditGroupId::new(),
            });
        }

        let active_handle = self.drag.map(|drag| drag.handle);
        if let Some(drag) = self.drag {
            if !ui.is_mouse_down(MouseButton::Left) {
                self.drag = None;
            } else if mouse != drag.start_mouse
                && mouse.iter().all(|value| value.is_finite())
                && let Some(location) = session.selected_location()
            {
                let size = session
                    .map()
                    .map_or((1, 1), |map| (map.size.x.max(1), map.size.y.max(1)));
                let shift_snap = ui.io().key_shift();
                let ctrl_reanchor = ui.io().key_ctrl();
                let (coord, next) = anchored_values(drag, mouse, shift_snap, ctrl_reanchor, size);
                let current = transform_values(target, mode).unwrap_or(drag.start_values);
                let mutations = transform_mutations(mode, current, next);
                let label = format!("move {} offsets", mode.label().to_ascii_lowercase());

                if coord != location.coord {
                    session.move_selected_instance(coord, label, &mutations, Some(drag.group));
                } else if !mutations.is_empty() {
                    session.edit_selected_instance_vars(label, &mutations, Some(drag.group));
                }
                if let Some(updated) = session.selected_transform() {
                    target = updated;
                }
            }
        }

        let origin = gizmo_origin(camera, map_view.min, target.sprite);
        let hovered = if self.drag.is_some() {
            None
        } else if map_view.hovered && values.is_some() {
            handle_at(mouse, origin, map_view.min, map_view.max)
        } else {
            None
        };
        let hot = self.drag.map(|drag| drag.handle).or(hovered).or(active_handle);
        let open_wheel = self.direction.and_then(|gesture| match gesture.phase {
            DirectionPhase::Open {
                origin, last_hovered, ..
            } => Some((origin, gesture.set, last_hovered)),
            DirectionPhase::Pending => None,
        });

        if let Some((wheel_origin, set, hovered_direction)) = open_wheel {
            if hovered_direction.is_some() {
                ui.set_mouse_cursor(Some(MouseCursor::Hand));
            }
            draw_direction_wheel(
                ui,
                wheel_origin,
                map_view.min,
                map_view.max,
                set,
                current_direction(selected_direction_target(target).state, set),
                hovered_direction,
            );
        } else if values.is_some() {
            if let Some(handle) = hot {
                ui.set_mouse_cursor(Some(handle.cursor()));
            }
            draw_gizmo(ui, origin, map_view.min, map_view.max, hot);
        }

        GizmoResponse {
            captures_mouse: was_dragging
                || self.drag.is_some()
                || was_direction_active
                || self.direction.is_some()
                || hovered.is_some(),
        }
    }

    pub(crate) fn draw_block(
        &mut self, ui: &Ui, settings: &Settings, camera: &Controller, target: BlockGizmoTarget, map_view: GizmoMapView,
    ) -> BlockGizmoResponse {
        self.drag = None;
        self.direction = None;

        let BlockGizmoTarget {
            mut selection,
            mut rotation,
            map_size,
            tile_size,
            kind,
        } = target;

        let mouse = ui.io().mouse_pos();
        let initial_origin = block_center_origin(camera, map_view.min, selection, tile_size);
        let was_dragging = self.block_drag.is_some();
        let mut resizing = self.block_drag.is_some_and(|drag| drag.action.is_resize());
        let (was_direction_active, requested_rotation) =
            self.update_block_direction(ui, settings, initial_origin, rotation, map_view);
        if let Some(requested) = requested_rotation {
            rotation = requested;
        }

        let hovered_handle = map_view
            .hovered
            .then(|| handle_at(mouse, initial_origin, map_view.min, map_view.max))
            .flatten();
        let resize_handle = (map_view.hovered && kind == BlockGizmoKind::Selection)
            .then(|| resize_handle_at(camera, map_view, selection, tile_size, mouse))
            .flatten();
        let action = block_drag_action(kind, ui.io().key_shift(), hovered_handle, resize_handle);
        if self.block_direction.is_none()
            && self.block_drag.is_none()
            && let Some((handle, action)) = action
            && ui.is_mouse_clicked(MouseButton::Left)
        {
            resizing = action.is_resize();
            self.block_drag = Some(BlockDragState {
                start_selection: selection,
                handle,
                action,
                start_mouse: mouse,
                zoom: camera.camera.zoom.max(f32::EPSILON),
                tile_size: tile_size.max(1),
                map_size,
            });
        }

        if let Some(drag) = self.block_drag {
            if mouse.iter().all(|value| value.is_finite()) {
                selection = dragged_block_selection(drag, mouse);
            }
            if !ui.is_mouse_down(MouseButton::Left) {
                self.block_drag = None;
            }
        }

        let origin = block_center_origin(camera, map_view.min, selection, tile_size);
        let hovered = if self.block_drag.is_some() {
            None
        } else if map_view.hovered {
            handle_at(mouse, origin, map_view.min, map_view.max)
        } else {
            None
        };

        BlockGizmoResponse {
            captures_mouse: was_dragging
                || self.block_drag.is_some()
                || was_direction_active
                || self.block_direction.is_some()
                || resize_handle.is_some()
                || hovered.is_some(),
            selection,
            rotation,
            resizing,
        }
    }

    pub(crate) fn draw_block_overlay(
        &self, ui: &Ui, camera: &Controller, target: BlockGizmoTarget, map_view: GizmoMapView,
    ) {
        let origin = block_center_origin(camera, map_view.min, target.selection, target.tile_size);
        let hovered = if self.block_drag.is_some() {
            None
        } else if map_view.hovered {
            handle_at(ui.io().mouse_pos(), origin, map_view.min, map_view.max)
        } else {
            None
        };

        let hot = self.block_drag.map(|drag| drag.handle).or(hovered);
        let open_wheel = self.block_direction.and_then(|gesture| match gesture.phase {
            BlockDirectionPhase::Open {
                origin, last_hovered, ..
            } => Some((origin, last_hovered)),
            BlockDirectionPhase::Pending => None,
        });

        if let Some((wheel_origin, hovered_direction)) = open_wheel {
            if hovered_direction.is_some() {
                ui.set_mouse_cursor(Some(MouseCursor::Hand));
            }

            let set = cardinal_direction_set();
            draw_direction_wheel(
                ui,
                wheel_origin,
                map_view.min,
                map_view.max,
                set,
                Some(rotation_direction(target.rotation)),
                hovered_direction,
            );
        } else {
            draw_gizmo(ui, origin, map_view.min, map_view.max, hot);
            let resize_handle = if target.kind == BlockGizmoKind::Selection {
                draw_resize_handles(ui, camera, map_view, target.selection, target.tile_size)
            } else {
                None
            };
            if let Some(handle) = hot {
                ui.set_mouse_cursor(Some(handle.cursor()));
            }

            let action = self.block_drag.map(|drag| drag.action).or_else(|| {
                block_drag_action(target.kind, ui.io().key_shift(), hovered, resize_handle).map(|(_, action)| action)
            });
            if let Some(BlockDragAction::Resize { x, y }) = action {
                ui.set_mouse_cursor(Some(resize_cursor(x, y)));
            }
        }
    }

    fn update_block_direction(
        &mut self, ui: &Ui, settings: &Settings, origin: [f32; 2], rotation: SelectionRotation, map_view: GizmoMapView,
    ) -> (bool, Option<SelectionRotation>) {
        let rotation_key = settings.keybindings.get(KeybindAction::Rotate);
        if self.block_direction.is_none()
            && self.block_drag.is_none()
            && map_view.hovered
            && rotation_key.is_pressed(ui)
        {
            self.block_direction = Some(BlockDirectionGesture {
                pressed_at: ui.time(),
                phase: BlockDirectionPhase::Pending,
            });
        }

        let was_active = self.block_direction.is_some();
        let key_down = rotation_key.is_down(ui);
        let key_released = rotation_key.is_released(ui);
        let mut requested = None;
        if let Some(mut gesture) = self.block_direction {
            if matches!(gesture.phase, BlockDirectionPhase::Pending) {
                match pending_direction_outcome(gesture.pressed_at, ui.time(), key_down, key_released) {
                    PendingDirectionOutcome::Pending => {},
                    PendingDirectionOutcome::Open => {
                        gesture.phase = BlockDirectionPhase::Open {
                            origin,
                            last_hovered: None,
                        };
                    },
                    PendingDirectionOutcome::Tap => {
                        requested = Some(rotation.clockwise());
                        self.block_direction = None;
                    },
                    PendingDirectionOutcome::Cancel => self.block_direction = None,
                }
            }

            if let BlockDirectionPhase::Open {
                origin,
                mut last_hovered,
            } = gesture.phase
            {
                if key_released || !key_down {
                    self.block_direction = None;
                } else {
                    let hovered = direction_at(
                        ui.io().mouse_pos(),
                        origin,
                        map_view.min,
                        map_view.max,
                        cardinal_direction_set(),
                    );
                    if hovered != last_hovered {
                        last_hovered = hovered;
                        requested = hovered.map(direction_rotation);
                    }
                    gesture.phase = BlockDirectionPhase::Open { origin, last_hovered };
                    self.block_direction = Some(gesture);
                }
            } else if self.block_direction.is_some() {
                self.block_direction = Some(gesture);
            }
        }

        (was_active, requested)
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
        self.block_drag = None;
        self.block_direction = None;
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
            } => Some((origin, gesture.set, last_hovered)),
            DirectionPhase::Pending => None,
        });

        if let Some((wheel_origin, set, hovered_direction)) = open_wheel {
            if hovered_direction.is_some() {
                ui.set_mouse_cursor(Some(MouseCursor::Hand));
            }
            draw_direction_wheel(
                ui,
                wheel_origin,
                map_view.min,
                map_view.max,
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
        if self.direction.is_none()
            && self.drag.is_none()
            && map_view.hovered
            && rotation_key.is_pressed(ui)
            && origin.is_some()
            && let Some(set) = direction_set(target.state.dmi_directions, target.state.directional_types)
        {
            self.direction = Some(DirectionGesture {
                target: target.id,
                placement_coord,
                pressed_at: ui.time(),
                set,
                phase: DirectionPhase::Pending,
            });
        }
        let was_direction_active = self.direction.is_some();
        let key_down = rotation_key.is_down(ui);
        let key_released = rotation_key.is_released(ui);
        let mut requested_direction = None;

        if let Some(mut gesture) = self.direction {
            if matches!(gesture.phase, DirectionPhase::Pending) {
                match pending_direction_outcome(gesture.pressed_at, ui.time(), key_down, key_released) {
                    PendingDirectionOutcome::Pending => {},
                    PendingDirectionOutcome::Open => {
                        if let Some(origin) = origin {
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
                        requested_direction =
                            next_clockwise_direction(gesture.set, current_direction(target.state, gesture.set))
                                .map(|direction| (direction, None));
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
            {
                if key_released || !key_down {
                    self.direction = None;
                } else {
                    let hovered = direction_at(ui.io().mouse_pos(), origin, map_view.min, map_view.max, gesture.set);
                    if hovered != last_hovered {
                        last_hovered = hovered;
                        if let Some(direction) = hovered
                            && current_direction(target.state, gesture.set) != Some(direction)
                        {
                            requested_direction = Some((direction, Some(group)));
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

        if let Some((direction, group)) = requested_direction {
            apply_direction(session, target, direction, group);
        }

        was_direction_active
    }
}

fn transform_values(target: SelectedTransform, mode: TransformMode) -> Option<[i32; 2]> {
    match mode {
        TransformMode::Pixel => Some(target.pixel),
        TransformMode::Step if target.is_movable => Some(target.step),
        TransformMode::Step => None,
    }
}

fn gizmo_origin(camera: &Controller, viewport_min: [f32; 2], sprite: render::SpriteInstance) -> [f32; 2] {
    let center = camera.map_to_screen([sprite.x + sprite.width * 0.5, sprite.y + sprite.height * 0.5]);

    [viewport_min[0] + center[0], viewport_min[1] + center[1]]
}

fn tile_center_origin(camera: &Controller, viewport_min: [f32; 2], coord: Coord, tile_size: u32) -> [f32; 2] {
    let tile_size = tile_size.max(1) as f32;
    let center = camera.map_to_screen([(coord.x as f32 - 0.5) * tile_size, (coord.y as f32 - 0.5) * tile_size]);

    [viewport_min[0] + center[0], viewport_min[1] + center[1]]
}

fn block_center_origin(camera: &Controller, viewport_min: [f32; 2], selection: Selection, tile_size: u32) -> [f32; 2] {
    let tile_size = tile_size.max(1) as f32;
    let center = camera.map_to_screen([
        (selection.min.x.saturating_sub(1) as f32 + selection.width() as f32 * 0.5) * tile_size,
        (selection.min.y.saturating_sub(1) as f32 + selection.height() as f32 * 0.5) * tile_size,
    ]);

    [viewport_min[0] + center[0], viewport_min[1] + center[1]]
}

fn dragged_block_selection(drag: BlockDragState, mouse: [f32; 2]) -> Selection {
    let scale = drag.zoom * drag.tile_size as f32;

    let delta = [
        ((mouse[0] - drag.start_mouse[0]) / scale).round() as i64,
        (-(mouse[1] - drag.start_mouse[1]) / scale).round() as i64,
    ];

    if let BlockDragAction::Resize { x, y } = drag.action {
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

    let move_x = if drag.handle.moves_x() { delta[0] } else { 0 };
    let move_y = if drag.handle.moves_y() { delta[1] } else { 0 };

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
            .saturating_add(move_x)
            .clamp(1, i64::from(max_x)) as u32,
        i64::from(drag.start_selection.min.y)
            .saturating_add(move_y)
            .clamp(1, i64::from(max_y)) as u32,
        drag.start_selection.min.z,
    );

    drag.start_selection.with_min(min).unwrap_or(drag.start_selection)
}

fn block_drag_action(
    kind: BlockGizmoKind, shift: bool, gizmo: Option<Handle>, edge: Option<(i8, i8)>,
) -> Option<(Handle, BlockDragAction)> {
    if let Some((x, y)) = edge {
        return Some((Handle::XY, BlockDragAction::Resize { x, y }));
    }
    let handle = gizmo?;
    let action = match (kind, shift) {
        (BlockGizmoKind::Selection, true) => BlockDragAction::Resize {
            x: i8::from(handle.moves_x()),
            y: i8::from(handle.moves_y()),
        },
        (BlockGizmoKind::Placement, true) => return None,
        _ => BlockDragAction::Move,
    };
    Some((handle, action))
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
    camera: &Controller, viewport_min: [f32; 2], selection: Selection, tile_size: u32,
) -> [(i8, i8, [f32; 2]); 8] {
    let size = tile_size.max(1) as f32;
    let lower = camera.map_to_screen([(selection.min.x - 1) as f32 * size, (selection.min.y - 1) as f32 * size]);
    let upper = camera.map_to_screen([selection.max.x as f32 * size, selection.max.y as f32 * size]);
    let left = viewport_min[0] + lower[0] - 7.0;
    let right = viewport_min[0] + upper[0] + 7.0;
    let bottom = viewport_min[1] + lower[1] + 7.0;
    let top = viewport_min[1] + upper[1] - 7.0;
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
    camera: &Controller, view: GizmoMapView, selection: Selection, tile_size: u32, mouse: [f32; 2],
) -> Option<(i8, i8)> {
    if !contains(mouse, view.min, view.max) {
        return None;
    }
    resize_handle_positions(camera, view.min, selection, tile_size)
        .into_iter()
        .find(|(_, _, position)| {
            contains(
                mouse,
                [position[0] - 5.0, position[1] - 5.0],
                [position[0] + 5.0, position[1] + 5.0],
            )
        })
        .map(|(x, y, _)| (x, y))
}

fn draw_resize_handles(
    ui: &Ui, camera: &Controller, view: GizmoMapView, selection: Selection, tile_size: u32,
) -> Option<(i8, i8)> {
    let hovered = view
        .hovered
        .then(|| resize_handle_at(camera, view, selection, tile_size, ui.io().mouse_pos()))
        .flatten();
    let draw = ui.get_window_draw_list();
    draw.with_clip_rect(view.min, view.max, || {
        for (x, y, position) in resize_handle_positions(camera, view.min, selection, tile_size) {
            let min = [position[0] - 3.5, position[1] - 3.5];
            let max = [position[0] + 3.5, position[1] + 3.5];
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

fn handle_at(point: [f32; 2], origin: [f32; 2], viewport_min: [f32; 2], viewport_max: [f32; 2]) -> Option<Handle> {
    if !contains(point, viewport_min, viewport_max) {
        return None;
    }

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

fn contains(point: [f32; 2], min: [f32; 2], max: [f32; 2]) -> bool {
    point.iter().all(|value| value.is_finite())
        && point[0] >= min[0]
        && point[0] <= max[0]
        && point[1] >= min[1]
        && point[1] <= max[1]
}

fn direction_set(dmi_directions: Option<u32>, directional_types: Option<DirectionalTypes>) -> Option<DirectionSet> {
    let mut set = DirectionSet {
        supported: [false; 8],
        slots: 4,
    };

    if let Some(directional_types) = directional_types {
        for (direction, supported) in Dir::ORDER.into_iter().zip(directional_types.supported) {
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

    set.slots = if set.directions().any(is_diagonal) { 8 } else { 4 };

    (set.len() > 1).then_some(set)
}

fn cardinal_direction_set() -> DirectionSet {
    let mut supported = [false; 8];
    for direction in [Dir::North, Dir::East, Dir::South, Dir::West] {
        supported[clockwise_index(direction)] = true;
    }

    DirectionSet { supported, slots: 4 }
}

const fn rotation_direction(rotation: SelectionRotation) -> Dir {
    match rotation {
        SelectionRotation::Original => Dir::North,
        SelectionRotation::Clockwise => Dir::East,
        SelectionRotation::Half => Dir::South,
        SelectionRotation::CounterClockwise => Dir::West,
    }
}

const fn direction_rotation(direction: Dir) -> SelectionRotation {
    match direction {
        Dir::North | Dir::Northeast | Dir::Northwest => SelectionRotation::Original,
        Dir::East | Dir::Southeast => SelectionRotation::Clockwise,
        Dir::South | Dir::Southwest => SelectionRotation::Half,
        Dir::West => SelectionRotation::CounterClockwise,
    }
}

fn selected_direction_target(target: SelectedTransform) -> DirectionTarget {
    DirectionTarget {
        id: DirectionGestureTarget::Selection(target.selected),
        state: DirectionState {
            dir: target.dir,
            dmi_directions: target.dmi_directions,
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
        DirectionGestureTarget::Selection(_) if target.state.directional_types.is_some() => {
            session.set_selected_directional_type(direction, group);
        },
        DirectionGestureTarget::Selection(_) => {
            session.edit_selected_instance_vars("set dir", &[direction_mutation(direction)], group);
        },
        DirectionGestureTarget::Placement => {
            session.set_placement_direction(direction);
        },
    }
}

fn direction_mutation(direction: Dir) -> VarMutation {
    VarMutation::Set(Identifier::from("dir"), Value::Num(direction.to_bits() as f32))
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

fn direction_head(origin: [f32; 2], direction: Dir, expansion: f32) -> [[f32; 2]; 3] {
    let vector = direction_vector(direction);
    let perpendicular = [-vector[1], vector[0]];
    let tip_radius = DIRECTION_TIP_RADIUS + expansion;
    let base_radius = DIRECTION_TIP_RADIUS - DIRECTION_ARROW_LENGTH - expansion;
    let half_width = DIRECTION_ARROW_HALF_WIDTH + expansion;
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
    point: [f32; 2], origin: [f32; 2], viewport_min: [f32; 2], viewport_max: [f32; 2], set: DirectionSet,
) -> Option<Dir> {
    if !contains(point, viewport_min, viewport_max) {
        return None;
    }

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
struct DraggedTransform {
    values: [i32; 2],
    anchor: [f32; 2],
}

fn dragged_values(drag: DragState, mouse: [f32; 2], snap: bool) -> DraggedTransform {
    let map_delta = [
        (mouse[0] - drag.start_mouse[0]) / drag.zoom,
        -(mouse[1] - drag.start_mouse[1]) / drag.zoom,
    ];
    let mut next = drag.start_values;
    let mut anchor = drag.start_anchor;

    for axis in 0..2 {
        let enabled = if axis == 0 {
            drag.handle.moves_x()
        } else {
            drag.handle.moves_y()
        };
        if !enabled {
            continue;
        }

        let desired = drag.start_anchor[axis] + map_delta[axis];
        anchor[axis] = if snap {
            let tile = drag.tile_size as f32;

            (desired / tile).round() * tile
        } else {
            desired
        };
        next[axis] = drag.start_values[axis].saturating_add(rounded_i32(anchor[axis] - drag.start_anchor[axis]));
    }

    DraggedTransform { values: next, anchor }
}

fn anchored_values(
    drag: DragState, mouse: [f32; 2], snap: bool, reanchor: bool, size: (u32, u32),
) -> (Coord, [i32; 2]) {
    let dragged = dragged_values(drag, mouse, snap);
    if !reanchor {
        return (drag.start_coord, dragged.values);
    }

    let anchored = anchor_to_tile(dragged.anchor, drag.start_coord, drag.tile_size, size);
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

fn rounded_i32(value: f32) -> i32 { if value.is_finite() { value.round() as i32 } else { 0 } }

fn themed_color(ui: &Ui, color: StyleColor, alpha: f32) -> [f32; 4] {
    let [red, green, blue, _] = ui.style_color(color);

    [red, green, blue, alpha]
}

fn draw_gizmo(ui: &Ui, origin: [f32; 2], viewport_min: [f32; 2], viewport_max: [f32; 2], hot: Option<Handle>) {
    let center = themed_color(ui, StyleColor::Text, 1.0);
    let hot_color = themed_color(ui, StyleColor::ButtonHovered, 1.0);
    let shadow_color = themed_color(ui, StyleColor::BorderShadow, 0.85);
    let draw_list = ui.get_window_draw_list();
    draw_list.with_clip_rect(viewport_min, viewport_max, || {
        let x_base = [origin[0] + AXIS_LENGTH - ARROW_LENGTH, origin[1]];
        let x_tip = [origin[0] + AXIS_LENGTH, origin[1]];
        let y_base = [origin[0], origin[1] - AXIS_LENGTH + ARROW_LENGTH];
        let y_tip = [origin[0], origin[1] - AXIS_LENGTH];
        let x_color = if hot == Some(Handle::X) { hot_color } else { X_COLOR };
        let y_color = if hot == Some(Handle::Y) { hot_color } else { Y_COLOR };
        let center_color = if hot == Some(Handle::XY) { hot_color } else { center };
        let x_thickness = if hot == Some(Handle::X) {
            HOT_LINE_THICKNESS
        } else {
            LINE_THICKNESS
        };
        let y_thickness = if hot == Some(Handle::Y) {
            HOT_LINE_THICKNESS
        } else {
            LINE_THICKNESS
        };

        draw_list
            .add_line(origin, x_base, shadow_color)
            .thickness(x_thickness + 2.0)
            .build();
        draw_list
            .add_line(origin, y_base, shadow_color)
            .thickness(y_thickness + 2.0)
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
                [x_base[0], x_base[1] - ARROW_HALF_WIDTH],
                [x_base[0], x_base[1] + ARROW_HALF_WIDTH],
                x_color,
            )
            .filled(true)
            .build();
        draw_list
            .add_triangle(
                y_tip,
                [y_base[0] - ARROW_HALF_WIDTH, y_base[1]],
                [y_base[0] + ARROW_HALF_WIDTH, y_base[1]],
                y_color,
            )
            .filled(true)
            .build();

        let center_min = [origin[0] - CENTER_HALF_SIZE, origin[1] - CENTER_HALF_SIZE];
        let center_max = [origin[0] + CENTER_HALF_SIZE, origin[1] + CENTER_HALF_SIZE];
        draw_list
            .add_rect(
                [center_min[0] - 1.0, center_min[1] - 1.0],
                [center_max[0] + 1.0, center_max[1] + 1.0],
                shadow_color,
            )
            .filled(true)
            .build();
        draw_list
            .add_rect(center_min, center_max, center_color)
            .filled(true)
            .build();
        draw_list.add_text([x_tip[0] + 3.0, x_tip[1] - 8.0], x_color, "X");
        draw_list.add_text([y_tip[0] - 4.0, y_tip[1] - 17.0], y_color, "Y");
    });
}

fn direction_angle(direction: Dir) -> f32 {
    let vector = direction_vector(direction);

    vector[1].atan2(vector[0])
}

fn direction_sector(origin: [f32; 2], direction: Dir, slots: u8) -> Vec<[[f32; 2]; 4]> {
    let center = direction_angle(direction);
    let half_width = std::f32::consts::PI / slots as f32;
    let start = center - half_width + DIRECTION_SECTOR_GAP;
    let end = center + half_width - DIRECTION_SECTOR_GAP;
    let point = |angle: f32, radius: f32| [origin[0] + angle.cos() * radius, origin[1] + angle.sin() * radius];
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
    ui: &Ui, origin: [f32; 2], viewport_min: [f32; 2], viewport_max: [f32; 2], set: DirectionSet, current: Option<Dir>,
    hovered: Option<Dir>,
) {
    let direction_color = themed_color(ui, StyleColor::Text, 1.0);
    let wheel_color = themed_color(ui, StyleColor::WindowBg, 0.28);
    let sector_color = themed_color(ui, StyleColor::WindowBg, 0.42);
    let current_color = themed_color(ui, StyleColor::Tab, 0.56);
    let hot_direction_color = themed_color(ui, StyleColor::PlotHistogramHovered, 0.68);
    let shadow_color = themed_color(ui, StyleColor::BorderShadow, 0.85);
    let draw_list = ui.get_window_draw_list();
    draw_list.with_clip_rect(viewport_min, viewport_max, || {
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

            for quad in direction_sector(origin, direction, set.slots) {
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
            .add_circle(origin, DIRECTION_OUTER_RADIUS, shadow_color)
            .thickness(1.5)
            .build();
        draw_list
            .add_circle(origin, DIRECTION_INNER_RADIUS, wheel_color)
            .thickness(1.5)
            .build();

        for direction in set.directions() {
            let shadow = direction_head(origin, direction, 1.5);
            draw_list
                .add_triangle(shadow[0], shadow[1], shadow[2], shadow_color)
                .filled(true)
                .build();

            let head = direction_head(origin, direction, 0.0);
            draw_list
                .add_triangle(head[0], head[1], head[2], direction_color)
                .filled(true)
                .build();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drag(handle: Handle) -> DragState {
        DragState {
            selected: selected(),
            mode: TransformMode::Pixel,
            handle,
            start_mouse: [100.0, 100.0],
            start_values: [5, -3],
            start_anchor: [45.0, 29.0],
            start_coord: dmm::Coord::new(2, 1, 1),
            zoom: 2.0,
            tile_size: 32,
            group: EditGroupId::new(),
        }
    }

    fn block_drag(handle: Handle) -> BlockDragState {
        BlockDragState {
            start_selection: Selection::from_drag(Coord::new(3, 2, 1), Coord::new(5, 4, 1)),
            handle,
            action: BlockDragAction::Move,
            start_mouse: [100.0, 100.0],
            zoom: 2.0,
            tile_size: 32,
            map_size: Size { x: 6, y: 5, z: 1 },
        }
    }

    fn selected() -> PrefabInstanceId {
        let mut map = dmm::Map::new(dmm::Size { x: 1, y: 1, z: 1 });
        let key = map.intern_tile(vec![dmm::Prefab::new(core::path::TreePath::parse("/obj/test"))]);
        map.grid[0][0][0] = key;
        let document = editor::document::MapDocument::new(map, 1);

        document.instance_ids_at(dmm::Coord::new(1, 1, 1))[0]
    }

    #[test]
    fn center_handle_wins_where_the_handles_meet() {
        let viewport_min = [100.0, 100.0];
        let viewport_max = [200.0, 200.0];
        let origin = [150.0, 150.0];

        assert_eq!(handle_at(origin, origin, viewport_min, viewport_max), Some(Handle::XY));
        assert_eq!(
            handle_at([180.0, 150.0], origin, viewport_min, viewport_max),
            Some(Handle::X)
        );
        assert_eq!(
            handle_at([150.0, 120.0], origin, viewport_min, viewport_max),
            Some(Handle::Y)
        );
        assert_eq!(handle_at([205.0, 150.0], origin, viewport_min, viewport_max), None);
    }

    #[test]
    fn direction_wheel_origin_uses_the_tile_center_instead_of_sprite_offsets() {
        let mut camera = Controller::new();
        camera.resize(64, 64);
        camera.camera.x = 32.0;
        camera.camera.y = 32.0;

        assert_eq!(
            tile_center_origin(&camera, [100.0, 200.0], Coord::new(1, 1, 1), 32),
            [116.0, 248.0]
        );
        assert_eq!(
            tile_center_origin(&camera, [100.0, 200.0], Coord::new(2, 2, 1), 32),
            [148.0, 216.0]
        );
    }

    #[test]
    fn block_gizmo_origin_uses_the_center_of_the_whole_selection() {
        let mut camera = Controller::new();
        camera.resize(64, 64);
        camera.camera.x = 32.0;
        camera.camera.y = 32.0;
        let selection = Selection::from_drag(Coord::new(2, 1, 1), Coord::new(4, 2, 1));

        assert_eq!(
            block_center_origin(&camera, [100.0, 200.0], selection, 32),
            [180.0, 232.0]
        );
    }

    #[test]
    fn block_gizmo_moves_whole_tiles_along_its_active_axes_and_clamps_to_the_map() {
        assert_eq!(
            dragged_block_selection(block_drag(Handle::X), [164.0, 36.0]),
            Selection::from_drag(Coord::new(4, 2, 1), Coord::new(6, 4, 1))
        );
        assert_eq!(
            dragged_block_selection(block_drag(Handle::Y), [164.0, 36.0]),
            Selection::from_drag(Coord::new(3, 3, 1), Coord::new(5, 5, 1))
        );
        assert_eq!(
            dragged_block_selection(block_drag(Handle::XY), [-1_000.0, 1_000.0]),
            Selection::from_drag(Coord::new(1, 1, 1), Coord::new(3, 3, 1))
        );
    }

    #[test]
    fn every_resize_handle_keeps_the_opposite_edges_anchored() {
        for (x, y, min, max) in [
            (-1, -1, (4, 3), (5, 4)),
            (1, -1, (3, 3), (6, 4)),
            (-1, 1, (4, 2), (5, 5)),
            (1, 1, (3, 2), (6, 5)),
            (-1, 0, (4, 2), (5, 4)),
            (1, 0, (3, 2), (6, 4)),
            (0, -1, (3, 3), (5, 4)),
            (0, 1, (3, 2), (5, 5)),
        ] {
            let mut drag = block_drag(Handle::XY);
            drag.action = BlockDragAction::Resize { x, y };
            assert_eq!(
                dragged_block_selection(drag, [164.0, 36.0]),
                Selection::from_drag(Coord::new(min.0, min.1, 1), Coord::new(max.0, max.1, 1)),
                "{x}, {y}"
            );
        }
    }

    #[test]
    fn resize_clamps_at_one_tile_and_at_map_edges_without_flipping() {
        let mut drag = block_drag(Handle::XY);
        drag.action = BlockDragAction::Resize { x: 1, y: 1 };
        assert_eq!(
            dragged_block_selection(drag, [-10_000.0, 10_000.0]),
            Selection::from_drag(drag.start_selection.min, drag.start_selection.min)
        );
        assert_eq!(
            dragged_block_selection(drag, [f32::MAX, -f32::MAX]),
            Selection::from_drag(drag.start_selection.min, Coord::new(6, 5, 1))
        );
        drag.action = BlockDragAction::Resize { x: -1, y: -1 };
        assert_eq!(
            dragged_block_selection(drag, [10_000.0, -10_000.0]),
            Selection::from_drag(drag.start_selection.max, drag.start_selection.max)
        );
        assert_eq!(
            dragged_block_selection(drag, [-f32::MAX, f32::MAX]),
            Selection::from_drag(Coord::new(1, 1, 1), drag.start_selection.max)
        );
    }

    #[test]
    fn shift_gizmo_resizes_only_the_requested_axes_and_keeps_clipboard_movement() {
        for (handle, x, y) in [(Handle::X, 1, 0), (Handle::Y, 0, 1), (Handle::XY, 1, 1)] {
            assert_eq!(
                block_drag_action(BlockGizmoKind::Selection, true, Some(handle), None),
                Some((handle, BlockDragAction::Resize { x, y }))
            );
            assert_eq!(
                block_drag_action(BlockGizmoKind::Selection, false, Some(handle), None),
                Some((handle, BlockDragAction::Move))
            );
            assert_eq!(
                block_drag_action(BlockGizmoKind::Clipboard, true, Some(handle), None),
                Some((handle, BlockDragAction::Move))
            );
            assert_eq!(
                block_drag_action(BlockGizmoKind::Placement, true, Some(handle), None),
                None
            );
        }
    }

    #[test]
    fn resize_handles_remain_hittable_at_different_zooms_without_covering_the_center() {
        let selection = Selection::from_drag(Coord::new(3, 3, 1), Coord::new(3, 3, 1));
        let mut camera = Controller::new();
        camera.resize(800, 600);
        camera.center_on_tile(selection.min, 32);
        let view = GizmoMapView {
            min: [0.0; 2],
            max: [800.0, 600.0],
            hovered: true,
        };
        for zoom in [0.1, 1.0, 3.0] {
            camera.camera.zoom = zoom;
            for (x, y, position) in resize_handle_positions(&camera, view.min, selection, 32) {
                assert_eq!(resize_handle_at(&camera, view, selection, 32, position), Some((x, y)));
            }
            let center = block_center_origin(&camera, view.min, selection, 32);
            assert_eq!(resize_handle_at(&camera, view, selection, 32, center), None);
        }
    }

    #[test]
    fn block_rotation_wheel_exposes_the_four_cardinal_orientations() {
        assert_eq!(
            cardinal_direction_set().directions().collect::<Vec<_>>(),
            [Dir::North, Dir::East, Dir::South, Dir::West]
        );
        assert_eq!(direction_rotation(Dir::North), SelectionRotation::Original);
        assert_eq!(direction_rotation(Dir::East), SelectionRotation::Clockwise);
        assert_eq!(direction_rotation(Dir::South), SelectionRotation::Half);
        assert_eq!(direction_rotation(Dir::West), SelectionRotation::CounterClockwise);
    }

    #[test]
    fn direction_picker_only_uses_supported_dmi_directions() {
        assert_eq!(direction_set(None, None), None);
        assert_eq!(direction_set(Some(1), None), None);
        assert_eq!(direction_set(Some(2), None), None);

        let four = direction_set(Some(4), None).unwrap();
        assert_eq!(four.slots, 4);
        assert_eq!(
            four.directions().collect::<Vec<_>>(),
            [Dir::North, Dir::East, Dir::South, Dir::West]
        );

        let eight = direction_set(Some(8), None).unwrap();
        assert_eq!(eight.slots, 8);
        assert_eq!(eight.directions().collect::<Vec<_>>(), CLOCKWISE_DIRECTIONS);
    }

    #[test]
    fn directional_types_take_priority_over_dmi_directions() {
        let mut supported = [false; 8];
        supported[1] = true;
        supported[2] = true;
        supported[3] = true;
        let types = DirectionalTypes {
            supported,
            current: Some(Dir::North),
        };

        let set = direction_set(Some(8), Some(types)).unwrap();
        assert_eq!(set.directions().collect::<Vec<_>>(), [Dir::North, Dir::East, Dir::West]);
        assert_eq!(set.slots, 4);

        let one_type = DirectionalTypes {
            supported: [true, false, false, false, false, false, false, false],
            current: None,
        };
        assert_eq!(direction_set(Some(8), Some(one_type)), None);
    }

    #[test]
    fn direction_sectors_cover_the_wheel_but_not_its_dead_zone_or_exterior() {
        let origin = [150.0, 150.0];
        let viewport_min = [0.0, 0.0];
        let viewport_max = [300.0, 300.0];
        let set = direction_set(Some(8), None).unwrap();

        for direction in CLOCKWISE_DIRECTIONS {
            let vector = direction_vector(direction);
            let point = [origin[0] + vector[0] * 70.0, origin[1] + vector[1] * 70.0];

            assert_eq!(
                direction_at(point, origin, viewport_min, viewport_max, set),
                Some(direction)
            );
            assert_eq!(handle_at(point, origin, viewport_min, viewport_max), None);
        }

        assert_eq!(direction_at(origin, origin, viewport_min, viewport_max, set), None);
        assert_eq!(
            direction_at([origin[0] + 100.0, origin[1]], origin, viewport_min, viewport_max, set),
            None
        );
        assert_eq!(
            direction_at([301.0, 150.0], origin, viewport_min, viewport_max, set),
            None
        );
    }

    #[test]
    fn missing_direction_slots_are_not_selectable() {
        let mut supported = [false; 8];
        supported[1] = true;
        supported[3] = true;
        let set = direction_set(
            None,
            Some(DirectionalTypes {
                supported,
                current: None,
            }),
        )
        .unwrap();
        let origin = [100.0, 100.0];

        assert_eq!(
            direction_at([100.0, 40.0], origin, [0.0, 0.0], [200.0, 200.0], set),
            Some(Dir::North)
        );
        assert_eq!(
            direction_at([160.0, 100.0], origin, [0.0, 0.0], [200.0, 200.0], set),
            None
        );
    }

    #[test]
    fn tap_rotates_clockwise_and_skips_unsupported_directions() {
        let four = direction_set(Some(4), None).unwrap();
        assert_eq!(next_clockwise_direction(four, Some(Dir::North)), Some(Dir::East));
        assert_eq!(next_clockwise_direction(four, Some(Dir::East)), Some(Dir::South));
        assert_eq!(next_clockwise_direction(four, Some(Dir::South)), Some(Dir::West));
        assert_eq!(next_clockwise_direction(four, Some(Dir::West)), Some(Dir::North));

        let mut supported = [false; 8];
        supported[1] = true;
        supported[2] = true;
        supported[7] = true;
        let sparse = direction_set(
            None,
            Some(DirectionalTypes {
                supported,
                current: None,
            }),
        )
        .unwrap();
        assert_eq!(next_clockwise_direction(sparse, Some(Dir::North)), Some(Dir::East));
        assert_eq!(next_clockwise_direction(sparse, Some(Dir::East)), Some(Dir::Northwest));
        assert_eq!(next_clockwise_direction(sparse, None), Some(Dir::Northwest));
    }

    #[test]
    fn r_release_is_a_tap_only_before_the_hold_threshold() {
        assert_eq!(
            pending_direction_outcome(10.0, 10.199, false, true),
            PendingDirectionOutcome::Tap
        );
        assert_eq!(
            pending_direction_outcome(10.0, 10.2, true, false),
            PendingDirectionOutcome::Open
        );
        assert_eq!(
            pending_direction_outcome(10.0, 10.2, false, true),
            PendingDirectionOutcome::Cancel
        );
    }

    #[test]
    fn direction_mutations_use_byond_direction_bits() {
        for direction in Dir::ORDER {
            assert_eq!(
                direction_mutation(direction),
                VarMutation::Set(Identifier::from("dir"), Value::Num(direction.to_bits() as f32))
            );
        }
    }

    #[test]
    fn drag_converts_screen_motion_to_integer_map_pixels() {
        let both = dragged_values(drag(Handle::XY), [107.0, 95.0], false);
        let x = dragged_values(drag(Handle::X), [107.0, 95.0], false);
        let y = dragged_values(drag(Handle::Y), [107.0, 95.0], false);

        assert_eq!(both.values, [9, 0]);
        assert_eq!(x.values, [9, -3]);
        assert_eq!(y.values, [5, 0]);
        // The anchor tracks the free (unsnapged) rendered position, which the
        // caller re-anchors onto the nearest tile cell.
        assert_eq!(both.anchor, [48.5, 31.5]);
        assert_eq!(y.anchor, [45.0, 31.5]);
    }

    #[test]
    fn shift_snaps_the_visual_anchor_instead_of_the_active_value() {
        let mut state = drag(Handle::XY);
        state.zoom = 1.0;

        // The active value starts at 5, while other offsets put the rendered
        // anchor at 45. Snapping its dragged position to 64 adds 19, yielding 24.
        let snapped = dragged_values(state, [110.0, 90.0], true);

        assert_eq!(snapped.values, [24, 0]);
        assert_eq!(snapped.anchor, [64.0, 32.0]);
    }

    #[test]
    fn shift_snapping_respects_axis_constraints() {
        let mut state = drag(Handle::X);
        state.zoom = 1.0;

        assert_eq!(dragged_values(state, [110.0, 90.0], true).values, [24, -3]);
    }

    #[test]
    fn reanchoring_is_disabled_without_ctrl() {
        let mut state = drag(Handle::X);
        state.start_coord = dmm::Coord::new(2, 1, 1);
        state.start_values = [15, 0];
        state.start_anchor = [47.0, 0.0];
        state.zoom = 1.0;

        let (coord, values) = anchored_values(state, [120.0, 100.0], false, false, (10, 10));

        assert_eq!(coord, state.start_coord);
        assert_eq!(values, [35, 0]);
    }

    #[test]
    fn ctrl_reanchoring_stays_relative_to_the_start_tile_during_a_continued_drag() {
        let mut state = drag(Handle::X);
        state.start_coord = dmm::Coord::new(2, 1, 1);
        state.start_values = [15, 0];
        state.start_anchor = [47.0, 0.0];
        state.zoom = 1.0;

        let (first_coord, first_values) = anchored_values(state, [120.0, 100.0], false, true, (10, 10));
        let (second_coord, second_values) = anchored_values(state, [125.0, 100.0], false, true, (10, 10));

        assert_eq!(first_coord, dmm::Coord::new(3, 1, 1));
        assert_eq!(second_coord, first_coord);
        assert_eq!(first_values, [3, 0]);
        assert_eq!(second_values, [8, 0]);

        assert_eq!((first_coord.x - 1) * 32 + first_values[0] as u32, 67);
        assert_eq!((second_coord.x - 1) * 32 + second_values[0] as u32, 72);
    }

    #[test]
    fn mutations_target_only_changed_axes_in_the_active_mode() {
        assert_eq!(
            transform_mutations(TransformMode::Pixel, [1, 2], [3, 2]),
            [VarMutation::Set(Identifier::from("pixel_x"), Value::Num(3.0))]
        );
        assert_eq!(
            transform_mutations(TransformMode::Step, [1, 2], [1, 4]),
            [VarMutation::Set(Identifier::from("step_y"), Value::Num(4.0))]
        );
    }
}
