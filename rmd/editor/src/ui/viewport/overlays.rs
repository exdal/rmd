use dear_imgui_rs::Ui;
use dmm::Coord;
use editor::tool::Tool;

use super::{ViewFrame, stroke::active_placement_flash};
use crate::{
    session::{GuideBadge, Session},
    settings::Settings,
    ui::{
        TopOverlayState,
        UiState,
        common::dpi,
        draw_guide_badges,
        draw_highlights,
        draw_history_overlay,
        draw_identical_outlines,
        draw_placement_preview,
        draw_selected_pixel_grid,
        draw_tile_grid,
        draw_top_overlay,
    },
};

pub(super) fn banner_contains(ui: &Ui, session: &Session, frame: &ViewFrame<'_>) -> bool {
    let scale = dpi(ui);
    let min = frame.layout.viewport.min;
    let top = frame.layout.top_overlay.max[1];
    let mouse = frame.mouse;

    session.git_state(frame.id).is_some_and(|git| git.pending_load)
        && mouse[0] >= min[0] + 8.0 * scale
        && mouse[0] <= min[0] + 320.0 * scale
        && mouse[1] >= top + 4.0 * scale
        && mouse[1] <= top + ui.frame_height() + 16.0 * scale
}

pub(super) fn draw_recent_prefabs(ui: &Ui, session: &mut Session, settings: &Settings, frame: &ViewFrame<'_>) {
    let recent_prefabs = session.recent_prefabs();
    if !recent_prefabs.is_empty() {
        draw_history_overlay(
            ui,
            session,
            settings.keybindings,
            frame.layout.bottom_overlay,
            recent_prefabs.to_vec(),
        );
    }
}

impl UiState {
    pub(super) fn draw_view_overlays(
        &mut self, ui: &Ui, session: &mut Session, settings: &Settings, frame: &ViewFrame<'_>,
        previous_hover: Option<Coord>, guide_badges: &[GuideBadge],
    ) {
        let viewport = frame.layout.viewport;
        if settings.show_tile_grid {
            draw_tile_grid(
                ui,
                session,
                frame.id,
                frame.camera,
                settings.tile_grid_min_pixels,
                settings.show_tile_grid_axis,
                viewport.min,
                viewport.max,
            );
        }

        if frame.is_active
            && settings.show_selected_pixel_grid
            && let Some(transform) = session.gizmo_transform()
        {
            draw_selected_pixel_grid(
                ui,
                frame.camera,
                &transform,
                session.options.tile_size,
                settings.selected_pixel_grid_min_pixels,
                settings.show_pixel_grid_axis,
                viewport.min,
                viewport.max,
            );
        }

        if frame.is_active && self.inspector.edits_identical() {
            session.refresh_identical();
            draw_identical_outlines(ui, session, frame.camera, viewport);
        }

        {
            // The hover comes from the previous frame, this runs before the cursor is resolved
            let highlights = session.highlights(frame.id, previous_hover);
            let highlights = highlights.iter().collect::<Vec<_>>();
            draw_highlights(ui, frame.camera, viewport, &highlights, session.options.tile_size);
        }

        draw_guide_badges(ui, frame.camera, viewport.min, viewport.max, guide_badges);
    }

    pub(super) fn draw_place_preview(
        &mut self, ui: &Ui, session: &mut Session, settings: &Settings, frame: &mut ViewFrame<'_>, tool: Tool,
    ) {
        let preview_coord = (tool == Tool::Place)
            .then(|| self.gizmo.placement_coord().or(frame.coord))
            .flatten();
        let active_flash = active_placement_flash(&mut self.placement_flash, ui.time(), settings.tile_place_flash);
        frame.interaction.placement_flash = active_flash.map(|(_, flash)| flash);

        if let Some(coord) = preview_coord
            && session.can_edit_at(coord)
            && active_flash.is_none_or(|(flash_coord, _)| flash_coord != coord)
        {
            draw_placement_preview(
                ui,
                session,
                frame.camera,
                coord,
                frame.layout.viewport.min,
                frame.layout.viewport.max,
            );
        }
    }

    pub(super) fn draw_top_overlays(
        &mut self, ui: &Ui, session: &mut Session, settings: &Settings, frame: &ViewFrame<'_>,
    ) {
        let top_overlay = frame.layout.top_overlay;
        draw_top_overlay(
            ui,
            session,
            top_overlay,
            TopOverlayState {
                keybindings: settings.keybindings,
                selection_busy: frame.gestures.is_busy(),
                block_selection_options: &mut self.block_selection_options,
                fill_mode: &mut self.fill_mode,
                custom_fill_boundaries: &mut self.custom_fill_boundaries,
                custom_fill_search: &mut self.custom_fill_search,
                new_level_dialog: &mut self.new_level_dialog,
            },
        );

        if frame.is_active && session.git_state(frame.id).is_some_and(|git| git.pending_load) {
            let scale = dpi(ui);
            ui.set_cursor_screen_pos([
                frame.layout.viewport.min[0] + 12.0 * scale,
                top_overlay.max[1] + 8.0 * scale,
            ]);
            ui.text("Merge conflicts on disk");
            ui.same_line();
            if ui.small_button("Load conflicts") {
                self.pending_conflict_reload = Some(frame.id);
            }
        }
    }
}
