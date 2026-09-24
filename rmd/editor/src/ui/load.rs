use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use dear_imgui_rs::{Condition, InputTextMultilineFlags, Key, StyleColor, Ui, WindowFlags, WindowKey};
use editor::progress::{Snapshot, Stage};

use super::SAVE_ERROR_COLOR;
use crate::{
    loader::LoadView,
    session::{DiagnosticSeverity, LoadReport, MAX_REPORTED_DIAGNOSTICS},
};

const LOAD_POPUP_WIDTH: f32 = 420.0;

const LOAD_TEXT_WIDTH: f32 = 620.0;

const LOAD_DIAGNOSTICS_LINES: usize = 14;

const LOAD_TEXT_MIN_LINES: usize = 4;

const LOAD_ERROR_MAX_LINES: usize = 10;

const LOAD_PATH_MAX_CHARS: usize = 60;

pub(super) const DIAGNOSTIC_WARNING_COLOR: [f32; 4] = [1.0, 0.8, 0.25, 1.0];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadNotice {
    Failed {
        title: String,
        path: String,
        message: String,
    },
}

impl LoadNotice {
    pub fn failed(title: &str, path: &Path, message: impl Into<String>) -> Self {
        Self::Failed {
            title: format!("{title} failed"),
            path: path.display().to_string(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct DiagnosticsState {
    codebase: Option<LoadReport>,
    maps: HashMap<PathBuf, LoadReport>,
    failed_codebase: Option<LoadReport>,
    pub(super) open: Option<DiagnosticSeverity>,
}

impl DiagnosticsState {
    pub(super) fn set_codebase(&mut self, report: LoadReport) {
        self.maps.clear();
        self.failed_codebase = None;
        self.open = (report.errors > 0).then_some(DiagnosticSeverity::Error);
        self.codebase = (!report.is_empty()).then_some(report);
    }

    pub(super) fn set_map(&mut self, path: PathBuf, report: LoadReport) {
        if report.errors > 0 {
            self.open = Some(DiagnosticSeverity::Error);
        }
        if report.is_empty() {
            self.maps.remove(&path);
        } else {
            self.maps.insert(path, report);
        }
        self.close_empty_view();
    }

    pub(super) fn set_failed_codebase(&mut self, report: LoadReport) {
        self.open = Some(DiagnosticSeverity::Error);
        self.failed_codebase = Some(report);
    }

    fn close_empty_view(&mut self) {
        if self.open.is_some_and(|severity| self.count(severity) == 0) {
            self.open = None;
        }
    }

    pub(super) fn count(&self, severity: DiagnosticSeverity) -> usize {
        self.codebase.as_ref().map_or(0, |report| report.count(severity))
            + self.failed_codebase.as_ref().map_or(0, |report| report.count(severity))
            + self.maps.values().map(|report| report.count(severity)).sum::<usize>()
    }

    fn text(&self, severity: DiagnosticSeverity) -> (String, usize) {
        let mut reports = Vec::new();
        if let Some(report) = &self.codebase {
            reports.push(report);
        }
        if let Some(report) = &self.failed_codebase {
            reports.push(report);
        }
        let mut maps = self.maps.iter().collect::<Vec<_>>();
        maps.sort_by(|left, right| left.0.cmp(right.0));
        reports.extend(maps.into_iter().map(|(_, report)| report));

        let mut text = String::new();
        let mut shown = 0;
        for report in reports {
            for line in report.lines(severity) {
                if shown == MAX_REPORTED_DIAGNOSTICS {
                    return (text, shown);
                }
                if shown > 0 {
                    text.push('\n');
                }
                text.push_str(line);
                shown += 1;
            }
        }
        (text, shown)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct LoadPopup {
    pub(super) cancel: bool,
    pub(super) dismiss: bool,
    pub(super) copy: Option<String>,
}

pub(super) fn draw_load_popup(
    ui: &Ui, window: &WindowKey, measured: &mut [f32; 2], load: Option<&LoadView>, notice: Option<&mut LoadNotice>,
    diagnostics: &DiagnosticsState,
) -> LoadPopup {
    let mut popup = LoadPopup::default();
    if load.is_none() && notice.is_none() && diagnostics.open.is_none() {
        return popup;
    }

    let center = ui.main_viewport().work_center();
    let position = [center[0] - measured[0] / 2.0, center[1] - measured[1] / 2.0];
    let flags = WindowFlags::ALWAYS_AUTO_RESIZE
        | WindowFlags::NO_TITLE_BAR
        | WindowFlags::NO_RESIZE
        | WindowFlags::NO_MOVE
        | WindowFlags::NO_COLLAPSE
        | WindowFlags::NO_SAVED_SETTINGS
        | WindowFlags::NO_DOCKING;

    ui.window(window)
        .flags(flags)
        .position(position, Condition::Always)
        .build(|| draw_load_body(ui, measured, load, notice, diagnostics, &mut popup));

    popup
}

fn draw_load_body(
    ui: &Ui, measured: &mut [f32; 2], load: Option<&LoadView>, notice: Option<&mut LoadNotice>,
    diagnostics: &DiagnosticsState, popup: &mut LoadPopup,
) {
    let escape = ui.is_key_pressed(Key::Escape);
    match (load, notice) {
        (Some(view), _) => {
            draw_load_heading(ui, view.title, &view.path);
            draw_load_progress(ui, &view.snapshot);

            if view.cancellable {
                ui.separator();

                let _disabled = ui.begin_disabled_with_cond(view.cancelling);
                if ui.button(if view.cancelling { "Cancelling..." } else { "Cancel" }) || escape {
                    popup.cancel = true;
                }
            }
        },

        (None, Some(LoadNotice::Failed { title, path, message })) => {
            draw_load_heading(ui, title, path);
            let lines = wrapped_lines(ui, message).clamp(LOAD_TEXT_MIN_LINES, LOAD_ERROR_MAX_LINES);
            draw_selectable_text(ui, "##load-error", message, lines, Some(SAVE_ERROR_COLOR));
            ui.separator();

            if ui.button("Close") || escape {
                popup.dismiss = true;
            }
            ui.same_line();
            if ui.button("Copy") {
                popup.copy = Some(message.clone());
            }
        },

        (None, None) => {
            if let Some(severity) = diagnostics.open {
                let (title, label, color) = match severity {
                    DiagnosticSeverity::Warning => ("Load warnings", "warning", DIAGNOSTIC_WARNING_COLOR),
                    DiagnosticSeverity::Error => ("Load errors", "error", SAVE_ERROR_COLOR),
                };
                ui.text(title);
                ui.separator();
                let count = diagnostics.count(severity);
                let plural = if count == 1 { "" } else { "s" };
                ui.text(format!("{count} {label}{plural}"));
                let (mut text, shown) = diagnostics.text(severity);
                if shown < count {
                    ui.text_disabled(format!("Showing first {shown} of {count}"));
                }

                let lines = wrapped_lines(ui, &text).clamp(LOAD_TEXT_MIN_LINES, LOAD_DIAGNOSTICS_LINES);
                draw_selectable_text(ui, "##load-diagnostics", &mut text, lines, Some(color));
                ui.separator();

                if ui.button("Close") || escape {
                    popup.dismiss = true;
                }

                ui.same_line();
                if ui.button("Copy") {
                    popup.copy = Some(text);
                }
            }
        },
    }

    *measured = ui.window_size();
}

fn draw_selectable_text(ui: &Ui, id: &str, text: &mut String, lines: usize, color: Option<[f32; 4]>) {
    let _color = color.map(|color| ui.push_style_color(StyleColor::Text, color));
    let height = ui.text_line_height_with_spacing() * lines as f32 + ui.clone_style().frame_padding()[1] * 2.0;

    ui.input_text_multiline(id, text, [LOAD_TEXT_WIDTH, height])
        .flags(InputTextMultilineFlags::READ_ONLY | InputTextMultilineFlags::WORD_WRAP)
        .build();
}

fn wrapped_lines(ui: &Ui, text: &str) -> usize {
    text.lines()
        .map(|line| {
            let width = ui.calc_text_size(line)[0];

            ((width / LOAD_TEXT_WIDTH).ceil() as usize).max(1)
        })
        .sum::<usize>()
        .max(1)
}

fn draw_load_heading(ui: &Ui, title: &str, path: &str) {
    ui.text(title);
    ui.text_disabled(shorten_path(path, LOAD_PATH_MAX_CHARS));
    ui.separator();
}

fn draw_load_progress(ui: &Ui, snapshot: &Snapshot) {
    match snapshot.fraction() {
        Some(fraction) => ui
            .progress_bar(fraction)
            .size([LOAD_POPUP_WIDTH, 0.0])
            .overlay_text(format!("{} / {}", grouped(snapshot.done), grouped(snapshot.total)))
            .build(),

        None => {
            let bar = ui.progress_bar(-(ui.time() as f32)).size([LOAD_POPUP_WIDTH, 0.0]);
            match (snapshot.stage, snapshot.done) {
                (Stage::Preprocess, done) if done > 0 => bar.overlay_text(format!("{} files", grouped(done))).build(),
                _ => bar.overlay_text("").build(),
            }
        },
    }

    ui.text(snapshot.stage.label());
    ui.text_disabled(shorten_path(&snapshot.detail, LOAD_PATH_MAX_CHARS));
}

fn shorten_path(path: &str, max: usize) -> String {
    let count = path.chars().count();
    if count <= max {
        return String::from(path);
    }

    let kept = path.chars().skip(count - max.saturating_sub(1)).collect::<String>();

    format!("\u{2026}{kept}")
}

fn grouped(value: usize) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);

    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }

    out
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{DiagnosticsState, LoadNotice, grouped, shorten_path};
    use crate::session::{DiagnosticSeverity, LoadReport};

    #[test]
    fn a_short_path_is_left_alone_and_a_long_one_keeps_its_tail() {
        assert_eq!(shorten_path("code/game/area.dm", 60), "code/game/area.dm");
        assert_eq!(shorten_path("abcdef", 6), "abcdef");
        assert_eq!(shorten_path("abcdef", 4), "\u{2026}def");
        assert_eq!(shorten_path("", 8), "");
    }

    #[test]
    fn file_counts_are_grouped_in_threes() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(7), "7");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1482), "1,482");
        assert_eq!(grouped(1_234_567), "1,234,567");
    }

    #[test]
    fn a_load_notice_keeps_the_job_title_and_message() {
        let notice = LoadNotice::failed("Opening codebase", Path::new("game/tg.dme"), "no such file");

        assert_eq!(
            notice,
            LoadNotice::Failed {
                title: String::from("Opening codebase failed"),
                path: String::from("game/tg.dme"),
                message: String::from("no such file"),
            }
        );
    }

    #[test]
    fn diagnostics_accumulate_by_map_and_replace_reopened_map_results() {
        let mut state = DiagnosticsState::default();
        state.set_codebase(LoadReport {
            warnings: 1,
            warning_lines: vec![String::from("codebase warning")],
            ..LoadReport::default()
        });
        assert_eq!(state.count(DiagnosticSeverity::Warning), 1);
        assert_eq!(state.open, None);

        state.set_map(
            PathBuf::from("b.dmm"),
            LoadReport {
                errors: 2,
                error_lines: vec![String::from("b.dmm: first"), String::from("b.dmm: second")],
                ..LoadReport::default()
            },
        );
        state.set_map(
            PathBuf::from("a.dmm"),
            LoadReport {
                errors: 1,
                error_lines: vec![String::from("a.dmm: error")],
                ..LoadReport::default()
            },
        );
        assert_eq!(state.count(DiagnosticSeverity::Error), 3);
        assert_eq!(state.open, Some(DiagnosticSeverity::Error));
        assert_eq!(
            state.text(DiagnosticSeverity::Error).0,
            "a.dmm: error\nb.dmm: first\nb.dmm: second"
        );

        state.open = None;
        assert_eq!(state.count(DiagnosticSeverity::Error), 3);
        state.set_map(PathBuf::from("b.dmm"), LoadReport::default());
        assert_eq!(state.count(DiagnosticSeverity::Error), 1);
        assert_eq!(state.open, None);
    }

    #[test]
    fn failed_loads_remain_visible_until_a_successful_codebase_replaces_them() {
        let mut state = DiagnosticsState::default();
        state.set_failed_codebase(LoadReport::failure(
            "game.dme",
            Path::new("game.dme"),
            "cannot read file",
        ));
        assert_eq!(state.count(DiagnosticSeverity::Error), 1);
        assert_eq!(state.open, Some(DiagnosticSeverity::Error));
        state.open = None;
        assert!(
            state
                .text(DiagnosticSeverity::Error)
                .0
                .contains("game.dme: cannot read file")
        );

        state.set_codebase(LoadReport::default());
        assert_eq!(state.count(DiagnosticSeverity::Error), 0);
        assert!(state.failed_codebase.is_none());
    }
}
