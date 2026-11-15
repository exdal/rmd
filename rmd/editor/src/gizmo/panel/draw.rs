use dear_imgui_rs::{DrawListMut, StyleColor, Ui};
use dmi::metadata::Dir;
use ui::widgets::dpi;

use super::{
    GizmoMapView,
    direction::{direction_head, direction_vector},
    state::{
        ARROW_HALF_WIDTH, ARROW_LENGTH, AXIS_LENGTH, CENTER_HALF_SIZE, DIRECTION_INNER_RADIUS, DIRECTION_OUTER_RADIUS,
        DIRECTION_SECTOR_GAP, DIRECTION_SECTOR_SEGMENTS, DirectionSet, FACING_ARROW_HALF_WIDTH, FACING_ARROW_LENGTH,
        FACING_LENGTH, FACING_LINE_THICKNESS, FACING_OFFSET, HOT_LINE_THICKNESS, Handle, LINE_THICKNESS, X_COLOR,
        Y_COLOR,
    },
};

fn themed_color(ui: &Ui, color: StyleColor, alpha: f32) -> [f32; 4] {
    let [red, green, blue, _] = ui.style_color(color);

    [red, green, blue, alpha]
}

pub(super) fn draw_gizmo(ui: &Ui, origin: [f32; 2], map_view: GizmoMapView, hot: Option<Handle>, facing: Option<Dir>) {
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

pub(super) fn draw_direction_wheel(
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
