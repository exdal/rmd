use dear_imgui_rs::{DragFlags, DrawListMut, Key, Ui};
use dmm::{Coord, Size};
use editor::{
    document::{DocumentId, Selection},
    icons::materialdesignicons::{ICON_CIRCLE_SMALL, ICON_MULTIPLICATION},
    tool::{BlockSelectionMode, SelectionMask, SelectionPlacement, SelectionRotation, Tool},
};

use super::{OVERLAY_BG, OVERLAY_PADDING, OverlayRect, PASTE_LABELS, draw_overlay_underlay};
use crate::{camera::Controller, session::Session};

const BLOCK_PLACEMENT_LABELS: [&str; 4] = ["Move", "Copy", "Fill selection", "Clear selection"];

const BLOCK_SELECTION_GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];

const BLOCK_SELECTION_WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

const BLOCK_SELECTION_SHADOW: [f32; 4] = [0.0, 0.0, 0.0, 0.85];

const BLOCK_STRIPE_LENGTH: f32 = 6.0;

const BLOCK_STRIPE_SPEED: f32 = 12.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct BlockSelectionOptions {
    pub(super) full_rectangle: bool,
    pub(super) line_width: i32,
}

impl Default for BlockSelectionOptions {
    fn default() -> Self {
        Self {
            full_rectangle: true,
            line_width: 1,
        }
    }
}

impl BlockSelectionOptions {
    pub(super) fn drawing_mode(self, shift: bool) -> BlockSelectionMode {
        if shift { self.hollow() } else { self.mode() }
    }

    pub(super) fn mode(self) -> BlockSelectionMode {
        if self.full_rectangle {
            BlockSelectionMode::Full
        } else {
            self.hollow()
        }
    }

    fn hollow(self) -> BlockSelectionMode {
        BlockSelectionMode::Hollow {
            line_width: self.line_width.max(1) as u32,
        }
    }

    pub(super) fn label(self) -> String {
        match self.mode() {
            BlockSelectionMode::Full => String::from("Full rectangle"),
            BlockSelectionMode::Hollow { line_width: 1 } => String::from("Border, 1 tile"),
            BlockSelectionMode::Hollow { line_width } => format!("Border, {line_width} tiles"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PendingBlockPlacement {
    pub(super) source: Selection,
    pub(super) target: Selection,
    pub(super) rotation: SelectionRotation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RectangleGesture {
    pub(super) start: Option<SelectionMask>,
    pub(super) z: u32,
}

pub(super) fn restore_rectangle_gesture(session: &mut Session, id: DocumentId, gesture: &mut Option<RectangleGesture>) {
    if let Some(gesture) = gesture.take()
        && let Some(document) = session.state.document_mut(id)
        && document.z == gesture.z
    {
        document.selection = gesture.start.map(|mask| mask.bounds);
        document.selection_mode = gesture.start.map_or(BlockSelectionMode::Full, |mask| mask.mode);
    }
}

pub(super) fn rectangle_drag_coord(
    camera: &Controller, mouse: [f32; 2], viewport_min: [f32; 2], size: Size, tile_size: u32, z: u32,
) -> Option<Coord> {
    let point = camera.screen_to_map([mouse[0] - viewport_min[0], mouse[1] - viewport_min[1]]);
    if !point.iter().all(|value| value.is_finite()) || size.x == 0 || size.y == 0 {
        return None;
    }
    let tile_size = tile_size.max(1) as f32;
    Some(Coord::new(
        ((point[0] / tile_size).floor() as i64)
            .saturating_add(1)
            .clamp(1, i64::from(size.x)) as u32,
        ((point[1] / tile_size).floor() as i64)
            .saturating_add(1)
            .clamp(1, i64::from(size.y)) as u32,
        z,
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BlockPlacementAction {
    Move,
    Copy,
    Fill,
    Cancel,
}

#[derive(Clone, Copy)]
pub(super) enum PlacementControls {
    Block,
    Paste,
}

impl PlacementControls {
    fn labels(self) -> &'static [&'static str] {
        match self {
            Self::Block => &BLOCK_PLACEMENT_LABELS,
            Self::Paste => &PASTE_LABELS,
        }
    }

    fn rows(self) -> usize {
        match self {
            Self::Block => 3,
            Self::Paste => 1,
        }
    }
}

pub(super) fn marching_stripe_offset(ui: &Ui) -> f32 {
    (ui.time() as f32 * BLOCK_STRIPE_SPEED).rem_euclid(BLOCK_STRIPE_LENGTH * 2.0)
}

fn draw_marching_border(draw: &DrawListMut<'_>, bounds: OverlayRect, clip: OverlayRect, offset: f32, accent: [f32; 4]) {
    for (start, end, on_accent) in block_border_segments(bounds, clip, offset) {
        draw.add_line(start, end, if on_accent { accent } else { BLOCK_SELECTION_WHITE })
            .thickness(2.0)
            .build();
    }
}

pub(super) fn draw_marching_edge(draw: &DrawListMut<'_>, from: [f32; 2], to: [f32; 2], offset: f32, accent: [f32; 4]) {
    let horizontal = (to[0] - from[0]).abs() >= (to[1] - from[1]).abs();
    let (start, end) = if horizontal { (from[0], to[0]) } else { (from[1], to[1]) };
    let span = end - start;
    if !span.is_finite() || span.abs() <= f32::EPSILON {
        return;
    }

    let (low, high) = (start.min(end), start.max(end));
    let mut stripe = ((low - offset) / BLOCK_STRIPE_LENGTH).floor();
    let mut cut = low;
    while cut < high {
        let next = (offset + (stripe + 1.0) * BLOCK_STRIPE_LENGTH).min(high);
        let next = if next > cut { next } else { high };
        if stripe.rem_euclid(2.0) != 0.0 {
            let point = |value: f32| {
                if horizontal { [value, from[1]] } else { [from[0], value] }
            };
            draw.add_line(point(cut), point(next), accent).thickness(2.0).build();
        }
        cut = next;
        stripe += 1.0;
    }
}

pub(super) fn block_selection_bounds(
    camera: &Controller, viewport_min: [f32; 2], selection: Selection, tile_size: u32,
) -> OverlayRect {
    let tile_size = tile_size.max(1) as f32;
    let left = selection.min.x.saturating_sub(1) as f32 * tile_size;
    let right = selection.max.x as f32 * tile_size;
    let bottom = selection.min.y.saturating_sub(1) as f32 * tile_size;
    let top = selection.max.y as f32 * tile_size;
    let top_left = camera.map_to_screen([left, top]);
    let bottom_right = camera.map_to_screen([right, bottom]);

    OverlayRect {
        min: [viewport_min[0] + top_left[0], viewport_min[1] + top_left[1]],
        max: [viewport_min[0] + bottom_right[0], viewport_min[1] + bottom_right[1]],
    }
}

pub(super) fn draw_block_outline(
    ui: &Ui, session: &Session, camera: &Controller, displayed: Selection, mode: BlockSelectionMode,
    viewport: OverlayRect,
) {
    let bounds = block_selection_bounds(camera, viewport.min, displayed, session.options.tile_size);
    let inner_bounds = hollow_selection_inner(displayed, mode)
        .map(|inner| block_selection_bounds(camera, viewport.min, inner, session.options.tile_size));
    let draw = ui.get_window_draw_list();
    draw.with_clip_rect(viewport.min, viewport.max, || {
        let tint = [0.25, 0.85, 0.5, 0.12];
        if let Some(inner) = inner_bounds {
            for region in [
                OverlayRect {
                    min: bounds.min,
                    max: [bounds.max[0], inner.min[1]],
                },
                OverlayRect {
                    min: [bounds.min[0], inner.max[1]],
                    max: bounds.max,
                },
                OverlayRect {
                    min: [bounds.min[0], inner.min[1]],
                    max: [inner.min[0], inner.max[1]],
                },
                OverlayRect {
                    min: [inner.max[0], inner.min[1]],
                    max: [bounds.max[0], inner.max[1]],
                },
            ] {
                draw.add_rect(region.min, region.max, tint).filled(true).build();
            }
        } else {
            draw.add_rect(bounds.min, bounds.max, tint).filled(true).build();
        }
        draw.add_rect(bounds.min, bounds.max, BLOCK_SELECTION_SHADOW)
            .thickness(4.0)
            .build();
        if let Some(inner) = inner_bounds {
            draw.add_rect(inner.min, inner.max, BLOCK_SELECTION_SHADOW)
                .thickness(4.0)
                .build();
        }
        let offset = marching_stripe_offset(ui);
        for border in std::iter::once(bounds).chain(inner_bounds) {
            draw_marching_border(&draw, border, viewport, offset, BLOCK_SELECTION_GREEN);
        }
        let mode_label = match mode {
            BlockSelectionMode::Full => String::from("Full"),
            BlockSelectionMode::Hollow { line_width } => format!("Border {line_width}"),
        };
        let label = format!(
            "{} {ICON_MULTIPLICATION} {} {ICON_CIRCLE_SMALL} {mode_label}",
            displayed.width(),
            displayed.height()
        );
        let text_size = ui.calc_text_size(&label);
        let label_min = [
            bounds.min[0].max(viewport.min[0] + 4.0),
            (bounds.min[1] - text_size[1] - 18.0).max(viewport.min[1] + 4.0),
        ];
        draw.add_rect(
            [label_min[0] - 3.0, label_min[1] - 2.0],
            [label_min[0] + text_size[0] + 3.0, label_min[1] + text_size[1] + 2.0],
            OVERLAY_BG,
        )
        .filled(true)
        .build();
        draw.add_text(label_min, [1.0; 4], label);
    });
}

fn hollow_selection_inner(selection: Selection, mode: BlockSelectionMode) -> Option<Selection> {
    let BlockSelectionMode::Hollow { line_width } = mode else {
        return None;
    };
    let line_width = line_width.max(1);
    let min = Coord::new(
        selection.min.x.checked_add(line_width)?,
        selection.min.y.checked_add(line_width)?,
        selection.min.z,
    );
    let max = Coord::new(
        selection.max.x.checked_sub(line_width)?,
        selection.max.y.checked_sub(line_width)?,
        selection.max.z,
    );

    (min.x <= max.x && min.y <= max.y).then_some(Selection { min, max })
}

fn block_border_segments(bounds: OverlayRect, clip: OverlayRect, offset: f32) -> Vec<([f32; 2], [f32; 2], bool)> {
    let width = (bounds.max[0] - bounds.min[0]).max(0.0);
    let height = (bounds.max[1] - bounds.min[1]).max(0.0);
    let perimeter = 2.0 * (width + height);
    if perimeter <= f32::EPSILON {
        return Vec::new();
    }

    let mut segments = Vec::new();
    for (path_start, start, end) in [
        (0.0, bounds.min, [bounds.max[0], bounds.min[1]]),
        (width, [bounds.max[0], bounds.min[1]], bounds.max),
        (width + height, bounds.max, [bounds.min[0], bounds.max[1]]),
        (width * 2.0 + height, [bounds.min[0], bounds.max[1]], bounds.min),
    ] {
        if let Some((visible_start, visible_end)) = visible_edge_interval(start, end, path_start, clip) {
            let mut stripe = ((visible_start - offset) / BLOCK_STRIPE_LENGTH).floor() as i64;
            let mut part_start = visible_start;
            while part_start < visible_end {
                let stripe_end = offset + (stripe + 1) as f32 * BLOCK_STRIPE_LENGTH;
                let part_end = if stripe_end.is_finite() && stripe_end > part_start {
                    stripe_end.min(visible_end)
                } else {
                    visible_end
                };
                let green = stripe.rem_euclid(2) != 0;
                segments.push((
                    block_border_point(bounds, part_start),
                    block_border_point(bounds, part_end),
                    green,
                ));
                part_start = part_end;
                stripe += 1;
            }
        }
    }

    segments
}

fn visible_edge_interval(start: [f32; 2], end: [f32; 2], path_start: f32, clip: OverlayRect) -> Option<(f32, f32)> {
    if start[1] == end[1] {
        if !(clip.min[1]..=clip.max[1]).contains(&start[1]) {
            return None;
        }
        let low = start[0].min(end[0]).max(clip.min[0]);
        let high = start[0].max(end[0]).min(clip.max[0]);
        if high <= low {
            return None;
        }
        let (local_start, local_end) = if end[0] >= start[0] {
            (low - start[0], high - start[0])
        } else {
            (start[0] - high, start[0] - low)
        };

        Some((path_start + local_start, path_start + local_end))
    } else {
        if !(clip.min[0]..=clip.max[0]).contains(&start[0]) {
            return None;
        }
        let low = start[1].min(end[1]).max(clip.min[1]);
        let high = start[1].max(end[1]).min(clip.max[1]);
        if high <= low {
            return None;
        }
        let (local_start, local_end) = if end[1] >= start[1] {
            (low - start[1], high - start[1])
        } else {
            (start[1] - high, start[1] - low)
        };

        Some((path_start + local_start, path_start + local_end))
    }
}

fn block_border_point(bounds: OverlayRect, distance: f32) -> [f32; 2] {
    let width = (bounds.max[0] - bounds.min[0]).max(0.0);
    let height = (bounds.max[1] - bounds.min[1]).max(0.0);
    if distance <= width {
        [bounds.min[0] + distance, bounds.min[1]]
    } else if distance <= width + height {
        [bounds.max[0], bounds.min[1] + distance - width]
    } else if distance <= width * 2.0 + height {
        [bounds.max[0] - (distance - width - height), bounds.max[1]]
    } else {
        [bounds.min[0], bounds.max[1] - (distance - width * 2.0 - height)]
    }
}

pub(super) fn block_controls_placement(
    tool: Tool, selecting: bool, rotation_open: bool, selection: Option<Selection>,
    pending: Option<PendingBlockPlacement>,
) -> Option<PendingBlockPlacement> {
    (matches!(tool, Tool::BlockSelect | Tool::Fill) && !selecting && !rotation_open)
        .then(|| {
            selection.map(|source| {
                pending.unwrap_or(PendingBlockPlacement {
                    source,
                    target: source,
                    rotation: SelectionRotation::Original,
                })
            })
        })
        .flatten()
}

pub(super) fn block_placement_controls_layout(
    ui: &Ui, camera: &Controller, controls: PlacementControls, target: Selection, tile_size: u32,
    viewport_min: [f32; 2], controls_bounds: OverlayRect,
) -> ([f32; 2], OverlayRect) {
    let labels = controls.labels();
    let rows = controls.rows();
    let style = ui.clone_style();
    let padding = style.frame_padding();
    let spacing = style.item_spacing()[0];
    let width = labels
        .iter()
        .map(|label| ui.calc_text_size(*label)[0] + padding[0] * 2.0)
        .sum::<f32>()
        + spacing * labels.len().saturating_sub(1) as f32;
    let width = if rows > 1 { width.max(280.0) } else { width };
    let height = ui.frame_height() * rows as f32 + style.item_spacing()[1] * rows.saturating_sub(1) as f32;
    let selection = block_selection_bounds(camera, viewport_min, target, tile_size);
    let center = [
        (selection.min[0] + selection.max[0]) * 0.5,
        (selection.min[1] + selection.max[1]) * 0.5,
    ];
    let min_x = controls_bounds.min[0] + OVERLAY_PADDING;
    let min_y = controls_bounds.min[1] + OVERLAY_PADDING;
    let max_x = (controls_bounds.max[0] - width - OVERLAY_PADDING).max(min_x);
    let max_y = (controls_bounds.max[1] - height - OVERLAY_PADDING).max(min_y);
    let position = [
        (center[0] - width * 0.5).clamp(min_x, max_x),
        (if rows > 1 {
            if selection.max[1] + 20.0 <= max_y {
                selection.max[1] + 20.0
            } else if selection.min[1] - height - 20.0 >= min_y {
                selection.min[1] - height - 20.0
            } else {
                max_y
            }
        } else {
            center[1] + 14.0
        })
        .clamp(min_y, max_y),
    ];
    let bounds = OverlayRect {
        min: [position[0] - OVERLAY_PADDING, position[1] - OVERLAY_PADDING],
        max: [
            position[0] + width + OVERLAY_PADDING,
            position[1] + height + OVERLAY_PADDING,
        ],
    };

    (position, bounds)
}

pub(super) fn draw_block_placement_controls(
    ui: &Ui, camera: &Controller, placement: PendingBlockPlacement, viewport_min: [f32; 2],
    controls_bounds: OverlayRect, session: &mut Session, options: &mut BlockSelectionOptions,
) -> Option<BlockPlacementAction> {
    let (position, bounds) = block_placement_controls_layout(
        ui,
        camera,
        PlacementControls::Block,
        placement.target,
        session.options.tile_size,
        viewport_min,
        controls_bounds,
    );
    draw_overlay_underlay(ui, bounds);
    ui.set_cursor_screen_pos(position);

    let mode = session.selection_mode();
    let mode_label = match mode {
        BlockSelectionMode::Full => String::from("Full"),
        BlockSelectionMode::Hollow { line_width } => format!("Border ({line_width})"),
    };
    ui.align_text_to_frame_padding();
    ui.text(format!(
        "{} {ICON_MULTIPLICATION} {} {ICON_CIRCLE_SMALL} {mode_label}",
        placement.target.width(),
        placement.target.height()
    ));
    let row_height = ui.frame_height() + ui.clone_style().item_spacing()[1];
    ui.set_cursor_screen_pos([position[0], position[1] + row_height]);
    let pending = placement.target != placement.source || placement.rotation != SelectionRotation::Original;
    ui.with_disabled_if(pending, || draw_selection_mode_controls(ui, session, options));
    let mode = session.selection_mode();
    let can_move = session.can_place_selected_block_with_mode(
        placement.target.min,
        placement.rotation,
        SelectionPlacement::Move,
        mode,
    );
    let can_fill = session.can_fill_selected_block(placement.target.min, placement.rotation, mode);

    let mut action = None;
    ui.set_cursor_screen_pos([position[0], position[1] + row_height * 2.0]);
    if session.tool() == Tool::BlockSelect {
        if ui.button(BLOCK_PLACEMENT_LABELS[0]) {
            action = Some(BlockPlacementAction::Move);
        }
        ui.same_line();
        if ui.button(BLOCK_PLACEMENT_LABELS[1]) {
            action = Some(BlockPlacementAction::Copy);
        }
        ui.same_line();
        if ui.with_disabled_if(!can_fill, || ui.button(BLOCK_PLACEMENT_LABELS[2])) {
            action = Some(BlockPlacementAction::Fill);
        }
        ui.set_item_tooltip(session.palette().map_or_else(
            || String::from("Choose a type to fill the selection"),
            |prefab| format!("Fill selected tiles with {}", prefab.path),
        ));
        ui.same_line();
    }
    if ui.button(if pending { "Cancel" } else { "Clear selection" }) {
        action = Some(BlockPlacementAction::Cancel);
    }

    if action.is_none()
        && ui.is_window_focused()
        && !ui.io().want_text_input()
        && !ui.is_popup_open_with_flags("", dear_imgui_rs::PopupQueryFlags::ANY_POPUP)
        && ui.is_key_pressed(Key::Enter)
        && can_move
    {
        action = Some(BlockPlacementAction::Move);
    }

    action
}

pub(super) fn draw_selection_mode_controls(ui: &Ui, session: &mut Session, options: &mut BlockSelectionOptions) {
    let current = session.selection_mask();
    let mode = current.map_or(options.mode(), |mask| mask.mode);
    let mut next = *options;
    next.full_rectangle = mode == BlockSelectionMode::Full;
    if let BlockSelectionMode::Hollow { line_width } = mode {
        next.line_width = line_width.min(i32::MAX as u32) as i32;
    }
    let mut changed = false;
    for (label, full) in [("Border", false), ("Full", true)] {
        if ui.radio_button(label, next.full_rectangle == full) {
            next.full_rectangle = full;
            changed = true;
        }
        ui.same_line();
    }
    if !next.full_rectangle {
        ui.set_next_item_width(100.0);
        changed |= ui
            .drag_int_config("##selection-border-width")
            .range(1, i32::MAX)
            .flags(DragFlags::ALWAYS_CLAMP)
            .build(ui, &mut next.line_width);
        ui.set_item_tooltip("Border width in tiles");
    } else {
        ui.text(" ");
    }
    if changed {
        if current.is_none_or(|mask| {
            session.try_select_block(SelectionMask {
                bounds: mask.bounds,
                mode: next.mode(),
            })
        }) {
            *options = next;
        } else {
            ui.tooltip_text("This selection would extend outside the focused area");
        }
    }
}

#[cfg(test)]
mod tests {
    use dmm::Coord;
    use editor::{
        document::Selection,
        tool::{BlockSelectionMode, SelectionRotation, Tool},
    };

    use super::{
        BlockSelectionOptions,
        PendingBlockPlacement,
        block_border_segments,
        block_controls_placement,
        block_selection_bounds,
        hollow_selection_inner,
    };
    use crate::{camera::Controller, ui::OverlayRect};

    #[test]
    fn block_selection_bounds_follow_whole_tile_edges() {
        let mut camera = Controller::new();
        camera.resize(64, 64);
        camera.camera.x = 32.0;
        camera.camera.y = 32.0;
        let selection = Selection::from_drag(Coord::new(1, 1, 1), Coord::new(2, 1, 1));

        assert_eq!(
            block_selection_bounds(&camera, [10.0, 20.0], selection, 32),
            OverlayRect {
                min: [10.0, 52.0],
                max: [74.0, 84.0],
            }
        );
    }

    #[test]
    fn block_selection_options_clamp_the_line_width_and_label_the_active_mode() {
        let mut options = BlockSelectionOptions::default();
        assert_eq!(options.mode(), BlockSelectionMode::Full);
        assert_eq!(options.label(), "Full rectangle");
        assert_eq!(options.drawing_mode(true), BlockSelectionMode::Hollow { line_width: 1 });
        assert_eq!(options.drawing_mode(false), options.mode());

        // the width stays configured while the full rectangle is selected, and is ignored
        options.line_width = 4;
        assert_eq!(options.mode(), BlockSelectionMode::Full);
        assert_eq!(options.drawing_mode(true), BlockSelectionMode::Hollow { line_width: 4 });

        options.full_rectangle = false;
        assert_eq!(options.mode(), BlockSelectionMode::Hollow { line_width: 4 });
        assert_eq!(options.label(), "Border, 4 tiles");

        options.line_width = 1;
        assert_eq!(options.mode(), BlockSelectionMode::Hollow { line_width: 1 });
        assert_eq!(options.label(), "Border, 1 tile");

        // the drag widget clamps to one, but a stale or hand-edited value must not wrap round the cast
        for line_width in [0, -1, i32::MIN] {
            options.line_width = line_width;
            assert_eq!(options.mode(), BlockSelectionMode::Hollow { line_width: 1 });
            assert_eq!(options.drawing_mode(true), BlockSelectionMode::Hollow { line_width: 1 });
        }
    }

    #[test]
    fn hollow_block_selection_exposes_the_inset_hole() {
        let selection = Selection::from_drag(Coord::new(2, 3, 1), Coord::new(6, 7, 1));

        assert_eq!(hollow_selection_inner(selection, BlockSelectionMode::Full), None);
        assert_eq!(
            hollow_selection_inner(selection, BlockSelectionMode::Hollow { line_width: 1 }),
            Some(Selection::from_drag(Coord::new(3, 4, 1), Coord::new(5, 6, 1)))
        );
        assert_eq!(
            hollow_selection_inner(selection, BlockSelectionMode::Hollow { line_width: 2 }),
            Some(Selection::from_drag(Coord::new(4, 5, 1), Coord::new(4, 5, 1)))
        );
        assert_eq!(
            hollow_selection_inner(selection, BlockSelectionMode::Hollow { line_width: 3 }),
            None
        );
    }

    #[test]
    fn block_controls_are_available_before_a_selection_is_moved() {
        let selection = Selection::from_drag(Coord::new(1, 1, 1), Coord::new(2, 2, 1));
        let expected = PendingBlockPlacement {
            source: selection,
            target: selection,
            rotation: SelectionRotation::Original,
        };

        assert_eq!(
            block_controls_placement(Tool::BlockSelect, false, false, Some(selection), None),
            Some(expected)
        );
        assert_eq!(
            block_controls_placement(Tool::BlockSelect, true, false, Some(selection), None),
            None
        );
        assert_eq!(
            block_controls_placement(Tool::BlockSelect, false, true, Some(selection), None),
            None
        );
    }

    #[test]
    fn block_border_stripes_stay_on_each_rectangle_edge() {
        let bounds = OverlayRect {
            min: [10.0, 20.0],
            max: [42.0, 68.0],
        };
        let segments = block_border_segments(bounds, bounds, 3.0);

        assert!(!segments.is_empty());
        assert!(segments.iter().any(|(_, _, green)| *green));
        assert!(segments.iter().any(|(_, _, green)| !*green));
        assert!(segments.iter().all(|(start, end, _)| {
            (start[0] == end[0] || start[1] == end[1])
                && [start, end].into_iter().flatten().all(|value| value.is_finite())
        }));
    }

    #[test]
    fn block_border_generation_is_limited_to_visible_edges() {
        let bounds = OverlayRect {
            min: [-1_000_000.0, 20.0],
            max: [1_000_000.0, 68.0],
        };
        let clip = OverlayRect {
            min: [0.0, 0.0],
            max: [100.0, 100.0],
        };
        let segments = block_border_segments(bounds, clip, 0.0);

        assert!(segments.len() < 40);
        assert!(segments.iter().all(|(start, end, _)| {
            [start, end]
                .into_iter()
                .flatten()
                .all(|value| (-f32::EPSILON..=100.0 + f32::EPSILON).contains(value))
        }));
    }
}
