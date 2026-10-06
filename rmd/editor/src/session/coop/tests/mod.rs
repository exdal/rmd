use core::path::TreePath;
use std::{
    env,
    fs,
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    process,
    thread,
    time::{Duration, Instant},
};

use dmm::{Coord, Prefab, parser};
use editor::{command::Edit, document::DocumentId, tool::Tool};
use net::{
    CodebaseHash,
    CodebaseId,
    Cursor,
    Direction,
    Event,
    GenerationId,
    Impairment,
    LossyProxy,
    PeerId,
    PeerInfo,
    Server,
    ServerConfig,
    View,
};

use super::*;
use crate::{
    loader::LoadedMap,
    session::{Session, fixtures},
};

mod codebase;
mod connection;
mod edits;
mod maps;

fn session() -> Session {
    let mut session = Session::new();
    session
        .load_environment(&fixtures::examples().join("test.dme"))
        .unwrap();

    session
}

fn poll_until(sessions: &mut [&mut Session], mut done: impl FnMut(&[&mut Session]) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(sessions) {
        assert!(Instant::now() < deadline, "timed out");
        for session in sessions.iter_mut() {
            session.poll_coop();
        }

        thread::sleep(Duration::from_millis(5));
    }
}

fn connected(session: &Session) -> bool {
    session
        .coop()
        .is_some_and(|coop| matches!(coop.status, CoopStatus::Connected))
}

const OTHER: PeerId = PeerId(99);

fn hosting(name: &str) -> (PathBuf, Session) {
    let (dir, mut session) = codebase_with_map(name, "aa");
    session
        .host_coop(0, net::hash_password("hunter2"), String::from("host"))
        .unwrap();
    poll_until(&mut [&mut session], |sessions| connected(sessions[0]));

    (dir, session)
}

fn deliver(session: &mut Session, dir: &Path, event: Event) {
    session.coop.as_mut().unwrap().apply(event, Some(dir));
    session.poll_coop();
}

fn incoming(session: &mut Session, dir: &Path) {
    deliver(
        session,
        dir,
        Event::MapIncoming {
            path: String::from("_maps/a.dmm"),
            by: OTHER,
            len: 1234,
        },
    );
}

fn codebase_with_map(name: &str, rows: &str) -> (PathBuf, Session) {
    let dir = env::temp_dir().join(format!("rmd-coop-{name}-{}", process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("_maps")).unwrap();
    fs::write(dir.join("game.dme"), "").unwrap();
    fs::write(
        dir.join("_maps/a.dmm"),
        format!("\"a\" = (/turf,/area)\n\n(1,1,1) = {{\"\n{rows}\n\"}}\n"),
    )
    .unwrap();

    let mut session = Session::new();
    session.load_environment(&dir.join("game.dme")).unwrap();

    (dir, session)
}

fn open_local(session: &mut Session, file: PathBuf) -> DocumentId {
    let text = fs::read_to_string(&file).unwrap();
    let (map, errors) = parser::parse(&text);
    assert!(errors.is_empty());
    session.apply_map(LoadedMap {
        path: file.clone(),
        map,
        z: 1,
        errors,
        repo: None,
        conflict: None,
    });

    session.state.document_for_path(&file).unwrap()
}

// the guest has an unsaved one tile wide copy while the host shares a two tile wide one
fn guest_out_of_date(name: &str) -> (PathBuf, Session, PathBuf, Session, DocumentId) {
    let (host_dir, mut host) = codebase_with_map(&format!("{name}-host"), "aa");
    let (guest_dir, mut guest) = codebase_with_map(&format!("{name}-guest"), "a");
    open_local(&mut host, host_dir.join("_maps/a.dmm"));
    let local = open_local(&mut guest, guest_dir.join("_maps/a.dmm"));
    guest.state.document_mut(local).unwrap().mark_unsaved();

    host.host_coop(0, net::hash_password("hunter2"), String::from("host"))
        .unwrap();
    poll_until(&mut [&mut host], |sessions| connected(sessions[0]));
    assert!(host.can_share_coop_map());
    host.share_coop_map();
    poll_until(&mut [&mut host], |sessions| {
        sessions[0]
            .coop()
            .is_some_and(|coop| coop.shared_maps.contains_key("_maps/a.dmm"))
    });

    let port = host.coop().and_then(Coop::host).unwrap().port();
    guest
        .join_coop(
            format!("127.0.0.1:{port}"),
            net::hash_password("hunter2"),
            String::from("guest"),
        )
        .unwrap();
    poll_until(&mut [&mut host, &mut guest], |sessions| {
        sessions[1]
            .coop()
            .and_then(|coop| coop.shared_maps.get("_maps/a.dmm"))
            .is_some_and(|shared_map| matches!(shared_map.state, SharedState::Waiting(_)))
    });
    assert_eq!(guest.state.document(local).unwrap().map.size.x, 1);

    (host_dir, host, guest_dir, guest, local)
}

fn width_on_disk(file: &Path) -> u32 { parser::parse(&fs::read_to_string(file).unwrap()).0.size.x }

fn paint(session: &mut Session, file: &Path, coord: Coord, path: &str) {
    let id = session.state.document_for_path(file).unwrap();
    let document = session.state.document_mut(id).unwrap();
    let mut tile = document.map.tile_at(coord).cloned().unwrap();
    tile.insert(0, Prefab::new(TreePath::parse(path)));

    let placed = tile.into_iter().map(|prefab| document.instantiate(prefab)).collect();
    let mut edit = Edit::new("paint");
    edit.change(document, coord, placed);
    assert!(document.apply(edit));
}

fn top(session: &Session, file: &Path, coord: Coord) -> Option<String> {
    let document = session.state.document(session.state.document_for_path(file)?)?;

    Some(document.map.tile_at(coord)?.first()?.path.to_string())
}

fn settled(sessions: &[&mut Session]) -> bool {
    sessions.iter().all(|session| {
        session.coop().is_some_and(|coop| {
            coop.shared_maps.values().all(|shared_map| {
                matches!(shared_map.state, SharedState::Ready | SharedState::Closed)
                    && shared_map.pending_document.is_none()
                    && shared_map.in_flight.is_empty()
                    && shared_map.inbox.is_empty()
                    && shared_map.level_request.is_none()
            })
        })
    })
}
