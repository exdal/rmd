use dear_imgui_rs::{StyleColor, StyleVar, Ui};
use dmm::Prefab;
use editor::icons::materialdesignicons::ICON_IMAGE_BROKEN;
use render::Renderer;

use super::{OVERLAY_PADDING, OverlayRect, common::fit_icon, draw_overlay_underlay};
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
    ui.set_cursor_screen_pos([bounds.min[0] + OVERLAY_PADDING, bounds.min[1] + OVERLAY_PADDING]);

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
        let clicked = match session.prefab_thumbnail(prefab) {
            Some(thumbnail) => {
                let image_size = fit_recent_icon(thumbnail.texture.width, thumbnail.texture.height);
                let padding = [(button_size - image_size[0]) * 0.5, (button_size - image_size[1]) * 0.5];
                let _padding = ui.push_style_var(StyleVar::FramePadding(padding));

                ui.image_button_config(
                    format!("recent-{index}"),
                    Renderer::sprite_texture(thumbnail.texture.index),
                    image_size,
                )
                .uv0(thumbnail.uv0)
                .uv1(thumbnail.uv1)
                .tint_color(thumbnail.tint)
                .build()
            },
            None => ui.button_with_size(format!("{ICON_IMAGE_BROKEN}##recent-{index}"), [button_size; 2]),
        };
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

    RECENT_ICON_SIZE + padding[0].max(padding[1]) * 2.0
}

fn fit_recent_icon(width: u32, height: u32) -> [f32; 2] { fit_icon(width, height, RECENT_ICON_SIZE) }

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

    use super::{RECENT_ICON_SIZE, fit_recent_icon, prefab_tooltip};

    #[test]
    fn recent_icons_fit_inside_a_square_without_changing_aspect_ratio() {
        assert_eq!(fit_recent_icon(32, 32), [RECENT_ICON_SIZE, RECENT_ICON_SIZE]);
        assert_eq!(fit_recent_icon(64, 32), [RECENT_ICON_SIZE, RECENT_ICON_SIZE / 2.0]);
        assert_eq!(fit_recent_icon(16, 32), [RECENT_ICON_SIZE / 2.0, RECENT_ICON_SIZE]);
        assert_eq!(fit_recent_icon(0, 0), [RECENT_ICON_SIZE, RECENT_ICON_SIZE]);
    }

    #[test]
    fn prefab_tooltips_include_the_path_and_exact_overrides() {
        let mut prefab = Prefab::new(TreePath::parse("/obj/table"));
        prefab.set_var("name".into(), core::types::Value::Text("custom".into()));

        assert_eq!(prefab_tooltip(&prefab), "/obj/table\nname = \"custom\"");
    }
}
