use dear_imgui_rs::{Condition, ModalPopupToken, StyleColor, TableFlags, Ui, WindowFlags, WindowHoveredFlags, sys};

const MIN_STRETCH_WIDTH: f32 = 40.0;

/// Marks everything an identical scope edit reaches, in the inspector and on the map
pub(super) const IDENTICAL_EDIT_COLOR: [f32; 4] = [1.0, 0.7, 0.15, 1.0];

pub(super) fn center_next_window(ui: &Ui, condition: Condition) {
    let center = ui.main_viewport().work_center();
    ui.with_bound_context(|| unsafe {
        sys::igSetNextWindowPos(
            sys::ImVec2 {
                x: center[0],
                y: center[1],
            },
            condition as i32,
            sys::ImVec2 { x: 0.5, y: 0.5 },
        );
    });
}

pub(super) fn begin_centered_modal<'ui>(
    ui: &'ui Ui, name: &'ui str, flags: WindowFlags,
) -> Option<ModalPopupToken<'ui>> {
    let condition = if flags.contains(WindowFlags::NO_MOVE) {
        ui.set_next_window_viewport(ui.main_viewport().id());
        Condition::Always
    } else {
        Condition::Appearing
    };
    center_next_window(ui, condition);
    ui.begin_modal_popup_config(name).flags(flags).begin()
}

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

pub(crate) fn dpi(ui: &Ui) -> f32 {
    let style = unsafe { ui.style() };
    style.font_scale_dpi() * style.font_scale_main()
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
        text_wrapped_colored(ui, ui.style_color(StyleColor::TextDisabled), text);
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
    fixed.iter().sum::<f32>() + MIN_STRETCH_WIDTH * dpi(ui) * stretch as f32 + padding * columns as f32
}

pub(super) const fn opaque(color: [f32; 3]) -> [f32; 4] { [color[0], color[1], color[2], 1.0] }

pub(super) fn focus_window_on_hover(ui: &Ui) {
    if !ui.is_window_focused()
        && !ui.is_any_item_active()
        && ui.is_window_hovered_with_flags(WindowHoveredFlags::CHILD_WINDOWS | WindowHoveredFlags::NO_POPUP_HIERARCHY)
    {
        ui.set_window_focus(None);
    }
}

pub(super) fn fit_icon(width: u32, height: u32, extent: f32) -> [f32; 2] {
    let width = width.max(1) as f32;
    let height = height.max(1) as f32;
    let scale = extent.max(1.0) / width.max(height);

    [width * scale, height * scale]
}

#[cfg(test)]
mod tests {
    use dear_imgui_rs::{Condition, Ui, WindowFlags, sys};

    use super::{begin_centered_modal, focus_window_on_hover};
    use crate::ui::{
        IMGUI_CONTEXT,
        fixtures::{
            PopupContext,
            assert_window_centered,
            finish_frame,
            popup_context,
            rectangle_context,
            set_desktop_geometry,
        },
    };

    #[test]
    fn movable_modals_keep_their_position_until_reopened() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut context = popup_context(true);
        let name = "Movable modal";
        let flags = WindowFlags::ALWAYS_AUTO_RESIZE | WindowFlags::NO_SAVED_SETTINGS;
        let draw = |context: &mut PopupContext, open: bool, close: bool, position: Option<[f32; 2]>| {
            let ui = context.frame();
            if open {
                ui.open_popup(name);
            }
            if let Some(_modal) = begin_centered_modal(ui, name, flags) {
                ui.text("A movable confirmation dialog");
                if let Some(position) = position {
                    ui.set_window_pos(position);
                }
                if close {
                    ui.close_current_popup();
                }
            }
            finish_frame(context);
        };

        set_desktop_geometry(&mut context, [0.0, 0.0], [1920.0, 1080.0]);
        for frame in 0..4 {
            draw(&mut context, frame == 0, false, None);
        }
        assert_window_centered(&mut context, c"Movable modal");
        draw(&mut context, false, false, Some([40.0, 80.0]));
        for _ in 0..3 {
            draw(&mut context, false, false, None);
        }
        // SAFETY: the test owns the current context and its live modal.
        unsafe {
            let window = &*sys::igFindWindowByName(c"Movable modal".as_ptr());
            assert_eq!([window.Pos.x, window.Pos.y], [40.0, 80.0]);
        }
        draw(&mut context, false, true, None);
        draw(&mut context, false, false, None);
        set_desktop_geometry(&mut context, [180.0, 90.0], [960.0, 640.0]);
        for frame in 0..4 {
            draw(&mut context, frame == 0, false, None);
        }
        assert_window_centered(&mut context, c"Movable modal");
    }

    fn draw_hover_focus_test_window(ui: &Ui, open_popup: bool) -> (bool, bool) {
        let mut focused = false;
        let mut popup_open = false;
        ui.window("hover-focus-target")
            .position([20.0, 20.0], Condition::Always)
            .size([300.0, 240.0], Condition::Always)
            .build(|| {
                focus_window_on_hover(ui);
                focused = ui.is_window_focused();
                ui.child_window("hover-focus-child")
                    .size([200.0, 100.0])
                    .build(ui, || ui.text("Child content"));

                if open_popup {
                    ui.open_popup("hover-focus-popup");
                }
                if let Some(_popup) = ui.begin_popup("hover-focus-popup") {
                    popup_open = true;
                    ui.text("Popup content");
                    ui.dummy([120.0, 80.0]);
                }
            });

        (focused, popup_open)
    }

    fn draw_hover_focus_holder(ui: &Ui) {
        ui.window("hover-focus-holder")
            .position([400.0, 20.0], Condition::Always)
            .size([200.0, 200.0], Condition::Always)
            .build(|| ui.text("Initially focused"));
    }

    #[test]
    fn hover_focus_keeps_child_windows_and_ignores_popups() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut context = rectangle_context();
        context.io_mut().add_mouse_pos_event([100.0, 100.0]);

        let ui = context.frame();
        draw_hover_focus_test_window(ui, false);
        draw_hover_focus_holder(ui);
        ui.set_window_focus(Some("hover-focus-holder"));
        assert!(context.render_legacy().valid());

        let ui = context.frame();
        let (focused, popup_open) = draw_hover_focus_test_window(ui, true);
        draw_hover_focus_holder(ui);
        assert!(focused, "hovering an embedded child should focus its parent window");
        assert!(popup_open);
        assert!(context.render_legacy().valid());

        context.io_mut().add_mouse_pos_event([110.0, 110.0]);
        let ui = context.frame();
        let (_, popup_open) = draw_hover_focus_test_window(ui, false);
        draw_hover_focus_holder(ui);
        assert!(popup_open, "hovering a popup should not refocus its parent window");
        assert!(context.render_legacy().valid());
    }
}
