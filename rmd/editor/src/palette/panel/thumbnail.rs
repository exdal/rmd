use dear_imgui_rs::{StyleColor, Ui};
use render::Renderer;
use ui::{
    icons::materialdesignicons::ICON_IMAGE_BROKEN,
    widgets::{dpi, fit_icon},
};

use crate::palette::session::PrefabThumbnail;

const ORIGIN_RADIUS: f32 = 3.0;
const ORIGIN_MAX_DISTANCE: f32 = 32.0;
const ORIGIN_COLOR: [f32; 4] = [1.0, 0.5, 0.0, 1.0];

#[derive(Debug, Clone, Copy)]
pub(crate) struct IconOrigin {
    pub offset: [i32; 2],
    pub tile_size: u32,
}

pub(crate) fn draw_thumbnail(
    ui: &Ui, thumbnail: Option<PrefabThumbnail>, min: [f32; 2], extent: f32, origin: Option<IconOrigin>,
) {
    let draw = ui.get_window_draw_list();
    let Some(thumbnail) = thumbnail else {
        let icon = ICON_IMAGE_BROKEN.to_string();
        let size = ui.calc_text_size(&icon);
        draw.add_text(
            [min[0] + (extent - size[0]) * 0.5, min[1] + (extent - size[1]) * 0.5],
            ui.style_color(StyleColor::TextDisabled),
            icon,
        );
        return;
    };

    let size = fit_icon(thumbnail.texture.width, thumbnail.texture.height, extent);
    let image_min = [min[0] + (extent - size[0]) * 0.5, min[1] + (extent - size[1]) * 0.5];
    let image_max = [image_min[0] + size[0], image_min[1] + size[1]];
    draw.add_image(
        Renderer::sprite_texture(thumbnail.texture.index),
        image_min,
        image_max,
        thumbnail.uv0,
        thumbnail.uv1,
        thumbnail.tint,
    );

    let Some(IconOrigin { offset, tile_size }) = origin else {
        return;
    };

    if offset == [0, 0] {
        return;
    }

    let scale = size[0] / thumbnail.texture.width.max(1) as f32;
    let half_tile = tile_size.max(1) as f32 * 0.5;
    let mut delta = [
        half_tile - offset[0] as f32 - thumbnail.texture.width as f32 * 0.5,
        half_tile - offset[1] as f32 - thumbnail.texture.height as f32 * 0.5,
    ];
    let distance = delta[0].hypot(delta[1]);
    if distance > ORIGIN_MAX_DISTANCE {
        let shrink = ORIGIN_MAX_DISTANCE / distance;
        delta = [delta[0] * shrink, delta[1] * shrink];
    }

    let radius = ORIGIN_RADIUS * dpi(ui);
    let center = [
        ((image_min[0] + image_max[0]) * 0.5 + delta[0] * scale).clamp(min[0] + radius, min[0] + extent - radius),
        ((image_min[1] + image_max[1]) * 0.5 - delta[1] * scale).clamp(min[1] + radius, min[1] + extent - radius),
    ];
    draw.add_circle(center, radius + 1.0, [0.05, 0.05, 0.05, 0.95])
        .filled(true)
        .build();
    draw.add_circle(center, radius, ORIGIN_COLOR).filled(true).build();
}
