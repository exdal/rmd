use std::{
    env::consts::{ARCH, OS},
    ptr,
};

use dear_imgui_rs::{TableOptions, TableSizingPolicy, Ui, sys};
use render::AdapterInfo;
use ui::{
    icons::materialdesignicons::ICON_GITHUB,
    widgets::{DialogButton, begin_centered_modal, dialog_buttons, dpi},
};

use super::{UiState, dialog::MODAL_FLAGS};
use crate::build_info::{APP_NAME, COMMIT_URL, CONTRIBUTORS, GIT_SHORT_HASH, REPOSITORY_URL, VERSION, VERSION_URL};

const ABOUT_POPUP: &str = "About##about";

const APP_TITLE_SIZE: f32 = 40.0;

const ABOUT_WIDTH: f32 = 440.0;

const CONTRIBUTOR_COLUMNS: usize = 3;

pub(super) fn draw_app_title(ui: &Ui) {
    let _font = ui.push_font_with_size(None, APP_TITLE_SIZE);
    ui.text(APP_NAME);
}

pub(super) fn draw_build_info(ui: &Ui) {
    let version = format!("v{VERSION}");
    match VERSION_URL {
        Some(url) => {
            ui.text_link_open_url(&version, url);
        },
        None => ui.text_disabled(&version),
    }

    ui.same_line();
    ui.text_disabled("\u{2022}");
    ui.same_line();

    let hash = GIT_SHORT_HASH.unwrap_or("unknown");
    match (GIT_SHORT_HASH, COMMIT_URL) {
        (Some(_), Some(url)) => {
            ui.text_link_open_url(hash, url);
        },
        _ => ui.text_disabled(hash),
    }
}

impl UiState {
    pub(super) fn draw_about(&mut self, ui: &Ui, open: bool) {
        if open {
            ui.open_popup(ABOUT_POPUP);
        }

        let width = ABOUT_WIDTH * dpi(ui);
        unsafe {
            sys::igSetNextWindowSizeConstraints(
                sys::ImVec2_c { x: width, y: 0.0 },
                sys::ImVec2_c { x: width, y: f32::MAX },
                None,
                ptr::null_mut(),
            );
        }
        let Some(_modal) = begin_centered_modal(ui, ABOUT_POPUP, MODAL_FLAGS) else {
            return;
        };

        draw_app_title(ui);
        draw_build_info(ui);
        if let Some(url) = REPOSITORY_URL {
            ui.same_line();
            ui.text_disabled("\u{2022}");
            ui.same_line();
            ui.text_link_open_url(format!("{ICON_GITHUB} GitHub"), url);
        }

        ui.spacing();

        ui.separator_with_text("Host");
        match &self.gpu {
            Some(AdapterInfo { name, driver }) => ui.text_wrapped(format!("GPU: {name} ({driver})")),
            None => ui.text_wrapped("GPU: unknown"),
        }

        ui.text(format!("OS: {OS} {ARCH}"));

        ui.separator_with_text("Contributors");
        if CONTRIBUTORS.trim().is_empty() {
            ui.text_disabled("unavailable in this build");
        }

        let options = TableOptions::new().sizing_policy(TableSizingPolicy::StretchSame);
        if let Some(_table) = ui.begin_table_with_flags("about-contributors", CONTRIBUTOR_COLUMNS, options) {
            for name in CONTRIBUTORS.lines() {
                ui.table_next_column();
                ui.text(name);
            }
        }

        if dialog_buttons(ui, &[DialogButton::cancel("Close")], true).is_some() {
            ui.close_current_popup();
        }
    }
}
