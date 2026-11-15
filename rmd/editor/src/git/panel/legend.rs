use dear_imgui_rs::Ui;
use ui::widgets::{dpi, label_width, opaque, same_line_if_fits};

use crate::git::blame::{
    BOUNDARY, UNCOMMITTED,
    session::{BlameState, blame_color},
};

const LEGEND_STEPS: usize = 32;
const LEGEND_WIDTH: f32 = 140.0;
const LEGEND_MIN_WIDTH: f32 = 48.0;

pub(super) fn draw_legend(ui: &Ui, blame: &BlameState) {
    let height = ui.text_line_height();
    let spacing = ui.clone_style().item_spacing()[0];
    let span = blame.result.heat_span;

    if span != 0 {
        let labels = label_width(ui, "Newest") + label_width(ui, "Oldest") + spacing * 2.0;
        let scale = dpi(ui);
        let width = (LEGEND_WIDTH * scale)
            .min(ui.content_region_avail_width() - labels)
            .max(LEGEND_MIN_WIDTH * scale);
        ui.text_disabled("Newest");
        ui.same_line();
        let origin = ui.cursor_screen_pos();
        let step = width / LEGEND_STEPS as f32;
        for index in 0..LEGEND_STEPS {
            let cell = (index * span.saturating_sub(1) / (LEGEND_STEPS - 1)) as u32;
            let color = blame_color(&blame.result, cell);
            let min = [origin[0] + step * index as f32, origin[1]];
            ui.get_window_draw_list()
                .add_rect(min, [min[0] + step + 0.5, min[1] + height], opaque(color))
                .filled(true)
                .build();
        }
        ui.dummy([width, height]);
        ui.same_line();
        ui.text_disabled("Oldest");
    }

    for (cell, label) in [(UNCOMMITTED, "Uncommitted"), (BOUNDARY, "Older")] {
        if span != 0 || cell != UNCOMMITTED {
            same_line_if_fits(ui, height + spacing + label_width(ui, label));
        }

        let origin = ui.cursor_screen_pos();
        draw_swatch(ui, origin, height, blame_color(&blame.result, cell));
        ui.dummy([height, height]);
        ui.same_line();
        ui.text_disabled(label);
    }
}

pub(super) fn draw_swatch(ui: &Ui, origin: [f32; 2], size: f32, color: [f32; 3]) {
    let inset = (size * 0.15).round();
    ui.get_window_draw_list()
        .add_rect(
            [origin[0] + inset, origin[1] + inset],
            [origin[0] + size - inset, origin[1] + size - inset],
            opaque(color),
        )
        .filled(true)
        .rounding(2.0)
        .build();
}
