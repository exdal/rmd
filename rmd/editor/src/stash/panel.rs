use std::mem;

use dear_imgui_rs::{Condition, MouseButton, Ui, WindowKey, WindowKeyError};
use ui::widgets::focus_window_on_hover;

use crate::{
    document::DocumentId,
    frontend::UiState,
    session::Session,
    settings::{Settings, keybind::KeybindAction},
    viewport::{
        camera::Controller,
        panel::{MapViewState, ViewKind, ViewOutput, overlay::OverlayRect, panel_extent},
    },
};

const GAP_TILES: f32 = 2.0;

pub(crate) struct StashView {
    window: WindowKey,
    camera: Controller,
    placed: bool,
    refit: bool,
}

impl StashView {
    pub(crate) fn new() -> Result<Self, WindowKeyError> {
        Ok(Self {
            window: WindowKey::new("stash", "Stash")?,
            camera: Controller::new(),
            placed: false,
            refit: true,
        })
    }

    pub(crate) fn refit(&mut self) {
        self.refit = true;
    }
}

/// World pixels in the strip, entries sit side by side on its bottom edge
struct StripEntry {
    id: DocumentId,
    x: f32,
    width: f32,
    height: f32,
}

fn strip(session: &Session) -> Vec<StripEntry> {
    let tile = session.options.tile_size.max(1) as f32;
    let mut x = 0.0;
    session
        .stash_ids()
        .into_iter()
        .filter_map(|id| {
            let size = session.state.document(id)?.map.size();
            let entry = StripEntry {
                id,
                x,
                width: size.x as f32 * tile,
                height: size.y as f32 * tile,
            };
            x += entry.width + GAP_TILES * tile;

            Some(entry)
        })
        .collect()
}

impl UiState {
    pub(crate) fn draw_stash(
        &mut self, ui: &Ui, session: &mut Session, settings: &mut Settings, refit_active: bool, output: ViewOutput<'_>,
    ) {
        let ViewOutput { visible, coop } = output;
        if !mem::replace(&mut self.stash_view.placed, true)
            && let Some(node) = self.central_node.or(self.dockspace_root)
        {
            ui.set_next_window_dock_id_with_cond(node, Condition::Always);
        }

        self.stash_view.refit |= refit_active && session.is_stash_active();
        let window = self.stash_view.window.clone();
        let mut is_open = true;
        ui.window(window.label(format!("Stash ({})", session.stash_len())))
            .opened(&mut is_open)
            .size([640.0, 360.0], Condition::FirstUseEver)
            .build(|| {
                if settings.focus_windows_on_hover && !self.panel_focus_requested {
                    focus_window_on_hover(ui);
                }

                let dock = ui.get_window_dock_id();
                if dock.raw() != 0 {
                    self.central_node = Some(dock);
                }

                let entries = strip(session);
                let Some(last) = entries.last() else {
                    ui.text_wrapped(
                        "Select a region on a map and right-click it, then choose Move to stash or Copy to stash.",
                    );

                    return;
                };

                let strip_width = last.x + last.width;
                let strip_height = entries.iter().map(|entry| entry.height).fold(0.0, f32::max);
                let origin = ui.cursor_screen_pos();
                let (size, extent) = panel_extent(ui.content_region_avail());
                ui.dummy(size);
                let canvas = OverlayRect {
                    min: origin,
                    max: [origin[0] + size[0], origin[1] + size[1]],
                };

                let mut camera = mem::take(&mut self.stash_view.camera);
                camera.resize(extent.0, extent.1);
                let mouse = ui.io().mouse_pos();
                let is_hovered = ui.is_window_hovered() && canvas.contains(mouse);
                if is_hovered && !ui.io().want_text_input() {
                    let is_drag_panning = settings.keybindings.get(KeybindAction::PanDrag).is_held(ui);
                    let center = [extent.0 as f32 * 0.5, extent.1 as f32 * 0.5];
                    self.pan_and_zoom_camera(ui, settings, &mut camera, origin, center, is_drag_panning);
                    let mut refit = mem::take(&mut self.stash_view.refit);
                    self.apply_view_shortcuts(ui, session, settings, &mut refit);
                    self.stash_view.refit = refit;
                }

                if mem::take(&mut self.stash_view.refit) {
                    camera.frame_map(strip_width, strip_height);
                }

                let shared = camera.camera;
                self.stash_view.camera = camera;

                let is_clicked = ui.is_mouse_clicked(MouseButton::Left) || ui.is_mouse_clicked(MouseButton::Right);
                for entry in entries {
                    let top_left = self.stash_view.camera.map_to_screen([entry.x, entry.height]);
                    let bottom_right = self.stash_view.camera.map_to_screen([entry.x + entry.width, 0.0]);
                    let min = [top_left[0].max(0.0), top_left[1].max(0.0)];
                    let max = [bottom_right[0].min(size[0]), bottom_right[1].min(size[1])];
                    if max[0] - min[0] < 1.0 || max[1] - min[1] < 1.0 {
                        continue;
                    }

                    let center = self
                        .stash_view
                        .camera
                        .screen_to_map([(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5]);
                    let area = OverlayRect {
                        min: [origin[0] + min[0], origin[1] + min[1]],
                        max: [origin[0] + max[0], origin[1] + max[1]],
                    };

                    if is_hovered && is_clicked && area.contains(mouse) && session.state.active() != Some(entry.id) {
                        self.cancel_edit_gestures(session, session.state.active());
                        session.set_active_document(entry.id);
                    }

                    if !self.map_views.contains_key(&entry.id)
                        && let Ok(view) = MapViewState::new(entry.id)
                    {
                        self.map_views.insert(entry.id, view);
                    }

                    if let Some(view) = self.map_views.get_mut(&entry.id) {
                        view.refit = false;
                        view.camera.camera.x = center[0] - entry.x;
                        view.camera.camera.y = center[1];
                        view.camera.camera.zoom = shared.zoom;
                    }

                    self.draw_document_view(
                        ui,
                        session,
                        settings,
                        entry.id,
                        false,
                        ViewKind::StashEntry(area),
                        ViewOutput {
                            visible: &mut *visible,
                            coop: &mut *coop,
                        },
                    );
                }
            });

        settings.panels.stash = is_open;
    }

    pub(crate) fn hide_stash(&mut self, session: &mut Session) {
        for id in session.stash_ids() {
            self.map_views.remove(&id);
        }

        if session.is_stash_active()
            && let Some(id) = session.state.document_ids().first().copied()
        {
            session.set_active_document(id);
        }
    }
}
