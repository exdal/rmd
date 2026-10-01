use std::{env, iter};

use dear_imgui_rs::{Key, PopupQueryFlags, StyleColor, Ui, sys};
use editor::{document::DocumentId, icons::materialdesignicons::ICON_CLOSE};
use net::{CodebaseId, Comment, CommentId, MAX_COMMENT_LEN, PeerId};

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

pub(super) const STATUS_POPUP: &str = "Co-op unavailable##coop-status";

pub(super) const OUT_OF_DATE_POPUP: &str = "Map out of date##coop-out-of-date";

const NICK_PLACEHOLDER: &str = "someone";

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
        let is_ready =
            !nick.is_empty() && !password.is_empty() && (state.kind == CoopDialogKind::Host || !address.is_empty());

        ui.same_line();
        let label = match state.kind {
            CoopDialogKind::Host => "Host",
            CoopDialogKind::Join => "Join",
        };

        let clicked = {
            let _disabled = ui.begin_disabled_with_cond(!is_ready);
            ui.button(label)
        };

        if is_ready && (clicked || submitted) {
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
        SharedState::Waiting(_) => String::from("out of date"),
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
            let nick = coop.nick_of(by).unwrap_or(NICK_PLACEHOLDER);
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

fn tab_rect() -> ([f32; 2], [f32; 2]) {
    unsafe {
        let window = sys::igGetCurrentWindow();
        if (*window).DockTabIsVisible() {
            let rect = (*window).DC.DockTabItemRect;
            ([rect.Min.x, rect.Min.y], [rect.Max.x, rect.Max.y])
        } else {
            let rect = sys::ImGuiWindow_TitleBarRect(window);
            ([rect.Min.x, rect.Min.y], [rect.Max.x, rect.Max.y])
        }
    }
}

fn fill_tab(ui: &Ui, min: [f32; 2], max: [f32; 2], mut color: [f32; 4]) {
    color[3] *= TAB_FILL_ALPHA;
    let draw = ui.get_window_draw_list();
    draw.with_clip_rect(min, max, || {
        draw.add_rect(min, max, color).filled(true).build();
    });
}

pub(super) fn draw_tab_progress(ui: &Ui, state: &SharedState) {
    let (min, max) = tab_rect();
    let width = max[0] - min[0];
    let (from, to) = match fraction(state) {
        Some(fraction) => (0.0, fraction),
        None => {
            let start = (ui.time() as f32 * TAB_SWEEP_SPEED).fract() * (1.0 + TAB_SWEEP_WIDTH) - TAB_SWEEP_WIDTH;
            (start.max(0.0), (start + TAB_SWEEP_WIDTH).min(1.0))
        },
    };

    fill_tab(
        ui,
        [min[0] + width * from, min[1]],
        [min[0] + width * to, max[1]],
        ui.style_color(StyleColor::PlotHistogram),
    );
}

pub(super) fn draw_tab_out_of_date(ui: &Ui) {
    let (min, max) = tab_rect();
    fill_tab(ui, min, max, DIAGNOSTIC_WARNING_COLOR);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OutOfDateChoice {
    SaveMine,
    DiscardMine,
    Leave,
}

pub(super) fn draw_out_of_date_dialog(ui: &Ui, session: &Session) -> Option<(DocumentId, OutOfDateChoice)> {
    let out_of_date = session
        .state
        .document_ids()
        .into_iter()
        .find_map(|id| Some((id, session.coop_out_of_date(id)?)));
    if out_of_date.is_some()
        && !ui.is_popup_open(OUT_OF_DATE_POPUP)
        && !ui.is_popup_open_with_flags("", PopupQueryFlags::ANY_POPUP)
    {
        ui.open_popup(OUT_OF_DATE_POPUP);
    }

    let _modal = ui
        .begin_modal_popup_config(OUT_OF_DATE_POPUP)
        .flags(MODAL_FLAGS)
        .begin()?;
    let found = out_of_date.and_then(|(id, by)| Some((id, by, session.coop()?, session.state.document(id)?)));
    let Some((id, by, coop, document)) = found else {
        ui.close_current_popup();
        return None;
    };

    let nick = coop.nick_of(by).unwrap_or(NICK_PLACEHOLDER);
    ui.text(format!(
        "{nick} shared a newer {}.",
        document.title().trim_end_matches(" *")
    ));
    ui.text("Your copy has unsaved changes and is read only until you choose.");
    ui.text_disabled("Leaving co-op keeps your copy as it is.");
    ui.dummy([0.0, ui.frame_height() * 0.25]);

    let mut choice = None;
    if ui.button("Save mine, load shared") {
        choice = Some(OutOfDateChoice::SaveMine);
    }

    ui.same_line();
    if ui.button("Discard mine, load shared") {
        choice = Some(OutOfDateChoice::DiscardMine);
    }

    ui.same_line();
    let leave = if coop.is_hosting() {
        "Stop hosting, keep mine"
    } else {
        "Leave co-op, keep mine"
    };
    if ui.button(leave) {
        choice = Some(OutOfDateChoice::Leave);
    }
    ui.set_item_tooltip("Roach out");

    if choice.is_some() {
        ui.close_current_popup();
    }

    choice.map(|choice| (id, choice))
}

pub(super) fn draw_receiving(ui: &Ui, coop: &Coop, name: &str, state: &SharedState) {
    let (text, overlay) = match *state {
        SharedState::Incoming { by, len, .. } => (
            format!(
                "Waiting for {} to upload {name}",
                coop.nick_of(by).unwrap_or(NICK_PLACEHOLDER)
            ),
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
        CoopStatus::Connecting => (PENDING_COLOR, String::from("Co-op: connecting...")),
        CoopStatus::Connected if coop.is_paused() => (PENDING_COLOR, String::from("Co-op: reloading codebase...")),
        CoopStatus::Connected => {
            let is_waiting = coop
                .shared_maps
                .values()
                .any(|shared_map| matches!(shared_map.state, SharedState::Waiting(_)));
            let color = if is_waiting {
                DIAGNOSTIC_WARNING_COLOR
            } else {
                CONNECTED_COLOR
            };

            (color, String::from("Co-op:"))
        },
        CoopStatus::Ended(reason) => (SAVE_ERROR_COLOR, format!("Co-op: {reason}")),
        CoopStatus::CodebaseMismatch { .. } => (DIAGNOSTIC_WARNING_COLOR, String::from("Co-op: different codebase")),
    };

    ui.text_colored(color, label);
    if !ui.is_item_hovered() || coop.host().is_none() && coop.shared_maps.is_empty() {
        return;
    }

    ui.tooltip(|| {
        if let Some((addr, password)) = coop.host() {
            ui.text(format!("Hosting on port {} with password {password}", addr.port()));
            if !coop.shared_maps.is_empty() {
                ui.separator();
            }
        }

        if !coop.shared_maps.is_empty() {
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
                    format!("{path}: out of date, save or discard your copy to load the shared one"),
                );
            }
        }
    });
}

pub(super) fn draw_coop_peers(ui: &Ui, coop: &Coop) {
    let Some(you) = coop.you.filter(|_| coop.is_connected()) else {
        return;
    };

    let scale = dpi(ui);
    let pad = 4.0 * scale;
    let spacing = 8.0 * scale;
    let entries = iter::once((you, coop.nick.as_str(), None))
        .chain(
            coop.peers
                .values()
                .map(|peer| (peer.info.id, peer.info.nick.as_str(), Some(peer))),
        )
        .collect::<Vec<_>>();

    for (index, (id, nick, peer)) in entries.iter().enumerate() {
        let text_size = ui.calc_text_size(nick);
        let width = text_size[0] + 2.0 * pad;
        let rest = entries.len() - index - 1;
        let more = format!("+{}", rest + 1);
        let reserved = if rest == 0 {
            0.0
        } else {
            spacing + ui.calc_text_size(&more)[0]
        };

        ui.same_line_with_spacing(0.0, spacing);
        if ui.content_region_avail()[0] < width + reserved {
            ui.text_disabled(&more);
            if ui.is_item_hovered() {
                ui.tooltip(|| {
                    for (_, nick, _) in &entries[index..] {
                        ui.text(nick);
                    }
                });
            }

            return;
        }

        ui.invisible_button(format!("##peer-{}", id.0), [width, text_size[1]]);
        let (min, max) = (ui.item_rect_min(), ui.item_rect_max());
        let color = peer_color(*id);
        let draw = ui.get_window_draw_list();
        draw.add_text([min[0] + pad, min[1]], color, nick);
        draw.add_line([min[0], max[1] + scale], [max[0], max[1] + scale], color)
            .thickness(2.0 * scale)
            .build();

        if ui.is_item_hovered() {
            ui.tooltip_text(match peer.map(|peer| peer.cursor.as_ref()) {
                None => String::from("you"),
                Some(None) => String::from("not over a shared map"),
                Some(Some(cursor)) if cursor.z > 1 => format!("on {} (z {})", cursor.map, cursor.z),
                Some(Some(cursor)) => format!("on {}", cursor.map),
            });
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CoopNoticeChoice {
    Retry,
    ReloadAndRetry,
    Dismiss,
}

fn codebase_description(codebase: &CodebaseId) -> String {
    let hash = codebase.hash.0[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    match codebase.git_hint.as_deref() {
        Some(hint) => format!("{hint} ({hash})"),
        None => hash,
    }
}

#[derive(Default)]
pub(super) struct CoopNotice {
    status: Option<CoopStatus>,
    requested: bool,
}

impl CoopNotice {
    pub(super) fn request(&mut self) { self.requested = true; }

    pub(super) fn reset(&mut self) { *self = Self::default(); }

    fn observe(&mut self, coop: Option<&Coop>) {
        let status = coop
            .map(|coop| &coop.status)
            .filter(|status| matches!(status, CoopStatus::Ended(_) | CoopStatus::CodebaseMismatch { .. }))
            .cloned();
        if status != self.status {
            self.status = status;
            self.requested = self.status.is_some();
        }
    }
}

pub(super) fn draw_coop_notice(
    ui: &Ui, coop: Option<&Coop>, is_loading: bool, notice: &mut CoopNotice,
) -> Option<CoopNoticeChoice> {
    notice.observe(coop);
    let coop = coop.filter(|_| notice.status.is_some())?;
    if notice.requested && !is_loading && !ui.is_popup_open_with_flags("", PopupQueryFlags::ANY_POPUP) {
        ui.open_popup(STATUS_POPUP);
        notice.requested = false;
    }

    let _modal = ui.begin_modal_popup_config(STATUS_POPUP).flags(MODAL_FLAGS).begin()?;
    // a host's own codebase is the session's, so hosting again needs no reload
    let is_mismatch = matches!(coop.status, CoopStatus::CodebaseMismatch { .. }) && !coop.was_hosting();
    {
        let _wrap = ui.push_text_wrap_pos(ui.cursor_pos()[0] + DIALOG_FIELD_WIDTH * dpi(ui));
        match &coop.status {
            CoopStatus::CodebaseMismatch { .. } if coop.was_hosting() => {
                ui.text_colored(DIAGNOSTIC_WARNING_COLOR, "Your codebase changed while hosting.");
                ui.text_wrapped("Guests were disconnected. Host again to start a session on the new codebase.");
            },
            CoopStatus::CodebaseMismatch { expected } => {
                ui.text_colored(DIAGNOSTIC_WARNING_COLOR, "Your codebase differs from the session.");
                ui.text_wrapped("Update your checkout to match the session, then reload and retry.");
                ui.text_wrapped(format!(
                    "Your codebase: {}",
                    codebase_description(coop.local_codebase())
                ));
                ui.text_wrapped(format!("Session codebase: {}", codebase_description(expected)));
            },
            CoopStatus::Ended(reason) => ui.text_wrapped(format!("Co-op stopped: {reason}")),
            _ => {},
        }

        ui.text_wrapped("Open maps and unsaved changes are preserved.");
    }

    ui.separator();
    if ui.button("Close") || ui.is_key_pressed(Key::Escape) {
        notice.reset();
        ui.close_current_popup();
        return Some(CoopNoticeChoice::Dismiss);
    }

    ui.same_line();
    let label = match (is_mismatch, coop.was_hosting()) {
        (true, _) => "Reload codebase and retry",
        (false, true) => "Host again",
        (false, false) => "Retry",
    };

    let is_clicked = {
        let _disabled = ui.begin_disabled_with_cond(is_loading || !coop.can_retry());
        ui.button(label)
    };

    if !is_clicked {
        return None;
    }

    ui.close_current_popup();
    Some(if is_mismatch {
        CoopNoticeChoice::ReloadAndRetry
    } else {
        CoopNoticeChoice::Retry
    })
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
