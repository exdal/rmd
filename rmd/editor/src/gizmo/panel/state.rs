use base::coord::Coord;
use dear_imgui_rs::MouseCursor;
use dmi::metadata::Dir;
use dmm::Size;
use prefab::PrefabInstanceId;

use super::{
    GizmoMapView,
    direction::{clockwise_index, is_diagonal},
};
use crate::{
    document::{command::EditGroupId, selection::Selection},
    gizmo::direction::DirectionState,
    inspector::panel::TransformMode,
    tool::SelectionRotation,
};

pub(super) const AXIS_LENGTH: f32 = 48.0;
pub(super) const ARROW_LENGTH: f32 = 11.0;
pub(super) const ARROW_HALF_WIDTH: f32 = 6.0;
pub(super) const CENTER_HALF_SIZE: f32 = 5.0;
pub(super) const HIT_RADIUS: f32 = 7.0;
pub(super) const LINE_THICKNESS: f32 = 3.0;
pub(super) const HOT_LINE_THICKNESS: f32 = 5.0;
pub(super) const RESIZE_HANDLE_OFFSET: f32 = 7.0;
pub(super) const RESIZE_HANDLE_HIT_HALF_SIZE: f32 = 5.0;
pub(super) const RESIZE_HANDLE_HALF_SIZE: f32 = 3.5;
pub(super) const DIRECTION_HOLD_SECONDS: f64 = 0.2;
pub(super) const DIRECTION_TIME_EPSILON: f64 = 1.0e-9;
pub(super) const DIRECTION_INNER_RADIUS: f32 = 22.0;
pub(super) const DIRECTION_OUTER_RADIUS: f32 = 78.0;
pub(super) const DIRECTION_TIP_RADIUS: f32 = 67.0;
pub(super) const DIRECTION_ARROW_LENGTH: f32 = 14.0;
pub(super) const DIRECTION_ARROW_HALF_WIDTH: f32 = 7.5;
pub(super) const DIRECTION_SECTOR_GAP: f32 = 0.035;
pub(super) const DIRECTION_SECTOR_SEGMENTS: usize = 8;
pub(super) const FACING_OFFSET: f32 = 26.0;
pub(super) const FACING_LENGTH: f32 = 20.0;
pub(super) const FACING_ARROW_LENGTH: f32 = 9.0;
pub(super) const FACING_ARROW_HALF_WIDTH: f32 = 6.0;
pub(super) const FACING_LINE_THICKNESS: f32 = 3.0;

pub(super) const CLOCKWISE_DIRECTIONS: [Dir; 8] = [
    Dir::North,
    Dir::Northeast,
    Dir::East,
    Dir::Southeast,
    Dir::South,
    Dir::Southwest,
    Dir::West,
    Dir::Northwest,
];
pub(super) const ALL_DIRECTION_SLOTS: u8 = CLOCKWISE_DIRECTIONS.len() as u8;
pub(super) const CARDINAL_DIRECTION_SLOTS: u8 = ALL_DIRECTION_SLOTS / 2;

pub(super) const X_COLOR: [f32; 4] = [0.92, 0.24, 0.22, 1.0];
pub(super) const Y_COLOR: [f32; 4] = [0.28, 0.82, 0.35, 1.0];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Handle {
    X,
    Y,
    XY,
}

impl Handle {
    pub(super) const fn moves_x(self) -> bool {
        matches!(self, Self::X | Self::XY)
    }

    pub(super) const fn moves_y(self) -> bool {
        matches!(self, Self::Y | Self::XY)
    }

    pub(super) const fn cursor(self) -> MouseCursor {
        match self {
            Self::X => MouseCursor::ResizeEW,
            Self::Y => MouseCursor::ResizeNS,
            Self::XY => MouseCursor::ResizeAll,
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct DragState {
    pub(super) members: Vec<DragMember>,
    pub(super) start_group_anchor: [f32; 2],
    pub(super) mode: TransformMode,
    pub(super) handle: Handle,
    pub(super) start_mouse: [f32; 2],
    pub(super) zoom: f32,
    pub(super) tile_size: u32,
    pub(super) group: EditGroupId,
    pub(super) is_rebuilding: bool,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct DragMember {
    pub(super) id: PrefabInstanceId,
    pub(super) start_values: [i32; 2],
    pub(super) start_anchor: [f32; 2],
    pub(super) start_coord: Coord,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct RegionDragState {
    pub(super) start_selection: Selection,
    pub(super) action: RegionDragAction,
    pub(super) start_mouse: [f32; 2],
    pub(super) zoom: f32,
    pub(super) tile_size: u32,
    pub(super) map_size: Size,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RegionDragAction {
    Move { copy: bool },
    Resize { x: i8, y: i8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DirectionSet {
    pub(super) supported: [bool; 8],
    pub(super) slots: u8,
}

impl DirectionSet {
    pub(super) fn contains(self, direction: Dir) -> bool {
        self.supported[clockwise_index(direction)]
    }

    pub(super) fn slot_directions(self) -> impl Iterator<Item = Dir> {
        CLOCKWISE_DIRECTIONS
            .into_iter()
            .filter(move |direction| self.slots == ALL_DIRECTION_SLOTS || !is_diagonal(*direction))
    }

    pub(super) fn directions(self) -> impl Iterator<Item = Dir> {
        CLOCKWISE_DIRECTIONS
            .into_iter()
            .filter(move |direction| self.contains(*direction))
    }

    pub(super) fn len(self) -> usize {
        self.supported.iter().filter(|supported| **supported).count()
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct DirectionGesture {
    pub(super) target: DirectionGestureTarget,
    pub(super) placement_coord: Option<Coord>,
    pub(super) pressed_at: f64,
    pub(super) action: DirectionAction,
    pub(super) set: Option<DirectionSet>,
    pub(super) phase: DirectionPhase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DirectionGestureTarget {
    Selection(PrefabInstanceId),
    Placement,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct DirectionTarget {
    pub(super) id: DirectionGestureTarget,
    pub(super) state: DirectionState,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct DirectionUpdate {
    pub(super) target: DirectionTarget,
    pub(super) origin: Option<[f32; 2]>,
    pub(super) placement_coord: Option<Coord>,
    pub(super) map_view: GizmoMapView,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum DirectionPhase {
    Pending,
    Open {
        origin: [f32; 2],
        group: EditGroupId,
        last_hovered: Option<Dir>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PendingDirectionOutcome {
    Pending,
    Open,
    Tap,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DirectionTap {
    RotateGroup,
    Face(Dir),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DirectionAction {
    Face,
    RotateGroup,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum DirectionRequest {
    RotateGroup {
        rotation: SelectionRotation,
        group: Option<EditGroupId>,
    },
    Face {
        direction: Dir,
        group: Option<EditGroupId>,
    },
}
