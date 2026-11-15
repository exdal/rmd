use dear_imgui_rs::{DragFlags, Ui};
use ui::widgets::dpi;

use crate::{
    session::Session,
    settings::{SelectionHighlight, Settings},
};

use super::draw_section_heading;

pub(super) fn draw_viewport_settings(ui: &Ui, session: &mut Session, settings: &mut Settings) {
    draw_section_heading(ui, "Areas");
    ui.checkbox("Show areas", &mut session.options.show_areas);
    ui.checkbox("Show area outlines", &mut session.options.show_area_outlines);

    ui.separator();
    draw_section_heading(ui, "Lighting");
    ui.checkbox("Show lighting", &mut session.options.show_lighting);
    ui.set_next_item_width(200.0 * dpi(ui));
    ui.slider(
        "Minimum brightness (%)",
        0,
        100,
        &mut session.options.minimum_light_brightness_percent,
    );
    ui.set_item_tooltip("0% keeps original lighting, 100% removes darkness.");

    ui.separator();
    draw_section_heading(ui, "Feedback");
    ui.checkbox("Tile placement flash", &mut settings.tile_place_flash);
    ui.checkbox("Selection guide lines", &mut settings.selection_guide_line);

    ui.separator();
    draw_section_heading(ui, "Grid");
    ui.checkbox("Tile grid overlay", &mut settings.show_tile_grid);
    ui.set_next_item_width(120.0 * dpi(ui));
    drag_min_pixels(
        ui,
        "Hide tile grid below (px per tile)",
        &mut settings.tile_grid_min_pixels,
    );
    ui.checkbox("Tile grid axis", &mut settings.show_tile_grid_axis);

    ui.checkbox("Pixel grid on selected tile", &mut settings.show_selected_pixel_grid);
    ui.set_next_item_width(120.0 * dpi(ui));
    drag_min_pixels(
        ui,
        "Hide pixel grid below (px per world px)",
        &mut settings.selected_pixel_grid_min_pixels,
    );
    ui.checkbox("Pixel grid axis", &mut settings.show_pixel_grid_axis);

    ui.separator();
    draw_section_heading(ui, "Selection");
    ui.text("Highlight style");
    for (index, highlight) in SelectionHighlight::ALL.into_iter().enumerate() {
        if index > 0 {
            ui.same_line();
        }
        if ui.radio_button(highlight.label(), settings.selection_highlight == highlight) {
            settings.selection_highlight = highlight;
        }
    }
}

fn drag_min_pixels(ui: &Ui, label: &str, value: &mut u32) {
    let mut pixels = *value as i32;
    if ui
        .drag_int_config(label)
        .range(1, 128)
        .flags(DragFlags::ALWAYS_CLAMP)
        .build(ui, &mut pixels)
    {
        *value = pixels.max(1) as u32;
    }
}
