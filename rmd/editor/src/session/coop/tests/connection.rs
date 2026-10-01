use super::*;

#[test]
fn a_joined_peer_sees_the_hosts_cursor() {
    let mut host = session();
    host.host_coop(0, net::hash_password("hunter2"), String::from("host"))
        .unwrap();
    poll_until(&mut [&mut host], |sessions| connected(sessions[0]));

    let addr = format!("127.0.0.1:{}", host.coop().and_then(Coop::host).unwrap().port());
    let mut guest = session();
    guest
        .join_coop(addr, net::hash_password("hunter2"), String::from("guest"))
        .unwrap();
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        connected(sessions[1]) && sessions[0].coop().is_some_and(|coop| coop.peers.len() == 1)
    });

    let peer = guest.coop().unwrap().peers.values().next().unwrap();
    assert_eq!(peer.info.nick, "host");
    assert_eq!(peer.info.codebase.hash, guest.coop().unwrap().local_codebase().hash);

    let cursor = Cursor {
        map: String::from("_maps/a.dmm"),
        z: 1,
        pos: [32.0, 64.0],
        tool: net::Tool::Fill,
    };
    host.coop_cursor(Some(cursor.clone()));
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        sessions[1]
            .coop()
            .is_some_and(|coop| coop.peers.values().any(|peer| peer.cursor.as_ref() == Some(&cursor)))
    });

    guest.leave_coop();
    poll_until(&mut [&mut host], |sessions| {
        sessions[0].coop().is_some_and(|coop| coop.peers.is_empty())
    });
}

#[test]
fn peers_joining_and_leaving_are_recorded() {
    let mut host = session();
    host.host_coop(0, net::hash_password("hunter2"), String::from("host"))
        .unwrap();
    poll_until(&mut [&mut host], |sessions| connected(sessions[0]));

    let port = host.coop().and_then(Coop::host).unwrap().port();
    let mut guest = session();
    guest
        .join_coop(
            format!("127.0.0.1:{port}"),
            net::hash_password("hunter2"),
            String::from("guest"),
        )
        .unwrap();
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        connected(sessions[1]) && sessions[0].coop().is_some_and(|coop| coop.peers.len() == 1)
    });

    let changes = &host.coop().unwrap().activity;
    assert_eq!(changes.len(), 1);
    assert!(matches!(&changes[0].1, Activity::Joined(info) if info.nick == "guest"));
    assert!(guest.coop().unwrap().activity.is_empty());

    guest.leave_coop();
    poll_until(&mut [&mut host], |sessions| {
        sessions[0]
            .coop()
            .and_then(|coop| coop.activity.back())
            .is_some_and(|(_, change)| matches!(change, Activity::Left(info) if info.nick == "guest"))
    });
}

#[test]
fn a_wrong_password_ends_the_session_with_the_reason() {
    let mut host = session();
    host.host_coop(0, net::hash_password("hunter2"), String::from("host"))
        .unwrap();
    let port = host.coop().and_then(Coop::host).unwrap().port();

    let mut guest = session();
    guest
        .join_coop(
            format!("127.0.0.1:{port}"),
            net::hash_password("nope"),
            String::from("guest"),
        )
        .unwrap();
    poll_until(&mut [&mut guest], |sessions| {
        sessions[0]
            .coop()
            .is_some_and(|coop| matches!(&coop.status, CoopStatus::Ended(reason) if reason.contains("wrong password")))
    });
}

#[test]
fn remote_cursors_glide_but_jump_across_maps_and_levels() {
    let cursor = |map: &str, z, x| Cursor {
        map: String::from(map),
        z,
        pos: [x, 0.0],
        tool: net::Tool::Select,
    };
    let mut peer = RemotePeer::new(PeerInfo {
        id: PeerId(1),
        nick: String::from("peer"),
        codebase: CodebaseId {
            hash: CodebaseHash([0; 32]),
            git_hint: None,
        },
    });

    peer.set_cursor(Some(cursor("a.dmm", 1, 100.0)));
    assert_eq!(peer.shown, [100.0, 0.0]);

    peer.set_cursor(Some(cursor("a.dmm", 1, 200.0)));
    peer.follow(0.02);
    assert!(peer.shown[0] > 100.0 && peer.shown[0] < 200.0);

    peer.follow(1.0);
    assert!((peer.shown[0] - 200.0).abs() < 0.01);

    peer.set_cursor(Some(cursor("a.dmm", 2, 900.0)));
    assert_eq!(peer.shown, [900.0, 0.0]);

    peer.set_cursor(None);
    peer.set_cursor(Some(cursor("a.dmm", 2, 50.0)));
    assert_eq!(peer.shown, [50.0, 0.0]);
}

#[test]
fn the_comment_tool_only_works_while_connected() {
    let mut host = session();
    host.set_tool(Tool::Comment);
    assert_eq!(host.tool(), Tool::Select);

    host.host_coop(0, net::hash_password("hunter2"), String::from("host"))
        .unwrap();
    poll_until(&mut [&mut host], |sessions| connected(sessions[0]));
    host.set_tool(Tool::Comment);
    assert_eq!(host.tool(), Tool::Comment);

    host.leave_coop();
    host.poll_coop();
    assert_eq!(host.tool(), Tool::Select);
}

#[test]
fn cursors_and_comments_need_a_shared_map() {
    let (host_dir, mut host) = codebase_with_map("unshared-host", "aa");
    let (guest_dir, mut guest) = codebase_with_map("unshared-guest", "aa");
    let host_map = open_local(&mut host, host_dir.join("_maps/a.dmm"));
    let guest_map = open_local(&mut guest, guest_dir.join("_maps/a.dmm"));
    host.host_coop(0, net::hash_password("hunter2"), String::from("host"))
        .unwrap();
    poll_until(&mut [&mut host], |sessions| connected(sessions[0]));
    let port = host.coop().and_then(Coop::host).unwrap().port();
    guest
        .join_coop(
            format!("127.0.0.1:{port}"),
            net::hash_password("hunter2"),
            String::from("guest"),
        )
        .unwrap();
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        connected(sessions[1]) && sessions[0].coop().is_some_and(|coop| coop.peers.len() == 1)
    });

    assert_eq!(host.coop_shared_map_path(host_map), None);
    assert_eq!(guest.coop_shared_map_path(guest_map), None);
    host.add_coop_comment(host_map, [0.0, 0.0], String::from("unshared"));

    host.share_coop_map();
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        settled(sessions)
            && sessions.iter().all(|session| {
                session
                    .coop()
                    .is_some_and(|coop| coop.shared_maps.contains_key("_maps/a.dmm"))
            })
    });
    let guest_map = guest.state.document_for_path(&guest_dir.join("_maps/a.dmm")).unwrap();
    assert_eq!(host.coop_shared_map_path(host_map).as_deref(), Some("_maps/a.dmm"));
    assert_eq!(guest.coop_shared_map_path(guest_map).as_deref(), Some("_maps/a.dmm"));

    host.add_coop_comment(host_map, [0.0, 0.0], String::from("shared"));
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        sessions[1].coop().is_some_and(|coop| !coop.comments.is_empty())
    });
    let texts = guest
        .coop()
        .unwrap()
        .comments
        .values()
        .map(|comment| comment.text.as_str())
        .collect::<Vec<_>>();
    assert_eq!(texts, ["shared"]);

    let _ = fs::remove_dir_all(host_dir);
    let _ = fs::remove_dir_all(guest_dir);
}

#[test]
fn coop_needs_a_codebase() {
    assert!(
        Session::new()
            .join_coop(
                String::from("127.0.0.1:1"),
                net::hash_password("pw"),
                String::from("me")
            )
            .is_err()
    );
}

fn following(name: &str) -> (PathBuf, Session, DocumentId, DocumentId) {
    let (dir, mut session) = hosting(name);
    fs::write(
        dir.join("_maps/b.dmm"),
        "\"a\" = (/turf,/area)\n\n(1,1,1) = {\"\na\n\"}\n(1,1,2) = {\"\na\n\"}\n",
    )
    .unwrap();

    let mut maps = Vec::new();
    for file in ["_maps/a.dmm", "_maps/b.dmm"] {
        let id = open_local(&mut session, dir.join(file));
        session.share_coop_map();
        poll_until(&mut [&mut session], |sessions| {
            sessions[0].coop_shared_map_path(id).is_some() && settled(sessions)
        });
        maps.push(id);
    }

    let codebase = session.coop().unwrap().local_codebase().clone();
    deliver(
        &mut session,
        &dir,
        Event::PeerJoined(PeerInfo {
            id: OTHER,
            nick: String::from("other"),
            codebase,
        }),
    );

    (dir, session, maps[0], maps[1])
}

fn look_at(session: &mut Session, dir: &Path, map: &str, z: u32) {
    deliver(
        session,
        dir,
        Event::View {
            from: OTHER,
            view: Some(View {
                map: String::from(map),
                z,
                center: [16.0, 16.0],
                zoom: 2.0,
            }),
        },
    );
}

#[test]
fn following_a_peer_brings_their_map_and_level_forward() {
    let (dir, mut session, a, b) = following("follow");
    assert_eq!(session.state.active(), Some(b));
    look_at(&mut session, &dir, "_maps/a.dmm", 1);

    session.toggle_coop_follow(OTHER);
    assert_eq!(session.coop_following(), Some(OTHER));
    let (document, view) = session.coop_follow_target().unwrap();
    assert_eq!((document, view.zoom), (a, 2.0));
    assert_eq!(session.state.active(), Some(a));

    look_at(&mut session, &dir, "_maps/b.dmm", 2);
    assert_eq!(session.coop_follow_target().map(|(document, _)| document), Some(b));
    assert_eq!(session.state.active(), Some(b));
    assert_eq!(session.state.document(b).unwrap().z, 2);

    session.toggle_coop_follow(OTHER);
    assert_eq!(session.coop_following(), None);
    assert!(session.coop_follow_target().is_none());

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn switching_documents_or_the_peer_leaving_stops_following() {
    let (dir, mut session, a, b) = following("follow-stop");
    look_at(&mut session, &dir, "_maps/a.dmm", 1);
    session.toggle_coop_follow(OTHER);
    assert!(session.coop_follow_target().is_some());

    session.set_active_document(b);
    assert!(session.coop_follow_target().is_none());
    assert_eq!(session.coop_following(), None);
    assert_eq!(session.state.active(), Some(b));

    session.toggle_coop_follow(OTHER);
    deliver(&mut session, &dir, Event::PeerLeft(OTHER));
    assert_eq!(session.coop_following(), None);

    let you = session.coop().unwrap().you.unwrap();
    session.toggle_coop_follow(you);
    assert_eq!(session.coop_following(), None);
    assert_eq!(session.state.active(), Some(b));
    assert!(session.state.document(a).is_some());

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn following_reopens_a_closed_shared_map() {
    let (dir, mut session, a, _) = following("follow-reopen");
    let file = dir.join("_maps/a.dmm");
    session.close_map(a);
    session.poll_coop();
    assert!(session.state.document_for_path(&file).is_none());

    look_at(&mut session, &dir, "_maps/a.dmm", 1);
    session.toggle_coop_follow(OTHER);
    let (document, _) = session.coop_follow_target().unwrap();
    assert_eq!(session.state.document_for_path(&file), Some(document));

    poll_until(&mut [&mut session], |sessions| {
        sessions[0].coop_shared_map_path(document).is_some()
    });
    assert_eq!(session.coop_follow_target().map(|(id, _)| id), Some(document));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_map_someone_shares_does_not_stop_following() {
    let (dir, mut session, a, _) = following("follow-share");
    look_at(&mut session, &dir, "_maps/a.dmm", 1);
    session.toggle_coop_follow(OTHER);
    assert!(session.coop_follow_target().is_some());

    deliver(
        &mut session,
        &dir,
        Event::MapIncoming {
            path: String::from("_maps/c.dmm"),
            by: OTHER,
            len: 1234,
        },
    );
    assert_ne!(
        session.state.active(),
        Some(a),
        "the incoming map opens as the active document"
    );

    assert_eq!(session.coop_follow_target().map(|(id, _)| id), Some(a));
    assert_eq!(session.state.active(), Some(a));
    assert_eq!(session.coop_following(), Some(OTHER));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_broken_view_is_dropped() {
    let (dir, mut session, ..) = following("follow-broken");
    for (center, zoom) in [
        ([0.0, 0.0], 0.0),
        ([0.0, 0.0], -1.0),
        ([f32::NAN, 0.0], 1.0),
        ([0.0, 0.0], f32::INFINITY),
    ] {
        deliver(
            &mut session,
            &dir,
            Event::View {
                from: OTHER,
                view: Some(View {
                    map: String::from("_maps/a.dmm"),
                    z: 1,
                    center,
                    zoom,
                }),
            },
        );
        assert!(
            session.coop().unwrap().peers[&OTHER].view.is_none(),
            "{center:?} at {zoom}"
        );
    }

    let _ = fs::remove_dir_all(dir);
}
