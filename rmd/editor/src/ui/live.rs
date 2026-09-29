use dear_imgui_rs::{Key, Ui};
use editor::{document::DocumentId, icons::materialdesignicons::ICON_CLOSE};
use net::{Comment, CommentId, MAX_COMMENT_LEN, PeerId};

use super::{
    DIAGNOSTIC_WARNING_COLOR,
    common::dpi,
    dialog::{DIALOG_FIELD_WIDTH, MODAL_FLAGS, SAVE_ERROR_COLOR},
    overlay::{OVERLAY_BG, OverlayRect},
};
use crate::{
    camera::Controller,
    session::{LiveShare, LiveStatus, Session},
    settings::Settings,
};

pub(super) const HOST_POPUP: &str = "Host live share##live-host";

pub(super) const JOIN_POPUP: &str = "Join live share##live-join";

const LIVE_COLOR: [f32; 4] = [0.45, 0.85, 0.45, 1.0];

const PENDING_COLOR: [f32; 4] = [0.7, 0.7, 0.7, 1.0];

pub(super) const COMMENT_POPUP: &str = "##live-comment";

const COMMENT_WIDTH: f32 = 240.0;

const COMMENT_HEADER: f32 = 5.0;

const COMMENT_BG: [f32; 4] = [0.1, 0.1, 0.12, 0.94];

pub(super) struct CommentDraft {
    pub(super) document: DocumentId,
    pos: [f32; 2],
    text: String,
}

impl CommentDraft {
    pub(super) const fn new(document: DocumentId, pos: [f32; 2]) -> Self {
        Self {
            document,
            pos,
            text: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LiveDialogKind {
    Host,
    Join,
}

pub(super) struct LiveDialog {
    kind: LiveDialogKind,
    nick: String,
    address: String,
    port: i32,
    password: String,
    error: Option<String>,
}

impl LiveDialog {
    pub(super) fn new(kind: LiveDialogKind, settings: &Settings) -> Self {
        let nick = Some(settings.live_share.nick.clone())
            .filter(|nick| !nick.is_empty())
            .or_else(|| std::env::var("USER").ok())
            .or_else(|| std::env::var("USERNAME").ok())
            .unwrap_or_default();

        Self {
            kind,
            nick,
            address: settings.live_share.address.clone(),
            port: i32::from(settings.live_share.port),
            password: String::new(),
            error: None,
        }
    }

    pub(super) const fn popup(&self) -> &'static str {
        match self.kind {
            LiveDialogKind::Host => HOST_POPUP,
            LiveDialogKind::Join => JOIN_POPUP,
        }
    }
}

pub(super) fn draw_live_dialog(
    ui: &Ui, session: &mut Session, settings: &mut Settings, dialog: &mut Option<LiveDialog>,
) {
    let mut close = false;

    if let Some(state) = dialog.as_mut()
        && let Some(_modal) = ui.begin_modal_popup_config(state.popup()).flags(MODAL_FLAGS).begin()
    {
        let width = DIALOG_FIELD_WIDTH * dpi(ui);
        let mut submitted = false;

        ui.text("Nick");
        ui.set_next_item_width(width);
        if ui.is_window_appearing() {
            ui.set_keyboard_focus_here();
        }

        submitted |= ui
            .input_text("##live-nick", &mut state.nick)
            .enter_returns_true(true)
            .build();
        match state.kind {
            LiveDialogKind::Host => {
                ui.text("Port");
                ui.set_next_item_width(width);
                ui.input_int("##live-port", &mut state.port);
                state.port = state.port.clamp(1, i32::from(u16::MAX));
            },
            LiveDialogKind::Join => {
                ui.text("Address");
                ui.set_next_item_width(width);
                submitted |= ui
                    .input_text("##live-address", &mut state.address)
                    .hint("host:port")
                    .enter_returns_true(true)
                    .build();
            },
        }

        ui.text("Password");
        ui.set_next_item_width(width);
        submitted |= ui
            .input_text("##live-password", &mut state.password)
            .enter_returns_true(true)
            .build();

        if let Some(error) = state.error.as_deref() {
            ui.text_colored(SAVE_ERROR_COLOR, error);
        }

        ui.separator();
        if ui.button("Cancel") || ui.is_key_pressed(Key::Escape) {
            close = true;
            ui.close_current_popup();
        }

        let nick = state.nick.trim();
        let address = state.address.trim();
        let password = state.password.trim();
        let ready =
            !nick.is_empty() && !password.is_empty() && (state.kind == LiveDialogKind::Host || !address.is_empty());

        ui.same_line();
        let label = match state.kind {
            LiveDialogKind::Host => "Host",
            LiveDialogKind::Join => "Join",
        };

        let clicked = {
            let _disabled = ui.begin_disabled_with_cond(!ready);
            ui.button(label)
        };

        if ready && (clicked || submitted) {
            let result = match state.kind {
                LiveDialogKind::Host => session.host_live(state.port as u16, password.to_owned(), nick.to_owned()),
                LiveDialogKind::Join => session.join_live(address.to_owned(), password.to_owned(), nick.to_owned()),
            };

            match result {
                Ok(()) => {
                    settings.live_share.nick = nick.to_owned();
                    match state.kind {
                        LiveDialogKind::Host => settings.live_share.port = state.port as u16,
                        LiveDialogKind::Join => settings.live_share.address = address.to_owned(),
                    }

                    settings.save();
                    close = true;
                    ui.close_current_popup();
                },
                Err(error) => state.error = Some(error),
            }
        }
    }

    if close {
        *dialog = None;
    }
}

pub(super) fn draw_live_status(ui: &Ui, live: &LiveShare) {
    let (color, label) = match &live.status {
        LiveStatus::Hashing => (PENDING_COLOR, String::from("Live: reading codebase...")),
        LiveStatus::Connecting => (PENDING_COLOR, String::from("Live: connecting...")),
        LiveStatus::Connected => {
            let others = live.peers.len();
            let color = if live.peers.values().all(|peer| live.same_codebase(&peer.info)) {
                LIVE_COLOR
            } else {
                DIAGNOSTIC_WARNING_COLOR
            };

            (
                color,
                format!("Live: {} {}", others + 1, if others == 0 { "user" } else { "users" }),
            )
        },
        LiveStatus::Ended(reason) => (SAVE_ERROR_COLOR, format!("Live: {reason}")),
    };

    ui.text_colored(color, label);
    if !ui.is_item_hovered() {
        return;
    }

    ui.tooltip(|| {
        if let Some((addr, password)) = live.host() {
            ui.text(format!("Hosting on port {} with password {password}", addr.port()));
            ui.separator();
        }

        ui.text(format!("{} (you)", live.nick));
        for peer in live.peers.values() {
            if live.same_codebase(&peer.info) {
                ui.text_colored(peer_color(peer.info.id), &peer.info.nick);
            } else {
                let theirs = peer.info.codebase.git_hint.as_deref().unwrap_or("no git");
                let ours = live.git_hint().unwrap_or("no git");
                ui.text_colored(
                    DIAGNOSTIC_WARNING_COLOR,
                    format!("{}: different codebase ({theirs}, you are on {ours})", peer.info.nick),
                );
            }
        }
    });
}

pub(super) fn draw_remote_cursors(
    ui: &Ui, camera: &Controller, viewport: OverlayRect, live: &LiveShare, map: &str, z: u32,
) {
    let scale = dpi(ui);
    let viewport_min = viewport.min;
    let draw = ui.get_window_draw_list();
    draw.with_clip_rect(viewport.min, viewport.max, || {
        for peer in live.peers.values() {
            if !peer
                .cursor
                .as_ref()
                .is_some_and(|cursor| cursor.map == map && cursor.z == z)
            {
                continue;
            }

            let local = camera.map_to_screen(peer.shown);
            if !local.iter().all(|value| value.is_finite()) {
                continue;
            }

            let color = peer_color(peer.info.id);
            let tip = [viewport_min[0] + local[0], viewport_min[1] + local[1]];
            let bottom = [tip[0], tip[1] + 16.0 * scale];
            let side = [tip[0] + 11.0 * scale, tip[1] + 11.0 * scale];
            draw.add_triangle(tip, bottom, side, color).filled(true).build();
            draw.add_triangle(tip, bottom, side, [0.0, 0.0, 0.0, 1.0]).build();

            let text_size = ui.calc_text_size(&peer.info.nick);
            let min = [side[0] + 2.0 * scale, side[1]];
            let max = [min[0] + text_size[0] + 6.0, min[1] + text_size[1] + 4.0];
            draw.add_rect(min, max, OVERLAY_BG).filled(true).build();
            draw.add_rect(min, max, color).build();
            draw.add_text([min[0] + 3.0, min[1] + 2.0], color, &peer.info.nick);
        }
    });
}

pub(super) fn draw_comment_composer(ui: &Ui, session: &Session, draft: &mut Option<CommentDraft>) {
    let Some(state) = draft.as_mut() else {
        return;
    };

    let Some(_popup) = ui.begin_popup(COMMENT_POPUP) else {
        *draft = None;
        return;
    };

    if ui.is_window_appearing() {
        ui.set_keyboard_focus_here();
    }

    ui.set_next_item_width(COMMENT_WIDTH * dpi(ui));
    let submitted = ui
        .input_text("##comment-text", &mut state.text)
        .hint("Add a comment")
        .enter_returns_true(true)
        .build();

    let text = state.text.trim();
    if submitted && !text.is_empty() {
        session.add_live_comment(state.document, state.pos, text.chars().take(MAX_COMMENT_LEN).collect());
    }

    if submitted || ui.is_key_pressed(Key::Escape) {
        ui.close_current_popup();
        *draft = None;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CommentHit {
    pub(super) id: CommentId,
    pub(super) delete: bool,
}

struct CommentLayout<'a> {
    comment: &'a Comment,
    // the author may have left
    nick: Option<&'a str>,
    min: [f32; 2],
    max: [f32; 2],
    row: [f32; 2],
    text: [f32; 2],
    delete: [f32; 2],
    delete_size: [f32; 2],
}

impl CommentLayout<'_> {
    fn contains(&self, point: [f32; 2]) -> bool {
        (self.min[0]..=self.max[0]).contains(&point[0]) && (self.min[1]..=self.max[1]).contains(&point[1])
    }

    fn delete_contains(&self, point: [f32; 2]) -> bool {
        let max = [
            self.delete[0] + self.delete_size[0],
            self.delete[1] + self.delete_size[1],
        ];

        (self.delete[0]..=max[0]).contains(&point[0]) && (self.delete[1]..=max[1]).contains(&point[1])
    }
}

fn comment_layouts<'a>(
    ui: &'a Ui, camera: &'a Controller, viewport_min: [f32; 2], live: &'a LiveShare, map: &'a str, z: u32,
) -> impl Iterator<Item = CommentLayout<'a>> + 'a {
    let scale = dpi(ui);
    let padding = 6.0 * scale;
    let wrap = COMMENT_WIDTH * scale;
    let header = COMMENT_HEADER * scale;
    let delete_size = ui.calc_text_size(ICON_CLOSE.to_string());

    live.comments
        .values()
        .filter(move |comment| comment.map == map && comment.z == z)
        .filter_map(move |comment| {
            let local = camera.map_to_screen(comment.pos);
            if !local.iter().all(|value| value.is_finite()) {
                return None;
            }

            let nick = live.nick_of(comment.author);
            let nick_width = nick.map_or(0.0, |nick| ui.calc_text_size(nick)[0]);
            let text_size = ui.calc_text_size_with_opts(&comment.text, false, wrap);
            let width = (nick_width + padding + delete_size[0]).max(text_size[0]);
            let min = [viewport_min[0] + local[0], viewport_min[1] + local[1]];
            let max = [
                min[0] + width + padding * 2.0,
                min[1] + header + delete_size[1] + text_size[1] + padding * 2.5,
            ];

            let row = [min[0] + padding, min[1] + header + padding];
            Some(CommentLayout {
                comment,
                nick,
                min,
                max,
                row,
                text: [row[0], row[1] + delete_size[1] + padding * 0.5],
                delete: [max[0] - padding - delete_size[0], row[1]],
                delete_size,
            })
        })
}

pub(super) fn comment_at(
    ui: &Ui, camera: &Controller, viewport_min: [f32; 2], live: &LiveShare, map: &str, z: u32, point: [f32; 2],
) -> Option<CommentHit> {
    comment_layouts(ui, camera, viewport_min, live, map, z)
        .filter(|layout| layout.contains(point))
        .last()
        .map(|layout| CommentHit {
            id: layout.comment.id,
            delete: layout.delete_contains(point),
        })
}

pub(super) fn draw_comments(
    ui: &Ui, camera: &Controller, viewport: OverlayRect, live: &LiveShare, map: &str, z: u32,
    hovered: Option<CommentHit>,
) {
    let header = COMMENT_HEADER * dpi(ui);
    let wrap = COMMENT_WIDTH * dpi(ui);
    let draw = ui.get_window_draw_list();
    draw.with_clip_rect(viewport.min, viewport.max, || {
        for layout in comment_layouts(ui, camera, viewport.min, live, map, z) {
            let color = peer_color(layout.comment.author);
            draw.add_rect(layout.min, layout.max, COMMENT_BG).filled(true).build();
            draw.add_rect(layout.min, [layout.max[0], layout.min[1] + header], color)
                .filled(true)
                .build();

            if let Some(nick) = layout.nick {
                draw.add_text(layout.row, color, nick);
            }

            if let Some(hit) = hovered.filter(|hit| hit.id == layout.comment.id) {
                let icon = if hit.delete { SAVE_ERROR_COLOR } else { PENDING_COLOR };
                draw.add_text(layout.delete, icon, ICON_CLOSE.to_string());
            }

            draw.add_text_with_font(
                ui.current_font(),
                ui.current_font_size(),
                layout.text,
                [1.0; 4],
                &layout.comment.text,
                wrap,
                None,
            );
        }
    });
}

fn peer_color(id: PeerId) -> [f32; 4] {
    let hue = (id.0 as f32 * 0.618_034).fract() * 6.0;
    let x = 1.0 - (hue % 2.0 - 1.0).abs();
    let [r, g, b] = match hue as u32 {
        0 => [1.0, x, 0.0],
        1 => [x, 1.0, 0.0],
        2 => [0.0, 1.0, x],
        3 => [0.0, x, 1.0],
        4 => [x, 0.0, 1.0],
        _ => [1.0, 0.0, x],
    };

    let tint = |channel: f32| 0.35 + channel * 0.65;
    [tint(r), tint(g), tint(b), 1.0]
}
