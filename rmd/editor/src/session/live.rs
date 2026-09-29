use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::mpsc,
    time::Instant,
};

use dmm::Coord;
use editor::{document::DocumentId, tool::Tool};
use net::{Client, CodebaseId, Comment, CommentId, Cursor, Event, MapEdit, PeerId, PeerInfo, Server, ServerConfig};

use super::Session;
use crate::loader::LoadedMap;

// how fast a remote cursor closes the gap to its latest position, per second
const CURSOR_SMOOTHING: f32 = 20.0;

pub(crate) enum LiveStatus {
    Hashing,
    Connecting,
    Connected,
    Ended(String),
}

pub(crate) struct RemotePeer {
    pub info: PeerInfo,
    pub cursor: Option<Cursor>,
    pub shown: [f32; 2],
}

impl RemotePeer {
    fn new(info: PeerInfo) -> Self {
        Self {
            info,
            cursor: None,
            shown: [0.0; 2],
        }
    }

    fn set_cursor(&mut self, cursor: Option<Cursor>) {
        if let Some(next) = cursor.as_ref()
            && self
                .cursor
                .as_ref()
                .is_none_or(|previous| previous.map != next.map || previous.z != next.z)
        {
            self.shown = next.pos;
        }

        self.cursor = cursor;
    }

    fn follow(&mut self, elapsed: f32) {
        let Some(cursor) = self.cursor.as_ref() else {
            return;
        };

        let blend = 1.0 - (-elapsed * CURSOR_SMOOTHING).exp();
        for (shown, target) in self.shown.iter_mut().zip(cursor.pos) {
            *shown += (target - *shown) * blend;
        }
    }
}

pub(crate) struct LiveShare {
    pub status: LiveStatus,
    pub nick: String,
    pub you: Option<PeerId>,
    pub peers: BTreeMap<PeerId, RemotePeer>,
    pub comments: BTreeMap<CommentId, Comment>,
    pub shared: BTreeMap<String, SharedMap>,
    pub skipped: BTreeMap<String, ReceivedMap>,
    received: BTreeSet<String>,
    prepared: (mpsc::Sender<Prepared>, mpsc::Receiver<Prepared>),
    server: Option<Server>,
    codebase: Option<CodebaseId>,
    pending: Option<PendingJoin>,
    client: Option<Client>,
    last_poll: Instant,
}

pub(crate) struct SharedMap {
    pub by: PeerId,
    generation: u32,
    loaded_generation: Option<u32>,
    // edits from the server that arrived before our copy of the map was loaded
    inbox: Vec<(PeerId, MapEdit)>,
    // tiles we edited whose edits the server hasn't sent back yet, counted per tile
    in_flight: HashMap<Coord, u32>,
}

impl SharedMap {
    fn new(by: PeerId, generation: u32) -> Self {
        Self {
            by,
            generation,
            loaded_generation: None,
            inbox: Vec::new(),
            in_flight: HashMap::new(),
        }
    }

    fn is_ready(&self) -> bool { self.loaded_generation == Some(self.generation) }
}

enum Prepared {
    Upload { path: String, bytes: Vec<u8> },
    Snapshot(ReceivedMap),
}

pub(crate) struct ReceivedMap {
    path: String,
    generation: u32,
    file: PathBuf,
    map: dmm::Map,
    errors: Vec<dmm::error::MapError>,
    modified: bool,
}

struct PendingJoin {
    addr: String,
    password: String,
    codebase: mpsc::Receiver<Result<CodebaseId, String>>,
}

impl LiveShare {
    pub fn is_hosting(&self) -> bool { self.server.is_some() }

    pub fn is_connected(&self) -> bool { matches!(self.status, LiveStatus::Connected) }

    pub fn nick_of(&self, id: PeerId) -> Option<&str> {
        if self.you == Some(id) {
            return Some(&self.nick);
        }

        self.peers.get(&id).map(|peer| peer.info.nick.as_str())
    }

    pub fn host(&self) -> Option<(SocketAddr, &str)> {
        self.server
            .as_ref()
            .map(|server| (server.local_addr(), server.password()))
    }

    pub fn same_codebase(&self, peer: &PeerInfo) -> bool {
        self.codebase
            .as_ref()
            .is_none_or(|codebase| codebase.hash == peer.codebase.hash)
    }

    pub fn git_hint(&self) -> Option<&str> { self.codebase.as_ref()?.git_hint.as_deref() }

    fn receive_map(&self, path: String, generation: u32, bytes: Vec<u8>, codebase: Option<&Path>) {
        if !net::is_map_path(&path) {
            log::warn!("ignoring a shared map outside the codebase: {path}");
            return;
        }

        let Some(file) = codebase.map(|codebase| codebase.join(&path)) else {
            return;
        };

        let prepared = self.prepared.0.clone();
        std::thread::spawn(move || {
            let modified = std::fs::read(&file).map_or(true, |disk| disk != bytes);
            let text = match String::from_utf8(bytes) {
                Ok(text) => text,
                Err(e) => {
                    log::warn!("{path}: the shared map is not text: {e}");
                    return;
                },
            };

            let (map, errors) = dmm::parser::parse(&text);
            let _ = prepared.send(Prepared::Snapshot(ReceivedMap {
                path,
                generation,
                file,
                map,
                errors,
                modified,
            }));
        });
    }

    fn apply(&mut self, event: Event, codebase: Option<&Path>) {
        match event {
            Event::Connected { you, peers, comments } => {
                self.status = LiveStatus::Connected;
                self.you = Some(you);
                self.peers = peers.into_iter().map(|info| (info.id, RemotePeer::new(info))).collect();
                self.comments = comments.into_iter().map(|comment| (comment.id, comment)).collect();
            },
            Event::PeerJoined(info) => {
                self.peers.insert(info.id, RemotePeer::new(info));
            },
            Event::PeerLeft(id) => {
                self.peers.remove(&id);
            },
            Event::Cursor { from, cursor } => {
                if let Some(peer) = self.peers.get_mut(&from) {
                    peer.set_cursor(cursor);
                }
            },
            Event::Comment(comment) => {
                self.comments.insert(comment.id, comment);
            },
            Event::CommentDeleted(id) => {
                self.comments.remove(&id);
            },
            Event::MapShared { path, by, generation } => {
                // the snapshot may have come first
                let shared = self
                    .shared
                    .entry(path)
                    .or_insert_with(|| SharedMap::new(by, generation));

                if shared.generation != generation {
                    *shared = SharedMap::new(by, generation);
                }

                shared.by = by;
                if Some(by) == self.you {
                    shared.loaded_generation = Some(generation);
                }
            },
            Event::MapSnapshot {
                path,
                generation,
                bytes,
            } => self.receive_map(path, generation, bytes, codebase),
            Event::Edit { by, edit } => {
                if let Some(shared) = self
                    .shared
                    .get_mut(&edit.path)
                    .filter(|shared| shared.generation == edit.generation)
                {
                    shared.inbox.push((by, edit));
                }
            },
            Event::Rejected(reason) => self.end(format!("rejected: {reason}")),
            Event::Disconnected(reason) => self.end(reason),
        }
    }

    fn end(&mut self, reason: String) {
        self.status = LiveStatus::Ended(reason);
        self.peers.clear();
        self.comments.clear();
        self.shared.clear();
        self.skipped.clear();
        self.received.clear();
        self.client = None;
    }
}

impl Session {
    pub fn live(&self) -> Option<&LiveShare> { self.live.as_ref() }

    pub fn host_live(&mut self, port: u16, password: String, nick: String) -> Result<(), String> {
        let server = Server::spawn(ServerConfig {
            bind: SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)),
            password,
        })
        .map_err(|e| format!("could not host on port {port}: {e}"))?;

        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, server.local_addr().port())).to_string();
        let password = server.password().to_owned();
        self.start_live(addr, password, nick, Some(server))
    }

    pub fn join_live(&mut self, addr: String, password: String, nick: String) -> Result<(), String> {
        self.start_live(addr, password, nick, None)
    }

    fn start_live(
        &mut self, addr: String, password: String, nick: String, server: Option<Server>,
    ) -> Result<(), String> {
        let Some(environment) = self.state.environment.clone() else {
            return Err(String::from("open a codebase first"));
        };

        self.leave_live();

        let (sender, codebase) = mpsc::channel();
        std::thread::spawn(move || {
            let codebase = environment
                .fingerprint()
                .map(|hash| CodebaseId {
                    hash,
                    git_hint: environment.git_hint(),
                })
                .map_err(|e| format!("could not read the codebase: {e}"));

            let _ = sender.send(codebase);
        });

        self.live = Some(LiveShare {
            status: LiveStatus::Hashing,
            nick,
            you: None,
            peers: BTreeMap::new(),
            server,
            codebase: None,
            pending: Some(PendingJoin {
                addr,
                password,
                codebase,
            }),
            comments: BTreeMap::new(),
            shared: BTreeMap::new(),
            skipped: BTreeMap::new(),
            received: BTreeSet::new(),
            prepared: mpsc::channel(),
            client: None,
            last_poll: Instant::now(),
        });

        Ok(())
    }

    pub fn leave_live(&mut self) {
        // a server waits for its peers to hear the close, keep that off the frame
        if let Some(server) = self.live.take().and_then(|mut live| live.server.take()) {
            std::thread::spawn(move || drop(server));
        }

        self.drop_comment_tool();
    }

    pub fn comment_tool_available(&self) -> bool { self.live.as_ref().is_some_and(LiveShare::is_connected) }

    pub fn add_live_comment(&self, id: DocumentId, pos: [f32; 2], text: String) {
        let (Some(map), Some(document)) = (self.live_map_key(id), self.state.document(id)) else {
            return;
        };

        if let Some(client) = self.live.as_ref().and_then(|live| live.client.as_ref()) {
            client.send_comment(map, document.z, pos, text);
        }
    }

    pub fn delete_live_comment(&self, id: CommentId) {
        if let Some(client) = self.live.as_ref().and_then(|live| live.client.as_ref()) {
            client.delete_comment(id);
        }
    }

    fn drop_comment_tool(&mut self) {
        if self.state.tool == Tool::Comment && !self.comment_tool_available() {
            self.state.tool = Tool::Select;
        }
    }

    pub fn poll_live(&mut self) {
        self.drop_comment_tool();
        let codebase = self.codebase_dir().map(Path::to_path_buf);
        let Some(live) = self.live.as_mut() else {
            return;
        };

        if let Some(pending) = live.pending.as_ref() {
            match pending.codebase.try_recv() {
                Ok(Ok(codebase)) => {
                    let pending = live.pending.take().expect("checked above");
                    live.client = Some(Client::connect(
                        pending.addr,
                        pending.password,
                        live.nick.clone(),
                        codebase.clone(),
                    ));
                    live.codebase = Some(codebase);
                    live.status = LiveStatus::Connecting;
                },
                Ok(Err(e)) => {
                    live.pending = None;
                    live.end(e);
                },
                Err(mpsc::TryRecvError::Empty) => {},
                Err(mpsc::TryRecvError::Disconnected) => {
                    live.pending = None;
                    live.end(String::from("could not read the codebase"));
                },
            }
        }

        let events = live
            .client
            .as_ref()
            .map(|client| client.poll().collect::<Vec<_>>())
            .unwrap_or_default();

        for event in events {
            live.apply(event, codebase.as_deref());
        }

        let prepared = live.prepared.1.try_iter().collect::<Vec<_>>();

        let now = Instant::now();
        let elapsed = now.duration_since(live.last_poll).as_secs_f32();
        live.last_poll = now;
        for peer in live.peers.values_mut() {
            peer.follow(elapsed);
        }

        let waiting = self
            .live
            .as_mut()
            .map(|live| std::mem::take(&mut live.skipped))
            .unwrap_or_default();
        for received in waiting.into_values() {
            self.open_shared_map(received);
        }

        for prepared in prepared {
            match prepared {
                Prepared::Upload { path, bytes } => {
                    if let Some(client) = self.live.as_ref().and_then(|live| live.client.as_ref()) {
                        client.share_map(path, bytes);
                    }
                },
                Prepared::Snapshot(received) => self.open_shared_map(received),
            }
        }

        self.apply_live_edits();
        self.send_live_edits();
    }

    fn shared_document(&self, path: &str) -> Option<DocumentId> {
        self.state.document_for_path(&self.codebase_dir()?.join(path))
    }

    fn apply_live_edits(&mut self) {
        let Some(live) = self.live.as_mut() else {
            return;
        };

        let you = live.you;
        let mut incoming = Vec::new();
        for (path, shared) in live.shared.iter_mut().filter(|(_, shared)| shared.is_ready()) {
            for (by, edit) in shared.inbox.drain(..) {
                incoming.push((path.clone(), by, edit));
            }
        }

        for (path, by, edit) in incoming {
            let tiles = match editor::patch::decode(&edit.patch, edit.coords.len()) {
                Ok(tiles) => tiles,
                Err(e) => {
                    log::warn!("{path}: dropping an edit that does not parse: {e}");
                    continue;
                },
            };

            let coords = edit.coords.iter().map(|&[x, y, z]| Coord::new(x, y, z));
            let Some(shared) = self.live.as_mut().and_then(|live| live.shared.get_mut(&path)) else {
                continue;
            };

            // TODO: we should probably not do this in the first place, this is literally wasted push pop
            if Some(by) == you {
                for coord in coords {
                    if let Some(count) = shared.in_flight.get_mut(&coord) {
                        *count -= 1;
                        if *count == 0 {
                            shared.in_flight.remove(&coord);
                        }
                    }
                }

                continue;
            }

            let tiles = coords
                .zip(tiles)
                .filter(|(coord, _)| !shared.in_flight.contains_key(coord))
                .collect::<Vec<_>>();

            let Some(id) = self.shared_document(&path).filter(|_| !tiles.is_empty()) else {
                continue;
            };

            let Some(document) = self.state.document_mut(id) else {
                continue;
            };

            let affected = document.apply_remote(tiles);
            self.update_document_instances(id, &affected);
            if let Some(cache) = self.caches.get_mut(&id) {
                cache.map_revision = cache.map_revision.wrapping_add(1);
            }
        }
    }

    fn send_live_edits(&mut self) {
        let Some(live) = self.live.as_ref() else {
            return;
        };

        let ready = live
            .shared
            .iter()
            .filter(|(_, shared)| shared.is_ready())
            .map(|(path, shared)| (path.clone(), shared.generation))
            .collect::<Vec<_>>();

        for (path, generation) in ready {
            let Some(id) = self.shared_document(&path) else {
                continue;
            };

            let Some(journal) = self.state.document_mut(id).and_then(|document| document.take_journal()) else {
                continue;
            };

            if journal.reshaped {
                self.share_live_document(id);
                continue;
            }

            let Some((coords, patch)) = self
                .state
                .document(id)
                .and_then(|document| editor::patch::encode(&document.map, journal.coords))
            else {
                continue;
            };

            let Some(live) = self.live.as_mut() else {
                return;
            };

            if let Some(shared) = live.shared.get_mut(&path) {
                for coord in &coords {
                    *shared.in_flight.entry(*coord).or_insert(0) += 1;
                }
            }

            if let Some(client) = live.client.as_ref() {
                client.send_edit(MapEdit {
                    path,
                    generation,
                    coords: coords.iter().map(|coord| [coord.x, coord.y, coord.z]).collect(),
                    patch,
                });
            }
        }
    }

    pub fn can_share_live_map(&self) -> bool {
        self.comment_tool_available() && self.state.active().and_then(|id| self.live_map_path(id)).is_some()
    }

    pub fn share_live_map(&mut self) {
        if let Some(id) = self.state.active() {
            self.share_live_document(id);
        }
    }

    fn share_live_document(&mut self, id: DocumentId) {
        let Some(path) = self.live_map_path(id) else {
            return;
        };

        let Some(document) = self.state.document_mut(id) else {
            return;
        };

        // edits so far are part of the snapshot, the journal only carries what comes after it
        document.start_journal();
        document.take_journal();

        let Some(live) = self.live.as_mut() else {
            return;
        };

        // hold back new edits until the server numbers this share
        if let Some(shared) = live.shared.get_mut(&path) {
            shared.loaded_generation = None;
        }

        let map = document.map.clone();
        let prepared = live.prepared.0.clone();
        std::thread::spawn(move || {
            let bytes = dmm::writer::write(&map).into_bytes();
            let _ = prepared.send(Prepared::Upload { path, bytes });
        });
    }

    fn open_shared_map(&mut self, received: ReceivedMap) {
        let Some(live) = self.live.as_mut() else {
            return;
        };

        let open = self.state.document_for_path(&received.file);
        // a local copy with unsaved work waits until it is saved or closed, our copy of the session's map does not
        if open.is_some_and(|id| {
            !live.received.contains(&received.path)
                && self.state.document(id).is_some_and(|document| document.is_dirty())
        }) {
            live.skipped.insert(received.path.clone(), received);
            return;
        }

        let ReceivedMap {
            path,
            generation,
            file,
            map,
            errors,
            modified,
        } = received;

        let shared = live
            .shared
            .entry(path.clone())
            .or_insert_with(|| SharedMap::new(PeerId(0), generation));

        if generation < shared.generation {
            return;
        }

        shared.generation = generation;
        shared.loaded_generation = Some(generation);
        shared.in_flight.clear();
        live.received.insert(path);

        let repo = self.git_enabled.then(|| editor::git::discover(&file)).flatten();
        let loaded = LoadedMap {
            path: file.clone(),
            map,
            z: 1,
            errors,
            repo,
            conflict: None,
        };

        match open {
            Some(id) => {
                self.reload_map(id, loaded);
            },
            None => self.apply_map(loaded),
        }

        if let Some(document) = self
            .state
            .document_for_path(&file)
            .and_then(|id| self.state.document_mut(id))
        {
            document.start_journal();
            if modified {
                document.mark_unsaved();
            }
        }
    }

    fn live_map_path(&self, id: DocumentId) -> Option<String> {
        let path = self.state.document(id)?.path.as_deref()?;
        let relative = path.strip_prefix(self.codebase_dir()?).ok()?;
        let path = relative
            .components()
            .map(|part| part.as_os_str().to_str())
            .collect::<Option<Vec<_>>>()?
            .join("/");

        net::is_map_path(&path).then_some(path)
    }

    pub fn live_cursor(&self, cursor: Option<Cursor>) {
        if let Some(client) = self.live.as_ref().and_then(|live| live.client.as_ref()) {
            client.send_cursor(cursor);
        }
    }

    pub fn live_map_key(&self, id: DocumentId) -> Option<String> {
        self.live.as_ref()?;
        let path = self.state.document(id)?.path.as_deref()?;

        editor::codebase_key(self.codebase_dir()?, path)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };

    use editor::Environment;
    use objtree::ObjectTree;

    use super::*;

    fn session() -> Session {
        let mut session = Session::new();
        session.state.environment = Some(Arc::new(Environment::new("game.dme", ObjectTree::new())));

        session
    }

    fn poll_until(sessions: &mut [&mut Session], mut done: impl FnMut(&[&mut Session]) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done(sessions) {
            assert!(Instant::now() < deadline, "timed out");
            for session in sessions.iter_mut() {
                session.poll_live();
            }

            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn connected(session: &Session) -> bool {
        session
            .live()
            .is_some_and(|live| matches!(live.status, LiveStatus::Connected))
    }

    #[test]
    fn a_joined_peer_sees_the_hosts_cursor() {
        let mut host = session();
        host.host_live(0, String::from("hunter2"), String::from("host"))
            .unwrap();
        poll_until(&mut [&mut host], |sessions| connected(sessions[0]));

        let (addr, password) = host.live().and_then(LiveShare::host).unwrap();
        let (addr, password) = (format!("127.0.0.1:{}", addr.port()), password.to_owned());
        let mut guest = session();
        guest.join_live(addr, password, String::from("guest")).unwrap();
        poll_until(&mut [&mut host, &mut guest], |sessions| {
            connected(sessions[1]) && sessions[0].live().is_some_and(|live| live.peers.len() == 1)
        });

        let peer = guest.live().unwrap().peers.values().next().unwrap();
        assert_eq!(peer.info.nick, "host");
        assert!(guest.live().unwrap().same_codebase(&peer.info));

        let cursor = Cursor {
            map: String::from("_maps/a.dmm"),
            z: 1,
            pos: [32.0, 64.0],
        };
        host.live_cursor(Some(cursor.clone()));
        poll_until(&mut [&mut host, &mut guest], |sessions| {
            sessions[1]
                .live()
                .is_some_and(|live| live.peers.values().any(|peer| peer.cursor.as_ref() == Some(&cursor)))
        });

        guest.leave_live();
        poll_until(&mut [&mut host], |sessions| {
            sessions[0].live().is_some_and(|live| live.peers.is_empty())
        });
    }

    #[test]
    fn a_wrong_password_ends_the_session_with_the_reason() {
        let mut host = session();
        host.host_live(0, String::from("hunter2"), String::from("host"))
            .unwrap();
        let port = host.live().and_then(LiveShare::host).unwrap().0.port();

        let mut guest = session();
        guest
            .join_live(format!("127.0.0.1:{port}"), String::from("nope"), String::from("guest"))
            .unwrap();
        poll_until(&mut [&mut guest], |sessions| {
            sessions[0].live().is_some_and(
                |live| matches!(&live.status, LiveStatus::Ended(reason) if reason.contains("wrong password")),
            )
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
                hash: [0; 32],
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

        host.host_live(0, String::from("hunter2"), String::from("host"))
            .unwrap();
        poll_until(&mut [&mut host], |sessions| connected(sessions[0]));
        host.set_tool(Tool::Comment);
        assert_eq!(host.tool(), Tool::Comment);

        host.leave_live();
        assert_eq!(host.tool(), Tool::Select);
    }

    fn codebase_with_map(name: &str, rows: &str) -> (PathBuf, Session) {
        let dir = std::env::temp_dir().join(format!("rmd-live-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("_maps")).unwrap();
        std::fs::write(dir.join("game.dme"), "").unwrap();
        std::fs::write(
            dir.join("_maps/a.dmm"),
            format!("\"a\" = (/turf,/area)\n\n(1,1,1) = {{\"\n{rows}\n\"}}\n"),
        )
        .unwrap();

        let mut session = Session::new();
        session.state.environment = Some(Arc::new(Environment::new(dir.join("game.dme"), ObjectTree::new())));

        (dir, session)
    }

    fn open_local(session: &mut Session, file: PathBuf) -> DocumentId {
        let text = std::fs::read_to_string(&file).unwrap();
        let (map, errors) = dmm::parser::parse(&text);
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

    #[test]
    fn a_shared_map_waits_for_unsaved_work_then_replaces_the_local_copy() {
        let (host_dir, mut host) = codebase_with_map("share-host", "aa");
        let (guest_dir, mut guest) = codebase_with_map("share-guest", "a");
        open_local(&mut host, host_dir.join("_maps/a.dmm"));
        let local = open_local(&mut guest, guest_dir.join("_maps/a.dmm"));
        guest.state.document_mut(local).unwrap().mark_unsaved();

        host.host_live(0, String::from("hunter2"), String::from("host"))
            .unwrap();
        poll_until(&mut [&mut host], |sessions| connected(sessions[0]));
        assert!(host.can_share_live_map());
        host.share_live_map();
        poll_until(&mut [&mut host], |sessions| {
            sessions[0]
                .live()
                .is_some_and(|live| live.shared.contains_key("_maps/a.dmm"))
        });

        let port = host.live().and_then(LiveShare::host).unwrap().0.port();
        guest
            .join_live(
                format!("127.0.0.1:{port}"),
                String::from("hunter2"),
                String::from("guest"),
            )
            .unwrap();
        poll_until(&mut [&mut host, &mut guest], |sessions| {
            sessions[1]
                .live()
                .is_some_and(|live| live.skipped.contains_key("_maps/a.dmm"))
        });
        assert_eq!(guest.state.document(local).unwrap().map.size.x, 1);

        guest.close_map(local);
        let file = guest_dir.join("_maps/a.dmm");
        poll_until(&mut [&mut host, &mut guest], |sessions| {
            sessions[1].state.document_for_path(&file).is_some()
        });

        let received = guest
            .state
            .document(guest.state.document_for_path(&file).unwrap())
            .unwrap();
        assert_eq!(received.map.size.x, 2);
        assert!(received.is_dirty());
        assert!(guest.live().unwrap().skipped.is_empty());

        let _ = std::fs::remove_dir_all(&host_dir);
        let _ = std::fs::remove_dir_all(&guest_dir);
    }

    fn paint(session: &mut Session, file: &Path, coord: Coord, path: &str) {
        let id = session.state.document_for_path(file).unwrap();
        let document = session.state.document_mut(id).unwrap();
        let mut tile = document.map.tile_at(coord).cloned().unwrap();
        tile.insert(0, dmm::Prefab::new(core::path::TreePath::parse(path)));

        let placed = tile.into_iter().map(|prefab| document.instantiate(prefab)).collect();
        let mut edit = editor::command::Edit::new("paint");
        edit.change(document, coord, placed);
        assert!(document.apply(edit));
    }

    fn top(session: &Session, file: &Path, coord: Coord) -> Option<String> {
        let document = session.state.document(session.state.document_for_path(file)?)?;

        Some(document.map.tile_at(coord)?.first()?.path.to_string())
    }

    fn settled(sessions: &[&mut Session]) -> bool {
        sessions.iter().all(|session| {
            session.live().is_some_and(|live| {
                live.shared
                    .values()
                    .all(|shared| shared.in_flight.is_empty() && shared.inbox.is_empty())
            })
        })
    }

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

        host.host_live(0, String::from("hunter2"), String::from("host"))
            .unwrap();
        poll_until(&mut [&mut host], |sessions| connected(sessions[0]));
        host.share_live_map();
        let port = host.live().and_then(LiveShare::host).unwrap().0.port();
        let join = |session: &mut Session, nick: &str| {
            session
                .join_live(format!("127.0.0.1:{port}"), String::from("hunter2"), String::from(nick))
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
            document.is_some_and(|document| document.map.size.x == 3) && settled(sessions)
        });

        let corner = Coord::new(3, 1, 1);
        paint(&mut host, &host_file, corner, "/obj/after_resize");
        poll_until(&mut [&mut host, &mut guest], |sessions| {
            top(sessions[1], &guest_file, corner).as_deref() == Some("/obj/after_resize")
        });

        for dir in [host_dir, guest_dir, late_dir] {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn live_share_needs_a_codebase() {
        assert!(
            Session::new()
                .join_live(String::from("127.0.0.1:1"), String::from("pw"), String::from("me"))
                .is_err()
        );
    }
}
