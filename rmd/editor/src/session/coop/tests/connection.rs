use super::*;

#[test]
fn a_joined_peer_sees_the_hosts_cursor() {
    let mut host = session();
    host.host_coop(0, String::from("hunter2"), String::from("host"))
        .unwrap();
    poll_until(&mut [&mut host], |sessions| connected(sessions[0]));

    let (addr, password) = host.coop().and_then(Coop::host).unwrap();
    let (addr, password) = (format!("127.0.0.1:{}", addr.port()), password.to_owned());
    let mut guest = session();
    guest.join_coop(addr, password, String::from("guest")).unwrap();
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        connected(sessions[1]) && sessions[0].coop().is_some_and(|coop| coop.peers.len() == 1)
    });

    let peer = guest.coop().unwrap().peers.values().next().unwrap();
    assert_eq!(peer.info.nick, "host");
    assert!(guest.coop().unwrap().same_codebase(&peer.info));

    let cursor = Cursor {
        map: String::from("_maps/a.dmm"),
        z: 1,
        pos: [32.0, 64.0],
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
fn a_wrong_password_ends_the_session_with_the_reason() {
    let mut host = session();
    host.host_coop(0, String::from("hunter2"), String::from("host"))
        .unwrap();
    let port = host.coop().and_then(Coop::host).unwrap().0.port();

    let mut guest = session();
    guest
        .join_coop(format!("127.0.0.1:{port}"), String::from("nope"), String::from("guest"))
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

    host.host_coop(0, String::from("hunter2"), String::from("host"))
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
    host.host_coop(0, String::from("hunter2"), String::from("host"))
        .unwrap();
    poll_until(&mut [&mut host], |sessions| connected(sessions[0]));
    let port = host.coop().and_then(Coop::host).unwrap().0.port();
    guest
        .join_coop(
            format!("127.0.0.1:{port}"),
            String::from("hunter2"),
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
            .join_coop(String::from("127.0.0.1:1"), String::from("pw"), String::from("me"))
            .is_err()
    );
}
