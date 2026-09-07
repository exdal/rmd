use core::types::{Identifier, Value};

use dear_imgui_rs::{MouseButton, MouseCursor, Ui};
use editor::{
    command::EditGroupId,
    document::{PrefabInstanceId, VarMutation},
};

use crate::{
    camera::Controller,
    inspector::TransformMode,
    session::{SelectedTransform, Session},
};

const AXIS_LENGTH: f32 = 48.0;
const ARROW_LENGTH: f32 = 11.0;
const ARROW_HALF_WIDTH: f32 = 6.0;
const CENTER_HALF_SIZE: f32 = 5.0;
const HIT_RADIUS: f32 = 7.0;
const LINE_THICKNESS: f32 = 3.0;
const HOT_LINE_THICKNESS: f32 = 5.0;

const X_COLOR: [f32; 4] = [0.92, 0.24, 0.22, 1.0];
const Y_COLOR: [f32; 4] = [0.28, 0.82, 0.35, 1.0];
const CENTER_COLOR: [f32; 4] = [0.92, 0.92, 0.92, 1.0];
const HOT_COLOR: [f32; 4] = [1.0, 0.78, 0.16, 1.0];
const SHADOW_COLOR: [f32; 4] = [0.04, 0.04, 0.04, 0.85];

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
    zoom: f32,
    tile_size: u32,
    group: EditGroupId,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct GizmoResponse {
    pub captures_mouse: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct GizmoViewport {
    pub min: [f32; 2],
    pub max: [f32; 2],
    pub hovered: bool,
}

#[derive(Default)]
pub(crate) struct GizmoState {
    drag: Option<DragState>,
}

impl GizmoState {
    pub(crate) const fn is_dragging(&self) -> bool { self.drag.is_some() }

    pub(crate) fn draw(
        &mut self, ui: &Ui, session: &mut Session, camera: &Controller, mode: TransformMode, viewport: GizmoViewport,
    ) -> GizmoResponse {
        let Some(mut target) = session.selected_transform() else {
            self.drag = None;

            return GizmoResponse::default();
        };
        let Some(values) = transform_values(target, mode) else {
            self.drag = None;

            return GizmoResponse::default();
        };

        if self
            .drag
            .is_some_and(|drag| drag.selected != target.selected || drag.mode != mode)
        {
            self.drag = None;
        }

        let mouse = ui.io().mouse_pos();
        let initial_origin = gizmo_origin(camera, viewport.min, target.sprite);
        let hovered = viewport
            .hovered
            .then(|| handle_at(mouse, initial_origin, viewport.min, viewport.max))
            .flatten();
        let was_dragging = self.drag.is_some();

        if self.drag.is_none()
            && let Some(handle) = hovered
            && ui.is_mouse_clicked(MouseButton::Left)
        {
            self.drag = Some(DragState {
                selected: target.selected,
                mode,
                handle,
                start_mouse: mouse,
                start_values: values,
                start_anchor: [target.sprite.x, target.sprite.y],
                zoom: camera.camera.zoom.max(f32::EPSILON),
                tile_size: session.options.tile_size.max(1),
                group: EditGroupId::new(),
            });
        }

        let active_handle = self.drag.map(|drag| drag.handle);
        if let Some(drag) = self.drag {
            if !ui.is_mouse_down(MouseButton::Left) {
                self.drag = None;
            } else if mouse != drag.start_mouse && mouse.iter().all(|value| value.is_finite()) {
                let next = dragged_values(drag, mouse, ui.io().key_shift());
                let current = transform_values(target, mode).unwrap_or(drag.start_values);
                let mutations = transform_mutations(mode, current, next);

                if !mutations.is_empty() {
                    session.edit_selected_instance_vars(
                        format!("move {} offsets", mode.label().to_ascii_lowercase()),
                        &mutations,
                        Some(drag.group),
                    );
                    if let Some(updated) = session.selected_transform() {
                        target = updated;
                    }
                }
            }
        }

        let origin = gizmo_origin(camera, viewport.min, target.sprite);
        let hovered = if self.drag.is_some() {
            None
        } else if viewport.hovered {
            handle_at(mouse, origin, viewport.min, viewport.max)
        } else {
            None
        };
        let hot = self.drag.map(|drag| drag.handle).or(hovered).or(active_handle);
        if let Some(handle) = hot {
            ui.set_mouse_cursor(Some(handle.cursor()));
        }

        draw_gizmo(ui, origin, viewport.min, viewport.max, hot);

        GizmoResponse {
            captures_mouse: was_dragging || self.drag.is_some() || hovered.is_some(),
        }
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

fn dragged_values(drag: DragState, mouse: [f32; 2], snap: bool) -> [i32; 2] {
    let map_delta = [
        (mouse[0] - drag.start_mouse[0]) / drag.zoom,
        -(mouse[1] - drag.start_mouse[1]) / drag.zoom,
    ];
    let mut next = drag.start_values;

    for axis in 0..2 {
        let enabled = if axis == 0 {
            drag.handle.moves_x()
        } else {
            drag.handle.moves_y()
        };
        if !enabled {
            continue;
        }

        let delta = if snap {
            let tile = drag.tile_size as f32;
            let desired_anchor = drag.start_anchor[axis] + map_delta[axis];
            let snapped_anchor = (desired_anchor / tile).round() * tile;

            rounded_i32(snapped_anchor - drag.start_anchor[axis])
        } else {
            rounded_i32(map_delta[axis])
        };
        next[axis] = drag.start_values[axis].saturating_add(delta);
    }

    next
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

fn draw_gizmo(ui: &Ui, origin: [f32; 2], viewport_min: [f32; 2], viewport_max: [f32; 2], hot: Option<Handle>) {
    let draw_list = ui.get_window_draw_list();
    draw_list.with_clip_rect(viewport_min, viewport_max, || {
        let x_base = [origin[0] + AXIS_LENGTH - ARROW_LENGTH, origin[1]];
        let x_tip = [origin[0] + AXIS_LENGTH, origin[1]];
        let y_base = [origin[0], origin[1] - AXIS_LENGTH + ARROW_LENGTH];
        let y_tip = [origin[0], origin[1] - AXIS_LENGTH];
        let x_color = if hot == Some(Handle::X) { HOT_COLOR } else { X_COLOR };
        let y_color = if hot == Some(Handle::Y) { HOT_COLOR } else { Y_COLOR };
        let center_color = if hot == Some(Handle::XY) {
            HOT_COLOR
        } else {
            CENTER_COLOR
        };
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
            .add_line(origin, x_base, SHADOW_COLOR)
            .thickness(x_thickness + 2.0)
            .build();
        draw_list
            .add_line(origin, y_base, SHADOW_COLOR)
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
                SHADOW_COLOR,
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
            zoom: 2.0,
            tile_size: 32,
            group: EditGroupId::new(),
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

        assert_eq!(
            handle_at(origin, origin, viewport_min, viewport_max),
            Some(Handle::XY)
        );
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
    fn drag_converts_screen_motion_to_integer_map_pixels() {
        let both = dragged_values(drag(Handle::XY), [107.0, 95.0], false);
        let x = dragged_values(drag(Handle::X), [107.0, 95.0], false);
        let y = dragged_values(drag(Handle::Y), [107.0, 95.0], false);

        assert_eq!(both, [9, 0]);
        assert_eq!(x, [9, -3]);
        assert_eq!(y, [5, 0]);
    }

    #[test]
    fn shift_snaps_the_visual_anchor_instead_of_the_active_value() {
        let mut state = drag(Handle::XY);
        state.zoom = 1.0;

        // The active value starts at 5, while other offsets put the rendered
        // anchor at 45. Snapping its dragged position to 64 adds 19, yielding 24.
        assert_eq!(dragged_values(state, [110.0, 90.0], true), [24, 0]);
    }

    #[test]
    fn shift_snapping_respects_axis_constraints() {
        let mut state = drag(Handle::X);
        state.zoom = 1.0;

        assert_eq!(dragged_values(state, [110.0, 90.0], true), [24, -3]);
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
