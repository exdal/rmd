use std::{
    net::{Ipv4Addr, SocketAddr},
    thread,
    time::{Duration, Instant},
};

use net::{
    Client,
    CodebaseId,
    Cursor,
    Direction,
    Event,
    GenerationId,
    Impairment,
    LossyProxy,
    MapEdit,
    SeqId,
    Server,
    ServerConfig,
};

const PASSWORD: &str = "hunter2";

fn server() -> Server {
    Server::spawn(ServerConfig {
        bind: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        password: String::from(PASSWORD),
    })
    .unwrap()
}

fn join(server: &Server, password: &str, nick: &str) -> Client { join_at(server.local_addr(), password, nick) }

fn join_at(addr: SocketAddr, password: &str, nick: &str) -> Client {
    Client::connect(
        addr.to_string(),
        password.to_owned(),
        nick.to_owned(),
        CodebaseId {
            hash: [7; 32],
            git_hint: None,
        },
    )
}

fn wait_for<T>(client: &Client, mut matches: impl FnMut(Event) -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(found) = client.poll().find_map(&mut matches) {
            return found;
        }

        assert!(Instant::now() < deadline, "timed out waiting for an event");
        thread::sleep(Duration::from_millis(5));
    }
}

fn collect_until(client: &Client, mut last: impl FnMut(&Event) -> bool) -> Vec<Event> {
    let mut events = Vec::new();
    wait_for(client, |event| {
        let done = last(&event);
        events.push(event);

        done.then_some(())
    });

    events
}

fn cursor(x: f32) -> Cursor {
    Cursor {
        map: String::from("_maps/test.dmm"),
        z: 1,
        pos: [x, 64.0],
    }
}

#[test]
fn cursors_reach_the_other_peers_stamped_with_the_sender() {
    let server = server();

    let alice = join(&server, PASSWORD, "alice");
    let (alice_id, peers) = wait_for(&alice, |event| match event {
        Event::Connected { you, peers, .. } => Some((you, peers)),
        _ => None,
    });
    assert!(peers.is_empty());

    let bob = join(&server, PASSWORD, "bob");
    let (bob_id, peers) = wait_for(&bob, |event| match event {
        Event::Connected { you, peers, .. } => Some((you, peers)),
        _ => None,
    });
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0].id, alice_id);
    assert_eq!(peers[0].nick, "alice");

    let joined = wait_for(&alice, |event| match event {
        Event::PeerJoined(peer) => Some(peer),
        _ => None,
    });
    assert_eq!(joined.id, bob_id);

    // Datagrams are unreliable, so keep moving until one arrives
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut x = 0.0;
    let received = loop {
        x += 1.0;
        alice.send_cursor(Some(cursor(x)));
        let received = bob.poll().find_map(|event| match event {
            Event::Cursor { from, cursor } => Some((from, cursor)),
            _ => None,
        });

        if let Some(received) = received {
            break received;
        }

        assert!(Instant::now() < deadline, "no cursor arrived");
        thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(received.0, alice_id);
    assert_eq!(
        received.1.map(|cursor| cursor.map),
        Some(String::from("_maps/test.dmm"))
    );

    drop(bob);
    let left = wait_for(&alice, |event| match event {
        Event::PeerLeft(id) => Some(id),
        _ => None,
    });
    assert_eq!(left, bob_id);
}

#[test]
fn a_wrong_password_is_rejected() {
    let server = server();
    let client = join(&server, "letmein", "mallory");

    let reason = wait_for(&client, |event| match event {
        Event::Rejected(reason) => Some(reason),
        Event::Connected { .. } => panic!("connected with a wrong password"),
        _ => None,
    });
    assert_eq!(reason, "wrong password");
}

#[test]
fn an_unreachable_host_disconnects() {
    let client = Client::connect(
        String::from("not a host"),
        String::from(PASSWORD),
        String::from("alice"),
        CodebaseId {
            hash: [0; 32],
            git_hint: None,
        },
    );

    wait_for(&client, |event| match event {
        Event::Disconnected(reason) => Some(reason),
        _ => None,
    });
}

#[test]
fn comments_reach_everyone_and_late_joiners() {
    let server = server();
    let alice = join(&server, PASSWORD, "alice");
    let alice_id = wait_for(&alice, |event| match event {
        Event::Connected { you, .. } => Some(you),
        _ => None,
    });

    let bob = join(&server, PASSWORD, "bob");
    wait_for(&bob, |event| matches!(event, Event::Connected { .. }).then_some(()));

    alice.send_comment(
        String::from("_maps/test.dmm"),
        1,
        [32.0, 64.0],
        String::from("  move this door\n"),
    );
    for client in [&alice, &bob] {
        let comment = wait_for(client, |event| match event {
            Event::Comment(comment) => Some(comment),
            _ => None,
        });
        assert_eq!(comment.author, alice_id);
        assert_eq!(comment.text, "move this door");
    }

    alice.send_comment(String::from("_maps/test.dmm"), 1, [0.0, 0.0], String::from("   "));
    let carol = join(&server, PASSWORD, "carol");
    let comments = wait_for(&carol, |event| match event {
        Event::Connected { comments, .. } => Some(comments),
        _ => None,
    });
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].text, "move this door");
}

#[test]
fn anyone_can_delete_a_comment_for_everyone() {
    let server = server();
    let alice = join(&server, PASSWORD, "alice");
    wait_for(&alice, |event| matches!(event, Event::Connected { .. }).then_some(()));
    let bob = join(&server, PASSWORD, "bob");
    wait_for(&bob, |event| matches!(event, Event::Connected { .. }).then_some(()));

    let comment = |client: &Client| {
        wait_for(client, |event| match event {
            Event::Comment(comment) => Some(comment.id),
            _ => None,
        })
    };

    for text in ["first", "second"] {
        alice.send_comment(String::from("_maps/test.dmm"), 1, [0.0, 0.0], String::from(text));
    }

    let first = comment(&bob);
    let second = comment(&bob);
    assert_ne!(first, second);

    bob.delete_comment(first);
    for client in [&alice, &bob] {
        let deleted = wait_for(client, |event| match event {
            Event::CommentDeleted(id) => Some(id),
            _ => None,
        });
        assert_eq!(deleted, first);
    }

    alice.send_comment(String::from("_maps/test.dmm"), 1, [0.0, 0.0], String::from("third"));
    let third = comment(&bob);
    assert!(third != first && third != second);

    let carol = join(&server, PASSWORD, "carol");
    let comments = wait_for(&carol, |event| match event {
        Event::Connected { comments, .. } => Some(comments),
        _ => None,
    });
    assert_eq!(
        comments.iter().map(|comment| comment.id).collect::<Vec<_>>(),
        [second, third]
    );
}

#[test]
fn shared_maps_reach_everyone_else_and_late_joiners() {
    let server = server();
    let alice = join(&server, PASSWORD, "alice");
    let alice_id = wait_for(&alice, |event| match event {
        Event::Connected { you, .. } => Some(you),
        _ => None,
    });

    let bob = join(&server, PASSWORD, "bob");
    wait_for(&bob, |event| matches!(event, Event::Connected { .. }).then_some(()));

    let map = b"\"aa\" = (/turf/open/floor)\n".repeat(50_000);
    alice.share_map(String::from("../outside.dmm"), None, b"nope".to_vec());
    alice.share_map(String::from("_maps/test.dmm"), None, map.clone());

    for client in [&alice, &bob] {
        let shared = wait_for(client, |event| match event {
            Event::MapShared { path, by, .. } => Some((path, by)),
            _ => None,
        });
        assert_eq!(shared, (String::from("_maps/test.dmm"), alice_id));
    }

    let received = wait_for(&bob, |event| match event {
        Event::MapSnapshot { path, bytes, .. } => Some((path, bytes)),
        _ => None,
    });
    assert_eq!(received.0, "_maps/test.dmm");
    assert_eq!(received.1, map);

    let carol = join(&server, PASSWORD, "carol");
    let (path, bytes) = wait_for(&carol, |event| match event {
        Event::MapSnapshot { path, bytes, .. } => Some((path, bytes)),
        _ => None,
    });
    assert_eq!(path, "_maps/test.dmm");
    assert_eq!(bytes, map);
}

#[test]
fn edits_reach_everyone_and_replay_for_late_joiners_until_the_map_is_shared_again() {
    let server = server();
    let alice = join(&server, PASSWORD, "alice");
    wait_for(&alice, |event| matches!(event, Event::Connected { .. }).then_some(()));
    let bob = join(&server, PASSWORD, "bob");
    let bob_id = wait_for(&bob, |event| match event {
        Event::Connected { you, .. } => Some(you),
        _ => None,
    });

    alice.share_map(String::from("_maps/test.dmm"), None, b"map".to_vec());
    let generation = wait_for(&bob, |event| match event {
        Event::MapShared { generation, .. } => Some(generation),
        _ => None,
    });
    assert_eq!(generation, GenerationId(1));

    let edit = |generation, patch: &str| MapEdit {
        path: String::from("_maps/test.dmm"),
        generation,
        coords: vec![[1, 1, 1]],
        patch: String::from(patch),
    };
    bob.send_edit(edit(GenerationId(0), "stale"));
    bob.send_edit(edit(GenerationId(1), "fresh"));
    for client in [&alice, &bob] {
        let (by, seq, received) = wait_for(client, |event| match event {
            Event::Edit { by, seq, edit } => Some((by, seq, edit)),
            _ => None,
        });
        assert_eq!(by, bob_id);
        assert_eq!(seq, SeqId(0));
        assert_eq!(received, edit(GenerationId(1), "fresh"));
    }

    let carol = join(&server, PASSWORD, "carol");
    let replayed = wait_for(&carol, |event| match event {
        Event::Edit { edit, .. } => Some(edit),
        _ => None,
    });
    assert_eq!(replayed, edit(GenerationId(1), "fresh"));

    alice.share_map(String::from("_maps/test.dmm"), None, b"map again".to_vec());
    let generation = wait_for(&carol, |event| match event {
        Event::MapShared { generation, .. } => Some(generation),
        _ => None,
    });
    assert_eq!(generation, GenerationId(2));

    let dave = join(&server, PASSWORD, "dave");
    wait_for(&dave, |event| match event {
        Event::MapSnapshot { generation, bytes, .. } => Some((generation, bytes)),
        Event::Edit { .. } => panic!("the log was not reset"),
        _ => None,
    });
}

#[test]
fn a_new_share_carries_the_edits_its_snapshot_missed() {
    let server = server();
    let alice = join(&server, PASSWORD, "alice");
    let alice_id = wait_for(&alice, |event| match event {
        Event::Connected { you, .. } => Some(you),
        _ => None,
    });
    let bob = join(&server, PASSWORD, "bob");
    let bob_id = wait_for(&bob, |event| match event {
        Event::Connected { you, .. } => Some(you),
        _ => None,
    });

    let path = String::from("_maps/test.dmm");
    let edit = |generation, patch: &str| MapEdit {
        path: path.clone(),
        generation,
        coords: vec![[1, 1, 1]],
        patch: String::from(patch),
    };
    let next_edit = |client: &Client| {
        wait_for(client, |event| match event {
            Event::Edit { by, seq, edit } => Some((by, seq, edit)),
            _ => None,
        })
    };
    let shared = |client: &Client, generation| {
        wait_for(client, |event| {
            matches!(event, Event::MapShared { generation: shared, .. } if shared == generation).then_some(())
        });
    };

    alice.share_map(path.clone(), None, b"map".to_vec());
    shared(&bob, GenerationId(1));

    bob.send_edit(edit(GenerationId(1), "seen"));
    assert_eq!(next_edit(&alice).1, SeqId(0));
    alice.send_edit(edit(GenerationId(1), "own"));
    assert_eq!(next_edit(&alice).1, SeqId(1));
    bob.send_edit(edit(GenerationId(1), "missed"));
    assert_eq!(next_edit(&alice).1, SeqId(2));

    // the snapshot has the first edit, and alice's own edit is in it too
    alice.share_map(path.clone(), Some((GenerationId(1), SeqId(1))), b"map again".to_vec());
    shared(&bob, GenerationId(2));
    assert_eq!(next_edit(&bob), (bob_id, SeqId(0), edit(GenerationId(2), "missed")));

    bob.send_edit(edit(GenerationId(1), "late"));
    assert_eq!(next_edit(&bob), (bob_id, SeqId(1), edit(GenerationId(2), "late")));

    alice.send_edit(edit(GenerationId(1), "stale"));
    alice.send_edit(edit(GenerationId(2), "after"));
    assert_eq!(next_edit(&bob), (alice_id, SeqId(2), edit(GenerationId(2), "after")));
}

#[test]
fn a_resync_sends_the_snapshot_and_the_numbered_log_again() {
    let server = server();
    let alice = join(&server, PASSWORD, "alice");
    wait_for(&alice, |event| matches!(event, Event::Connected { .. }).then_some(()));
    let bob = join(&server, PASSWORD, "bob");
    wait_for(&bob, |event| matches!(event, Event::Connected { .. }).then_some(()));

    let path = String::from("_maps/test.dmm");
    alice.share_map(path.clone(), None, b"map".to_vec());
    wait_for(&bob, |event| matches!(event, Event::MapSnapshot { .. }).then_some(()));

    let edit = |patch: &str| MapEdit {
        path: path.clone(),
        generation: GenerationId(1),
        coords: vec![[1, 1, 1]],
        patch: String::from(patch),
    };
    bob.send_edit(edit("first"));
    bob.send_edit(edit("second"));
    wait_for(&bob, |event| {
        matches!(event, Event::Edit { seq: SeqId(1), .. }).then_some(())
    });

    bob.resync(path.clone());
    let mut snapshot = None;
    let mut replayed = Vec::new();
    while snapshot.is_none() || replayed.len() < 2 {
        let event = wait_for(&bob, |event| match event {
            Event::MapSnapshot { .. } | Event::Edit { .. } => Some(event),
            _ => None,
        });

        match event {
            Event::MapSnapshot { generation, bytes, .. } => snapshot = Some((generation, bytes)),
            Event::Edit { seq, edit, .. } => replayed.push((seq, edit.patch)),
            _ => unreachable!(),
        }
    }
    assert_eq!(snapshot, Some((GenerationId(1), b"map".to_vec())));
    assert_eq!(
        replayed,
        [(SeqId(0), String::from("first")), (SeqId(1), String::from("second"))]
    );
}

#[test]
fn edits_and_snapshots_survive_a_lossy_link() {
    let server = server();
    let proxy = LossyProxy::spawn(
        SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        server.local_addr(),
        Impairment {
            latency: Duration::from_millis(20),
            jitter: Duration::from_millis(20),
            loss: 0.05,
            seed: 7,
        },
    )
    .unwrap();

    let alice = join_at(proxy.local_addr(), PASSWORD, "alice");
    wait_for(&alice, |event| matches!(event, Event::Connected { .. }).then_some(()));
    let bob = join_at(proxy.local_addr(), PASSWORD, "bob");
    wait_for(&bob, |event| matches!(event, Event::Connected { .. }).then_some(()));

    let path = String::from("_maps/test.dmm");
    let map = (0..200_000).map(|i| b'a' + (i % 26) as u8).collect::<Vec<_>>();
    alice.share_map(path.clone(), None, map.clone());
    let received = wait_for(&bob, |event| match event {
        Event::MapSnapshot { bytes, .. } => Some(bytes),
        _ => None,
    });
    assert!(received == map, "the snapshot arrived damaged");

    for patch in 0..50 {
        bob.send_edit(MapEdit {
            path: path.clone(),
            generation: GenerationId(1),
            coords: vec![[1, 1, 1]],
            patch: patch.to_string(),
        });
    }

    let mut edits = Vec::new();
    while edits.len() < 50 {
        edits.push(wait_for(&alice, |event| match event {
            Event::Edit { seq, edit, .. } => Some((seq, edit.patch)),
            _ => None,
        }));
    }
    assert_eq!(
        edits,
        (0..50).map(|seq| (SeqId(seq), seq.to_string())).collect::<Vec<_>>()
    );

    alice.send_comment(path, 1, [0.0, 0.0], String::from("laggy"));
    for client in [&alice, &bob] {
        wait_for(client, |event| matches!(event, Event::Comment(_)).then_some(()));
    }
}

#[test]
fn peers_see_an_upload_before_it_lands() {
    let server = server();
    let alice = join(&server, PASSWORD, "alice");
    let alice_id = wait_for(&alice, |event| match event {
        Event::Connected { you, .. } => Some(you),
        _ => None,
    });

    let bob = join(&server, PASSWORD, "bob");
    wait_for(&bob, |event| matches!(event, Event::Connected { .. }).then_some(()));

    let path = String::from("_maps/test.dmm");
    let map = vec![b'a'; 1_300_000];
    let len = map.len() as u64;
    alice.share_map(path.clone(), None, map);

    let sent = collect_until(&alice, |event| matches!(event, Event::MapShared { .. }));
    let sending = sent
        .iter()
        .filter_map(|event| match event {
            Event::Progress {
                direction: Direction::Sending,
                done,
                total,
                ..
            } => Some((*done, *total)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(sending.first(), Some(&(0, len)));
    assert_eq!(sending.last(), Some(&(len, len)));

    let received = collect_until(&bob, |event| matches!(event, Event::MapSnapshot { .. }));
    let incoming = received
        .iter()
        .position(|event| {
            *event
                == Event::MapIncoming {
                    path: path.clone(),
                    by: alice_id,
                    len,
                }
        })
        .expect("bob never heard about the upload");
    let shared = received
        .iter()
        .position(|event| matches!(event, Event::MapShared { .. }))
        .expect("bob never heard about the share");
    assert!(incoming < shared);

    let receiving = received
        .iter()
        .filter_map(|event| match event {
            Event::Progress {
                direction: Direction::Receiving,
                done,
                total,
                ..
            } => Some((*done, *total)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(receiving.first(), Some(&(0, len)));
    assert_eq!(receiving.last(), Some(&(len, len)));
}

#[test]
fn a_dropped_upload_is_cancelled() {
    let server = server();
    let proxy = LossyProxy::spawn(
        SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        server.local_addr(),
        Impairment {
            latency: Duration::from_millis(50),
            ..Impairment::default()
        },
    )
    .unwrap();

    let alice = join_at(proxy.local_addr(), PASSWORD, "alice");
    let alice_id = wait_for(&alice, |event| match event {
        Event::Connected { you, .. } => Some(you),
        _ => None,
    });

    let bob = join(&server, PASSWORD, "bob");
    wait_for(&bob, |event| matches!(event, Event::Connected { .. }).then_some(()));

    let path = String::from("_maps/test.dmm");
    alice.share_map(path.clone(), None, vec![b'a'; 32 * 1024 * 1024]);
    wait_for(&bob, |event| matches!(event, Event::MapIncoming { .. }).then_some(()));
    drop(alice);

    let cancelled = wait_for(&bob, |event| match event {
        Event::MapCancelled { path, by } => Some((path, by)),
        Event::MapShared { .. } => panic!("the upload finished before it was dropped"),
        _ => None,
    });
    assert_eq!(cancelled, (path, alice_id));

    let carol = join(&server, PASSWORD, "carol");
    wait_for(&carol, |event| matches!(event, Event::Connected { .. }).then_some(()));
    thread::sleep(Duration::from_millis(200));
    assert!(!carol.poll().any(|event| matches!(event, Event::MapIncoming { .. })));
}
