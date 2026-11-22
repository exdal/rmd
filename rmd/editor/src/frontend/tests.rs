use base::{coord::Coord, path::TreePath};
use std::{cell::Cell, path::PathBuf};

use dear_imgui_rs::DockLayout;
use dmm::Size;
use prefab::Prefab;
use ui::{
    keybind::KeyBinding,
    testing::{CONTEXT_LOCK, rectangle_context},
    theme::{DEFAULT_THEME, Themes},
    widgets::button_width,
};

use super::{DEFAULT_CUSTOM_FILL_BOUNDARY, RegionSelectionOptions, UiState, menu::MenuActions};
use crate::git::panel::GitTab;
use crate::{
    environment::session::DiagnosticSeverity,
    session::{Session, fixtures::install_diff},
    settings::{
        OpenPanels, Panel, Settings,
        keybind::{KeyBindings, KeybindAction},
    },
    testing::PASSWORD,
    tool::FillMode,
};

#[test]
fn the_default_dock_layout_is_valid() {
    let state = UiState::new(false).expect("valid window keys");

    assert_eq!(state.layout.validate(), Ok(()));
    assert_eq!(state.region_selection_options, RegionSelectionOptions::default());
    assert_eq!(state.fill_mode, FillMode::Wall);
    assert_eq!(
        state.custom_fill_boundaries,
        [TreePath::parse(DEFAULT_CUSTOM_FILL_BOUNDARY)]
    );
    assert!(state.custom_fill_search.is_empty());
    assert!(state.pending_fill_warning.is_none());
    assert!(state.new_map_dialog.is_none());
    assert!(state.load_notice.is_none());
    assert_eq!(state.diagnostics.count(DiagnosticSeverity::Error), 0);
    assert!(state.save_dialog.is_none());
    assert!(!state.exit_requested);
    assert!(state.show_welcome);
    assert!(state.open_error.is_none());
    assert!(!state.keybind_preset_prompt);
}

#[test]
fn the_central_node_holds_only_the_welcome_window() {
    let state = UiState::new(false).expect("valid window keys");
    let DockLayout::Split { second, .. } = &state.layout else {
        panic!("the root is split between the object tree and everything else");
    };
    let DockLayout::Split { second, .. } = second.as_ref() else {
        panic!("the remainder is split between the inspector and the central node");
    };

    // Map views are minted per open map and dock themselves in at runtime,
    // so the declared layout cannot name them.
    assert_eq!(second.as_ref(), &DockLayout::tabs([&state.welcome_window]));
}

#[test]
fn the_git_panel_shares_the_object_tree_dock_node() {
    let state = UiState::new(false).expect("valid window keys");
    let DockLayout::Split { first, .. } = &state.layout else {
        panic!("the root is split between the object tree and everything else");
    };

    assert_eq!(
        first.as_ref(),
        &DockLayout::tabs([state.object_tree.window(), state.git_panel.window()])
    );
}

#[test]
fn the_search_panel_shares_the_inspector_dock_node() {
    let state = UiState::new(false).expect("valid window keys");
    let DockLayout::Split { second, .. } = &state.layout else {
        panic!("the root is split between the object tree and everything else");
    };
    let DockLayout::Split { first, .. } = second.as_ref() else {
        panic!("the remainder is split between the inspector and the central node");
    };

    assert_eq!(
        first.as_ref(),
        &DockLayout::tabs([state.inspector.window(), state.find.window(), state.issues.window()])
    );
}

#[test]
fn the_inspector_tab_starts_in_front_of_the_search_tab() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut context = rectangle_context();
    let flags = context.io().config_flags() | dear_imgui_rs::ConfigFlags::DOCKING_ENABLE;
    context.io_mut().set_config_flags(flags);
    let mut state = UiState::new(false).unwrap();
    let mut session = Session::new();
    let mut themes = Themes::load(None, context.style(), DEFAULT_THEME);
    let mut settings = Settings::default();
    let mut frame = |state: &mut UiState| {
        let ui = context.frame();
        state.draw(ui, &mut session, &mut settings, &mut themes, None).unwrap();
        assert!(context.render_legacy().valid());
    };

    for _ in 0..4 {
        frame(&mut state);
    }
    assert!(
        !state.select_inspector.requested(),
        "the inspector docked and took the front"
    );
    assert!(!state.find.visible(), "the Search tab waits behind the inspector");

    state.find.request();
    for _ in 0..2 {
        frame(&mut state);
    }
    assert!(state.find.visible(), "a request brings the Search tab forward");
}

fn draw_frames(context: &mut dear_imgui_rs::Context, state: &mut UiState, settings: &mut Settings, frames: usize) {
    let mut session = Session::new();
    let mut themes = Themes::load(None, context.style(), DEFAULT_THEME);
    for _ in 0..frames {
        let ui = context.frame();
        state.draw(ui, &mut session, settings, &mut themes, None).unwrap();
        assert!(context.render_legacy().valid());
    }
}

fn docking_context() -> dear_imgui_rs::Context {
    let mut context = rectangle_context();
    let flags = context.io().config_flags() | dear_imgui_rs::ConfigFlags::DOCKING_ENABLE;
    context.io_mut().set_config_flags(flags);
    context
}

/// Whether ImGui drew the window last frame, and whether it sits in a dock node.
fn window_status(key: &dear_imgui_rs::WindowKey) -> (bool, bool) {
    let name = std::ffi::CString::new(format!("###{}", key.stable_id())).unwrap();
    // SAFETY: the test's context is current between frames, and the name is NUL-terminated.
    unsafe {
        let window = dear_imgui_rs::sys::igFindWindowByID(dear_imgui_rs::sys::igImHashStr(name.as_ptr(), 0, 0));
        if window.is_null() {
            return (false, false);
        }

        let node = dear_imgui_rs::sys::igDockBuilderGetNode((*window).DockId);

        ((*window).Active, !node.is_null())
    }
}

fn toggle(panel: Panel) -> MenuActions {
    MenuActions {
        toggle_panel: Some(panel),
        ..MenuActions::default()
    }
}

#[test]
fn showing_a_panel_opens_and_focuses_it_and_never_closes_it() {
    let mut state = UiState::new(false).unwrap();
    let mut settings = Settings::default();
    let show = MenuActions {
        show_panel: Some(Panel::Git),
        ..MenuActions::default()
    };
    settings.panels.git = false;

    state.apply_window_actions(&mut settings, &show);

    assert!(settings.panels.git);
    assert!(state.git_panel.has_focus_request());

    state.apply_window_actions(&mut settings, &show);

    assert!(settings.panels.git, "showing an open panel keeps it open");
}

#[test]
fn a_closed_panel_reopens_from_the_window_menu_in_its_dock() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut context = docking_context();
    let mut state = UiState::new(false).unwrap();
    let mut settings = Settings::default();
    draw_frames(&mut context, &mut state, &mut settings, 4);
    assert_eq!(window_status(state.inspector.window()), (true, true));

    state.apply_window_actions(&mut settings, &toggle(Panel::Inspector));
    state.apply_window_actions(&mut settings, &toggle(Panel::Search));
    draw_frames(&mut context, &mut state, &mut settings, 2);
    assert!(!settings.panels.inspector && !settings.panels.search);
    assert!(
        !window_status(state.inspector.window()).0,
        "a closed inspector is not drawn"
    );
    assert!(!state.find.visible());

    state.apply_window_actions(&mut settings, &toggle(Panel::Inspector));
    draw_frames(&mut context, &mut state, &mut settings, 4);
    assert!(settings.panels.inspector);
    assert_eq!(
        window_status(state.inspector.window()),
        (true, true),
        "the inspector comes back docked"
    );
    assert!(!state.select_inspector.requested());
}

#[test]
fn a_closed_panel_releases_startup_focus() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut context = docking_context();
    let mut state = UiState::new(false).unwrap();
    let mut settings = Settings::default();
    settings.panels.object_tree = false;
    settings.panels.inspector = false;
    draw_frames(&mut context, &mut state, &mut settings, 1);

    assert!(!state.select_object_tree.requested());
    assert!(!state.select_inspector.requested());
}

#[test]
fn a_search_request_reopens_the_search_panel() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut context = docking_context();
    let mut state = UiState::new(false).unwrap();
    let mut settings = Settings::default();
    settings.panels.search = false;
    draw_frames(&mut context, &mut state, &mut settings, 4);
    assert!(!state.find.visible());

    state.find.request();
    draw_frames(&mut context, &mut state, &mut settings, 2);
    assert!(settings.panels.search);
    assert!(state.find.visible());
}

#[test]
fn resetting_the_layout_reopens_every_panel() {
    let mut state = UiState::new(false).unwrap();
    let mut settings = Settings::default();
    let open = Panel::ALL.into_iter().filter(|panel| settings.panels.is_open(*panel));
    for panel in open.collect::<Vec<_>>() {
        state.apply_window_actions(&mut settings, &toggle(panel));
    }
    assert!(Panel::ALL.into_iter().all(|panel| !settings.panels.is_open(panel)));

    let reset = MenuActions {
        reset_layout: true,
        ..MenuActions::default()
    };
    state.apply_window_actions(&mut settings, &reset);

    assert_eq!(settings.panels, OpenPanels::default());
    assert!(state.reset_layout);
}

#[test]
fn restored_floating_panels_release_startup_focus() {
    let _guard = CONTEXT_LOCK.lock().unwrap();

    for panel in ["inspector", "object-tree"] {
        let mut saved = {
            let mut context = rectangle_context();
            let flags = context.io().config_flags() | dear_imgui_rs::ConfigFlags::DOCKING_ENABLE;
            context.io_mut().set_config_flags(flags);
            let mut state = UiState::new(false).unwrap();
            let mut session = Session::new();
            let mut themes = Themes::load(None, context.style(), DEFAULT_THEME);
            let mut settings = Settings::default();
            for _ in 0..4 {
                let ui = context.frame();
                state.draw(ui, &mut session, &mut settings, &mut themes, None).unwrap();
                assert!(context.render_legacy().valid());
            }
            let mut ini = String::new();
            context.save_ini_settings(&mut ini);
            ini
        };

        // A detached panel has a saved window entry without a dock node.
        let header = format!("[Window][{panel}]");
        let start = saved.find(&header).expect("panel saved in the layout");
        let end = saved[start..].find("\n\n").map_or(saved.len(), |offset| start + offset);
        let section = &saved[start..end];
        assert!(section.contains("DockId="));
        let floating = section
            .lines()
            .filter(|line| !line.starts_with("DockId="))
            .collect::<Vec<_>>()
            .join("\n");
        saved.replace_range(start..end, &floating);

        let mut context = rectangle_context();
        let flags = context.io().config_flags() | dear_imgui_rs::ConfigFlags::DOCKING_ENABLE;
        context.io_mut().set_config_flags(flags);
        context.load_ini_settings(&saved);
        let mut state = UiState::new(false).unwrap();
        let mut session = Session::new();
        let mut themes = Themes::load(None, context.style(), DEFAULT_THEME);
        let mut settings = Settings {
            focus_windows_on_hover: false,
            ..Settings::default()
        };
        for _ in 0..4 {
            let ui = context.frame();
            state.draw(ui, &mut session, &mut settings, &mut themes, None).unwrap();
            assert!(context.render_legacy().valid());
        }
        let name = std::ffi::CString::new(format!("###{panel}")).unwrap();
        // SAFETY: the context is current between frames and the window ID is NUL-terminated.
        let window =
            unsafe { dear_imgui_rs::sys::igFindWindowByID(dear_imgui_rs::sys::igImHashStr(name.as_ptr(), 0, 0)) };
        assert!(!window.is_null(), "{panel} was not restored");
        assert_eq!(unsafe { (*window).DockId }, 0, "{panel} did not restore floating");
        let focus_pending = match panel {
            "inspector" => state.select_inspector.requested(),
            _ => state.select_object_tree.requested(),
        };
        assert!(!focus_pending, "{panel} still requests focus after restoring floating");

        let ui = context.frame();
        state.draw(ui, &mut session, &mut settings, &mut themes, None).unwrap();
        ui.window("focus-sentinel").focused(true).build(|| {});
        assert!(context.render_legacy().valid());

        let ui = context.frame();
        state.draw(ui, &mut session, &mut settings, &mut themes, None).unwrap();
        let mut retained_focus = false;
        ui.window("focus-sentinel").build(|| {
            retained_focus = ui.is_window_focused();
        });
        assert!(context.render_legacy().valid());
        assert!(retained_focus, "{panel} took focus back from another window");
    }
}

#[test]
fn resetting_the_layout_keeps_open_map_views_docked() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut context = rectangle_context();
    let flags = context.io().config_flags() | dear_imgui_rs::ConfigFlags::DOCKING_ENABLE;
    context.io_mut().set_config_flags(flags);
    let mut state = UiState::new(false).unwrap();
    state.show_welcome = false;
    let mut session = Session::new();
    let mut themes = Themes::load(None, context.style(), DEFAULT_THEME);
    session.apply_map(crate::environment::worker::LoadedMap {
        path: PathBuf::from("reset-layout-test.dmm"),
        map: dmm::Map::new(dmm::Size { x: 10, y: 10, z: 1 }),
        z: 1,
        errors: vec![],
        repo: None,
        conflict: None,
    });
    let mut settings = Settings::default();
    let mut frame = |state: &mut UiState| {
        let ui = context.frame();
        state.draw(ui, &mut session, &mut settings, &mut themes, None).unwrap();
        assert!(context.render_legacy().valid());
    };

    for _ in 0..4 {
        frame(&mut state);
    }
    assert!(state.central_node.is_some(), "the map view starts docked");

    state.reset_layout = true;
    state.central_node = None;
    for _ in 0..3 {
        frame(&mut state);
    }
    assert!(!state.reset_layout);
    assert!(
        state.central_node.is_some(),
        "the map view is docked again after the reset"
    );
}

/// Whether ImGui has the window docked in the dockspace's central node.
fn docked_in_the_center(key: &dear_imgui_rs::WindowKey) -> bool {
    let name = std::ffi::CString::new(format!("###{}", key.stable_id())).unwrap();
    // SAFETY: the test's context is current between frames, and the name is NUL-terminated.
    unsafe {
        let window = dear_imgui_rs::sys::igFindWindowByID(dear_imgui_rs::sys::igImHashStr(name.as_ptr(), 0, 0));
        if window.is_null() {
            return false;
        }

        let node = dear_imgui_rs::sys::igDockBuilderGetNode((*window).DockId);

        !node.is_null() && dear_imgui_rs::sys::ImGuiDockNode_IsCentralNode(node)
    }
}

#[test]
fn a_new_map_view_ignores_a_saved_dock_node_that_no_longer_exists() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut context = rectangle_context();
    let flags = context.io().config_flags() | dear_imgui_rs::ConfigFlags::DOCKING_ENABLE;
    context.io_mut().set_config_flags(flags);
    let mut state = UiState::new(false).unwrap();
    let mut session = Session::new();
    let mut themes = Themes::load(None, context.style(), DEFAULT_THEME);
    let mut settings = Settings::default();
    for _ in 0..3 {
        let ui = context.frame();
        state.draw(ui, &mut session, &mut settings, &mut themes, None).unwrap();
        assert!(context.render_legacy().valid());
    }

    state.reset_layout = true;
    session.apply_map(crate::environment::worker::LoadedMap {
        path: PathBuf::from("saved-dock.dmm"),
        map: dmm::Map::new(dmm::Size { x: 10, y: 10, z: 1 }),
        z: 1,
        errors: vec![],
        repo: None,
        conflict: None,
    });
    let id = session.state.active().unwrap();
    // map view keys restart with the document ids every launch, so the ini remembers where an
    // earlier session's view with this number was docked, in a node the reset does not rebuild
    context.load_ini_settings(&format!(
        "[Window][###viewport-{}]\nPos=0,0\nSize=400,300\nCollapsed=0\nDockId=0x0000ABCD,0\n",
        id.get()
    ));
    for _ in 0..4 {
        let ui = context.frame();
        state.draw(ui, &mut session, &mut settings, &mut themes, None).unwrap();
        assert!(context.render_legacy().valid());
    }

    assert!(
        docked_in_the_center(state.map_views[&id].window()),
        "the new map view floats"
    );
}

#[test]
fn a_search_request_brings_its_tab_forward_while_the_mouse_is_over_the_map() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut context = rectangle_context();
    let flags = context.io().config_flags() | dear_imgui_rs::ConfigFlags::DOCKING_ENABLE;
    context.io_mut().set_config_flags(flags);
    let mut state = UiState::new(false).unwrap();
    let mut session = Session::new();
    let mut themes = Themes::load(None, context.style(), DEFAULT_THEME);
    session.apply_map(crate::environment::worker::LoadedMap {
        path: PathBuf::from("search-focus-test.dmm"),
        map: dmm::Map::new(dmm::Size { x: 10, y: 10, z: 1 }),
        z: 1,
        errors: vec![],
        repo: None,
        conflict: None,
    });
    let mut settings = Settings::default();
    assert!(settings.focus_windows_on_hover);
    let mut frame = |state: &mut UiState| {
        context.io_mut().add_mouse_pos_event([400.0, 300.0]);
        let ui = context.frame();
        state.draw(ui, &mut session, &mut settings, &mut themes, None).unwrap();
        assert!(context.render_legacy().valid());
    };

    for _ in 0..4 {
        frame(&mut state);
    }
    assert!(!state.find.visible());

    state.find.request();
    for _ in 0..3 {
        frame(&mut state);
    }
    assert!(state.find.visible(), "the map view must not take the focus back first");
}

#[test]
fn the_object_tree_tab_starts_in_front_of_the_git_tab() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut context = rectangle_context();
    let flags = context.io().config_flags() | dear_imgui_rs::ConfigFlags::DOCKING_ENABLE;
    context.io_mut().set_config_flags(flags);
    let mut state = UiState::new(false).unwrap();
    let mut session = Session::new();
    let mut themes = Themes::load(None, context.style(), DEFAULT_THEME);
    let mut settings = Settings::default();
    let mut frame = |state: &mut UiState| {
        let ui = context.frame();
        state.draw(ui, &mut session, &mut settings, &mut themes, None).unwrap();
        assert!(context.render_legacy().valid());
    };

    for _ in 0..4 {
        frame(&mut state);
    }
    assert!(
        !state.select_object_tree.requested(),
        "the object tree docked and took the front"
    );
    assert!(!state.git_panel.visible(), "the Git tab waits behind the object tree");

    state.git_panel.request(GitTab::Conflicts);
    for _ in 0..2 {
        frame(&mut state);
    }
    assert!(state.git_panel.visible(), "a request brings the Git tab forward");
}

#[test]
fn the_diff_tab_draws_the_versions_and_the_changes() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut context = rectangle_context();
    let flags = context.io().config_flags() | dear_imgui_rs::ConfigFlags::DOCKING_ENABLE;
    context.io_mut().set_config_flags(flags);
    let mut state = UiState::new(false).unwrap();
    let mut session = Session::new();
    let mut settings = Settings::default();
    let mut map = dmm::Map::new(Size { x: 4, y: 4, z: 1 });
    let floor = map.intern_tile(vec![Prefab::new(TreePath::parse("/turf/floor"))]);
    for row in &mut map.grid[0] {
        row.fill(floor);
    }
    session.apply_map(crate::environment::worker::LoadedMap {
        path: PathBuf::from("diff-ui-test.dmm"),
        map,
        z: 1,
        errors: vec![],
        repo: Some(crate::git::repo::RepoPath {
            root: PathBuf::from("missing-repository"),
            git_dir: PathBuf::from("missing-repository/.git"),
            rel: String::from("diff-ui-test.dmm"),
        }),
        conflict: None,
    });
    let id = session.state.active().unwrap();
    install_diff(&mut session, id);
    let mut frame = |state: &mut UiState, session: &mut Session| {
        let ui = context.frame();
        let mut themes = Themes::load(None, &ui.clone_style(), DEFAULT_THEME);
        state.draw(ui, session, &mut settings, &mut themes, None).unwrap();
        assert!(context.render_legacy().valid());
    };

    // the dock layout settles before the Git tab can be brought forward
    for _ in 0..4 {
        frame(&mut state, &mut session);
    }
    state.git_panel.request(GitTab::Diff);
    for _ in 0..3 {
        frame(&mut state, &mut session);
    }

    assert!(state.git_panel.visible());
    assert_eq!(session.diff(id).map(|diff| diff.len()), Some(1));
}

#[test]
fn an_out_of_date_map_asks_in_a_popup_until_the_user_leaves() {
    use std::{
        thread,
        time::{Duration, Instant},
    };

    let _guard = CONTEXT_LOCK.lock().unwrap();
    let entry = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env/test.dme");
    let open = |width: u32| {
        let mut session = Session::new();
        session.load_environment(&entry).unwrap();
        let path = session.codebase_dir().unwrap().join("_maps/out-of-date.dmm");
        session.apply_map(crate::environment::worker::LoadedMap {
            path,
            map: dmm::Map::new(Size { x: width, y: 1, z: 1 }),
            z: 1,
            errors: vec![],
            repo: None,
            conflict: None,
        });
        let id = session.state.active().unwrap();

        (session, id)
    };
    let poll_until = |sessions: &mut [&mut Session], done: &dyn Fn(&[&mut Session]) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done(sessions) {
            assert!(Instant::now() < deadline, "co-op never got there");
            for session in sessions.iter_mut() {
                session.poll_coop();
            }

            thread::sleep(Duration::from_millis(5));
        }
    };

    let (mut host, _) = open(2);
    host.host_coop(0, net::hash_password(PASSWORD), String::from("host"))
        .unwrap();
    poll_until(&mut [&mut host], &|sessions| {
        sessions[0].coop().is_some_and(|coop| coop.is_connected())
    });
    host.share_coop_map();
    poll_until(&mut [&mut host], &|sessions| {
        sessions[0].coop().is_some_and(|coop| !coop.shared_maps.is_empty())
    });

    let (mut guest, local) = open(1);
    guest.state.document_mut(local).unwrap().mark_unsaved();
    let port = host.coop().and_then(|coop| coop.host()).unwrap().port();
    guest
        .join_coop(
            format!("127.0.0.1:{port}"),
            net::hash_password(PASSWORD),
            String::from("guest"),
        )
        .unwrap();
    poll_until(&mut [&mut host, &mut guest], &|sessions| {
        sessions[1].coop_out_of_date(local).is_some()
    });

    let mut context = rectangle_context();
    let flags = context.io().config_flags() | dear_imgui_rs::ConfigFlags::DOCKING_ENABLE;
    context.io_mut().set_config_flags(flags);
    let mut state = UiState::new(false).unwrap();
    let mut settings = Settings {
        check_for_updates: false,
        ..Settings::default()
    };
    let mut frame = |state: &mut UiState, session: &mut Session| {
        let ui = context.frame();
        let mut themes = Themes::load(None, &ui.clone_style(), DEFAULT_THEME);
        state.draw(ui, session, &mut settings, &mut themes, None).unwrap();
        let is_open = ui.is_popup_open(crate::coop::panel::OUT_OF_DATE_POPUP);
        assert!(context.render_legacy().valid());
        is_open
    };
    assert!((0..3).map(|_| frame(&mut state, &mut guest)).last().unwrap());

    guest.leave_coop();
    assert!(!(0..3).map(|_| frame(&mut state, &mut guest)).last().unwrap());
    let document = guest.state.document(local).unwrap();
    assert!(document.is_dirty());
    assert!(!document.is_read_only());
    assert_eq!(document.map.size().x, 1);
}

#[test]
fn the_codebase_dialog_supports_retry_close_and_escape_without_maps() {
    use dear_imgui_rs::{Key, MouseButton, sys};

    use super::OpenRequest;
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut context = rectangle_context();
    let flags = context.io().config_flags() | dear_imgui_rs::ConfigFlags::DOCKING_ENABLE;
    context.io_mut().set_config_flags(flags);
    let mut state = UiState::new(false).unwrap();
    let mut session = Session::new();
    let original = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env/test.dme");
    session.load_environment(&original).unwrap();
    session
        .join_coop("not a host".into(), net::hash_password("password"), "guest".into())
        .unwrap();
    let dir = std::env::temp_dir().join(format!("rmd-codebase-notice-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("game.dme"), "/obj/different").unwrap();
    session.load_environment(&dir.join("game.dme")).unwrap();
    assert!(matches!(
        session.coop().unwrap().status,
        crate::coop::CoopStatus::CodebaseMismatch { .. }
    ));
    assert!(session.state.is_empty());
    let mut settings = Settings {
        check_for_updates: false,
        ..Settings::default()
    };
    let load = crate::environment::worker::LoadView {
        title: "Opening codebase",
        path: "game.dme".into(),
        snapshot: crate::environment::progress::Progress::new().snapshot(),
        cancelling: false,
        cancellable: true,
    };
    let widths = Cell::new([0.0_f32; 3]);
    let frame = |context: &mut dear_imgui_rs::Context,
                 state: &mut UiState,
                 session: &mut Session,
                 settings: &mut Settings,
                 loading: bool| {
        let ui = context.frame();
        let mut themes = Themes::load(None, &ui.clone_style(), DEFAULT_THEME);
        let output = state
            .draw(ui, session, settings, &mut themes, loading.then_some(&load))
            .unwrap();
        widths.set([
            button_width(ui, "Reload Codebase and Retry"),
            button_width(ui, "Close"),
            ui.clone_style().item_spacing()[0],
        ]);
        let is_open = ui.is_popup_open(crate::coop::panel::STATUS_POPUP);
        assert!(context.render_legacy().valid());
        (output.open, is_open)
    };
    for _ in 0..4 {
        assert_eq!(
            frame(&mut context, &mut state, &mut session, &mut settings, true),
            (None, false),
            "the dialog waits for loading to finish"
        );
    }

    let work_pos = context.main_viewport().work_pos();
    for _ in 0..4 {
        assert_eq!(
            frame(&mut context, &mut state, &mut session, &mut settings, false),
            (None, true)
        );
    }

    let (position, size) = unsafe {
        let window = sys::igFindWindowByName(c"Co-op unavailable##coop-status".as_ptr());
        assert!(!window.is_null());
        assert_ne!((*window).Flags & sys::ImGuiWindowFlags_Modal, 0);
        ([(*window).Pos.x, (*window).Pos.y], [(*window).Size.x, (*window).Size.y])
    };
    assert_eq!(
        context.main_viewport().work_pos(),
        work_pos,
        "the modal preserves the dockspace layout"
    );
    let padding = context.style().window_padding();
    let [retry_width, close_width, spacing] = widths.get();
    let y = position[1] + size[1] - padding[1] - 8.0;
    let right = position[0] + size[0] - padding[0];
    let retry = [right - retry_width / 2.0, y];
    let close = [right - retry_width - spacing - close_width / 2.0, y];
    for down in [false, true, false] {
        context.io_mut().add_mouse_pos_event(retry);
        context.io_mut().add_mouse_button_event(MouseButton::Left, down);
        assert_eq!(
            frame(&mut context, &mut state, &mut session, &mut settings, true),
            (None, true),
            "retry is disabled while loading"
        );
    }

    let mut requested = None;
    for down in [false, true, false] {
        context.io_mut().add_mouse_pos_event(retry);
        context.io_mut().add_mouse_button_event(MouseButton::Left, down);
        requested = frame(&mut context, &mut state, &mut session, &mut settings, false)
            .0
            .or(requested);
    }

    assert_eq!(requested, Some(OpenRequest::ReloadCoop));
    for _ in 0..2 {
        assert_eq!(
            frame(&mut context, &mut state, &mut session, &mut settings, false),
            (None, false),
            "the previous failure must not reopen during a retry"
        );
    }

    state.reset_coop_notice();
    for _ in 0..3 {
        assert_eq!(
            frame(&mut context, &mut state, &mut session, &mut settings, false),
            (None, true),
            "a repeated mismatch opens the dialog for the new attempt"
        );
    }

    for down in [false, true, false] {
        context.io_mut().add_mouse_pos_event(close);
        context.io_mut().add_mouse_button_event(MouseButton::Left, down);
        frame(&mut context, &mut state, &mut session, &mut settings, false);
    }

    assert!(session.coop().is_none());

    session
        .join_coop("not a host".into(), net::hash_password("password"), "guest".into())
        .unwrap();
    session.load_environment(&original).unwrap();
    for _ in 0..3 {
        assert_eq!(
            frame(&mut context, &mut state, &mut session, &mut settings, false),
            (None, true)
        );
    }

    context.io_mut().add_key_event(Key::Escape, true);
    assert_eq!(
        frame(&mut context, &mut state, &mut session, &mut settings, false),
        (None, false)
    );
    assert!(session.coop().is_none());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn editing_commands_are_bound_to_the_usual_chords() {
    let bindings = KeyBindings::default();

    assert_eq!(
        bindings.get(KeybindAction::Save),
        KeyBinding::with_ctrl(dear_imgui_rs::Key::S)
    );
    assert_eq!(
        bindings.get(KeybindAction::Undo),
        KeyBinding::with_ctrl(dear_imgui_rs::Key::Z)
    );
    assert_eq!(
        bindings.get(KeybindAction::Redo),
        KeyBinding::with_ctrl(dear_imgui_rs::Key::Y)
    );
    assert_eq!(
        bindings.get(KeybindAction::Copy),
        KeyBinding::with_ctrl(dear_imgui_rs::Key::C)
    );
    assert_eq!(
        bindings.get(KeybindAction::Paste),
        KeyBinding::with_ctrl(dear_imgui_rs::Key::V)
    );
    assert_eq!(
        bindings.get(KeybindAction::NodeTool),
        KeyBinding::new(dear_imgui_rs::Key::N)
    );
    assert_eq!(
        bindings.get(KeybindAction::MoveTool),
        KeyBinding::new(dear_imgui_rs::Key::M)
    );
    // Every action is reachable from the settings list, or it cannot be rebound.
    assert_eq!(KeybindAction::ALL.len(), 64);
    assert_eq!(KeybindAction::RECENT.len(), 10);
    assert!(KeybindAction::ALL.contains(&KeybindAction::Save));
    assert!(KeybindAction::ALL.contains(&KeybindAction::Undo));
    assert!(KeybindAction::ALL.contains(&KeybindAction::Redo));
    assert!(KeybindAction::ALL.contains(&KeybindAction::Copy));
    assert!(KeybindAction::ALL.contains(&KeybindAction::Paste));
    assert!(KeybindAction::ALL.contains(&KeybindAction::NodeTool));
    assert!(KeybindAction::ALL.contains(&KeybindAction::MoveTool));
    assert!(KeybindAction::ALL.contains(&KeybindAction::ShowLighting));
    assert!(KeybindAction::ALL.contains(&KeybindAction::Recent1));
    assert!(KeybindAction::ALL.contains(&KeybindAction::Recent0));
}

fn stash_region(session: &mut Session) -> crate::document::DocumentId {
    let mut map = dmm::Map::new(Size { x: 2, y: 2, z: 1 });
    let key = map.intern_tile(vec![Prefab::new(TreePath::parse("/turf/open/floor"))]);
    for row in &mut map.grid[0] {
        row.fill(key);
    }

    let document = crate::document::MapDocument::new(map, 1);
    let selection = crate::document::selection::Selection::from_drag(Coord::new(1, 1, 1), Coord::new(2, 2, 1));
    let region = crate::clipboard::copy_region(
        (&document.map).into(),
        selection,
        crate::tool::RegionSelectionMode::Full,
        crate::frame::options::HiddenTypes::default(),
    )
    .unwrap();

    session.state.stash_mut().push(region)
}

#[test]
fn the_stash_view_follows_its_panel_and_hands_the_active_map_back() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut context = docking_context();
    let mut state = UiState::new(false).unwrap();
    state.show_welcome = false;
    let mut session = Session::new();
    let mut themes = Themes::load(None, context.style(), DEFAULT_THEME);
    let map = session.state.open_document(crate::document::MapDocument::new(
        dmm::Map::new(Size { x: 4, y: 4, z: 1 }),
        1,
    ));
    let entry = stash_region(&mut session);
    let mut settings = Settings::default();
    settings.panels.stash = true;

    for _ in 0..3 {
        let ui = context.frame();
        state.draw(ui, &mut session, &mut settings, &mut themes, None).unwrap();
        assert!(context.render_legacy().valid());
    }
    assert!(state.map_views.contains_key(&entry));
    assert!(state.map_views.contains_key(&map));

    assert!(session.set_active_document(entry));
    settings.panels.stash = false;
    let ui = context.frame();
    state.draw(ui, &mut session, &mut settings, &mut themes, None).unwrap();
    assert!(context.render_legacy().valid());
    assert!(!state.map_views.contains_key(&entry));
    assert_eq!(session.state.active(), Some(map));
}

#[test]
fn stash_entries_sit_side_by_side_in_one_view_without_any_map_open() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut context = rectangle_context();
    let mut state = UiState::new(false).unwrap();
    let mut session = Session::new();
    let entries = [stash_region(&mut session), stash_region(&mut session)];
    let mut settings = Settings::default();
    settings.panels.stash = true;

    let mut views = Vec::new();
    for _ in 0..3 {
        let ui = context.frame();
        (views, ..) = state.draw_map_views(ui, &mut session, &mut settings, false);
        assert!(context.render_legacy().valid());
    }

    assert_eq!(views.iter().map(|view| view.document).collect::<Vec<_>>(), entries);
    assert!(settings.panels.stash);

    let [first, second] = [views[0].rect, views[1].rect];
    assert!(second.x >= first.x + first.width, "{first:?} {second:?}");
    assert_eq!(first.y + first.height, second.y + second.height);
}

#[test]
fn exiting_with_a_stashed_region_asks_first() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut context = rectangle_context();
    let mut state = UiState::new(false).unwrap();
    let mut session = Session::new();

    state.request_exit();
    assert!(state.draw_exit_confirmation(context.frame(), &session));
    assert!(context.render_legacy().valid());

    stash_region(&mut session);
    state.request_exit();
    assert!(!state.draw_exit_confirmation(context.frame(), &session));
    assert!(state.exit_requested);
    assert!(context.render_legacy().valid());
}
