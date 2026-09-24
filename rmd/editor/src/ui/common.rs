use dear_imgui_rs::{StyleColor, TableFlags, Ui};

const MIN_STRETCH_WIDTH: f32 = 40.0;

/// Continues the line when the next item fits, wraps it onto a new line otherwise
pub(super) fn same_line_if_fits(ui: &Ui, width: f32) {
    ui.same_line();
    if ui.content_region_avail_width() < width {
        ui.new_line();
    }
}

/// Moves the next item against the right edge, on a new line when this one is full
pub(super) fn align_right(ui: &Ui, width: f32) {
    same_line_if_fits(ui, width);
    let available = ui.content_region_avail_width();
    if available > width {
        let [x, y] = ui.cursor_pos();
        ui.set_cursor_pos([x + available - width, y]);
    }
}

pub(super) fn label_width(ui: &Ui, label: &str) -> f32 { ui.calc_text_size_with_opts(label, true, -1.0)[0] }

pub(super) fn button_width(ui: &Ui, label: &str) -> f32 {
    label_width(ui, label) + ui.clone_style().frame_padding()[0] * 2.0
}

pub(super) fn checkbox_width(ui: &Ui, label: &str) -> f32 {
    ui.frame_height() + ui.clone_style().item_inner_spacing()[0] + label_width(ui, label)
}

pub(super) fn text_wrapped_colored(ui: &Ui, color: [f32; 4], text: &str) {
    let _color = ui.push_style_color(StyleColor::Text, color);
    ui.text_wrapped(text);
}

/// Disabled text centered below the cursor, wrapped when it doesn't fit
pub(super) fn centered_note(ui: &Ui, text: &str) {
    let width = ui.content_region_avail()[0];
    let size = ui.calc_text_size(text);
    let [x, y] = ui.cursor_pos();
    if width > size[0] {
        ui.set_cursor_pos([x + (width - size[0]) * 0.5, y + ui.text_line_height()]);
        ui.text_disabled(text);
    } else {
        ui.set_cursor_pos([x, y + ui.text_line_height()]);
        ui.text_wrapped(text);
    }
}

/// Lets a table scroll sideways once the panel is narrower than its columns need
pub(super) fn overflow_scroll(ui: &Ui, min_width: f32) -> (TableFlags, f32) {
    if ui.content_region_avail_width() < min_width {
        (TableFlags::SCROLL_X, min_width)
    } else {
        (TableFlags::NONE, 0.0)
    }
}

/// Width a table needs for its fixed columns plus a floor for each stretch column
pub(super) fn table_min_width(ui: &Ui, fixed: &[f32], stretch: usize) -> f32 {
    let padding = ui.clone_style().cell_padding()[0] * 2.0 + 1.0;
    let columns = fixed.len() + stretch;
    fixed.iter().sum::<f32>() + MIN_STRETCH_WIDTH * stretch as f32 + padding * columns as f32
}
