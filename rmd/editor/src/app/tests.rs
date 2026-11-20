use std::time::Duration;

use dear_imgui_rs::sys;
use ui::theme;
use winit::event::WindowEvent;

use super::*;
use super::{args::Arguments, events::is_input, redraw::settle_appearing_windows};
use crate::{document::grid::TileView, environment::worker};

fn headless_editor() -> App {
    let mut imgui = Context::create();
    imgui.set_ini_filename(None::<PathBuf>).unwrap();
    imgui.font_atlas().try_claim_legacy_renderer().unwrap().build();
    imgui.io_mut().set_display_size([1280.0, 720.0]);
    imgui.io_mut().set_delta_time(1.0 / 60.0);
    imgui.io_mut().set_config_flags(ConfigFlags::DOCKING_ENABLE);
    let themes = Themes::load(None, imgui.style(), theme::DEFAULT_THEME);

    App {
        session: Session::new(),
        settings: Settings {
            check_for_updates: false,
            ..Settings::default()
        },
        ui: UiState::new(false).unwrap(),
        loader: Loader::new(),
        pending_map: None,
        pending_reload: None,
        pending_share: None,
        pending_trust: None,
        declined_trust: HashSet::new(),
        retry_coop_after_load: false,
        deferred_job: None,
        settings_ready: true,
        startup_error: None,
        uploaded_texture_revision: None,
        pending_screenshot: None,
        screenshot_dir: None,
        clipboard: None,
        title: String::new(),
        consumer: None,
        renderer: None,
        platform: None,
        imgui: Some(imgui),
        scaled_style: None,
        themes: Some(themes),
        ui_font: None,
        window: None,
        is_revealed: false,
        pacer: FramePacer::default(),
        is_occluded: false,
    }
}

fn headless_frame(app: &mut App, now: Instant, load: Option<&worker::LoadView>) -> NextFrame {
    let imgui = app.imgui.as_mut().unwrap();
    let output = app
        .ui
        .draw(
            imgui.frame(),
            &mut app.session,
            &mut app.settings,
            app.themes.as_mut().unwrap(),
            load,
        )
        .unwrap();
    assert!(imgui.render_legacy().valid());
    settle_appearing_windows(imgui, &mut app.pacer, now);

    app.pacer.next(
        now,
        now,
        output.demand,
        Status {
            is_busy: load.is_some() || app.is_busy(),
            is_hidden: false,
        },
    )
}

fn settle_headless_editor(app: &mut App, start: Instant) -> Instant {
    for frame in 0..100 {
        let now = start + Duration::from_millis(16) * frame;
        match headless_frame(app, now, None) {
            NextFrame::Now => {},
            NextFrame::OnEvent => return now,
            NextFrame::At(_) => panic!("static UI should not need background polling or animation"),
        }
    }
    panic!("static UI did not settle");
}

struct PopupGeometry {
    id: u32,
    position: [f32; 2],
    size: [f32; 2],
    appearing: bool,
    visible: bool,
}

fn load_popup_geometry() -> PopupGeometry {
    // SAFETY: the test holds CONTEXT_LOCK and inspects its live window after rendering.
    unsafe {
        let window = sys::igFindWindowByName(c"###load".as_ptr());
        assert!(!window.is_null(), "the load popup should exist");
        let window = &*window;
        let lists = &(*window.Viewport).DrawDataP.CmdLists;
        let rendered = (0..lists.Size).any(|index| *lists.Data.add(index as usize) == window.DrawList);

        PopupGeometry {
            id: window.ID,
            position: [window.Pos.x, window.Pos.y],
            size: [window.Size.x, window.Size.y],
            appearing: window.Appearing,
            visible: window.Active && !window.Hidden && rendered,
        }
    }
}

fn assert_error_popup_is_visible_and_centered(app: &mut App) {
    let popup = load_popup_geometry();
    let imgui = app.imgui.as_mut().unwrap();
    assert!(popup.visible, "the popup should be included in the rendered draw data");
    assert!(!popup.appearing, "the open popup should have finished appearing");
    let expected_width = 620.0 + imgui.style().window_padding()[0] * 2.0;
    assert!((popup.size[0] - expected_width).abs() <= 1.0, "{:#?}", popup.size);
    assert!(popup.size[1] > 100.0 && popup.size[1] < 720.0);
    let center = imgui.main_viewport().work_center();
    for (axis, center) in center.into_iter().enumerate() {
        assert!((popup.position[axis] + popup.size[axis] / 2.0 - center).abs() <= 1.0);
    }
}

#[test]
fn an_incoming_load_notice_appears_and_settles_without_input() {
    let _guard = ui::testing::CONTEXT_LOCK.lock().unwrap();
    let mut app = headless_editor();
    let idle = settle_headless_editor(&mut app, Instant::now());

    app.ui.set_load_notice(Some(LoadNotice::failed(
        "Opening map",
        Path::new("missing.dmm"),
        "The map could not be opened.\nNo such file or directory.",
    )));
    assert_eq!(headless_frame(&mut app, idle, None), NextFrame::Now);
    let settled = settle_headless_editor(&mut app, idle + Duration::from_millis(16));
    assert_error_popup_is_visible_and_centered(&mut app);
    assert_eq!(
        headless_frame(&mut app, settled + Duration::from_secs(1), None),
        NextFrame::OnEvent
    );
    assert_error_popup_is_visible_and_centered(&mut app);

    let resized = settled + Duration::from_secs(2);
    assert!(is_input(&WindowEvent::Moved(winit::dpi::PhysicalPosition::new(
        180, 90
    ))));
    app.imgui.as_mut().unwrap().io_mut().set_display_size([960.0, 640.0]);
    app.pacer.activity(resized);
    assert_eq!(headless_frame(&mut app, resized, None), NextFrame::Now);
    let settled = settle_headless_editor(&mut app, resized + Duration::from_millis(16));
    assert_eq!(app.imgui.as_mut().unwrap().main_viewport().size(), [960.0, 640.0]);
    assert_error_popup_is_visible_and_centered(&mut app);
    assert_eq!(
        headless_frame(&mut app, settled + Duration::from_secs(1), None),
        NextFrame::OnEvent
    );
}

#[test]
fn a_load_result_settles_when_error_replaces_progress_in_the_same_window() {
    let _guard = ui::testing::CONTEXT_LOCK.lock().unwrap();
    let mut app = headless_editor();
    // Keep simulated frames in the past: applying a loader result records real time.
    let idle = settle_headless_editor(&mut app, Instant::now() - Duration::from_secs(5));
    let progress = worker::LoadView {
        title: "Opening map",
        path: String::from("missing.dmm"),
        snapshot: crate::environment::progress::Progress::new().snapshot(),
        cancelling: false,
        cancellable: true,
    };
    for frame in 0..5 {
        assert_eq!(
            headless_frame(&mut app, idle + Duration::from_millis(16) * frame, Some(&progress)),
            NextFrame::Now
        );
    }
    let progress_geometry = load_popup_geometry();
    assert!(progress_geometry.visible && !progress_geometry.appearing);
    // Let the appearance activity expire while background loading keeps frames running.
    let completed = idle + Duration::from_secs(1);
    assert_eq!(headless_frame(&mut app, completed, Some(&progress)), NextFrame::Now);
    let completed = Instant::now();
    assert_eq!(
        app.pacer
            .next(completed, completed, FrameDemand::Idle, Status::default()),
        NextFrame::OnEvent
    );

    app.apply_outcome(Outcome::Failed {
        job: Job::Map {
            path: PathBuf::from("missing.dmm"),
            z: 1,
            git_enabled: false,
        },
        error: String::from("The map could not be opened.\nNo such file or directory."),
    });
    let completed = Instant::now();
    assert_eq!(
        app.pacer
            .next(completed, completed, FrameDemand::Idle, Status::default()),
        NextFrame::Now
    );
    assert_eq!(headless_frame(&mut app, completed, None), NextFrame::Now);
    let error_geometry = load_popup_geometry();
    assert_eq!(error_geometry.id, progress_geometry.id);
    assert!(
        !error_geometry.appearing,
        "the existing window does not appear again for an error"
    );
    let settled = settle_headless_editor(&mut app, completed + Duration::from_millis(16));
    assert_error_popup_is_visible_and_centered(&mut app);
    let error_geometry = load_popup_geometry();
    assert!(error_geometry.size[0] > progress_geometry.size[0]);
    assert!(error_geometry.size[1] > progress_geometry.size[1]);
    assert_eq!(
        headless_frame(&mut app, settled + Duration::from_secs(1), None),
        NextFrame::OnEvent
    );
}

#[test]
fn unknown_map_types_open_the_issues_panel_and_keep_their_overrides_when_saved() {
    use base::{coord::Coord, location::Location, path::TreePath};
    use std::sync::Arc;

    use dmm::{MapFormat, writer::MapWriter};

    let _guard = ui::testing::CONTEXT_LOCK.lock().unwrap();
    let mut app = headless_editor();
    let root = std::env::temp_dir().join(format!("rmd-unknown-map-types-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let mut tree = objtree::ObjectTree::new();
    for path in ["/turf", "/area"] {
        tree.register(&TreePath::parse(path), Location::default());
    }
    app.session.state.environment = Some(Arc::new(crate::environment::Environment::new(
        root.join("game.dme"),
        tree,
    )));
    app.session.set_sanitize_vars_on_save(true);
    let (map, errors) = dmm::parser::parse(
        r#"
"a" = (/obj/missing{name = "old object"; dir = 4},/turf,/area)
(1,1,1) = {"
a
"}
"#,
    );
    assert!(errors.is_empty(), "{errors:?}");

    for (index, format) in [MapFormat::Standard, MapFormat::Tgm].into_iter().enumerate() {
        let path = root.join(format!("map-{index}.dmm"));
        fs::write(&path, MapWriter::new(&map).with_format(format).write()).unwrap();
        let loaded = worker::load_map(&path, 1, &crate::environment::progress::Progress::new()).unwrap();
        app.apply_outcome(Outcome::Map(Box::new(loaded)));
        assert!(!app.session.state.active_document().unwrap().is_dirty());
        assert_eq!(
            app.session.map().unwrap().tile_at(Coord::new(1, 1, 1)),
            map.tile_at(Coord::new(1, 1, 1)).map(TileView::from)
        );
        let rendered = Instant::now();
        for frame in 0..5 {
            headless_frame(&mut app, rendered + Duration::from_millis(16) * frame, None);
        }
        let settled = rendered + Duration::from_secs(1);
        assert!(matches!(headless_frame(&mut app, settled, None), NextFrame::At(_)));
        assert!(app.ui.map_issues_visible(), "unknown paths should open Map Issues");

        // A reload uses the same validation and keeps the existing document.
        let id = app.session.state.active().unwrap();
        app.pending_reload = Some(id);
        let loaded = worker::load_map(&path, 1, &crate::environment::progress::Progress::new()).unwrap();
        app.apply_outcome(Outcome::Map(Box::new(loaded)));
        assert_eq!(app.session.state.active(), Some(id));
        assert_eq!(app.session.state.document_ids().len(), index + 1);
        headless_frame(&mut app, settled + Duration::from_secs(1), None);
        assert!(app.ui.map_issues_visible());

        app.session.save_map_as(&path, format).unwrap();
        let (saved, errors) = dmm::parser::parse(&fs::read_to_string(&path).unwrap());
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(saved.format, format);
        assert_eq!(saved.tile_at(Coord::new(1, 1, 1)), map.tile_at(Coord::new(1, 1, 1)));
    }
    fs::remove_dir_all(root).unwrap();
}

fn parse(arguments: &[&str]) -> Result<Arguments, String> {
    let arguments = arguments.iter().map(PathBuf::from).collect::<Vec<_>>();

    parse_arguments(&arguments)
}

#[test]
fn defaults_to_first_level() {
    assert_eq!(
        parse(&["map.dmm"]),
        Ok(Arguments {
            environment: None,
            map: Some(PathBuf::from("map.dmm")),
            z: 1,
        })
    );
    assert_eq!(
        parse(&["environment.dme"]),
        Ok(Arguments {
            environment: Some(PathBuf::from("environment.dme")),
            map: None,
            z: 1,
        })
    );
    assert_eq!(
        parse(&["environment.dme", "map.dmm"]),
        Ok(Arguments {
            environment: Some(PathBuf::from("environment.dme")),
            map: Some(PathBuf::from("map.dmm")),
            z: 1,
        })
    );
}

#[test]
fn accepts_a_trailing_level() {
    assert_eq!(
        parse(&["map.dmm", "2"]),
        Ok(Arguments {
            environment: None,
            map: Some(PathBuf::from("map.dmm")),
            z: 2,
        })
    );
    assert_eq!(
        parse(&["environment.dme", "2"]),
        Ok(Arguments {
            environment: Some(PathBuf::from("environment.dme")),
            map: None,
            z: 2,
        })
    );
    assert_eq!(
        parse(&["environment.dme", "map.dmm", "2"]),
        Ok(Arguments {
            environment: Some(PathBuf::from("environment.dme")),
            map: Some(PathBuf::from("map.dmm")),
            z: 2,
        })
    );
}

#[test]
fn no_arguments_open_nothing() {
    assert_eq!(
        parse(&[]),
        Ok(Arguments {
            environment: None,
            map: None,
            z: 1,
        })
    );
}

#[test]
fn rejects_invalid_arguments_and_levels() {
    assert!(parse(&["map.dmm", "0"]).is_err());
    assert!(parse(&["map.dmm", "two"]).is_err());
    assert!(parse(&["environment.dme", "not-a-map", "2"]).is_err());
    assert!(parse(&["environment.dme", "map.dmm", "2", "extra"]).is_err());
}

#[test]
fn bake_options_restore_native_and_forced_profile_choices_per_codebase() {
    let mut settings = Settings::default();
    let environment = std::path::Path::new("station.dme");
    settings.set_profile_for(environment, Some("/datum/demir/station/debug"));
    settings.set_forced_profile_for(environment, Some("tgstation"));

    let options = bake_options(&settings, Some(environment));

    assert_eq!(
        options.profile,
        Some(base::path::TreePath::parse("/datum/demir/station/debug"))
    );
    assert_eq!(options.forced_profile.as_deref(), Some("tgstation"));
    assert!(options.optimizations_enabled);
    settings.optimizations_enabled = false;
    assert!(!bake_options(&settings, Some(environment)).optimizations_enabled);
    assert_eq!(bake_options(&settings, None).forced_profile, None);
}

#[test]
fn opening_an_already_open_map_focuses_it_without_loading() {
    let _guard = ui::testing::CONTEXT_LOCK.lock().unwrap();
    let dir = std::env::temp_dir().join(format!("rmd-open-focus-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let mut app = headless_editor();
    let open = |app: &mut App, name: &str| {
        let path = dir.join(name);
        fs::write(&path, "").unwrap();
        app.session.apply_map(worker::LoadedMap {
            path,
            map: dmm::Map::new(dmm::Size { x: 4, y: 4, z: 1 }),
            z: 1,
            errors: vec![],
            repo: None,
            conflict: None,
        });

        app.session.state.active().unwrap()
    };
    let first = open(&mut app, "first.dmm");
    let second = open(&mut app, "second.dmm");
    assert_eq!(app.session.state.active(), Some(second));

    app.apply_open(OpenRequest::Map(dir.join(".").join("first.dmm")));
    assert_eq!(app.session.state.active(), Some(first));
    assert!(!app.loader.is_busy());
    assert_eq!(app.session.state.document_ids().len(), 2);
    let _ = fs::remove_dir_all(dir);
}

fn loaded_codebase_calling_external_libraries() -> LoadedCodebase {
    let mut environment = crate::testing::compile_environment(&[(
        "<test-profile-default.dm>",
        "/datum/demir/test\n    default = TRUE\n",
    )]);
    environment.bake_program.as_mut().unwrap().uses_external_libraries = true;

    LoadedCodebase {
        environment,
        diagnostics: Default::default(),
        textures: Default::default(),
        thumbnails: Default::default(),
        maps: Vec::new(),
    }
}

#[test]
fn a_codebase_calling_external_libraries_waits_for_a_trust_decision() {
    let _guard = ui::testing::CONTEXT_LOCK.lock().unwrap();
    let mut app = headless_editor();
    let path = PathBuf::from("untrusted.dme");

    app.apply_outcome(Outcome::Codebase {
        path: path.clone(),
        loaded: Box::new(loaded_codebase_calling_external_libraries()),
    });
    assert!(app.session.state.environment.is_none());
    assert_eq!(app.ui.trust_prompt, Some(path.clone()));
    assert!(app.pending_trust.is_some());

    app.decide_trust(TrustDecision::Decline);
    assert!(app.session.state.environment.is_some());
    assert!(app.ui.trust_prompt.is_none());
    assert!(app.pending_trust.is_none());
    assert!(app.declined_trust.contains(&path));
    assert!(!app.settings.is_trusted(&path));

    app.apply_outcome(Outcome::Codebase {
        path,
        loaded: Box::new(loaded_codebase_calling_external_libraries()),
    });
    assert!(app.pending_trust.is_none(), "a declined codebase is not asked again");
}
