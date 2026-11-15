use std::path::{Path, PathBuf};

use dear_imgui_rs::{Context, WindowKey, sys};
use ui::testing::{CONTEXT_LOCK, assert_window_centered, finish_frame, popup_context, set_desktop_geometry};

use super::{DiagnosticsState, LOAD_POPUP_WIDTH, LoadNotice, draw_load_popup, fit_path, grouped, shorten_path};
use crate::environment::{
    progress::{Snapshot, Stage},
    session::{DiagnosticSeverity, LoadReport},
    worker::LoadView,
};

#[test]
fn load_popups_follow_window_resize_and_content_changes() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    for desktop in [false, true] {
        let mut context = popup_context(desktop);
        let window = WindowKey::new("load-resize-test", "Load").unwrap();
        let diagnostics = DiagnosticsState::default();
        let progress = LoadView {
            title: "Opening map",
            path: String::from("missing.dmm"),
            snapshot: crate::environment::progress::Progress::new().snapshot(),
            cancelling: false,
            cancellable: true,
        };
        let mut notice = LoadNotice::failed("Opening map", Path::new("missing.dmm"), "No such file or directory.");
        let draw = |context: &mut Context, load: Option<&LoadView>, notice: Option<&mut LoadNotice>| {
            draw_load_popup(context.frame(), &window, load, notice, &diagnostics);
            finish_frame(context);
        };

        set_desktop_geometry(&mut context, [0.0, 0.0], [1920.0, 1080.0]);
        for _ in 0..4 {
            draw(&mut context, Some(&progress), None);
        }
        assert_window_centered(&mut context, c"###load-resize-test");

        let position = if desktop { [230.0, 120.0] } else { [0.0, 0.0] };
        set_desktop_geometry(&mut context, position, [960.0, 640.0]);
        for _ in 0..4 {
            draw(&mut context, Some(&progress), None);
        }
        assert_eq!(context.main_viewport().pos(), position);
        assert_eq!(context.main_viewport().size(), [960.0, 640.0]);
        assert_window_centered(&mut context, c"###load-resize-test");

        for _ in 0..4 {
            draw(&mut context, None, Some(&mut notice));
        }
        assert_window_centered(&mut context, c"###load-resize-test");
        draw(&mut context, None, None);

        let position = if desktop { [320.0, 200.0] } else { [0.0, 0.0] };
        set_desktop_geometry(&mut context, position, [900.0, 600.0]);
        context.style_mut().set_font_scale_main(1.25);
        for _ in 0..4 {
            draw(&mut context, None, Some(&mut notice));
        }
        assert_eq!(context.main_viewport().pos(), position);
        assert_eq!(context.main_viewport().size(), [900.0, 600.0]);
        assert_window_centered(&mut context, c"###load-resize-test");
    }
}

#[test]
fn long_progress_details_do_not_widen_the_popup() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut context = popup_context(true);
    set_desktop_geometry(&mut context, [0.0, 0.0], [1920.0, 1080.0]);
    let window = WindowKey::new("load-width-test", "Load").unwrap();
    let diagnostics = DiagnosticsState::default();
    let mut width = |detail: &str| {
        let progress = LoadView {
            title: "Opening codebase",
            path: "code/".repeat(40),
            snapshot: Snapshot {
                stage: Stage::Preprocess,
                done: 0,
                total: 0,
                detail: String::from(detail),
            },
            cancelling: false,
            cancellable: true,
        };
        for _ in 0..4 {
            draw_load_popup(context.frame(), &window, Some(&progress), None, &diagnostics);
            finish_frame(&mut context);
        }

        // SAFETY: the test owns the current context and its live window.
        unsafe { (*sys::igFindWindowByName(c"###load-width-test".as_ptr())).Size.x }
    };

    let short = width("");
    assert_eq!(width(&"code/modules/".repeat(20)), short);
    assert_eq!(width(""), short);
    let padding = context.style().window_padding()[0];
    assert_eq!(short, LOAD_POPUP_WIDTH + padding * 2.0);
}

#[test]
fn paths_are_fitted_to_a_pixel_width_keeping_their_tail() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut context = popup_context(false);
    let ui = context.frame();
    let path = "code/modules/mob/living/carbon/human/human.dm";
    let width = ui.calc_text_size("human/human.dm")[0] + 1.0;

    let fitted = fit_path(ui, path, width);
    assert!(ui.calc_text_size(&fitted)[0] <= width);
    assert!(fitted.starts_with('\u{2026}') && path.ends_with(fitted.trim_start_matches('\u{2026}')));
    assert_eq!(fit_path(ui, path, 1000.0), path);
    assert_eq!(fit_path(ui, "", 10.0), "");
    finish_frame(&mut context);
}

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
fn map_warnings_open_automatically_and_remain_available_after_dismissal() {
    let mut state = DiagnosticsState::default();
    let path = PathBuf::from("map.dmm");
    let report = LoadReport {
        warnings: 1,
        warning_lines: vec![String::from("map.dmm: unknown type path /obj/missing")],
        ..LoadReport::default()
    };
    state.set_map(path.clone(), report.clone());
    assert_eq!(state.open, Some(DiagnosticSeverity::Warning));

    state.open = None;
    assert_eq!(state.count(DiagnosticSeverity::Warning), 1);
    assert_eq!(state.text(DiagnosticSeverity::Warning).0, report.warning_lines[0]);
    state.set_map(path.clone(), report);
    assert_eq!(state.count(DiagnosticSeverity::Warning), 1);
    assert_eq!(state.open, Some(DiagnosticSeverity::Warning));

    state.set_map(path, LoadReport::default());
    assert_eq!(state.count(DiagnosticSeverity::Warning), 0);
    assert_eq!(state.open, None);
}

#[test]
fn map_errors_take_priority_over_warnings_and_can_be_replaced_by_warnings() {
    let mut state = DiagnosticsState::default();
    let warning = LoadReport {
        warnings: 1,
        warning_lines: vec![String::from("unknown type path")],
        ..LoadReport::default()
    };
    state.set_map(
        PathBuf::from("errors.dmm"),
        LoadReport {
            errors: 1,
            error_lines: vec![String::from("map error")],
            ..warning.clone()
        },
    );
    assert_eq!(state.open, Some(DiagnosticSeverity::Error));
    state.set_map(PathBuf::from("warnings.dmm"), warning.clone());
    assert_eq!(state.open, Some(DiagnosticSeverity::Error));
    assert_eq!(state.count(DiagnosticSeverity::Warning), 2);

    state.set_map(PathBuf::from("errors.dmm"), warning);
    assert_eq!(state.count(DiagnosticSeverity::Error), 0);
    assert_eq!(state.open, Some(DiagnosticSeverity::Warning));
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
