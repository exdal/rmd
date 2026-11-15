use dear_imgui_rs::{MouseCursor, Ui};
use dmm::Coord;
use ui::widgets::dpi;

use super::{
    GizmoMapView, contains,
    state::{
        RESIZE_HANDLE_HALF_SIZE, RESIZE_HANDLE_HIT_HALF_SIZE, RESIZE_HANDLE_OFFSET, RegionDragAction, RegionDragState,
    },
};
use crate::{document::Selection, viewport::camera::Controller};

pub(super) fn dragged_region_selection(drag: RegionDragState, mouse: [f32; 2]) -> Selection {
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

pub(super) fn region_hit_at(
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

pub(super) fn resize_cursor(x: i8, y: i8) -> MouseCursor {
    match (x, y) {
        (0, _) => MouseCursor::ResizeNS,
        (_, 0) => MouseCursor::ResizeEW,
        _ if x == y => MouseCursor::ResizeNESW,
        _ => MouseCursor::ResizeNWSE,
    }
}

pub(super) fn resize_handle_positions(
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

pub(super) fn resize_handle_at(
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

pub(super) fn draw_resize_handles(
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
