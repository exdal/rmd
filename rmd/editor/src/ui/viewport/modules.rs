use dear_imgui_rs::{MouseButton, Ui};
use dmm::Coord;
use editor::{maploader::preview::ModuleVariantKey, tool::Tool};
use render::Renderer;

use super::ViewFrame;
use crate::{
    camera::Controller,
    session::Session,
    ui::{
        OverlayRect,
        common::dpi,
        draw_overlay_underlay,
        overlay::placement_preview_bounds,
        welcome::codebase_relative,
    },
};

fn label(choice: &ModuleVariantKey) -> String {
    let filename = choice.path.file_name().unwrap_or_default().to_string_lossy();
    if choice.occurrence == 0 {
        filename.into_owned()
    } else {
        format!("{filename} ({})", choice.occurrence + 1)
    }
}

fn module_path(session: &Session, choice: &ModuleVariantKey) -> String {
    match session.codebase_dir() {
        Some(base) => codebase_relative(base, &choice.path),
        None => choice.path.display().to_string(),
    }
}

pub(super) fn draw(
    ui: &Ui, session: &mut Session, frame: &mut ViewFrame<'_>, is_escape: bool, is_gizmo_hot: bool,
) -> bool {
    if !session.modules_enabled() || is_escape {
        frame.gestures.module_popup = None;
        return false;
    }
    let roots = session.module_roots().to_vec();
    if let Some((key, coord)) = frame.gestures.module_popup.as_mut() {
        match roots
            .iter()
            .find(|root| root.key == *key && root.coord.z == session.z())
        {
            Some(root) => *coord = root.coord,
            None => frame.gestures.module_popup = None,
        }
    }
    let viewport = frame.layout.viewport;
    let mut hovered = Vec::new();
    for root in &roots {
        let coord = root.coord;
        if coord.z != session.z() {
            continue;
        }

        let Some(preview) = root.marker.as_ref().and_then(|prefab| session.module_marker(prefab)) else {
            continue;
        };
        let bounds = placement_preview_bounds(frame.camera, viewport.min, coord, session.options.tile_size, preview);
        let draw = ui.get_window_draw_list();
        draw.with_clip_rect(viewport.min, viewport.max, || {
            draw.add_image(
                Renderer::sprite_texture(preview.thumbnail.texture.index),
                bounds.min,
                bounds.max,
                preview.thumbnail.uv0,
                preview.thumbnail.uv1,
                preview.thumbnail.tint,
            );
        });
        if viewport.contains(frame.mouse) && bounds.contains(frame.mouse) {
            hovered.push(root);
        }
    }
    let mut captured = false;
    let mut combo_open = false;
    if let Some((root, coord)) = frame
        .gestures
        .module_popup
        .as_ref()
        .and_then(|(key, _)| roots.iter().find(|root| root.key == *key))
        .map(|root| (root, root.coord))
    {
        let is_nested = !root.key.ancestry.is_empty();
        let (position, width, bounds) = controls_layout(
            ui,
            frame.camera,
            coord,
            session.options.tile_size,
            viewport.min,
            frame.layout.controls_area(),
            is_nested,
        );
        captured = bounds.contains(frame.mouse);
        draw_overlay_underlay(ui, bounds);
        if is_nested {
            ui.set_cursor_screen_pos([position[0], position[1] - ui.text_line_height_with_spacing()]);
            ui.text_disabled("Nested nodes cannot be moved");
        }

        ui.set_cursor_screen_pos(position);
        let _id = ui.push_id("module-variant");
        let current = root
            .selected
            .and_then(|index| root.variants.get(index))
            .map(|variant| label(&variant.choice))
            .unwrap_or_else(|| "No valid variants".into());
        let button = ui.frame_height();
        ui.set_next_item_width(width - 2.0 * (button + ui.clone_style().item_spacing()[0]));
        let mut choose = None;
        if let Some(_combo) = ui.begin_combo("##variant", &current) {
            combo_open = true;
            for (index, variant) in root.variants.iter().enumerate() {
                let _variant = ui.push_id(index as i32);
                let _disabled = ui.begin_disabled_with_cond(variant.error.is_some());
                if ui
                    .selectable_config(&label(&variant.choice))
                    .selected(root.selected == Some(index))
                    .build()
                {
                    choose = Some(index);
                }
                ui.set_item_tooltip(
                    variant
                        .error
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| module_path(session, &variant.choice)),
                );
            }
            if let Some(error) = &root.error {
                ui.text_wrapped(error.to_string());
            }
        }
        if let Some(error) = &root.error {
            ui.set_item_tooltip(error.to_string());
        }
        for (forward, text) in [(false, "<"), (true, ">")] {
            ui.same_line();
            let next = root.adjacent(forward);
            let _disabled = ui.begin_disabled_with_cond(next.is_none());
            if ui.button_with_size(text, [button, button]) {
                choose = next;
            }
        }
        if let Some(index) = choose {
            session.choose_module_variant(&root.key, index);
        }
    }
    if !captured && !combo_open && ui.is_mouse_clicked(MouseButton::Left) {
        if frame.is_hovered
            && let Some(first) = hovered.first()
        {
            // Repeated clicks cycle roots sharing the same marker position.
            let index = frame
                .gestures
                .module_popup
                .as_ref()
                .and_then(|(key, _)| hovered.iter().position(|root| root.key == *key))
                .map_or(0, |index| (index + 1) % hovered.len());
            let root = hovered.get(index).unwrap_or(first);
            let is_source = session.is_source_root(root);
            if is_gizmo_hot && is_source && session.selected_instance() == Some(root.key.instance) {
                return false;
            }

            frame.gestures.module_popup = Some((root.key.clone(), root.coord));
            if is_source && session.tool() == Tool::Select {
                session.select_instance(Some(root.key.instance));
            }
            captured = true;
        } else {
            frame.gestures.module_popup = None;
        }
    }
    captured || combo_open
}

pub(in crate::ui) fn controls_layout(
    ui: &Ui, camera: &Controller, coord: Coord, tile_size: u32, viewport_min: [f32; 2], area: OverlayRect,
    is_nested: bool,
) -> ([f32; 2], f32, OverlayRect) {
    let scale = dpi(ui);
    let width = 290.0 * scale;
    let padding = 6.0 * scale;
    let note = if is_nested {
        ui.text_line_height_with_spacing()
    } else {
        0.0
    };
    // Below the tile, since the gizmo's arrows point right and up.
    let anchor = camera.map_to_screen([
        (coord.x as f32 - 0.5) * tile_size as f32,
        (coord.y as f32 - 1.0) * tile_size as f32,
    ]);
    let position = [
        (viewport_min[0] + anchor[0] - width / 2.0).clamp(
            area.min[0] + padding,
            (area.max[0] - width - padding).max(area.min[0] + padding),
        ),
        (viewport_min[1] + anchor[1] + padding * 2.0 + note).clamp(
            area.min[1] + padding + note,
            (area.max[1] - ui.frame_height() - padding).max(area.min[1] + padding + note),
        ),
    ];
    let bounds = OverlayRect {
        min: [position[0] - padding, position[1] - note - padding],
        max: [position[0] + width + padding, position[1] + ui.frame_height() + padding],
    };
    (position, width, bounds)
}
