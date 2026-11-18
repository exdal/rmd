use base::{
    coord::Coord,
    types::{Identifier, Value},
};
use dmi::metadata::Dir;
use prefab::{Prefab, PrefabInstanceId};

use crate::{
    document::instance::VarMutation,
    gizmo::direction::{DirectionState, DirectionalTypes},
    tool::SelectionRotation,
};

use super::*;
use super::{
    direction::{
        direction_at, direction_set, direction_tap, direction_vector, group_direction_set, hover_request,
        next_clockwise_direction, pending_direction_outcome, quarter_turns,
    },
    drag::{DraggedTransform, anchored_member, dragged_group, dragged_member, transform_mutations},
    region::{dragged_region_selection, region_hit_at, resize_handle_at, resize_handle_positions},
    state::{
        ALL_DIRECTION_SLOTS, CARDINAL_DIRECTION_SLOTS, CLOCKWISE_DIRECTIONS, DirectionAction, DirectionRequest,
        DirectionTap, DragMember, DragState, Handle, PendingDirectionOutcome, RegionDragAction, RegionDragState,
    },
};

fn drag(handle: Handle) -> DragState {
    DragState {
        members: vec![DragMember {
            id: picked(),
            start_values: [5, -3],
            start_anchor: [45.0, 29.0],
            start_coord: Coord::new(2, 1, 1),
        }],
        start_group_anchor: [45.0, 29.0],
        mode: TransformMode::Pixel,
        handle,
        start_mouse: [100.0, 100.0],
        zoom: 2.0,
        tile_size: 32,
        group: EditGroupId::new(),
        is_rebuilding: false,
    }
}

fn restart(state: &mut DragState, values: [i32; 2], anchor: [f32; 2], coord: Coord) {
    state.members[0].start_values = values;
    state.members[0].start_anchor = anchor;
    state.members[0].start_coord = coord;
    state.start_group_anchor = anchor;
}

fn dragged_values(state: &DragState, mouse: [f32; 2], snap: bool) -> DraggedTransform {
    dragged_member(state, &state.members[0], dragged_group(state, mouse, snap))
}

fn anchored_values(
    state: &DragState, mouse: [f32; 2], snap: bool, reanchor: bool, size: (u32, u32),
) -> (Coord, [i32; 2]) {
    let group = dragged_group(state, mouse, snap);

    anchored_member(state, &state.members[0], group, reanchor, size)
}

fn region_drag() -> RegionDragState {
    RegionDragState {
        start_selection: Selection::from_drag(Coord::new(3, 2, 1), Coord::new(5, 4, 1)),
        action: RegionDragAction::Move { copy: false },
        start_mouse: [100.0, 100.0],
        zoom: 2.0,
        tile_size: 32,
        map_size: Size { x: 6, y: 5, z: 1 },
    }
}

fn region_center_origin(camera: &Controller, viewport_min: [f32; 2], selection: Selection, tile_size: u32) -> [f32; 2] {
    let tile_size = tile_size.max(1) as f32;
    let center = camera.map_to_screen([
        (selection.min.x.saturating_sub(1) as f32 + selection.width() as f32 * 0.5) * tile_size,
        (selection.min.y.saturating_sub(1) as f32 + selection.height() as f32 * 0.5) * tile_size,
    ]);

    [viewport_min[0] + center[0], viewport_min[1] + center[1]]
}

fn picked() -> PrefabInstanceId {
    let mut map = dmm::Map::new(dmm::Size { x: 1, y: 1, z: 1 });
    let key = map.intern_tile(vec![Prefab::new(base::path::TreePath::parse("/obj/test"))]);
    map.grid[0][0][0] = key;
    let document = crate::document::MapDocument::new(map, 1);

    document.instance_ids_at(Coord::new(1, 1, 1))[0]
}

#[test]
fn center_handle_wins_where_the_handles_meet() {
    let viewport_min = [100.0, 100.0];
    let viewport_max = [200.0, 200.0];
    let origin = [150.0, 150.0];

    assert_eq!(
        handle_at(origin, origin, viewport_min, viewport_max, 1.0),
        Some(Handle::XY)
    );
    assert_eq!(
        handle_at([180.0, 150.0], origin, viewport_min, viewport_max, 1.0),
        Some(Handle::X)
    );
    assert_eq!(
        handle_at([150.0, 120.0], origin, viewport_min, viewport_max, 1.0),
        Some(Handle::Y)
    );
    assert_eq!(handle_at([205.0, 150.0], origin, viewport_min, viewport_max, 1.0), None);
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
fn region_drag_moves_whole_tiles_and_clamps_to_the_map() {
    assert_eq!(
        dragged_region_selection(region_drag(), [164.0, 36.0]),
        Selection::from_drag(Coord::new(4, 3, 1), Coord::new(6, 5, 1))
    );
    assert_eq!(
        dragged_region_selection(region_drag(), [-1_000.0, 1_000.0]),
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
        let mut drag = region_drag();
        drag.action = RegionDragAction::Resize { x, y };
        assert_eq!(
            dragged_region_selection(drag, [164.0, 36.0]),
            Selection::from_drag(Coord::new(min.0, min.1, 1), Coord::new(max.0, max.1, 1)),
            "{x}, {y}"
        );
    }
}

#[test]
fn resize_clamps_at_one_tile_and_at_map_edges_without_flipping() {
    let mut drag = region_drag();
    drag.action = RegionDragAction::Resize { x: 1, y: 1 };
    assert_eq!(
        dragged_region_selection(drag, [-10_000.0, 10_000.0]),
        Selection::from_drag(drag.start_selection.min, drag.start_selection.min)
    );
    assert_eq!(
        dragged_region_selection(drag, [f32::MAX, -f32::MAX]),
        Selection::from_drag(drag.start_selection.min, Coord::new(6, 5, 1))
    );
    drag.action = RegionDragAction::Resize { x: -1, y: -1 };
    assert_eq!(
        dragged_region_selection(drag, [10_000.0, -10_000.0]),
        Selection::from_drag(drag.start_selection.max, drag.start_selection.max)
    );
    assert_eq!(
        dragged_region_selection(drag, [-f32::MAX, f32::MAX]),
        Selection::from_drag(Coord::new(1, 1, 1), drag.start_selection.max)
    );
}

#[test]
fn the_selection_body_moves_and_its_edges_resize() {
    let selection = Selection::from_drag(Coord::new(3, 3, 1), Coord::new(5, 4, 1));
    let mut camera = Controller::new();
    camera.resize(800, 600);
    camera.center_on_tile(Coord::new(4, 3, 1), 32);
    let view = GizmoMapView {
        min: [0.0; 2],
        max: [800.0, 600.0],
        hovered: true,
    };
    let center = region_center_origin(&camera, view.min, selection, 32);
    let hit = |point, copy| region_hit_at(&camera, view, selection, 32, point, 1.0, copy);

    assert_eq!(hit(center, false), Some(RegionDragAction::Move { copy: false }));
    assert_eq!(hit(center, true), Some(RegionDragAction::Move { copy: true }));
    assert_eq!(hit([center[0] + 200.0, center[1]], false), None);
    for (x, y, position) in resize_handle_positions(&camera, view.min, selection, 32, 1.0) {
        assert_eq!(hit(position, false), Some(RegionDragAction::Resize { x, y }));
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
        for (x, y, position) in resize_handle_positions(&camera, view.min, selection, 32, 1.0) {
            assert_eq!(
                resize_handle_at(&camera, view, selection, 32, position, 1.0),
                Some((x, y))
            );
        }
        let center = region_center_origin(&camera, view.min, selection, 32);
        assert_eq!(resize_handle_at(&camera, view, selection, 32, center, 1.0), None);
    }
}

#[test]
fn direction_picker_only_uses_supported_dmi_directions() {
    assert_eq!(direction_set(None, None, None), None);
    assert_eq!(direction_set(Some(1), None, None), None);
    assert_eq!(direction_set(Some(2), None, None), None);

    let four = direction_set(Some(4), None, None).unwrap();
    assert_eq!(four.slots, CARDINAL_DIRECTION_SLOTS);
    assert_eq!(
        four.directions().collect::<Vec<_>>(),
        [Dir::North, Dir::East, Dir::South, Dir::West]
    );

    let eight = direction_set(Some(8), None, None).unwrap();
    assert_eq!(eight.slots, ALL_DIRECTION_SLOTS);
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

    let set = direction_set(Some(8), None, Some(types)).unwrap();
    assert_eq!(set.directions().collect::<Vec<_>>(), [Dir::North, Dir::East, Dir::West]);
    assert_eq!(set.slots, CARDINAL_DIRECTION_SLOTS);

    let one_type = DirectionalTypes {
        supported: [true, false, false, false, false, false, false, false],
        current: None,
    };
    assert_eq!(direction_set(Some(8), None, Some(one_type)), None);
}

#[test]
fn profile_declared_directions_rotate_a_single_direction_icon() {
    let mut cardinals = [false; 8];
    cardinals[..4].fill(true);

    let set = direction_set(Some(1), Some(cardinals), None).unwrap();
    assert_eq!(
        set.directions().collect::<Vec<_>>(),
        [Dir::North, Dir::East, Dir::South, Dir::West]
    );
    assert_eq!(set.slots, CARDINAL_DIRECTION_SLOTS);

    let types = DirectionalTypes {
        supported: [false, true, true, false, false, false, false, false],
        current: None,
    };
    let set = direction_set(Some(1), Some(cardinals), Some(types)).unwrap();
    assert_eq!(set.directions().collect::<Vec<_>>(), [Dir::North, Dir::East]);
}

#[test]
fn direction_sectors_cover_the_wheel_but_not_its_dead_zone_or_exterior() {
    let origin = [150.0, 150.0];
    let viewport_min = [0.0, 0.0];
    let viewport_max = [300.0, 300.0];
    let set = direction_set(Some(8), None, None).unwrap();

    for direction in CLOCKWISE_DIRECTIONS {
        let vector = direction_vector(direction);
        let point = [origin[0] + vector[0] * 70.0, origin[1] + vector[1] * 70.0];

        assert_eq!(
            direction_at(point, origin, viewport_min, viewport_max, set, 1.0),
            Some(direction)
        );
        assert_eq!(handle_at(point, origin, viewport_min, viewport_max, 1.0), None);
    }

    assert_eq!(direction_at(origin, origin, viewport_min, viewport_max, set, 1.0), None);
    assert_eq!(
        direction_at(
            [origin[0] + 100.0, origin[1]],
            origin,
            viewport_min,
            viewport_max,
            set,
            1.0
        ),
        None
    );
    assert_eq!(
        direction_at([301.0, 150.0], origin, viewport_min, viewport_max, set, 1.0),
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
        None,
        Some(DirectionalTypes {
            supported,
            current: None,
        }),
    )
    .unwrap();
    let origin = [100.0, 100.0];

    assert_eq!(
        direction_at([100.0, 40.0], origin, [0.0, 0.0], [200.0, 200.0], set, 1.0),
        Some(Dir::North)
    );
    assert_eq!(
        direction_at([160.0, 100.0], origin, [0.0, 0.0], [200.0, 200.0], set, 1.0),
        None
    );
}

#[test]
fn a_tap_rotates_the_group_even_without_a_direction_set() {
    let state = DirectionState {
        dir: Dir::North.to_bits(),
        dmi_directions: Some(4),
        declared: None,
        directional_types: None,
    };
    let set = direction_set(Some(4), None, None);

    assert_eq!(
        direction_tap(DirectionAction::RotateGroup, None, state),
        Some(DirectionTap::RotateGroup)
    );
    assert_eq!(
        direction_tap(DirectionAction::RotateGroup, set, state),
        Some(DirectionTap::RotateGroup)
    );
    assert_eq!(direction_tap(DirectionAction::Face, None, state), None);
    assert_eq!(
        direction_tap(DirectionAction::Face, set, state),
        Some(DirectionTap::Face(Dir::East))
    );
}

#[test]
fn quarter_turns_count_clockwise_steps_between_same_kind_directions() {
    assert_eq!(quarter_turns(Dir::North, Dir::East), Some(SelectionRotation::Clockwise));
    assert_eq!(quarter_turns(Dir::North, Dir::South), Some(SelectionRotation::Half));
    assert_eq!(
        quarter_turns(Dir::North, Dir::West),
        Some(SelectionRotation::CounterClockwise)
    );
    assert_eq!(quarter_turns(Dir::West, Dir::North), Some(SelectionRotation::Clockwise));
    assert_eq!(
        quarter_turns(Dir::Northeast, Dir::Southeast),
        Some(SelectionRotation::Clockwise)
    );
    assert_eq!(
        quarter_turns(Dir::Northeast, Dir::Northeast),
        Some(SelectionRotation::Original)
    );
    assert_eq!(quarter_turns(Dir::North, Dir::Northeast), None);
}

#[test]
fn the_group_wheel_offers_the_reference_and_its_quarter_turns() {
    let cardinal = group_direction_set(Dir::East);
    assert_eq!(cardinal.slots, CARDINAL_DIRECTION_SLOTS);
    assert_eq!(
        cardinal.directions().collect::<Vec<_>>(),
        [Dir::North, Dir::East, Dir::South, Dir::West]
    );

    let diagonal = group_direction_set(Dir::Southwest);
    assert_eq!(diagonal.slots, ALL_DIRECTION_SLOTS);
    assert_eq!(
        diagonal.directions().collect::<Vec<_>>(),
        [Dir::Northeast, Dir::Southeast, Dir::Southwest, Dir::Northwest]
    );
}

#[test]
fn a_group_hover_turns_from_the_current_facing_to_the_hovered_slot() {
    let state = DirectionState {
        dir: Dir::East.to_bits(),
        dmi_directions: None,
        declared: None,
        directional_types: None,
    };
    let set = group_direction_set(Dir::North);
    let group = EditGroupId::new();

    assert!(matches!(
        hover_request(DirectionAction::RotateGroup, state, set, Dir::West, group),
        Some(DirectionRequest::RotateGroup {
            rotation: SelectionRotation::Half,
            ..
        })
    ));
    assert!(hover_request(DirectionAction::RotateGroup, state, set, Dir::East, group).is_none());
}

#[test]
fn tap_rotates_clockwise_and_skips_unsupported_directions() {
    let four = direction_set(Some(4), None, None).unwrap();
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
fn drag_converts_screen_motion_to_integer_map_pixels() {
    let both = dragged_values(&drag(Handle::XY), [107.0, 95.0], false);
    let x = dragged_values(&drag(Handle::X), [107.0, 95.0], false);
    let y = dragged_values(&drag(Handle::Y), [107.0, 95.0], false);

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
    let snapped = dragged_values(&state, [110.0, 90.0], true);

    assert_eq!(snapped.values, [24, 0]);
    assert_eq!(snapped.anchor, [64.0, 32.0]);
}

#[test]
fn shift_snapping_respects_axis_constraints() {
    let mut state = drag(Handle::X);
    state.zoom = 1.0;

    assert_eq!(dragged_values(&state, [110.0, 90.0], true).values, [24, -3]);
}

#[test]
fn reanchoring_is_disabled_without_ctrl() {
    let mut state = drag(Handle::X);
    restart(&mut state, [15, 0], [47.0, 0.0], Coord::new(2, 1, 1));
    state.zoom = 1.0;

    let (coord, values) = anchored_values(&state, [120.0, 100.0], false, false, (10, 10));

    assert_eq!(coord, state.members[0].start_coord);
    assert_eq!(values, [35, 0]);
}

#[test]
fn ctrl_reanchoring_stays_relative_to_the_start_tile_during_a_continued_drag() {
    let mut state = drag(Handle::X);
    restart(&mut state, [15, 0], [47.0, 0.0], Coord::new(2, 1, 1));
    state.zoom = 1.0;

    let (first_coord, first_values) = anchored_values(&state, [120.0, 100.0], false, true, (10, 10));
    let (second_coord, second_values) = anchored_values(&state, [125.0, 100.0], false, true, (10, 10));

    assert_eq!(first_coord, Coord::new(3, 1, 1));
    assert_eq!(second_coord, first_coord);
    assert_eq!(first_values, [3, 0]);
    assert_eq!(second_values, [8, 0]);

    assert_eq!((first_coord.x - 1) * 32 + first_values[0] as u32, 67);
    assert_eq!((second_coord.x - 1) * 32 + second_values[0] as u32, 72);
}

fn two_member_drag(handle: Handle) -> DragState {
    let mut state = drag(handle);
    state.zoom = 1.0;
    state.start_group_anchor = [47.0, 0.0];
    state.members = vec![
        DragMember {
            id: picked(),
            start_values: [15, 0],
            start_anchor: [47.0, 0.0],
            start_coord: Coord::new(2, 1, 1),
        },
        DragMember {
            id: picked(),
            start_values: [0, 0],
            start_anchor: [128.0, 0.0],
            start_coord: Coord::new(5, 1, 1),
        },
    ];

    state
}

#[test]
fn every_member_receives_the_group_delta() {
    let state = two_member_drag(Handle::XY);
    let group = dragged_group(&state, [120.0, 90.0], false);

    assert_eq!(group.delta, [20, 10]);
    let moved = state
        .members
        .iter()
        .map(|member| dragged_member(&state, member, group).values)
        .collect::<Vec<_>>();
    assert_eq!(moved, [[35, 10], [20, 10]]);
}

#[test]
fn shift_snaps_the_group_anchor_and_members_keep_their_offsets() {
    let state = two_member_drag(Handle::XY);
    let group = dragged_group(&state, [120.0, 80.0], true);

    assert_eq!(group.anchor, [64.0, 32.0]);
    assert_eq!(group.delta, [17, 32]);
    let anchors = state
        .members
        .iter()
        .map(|member| dragged_member(&state, member, group).anchor)
        .collect::<Vec<_>>();
    assert_eq!(anchors, [[64.0, 32.0], [145.0, 32.0]]);
}

#[test]
fn ctrl_reanchors_each_member_against_its_own_start_tile() {
    let state = two_member_drag(Handle::X);
    let group = dragged_group(&state, [120.0, 100.0], false);
    let moved = state
        .members
        .iter()
        .map(|member| anchored_member(&state, member, group, true, (10, 10)))
        .collect::<Vec<_>>();

    assert_eq!(moved, [(Coord::new(3, 1, 1), [3, 0]), (Coord::new(6, 1, 1), [-12, 0])]);

    let unanchored = anchored_member(&state, &state.members[1], group, false, (10, 10));
    assert_eq!(unanchored, (Coord::new(5, 1, 1), [20, 0]));
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
