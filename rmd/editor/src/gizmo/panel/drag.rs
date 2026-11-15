use base::types::{Identifier, Value};

use dmm::Coord;

use super::state::{DragMember, DragState};
use crate::{document::VarMutation, gizmo::transform::anchor_to_tile, inspector::panel::TransformMode};

#[derive(Debug, Clone, Copy)]
pub(super) struct DraggedGroup {
    pub(super) delta: [i32; 2],
    pub(super) anchor: [f32; 2],
}

#[derive(Debug, Clone, Copy)]
pub(super) struct DraggedTransform {
    pub(super) values: [i32; 2],
    pub(super) anchor: [f32; 2],
}

pub(super) fn dragged_group(drag: &DragState, mouse: [f32; 2], snap: bool) -> DraggedGroup {
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

pub(super) fn dragged_member(drag: &DragState, member: &DragMember, group: DraggedGroup) -> DraggedTransform {
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

pub(super) fn anchored_member(
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

pub(super) fn transform_mutations(mode: TransformMode, current: [i32; 2], next: [i32; 2]) -> Vec<VarMutation> {
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
