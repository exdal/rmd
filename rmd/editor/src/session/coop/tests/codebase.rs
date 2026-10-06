use super::*;

#[test]
fn a_different_codebase_is_rejected_without_touching_unsaved_maps_then_can_retry() {
    let (host_dir, mut host) = hosting("codebase-reject-host");
    let host_file = host_dir.join("_maps/a.dmm");
    open_local(&mut host, host_file);
    host.share_coop_map();
    poll_until(&mut [&mut host], settled);
    let (guest_dir, mut guest) = codebase_with_map("codebase-reject-guest", "a");
    fs::write(guest_dir.join("game.dme"), "/obj/different").unwrap();
    guest.load_environment(&guest_dir.join("game.dme")).unwrap();
    let file = guest_dir.join("_maps/a.dmm");
    let local = open_local(&mut guest, file.clone());
    guest.state.document_mut(local).unwrap().mark_unsaved();
    let port = host.coop().unwrap().host().unwrap().port();
    guest
        .join_coop(
            format!("127.0.0.1:{port}"),
            net::hash_password("hunter2"),
            "guest".into(),
        )
        .unwrap();
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        matches!(sessions[1].coop().unwrap().status, CoopStatus::CodebaseMismatch { .. })
    });
    assert!(guest.coop().unwrap().shared_maps.is_empty());
    assert!(host.coop().unwrap().peers.is_empty());
    assert!(!guest.can_share_coop_map());
    guest.share_coop_map();
    assert!(guest.coop().unwrap().shared_maps.is_empty());
    let document = guest.state.document(local).unwrap();
    assert_eq!(document.map.size().x, 1);
    assert!(document.is_dirty());
    assert!(!document.is_read_only());
    assert_eq!(width_on_disk(&file), 1);

    fs::write(guest_dir.join("game.dme"), "").unwrap();
    guest.load_environment(&guest_dir.join("game.dme")).unwrap();
    guest.retry_coop().unwrap();
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        connected(sessions[1]) && sessions[1].coop_out_of_date(local).is_some()
    });
    assert!(guest.state.document(local).unwrap().is_dirty());
    guest.leave_coop();
    assert!(guest.coop().is_none());
    assert!(!guest.state.document(local).unwrap().is_read_only());
    let _ = fs::remove_dir_all(host_dir);
    let _ = fs::remove_dir_all(guest_dir);
}

#[test]
fn a_matching_reload_pauses_edits_and_keeps_the_connection_and_unsaved_map() {
    let (host_dir, mut host, guest_dir, mut guest, local) = guest_out_of_date("matching-reload");
    guest.discard_for_coop_map(local);
    let file = guest_dir.join("_maps/a.dmm");
    poll_until(&mut [&mut host, &mut guest], settled);
    let local = guest.state.document_for_path(&file).unwrap();
    let you = guest.coop().unwrap().you;
    guest.state.document_mut(local).unwrap().mark_unsaved();
    guest.begin_coop_reload();
    assert!(!guest.can_share_coop_map());
    assert!(!guest.comment_tool_available());
    assert!(guest.state.document(local).unwrap().is_read_only());
    paint(
        &mut host,
        &host_dir.join("_maps/a.dmm"),
        Coord::new(1, 1, 1),
        "/obj/reloaded",
    );
    host.poll_coop();
    guest.poll_coop();
    assert_ne!(
        top(&guest, &file, Coord::new(1, 1, 1)).as_deref(),
        Some("/obj/reloaded")
    );
    guest.load_environment(&guest_dir.join("game.dme")).unwrap();
    assert_eq!(guest.coop().unwrap().you, you);
    assert!(!guest.coop().unwrap().is_paused());
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        top(sessions[1], &file, Coord::new(1, 1, 1)).as_deref() == Some("/obj/reloaded")
    });
    assert!(guest.state.document(local).unwrap().is_dirty());
    assert!(!guest.state.document(local).unwrap().is_read_only());
    guest.begin_coop_reload();
    guest.finish_coop_reload(); // A cancelled or failed load keeps the previous environment.
    assert_eq!(guest.coop().unwrap().you, you);
    assert!(guest.can_share_coop_map());
    let _ = fs::remove_dir_all(host_dir);
    let _ = fs::remove_dir_all(guest_dir);
}

#[test]
fn an_incompatible_reload_drops_waiting_snapshots_and_late_workers() {
    let (host_dir, mut host, guest_dir, mut guest, local) = guest_out_of_date("incompatible-reload");
    let stale_worker = guest.coop().unwrap().prepared.0.clone();
    let file = guest_dir.join("_maps/a.dmm");
    guest.begin_coop_reload();
    fs::write(guest_dir.join("game.dme"), "/obj/different").unwrap();
    guest.load_environment(&guest_dir.join("game.dme")).unwrap();
    assert!(matches!(
        guest.coop().unwrap().status,
        CoopStatus::CodebaseMismatch { .. }
    ));
    assert!(guest.coop().unwrap().client.is_none());
    assert!(guest.coop().unwrap().shared_maps.is_empty());
    assert!(guest.state.document(local).unwrap().is_dirty());
    assert!(!guest.state.document(local).unwrap().is_read_only());
    assert_eq!(guest.state.document(local).unwrap().map.size().x, 1);
    assert!(
        stale_worker
            .send(Prepared::Unreadable {
                path: "_maps/a.dmm".into(),
                generation: GenerationId(1)
            })
            .is_err()
    );
    deliver(
        &mut guest,
        &guest_dir,
        Event::MapIncoming {
            path: "_maps/a.dmm".into(),
            by: OTHER,
            len: 1,
        },
    );
    assert!(guest.coop().unwrap().shared_maps.is_empty());
    assert_eq!(guest.state.document(local).unwrap().map.size().x, 1);
    assert_eq!(width_on_disk(&file), 1);
    poll_until(&mut [&mut host], |sessions| {
        sessions[0].coop().unwrap().peers.is_empty()
    });
    let _ = fs::remove_dir_all(host_dir);
    let _ = fs::remove_dir_all(guest_dir);
}

#[test]
fn changing_a_hosts_codebase_stops_its_relay_and_closes_pending_documents() {
    let (dir, mut host) = hosting("host-codebase-change");
    let mut guest = Session::new();
    let (guest_dir, environment) = codebase_with_map("host-codebase-change-guest", "a");
    guest.state.environment = environment.state.environment.clone();
    let port = host.coop().unwrap().host().unwrap().port();
    guest
        .join_coop(
            format!("127.0.0.1:{port}"),
            net::hash_password("hunter2"),
            "guest".into(),
        )
        .unwrap();
    poll_until(&mut [&mut host, &mut guest], |sessions| connected(sessions[1]));
    incoming(&mut host, &dir);
    let pending = host.state.document_for_path(&dir.join("_maps/a.dmm")).unwrap();
    fs::write(dir.join("game.dme"), "/obj/new_codebase").unwrap();
    host.load_environment(&dir.join("game.dme")).unwrap();
    assert!(host.coop().unwrap().was_hosting());
    assert!(!host.coop().unwrap().is_hosting());
    assert!(host.state.document(pending).is_none());
    poll_until(&mut [&mut guest], |sessions| {
        matches!(sessions[0].coop().unwrap().status, CoopStatus::Ended(_))
    });
    poll_until(&mut [&mut host], |sessions| sessions[0].coop().unwrap().can_retry());
    host.retry_coop().unwrap();
    poll_until(&mut [&mut host], |sessions| connected(sessions[0]));
    guest.retry_coop().unwrap();
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        matches!(sessions[1].coop().unwrap().status, CoopStatus::CodebaseMismatch { .. })
    });
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(guest_dir);
}

#[test]
fn switching_codebase_roots_disconnects_even_when_the_loaded_bytes_match() {
    let (dir, mut host) = hosting("switch-codebase");
    let (other_dir, _) = codebase_with_map("switch-codebase-other", "a");
    host.begin_coop_reload();
    host.load_environment(&other_dir.join("game.dme")).unwrap();
    assert!(matches!(host.coop().unwrap().status, CoopStatus::Ended(_)));
    assert!(!host.coop().unwrap().is_hosting());
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(other_dir);
}
