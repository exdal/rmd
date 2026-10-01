use super::*;

fn open_blank_map(session: &mut Session, path: PathBuf) -> DocumentId {
    session.apply_map(crate::loader::LoadedMap {
        path,
        map: dmm::Map::new(Size { x: 20, y: 20, z: 1 }),
        z: 1,
        errors: vec![],
        repo: None,
        conflict: None,
    });

    session.state.active().unwrap()
}

fn presence_frame(
    context: &mut dear_imgui_rs::Context, state: &mut UiState, session: &mut Session, settings: &mut Settings,
) -> CoopPresence {
    context.io_mut().add_mouse_pos_event([400.0, 300.0]);
    let ui = context.frame();
    for id in session.state.document_ids() {
        let name = format!("###viewport-{}", id.get());
        ui.set_window_pos_by_name(&name, [0.0; 2]);
        ui.set_window_size_by_name(&name, [800.0, 600.0]);
    }

    let (_, _, presence) = state.draw_map_views(ui, session, settings, false);
    assert!(context.render_legacy().valid());

    presence
}

#[test]
fn the_coop_cursor_only_follows_shared_maps() {
    let _guard = IMGUI_CONTEXT.lock().unwrap();
    let mut session = Session::new();
    session
        .load_environment(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env/test.dme"))
        .unwrap();
    let file = session.codebase_dir().unwrap().join("_maps/cursor.dmm");
    let shared = open_blank_map(&mut session, file);
    let outside = open_blank_map(&mut session, std::env::temp_dir().join("rmd-cursor-outside.dmm"));

    let mut context = rectangle_context();
    let mut state = UiState::new(false).unwrap();
    let mut settings = Settings::default();
    presence_frame(&mut context, &mut state, &mut session, &mut settings);

    let mut focus = |state: &mut UiState, session: &mut Session, id| {
        state.map_views.get_mut(&id).unwrap().focus = true;
        (0..3)
            .map(|_| presence_frame(&mut context, state, session, &mut settings).cursor)
            .last()
            .flatten()
    };

    assert_eq!(
        focus(&mut state, &mut session, shared),
        None,
        "the map is not shared yet"
    );
    assert_eq!(session.state.active(), Some(shared));

    session
        .host_coop(0, String::from("hunter2"), String::from("host"))
        .unwrap();
    let poll_until = |session: &mut Session, done: &dyn Fn(&Session) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done(session) {
            assert!(Instant::now() < deadline, "co-op never got there");
            session.poll_coop();
            thread::sleep(Duration::from_millis(5));
        }
    };
    poll_until(&mut session, &|session| {
        session.coop().is_some_and(|coop| coop.is_connected())
    });
    session.share_coop_map();
    poll_until(&mut session, &|session| session.coop_shared_map_path(shared).is_some());

    let cursor = focus(&mut state, &mut session, shared);
    assert_eq!(cursor.map(|cursor| cursor.map).as_deref(), Some("_maps/cursor.dmm"));

    assert_eq!(focus(&mut state, &mut session, outside), None);
    assert_eq!(session.state.active(), Some(outside));
    session.leave_coop();
}

fn follow_frame(context: &mut dear_imgui_rs::Context, state: &mut UiState, session: &mut Session) {
    let ui = context.frame();
    state.follow_coop_peer(ui, session);
    assert!(context.render_legacy().valid());
}

#[test]
fn following_snaps_the_camera_to_the_peer_and_stops_when_the_user_moves_it() {
    let _guard = IMGUI_CONTEXT.lock().unwrap();
    let load = || {
        let mut session = Session::new();
        session
            .load_environment(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env/test.dme"))
            .unwrap();

        session
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

    let mut host = load();
    let file = host.codebase_dir().unwrap().join("_maps/follow.dmm");
    let shared = open_blank_map(&mut host, file);
    host.host_coop(0, String::from("hunter2"), String::from("host"))
        .unwrap();
    poll_until(&mut [&mut host], &|sessions| {
        sessions[0].coop().is_some_and(|coop| coop.is_connected())
    });
    host.share_coop_map();
    poll_until(&mut [&mut host], &|sessions| {
        sessions[0].coop_shared_map_path(shared).is_some()
    });
    let outside = open_blank_map(&mut host, std::env::temp_dir().join("rmd-follow-outside.dmm"));

    let mut guest = load();
    let port = host.coop().and_then(|coop| coop.host()).unwrap().0.port();
    guest
        .join_coop(
            format!("127.0.0.1:{port}"),
            String::from("hunter2"),
            String::from("guest"),
        )
        .unwrap();
    poll_until(&mut [&mut host, &mut guest], &|sessions| {
        sessions[1].coop().is_some_and(|coop| coop.is_connected())
    });
    let view = net::View {
        map: String::from("_maps/follow.dmm"),
        z: 1,
        center: [96.0, 64.0],
        zoom: 3.0,
    };
    guest.coop_view(Some(view.clone()));
    poll_until(&mut [&mut host, &mut guest], &|sessions| {
        sessions[0]
            .coop()
            .is_some_and(|coop| coop.peers.values().any(|peer| peer.view.is_some()))
    });

    let mut context = rectangle_context();
    let mut state = UiState::new(false).unwrap();
    let mut settings = Settings::default();
    presence_frame(&mut context, &mut state, &mut host, &mut settings);
    assert_eq!(host.state.active(), Some(outside));

    let peer = *host.coop().unwrap().peers.keys().next().unwrap();
    host.toggle_coop_follow(peer);
    {
        let ui = context.frame();
        ui.open_popup("blocker");
        let popup = ui.begin_popup("blocker").expect("the popup is open");
        state.follow_coop_peer(ui, &mut host);
        ui.close_current_popup();
        drop(popup);

        assert!(context.render_legacy().valid());
    }
    assert_eq!(host.state.active(), Some(outside), "a dialog keeps following waiting");

    follow_frame(&mut context, &mut state, &mut host);
    let camera = state.map_views[&shared].camera.camera;
    assert_eq!(host.state.active(), Some(shared));
    assert_eq!([camera.x, camera.y, camera.zoom], [96.0, 64.0, 3.0]);
    state.stop_following_when_moved(&mut host);
    assert_eq!(host.coop_following(), Some(peer));

    state.map_views.get_mut(&shared).unwrap().refit = true;
    follow_frame(&mut context, &mut state, &mut host);
    assert!(
        !state.map_views[&shared].refit,
        "a pending refit would move the followed camera"
    );
    state.stop_following_when_moved(&mut host);

    follow_frame(&mut context, &mut state, &mut host);
    state.map_views.get_mut(&shared).unwrap().camera.pan_by([10.0, 0.0]);
    state.stop_following_when_moved(&mut host);
    assert_eq!(host.coop_following(), None);

    guest.leave_coop();
    host.leave_coop();
}
