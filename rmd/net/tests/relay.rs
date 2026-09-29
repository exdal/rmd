use std::{
    net::{Ipv4Addr, SocketAddr},
    time::{Duration, Instant},
};

use net::{Client, CodebaseId, Cursor, Event, MapEdit, Server, ServerConfig};

const PASSWORD: &str = "hunter2";

fn server() -> Server {
    Server::spawn(ServerConfig {
        bind: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        password: String::from(PASSWORD),
    })
    .unwrap()
}

fn join(server: &Server, password: &str, nick: &str) -> Client {
    Client::connect(
        server.local_addr().to_string(),
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
        std::thread::sleep(Duration::from_millis(5));
    }
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
        std::thread::sleep(Duration::from_millis(20));
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
    alice.share_map(String::from("../outside.dmm"), b"nope".to_vec());
    alice.share_map(String::from("_maps/test.dmm"), map.clone());

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

    alice.share_map(String::from("_maps/test.dmm"), b"map".to_vec());
    let generation = wait_for(&bob, |event| match event {
        Event::MapShared { generation, .. } => Some(generation),
        _ => None,
    });
    assert_eq!(generation, 1);

    let edit = |generation, patch: &str| MapEdit {
        path: String::from("_maps/test.dmm"),
        generation,
        coords: vec![[1, 1, 1]],
        patch: String::from(patch),
    };
    bob.send_edit(edit(0, "stale"));
    bob.send_edit(edit(1, "fresh"));
    for client in [&alice, &bob] {
        let (by, received) = wait_for(client, |event| match event {
            Event::Edit { by, edit } => Some((by, edit)),
            _ => None,
        });
        assert_eq!(by, bob_id);
        assert_eq!(received, edit(1, "fresh"));
    }

    let carol = join(&server, PASSWORD, "carol");
    let replayed = wait_for(&carol, |event| match event {
        Event::Edit { edit, .. } => Some(edit),
        _ => None,
    });
    assert_eq!(replayed, edit(1, "fresh"));

    alice.share_map(String::from("_maps/test.dmm"), b"map again".to_vec());
    let generation = wait_for(&carol, |event| match event {
        Event::MapShared { generation, .. } => Some(generation),
        _ => None,
    });
    assert_eq!(generation, 2);

    let dave = join(&server, PASSWORD, "dave");
    wait_for(&dave, |event| match event {
        Event::MapSnapshot { generation, bytes, .. } => Some((generation, bytes)),
        Event::Edit { .. } => panic!("the log was not reset"),
        _ => None,
    });
}
