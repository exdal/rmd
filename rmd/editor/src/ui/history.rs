use dear_imgui_rs::{StyleColor, Ui};
use dmm::Prefab;

use super::{
    OverlayRect,
    common::{IconOrigin, dpi, draw_thumbnail},
    draw_overlay_underlay, overlay_padding,
};
use crate::{session::Session, settings::KeyBindings};

const RECENT_ICON_SIZE: f32 = 48.0;

const RECENT_BADGE_BG: [f32; 4] = [0.0, 0.0, 0.0, 0.8];

const RECENT_BADGE_TEXT: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

pub(super) fn draw_history_overlay(
    ui: &Ui, session: &mut Session, keybindings: KeyBindings, bounds: OverlayRect, recent_prefabs: Vec<Prefab>,
) {
    draw_overlay_underlay(ui, bounds);

    let button_size = recent_button_size(ui);
    let palette = session.palette().cloned();
    let mut chosen = None;
    let padding = overlay_padding(ui);
    ui.set_cursor_screen_pos([bounds.min[0] + padding, bounds.min[1] + padding]);

    for (index, prefab) in recent_prefabs.iter().enumerate() {
        if index > 0 {
            ui.same_line();
        }
        let key = keybindings
            .recent(index)
            .map(|binding| binding.label(ui))
            .unwrap_or_else(|| String::from("?"));
        let _selected = (palette.as_ref() == Some(prefab))
            .then(|| ui.push_style_color(StyleColor::Button, ui.style_color(StyleColor::ButtonActive)));
        let clicked = ui.button_with_size(format!("##recent-{index}"), [button_size; 2]);
        let icon_extent = RECENT_ICON_SIZE * dpi(ui);
        let button_min = ui.item_rect_min();
        let inset = (button_size - icon_extent) * 0.5;
        let placement = session.prefab_placement(prefab);
        draw_thumbnail(
            ui,
            placement.map(|placement| placement.thumbnail),
            [button_min[0] + inset, button_min[1] + inset],
            icon_extent,
            placement.map(|placement| IconOrigin {
                offset: placement.offset,
                tile_size: session.options.tile_size,
            }),
        );
        draw_recent_badge(ui, &key);
        if clicked {
            chosen = Some(index);
        }
        ui.set_item_tooltip(prefab_tooltip(prefab));
    }

    if let Some(index) = chosen {
        session.choose_recent(index);
    }
}

pub(super) fn recent_button_size(ui: &Ui) -> f32 {
    let padding = ui.clone_style().frame_padding();

    RECENT_ICON_SIZE * dpi(ui) + padding[0].max(padding[1]) * 2.0
}

fn draw_recent_badge(ui: &Ui, key: &str) {
    let item_min = ui.item_rect_min();
    let text_size = ui.calc_text_size(key);
    let badge_min = [item_min[0] + 2.0, item_min[1] + 2.0];
    let badge_max = [badge_min[0] + text_size[0] + 4.0, badge_min[1] + text_size[1] + 2.0];
    let draw = ui.get_window_draw_list();
    draw.add_rect(badge_min, badge_max, RECENT_BADGE_BG)
        .rounding(2.0)
        .filled(true)
        .build();
    draw.add_text([badge_min[0] + 2.0, badge_min[1] + 1.0], RECENT_BADGE_TEXT, key);
}

fn prefab_tooltip(prefab: &Prefab) -> String {
    let mut tooltip = prefab.path.to_string();
    for (name, value) in &prefab.vars {
        tooltip.push_str(&format!("\n{name} = {}", dmm::writer::format_value(&value.value)));
    }

    tooltip
}

#[cfg(test)]
mod tests {
    use core::path::TreePath;

    use dmm::Prefab;

    use super::prefab_tooltip;

    #[test]
    fn prefab_tooltips_include_the_path_and_exact_overrides() {
        let mut prefab = Prefab::new(TreePath::parse("/obj/table"));
        prefab.set_var("name".into(), core::types::Value::Text("custom".into()));

        assert_eq!(prefab_tooltip(&prefab), "/obj/table\nname = \"custom\"");
    }
}
