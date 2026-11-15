use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use dear_imgui_rs::{Condition, InputTextMultilineFlags, StyleColor, Ui, WindowFlags, WindowKey};
use ui::widgets::{DialogButton, center_next_window, dialog_buttons, dpi};

use crate::environment::{
    progress::{Snapshot, Stage},
    session::{DiagnosticSeverity, LoadReport, MAX_REPORTED_DIAGNOSTICS},
    worker::LoadView,
};
use crate::frontend::dialog::SAVE_ERROR_COLOR;

const LOAD_POPUP_WIDTH: f32 = 420.0;

const LOAD_TEXT_WIDTH: f32 = 620.0;

const LOAD_DIAGNOSTICS_LINES: usize = 14;

const LOAD_TEXT_MIN_LINES: usize = 4;

const LOAD_ERROR_MAX_LINES: usize = 10;

pub(crate) const DIAGNOSTIC_WARNING_COLOR: [f32; 4] = [1.0, 0.8, 0.25, 1.0];

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
pub(crate) struct DiagnosticsState {
    codebase: Option<LoadReport>,
    maps: HashMap<PathBuf, LoadReport>,
    failed_codebase: Option<LoadReport>,
    pub(crate) open: Option<DiagnosticSeverity>,
}

impl DiagnosticsState {
    pub(crate) fn set_codebase(&mut self, report: LoadReport) {
        self.maps.clear();
        self.failed_codebase = None;
        self.open = (report.errors > 0).then_some(DiagnosticSeverity::Error);
        self.codebase = (!report.is_empty()).then_some(report);
    }

    pub(crate) fn set_map(&mut self, path: PathBuf, report: LoadReport) {
        let errors = report.errors;
        let warnings = report.warnings;
        if report.is_empty() {
            self.maps.remove(&path);
        } else {
            self.maps.insert(path, report);
        }

        self.close_empty_view();

        if errors > 0 {
            self.open = Some(DiagnosticSeverity::Error);
        } else if warnings > 0 && self.open != Some(DiagnosticSeverity::Error) {
            self.open = Some(DiagnosticSeverity::Warning);
        }
    }

    pub(crate) fn set_failed_codebase(&mut self, report: LoadReport) {
        self.open = Some(DiagnosticSeverity::Error);
        self.failed_codebase = Some(report);
    }

    fn close_empty_view(&mut self) {
        if self.open.is_some_and(|severity| self.count(severity) == 0) {
            self.open = None;
        }
    }

    pub(crate) fn count(&self, severity: DiagnosticSeverity) -> usize {
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
pub(crate) struct LoadPopup {
    pub(crate) cancel: bool,
    pub(crate) dismiss: bool,
    pub(crate) copy: Option<String>,
}

pub(crate) fn draw_load_popup(
    ui: &Ui, window: &WindowKey, load: Option<&LoadView>, notice: Option<&mut LoadNotice>,
    diagnostics: &DiagnosticsState,
) -> LoadPopup {
    let mut popup = LoadPopup::default();
    if load.is_none() && notice.is_none() && diagnostics.open.is_none() {
        return popup;
    }

    ui.set_next_window_viewport(ui.main_viewport().id());
    center_next_window(ui, Condition::Always);
    let flags = WindowFlags::ALWAYS_AUTO_RESIZE
        | WindowFlags::NO_TITLE_BAR
        | WindowFlags::NO_RESIZE
        | WindowFlags::NO_MOVE
        | WindowFlags::NO_COLLAPSE
        | WindowFlags::NO_SAVED_SETTINGS
        | WindowFlags::NO_DOCKING;

    ui.window(window)
        .flags(flags)
        .build(|| draw_load_body(ui, load, notice, diagnostics, &mut popup));

    popup
}

fn draw_load_body(
    ui: &Ui, load: Option<&LoadView>, notice: Option<&mut LoadNotice>, diagnostics: &DiagnosticsState,
    popup: &mut LoadPopup,
) {
    match (load, notice) {
        (Some(view), _) => {
            draw_load_heading(ui, view.title, &view.path, LOAD_POPUP_WIDTH);
            draw_load_progress(ui, &view.snapshot);

            if view.cancellable {
                let label = if view.cancelling { "Cancelling..." } else { "Cancel" };
                if dialog_buttons(ui, &[DialogButton::cancel(label).enabled(!view.cancelling)], true).is_some() {
                    popup.cancel = true;
                }
            }
        },

        (None, Some(LoadNotice::Failed { title, path, message })) => {
            draw_load_heading(ui, title, path, LOAD_TEXT_WIDTH);
            let lines = wrapped_lines(ui, message).clamp(LOAD_TEXT_MIN_LINES, LOAD_ERROR_MAX_LINES);
            draw_selectable_text(ui, "##load-error", message, lines, Some(SAVE_ERROR_COLOR));

            match dialog_buttons(
                ui,
                &[DialogButton::cancel("Close"), DialogButton::secondary("Copy Error")],
                true,
            ) {
                Some(0) => popup.dismiss = true,
                Some(_) => popup.copy = Some(message.clone()),
                None => {},
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

                match dialog_buttons(
                    ui,
                    &[
                        DialogButton::cancel("Close"),
                        DialogButton::secondary("Copy Diagnostics"),
                    ],
                    true,
                ) {
                    Some(0) => popup.dismiss = true,
                    Some(_) => popup.copy = Some(text),
                    None => {},
                }
            }
        },
    }
}

fn draw_selectable_text(ui: &Ui, id: &str, text: &mut String, lines: usize, color: Option<[f32; 4]>) {
    let _color = color.map(|color| ui.push_style_color(StyleColor::Text, color));
    let height = ui.text_line_height_with_spacing() * lines as f32 + ui.clone_style().frame_padding()[1] * 2.0;

    ui.input_text_multiline(id, text, [LOAD_TEXT_WIDTH * dpi(ui), height])
        .flags(InputTextMultilineFlags::READ_ONLY | InputTextMultilineFlags::WORD_WRAP)
        .build();
}

fn wrapped_lines(ui: &Ui, text: &str) -> usize {
    text.lines()
        .map(|line| {
            let width = ui.calc_text_size(line)[0];

            ((width / (LOAD_TEXT_WIDTH * dpi(ui))).ceil() as usize).max(1)
        })
        .sum::<usize>()
        .max(1)
}

fn draw_load_heading(ui: &Ui, title: &str, path: &str, width: f32) {
    ui.text(title);
    ui.text_disabled(fit_path(ui, path, width * dpi(ui)));
    ui.separator();
}

fn draw_load_progress(ui: &Ui, snapshot: &Snapshot) {
    match snapshot.fraction() {
        Some(fraction) => ui
            .progress_bar(fraction)
            .size([LOAD_POPUP_WIDTH * dpi(ui), 0.0])
            .overlay_text(format!("{} / {}", grouped(snapshot.done), grouped(snapshot.total)))
            .build(),

        None => {
            let bar = ui
                .progress_bar(-(ui.time() as f32))
                .size([LOAD_POPUP_WIDTH * dpi(ui), 0.0]);
            match (snapshot.stage, snapshot.done) {
                (Stage::Preprocess, done) if done > 0 => bar.overlay_text(format!("{} files", grouped(done))).build(),
                _ => bar.overlay_text("").build(),
            }
        },
    }

    ui.text(snapshot.stage.label());
    ui.text_disabled(fit_path(ui, &snapshot.detail, LOAD_POPUP_WIDTH * dpi(ui)));
}

fn fit_path(ui: &Ui, path: &str, width: f32) -> String {
    let count = path.chars().count();
    (0..=count)
        .rev()
        .map(|max| shorten_path(path, max))
        .find(|shortened| ui.calc_text_size(shortened)[0] <= width)
        .unwrap_or_default()
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
mod tests;
