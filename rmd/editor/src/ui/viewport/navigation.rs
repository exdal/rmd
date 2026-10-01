use std::collections::hash_map::Entry;

use dear_imgui_rs::{MouseButton, Ui};
use dmm::Coord;
use editor::document::DocumentId;

use super::{MapViewState, ViewFrame, stroke::ActivePlacementFlash};
use crate::{
    session::Session,
    settings::{KeybindAction, Settings},
    ui::{UiState, common::dpi, find::JumpTarget},
};

const KEY_PAN_SPEED: f32 = 800.0;
const KEY_PAN_FASTER: f32 = 3.0;

const PAN_KEYS: [(KeybindAction, [f32; 2]); 4] = [
    (KeybindAction::PanLeft, [1.0, 0.0]),
    (KeybindAction::PanRight, [-1.0, 0.0]),
    (KeybindAction::PanUp, [0.0, 1.0]),
    (KeybindAction::PanDown, [0.0, -1.0]),
];

impl UiState {
    pub(super) fn pan_and_zoom(&self, ui: &Ui, settings: &Settings, frame: &mut ViewFrame<'_>, is_drag_panning: bool) {
        let io = ui.io();
        let camera = &mut *frame.camera;
        if !self.gizmo.is_interacting()
            && (ui.is_mouse_down(MouseButton::Middle) || is_drag_panning && ui.is_mouse_down(MouseButton::Left))
        {
            camera.pan_by(io.mouse_delta());
        }

        let wheel = io.mouse_wheel();
        if !self.gizmo.is_interacting() && wheel != 0.0 {
            let mouse = io.mouse_pos();
            let min = frame.layout.viewport.min;
            camera.zoom_by(wheel, [mouse[0] - min[0], mouse[1] - min[1]]);
        }

        let faster = settings.keybindings.get(KeybindAction::PanFaster);
        let speed = KEY_PAN_SPEED * dpi(ui) * io.delta_time() * if faster.is_held(ui) { KEY_PAN_FASTER } else { 1.0 };
        let pan = PAN_KEYS
            .into_iter()
            .filter(|(action, _)| settings.keybindings.get(*action).is_down_with(ui, faster))
            .fold([0.0; 2], |pan, (_, direction)| {
                [pan[0] + direction[0] * speed, pan[1] + direction[1] * speed]
            });
        if pan != [0.0; 2] {
            camera.pan_by(pan);
        }

        let center = frame.layout.center();
        if settings.keybindings.get(KeybindAction::ZoomIn).is_pressed_repeating(ui) {
            camera.zoom_by(1.0, center);
        }

        if settings
            .keybindings
            .get(KeybindAction::ZoomOut)
            .is_pressed_repeating(ui)
        {
            camera.zoom_by(-1.0, center);
        }
    }

    pub(super) fn mirror_active_camera(&mut self, session: &mut Session) {
        let Some(active) = session.state.active() else {
            return;
        };
        let Some(camera) = self.map_views.get(&active).map(|view| view.camera.camera) else {
            return;
        };
        let z = session.z();
        for (id, view) in &mut self.map_views {
            if *id != active {
                view.camera.mirror(&camera);
                session.set_level_of(*id, z);
            }
        }
    }

    pub(in crate::ui) fn jump_to_instance(&mut self, session: &mut Session, target: JumpTarget) {
        let Some(location) = session
            .state
            .document(target.document)
            .and_then(|document| document.instance_location(target.instance))
        else {
            return;
        };

        self.cancel_edit_gestures(session, session.state.active());
        self.center_view_on(session, target.document, location.coord);
        if let Some(document) = session.state.active_document_mut() {
            document.set_focus(None);
            document.selection = None;
        }

        session.select_instance(Some(target.instance));
    }

    pub(in crate::ui) fn go_to_tile(&mut self, session: &mut Session, document: DocumentId, coord: Coord, now: f64) {
        self.cancel_edit_gestures(session, session.state.active());
        self.center_view_on(session, document, coord);
        self.placement_flash = session.turf_at(document, coord).map(|owner| ActivePlacementFlash {
            owner,
            coord,
            started_at: now,
        });
    }

    pub(in crate::ui) fn center_view_on(&mut self, session: &mut Session, document: DocumentId, coord: Coord) {
        session.set_active_document(document);
        session.set_level(coord.z);

        let Some(view) = self.map_view_mut(document) else {
            return;
        };

        view.camera.center_on_tile(coord, session.options.tile_size);
        view.refit = false;
        view.focus = true;
    }

    pub(super) fn map_view_mut(&mut self, document: DocumentId) -> Option<&mut MapViewState> {
        match self.map_views.entry(document) {
            Entry::Occupied(entry) => Some(entry.into_mut()),
            Entry::Vacant(entry) => match MapViewState::new(document) {
                Ok(view) => Some(entry.insert(view)),
                Err(e) => {
                    log::error!(
                        "could not create a map view window for document {}: {e}",
                        document.get()
                    );

                    None
                },
            },
        }
    }
}
