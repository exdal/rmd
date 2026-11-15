use dear_imgui_rs::Ui;
use dmi::metadata::Dir;
use ui::widgets::dpi;

use super::{
    GizmoState, contains,
    state::{
        ALL_DIRECTION_SLOTS, CARDINAL_DIRECTION_SLOTS, CLOCKWISE_DIRECTIONS, DIRECTION_ARROW_HALF_WIDTH,
        DIRECTION_ARROW_LENGTH, DIRECTION_HOLD_SECONDS, DIRECTION_INNER_RADIUS, DIRECTION_OUTER_RADIUS,
        DIRECTION_TIME_EPSILON, DIRECTION_TIP_RADIUS, DirectionAction, DirectionGesture, DirectionGestureTarget,
        DirectionPhase, DirectionRequest, DirectionSet, DirectionTap, DirectionTarget, DirectionUpdate,
        PendingDirectionOutcome,
    },
    unscaled,
};
use crate::{
    document::command::EditGroupId,
    gizmo::direction::{DirectionState, DirectionalTypes},
    inspector::session::PickedTransform,
    session::Session,
    settings::{Settings, keybind::KeybindAction},
    tool::SelectionRotation,
};

impl GizmoState {
    pub(super) fn update_direction(
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

pub(super) fn direction_set(
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

pub(super) fn picked_direction_target(target: PickedTransform) -> DirectionTarget {
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

pub(super) fn clockwise_index(direction: Dir) -> usize {
    CLOCKWISE_DIRECTIONS
        .iter()
        .position(|candidate| *candidate == direction)
        .unwrap_or_default()
}

pub(super) const fn is_diagonal(direction: Dir) -> bool {
    matches!(
        direction,
        Dir::Northeast | Dir::Southeast | Dir::Southwest | Dir::Northwest
    )
}

pub(super) fn facing_direction(state: DirectionState) -> Option<Dir> {
    direction_set(state.dmi_directions, state.declared, state.directional_types)
        .and_then(|set| current_direction(state, set))
}

pub(super) fn current_direction(state: DirectionState, set: DirectionSet) -> Option<Dir> {
    state
        .directional_types
        .and_then(|types| types.current)
        .or_else(|| Dir::from_bits(state.dir))
        .filter(|direction| set.contains(*direction))
}

pub(super) fn next_clockwise_direction(set: DirectionSet, current: Option<Dir>) -> Option<Dir> {
    let (start, include_start) = current
        .map(|direction| (clockwise_index(direction), false))
        .unwrap_or((clockwise_index(Dir::South), true));

    (usize::from(!include_start)..=CLOCKWISE_DIRECTIONS.len())
        .map(|offset| CLOCKWISE_DIRECTIONS[(start + offset) % CLOCKWISE_DIRECTIONS.len()])
        .find(|direction| set.contains(*direction))
}

pub(super) fn direction_tap(
    action: DirectionAction, set: Option<DirectionSet>, state: DirectionState,
) -> Option<DirectionTap> {
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

pub(super) fn group_direction_set(reference: Dir) -> DirectionSet {
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

pub(super) fn quarter_turns(from: Dir, to: Dir) -> Option<SelectionRotation> {
    let steps = (clockwise_index(to) + CLOCKWISE_DIRECTIONS.len() - clockwise_index(from)) % CLOCKWISE_DIRECTIONS.len();

    match steps {
        0 => Some(SelectionRotation::Original),
        2 => Some(SelectionRotation::Clockwise),
        4 => Some(SelectionRotation::Half),
        6 => Some(SelectionRotation::CounterClockwise),
        _ => None,
    }
}

pub(super) fn hover_request(
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

pub(super) fn pending_direction_outcome(
    pressed_at: f64, now: f64, key_down: bool, key_released: bool,
) -> PendingDirectionOutcome {
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

pub(super) fn direction_vector(direction: Dir) -> [f32; 2] {
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

pub(super) fn direction_head(origin: [f32; 2], direction: Dir, expansion: f32, scale: f32) -> [[f32; 2]; 3] {
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

pub(super) fn direction_at(
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
