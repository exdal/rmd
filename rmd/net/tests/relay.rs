use std::{
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
    thread,
    time::{Duration, Instant},
};

use net::{
    Client,
    CodebaseHash,
    CodebaseId,
    Cursor,
    Direction,
    Event,
    GenerationId,
    Impairment,
    LossyProxy,
    MapEdit,
    PRESENCE_REFRESH,
    Selection,
    SelectionMode,
    SeqId,
    Server,
    ServerConfig,
    Tool,
    View,
    hash_password,
};

const PASSWORD: &str = "hunter2";

fn server() -> Server {
    Server::spawn(ServerConfig {
        bind: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        codebase: None,
        password: hash_password(PASSWORD),
    })
    .unwrap()
}

fn join(server: &Server, password: &str, nick: &str) -> Client { join_at(server.local_addr(), password, nick) }

fn join_at(addr: SocketAddr, password: &str, nick: &str) -> Client {
    Client::connect(
        addr.to_string(),
        hash_password(password),
        nick.to_owned(),
        CodebaseId {
            hash: CodebaseHash([7; 32]),
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
        tool: Tool::Place,
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
fn views_reach_the_other_peers_and_repeat() {
    let server = server();
    let alice = join(&server, PASSWORD, "alice");
    let alice_id = wait_for(&alice, |event| match event {
        Event::Connected { you, .. } => Some(you),
        _ => None,
    });

    let bob = join(&server, PASSWORD, "bob");
    wait_for(&bob, |event| matches!(event, Event::Connected { .. }).then_some(()));

    let view = View {
        map: String::from("_maps/test.dmm"),
        z: 2,
        center: [320.0, 160.0],
        zoom: 1.5,
    };
    alice.send_view(Some(view.clone()));
    let received = |client: &Client| {
        wait_for(client, |event| match event {
            Event::View { from, view: Some(view) } => Some((from, view)),
            _ => None,
        })
    };

    assert_eq!(received(&bob), (alice_id, view.clone()));

    // nothing new was sent, so this one is the refresh
    assert_eq!(received(&bob), (alice_id, view));
}

#[test]
fn selections_reach_the_other_peers_and_repeat() {
    let server = server();
    let alice = join(&server, PASSWORD, "alice");
    let alice_id = wait_for(&alice, |event| match event {
        Event::Connected { you, .. } => Some(you),
        _ => None,
    });

    let bob = join(&server, PASSWORD, "bob");
    wait_for(&bob, |event| matches!(event, Event::Connected { .. }).then_some(()));

    let selection = Selection {
        map: String::from("_maps/test.dmm"),
        z: 2,
        min: [3, 4],
        max: [10, 12],
        mode: SelectionMode::Full,
    };
    alice.send_selection(Some(selection.clone()));
    let received = |client: &Client| {
        wait_for(client, |event| match event {
            Event::Selection {
                from,
                selection: Some(selection),
            } => Some((from, selection)),
            _ => None,
        })
    };

    assert_eq!(received(&bob), (alice_id, selection.clone()));

    // nothing new was sent, so this one is the refresh
    assert_eq!(received(&bob), (alice_id, selection));
}

#[test]
fn a_parked_cursor_repeats() {
    let server = server();
    let alice = join(&server, PASSWORD, "alice");
    wait_for(&alice, |event| matches!(event, Event::Connected { .. }).then_some(()));
    let bob = join(&server, PASSWORD, "bob");
    wait_for(&bob, |event| matches!(event, Event::Connected { .. }).then_some(()));

    alice.send_cursor(Some(cursor(1.0)));
    let received = |client: &Client| {
        wait_for(client, |event| match event {
            Event::Cursor {
                cursor: Some(cursor), ..
            } => Some(cursor),
            _ => None,
        })
    };

    assert_eq!(received(&bob), cursor(1.0));

    // nothing new was sent, so this one is the refresh
    assert_eq!(received(&bob), cursor(1.0));
}

#[test]
fn a_cleared_selection_is_sent_once() {
    let server = server();
    let alice = join(&server, PASSWORD, "alice");
    wait_for(&alice, |event| matches!(event, Event::Connected { .. }).then_some(()));
    let bob = join(&server, PASSWORD, "bob");
    wait_for(&bob, |event| matches!(event, Event::Connected { .. }).then_some(()));

    alice.send_selection(Some(Selection {
        map: String::from("_maps/test.dmm"),
        z: 1,
        min: [1, 1],
        max: [2, 2],
        mode: SelectionMode::Full,
    }));
    wait_for(&bob, |event| {
        matches!(event, Event::Selection { selection: Some(_), .. }).then_some(())
    });

    alice.send_selection(None);
    wait_for(&bob, |event| {
        matches!(event, Event::Selection { selection: None, .. }).then_some(())
    });

    thread::sleep(PRESENCE_REFRESH * 2);
    assert!(!bob.poll().any(|event| matches!(event, Event::Selection { .. })));
}

#[test]
fn stats_count_relayed_presence() {
    let server = server();
    let alice = join(&server, PASSWORD, "alice");
    wait_for(&alice, |event| matches!(event, Event::Connected { .. }).then_some(()));
    let bob = join(&server, PASSWORD, "bob");
    wait_for(&bob, |event| matches!(event, Event::Connected { .. }).then_some(()));

    alice.send_cursor(Some(cursor(1.0)));
    wait_for(&bob, |event| matches!(event, Event::Cursor { .. }).then_some(()));

    let stats = server.stats();
    assert_eq!(
        stats.peers.iter().map(|peer| peer.nick.as_str()).collect::<Vec<_>>(),
        ["alice", "bob"]
    );
    assert!(
        stats
            .peers
            .iter()
            .all(|peer| peer.sent.bytes > 0 && peer.received.packets > 0)
    );

    let cursors = stats.received.0["cursor"];
    assert_eq!(cursors.messages, 1);
    assert!(cursors.bytes > 0);
    assert_eq!(stats.sent.0["cursor"].messages, 1);
    assert_eq!(stats.sent.0["welcome"].messages, 2);
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

fn join_codebase(server: &Server, hash: u8, hint: &str) -> Client {
    Client::connect(
        server.local_addr().to_string(),
        hash_password(PASSWORD),
        format!("peer-{hash}"),
        CodebaseId {
            hash: CodebaseHash([hash; 32]),
            git_hint: Some(hint.to_owned()),
        },
    )
}

#[test]
fn a_different_codebase_receives_only_a_rejection_even_with_cached_data() {
    let server = server();
    let alice = join_codebase(&server, 7, "main");
    wait_for(&alice, |event| matches!(event, Event::Connected { .. }).then_some(()));
    alice.send_comment(
        "_maps/test.dmm".into(),
        1,
        [0.0, 0.0],
        "private to this codebase".into(),
    );
    wait_for(&alice, |event| matches!(event, Event::Comment(_)).then_some(()));
    alice.share_map("_maps/test.dmm".into(), None, b"map".to_vec());
    let generation = wait_for(&alice, |event| match event {
        Event::MapShared { generation, .. } => Some(generation),
        _ => None,
    });
    alice.send_edit(MapEdit {
        path: "_maps/test.dmm".into(),
        generation,
        coords: vec![[1, 1, 1]],
        patch: "edit".into(),
        new_level: None,
    });
    wait_for(&alice, |event| matches!(event, Event::Edit { .. }).then_some(()));

    let wrong = join_codebase(&server, 8, "main");
    wrong.share_map("_maps/wrong.dmm".into(), None, b"wrong codebase".to_vec());
    let expected = wait_for(&wrong, |event| match event {
        Event::CodebaseMismatch { expected } => Some(expected),
        other => panic!("mismatched client received {other:?}"),
    });
    assert_eq!(expected.hash, CodebaseHash([7; 32]));
    assert_eq!(expected.git_hint.as_deref(), Some("main"));

    let matching = join_codebase(&server, 7, "another-branch");
    let peers = wait_for(&matching, |event| match event {
        Event::Connected { peers, codebase, .. } => {
            assert_eq!(codebase, expected);
            Some(peers)
        },
        _ => None,
    });
    assert_eq!(peers.len(), 1, "a rejected peer was announced");
    assert!(
        matching
            .poll()
            .all(|event| !matches!(event, Event::MapShared { path, .. } if path == "_maps/wrong.dmm"))
    );
}

#[test]
fn hosting_pins_the_codebase_before_the_first_client() {
    let expected = CodebaseId {
        hash: CodebaseHash([7; 32]),
        git_hint: None,
    };
    let server = Server::spawn(ServerConfig {
        bind: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        password: hash_password(PASSWORD),
        codebase: Some(expected.clone()),
    })
    .unwrap();
    let wrong = join_codebase(&server, 8, "main");
    assert_eq!(
        wait_for(&wrong, |event| match event {
            Event::CodebaseMismatch { expected } => Some(expected),
            other => panic!("mismatched first client received {other:?}"),
        }),
        expected
    );
    let matching = join_codebase(&server, 7, "main");
    wait_for(&matching, |event| {
        matches!(event, Event::Connected { .. }).then_some(())
    });
}

#[test]
fn simultaneous_first_clients_cannot_establish_different_codebases() {
    let server = server();
    let clients = [join_codebase(&server, 7, "main"), join_codebase(&server, 8, "main")];
    let admitted = clients
        .iter()
        .map(|client| {
            wait_for(client, |event| match event {
                Event::Connected { codebase, .. } => Some((true, codebase.hash)),
                Event::CodebaseMismatch { expected } => Some((false, expected.hash)),
                _ => None,
            })
        })
        .collect::<Vec<_>>();
    assert_ne!(admitted[0].0, admitted[1].0);
    assert_eq!(admitted[0].1, admitted[1].1);
}

#[test]
fn an_unreachable_host_disconnects() {
    let client = Client::connect(
        String::from("not a host"),
        hash_password(PASSWORD),
        String::from("alice"),
        CodebaseId {
            hash: CodebaseHash([0; 32]),
            git_hint: None,
        },
    );

    wait_for(&client, |event| match event {
        Event::Disconnected(reason) => Some(reason),
        _ => None,
    });
}

#[test]
fn a_dual_stack_server_accepts_ipv4_and_ipv6_clients() {
    let server = Server::spawn(ServerConfig {
        bind: SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0)),
        codebase: None,
        password: hash_password(PASSWORD),
    })
    .unwrap();
    let port = server.local_addr().port();

    let alice = join_at(SocketAddr::from((Ipv4Addr::LOCALHOST, port)), PASSWORD, "alice");
    wait_for(&alice, |event| match event {
        Event::Connected { .. } => Some(()),
        _ => None,
    });

    // without IPv6 the server falls back to IPv4 and only that half applies
    if !server.local_addr().is_ipv6() {
        return;
    }

    let bob = join_at(SocketAddr::from((Ipv6Addr::LOCALHOST, port)), PASSWORD, "bob");
    let peers = wait_for(&bob, |event| match event {
        Event::Connected { peers, .. } => Some(peers),
        _ => None,
    });
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0].nick, "alice");
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
        new_level: None,
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
fn anyone_can_stop_sharing_a_map_for_everyone() {
    let server = server();
    let alice = join(&server, PASSWORD, "alice");
    wait_for(&alice, |event| matches!(event, Event::Connected { .. }).then_some(()));
    let bob = join(&server, PASSWORD, "bob");
    let bob_id = wait_for(&bob, |event| match event {
        Event::Connected { you, .. } => Some(you),
        _ => None,
    });

    alice.share_map(String::from("_maps/test.dmm"), None, b"map".to_vec());
    alice.share_map(String::from("_maps/other.dmm"), None, b"other".to_vec());
    alice.send_comment(String::from("_maps/test.dmm"), 1, [0.0, 0.0], String::from("gone"));
    alice.send_comment(String::from("_maps/other.dmm"), 1, [0.0, 0.0], String::from("kept"));
    let mut pending = 4;
    collect_until(&bob, |event| {
        if matches!(event, Event::MapShared { .. } | Event::Comment(_)) {
            pending -= 1;
        }

        pending == 0
    });

    bob.unshare_map(String::from("_maps/test.dmm"));
    for client in [&alice, &bob] {
        let unshared = wait_for(client, |event| match event {
            Event::MapUnshared { path, by } => Some((path, by)),
            _ => None,
        });
        assert_eq!(unshared, (String::from("_maps/test.dmm"), bob_id));
    }

    bob.send_edit(MapEdit {
        path: String::from("_maps/test.dmm"),
        generation: GenerationId(1),
        coords: vec![[1, 1, 1]],
        patch: String::from("late"),
        new_level: None,
    });
    bob.send_comment(String::from("_maps/other.dmm"), 1, [0.0, 0.0], String::from("marker"));
    let events = collect_until(&alice, |event| matches!(event, Event::Comment(_)));
    assert!(
        !events.iter().any(|event| matches!(event, Event::Edit { .. })),
        "an edit to a map nobody shares went through"
    );

    let carol = join(&server, PASSWORD, "carol");
    let comments = wait_for(&carol, |event| match event {
        Event::Connected { comments, .. } => Some(comments),
        _ => None,
    });
    assert_eq!(
        comments.iter().map(|comment| comment.text.as_str()).collect::<Vec<_>>(),
        ["kept", "marker"]
    );

    let events = collect_until(&carol, |event| matches!(event, Event::MapSnapshot { .. }));
    for event in events {
        if let Event::MapShared { path, .. } | Event::MapSnapshot { path, .. } = event {
            assert_eq!(path, "_maps/other.dmm");
        }
    }
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
        new_level: None,
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
fn heavy_traffic_both_ways_does_not_stall_the_session() {
    const EDITS: u32 = 20;

    let server = server();
    let alice = join(&server, PASSWORD, "alice");
    wait_for(&alice, |event| matches!(event, Event::Connected { .. }).then_some(()));

    let path = String::from("_maps/test.dmm");
    alice.share_map(path.clone(), None, b"map".to_vec());
    wait_for(&alice, |event| matches!(event, Event::MapShared { .. }).then_some(()));

    // every edit is echoed back, so both ends write far more than a stream window at once
    for _ in 0..EDITS {
        alice.send_edit(MapEdit {
            path: path.clone(),
            generation: GenerationId(1),
            coords: vec![[1, 1, 1]],
            patch: "a".repeat(512 * 1024),
            new_level: None,
        });
    }

    for expected in 0..EDITS {
        let seq = wait_for(&alice, |event| match event {
            Event::Edit { seq, .. } => Some(seq),
            _ => None,
        });
        assert_eq!(seq, SeqId(expected));
    }
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
        new_level: None,
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
            new_level: None,
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

const SLOW_MAP: &str = "_maps/slow.dmm";

// bob sits behind a slow link, so a large snapshot is still on its way to him when this returns
fn start_slow_download(server: &Server) -> (Client, LossyProxy, Client) {
    let alice = join(server, PASSWORD, "alice");
    wait_for(&alice, |event| matches!(event, Event::Connected { .. }).then_some(()));

    let proxy = LossyProxy::spawn(
        SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        server.local_addr(),
        Impairment {
            latency: Duration::from_millis(100),
            jitter: Duration::ZERO,
            loss: 0.0,
            seed: 1,
        },
    )
    .unwrap();
    let bob = join_at(proxy.local_addr(), PASSWORD, "bob");
    wait_for(&bob, |event| matches!(event, Event::Connected { .. }).then_some(()));

    alice.share_map(String::from(SLOW_MAP), None, vec![b'a'; 8_000_000]);
    wait_for(&bob, |event| match event {
        Event::Progress {
            path,
            direction: Direction::Receiving,
            ..
        } if path == SLOW_MAP => Some(()),
        _ => None,
    });

    (alice, proxy, bob)
}

// the rest of the 8 MB would keep reaching the proxy if the server were still pushing it
fn assert_push_stopped(proxy: &LossyProxy) {
    let before = proxy.downstream_bytes();
    thread::sleep(Duration::from_millis(1500));
    let sent = proxy.downstream_bytes() - before;
    assert!(
        sent < 200_000,
        "the server kept pushing, {sent} more bytes reached bob's link"
    );
}

#[test]
fn unsharing_a_map_stops_its_download() {
    let server = server();
    let (alice, proxy, bob) = start_slow_download(&server);

    alice.unshare_map(String::from(SLOW_MAP));
    wait_for(&bob, |event| matches!(event, Event::MapUnshared { .. }).then_some(()));
    assert_push_stopped(&proxy);

    for event in bob.poll() {
        let is_late = match &event {
            Event::Progress { path, .. } | Event::TransferFailed { path, .. } | Event::MapSnapshot { path, .. } => {
                path == SLOW_MAP
            },
            _ => false,
        };
        assert!(!is_late, "{event:?} arrived after the map stopped being shared");
    }
}

#[test]
fn sharing_a_map_again_replaces_its_download_in_flight() {
    let server = server();
    let (alice, proxy, bob) = start_slow_download(&server);

    alice.share_map(String::from(SLOW_MAP), None, b"replacement".to_vec());
    wait_for(&bob, |event| match event {
        Event::TransferFailed { path, reason, .. } if path == SLOW_MAP => {
            panic!("the replaced download reported a failure: {reason}")
        },
        Event::MapSnapshot { path, bytes, .. } if path == SLOW_MAP => {
            assert_eq!(bytes, b"replacement", "the replaced snapshot still arrived");
            Some(())
        },
        _ => None,
    });
    assert_push_stopped(&proxy);
}

#[test]
fn stopping_the_server_mid_download_disconnects_peers_promptly() {
    let server = server();
    let (_alice, _proxy, bob) = start_slow_download(&server);

    let started = Instant::now();
    drop(server);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "shutting down took {:?}",
        started.elapsed()
    );

    wait_for(&bob, |event| matches!(event, Event::Disconnected(_)).then_some(()));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "bob heard of the shutdown after {:?}",
        started.elapsed()
    );
}

#[test]
fn sharing_a_map_again_after_unsharing_gets_a_newer_generation() {
    let server = server();
    let alice = join(&server, PASSWORD, "alice");
    wait_for(&alice, |event| matches!(event, Event::Connected { .. }).then_some(()));
    let bob = join(&server, PASSWORD, "bob");
    wait_for(&bob, |event| matches!(event, Event::Connected { .. }).then_some(()));

    let path = String::from("_maps/test.dmm");
    let snapshot = |client: &Client| {
        wait_for(client, |event| match event {
            Event::MapSnapshot { generation, bytes, .. } => Some((generation, bytes)),
            _ => None,
        })
    };

    alice.share_map(path.clone(), None, b"first".to_vec());
    let (first, _) = snapshot(&bob);

    alice.unshare_map(path.clone());
    wait_for(&bob, |event| matches!(event, Event::MapUnshared { .. }).then_some(()));

    alice.share_map(path, None, b"second".to_vec());
    let (second, bytes) = snapshot(&bob);
    assert!(second > first, "{second:?} must be newer than {first:?}");
    assert_eq!(bytes, b"second");
}
