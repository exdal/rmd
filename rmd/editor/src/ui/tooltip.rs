use dear_imgui_rs::Ui;
use dmm::{Coord, Tile, merge::TileConflict};
use editor::{
    conflict::{OURS_COLOR, OURS_LABEL, THEIRS_COLOR, describe_tile},
    diff::{ADDED_COLOR, ChangeKind, LineKind, REMOVED_COLOR, TileLine, tile_lines},
    icons::materialdesignicons::ICON_ARROW_RIGHT,
};

use super::common::opaque;

/// Shown in place of a var value one side doesn't set
const UNSET: &str = "unset";

const MARKERS: [&str; 3] = ["+", "-", "~"];

/// What goes in front of a line, nothing for unchanged objects
const fn marker(kind: LineKind) -> &'static str {
    match kind {
        LineKind::Kept => "",
        LineKind::Added => MARKERS[0],
        LineKind::Removed => MARKERS[1],
        LineKind::Modified => MARKERS[2],
    }
}

/// Text in a line's diff color, or greyed out for unchanged objects
fn line_text(ui: &Ui, color: Option<[f32; 3]>, text: &str) {
    match color {
        Some(color) => ui.text_colored(opaque(color), text),
        None => ui.text_disabled(text),
    }
}

/// One line per object with `+`, `-` and `~` in a column of their own, the changed vars of
/// modified objects indented under them
fn draw_tile_lines(ui: &Ui, lines: &[TileLine]) {
    if lines.is_empty() {
        ui.text_disabled("(empty)");

        return;
    }

    let style = ui.clone_style();
    let spacing = style.item_spacing()[0];
    let marker_width = MARKERS
        .into_iter()
        .map(|marker| ui.calc_text_size(marker)[0])
        .fold(0.0, f32::max)
        + spacing;

    for line in lines {
        let start = ui.cursor_pos()[0];
        let color = line.kind.color();
        line_text(ui, color, marker(line.kind));
        ui.same_line_with_pos(start + marker_width);
        line_text(ui, color, &line.text);

        for var in &line.vars {
            ui.set_cursor_pos([start + marker_width + style.indent_spacing(), ui.cursor_pos()[1]]);
            ui.text_disabled(&var.name);
            ui.same_line();
            line_text(
                ui,
                var.before.as_ref().map(|_| REMOVED_COLOR),
                var.before.as_deref().unwrap_or(UNSET),
            );
            ui.same_line();
            ui.text_disabled(ICON_ARROW_RIGHT.to_string());
            ui.same_line();
            line_text(
                ui,
                var.after.as_ref().map(|_| ADDED_COLOR),
                var.after.as_deref().unwrap_or(UNSET),
            );
        }
    }
}

/// A side the diff reads from, noting when the tile is past the edge of that version
fn draw_missing_side(ui: &Ui, tile: Option<&Tile>, label: &str) {
    if tile.is_none() {
        ui.text_disabled(format!("(not in {label})"));
    }
}

pub(super) fn draw_conflict_tooltip(ui: &Ui, coord: Coord, conflict: &TileConflict) {
    ui.text(format!("Conflict at {}, {}, {}", coord.x, coord.y, coord.z));
    ui.separator();
    ui.text(format!("Base:\n{}", describe_tile(conflict.base.as_ref())));

    for (label, color, tile) in [
        (OURS_LABEL, OURS_COLOR, conflict.ours.as_ref()),
        ("Incoming", THEIRS_COLOR, conflict.theirs.as_ref()),
    ] {
        ui.separator();
        ui.text_colored(opaque(color), label);
        draw_missing_side(ui, tile, label);
        draw_tile_lines(ui, &tile_lines(conflict.base.as_ref(), tile));
    }
}

pub(super) fn draw_diff_tooltip(
    ui: &Ui, coord: Coord, kind: ChangeKind, (from_label, before): (&str, Option<&Tile>),
    (to_label, after): (&str, Option<&Tile>),
) {
    ui.text_colored(
        opaque(kind.color()),
        format!("{} at {}, {}, {}", kind.label(), coord.x, coord.y, coord.z),
    );
    ui.text_disabled(format!("{from_label} {ICON_ARROW_RIGHT} {to_label}"));
    ui.separator();
    draw_missing_side(ui, before, from_label);
    draw_missing_side(ui, after, to_label);
    draw_tile_lines(ui, &tile_lines(before, after));
}
