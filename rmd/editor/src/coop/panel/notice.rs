use dear_imgui_rs::{PopupQueryFlags, Ui};
use net::CodebaseId;
use ui::widgets::{DialogButton, begin_centered_modal, dialog_buttons, dpi};

use super::STATUS_POPUP;
use crate::{
    coop::{Coop, CoopStatus},
    environment::panel::DIAGNOSTIC_WARNING_COLOR,
    frontend::dialog::{DIALOG_FIELD_WIDTH, MODAL_FLAGS},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CoopNoticeChoice {
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
pub(crate) struct CoopNotice {
    status: Option<CoopStatus>,
    requested: bool,
}

impl CoopNotice {
    pub(crate) fn request(&mut self) {
        self.requested = true;
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

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

pub(crate) fn draw_coop_notice(
    ui: &Ui, coop: Option<&Coop>, is_loading: bool, notice: &mut CoopNotice,
) -> Option<CoopNoticeChoice> {
    notice.observe(coop);
    let coop = coop.filter(|_| notice.status.is_some())?;
    if notice.requested && !is_loading && !ui.is_popup_open_with_flags("", PopupQueryFlags::ANY_POPUP) {
        ui.open_popup(STATUS_POPUP);
        notice.requested = false;
    }

    let _modal = begin_centered_modal(ui, STATUS_POPUP, MODAL_FLAGS)?;
    // a host's own codebase is the session's, so hosting again needs no reload
    let is_mismatch = matches!(coop.status, CoopStatus::CodebaseMismatch { .. }) && !coop.was_hosting();
    {
        let _wrap = ui.push_text_wrap_pos(ui.cursor_pos()[0] + DIALOG_FIELD_WIDTH * dpi(ui));
        match &coop.status {
            CoopStatus::CodebaseMismatch { .. } if coop.was_hosting() => {
                ui.text_colored(DIAGNOSTIC_WARNING_COLOR, "Your codebase changed while hosting.");
                ui.text("Guests were disconnected. Host again to start a session on the new codebase.");
            },
            CoopStatus::CodebaseMismatch { expected } => {
                ui.text_colored(DIAGNOSTIC_WARNING_COLOR, "Your codebase differs from the session.");
                ui.text("Update your checkout to match the session, then reload and retry.");
                ui.text(format!(
                    "Your codebase: {}",
                    codebase_description(coop.local_codebase())
                ));
                ui.text(format!("Session codebase: {}", codebase_description(expected)));
            },
            CoopStatus::Ended(reason) => ui.text(format!("Co-op stopped: {reason}")),
            _ => {},
        }

        ui.text("Open maps and unsaved changes are preserved.");
    }

    let label = match (is_mismatch, coop.was_hosting()) {
        (true, _) => "Reload Codebase and Retry",
        (false, true) => "Host Again",
        (false, false) => "Retry",
    };

    let pressed = dialog_buttons(
        ui,
        &[
            DialogButton::cancel("Close"),
            DialogButton::primary(label).enabled(!is_loading && coop.can_retry()),
        ],
        true,
    );

    if pressed == Some(0) {
        notice.reset();
        ui.close_current_popup();
        return Some(CoopNoticeChoice::Dismiss);
    }

    if pressed != Some(1) {
        return None;
    }

    ui.close_current_popup();
    Some(if is_mismatch {
        CoopNoticeChoice::ReloadAndRetry
    } else {
        CoopNoticeChoice::Retry
    })
}
