use std::env;

use dear_imgui_rs::{Key, StyleColor, Ui, sys};
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
    session::{Coop, CoopStatus, Session, SharedState},
    settings::Settings,
};

pub(super) const HOST_POPUP: &str = "Host co-op##coop-host";

pub(super) const JOIN_POPUP: &str = "Join co-op##coop-join";

const CONNECTED_COLOR: [f32; 4] = [0.45, 0.85, 0.45, 1.0];

const PENDING_COLOR: [f32; 4] = [0.7, 0.7, 0.7, 1.0];

const RECEIVING_WIDTH: f32 = 280.0;

const TAB_FILL_ALPHA: f32 = 0.45;

const TAB_SWEEP_SPEED: f32 = 0.6;

const TAB_SWEEP_WIDTH: f32 = 0.3;

pub(super) const COMMENT_POPUP: &str = "##coop-comment";

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
pub(super) enum CoopDialogKind {
    Host,
    Join,
}

pub(super) struct CoopDialog {
    kind: CoopDialogKind,
    nick: String,
    address: String,
    port: i32,
    password: String,
    error: Option<String>,
}

impl CoopDialog {
    pub(super) fn new(kind: CoopDialogKind, settings: &Settings) -> Self {
        let nick = Some(settings.coop.nick.clone())
            .filter(|nick| !nick.is_empty())
            .or_else(|| env::var("USER").ok())
            .or_else(|| env::var("USERNAME").ok())
            .unwrap_or_default();

        Self {
            kind,
            nick,
            address: settings.coop.address.clone(),
            port: i32::from(settings.coop.port),
            password: String::new(),
            error: None,
        }
    }

    pub(super) const fn popup(&self) -> &'static str {
        match self.kind {
            CoopDialogKind::Host => HOST_POPUP,
            CoopDialogKind::Join => JOIN_POPUP,
        }
    }
}

pub(super) fn draw_coop_dialog(
    ui: &Ui, session: &mut Session, settings: &mut Settings, dialog: &mut Option<CoopDialog>,
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
            .input_text("##coop-nick", &mut state.nick)
            .enter_returns_true(true)
            .build();
        match state.kind {
            CoopDialogKind::Host => {
                ui.text("Port");
                ui.set_next_item_width(width);
                ui.input_int("##coop-port", &mut state.port);
                state.port = state.port.clamp(1, i32::from(u16::MAX));
            },
            CoopDialogKind::Join => {
                ui.text("Address");
                ui.set_next_item_width(width);
                submitted |= ui
                    .input_text("##coop-address", &mut state.address)
                    .hint("host:port")
                    .enter_returns_true(true)
                    .build();
            },
        }

        ui.text("Password");
        ui.set_next_item_width(width);
        submitted |= ui
            .input_text("##coop-password", &mut state.password)
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
            !nick.is_empty() && !password.is_empty() && (state.kind == CoopDialogKind::Host || !address.is_empty());

        ui.same_line();
        let label = match state.kind {
            CoopDialogKind::Host => "Host",
            CoopDialogKind::Join => "Join",
        };

        let clicked = {
            let _disabled = ui.begin_disabled_with_cond(!ready);
            ui.button(label)
        };

        if ready && (clicked || submitted) {
            let result = match state.kind {
                CoopDialogKind::Host => session.host_coop(state.port as u16, password.to_owned(), nick.to_owned()),
                CoopDialogKind::Join => session.join_coop(address.to_owned(), password.to_owned(), nick.to_owned()),
            };

            match result {
                Ok(()) => {
                    settings.coop.nick = nick.to_owned();
                    match state.kind {
                        CoopDialogKind::Host => settings.coop.port = state.port as u16,
                        CoopDialogKind::Join => settings.coop.address = address.to_owned(),
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

pub(super) fn shared_maps(coop: &Coop) -> Vec<(String, String)> {
    coop.shared_maps
        .keys()
        .map(|path| (path.clone(), map_note(coop, path)))
        .collect()
}

pub(super) fn map_note(coop: &Coop, path: &str) -> String {
    let Some(shared_map) = coop.shared_maps.get(path) else {
        return String::new();
    };

    let note = match &shared_map.state {
        SharedState::Incoming { by, .. } => match coop.nick_of(*by) {
            Some(nick) => format!("uploading from {nick}"),
            None => String::from("uploading"),
        },
        state @ SharedState::Sending { .. } => format!("sharing {}", percent(state)),
        state @ SharedState::Receiving { .. } => format!("receiving {}", percent(state)),
        SharedState::Loading(_) => String::from("loading"),
        SharedState::Closed => String::from("closed"),
        SharedState::Waiting(_) => String::from("waiting"),
        SharedState::Ready => return String::new(),
    };

    format!("({note})")
}

fn fraction(state: &SharedState) -> Option<f32> {
    match *state {
        SharedState::Sending { done, total } | SharedState::Receiving { done, total } if total > 0 => {
            Some(done as f32 / total as f32)
        },
        _ => None,
    }
}

fn percent(state: &SharedState) -> String {
    fraction(state).map_or_else(String::new, |fraction| format!("{:.0}%", fraction * 100.0))
}

fn megabytes(bytes: u64) -> String { format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0)) }

fn transfer_line(coop: &Coop, path: &str, state: &SharedState) -> Option<String> {
    let line = match *state {
        SharedState::Incoming { by, len, .. } => {
            let nick = coop.nick_of(by).unwrap_or("someone");
            format!("Waiting for {nick} to upload {path} ({})", megabytes(len))
        },
        SharedState::Sending { total: 0, .. } => format!("Sharing {path}: preparing"),
        SharedState::Sending { done, total } => format!("Sharing {path}: {} / {}", megabytes(done), megabytes(total)),
        SharedState::Receiving { done, total } => {
            format!("Receiving {path}: {} / {}", megabytes(done), megabytes(total))
        },
        SharedState::Loading(_) => format!("Loading {path}"),
        SharedState::Ready | SharedState::Closed | SharedState::Waiting(_) => return None,
    };

    Some(line)
}

pub(super) fn draw_tab_progress(ui: &Ui, state: &SharedState) {
    let (min, max) = unsafe {
        let window = sys::igGetCurrentWindow();
        if (*window).DockTabIsVisible() {
            let rect = (*window).DC.DockTabItemRect;
            ([rect.Min.x, rect.Min.y], [rect.Max.x, rect.Max.y])
        } else {
            let rect = sys::ImGuiWindow_TitleBarRect(window);
            ([rect.Min.x, rect.Min.y], [rect.Max.x, rect.Max.y])
        }
    };

    let width = max[0] - min[0];
    let (from, to) = match fraction(state) {
        Some(fraction) => (0.0, fraction),
        None => {
            let start = (ui.time() as f32 * TAB_SWEEP_SPEED).fract() * (1.0 + TAB_SWEEP_WIDTH) - TAB_SWEEP_WIDTH;
            (start.max(0.0), (start + TAB_SWEEP_WIDTH).min(1.0))
        },
    };

    let mut color = ui.style_color(StyleColor::PlotHistogram);
    color[3] *= TAB_FILL_ALPHA;
    let draw = ui.get_window_draw_list();
    draw.with_clip_rect(min, max, || {
        draw.add_rect([min[0] + width * from, min[1]], [min[0] + width * to, max[1]], color)
            .filled(true)
            .build();
    });
}

pub(super) fn draw_receiving(ui: &Ui, coop: &Coop, name: &str, state: &SharedState) {
    let (text, overlay) = match *state {
        SharedState::Incoming { by, len, .. } => (
            format!("Waiting for {} to upload {name}", coop.nick_of(by).unwrap_or("someone")),
            megabytes(len),
        ),
        SharedState::Receiving { done, total } | SharedState::Sending { done, total } => (
            format!("Receiving {name}"),
            format!("{} / {}", megabytes(done), megabytes(total)),
        ),
        _ => (format!("Loading {name}"), String::new()),
    };

    let width = RECEIVING_WIDTH * dpi(ui);
    let avail = ui.content_region_avail();
    let height = ui.text_line_height_with_spacing() + ui.frame_height();
    let origin = ui.cursor_pos();
    let left = origin[0] + ((avail[0] - width) / 2.0).max(0.0);
    ui.set_cursor_pos([left, origin[1] + ((avail[1] - height) / 2.0).max(0.0)]);
    ui.text(&text);
    ui.set_cursor_pos([left, ui.cursor_pos()[1]]);
    ui.progress_bar(fraction(state).unwrap_or(-(ui.time() as f32)))
        .size([width, 0.0])
        .overlay_text(overlay)
        .build();
}

pub(super) fn draw_coop_status(ui: &Ui, coop: &Coop) {
    let (color, label) = match &coop.status {
        CoopStatus::Hashing => (PENDING_COLOR, String::from("Co-op: reading codebase...")),
        CoopStatus::Connecting => (PENDING_COLOR, String::from("Co-op: connecting...")),
        CoopStatus::Connected => {
            let others = coop.peers.len();
            let is_waiting = coop
                .shared_maps
                .values()
                .any(|shared_map| matches!(shared_map.state, SharedState::Waiting(_)));
            let color = if !is_waiting && coop.peers.values().all(|peer| coop.same_codebase(&peer.info)) {
                CONNECTED_COLOR
            } else {
                DIAGNOSTIC_WARNING_COLOR
            };

            (
                color,
                format!("Co-op: {} {}", others + 1, if others == 0 { "user" } else { "users" }),
            )
        },
        CoopStatus::Ended(reason) => (SAVE_ERROR_COLOR, format!("Co-op: {reason}")),
    };

    ui.text_colored(color, label);
    if !ui.is_item_hovered() {
        return;
    }

    ui.tooltip(|| {
        if let Some((addr, password)) = coop.host() {
            ui.text(format!("Hosting on port {} with password {password}", addr.port()));
            ui.separator();
        }

        ui.text(format!("{} (you)", coop.nick));
        for peer in coop.peers.values() {
            if coop.same_codebase(&peer.info) {
                ui.text_colored(peer_color(peer.info.id), &peer.info.nick);
            } else {
                let theirs = peer.info.codebase.git_hint.as_deref().unwrap_or("no git");
                let ours = coop.git_hint().unwrap_or("no git");
                ui.text_colored(
                    DIAGNOSTIC_WARNING_COLOR,
                    format!("{}: different codebase ({theirs}, you are on {ours})", peer.info.nick),
                );
            }
        }

        if !coop.shared_maps.is_empty() {
            ui.separator();
            ui.text("Shared maps");
            for (path, shared_map) in &coop.shared_maps {
                let note = map_note(coop, path);
                match coop.nick_of(shared_map.by) {
                    Some(nick) => ui.text(format!("{path} (from {nick}) {note}")),
                    None => ui.text(format!("{path}{note}")),
                }
            }
        }

        let transfers = coop
            .shared_maps
            .iter()
            .filter_map(|(path, shared_map)| transfer_line(coop, path, &shared_map.state))
            .collect::<Vec<_>>();
        if !transfers.is_empty() {
            ui.separator();
            ui.text("Transfers");
            for line in transfers {
                ui.text(line);
            }
        }

        for (path, shared_map) in &coop.shared_maps {
            if matches!(shared_map.state, SharedState::Waiting(_)) {
                ui.text_colored(
                    DIAGNOSTIC_WARNING_COLOR,
                    format!("{path}: save or close your copy to receive the shared one"),
                );
            }
        }
    });
}

pub(super) fn draw_remote_cursors(ui: &Ui, camera: &Controller, viewport: OverlayRect, coop: &Coop, map: &str, z: u32) {
    let scale = dpi(ui);
    let viewport_min = viewport.min;
    let draw = ui.get_window_draw_list();
    draw.with_clip_rect(viewport.min, viewport.max, || {
        for peer in coop.peers.values() {
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
        session.add_coop_comment(state.document, state.pos, text.chars().take(MAX_COMMENT_LEN).collect());
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
    ui: &'a Ui, camera: &'a Controller, viewport_min: [f32; 2], coop: &'a Coop, map: &'a str, z: u32,
) -> impl Iterator<Item = CommentLayout<'a>> + 'a {
    let scale = dpi(ui);
    let padding = 6.0 * scale;
    let wrap = COMMENT_WIDTH * scale;
    let header = COMMENT_HEADER * scale;
    let delete_size = ui.calc_text_size(ICON_CLOSE.to_string());

    coop.comments
        .values()
        .filter(move |comment| comment.map == map && comment.z == z)
        .filter_map(move |comment| {
            let local = camera.map_to_screen(comment.pos);
            if !local.iter().all(|value| value.is_finite()) {
                return None;
            }

            let nick = coop.nick_of(comment.author);
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
    ui: &Ui, camera: &Controller, viewport_min: [f32; 2], coop: &Coop, map: &str, z: u32, point: [f32; 2],
) -> Option<CommentHit> {
    comment_layouts(ui, camera, viewport_min, coop, map, z)
        .filter(|layout| layout.contains(point))
        .last()
        .map(|layout| CommentHit {
            id: layout.comment.id,
            delete: layout.delete_contains(point),
        })
}

pub(super) fn draw_comments(
    ui: &Ui, camera: &Controller, viewport: OverlayRect, coop: &Coop, map: &str, z: u32, hovered: Option<CommentHit>,
) {
    let header = COMMENT_HEADER * dpi(ui);
    let wrap = COMMENT_WIDTH * dpi(ui);
    let draw = ui.get_window_draw_list();
    draw.with_clip_rect(viewport.min, viewport.max, || {
        for layout in comment_layouts(ui, camera, viewport.min, coop, map, z) {
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
