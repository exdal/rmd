use dear_imgui_rs::{DrawListMut, Ui};

use crate::{
    camera::Controller,
    session::{SelectedTransform, Session},
};

const TILE_GRID_COLOR: [f32; 4] = [1.0, 1.0, 1.0, 0.12];

const SELECTED_PIXEL_GRID_COLOR: [f32; 4] = [1.0, 1.0, 1.0, 0.2];

const GRID_VERTICAL_AXIS_COLOR: [f32; 4] = [0.2, 0.85, 0.3, 0.8];

const GRID_HORIZONTAL_AXIS_COLOR: [f32; 4] = [0.9, 0.25, 0.25, 0.8];

fn draw_map_grid_line(
    draw: &DrawListMut<'_>, camera: &Controller, viewport_min: [f32; 2], from: [f32; 2], to: [f32; 2], color: [f32; 4],
) {
    let from = camera.map_to_screen(from);
    let to = camera.map_to_screen(to);
    draw.add_line(
        [viewport_min[0] + from[0], viewport_min[1] + from[1]],
        [viewport_min[0] + to[0], viewport_min[1] + to[1]],
        color,
    )
    .thickness(1.0)
    .build();
}

pub(super) fn draw_tile_grid(
    ui: &Ui, session: &Session, camera: &Controller, min_pixels: u32, show_axis: bool, viewport_min: [f32; 2],
    viewport_max: [f32; 2],
) {
    let Some(size) = session.map().map(|map| map.size) else {
        return;
    };
    let tile_size = session.options.tile_size.max(1) as f32;
    let map_width = size.x as f32 * tile_size;
    let map_height = size.y as f32 * tile_size;

    if tile_size * camera.camera.zoom < min_pixels as f32 {
        return;
    }

    let local_max = [viewport_max[0] - viewport_min[0], viewport_max[1] - viewport_min[1]];
    let corner_a = camera.screen_to_map([0.0, 0.0]);
    let corner_b = camera.screen_to_map(local_max);
    let (min_x, max_x) = (corner_a[0].min(corner_b[0]), corner_a[0].max(corner_b[0]));
    let (min_y, max_y) = (corner_a[1].min(corner_b[1]), corner_a[1].max(corner_b[1]));

    let first_col = (min_x / tile_size).floor().max(0.0) as u32;
    let last_col = (max_x / tile_size).ceil().min(size.x as f32) as u32;
    let first_row = (min_y / tile_size).floor().max(0.0) as u32;
    let last_row = (max_y / tile_size).ceil().min(size.y as f32) as u32;
    let axis_x = ((size.x / 2) as f32 + 0.5) * tile_size;
    let axis_y = ((size.y / 2) as f32 + 0.5) * tile_size;

    let draw = ui.get_window_draw_list();
    draw.with_clip_rect(viewport_min, viewport_max, || {
        for col in first_col..=last_col {
            let x = col as f32 * tile_size;
            draw_map_grid_line(&draw, camera, viewport_min, [x, map_height], [x, 0.0], TILE_GRID_COLOR);
        }

        for row in first_row..=last_row {
            let y = row as f32 * tile_size;
            draw_map_grid_line(&draw, camera, viewport_min, [0.0, y], [map_width, y], TILE_GRID_COLOR);
        }

        if show_axis {
            draw_map_grid_line(
                &draw,
                camera,
                viewport_min,
                [axis_x, map_height],
                [axis_x, 0.0],
                GRID_VERTICAL_AXIS_COLOR,
            );
            draw_map_grid_line(
                &draw,
                camera,
                viewport_min,
                [0.0, axis_y],
                [map_width, axis_y],
                GRID_HORIZONTAL_AXIS_COLOR,
            );
        }
    });
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_selected_pixel_grid(
    ui: &Ui, camera: &Controller, transform: &SelectedTransform, tile_size: u32, min_pixels: u32, show_axis: bool,
    viewport_min: [f32; 2], viewport_max: [f32; 2],
) {
    let tile_size = tile_size.max(1) as f32;

    if camera.camera.zoom < min_pixels as f32 {
        return;
    }

    let center = [
        (transform.sprite.x / tile_size).round() * tile_size,
        (transform.sprite.y / tile_size).round() * tile_size,
    ];
    let origin = [center[0] - tile_size, center[1] - tile_size];
    let extent = tile_size * 3.0;
    let center_step = (extent / 2.0).round() as u32;
    let draw = ui.get_window_draw_list();
    draw.with_clip_rect(viewport_min, viewport_max, || {
        let steps = extent.round() as u32;
        for col in 0..=steps {
            let x = origin[0] + col as f32;
            let color = if show_axis && col == center_step {
                GRID_VERTICAL_AXIS_COLOR
            } else {
                SELECTED_PIXEL_GRID_COLOR
            };
            draw_map_grid_line(
                &draw,
                camera,
                viewport_min,
                [x, origin[1]],
                [x, origin[1] + extent],
                color,
            );
        }

        for row in 0..=steps {
            let y = origin[1] + row as f32;
            let color = if show_axis && row == center_step {
                GRID_HORIZONTAL_AXIS_COLOR
            } else {
                SELECTED_PIXEL_GRID_COLOR
            };
            draw_map_grid_line(
                &draw,
                camera,
                viewport_min,
                [origin[0], y],
                [origin[0] + extent, y],
                color,
            );
        }
    });
}
