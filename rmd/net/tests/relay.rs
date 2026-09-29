use std::{
    net::{Ipv4Addr, SocketAddr},
    time::{Duration, Instant},
};

use net::{Client, CodebaseId, Cursor, Event, Server, ServerConfig};

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
        Event::Connected { you, peers } => Some((you, peers)),
        _ => None,
    });
    assert!(peers.is_empty());

    let bob = join(&server, PASSWORD, "bob");
    let (bob_id, peers) = wait_for(&bob, |event| match event {
        Event::Connected { you, peers } => Some((you, peers)),
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
