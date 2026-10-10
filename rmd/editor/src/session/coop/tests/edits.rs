use std::error::Error;

use super::*;

#[test]
fn edits_sync_both_ways_converge_and_replay_for_late_joiners() {
    let (host_dir, mut host) = codebase_with_map("sync-host", "aa");
    let (guest_dir, mut guest) = codebase_with_map("sync-guest", "aa");
    let (late_dir, mut late) = codebase_with_map("sync-late", "aa");
    let host_file = host_dir.join("_maps/a.dmm");
    let guest_file = guest_dir.join("_maps/a.dmm");
    let late_file = late_dir.join("_maps/a.dmm");
    let (left, right) = (Coord::new(1, 1, 1), Coord::new(2, 1, 1));
    open_local(&mut host, host_file.clone());

    host.host_coop(0, net::hash_password("hunter2"), String::from("host"))
        .unwrap();
    poll_until(&mut [&mut host], |sessions| connected(sessions[0]));
    host.share_coop_map();
    let port = host.coop().and_then(Coop::host).unwrap().port();
    let join = |session: &mut Session, nick: &str| {
        session
            .join_coop(
                format!("127.0.0.1:{port}"),
                net::hash_password("hunter2"),
                String::from(nick),
            )
            .unwrap();
    };

    join(&mut guest, "guest");
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        sessions[1].state.document_for_path(&guest_file).is_some() && settled(sessions)
    });

    paint(&mut host, &host_file, left, "/obj/host");
    paint(&mut guest, &guest_file, right, "/obj/guest");
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        top(sessions[1], &guest_file, left).as_deref() == Some("/obj/host")
            && top(sessions[0], &host_file, right).as_deref() == Some("/obj/guest")
    });

    paint(&mut host, &host_file, left, "/obj/first");
    paint(&mut guest, &guest_file, left, "/obj/second");
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        settled(sessions) && top(sessions[0], &host_file, left) == top(sessions[1], &guest_file, left)
    });
    let winner = top(&host, &host_file, left);
    assert_eq!(winner, top(&guest, &guest_file, left));
    assert!(matches!(winner.as_deref(), Some("/obj/first" | "/obj/second")));

    let id = host.state.document_for_path(&host_file).unwrap();
    host.state.set_active(id);
    assert!(host.undo());
    assert_eq!(top(&host, &host_file, left).as_deref(), Some("/obj/host"));
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        settled(sessions) && top(sessions[1], &guest_file, left).as_deref() == Some("/obj/host")
    });

    join(&mut late, "late");
    poll_until(&mut [&mut host, &mut guest, &mut late], |sessions| {
        sessions[2].state.document_for_path(&late_file).is_some() && settled(sessions)
    });
    for coord in [left, right] {
        assert_eq!(
            top(&late, &late_file, coord),
            top(&host, &host_file, coord),
            "{coord:?}"
        );
    }

    let id = host.state.document_for_path(&host_file).unwrap();
    let document = host.state.document_mut(id).unwrap();
    let fill = document.map.tile_at(right).cloned().unwrap();
    assert!(document.resize(3, 1, &fill));
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        let document = sessions[1]
            .state
            .document(sessions[1].state.document_for_path(&guest_file).unwrap());
        document.is_some_and(|document| document.map.size().x == 3) && settled(sessions)
    });

    let corner = Coord::new(3, 1, 1);
    paint(&mut host, &host_file, corner, "/obj/after_resize");
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        top(sessions[1], &guest_file, corner).as_deref() == Some("/obj/after_resize")
    });

    let id = host.state.document_for_path(&host_file).unwrap();
    let document = host.state.document_mut(id).unwrap();
    let mut edit = Edit::new("erase everything");
    edit.change(document, corner, Vec::new());
    assert!(document.apply(edit));
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        let guest = &sessions[1];
        let document = guest
            .state
            .document(guest.state.document_for_path(&guest_file).unwrap());
        document.is_some_and(|document| document.map.tile_at(corner).is_some_and(|tile| tile.is_empty()))
    });

    for dir in [host_dir, guest_dir, late_dir] {
        let _ = fs::remove_dir_all(dir);
    }
}

#[test]
fn a_reshare_keeps_edits_its_snapshot_missed() {
    let (host_dir, mut host) = codebase_with_map("reshare-host", "aa");
    let (guest_dir, mut guest) = codebase_with_map("reshare-guest", "aa");
    let host_file = host_dir.join("_maps/a.dmm");
    let guest_file = guest_dir.join("_maps/a.dmm");
    let left = Coord::new(1, 1, 1);
    let id = open_local(&mut host, host_file.clone());

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

    // the server has the edit, but the host shares again before it applies it
    paint(&mut guest, &guest_file, left, "/obj/guest");
    poll_until(&mut [&mut guest], settled);
    host.share_coop_document(id);

    poll_until(&mut [&mut host, &mut guest], |sessions| {
        settled(sessions)
            && sessions[1].coop().unwrap().shared_maps["_maps/a.dmm"].generation == Some(GenerationId(2))
            && top(sessions[0], &host_file, left).as_deref() == Some("/obj/guest")
            && top(sessions[1], &guest_file, left).as_deref() == Some("/obj/guest")
    });

    for dir in [host_dir, guest_dir] {
        let _ = fs::remove_dir_all(dir);
    }
}

#[test]
fn edits_converge_through_a_lossy_link() {
    let server = Server::spawn(ServerConfig {
        codebase: None,
        bind: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        password: net::hash_password("hunter2"),
    })
    .unwrap();
    let proxy = LossyProxy::spawn(
        SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        server.local_addr(),
        Impairment {
            latency: Duration::from_millis(20),
            jitter: Duration::from_millis(20),
            loss: 0.05,
            seed: 11,
        },
    )
    .unwrap();

    let (a_dir, mut a) = codebase_with_map("lossy-a", "aaaa");
    let (b_dir, mut b) = codebase_with_map("lossy-b", "aaaa");
    let a_file = a_dir.join("_maps/a.dmm");
    let b_file = b_dir.join("_maps/a.dmm");
    open_local(&mut a, a_file.clone());
    for (session, nick) in [(&mut a, "a"), (&mut b, "b")] {
        session
            .join_coop(
                proxy.local_addr().to_string(),
                net::hash_password("hunter2"),
                String::from(nick),
            )
            .unwrap();
    }

    poll_until(&mut [&mut a, &mut b], |sessions| {
        sessions.iter().all(|session| connected(session))
    });
    a.share_coop_map();
    poll_until(&mut [&mut a, &mut b], |sessions| {
        sessions[1].state.document_for_path(&b_file).is_some() && settled(sessions)
    });

    let tiles = (1..=4).map(|x| Coord::new(x, 1, 1)).collect::<Vec<_>>();
    let converged = |sessions: &[&mut Session]| {
        settled(sessions)
            && tiles
                .iter()
                .all(|&coord| top(sessions[0], &a_file, coord) == top(sessions[1], &b_file, coord))
    };
    for round in 0..10 {
        for &coord in &tiles {
            paint(&mut a, &a_file, coord, &format!("/obj/a{round}"));
            paint(&mut b, &b_file, coord, &format!("/obj/b{round}"));
        }

        a.poll_coop();
        b.poll_coop();
    }

    poll_until(&mut [&mut a, &mut b], converged);

    b.close_map(b.state.document_for_path(&b_file).unwrap());
    poll_until(&mut [&mut a, &mut b], |sessions| {
        matches!(
            sessions[1].coop().unwrap().shared_maps["_maps/a.dmm"].state,
            SharedState::Closed
        )
    });
    for &coord in &tiles {
        paint(&mut a, &a_file, coord, "/obj/while_closed");
    }

    b.open_coop_map("_maps/a.dmm");
    poll_until(&mut [&mut a, &mut b], |sessions| {
        sessions[1].state.document_for_path(&b_file).is_some() && converged(sessions)
    });
    assert_eq!(top(&b, &b_file, tiles[0]).as_deref(), Some("/obj/while_closed"));

    for dir in [a_dir, b_dir] {
        let _ = fs::remove_dir_all(dir);
    }
}

#[test]
fn edits_queued_for_one_poll_apply_as_one_remote_edit() {
    let (dir, mut session) = hosting("batched-edits");
    let path = "_maps/a.dmm";
    let file = dir.join(path);
    incoming(&mut session, &dir);
    let coop = session.coop.as_mut().unwrap();
    coop.apply(
        Event::MapSnapshot {
            path: String::from(path),
            generation: GenerationId(1),
            bytes: b"\"a\" = (/turf,/area)\n\n(1,1,1) = {\"\naaa\n\"}\n".to_vec(),
        },
        Some(&dir),
    );
    coop.apply(
        Event::MapShared {
            path: String::from(path),
            by: OTHER,
            generation: GenerationId(1),
        },
        Some(&dir),
    );
    poll_until(&mut [&mut session], |sessions| {
        sessions[0].coop().unwrap().shared_maps[path].is_ready() && sessions[0].state.document_for_path(&file).is_some()
    });

    let (first, second, third) = (Coord::new(1, 1, 1), Coord::new(2, 1, 1), Coord::new(3, 1, 1));
    let you = session.coop().unwrap().you.unwrap();
    let coop = session.coop.as_mut().unwrap();
    coop.shared_maps.get_mut(path).unwrap().in_flight.insert(third, 1);
    let edit = |tiles: &[(Coord, &str)]| {
        let mut patch = dmm::Map::new(dmm::Size {
            x: tiles.len() as u32,
            y: 1,
            z: 1,
        });
        for (index, (_, top)) in tiles.iter().enumerate() {
            patch.grid[0][0][index] = patch.intern_tile(vec![
                Prefab::new(TreePath::parse(top)),
                Prefab::new(TreePath::parse("/turf")),
                Prefab::new(TreePath::parse("/area")),
            ]);
        }

        net::MapEdit {
            path: String::from(path),
            generation: GenerationId(1),
            coords: tiles.iter().map(|(coord, _)| [coord.x, coord.y, coord.z]).collect(),
            patch: dmm::writer::write(&patch),
            level: None,
            seen: net::SeqId(0),
        }
    };
    for (seq, by, tiles) in [
        (0, OTHER, vec![(first, "/obj/first"), (third, "/obj/other")]),
        (1, OTHER, vec![(first, "/obj/second"), (second, "/obj/two")]),
        (2, you, vec![(third, "/obj/mine")]),
    ] {
        coop.apply(
            Event::Edit {
                by,
                seq: net::SeqId(seq),
                edit: edit(&tiles),
            },
            Some(&dir),
        );
    }
    let generation = |session: &Session| {
        let id = session.state.document_for_path(&file).unwrap();
        session.state.document(id).unwrap().generation()
    };
    let before = generation(&session);

    session.poll_coop();

    assert_eq!(generation(&session), before + 1);
    assert_eq!(top(&session, &file, first).as_deref(), Some("/obj/second"));
    assert_eq!(top(&session, &file, second).as_deref(), Some("/obj/two"));
    assert_eq!(
        top(&session, &file, third).as_deref(),
        Some("/turf"),
        "our tile in flight keeps our version"
    );
    assert!(session.coop().unwrap().shared_maps[path].in_flight.is_empty());

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_new_level_reaches_every_peer_without_a_reshare() {
    let (host_dir, mut host) = codebase_with_map("level-host", "aa");
    let (guest_dir, mut guest) = codebase_with_map("level-guest", "aa");
    let host_file = host_dir.join("_maps/a.dmm");
    let guest_file = guest_dir.join("_maps/a.dmm");
    let host_id = open_local(&mut host, host_file.clone());

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
    let guest_id = guest.state.document_for_path(&guest_file).unwrap();
    let levels = |session: &Session, id| session.state.document(id).unwrap().map.size().z;
    let tile = |paths: &[&str]| {
        paths
            .iter()
            .map(|path| Prefab::new(TreePath::parse(path)))
            .collect::<Vec<_>>()
    };

    let fill = tile(&["/obj/guest", "/turf", "/area"]);
    assert_eq!(guest.create_level(guest_id, &fill), Ok(2));
    assert_eq!(levels(&guest, guest_id), 1, "the level waits for the server");
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        settled(sessions) && levels(sessions[0], host_id) == 2 && levels(sessions[1], guest_id) == 2
    });

    let corner = Coord::new(2, 1, 2);
    assert_eq!(top(&host, &host_file, corner).as_deref(), Some("/obj/guest"));
    assert_eq!(top(&guest, &guest_file, corner).as_deref(), Some("/obj/guest"));
    assert_eq!(guest.state.document(guest_id).unwrap().z, 2);
    assert_eq!(host.state.document(host_id).unwrap().z, 1);
    for session in [&host, &guest] {
        assert_eq!(
            session.coop().unwrap().shared_maps["_maps/a.dmm"].generation,
            Some(GenerationId(1))
        );
    }

    // both ask for the third level at once, the server's order picks one for everybody
    host.set_level(2);
    assert_eq!(
        host.create_level(host_id, &tile(&["/obj/host", "/turf", "/area"])),
        Ok(3)
    );
    assert_eq!(guest.create_level(guest_id, &fill), Ok(3));
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        settled(sessions) && levels(sessions[0], host_id) == 3 && levels(sessions[1], guest_id) == 3
    });
    let corner = Coord::new(1, 1, 3);
    assert_eq!(top(&host, &host_file, corner), top(&guest, &guest_file, corner));
    assert_eq!(levels(&host, host_id), 3);

    for dir in [host_dir, guest_dir] {
        let _ = fs::remove_dir_all(dir);
    }
}

fn shared_pair(name: &str) -> (PathBuf, Session, PathBuf, Session, DocumentId, DocumentId) {
    let (host_dir, mut host) = codebase_with_map(&format!("{name}-host"), "aa");
    let (guest_dir, mut guest) = codebase_with_map(&format!("{name}-guest"), "aa");
    let host_id = open_local(&mut host, host_dir.join("_maps/a.dmm"));

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
    let guest_id = guest.state.document_for_path(&guest_file).unwrap();

    (host_dir, host, guest_dir, guest, host_id, guest_id)
}

fn plain_fill() -> Vec<Prefab> {
    vec![
        Prefab::new(TreePath::parse("/turf")),
        Prefab::new(TreePath::parse("/area")),
    ]
}

#[test]
fn a_level_created_before_the_first_share_is_numbered_reaches_peers() {
    let (host_dir, mut host) = codebase_with_map("level-unnumbered-host", "aa");
    let (guest_dir, mut guest) = codebase_with_map("level-unnumbered-guest", "aa");
    let host_id = open_local(&mut host, host_dir.join("_maps/a.dmm"));
    host.host_coop(0, net::hash_password("hunter2"), String::from("host"))
        .unwrap();
    poll_until(&mut [&mut host], |sessions| connected(sessions[0]));

    host.share_coop_map();
    assert_eq!(host.coop().unwrap().shared_maps["_maps/a.dmm"].generation, None);
    assert_eq!(host.create_level(host_id, &plain_fill()), Ok(2));
    poll_until(&mut [&mut host], settled);

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
    let guest_id = guest.state.document_for_path(&guest_file).unwrap();
    assert_eq!(guest.state.document(guest_id).unwrap().map.size().z, 2);

    for dir in [host_dir, guest_dir] {
        let _ = fs::remove_dir_all(dir);
    }
}

#[test]
fn a_paused_map_cannot_ask_for_a_level() {
    let (host_dir, mut host, guest_dir, mut guest, host_id, guest_id) = shared_pair("level-paused");

    guest.begin_coop_reload();
    assert!(guest.state.document(guest_id).unwrap().is_read_only());
    assert!(guest.create_level(guest_id, &plain_fill()).is_err());
    guest.finish_coop_reload();
    poll_until(&mut [&mut host, &mut guest], settled);
    assert_eq!(host.state.document(host_id).unwrap().map.size().z, 1);
    assert_eq!(guest.state.document(guest_id).unwrap().map.size().z, 1);

    for dir in [host_dir, guest_dir] {
        let _ = fs::remove_dir_all(dir);
    }
}

#[test]
fn a_second_level_waits_for_the_first_to_come_back() {
    let (host_dir, mut host, guest_dir, mut guest, host_id, guest_id) = shared_pair("level-twice");

    assert_eq!(guest.create_level(guest_id, &plain_fill()), Ok(2));
    assert!(
        guest.create_level(guest_id, &plain_fill()).is_err(),
        "a second request for the same level would be dropped"
    );
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        settled(sessions) && sessions[0].state.document(host_id).unwrap().map.size().z == 2
    });
    assert_eq!(guest.state.document(guest_id).unwrap().map.size().z, 2);

    for dir in [host_dir, guest_dir] {
        let _ = fs::remove_dir_all(dir);
    }
}

#[test]
fn a_level_created_right_after_a_resize_reaches_peers() {
    let (host_dir, mut host, guest_dir, mut guest, host_id, guest_id) = shared_pair("level-after-resize");

    host.resize_map(3, 1, &plain_fill()).unwrap();
    assert_eq!(host.create_level(host_id, &plain_fill()), Ok(2));
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        settled(sessions) && sessions[1].state.document(guest_id).unwrap().map.size().x == 3
    });

    assert_eq!(host.state.document(host_id).unwrap().map.size().z, 2);
    assert_eq!(guest.state.document(guest_id).unwrap().map.size().z, 2);

    for dir in [host_dir, guest_dir] {
        let _ = fs::remove_dir_all(dir);
    }
}

#[test]
fn a_level_created_while_a_reshare_uploads_reaches_peers() {
    let (host_dir, mut host, guest_dir, mut guest, host_id, guest_id) = shared_pair("level-during-reshare");

    host.resize_map(3, 1, &plain_fill()).unwrap();
    host.poll_coop();
    assert!(!host.coop().unwrap().shared_maps["_maps/a.dmm"].is_ready());
    assert_eq!(host.create_level(host_id, &plain_fill()), Ok(2));
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        settled(sessions) && sessions[1].state.document(guest_id).unwrap().map.size().x == 3
    });

    assert_eq!(host.state.document(host_id).unwrap().map.size().z, 2);
    assert_eq!(guest.state.document(guest_id).unwrap().map.size().z, 2);

    for dir in [host_dir, guest_dir] {
        let _ = fs::remove_dir_all(dir);
    }
}

#[test]
fn shared_levels_keep_coordinates_and_allow_tile_history() -> Result<(), Box<dyn Error>> {
    let (host_dir, mut host, guest_dir, mut guest, host_id, guest_id) = shared_pair("level-history");
    guest.create_level(guest_id, &plain_fill())?;
    assert!(!guest.can_delete_level(guest_id));
    assert!(guest.delete_level(guest_id, 1).is_err());
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        settled(sessions) && sessions.iter().all(|session| session.level_count() == 2)
    });
    assert_eq!(
        guest
            .state
            .document(guest_id)
            .ok_or("guest document is missing")?
            .history
            .undo_depth(),
        1
    );
    assert_eq!(
        host.state
            .document(host_id)
            .ok_or("host document is missing")?
            .history
            .undo_depth(),
        0
    );
    assert_eq!(guest.undo_label(), Some("create Z level 2"));
    assert!(host.can_delete_level(host_id));
    let corner = Coord::new(1, 1, 2);
    let host_file = host_dir.join("_maps/a.dmm");
    let guest_file = guest_dir.join("_maps/a.dmm");
    paint(&mut host, &host_file, corner, "/obj/host");
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        settled(sessions) && top(sessions[1], &guest_file, corner).as_deref() == Some("/obj/host")
    });
    assert!(host.undo());
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        settled(sessions) && top(sessions[1], &guest_file, corner).as_deref() == Some("/turf")
    });
    assert!(host.redo());
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        settled(sessions) && top(sessions[1], &guest_file, corner).as_deref() == Some("/obj/host")
    });
    assert_eq!(host.level_count(), 2);
    assert_eq!(guest.level_count(), 2);
    fs::remove_dir_all(host_dir)?;
    fs::remove_dir_all(guest_dir)?;
    Ok(())
}

#[test]
fn shared_level_redo_stays_blocked_when_the_shared_entry_is_closed() -> Result<(), Box<dyn Error>> {
    let (host_dir, mut host, guest_dir, guest, host_id, _) = shared_pair("level-redo-guard");
    // Prepare history from before sharing, without changing the shared snapshot.
    let document = host.state.document_mut(host_id).unwrap();
    assert_eq!(document.append_level(&plain_fill()), Some(2));
    assert!(document.undo());
    assert_eq!(
        host.redo_label(),
        Some("create Z level 2"),
        "a ready share redoes through the server"
    );
    host.coop
        .as_mut()
        .unwrap()
        .shared_maps
        .get_mut("_maps/a.dmm")
        .unwrap()
        .state = SharedState::Closed;
    assert_eq!(host.redo_label(), None);
    assert!(!host.redo());
    assert_eq!(host.level_count(), 1);
    drop(guest);
    drop(host);
    fs::remove_dir_all(host_dir)?;
    fs::remove_dir_all(guest_dir)?;
    Ok(())
}

fn levels(session: &Session, id: DocumentId) -> u32 { session.state.document(id).unwrap().map.size().z }

fn marked(path: &str) -> Vec<Prefab> {
    vec![
        Prefab::new(TreePath::parse(path)),
        Prefab::new(TreePath::parse("/turf")),
        Prefab::new(TreePath::parse("/area")),
    ]
}

// levels 2 and 3 are filled with /obj/level2 and /obj/level3
fn shared_levels(name: &str) -> (PathBuf, Session, PathBuf, Session, DocumentId, DocumentId) {
    let (host_dir, mut host, guest_dir, mut guest, host_id, guest_id) = shared_pair(name);
    for level in 2..=3 {
        assert_eq!(
            guest.create_level(guest_id, &marked(&format!("/obj/level{level}"))),
            Ok(level)
        );
        poll_until(&mut [&mut host, &mut guest], |sessions| {
            settled(sessions) && levels(sessions[0], host_id) == level && levels(sessions[1], guest_id) == level
        });
    }

    (host_dir, host, guest_dir, guest, host_id, guest_id)
}

fn tops(session: &Session, file: &Path) -> Vec<Option<String>> {
    let id = session.state.document_for_path(file).unwrap();
    let size = session.state.document(id).unwrap().map.size();
    (1..=size.z)
        .flat_map(|z| (1..=size.x).map(move |x| Coord::new(x, 1, z)))
        .map(|coord| top(session, file, coord))
        .collect()
}

#[test]
fn a_level_deleted_while_a_peer_paints_moves_the_paint_with_its_level() {
    let (host_dir, mut host, guest_dir, mut guest, host_id, guest_id) = shared_levels("level-delete-race");
    let host_file = host_dir.join("_maps/a.dmm");
    let guest_file = guest_dir.join("_maps/a.dmm");
    host.set_level_of(host_id, 1);
    host.add_coop_comment(host_id, [0.0, 0.0], String::from("one"));
    host.set_level_of(host_id, 3);
    host.add_coop_comment(host_id, [0.0, 0.0], String::from("three"));
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        sessions
            .iter()
            .all(|session| session.coop().unwrap().comments.len() == 2)
    });

    assert_eq!(host.delete_level(host_id, 1), Ok(()));
    assert_eq!(levels(&host, host_id), 3, "the deletion waits for the server");
    assert!(!host.can_delete_level(host_id), "one level change at a time");
    paint(&mut guest, &guest_file, Coord::new(1, 1, 2), "/obj/guest");
    paint(&mut guest, &guest_file, Coord::new(2, 1, 1), "/obj/doomed");
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        settled(sessions) && levels(sessions[0], host_id) == 2 && levels(sessions[1], guest_id) == 2
    });

    let expected = ["/obj/guest", "/obj/level2", "/obj/level3", "/obj/level3"].map(|path| Some(String::from(path)));
    assert_eq!(tops(&host, &host_file), expected);
    assert_eq!(tops(&guest, &guest_file), expected);
    assert_eq!(host.undo_label(), Some("delete Z level 1"));
    assert_eq!(guest.state.document(guest_id).unwrap().history.undo_depth(), 0);
    for session in [&host, &guest] {
        let comments = session.coop().unwrap().comments.values().collect::<Vec<_>>();
        assert_eq!(comments.len(), 1);
        assert_eq!((comments[0].text.as_str(), comments[0].z), ("three", 2));
    }

    for dir in [host_dir, guest_dir] {
        let _ = fs::remove_dir_all(dir);
    }
}

#[test]
fn racing_deletions_of_one_level_delete_it_once() {
    let (host_dir, mut host, guest_dir, mut guest, host_id, guest_id) = shared_levels("level-delete-twice");
    assert_eq!(host.delete_level(host_id, 2), Ok(()));
    assert_eq!(guest.delete_level(guest_id, 2), Ok(()));
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        settled(sessions) && levels(sessions[0], host_id) == 2 && levels(sessions[1], guest_id) == 2
    });

    let expected =
        [Some("/turf"), Some("/turf"), Some("/obj/level3"), Some("/obj/level3")].map(|path| path.map(String::from));
    assert_eq!(tops(&host, &host_dir.join("_maps/a.dmm")), expected);
    assert_eq!(tops(&guest, &guest_dir.join("_maps/a.dmm")), expected);

    for dir in [host_dir, guest_dir] {
        let _ = fs::remove_dir_all(dir);
    }
}

// a copy of a map someone else shares, levels 1 to 3 topped with nothing, /obj/two and /obj/three
fn three_level_copy(name: &str) -> (PathBuf, Session, PathBuf) {
    let (dir, mut session) = hosting(name);
    let path = "_maps/a.dmm";
    let file = dir.join(path);
    incoming(&mut session, &dir);
    let coop = session.coop.as_mut().unwrap();
    coop.apply(
        Event::MapSnapshot {
            path: String::from(path),
            generation: GenerationId(1),
            bytes: b"\"a\" = (/turf,/area)\n\"b\" = (/obj/two,/turf,/area)\n\"c\" = (/obj/three,/turf,/area)\n\n\
                    (1,1,1) = {\"\naa\n\"}\n(1,1,2) = {\"\nbb\n\"}\n(1,1,3) = {\"\ncc\n\"}\n"
                .to_vec(),
        },
        Some(&dir),
    );
    coop.apply(
        Event::MapShared {
            path: String::from(path),
            by: OTHER,
            generation: GenerationId(1),
        },
        Some(&dir),
    );
    poll_until(&mut [&mut session], |sessions| {
        sessions[0].coop().unwrap().shared_maps[path].is_ready() && sessions[0].state.document_for_path(&file).is_some()
    });

    (dir, session, file)
}

fn level_edit(seen: u32, tiles: &[([u32; 3], &str)], level: Option<LevelOp>) -> net::MapEdit {
    let mut patch = dmm::Map::new(dmm::Size {
        x: tiles.len() as u32,
        y: 1,
        z: 1,
    });
    for (index, (_, top)) in tiles.iter().enumerate() {
        patch.grid[0][0][index] = patch.intern_tile(marked(top));
    }

    net::MapEdit {
        path: String::from("_maps/a.dmm"),
        generation: GenerationId(1),
        coords: tiles.iter().map(|(coord, _)| *coord).collect(),
        patch: if tiles.is_empty() {
            String::new()
        } else {
            dmm::writer::write(&patch)
        },
        level,
        seen: SeqId(seen),
    }
}

fn deliver_edits(session: &mut Session, dir: &Path, edits: Vec<(u32, PeerId, net::MapEdit)>) {
    let coop = session.coop.as_mut().unwrap();
    for (seq, by, edit) in edits {
        coop.apply(
            Event::Edit {
                by,
                seq: SeqId(seq),
                edit,
            },
            Some(dir),
        );
    }

    session.poll_coop();
}

fn in_flight(session: &Session) -> Vec<Coord> {
    session.coop().unwrap().shared_maps["_maps/a.dmm"]
        .in_flight
        .keys()
        .copied()
        .collect()
}

#[test]
fn edits_authored_before_a_deletion_are_rebased_onto_the_new_levels() {
    let (dir, mut session, file) = three_level_copy("level-delete-rebase");
    let id = session.state.document_for_path(&file).unwrap();
    session.set_level_of(id, 3);
    // ours, still unsent when the deletion lands
    paint(&mut session, &file, Coord::new(2, 1, 3), "/obj/mine");
    let you = session.coop().unwrap().you.unwrap();
    session
        .coop
        .as_mut()
        .unwrap()
        .shared_maps
        .get_mut("_maps/a.dmm")
        .unwrap()
        .in_flight
        .insert(Coord::new(1, 1, 3), 1);
    let delete = |z| Some(LevelOp::Delete(DeleteLevel { z }));
    deliver_edits(
        &mut session,
        &dir,
        vec![
            (0, OTHER, level_edit(0, &[], delete(1))),
            (
                1,
                OTHER,
                level_edit(0, &[([1, 1, 2], "/obj/late"), ([2, 1, 1], "/obj/gone")], None),
            ),
            (2, OTHER, level_edit(0, &[], delete(1))),
            (3, OTHER, level_edit(1, &[([2, 1, 1], "/obj/fresh")], None)),
            (4, you, level_edit(0, &[([1, 1, 3], "/obj/echo")], None)),
        ],
    );

    let expected = ["/obj/late", "/obj/fresh", "/obj/three", "/obj/mine"].map(|path| Some(String::from(path)));
    assert_eq!(tops(&session, &file), expected);
    assert_eq!(session.z(), 2, "the view stays on its level");
    assert_eq!(session.state.document(id).unwrap().history.undo_depth(), 0);
    assert_eq!(
        in_flight(&session),
        [Coord::new(2, 1, 2)],
        "our unsent paint went out on its new level"
    );

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn edits_authored_before_an_insertion_move_up_with_their_level() {
    let (dir, mut session, file) = three_level_copy("level-insert-rebase");
    let id = session.state.document_for_path(&file).unwrap();
    session.set_level_of(id, 2);
    paint(&mut session, &file, Coord::new(2, 1, 2), "/obj/mine");
    let tiles = "\"a\" = (/obj/inserted,/turf,/area)\n\"b\" = (/obj/other,/turf,/area)\n\n(1,1,1) = {\"\nab\n\"}\n";
    let insert = Some(LevelOp::Insert(InsertLevel {
        at: InsertAt::Z(2),
        contents: LevelContents::Tiles(String::from(tiles)),
    }));
    deliver_edits(
        &mut session,
        &dir,
        vec![
            (0, OTHER, level_edit(0, &[], insert)),
            (
                1,
                OTHER,
                level_edit(0, &[([1, 1, 2], "/obj/late"), ([1, 1, 1], "/obj/low")], None),
            ),
        ],
    );

    let expected = [
        "/obj/low",
        "/turf",
        "/obj/inserted",
        "/obj/other",
        "/obj/late",
        "/obj/mine",
        "/obj/three",
        "/obj/three",
    ]
    .map(|path| Some(String::from(path)));
    assert_eq!(tops(&session, &file), expected);
    assert_eq!(session.z(), 3, "the view stays on its level");
    assert_eq!(session.state.document(id).unwrap().history.undo_depth(), 0);
    assert_eq!(in_flight(&session), [Coord::new(2, 1, 3)]);

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn a_new_generation_drops_an_unsent_deletion_but_keeps_an_unsent_level() {
    let (host_dir, host, guest_dir, mut guest, _, guest_id) = shared_levels("level-delete-renumber");
    let renumber = |session: &mut Session, generation| {
        session
            .coop
            .as_mut()
            .unwrap()
            .shared_maps
            .get_mut("_maps/a.dmm")
            .unwrap()
            .renumber(generation);
    };

    assert_eq!(guest.delete_level(guest_id, 2), Ok(()));
    renumber(&mut guest, GenerationId(9));
    assert!(
        !guest.is_coop_level_requested(guest_id),
        "its z belongs to the old snapshot"
    );

    guest.set_level_of(guest_id, 3);
    assert_eq!(guest.create_level(guest_id, &plain_fill()), Ok(4));
    renumber(&mut guest, GenerationId(10));
    assert!(guest.is_coop_level_requested(guest_id));

    drop(host);
    for dir in [host_dir, guest_dir] {
        let _ = fs::remove_dir_all(dir);
    }
}

fn level_ids(session: &Session, id: DocumentId, z: u32) -> Vec<dmm::PrefabInstanceId> {
    let document = session.state.document(id).unwrap();
    (1..=2)
        .flat_map(|x| document.instance_ids_at(Coord::new(x, 1, z)).to_vec())
        .collect()
}

#[test]
fn undoing_a_shared_deletion_brings_the_level_back_everywhere() {
    let (host_dir, mut host, guest_dir, mut guest, host_id, guest_id) = shared_levels("level-delete-undo");
    let host_file = host_dir.join("_maps/a.dmm");
    let guest_file = guest_dir.join("_maps/a.dmm");
    let original = tops(&host, &host_file);
    let ids = level_ids(&host, host_id, 2);
    assert_eq!(host.delete_level(host_id, 2), Ok(()));
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        settled(sessions) && levels(sessions[0], host_id) == 2 && levels(sessions[1], guest_id) == 2
    });

    assert_eq!(host.undo_label(), Some("delete Z level 2"));
    assert!(host.undo());
    assert_eq!(levels(&host, host_id), 2, "the level waits for the server");
    assert!(!host.undo(), "one level step at a time");
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        settled(sessions) && levels(sessions[0], host_id) == 3 && levels(sessions[1], guest_id) == 3
    });

    assert_eq!(tops(&host, &host_file), original);
    assert_eq!(tops(&guest, &guest_file), original);
    assert_eq!(level_ids(&host, host_id, 2), ids, "our copy restores its own instances");
    assert_eq!(guest.state.document(guest_id).unwrap().history.undo_depth(), 0);

    assert_eq!(host.redo_label(), Some("delete Z level 2"));
    assert!(host.redo());
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        settled(sessions) && levels(sessions[0], host_id) == 2 && levels(sessions[1], guest_id) == 2
    });
    let expected =
        [Some("/turf"), Some("/turf"), Some("/obj/level3"), Some("/obj/level3")].map(|path| path.map(String::from));
    assert_eq!(tops(&host, &host_file), expected);
    assert_eq!(tops(&guest, &guest_file), expected);
    assert_eq!(host.undo_label(), Some("delete Z level 2"));

    for dir in [host_dir, guest_dir] {
        let _ = fs::remove_dir_all(dir);
    }
}

#[test]
fn undoing_a_shared_level_creation_deletes_it_everywhere() {
    let (host_dir, mut host, guest_dir, mut guest, host_id, guest_id) = shared_pair("level-create-undo");
    assert_eq!(guest.create_level(guest_id, &marked("/obj/made")), Ok(2));
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        settled(sessions) && levels(sessions[0], host_id) == 2 && levels(sessions[1], guest_id) == 2
    });

    assert!(guest.undo());
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        settled(sessions) && levels(sessions[0], host_id) == 1 && levels(sessions[1], guest_id) == 1
    });

    assert_eq!(guest.redo_label(), Some("create Z level 2"));
    assert!(guest.redo());
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        settled(sessions) && levels(sessions[0], host_id) == 2 && levels(sessions[1], guest_id) == 2
    });
    for (session, dir) in [(&host, &host_dir), (&guest, &guest_dir)] {
        assert_eq!(
            top(session, &dir.join("_maps/a.dmm"), Coord::new(2, 1, 2)).as_deref(),
            Some("/obj/made")
        );
    }

    for dir in [host_dir, guest_dir] {
        let _ = fs::remove_dir_all(dir);
    }
}

#[test]
fn a_peer_painting_while_a_deletion_is_undone_keeps_its_level() {
    let (host_dir, mut host, guest_dir, mut guest, host_id, guest_id) = shared_levels("level-undo-race");
    let host_file = host_dir.join("_maps/a.dmm");
    let guest_file = guest_dir.join("_maps/a.dmm");
    assert_eq!(host.delete_level(host_id, 1), Ok(()));
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        settled(sessions) && levels(sessions[0], host_id) == 2 && levels(sessions[1], guest_id) == 2
    });

    assert!(host.undo());
    // on what is level 2 again once the undo lands
    paint(&mut guest, &guest_file, Coord::new(1, 1, 1), "/obj/guest");
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        settled(sessions) && levels(sessions[0], host_id) == 3 && levels(sessions[1], guest_id) == 3
    });

    let expected = [
        "/turf",
        "/turf",
        "/obj/guest",
        "/obj/level2",
        "/obj/level3",
        "/obj/level3",
    ]
    .map(|path| Some(String::from(path)));
    assert_eq!(tops(&host, &host_file), expected);
    assert_eq!(tops(&guest, &guest_file), expected);

    for dir in [host_dir, guest_dir] {
        let _ = fs::remove_dir_all(dir);
    }
}
