use dear_imgui_rs::{DrawListMut, Ui};
use dmm::Coord;
use editor::{
    conflict::{Region, Side},
    document::DocumentId,
    tool::SelectionRotation,
};
use render::Renderer;

use super::{draw_marching_edge, marching_stripe_offset};
use crate::{
    camera::Controller,
    session::{BlockPreviewSource, GuideBadge, PlacementPreview, Session},
};

pub(super) const OVERLAY_PADDING: f32 = 4.0;

pub(super) const OVERLAY_BG: [f32; 4] = [0.0, 0.0, 0.0, 0.55];

const PLACEMENT_PREVIEW_PERIOD: f64 = 1.5;

pub(super) fn draw_overlay_underlay(ui: &Ui, bounds: OverlayRect) {
    ui.get_window_draw_list()
        .add_rect(bounds.min, bounds.max, OVERLAY_BG)
        .filled(true)
        .build();
}

pub(super) fn draw_highlights(
    ui: &Ui, camera: &Controller, viewport: OverlayRect, highlights: &[&editor::bake::Highlight], tile_size: u32,
) {
    if highlights.is_empty() {
        return;
    }

    let tile_size = tile_size.max(1) as f32;
    let offset = marching_stripe_offset(ui);
    let draw = ui.get_window_draw_list();
    draw.with_clip_rect(viewport.min, viewport.max, || {
        for highlight in highlights {
            let accent = [highlight.color[0], highlight.color[1], highlight.color[2], 1.0];
            let wash = [
                highlight.color[0],
                highlight.color[1],
                highlight.color[2],
                highlight.fill,
            ];

            for tile in &highlight.tiles {
                let left = (tile.position[0] - 1) as f32 * tile_size;
                let bottom = (tile.position[1] - 1) as f32 * tile_size;
                let top_left = camera.map_to_screen([left, bottom + tile_size]);
                let bottom_right = camera.map_to_screen([left + tile_size, bottom]);
                let min = [viewport.min[0] + top_left[0], viewport.min[1] + top_left[1]];
                let max = [viewport.min[0] + bottom_right[0], viewport.min[1] + bottom_right[1]];
                if !min.iter().chain(&max).all(|value| value.is_finite())
                    || max[0] < viewport.min[0]
                    || max[1] < viewport.min[1]
                    || min[0] > viewport.max[0]
                    || min[1] > viewport.max[1]
                {
                    continue;
                }

                if highlight.fill > 0.0 {
                    draw.add_rect(min, max, wash).filled(true).build();
                }
                if !highlight.outline {
                    continue;
                }

                // Screen y grows downward, so the map's north edge is the rectangle's top.
                for (edge, from, to) in [
                    (editor::bake::HIGHLIGHT_EDGE_NORTH, min, [max[0], min[1]]),
                    (editor::bake::HIGHLIGHT_EDGE_SOUTH, [min[0], max[1]], max),
                    (editor::bake::HIGHLIGHT_EDGE_WEST, min, [min[0], max[1]]),
                    (editor::bake::HIGHLIGHT_EDGE_EAST, [max[0], min[1]], max),
                ] {
                    if tile.edges & edge != 0 {
                        draw_marching_edge(&draw, from, to, offset, accent);
                    }
                }
            }

            draw_highlight_label(ui, &draw, camera, viewport, highlight, tile_size, accent);
        }
    });
}

pub(super) fn conflict_controls_layout(
    ui: &Ui, camera: &Controller, region: &Region, tile_size: u32, viewport_min: [f32; 2], bounds: OverlayRect,
    theirs: &str,
) -> Option<([f32; 2], OverlayRect)> {
    let tile_size = tile_size.max(1) as f32;
    let corner = camera.map_to_screen([(region.min.x - 1) as f32 * tile_size, region.max.y as f32 * tile_size]);
    let far = camera.map_to_screen([region.max.x as f32 * tile_size, (region.min.y - 1) as f32 * tile_size]);
    let corner = [viewport_min[0] + corner[0], viewport_min[1] + corner[1]];
    let far = [viewport_min[0] + far[0], viewport_min[1] + far[1]];
    if far[0] < bounds.min[0] || far[1] < bounds.min[1] || corner[0] > bounds.max[0] || corner[1] > bounds.max[1] {
        return None;
    }
    let style = ui.clone_style();
    let width = ui.calc_text_size("HEAD")[0]
        + ui.calc_text_size(theirs)[0]
        + style.frame_padding()[0] * 4.0
        + style.item_spacing()[0];
    let height = ui.frame_height();
    let x = corner[0].clamp(bounds.min[0], (bounds.max[0] - width).max(bounds.min[0]));
    let y = (corner[1] - height - 4.0).clamp(bounds.min[1], (bounds.max[1] - height).max(bounds.min[1]));
    Some((
        [x, y],
        OverlayRect {
            min: [x - 3.0, y - 3.0],
            max: [x + width + 3.0, y + height + 3.0],
        },
    ))
}

pub(super) fn draw_conflict_controls(
    ui: &Ui, session: &mut Session, id: DocumentId, camera: &Controller, regions: &[Region], viewport_min: [f32; 2],
    bounds: OverlayRect,
) {
    let tile_size = session.options.tile_size;
    let theirs = session
        .git_state(id)
        .and_then(|git| git.conflicts.as_ref())
        .map_or(String::from("theirs"), |state| {
            state.side_label(Side::Theirs).to_owned()
        });
    let detail = session
        .git_state(id)
        .and_then(|git| git.conflicts.as_ref())
        .map_or(String::from("Take the incoming side"), |state| {
            state.side_detail(Side::Theirs)
        });

    let mut choice = None;
    for region in regions {
        let Some((position, rect)) =
            conflict_controls_layout(ui, camera, region, tile_size, viewport_min, bounds, &theirs)
        else {
            continue;
        };

        draw_overlay_underlay(ui, rect);

        ui.set_cursor_screen_pos(position);
        let key = format!("conflict-{}", region.id());
        let _id = ui.push_id(&key);

        if ui.small_button("HEAD") {
            choice = Some((region.tiles.clone(), Side::Ours));
        }
        ui.same_line();
        if ui.small_button(&theirs) {
            choice = Some((region.tiles.clone(), Side::Theirs));
        }

        if ui.is_item_hovered() {
            ui.set_item_tooltip(&detail);
            session.prepare_block_preview(
                id,
                BlockPreviewSource::Conflict {
                    region: region.id(),
                    side: Side::Theirs,
                },
                region.min,
                SelectionRotation::Original,
            );
        }
    }

    if let Some((coords, side)) = choice {
        session.resolve_conflict(id, &coords, side);
    }
}

fn draw_highlight_label(
    ui: &Ui, draw: &DrawListMut<'_>, camera: &Controller, viewport: OverlayRect, highlight: &editor::bake::Highlight,
    tile_size: f32, accent: [f32; 4],
) {
    let Some(label) = highlight.label.as_deref() else {
        return;
    };
    let Some(anchor) = highlight
        .tiles
        .iter()
        .max_by_key(|tile| (tile.position[1], -tile.position[0]))
    else {
        return;
    };

    let local = camera.map_to_screen([
        (anchor.position[0] - 1) as f32 * tile_size,
        anchor.position[1] as f32 * tile_size,
    ]);

    if !local.iter().all(|value| value.is_finite()) {
        return;
    }

    let text_size = ui.calc_text_size(label);
    let min = [
        (viewport.min[0] + local[0]).clamp(viewport.min[0] + 4.0, viewport.max[0] - text_size[0] - 10.0),
        (viewport.min[1] + local[1] - text_size[1] - 8.0).max(viewport.min[1] + 4.0),
    ];

    let max = [min[0] + text_size[0] + 6.0, min[1] + text_size[1] + 4.0];
    draw.add_rect(min, max, OVERLAY_BG).filled(true).build();
    draw.add_rect(min, max, accent).build();
    draw.add_text([min[0] + 3.0, min[1] + 2.0], [1.0; 4], label);
}

pub(super) fn draw_guide_badges(
    ui: &Ui, camera: &Controller, viewport_min: [f32; 2], viewport_max: [f32; 2], badges: &[GuideBadge],
) {
    if badges.is_empty() {
        return;
    }

    let draw = ui.get_window_draw_list();
    draw.with_clip_rect(viewport_min, viewport_max, || {
        for badge in badges {
            let local = camera.map_to_screen(badge.position);
            if !local.iter().all(|value| value.is_finite()) {
                continue;
            }
            let label = format!("Z{}", badge.z);
            let text_size = ui.calc_text_size(&label);
            let min = [
                viewport_min[0] + local[0] + 6.0,
                viewport_min[1] + local[1] - text_size[1] - 6.0,
            ];
            let max = [min[0] + text_size[0] + 6.0, min[1] + text_size[1] + 4.0];
            draw.add_rect(min, max, [0.12, 0.08, 0.02, 0.92]).filled(true).build();
            draw.add_rect(min, max, [1.0, 0.5, 0.0, 1.0]).build();
            draw.add_text([min[0] + 3.0, min[1] + 2.0], [1.0; 4], label);
        }
    });
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct OverlayRect {
    pub(super) min: [f32; 2],
    pub(super) max: [f32; 2],
}

impl OverlayRect {
    pub(super) fn contains(self, point: [f32; 2]) -> bool {
        point[0] >= self.min[0] && point[0] < self.max[0] && point[1] >= self.min[1] && point[1] < self.max[1]
    }
}

pub(super) fn draw_placement_preview(
    ui: &Ui, session: &mut Session, camera: &Controller, coord: Coord, viewport_min: [f32; 2], viewport_max: [f32; 2],
) {
    let Some(preview) = session.placement_preview() else {
        return;
    };
    let bounds = placement_preview_bounds(camera, viewport_min, coord, session.options.tile_size, preview);
    let mut tint = preview.thumbnail.tint;
    tint[3] *= placement_preview_opacity(ui.time());
    let draw = ui.get_window_draw_list();

    draw.with_clip_rect(viewport_min, viewport_max, || {
        draw.add_image(
            Renderer::sprite_texture(preview.thumbnail.texture.index),
            bounds.min,
            bounds.max,
            preview.thumbnail.uv0,
            preview.thumbnail.uv1,
            tint,
        );
    });
}

fn placement_preview_bounds(
    camera: &Controller, viewport_min: [f32; 2], coord: Coord, tile_size: u32, preview: PlacementPreview,
) -> OverlayRect {
    let tile_size = tile_size.max(1);
    let x = coord.x.saturating_sub(1).saturating_mul(tile_size) as f32 + preview.offset[0] as f32;
    let y = coord.y.saturating_sub(1).saturating_mul(tile_size) as f32 + preview.offset[1] as f32;
    let width = preview.thumbnail.texture.width as f32;
    let height = preview.thumbnail.texture.height as f32;
    let top_left = camera.map_to_screen([x, y + height]);
    let bottom_right = camera.map_to_screen([x + width, y]);

    OverlayRect {
        min: [viewport_min[0] + top_left[0], viewport_min[1] + top_left[1]],
        max: [viewport_min[0] + bottom_right[0], viewport_min[1] + bottom_right[1]],
    }
}

fn placement_preview_opacity(time: f64) -> f32 {
    let phase = time.rem_euclid(PLACEMENT_PREVIEW_PERIOD) / PLACEMENT_PREVIEW_PERIOD * std::f64::consts::TAU;

    (0.9 + 0.1 * phase.cos()) as f32
}

#[cfg(test)]
mod tests {
    use dmm::Coord;
    use render::SpriteTexture;

    use super::{OverlayRect, placement_preview_bounds, placement_preview_opacity};
    use crate::{camera::Controller, session::PlacementPreview};

    #[test]
    fn placement_preview_breathes_between_full_and_eighty_percent_opacity() {
        assert!((placement_preview_opacity(0.0) - 1.0).abs() < f32::EPSILON);
        assert!((placement_preview_opacity(0.75) - 0.8).abs() < f32::EPSILON);
        assert!((placement_preview_opacity(1.5) - 1.0).abs() < f32::EPSILON);
        assert!((placement_preview_opacity(3.75) - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn placement_preview_bounds_preserve_anchor_offsets_dimensions_and_zoom() {
        let mut camera = Controller::new();
        camera.resize(100, 80);
        camera.camera.x = 50.0;
        camera.camera.y = 40.0;
        camera.camera.zoom = 2.0;
        let preview = PlacementPreview {
            thumbnail: crate::session::PrefabThumbnail {
                texture: SpriteTexture {
                    width: 64,
                    height: 32,
                    ..Default::default()
                },
                uv0: [0.0; 2],
                uv1: [1.0; 2],
                tint: [1.0; 4],
            },
            offset: [-16, 4],
        };

        let bounds = placement_preview_bounds(&camera, [10.0, 20.0], Coord::new(2, 2, 1), 32, preview);

        assert_eq!(bounds.min, [-8.0, 4.0]);
        assert_eq!(bounds.max, [120.0, 68.0]);
    }

    #[test]
    fn overlay_bounds_exclude_their_far_edges() {
        let bounds = OverlayRect {
            min: [10.0, 20.0],
            max: [30.0, 40.0],
        };

        assert!(bounds.contains([10.0, 20.0]));
        assert!(bounds.contains([29.0, 39.0]));
        assert!(!bounds.contains([30.0, 39.0]));
        assert!(!bounds.contains([29.0, 40.0]));
    }
}
