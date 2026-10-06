use dear_imgui_rs::{MouseButton, PopupQueryFlags, Ui};
use editor::document::DocumentId;

use super::{ViewFrame, selection::shows_block_selection};
use crate::{
    camera::clamp_zoom,
    pacing::FrameDemand,
    session::Session,
    ui::{
        UiState,
        coop::{
            CommentHit,
            comment_at,
            draw_comment_composer,
            draw_comments,
            draw_receiving,
            draw_remote_cursors,
            draw_remote_selections,
            draw_tab_out_of_date,
            draw_tab_progress,
        },
    },
};

const FOLLOW_SMOOTHING: f32 = 20.0;
// close enough to the followed camera to stop drawing at full rate
const FOLLOW_SETTLED_PIXELS: f32 = 0.5;
const FOLLOW_SETTLED_ZOOM: f32 = 0.001;

#[derive(Default)]
pub(in crate::ui) struct CoopPresence {
    pub(in crate::ui) cursor: Option<net::Cursor>,
    pub(in crate::ui) view: Option<net::View>,
    pub(in crate::ui) selection: Option<net::Selection>,
}

// where following left the camera, so a change while drawing means the user moved it
pub(in crate::ui) struct FollowedView {
    document: DocumentId,
    camera: [f32; 3],
    z: u32,
}

pub(super) fn draw_coop_status(ui: &Ui, session: &Session, id: DocumentId, name: &str, is_out_of_date: bool) -> bool {
    if let Some(state) = session.coop_transfer(id) {
        draw_tab_progress(ui, state);
    }

    if is_out_of_date {
        draw_tab_out_of_date(ui);
    }

    let Some((coop, state)) = session.coop().zip(session.coop_receiving(id)) else {
        return false;
    };

    draw_receiving(ui, coop, name, state);

    true
}

pub(super) fn draw_tab_menu(ui: &Ui, session: &mut Session, id: DocumentId) {
    if !session.can_share_coop_maps() {
        return;
    }

    let Some(_menu) = ui.begin_popup_context_item() else {
        return;
    };

    if ui.menu_item_enabled_selected_no_shortcut("Share map", false, session.can_share_coop_document(id)) {
        session.share_coop_document(id);
    }

    if ui.menu_item_enabled_selected_no_shortcut("Stop sharing", false, session.can_stop_sharing_coop_document(id)) {
        session.stop_sharing_coop_document(id);
    }
}

pub(super) fn hit_comment(ui: &Ui, session: &mut Session, frame: &ViewFrame<'_>) -> Option<CommentHit> {
    let hit = session
        .coop()
        .zip(session.coop_shared_map_path(frame.id))
        .filter(|_| frame.is_pointed)
        .and_then(|(coop, map)| {
            let z = session.state.document(frame.id)?.z;

            comment_at(ui, frame.camera, frame.layout.viewport.min, coop, &map, z, frame.mouse)
        });

    if let Some(hit) = hit
        && hit.delete
        && ui.is_mouse_clicked(MouseButton::Left)
    {
        session.delete_coop_comment(hit.id);
    }

    hit
}

pub(super) fn share_presence(session: &Session, frame: &ViewFrame<'_>, coop: &mut CoopPresence) {
    if !frame.is_active {
        return;
    }

    let Some(map) = session.coop_shared_map_path(frame.id) else {
        return;
    };

    let camera = &frame.camera;
    let pointer = frame
        .is_pointed
        .then_some(frame.mouse)
        .and_then(|point| frame.layout.local(point));
    coop.cursor = pointer.map(|cursor| net::Cursor {
        map: map.clone(),
        z: session.z(),
        pos: camera.screen_to_map(cursor),
        tool: session.tool().into(),
    });
    coop.selection = session
        .selection()
        .filter(|_| shows_block_selection(session.tool()))
        .map(|source| {
            let (selection, _) = frame.gestures.displayed(source);
            net::Selection {
                map: map.clone(),
                z: selection.min.z,
                min: [selection.min.x, selection.min.y],
                max: [selection.max.x, selection.max.y],
                mode: session.selection_mode().into(),
            }
        });
    coop.view = Some(net::View {
        map,
        z: session.z(),
        center: [camera.camera.x, camera.camera.y],
        zoom: camera.camera.zoom,
    });
}

impl UiState {
    pub(super) fn draw_coop_overlays(
        &mut self, ui: &Ui, session: &mut Session, frame: &ViewFrame<'_>, comment_hit: Option<CommentHit>,
    ) {
        if let Some(coop) = session.coop()
            && let Some(map) = session.coop_shared_map_path(frame.id)
            && let Some(z) = session.state.document(frame.id).map(|document| document.z)
        {
            let viewport = frame.layout.viewport;
            draw_remote_selections(ui, frame.camera, viewport, coop, &map, z, session.options.tile_size);
            draw_comments(ui, frame.camera, viewport, coop, &map, z, comment_hit);
            if draw_remote_cursors(ui, frame.camera, viewport, coop, &map, z) {
                self.raise_demand(FrameDemand::Full);
            }
        }

        if self
            .comment_draft
            .as_ref()
            .is_some_and(|draft| draft.document == frame.id)
        {
            draw_comment_composer(ui, session, &mut self.comment_draft);
        }
    }

    pub(in crate::ui) fn follow_coop_peer(&mut self, ui: &Ui, session: &mut Session) {
        // a dialog acts on the map it was opened for, so following waits until it closes
        if ui.is_popup_open_with_flags("", PopupQueryFlags::ANY_POPUP) {
            self.followed_view = None;
            return;
        }

        let before = session.state.active();
        let Some((document, target)) = session.coop_follow_target() else {
            self.followed_view = None;
            return;
        };

        let is_switched = before != Some(document);
        if is_switched {
            self.cancel_edit_gestures(session, before);
        }

        let Some(view) = self.map_view_mut(document) else {
            return;
        };

        let zoom = clamp_zoom(target.zoom);
        let camera = &mut view.camera.camera;
        if is_switched {
            [camera.x, camera.y] = target.center;
            camera.zoom = zoom;
            view.focus = true;
        } else {
            let blend = 1.0 - (-ui.io().delta_time() * FOLLOW_SMOOTHING).exp();
            camera.x += (target.center[0] - camera.x) * blend;
            camera.y += (target.center[1] - camera.y) * blend;
            camera.zoom = (camera.zoom.ln() + (zoom.ln() - camera.zoom.ln()) * blend).exp();
        }

        view.refit = false;

        let is_settled = (target.center[0] - camera.x).abs() < FOLLOW_SETTLED_PIXELS
            && (target.center[1] - camera.y).abs() < FOLLOW_SETTLED_PIXELS
            && (zoom.ln() - camera.zoom.ln()).abs() < FOLLOW_SETTLED_ZOOM;
        let camera = [camera.x, camera.y, camera.zoom];
        self.followed_view = session.state.document(document).map(|followed| FollowedView {
            document,
            camera,
            z: followed.z,
        });

        if !is_settled {
            self.raise_demand(FrameDemand::Full);
        }
    }

    pub(in crate::ui) fn stop_following_when_moved(&mut self, session: &mut Session) {
        let Some(followed) = self.followed_view.take() else {
            return;
        };

        let is_moved = self.map_views.get(&followed.document).is_some_and(|view| {
            let camera = view.camera.camera;
            [camera.x, camera.y, camera.zoom] != followed.camera
        }) || session
            .state
            .document(followed.document)
            .is_some_and(|document| document.z != followed.z);

        if is_moved {
            session.stop_coop_follow();
        }
    }
}
