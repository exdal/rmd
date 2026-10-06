use super::*;

#[test]
fn an_incoming_share_opens_a_locked_pending_document() {
    let (dir, mut session) = hosting("pending-open");
    let file = dir.join("_maps/a.dmm");
    incoming(&mut session, &dir);

    let id = session.state.document_for_path(&file).expect("no pending document");
    assert_eq!(session.state.active(), Some(id));
    assert!(session.state.document(id).unwrap().is_read_only());
    assert!(session.is_coop_shared_file(&file));
    assert!(matches!(
        session.coop_receiving(id),
        Some(SharedState::Incoming {
            by: OTHER,
            len: 1234,
            ..
        })
    ));

    deliver(
        &mut session,
        &dir,
        Event::MapCancelled {
            path: String::from("_maps/a.dmm"),
            by: OTHER,
        },
    );
    assert!(session.state.document_for_path(&file).is_none());
    assert!(!session.is_coop_shared_file(&file));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_failed_reshare_resyncs_the_map_from_the_server() {
    let (dir, mut session) = hosting("reshare-failed");
    let file = dir.join("_maps/a.dmm");
    let path = "_maps/a.dmm";
    let id = open_local(&mut session, file.clone());
    session.share_coop_map();
    poll_until(&mut [&mut session], |sessions| {
        sessions[0].coop().unwrap().shared_maps.contains_key(path) && settled(sessions)
    });

    let document = session.state.document_mut(id).unwrap();
    let fill = document.map.tile_at(Coord::new(1, 1, 1)).cloned().unwrap();
    assert!(document.resize(3, 1, &fill));
    session.share_coop_document(id);

    // the upload never reaches the server
    let prepared = session.coop().unwrap().prepared.1.recv_timeout(Duration::from_secs(10));
    assert!(matches!(prepared, Ok(Prepared::Upload { .. })));
    deliver(
        &mut session,
        &dir,
        Event::TransferFailed {
            path: String::from(path),
            direction: Direction::Sending,
            reason: String::from("lost"),
        },
    );

    poll_until(&mut [&mut session], |sessions| {
        let document = sessions[0].state.document_for_path(&file);
        settled(sessions)
            && document
                .and_then(|id| sessions[0].state.document(id))
                .is_some_and(|document| document.map.size().x == 2)
    });

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_newer_snapshot_replaces_one_still_loading() {
    let (dir, mut session) = hosting("snapshot-mid-load");
    let path = "_maps/a.dmm";
    incoming(&mut session, &dir);

    let snapshot = |generation, rows: &str| Event::MapSnapshot {
        path: String::from(path),
        generation: GenerationId(generation),
        bytes: format!("\"a\" = (/turf,/area)\n\n(1,1,1) = {{\"\n{rows}\n\"}}\n").into_bytes(),
    };

    // both arrive before the first one is parsed
    let coop = session.coop.as_mut().unwrap();
    coop.apply(snapshot(1, "aa"), Some(&dir));
    coop.apply(snapshot(2, "aaa"), Some(&dir));
    coop.apply(
        Event::MapShared {
            path: String::from(path),
            by: OTHER,
            generation: GenerationId(2),
        },
        Some(&dir),
    );

    let file = dir.join(path);
    poll_until(&mut [&mut session], |sessions| {
        let shared_map = &sessions[0].coop().unwrap().shared_maps[path];
        let document = sessions[0].state.document_for_path(&file);
        shared_map.is_ready()
            && shared_map.generation == Some(GenerationId(2))
            && document
                .and_then(|id| sessions[0].state.document(id))
                .is_some_and(|document| document.map.size().x == 3)
    });

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn sharing_a_file_needs_it_open_and_a_connection() {
    let (dir, mut session) = codebase_with_map("share-file", "aa");
    let file = dir.join("_maps/a.dmm");
    open_local(&mut session, file.clone());
    assert!(!session.share_coop_file(&file), "shared without a session");

    session
        .host_coop(0, net::hash_password("hunter2"), String::from("host"))
        .unwrap();
    poll_until(&mut [&mut session], |sessions| connected(sessions[0]));
    assert!(!session.share_coop_file(&dir.join("_maps/missing.dmm")));
    assert!(session.share_coop_file(&file));
    poll_until(&mut [&mut session], |sessions| {
        sessions[0].coop().unwrap().shared_maps.contains_key("_maps/a.dmm") && settled(sessions)
    });

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn closing_a_pending_document_drops_its_snapshot_until_reopened() {
    let (dir, mut session) = hosting("pending-close");
    let file = dir.join("_maps/a.dmm");
    let path = "_maps/a.dmm";
    incoming(&mut session, &dir);
    session.close_map(session.state.document_for_path(&file).unwrap());
    session.poll_coop();
    assert!(
        session.state.document_for_path(&file).is_none(),
        "the closed document came back"
    );

    deliver(
        &mut session,
        &dir,
        Event::MapShared {
            path: String::from(path),
            by: OTHER,
            generation: GenerationId(1),
        },
    );
    assert!(session.state.document_for_path(&file).is_none());

    let (map, errors) = parser::parse(&fs::read_to_string(&file).unwrap());
    session.open_shared_map(ReceivedMap {
        path: String::from(path),
        generation: GenerationId(1),
        file: file.clone(),
        map,
        errors,
        is_modified: false,
    });
    session.poll_coop();
    assert!(session.state.document_for_path(&file).is_none());
    assert!(matches!(
        session.coop().unwrap().shared_maps[path].state,
        SharedState::Closed
    ));

    assert!(session.open_coop_file(&file));
    let id = session
        .state
        .document_for_path(&file)
        .expect("reopening shows a pending document");
    assert!(matches!(
        session.coop().unwrap().shared_maps[path].state,
        SharedState::Receiving { .. }
    ));
    assert!(session.coop_receiving(id).is_some());

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn an_out_of_date_copy_has_no_cursors_or_comments() {
    let (host_dir, host, guest_dir, guest, local) = guest_out_of_date("cursor-out-of-date");
    let host_map = host.state.document_for_path(&host_dir.join("_maps/a.dmm")).unwrap();
    assert_eq!(host.coop_shared_map_path(host_map).as_deref(), Some("_maps/a.dmm"));
    assert_eq!(guest.coop_shared_map_path(local), None);

    let _ = fs::remove_dir_all(host_dir);
    let _ = fs::remove_dir_all(guest_dir);
}

#[test]
fn a_shared_map_waits_for_unsaved_work_then_replaces_the_local_copy() {
    let (host_dir, mut host, guest_dir, mut guest, local) = guest_out_of_date("share");

    guest.close_map(local);
    let file = guest_dir.join("_maps/a.dmm");
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        sessions[1].state.document_for_path(&file).is_some()
    });

    let received = guest
        .state
        .document(guest.state.document_for_path(&file).unwrap())
        .unwrap();
    assert_eq!(received.map.size().x, 2);
    assert!(received.is_dirty());
    assert!(guest.coop().unwrap().shared_maps["_maps/a.dmm"].is_ready());

    let _ = fs::remove_dir_all(&host_dir);
    let _ = fs::remove_dir_all(&guest_dir);
}

#[test]
fn an_out_of_date_copy_is_read_only_until_discarded() {
    let (host_dir, mut host, guest_dir, mut guest, local) = guest_out_of_date("discard");
    let host_id = host.coop().unwrap().you;
    assert_eq!(guest.coop_out_of_date(local), host_id);
    assert!(guest.state.document(local).unwrap().is_read_only());

    guest.discard_for_coop_map(local);
    assert_eq!(guest.coop_out_of_date(local), None, "a handed over copy no longer asks");
    let file = guest_dir.join("_maps/a.dmm");
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        sessions[1]
            .state
            .document_for_path(&file)
            .and_then(|id| sessions[1].state.document(id))
            .is_some_and(|document| document.map.size().x == 2)
    });

    let id = guest.state.document_for_path(&file).unwrap();
    assert_eq!(guest.coop_out_of_date(id), None);
    assert!(!guest.state.document(id).unwrap().is_read_only());
    assert!(guest.coop().unwrap().shared_maps["_maps/a.dmm"].is_ready());
    assert_eq!(width_on_disk(&file), 1);

    let _ = fs::remove_dir_all(&host_dir);
    let _ = fs::remove_dir_all(&guest_dir);
}

#[test]
fn leaving_keeps_an_out_of_date_copy() {
    let (host_dir, mut host, guest_dir, mut guest, local) = guest_out_of_date("leave");
    guest.leave_coop();
    host.poll_coop();

    let document = guest.state.document(local).unwrap();
    assert!(document.is_dirty());
    assert!(!document.is_read_only());
    assert_eq!(document.map.size().x, 1);
    assert_eq!(width_on_disk(&guest_dir.join("_maps/a.dmm")), 1);

    let _ = fs::remove_dir_all(host_dir);
    let _ = fs::remove_dir_all(guest_dir);
}

#[test]
fn saving_an_out_of_date_copy_loads_the_shared_one() {
    let (host_dir, mut host, guest_dir, mut guest, local) = guest_out_of_date("save");
    let file = guest_dir.join("_maps/a.dmm");
    fs::write(&file, "").unwrap();

    guest.save_document(local).unwrap();
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        sessions[1]
            .state
            .document_for_path(&file)
            .and_then(|id| sessions[1].state.document(id))
            .is_some_and(|document| document.map.size().x == 2)
    });

    let id = guest.state.document_for_path(&file).unwrap();
    assert!(!guest.state.document(id).unwrap().is_read_only());
    assert!(guest.coop().unwrap().shared_maps["_maps/a.dmm"].is_ready());
    assert_eq!(width_on_disk(&file), 1);

    let _ = fs::remove_dir_all(&host_dir);
    let _ = fs::remove_dir_all(&guest_dir);
}

#[test]
fn a_closed_shared_map_resyncs_when_reopened() {
    let (host_dir, mut host) = codebase_with_map("resync-host", "aa");
    let (guest_dir, mut guest) = codebase_with_map("resync-guest", "aa");
    let host_file = host_dir.join("_maps/a.dmm");
    let guest_file = guest_dir.join("_maps/a.dmm");
    let (left, right) = (Coord::new(1, 1, 1), Coord::new(2, 1, 1));
    open_local(&mut host, host_file.clone());

    host.host_coop(0, net::hash_password("hunter2"), String::from("host"))
        .unwrap();
    poll_until(&mut [&mut host], |sessions| connected(sessions[0]));
    host.share_coop_map();
    let port = host.coop().and_then(Coop::host).unwrap().port();
    guest
        .join_coop(
            format!("127.0.0.1:{port}"),
            net::hash_password("hunter2"),
            String::from("guest"),
        )
        .unwrap();
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        sessions[1].state.document_for_path(&guest_file).is_some() && settled(sessions)
    });

    paint(&mut guest, &guest_file, left, "/obj/guest");
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        settled(sessions) && top(sessions[0], &host_file, left).as_deref() == Some("/obj/guest")
    });

    guest.close_map(guest.state.document_for_path(&guest_file).unwrap());
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        matches!(
            sessions[1].coop().unwrap().shared_maps["_maps/a.dmm"].state,
            SharedState::Closed
        )
    });

    paint(&mut host, &host_file, right, "/obj/missed");
    poll_until(&mut [&mut host, &mut guest], settled);
    assert!(guest.state.document_for_path(&guest_file).is_none());

    guest.open_coop_map("_maps/a.dmm");
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        settled(sessions) && top(sessions[1], &guest_file, right).as_deref() == Some("/obj/missed")
    });
    assert_eq!(top(&guest, &guest_file, left).as_deref(), Some("/obj/guest"));

    paint(&mut host, &host_file, left, "/obj/after");
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        top(sessions[1], &guest_file, left).as_deref() == Some("/obj/after")
    });

    guest.close_map(guest.state.document_for_path(&guest_file).unwrap());
    paint(&mut host, &host_file, right, "/obj/missed_again");
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        settled(sessions)
            && matches!(
                sessions[1].coop().unwrap().shared_maps["_maps/a.dmm"].state,
                SharedState::Closed
            )
    });

    let reopened = open_local(&mut guest, guest_file.clone());
    guest.poll_coop();
    assert!(guest.state.document(reopened).unwrap().is_read_only());
    assert!(!guest.can_edit_at(left));

    poll_until(&mut [&mut host, &mut guest], |sessions| {
        settled(sessions) && top(sessions[1], &guest_file, right).as_deref() == Some("/obj/missed_again")
    });
    assert_eq!(top(&guest, &guest_file, left).as_deref(), Some("/obj/after"));
    assert!(!guest.state.document(reopened).unwrap().is_read_only());

    for dir in [host_dir, guest_dir] {
        let _ = fs::remove_dir_all(dir);
    }
}

#[test]
fn anyone_can_stop_sharing_a_map_and_every_copy_stays_open() {
    let (host_dir, mut host) = codebase_with_map("unshare-host", "aa");
    let (guest_dir, mut guest) = codebase_with_map("unshare-guest", "aa");
    let host_map = open_local(&mut host, host_dir.join("_maps/a.dmm"));
    host.host_coop(0, net::hash_password("hunter2"), String::from("host"))
        .unwrap();
    poll_until(&mut [&mut host], |sessions| connected(sessions[0]));
    host.share_coop_map();

    let port = host.coop().and_then(Coop::host).unwrap().port();
    guest
        .join_coop(
            format!("127.0.0.1:{port}"),
            net::hash_password("hunter2"),
            String::from("guest"),
        )
        .unwrap();
    let guest_file = guest_dir.join("_maps/a.dmm");
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        sessions[1].state.document_for_path(&guest_file).is_some() && settled(sessions)
    });

    host.add_coop_comment(host_map, [0.0, 0.0], String::from("gone"));
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        sessions
            .iter()
            .all(|session| session.coop().unwrap().comments.len() == 1)
    });

    let guest_map = guest.state.document_for_path(&guest_file).unwrap();
    assert!(guest.can_stop_sharing_coop_map());
    guest.stop_sharing_coop_map();
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        sessions
            .iter()
            .all(|session| session.coop().unwrap().shared_maps.is_empty())
    });

    for (session, file, id) in [
        (&host, host_dir.join("_maps/a.dmm"), host_map),
        (&guest, guest_file, guest_map),
    ] {
        assert!(session.coop().unwrap().comments.is_empty());
        assert!(!session.is_coop_shared_file(&file));
        assert!(!session.can_stop_sharing_coop_map());
        assert!(!session.state.document(id).unwrap().is_read_only());
        assert!(matches!(
            session.coop().unwrap().activity.back(),
            Some((_, Activity::Unshared { path, nick: Some(nick), .. })) if path == "_maps/a.dmm" && nick == "guest"
        ));
    }

    for dir in [host_dir, guest_dir] {
        let _ = fs::remove_dir_all(dir);
    }
}

#[test]
fn stopping_a_share_closes_the_document_waiting_for_it() {
    let (dir, mut session) = hosting("unshare-pending");
    let file = dir.join("_maps/a.dmm");
    incoming(&mut session, &dir);
    assert!(session.state.document_for_path(&file).is_some());

    session.forget_coop_map("_maps/a.dmm", OTHER);

    assert!(session.state.document_for_path(&file).is_none());
    assert!(!session.is_coop_shared_file(&file));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn stopping_a_share_keeps_an_out_of_date_copy() {
    let (host_dir, mut host, guest_dir, mut guest, local) = guest_out_of_date("unshare-waiting");
    host.stop_sharing_coop_map();
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        sessions[1].coop().unwrap().shared_maps.is_empty()
    });

    let document = guest.state.document(local).unwrap();
    assert!(document.is_dirty());
    assert!(!document.is_read_only());
    assert_eq!(document.map.size().x, 1);
    assert_eq!(guest.coop_out_of_date(local), None);

    let _ = fs::remove_dir_all(host_dir);
    let _ = fs::remove_dir_all(guest_dir);
}
