use std::{
    collections::{BTreeMap, HashMap},
    fs,
    mem,
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::Instant,
};

use dmm::{Coord, Size, error::MapError, parser, writer};
use editor::{
    Environment,
    document::{DocumentId, MapDocument},
    git,
    patch,
    tool::Tool,
};
use net::{
    Client,
    CodebaseId,
    Comment,
    CommentId,
    Cursor,
    Direction,
    Event,
    GenerationId,
    MapEdit,
    PeerId,
    PeerInfo,
    SeqId,
    Server,
    ServerConfig,
};

use super::Session;
use crate::loader::LoadedMap;

// how fast a remote cursor closes the gap to its latest position, per second
const CURSOR_SMOOTHING: f32 = 20.0;

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum CoopStatus {
    Connecting,
    Connected,
    Ended(String),
    CodebaseMismatch { expected: CodebaseId },
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

pub(crate) struct Coop {
    pub status: CoopStatus,
    pub nick: String,
    pub you: Option<PeerId>,
    pub peers: BTreeMap<PeerId, RemotePeer>,
    pub comments: BTreeMap<CommentId, Comment>,
    pub shared_maps: BTreeMap<String, SharedMap>,
    prepared: (mpsc::Sender<Prepared>, mpsc::Receiver<Prepared>),
    server: Option<Server>,
    codebase: CodebaseId,
    connection: CoopConnection,
    environment_root: PathBuf,
    paused: bool,
    needs_cleanup: bool,
    stopping_server: Option<mpsc::Receiver<()>>,
    client: Option<Client>,
    last_poll: Instant,
}

pub(crate) enum SharedState {
    // someone is uploading it, `before` comes back if they give up
    Incoming {
        by: PeerId,
        len: u64,
        before: Box<SharedState>,
    },
    // our own share, until the server numbers it
    Sending {
        done: u64,
        total: u64,
    },
    Receiving {
        done: u64,
        total: u64,
    },
    // parsing the snapshot of this generation
    Loading(GenerationId),
    Ready,
    Closed,
    // arrived, but an unsaved local copy is in the way
    Waiting(Box<ReceivedMap>),
}

impl SharedState {
    const RECEIVING: Self = Self::Receiving { done: 0, total: 0 };

    pub fn is_arriving(&self) -> bool {
        matches!(self, Self::Incoming { .. } | Self::Receiving { .. } | Self::Loading(_))
    }

    // a newer snapshot replaces one still parsing
    fn accepts(&self, generation: GenerationId) -> bool {
        match *self {
            Self::Incoming { .. } | Self::Receiving { .. } => true,
            Self::Loading(loading) => loading < generation,
            _ => false,
        }
    }
}

pub(crate) struct SharedMap {
    pub by: PeerId,
    pub state: SharedState,
    // none until the server numbers the share
    generation: Option<GenerationId>,
    // an empty read only document standing in for the map until it arrives
    pending_document: Option<DocumentId>,
    // our open copy is the session's, so a new snapshot replaces it even when unsaved
    received: bool,
    // edits from the server by seq, applied in order once our copy of the map is loaded
    inbox: BTreeMap<SeqId, (PeerId, MapEdit)>,
    next_seq: SeqId,
    // tiles we edited whose edits the server hasn't sent back yet, counted per tile
    in_flight: HashMap<Coord, u32>,
}

impl SharedMap {
    fn new(by: PeerId, state: SharedState) -> Self {
        Self {
            by,
            state,
            generation: None,
            pending_document: None,
            received: false,
            inbox: BTreeMap::new(),
            next_seq: SeqId(0),
            in_flight: HashMap::new(),
        }
    }

    fn is_ready(&self) -> bool { matches!(self.state, SharedState::Ready) }

    // an upload that never got numbered leaves nothing behind
    fn is_abandoned(&self) -> bool {
        self.generation.is_none() && matches!(self.state, SharedState::Closed) && self.pending_document.is_none()
    }

    fn restart(&mut self, state: SharedState) {
        self.state = state;
        self.forget_edits();
    }

    // the old generation's edits don't apply to a new one
    fn renumber(&mut self, generation: GenerationId) {
        if self.generation != Some(generation) {
            self.generation = Some(generation);
            self.forget_edits();
        }
    }

    fn forget_edits(&mut self) {
        self.inbox.clear();
        self.next_seq = SeqId(0);
        self.in_flight.clear();
    }
}

enum Prepared {
    Upload {
        path: String,
        base: Option<(GenerationId, SeqId)>,
        bytes: Vec<u8>,
    },
    Snapshot(ReceivedMap),
    Unreadable {
        path: String,
        generation: GenerationId,
    },
}

pub(crate) struct ReceivedMap {
    path: String,
    generation: GenerationId,
    file: PathBuf,
    map: dmm::Map,
    errors: Vec<MapError>,
    is_modified: bool,
}

#[derive(Clone)]
enum CoopConnection {
    Host { port: u16, password: String },
    Join { addr: String, password: String },
}

impl Coop {
    pub fn is_hosting(&self) -> bool { self.server.is_some() }

    pub fn is_connected(&self) -> bool { matches!(self.status, CoopStatus::Connected) }

    pub fn is_paused(&self) -> bool { self.paused }

    pub fn was_hosting(&self) -> bool { matches!(self.connection, CoopConnection::Host { .. }) }

    pub fn can_retry(&self) -> bool { self.stopping_server.is_none() && !self.paused }

    pub fn local_codebase(&self) -> &CodebaseId { &self.codebase }

    fn can_collaborate(&self) -> bool { self.is_connected() && !self.paused }

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

    pub fn same_codebase(&self, peer: &PeerInfo) -> bool { self.codebase.hash == peer.codebase.hash }

    pub fn git_hint(&self) -> Option<&str> { self.codebase.git_hint.as_deref() }

    fn mismatch(&mut self, expected: CodebaseId) {
        self.end(String::from("different codebase"));
        self.status = CoopStatus::CodebaseMismatch { expected };
    }

    fn receive_map(&mut self, path: String, generation: GenerationId, bytes: Vec<u8>, codebase: Option<&Path>) {
        if !net::is_map_path(&path) {
            log::warn!("ignoring a shared map outside the codebase: {path}");
            return;
        }

        let Some(file) = codebase.map(|codebase| codebase.join(&path)) else {
            return;
        };

        let Some(shared_map) = self
            .shared_maps
            .get_mut(&path)
            .filter(|shared_map| shared_map.state.accepts(generation))
        else {
            log::debug!("{path}: dropping a snapshot nobody is waiting for");
            return;
        };

        shared_map.state = SharedState::Loading(generation);

        let prepared = self.prepared.0.clone();
        thread::spawn(move || {
            let is_modified = fs::read(&file).map_or(true, |disk| disk != bytes);
            let text = match String::from_utf8(bytes) {
                Ok(text) => text,
                Err(e) => {
                    log::warn!("{path}: the shared map is not text: {e}");
                    let _ = prepared.send(Prepared::Unreadable { path, generation });
                    return;
                },
            };

            let (map, errors) = parser::parse(&text);
            let _ = prepared.send(Prepared::Snapshot(ReceivedMap {
                path,
                generation,
                file,
                map,
                errors,
                is_modified,
            }));
        });
    }

    fn apply(&mut self, event: Event, codebase: Option<&Path>) {
        if matches!(self.status, CoopStatus::Ended(_) | CoopStatus::CodebaseMismatch { .. }) {
            return;
        }

        match event {
            Event::Connected {
                you, peers, comments, ..
            } => {
                self.status = CoopStatus::Connected;
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
            Event::MapIncoming { path, by, len } => {
                let shared_map = self
                    .shared_maps
                    .entry(path)
                    .or_insert_with(|| SharedMap::new(by, SharedState::Closed));

                let before = match mem::replace(&mut shared_map.state, SharedState::Closed) {
                    SharedState::Incoming { before, .. } => before,
                    state => Box::new(state),
                };

                shared_map.state = SharedState::Incoming { by, len, before };
            },
            Event::MapCancelled { path, by } => {
                let Some(shared_map) = self.shared_maps.get_mut(&path) else {
                    return;
                };

                shared_map.state = match mem::replace(&mut shared_map.state, SharedState::Closed) {
                    SharedState::Incoming {
                        by: uploader, before, ..
                    } if uploader == by => *before,
                    state => state,
                };

                if shared_map.is_abandoned() {
                    self.shared_maps.remove(&path);
                }
            },
            Event::MapShared { path, by, generation } => {
                let you = self.you;
                let shared_map = self
                    .shared_maps
                    .entry(path)
                    .or_insert_with(|| SharedMap::new(by, SharedState::RECEIVING));

                let is_known = shared_map.generation == Some(generation);
                shared_map.by = by;
                shared_map.renumber(generation);
                if Some(by) == you {
                    // we share this map, skip all sharing stuff
                    shared_map.state = SharedState::Ready;
                    shared_map.received = true;
                } else if !is_known
                    && matches!(
                        shared_map.state,
                        SharedState::Incoming { .. } | SharedState::Ready | SharedState::Waiting(_)
                    )
                {
                    // a map closed while it was uploading stays closed, and the snapshot may have come first
                    shared_map.state = SharedState::RECEIVING;
                }
            },
            Event::Progress {
                path,
                direction: Direction::Sending,
                done,
                total,
            } => {
                if let Some(shared_map) = self.shared_maps.get_mut(&path)
                    && matches!(shared_map.state, SharedState::Sending { .. })
                {
                    shared_map.state = SharedState::Sending { done, total };
                }
            },
            Event::Progress {
                path,
                direction: Direction::Receiving,
                done,
                total,
            } => {
                // special case handling:L the download can beat its announcement
                let shared_map = self
                    .shared_maps
                    .entry(path)
                    .or_insert_with(|| SharedMap::new(PeerId(0), SharedState::RECEIVING));

                if matches!(
                    shared_map.state,
                    SharedState::Incoming { .. } | SharedState::Receiving { .. }
                ) {
                    shared_map.state = SharedState::Receiving { done, total };
                }
            },
            Event::MapSnapshot {
                path,
                generation,
                bytes,
            } => self.receive_map(path, generation, bytes, codebase),
            Event::TransferFailed {
                path,
                direction,
                reason,
            } => {
                log::warn!("{path}: the transfer failed: {reason}");
                let Some(shared_map) = self.shared_maps.get_mut(&path) else {
                    return;
                };

                let is_sending = matches!(shared_map.state, SharedState::Sending { .. });
                let is_receiving = matches!(shared_map.state, SharedState::Receiving { .. });
                match direction {
                    // our copy no longer matches what peers have, so take the server's again
                    Direction::Sending if is_sending && shared_map.generation.is_some() => {
                        shared_map.restart(SharedState::Closed)
                    },
                    Direction::Sending if is_sending => {
                        self.shared_maps.remove(&path);
                    },
                    Direction::Receiving if is_receiving => shared_map.restart(SharedState::Closed),
                    _ => {},
                }
            },
            Event::Edit { by, seq, edit } => {
                if let Some(shared_map) = self.shared_maps.get_mut(&edit.path).filter(|shared_map| {
                    shared_map.generation == Some(edit.generation)
                        && !matches!(shared_map.state, SharedState::Closed)
                        && seq >= shared_map.next_seq
                }) {
                    shared_map.inbox.insert(seq, (by, edit));
                }
            },
            Event::CodebaseMismatch { expected } => self.mismatch(expected),
            Event::Rejected(reason) => self.end(format!("rejected: {reason}")),
            Event::Disconnected(reason) => self.end(reason),
        }
    }

    // shared maps stay until the session closes their pending documents
    fn end(&mut self, reason: String) {
        self.status = CoopStatus::Ended(reason);
        self.peers.clear();
        self.comments.clear();
        self.client = None;
        self.paused = false;
        self.needs_cleanup = true;
        if let Some(server) = self.server.take() {
            let (done, stopping) = mpsc::channel();
            self.stopping_server = Some(stopping);
            thread::spawn(move || {
                drop(server);
                let _ = done.send(());
            });
        }
    }
}

impl Session {
    pub fn coop(&self) -> Option<&Coop> { self.coop.as_ref() }

    fn loaded_codebase_id(&self) -> Result<CodebaseId, String> {
        let environment = self.state.environment.as_ref().ok_or("open a codebase first")?;
        Ok(CodebaseId {
            hash: environment.fingerprint().map_err(|error| error.to_string())?,
            git_hint: environment.git_hint(),
        })
    }

    pub fn host_coop(&mut self, port: u16, password: String, nick: String) -> Result<(), String> {
        let codebase = self.loaded_codebase_id()?;
        let server = Server::spawn(ServerConfig {
            bind: SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)),
            password,
            codebase: Some(codebase.clone()),
        })
        .map_err(|e| format!("could not host on port {port}: {e}"))?;

        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, server.local_addr().port())).to_string();
        let password = server.password().to_owned();
        let connection = CoopConnection::Host {
            port: server.local_addr().port(),
            password: password.clone(),
        };
        self.start_coop(addr, password, nick, codebase, connection, Some(server))
    }

    pub fn join_coop(&mut self, addr: String, password: String, nick: String) -> Result<(), String> {
        let codebase = self.loaded_codebase_id()?;
        let connection = CoopConnection::Join {
            addr: addr.clone(),
            password: password.clone(),
        };
        self.start_coop(addr, password, nick, codebase, connection, None)
    }

    fn start_coop(
        &mut self, addr: String, password: String, nick: String, codebase: CodebaseId, connection: CoopConnection,
        server: Option<Server>,
    ) -> Result<(), String> {
        let environment_root = self.environment_path().ok_or("open a codebase first")?.to_path_buf();
        self.leave_coop();
        let client = Client::connect(addr, password, nick.clone(), codebase.clone());
        self.coop = Some(Coop {
            status: CoopStatus::Connecting,
            nick,
            you: None,
            peers: BTreeMap::new(),
            server,
            codebase,
            connection,
            environment_root,
            paused: false,
            needs_cleanup: false,
            stopping_server: None,
            comments: BTreeMap::new(),
            shared_maps: BTreeMap::new(),
            prepared: mpsc::channel(),
            client: Some(client),
            last_poll: Instant::now(),
        });

        Ok(())
    }

    pub fn retry_coop(&mut self) -> Result<(), String> {
        let coop = self.coop.as_ref().ok_or("no co-op attempt to retry")?;
        if !coop.can_retry() {
            return Err(String::from("wait for the previous co-op attempt to finish"));
        }

        let nick = coop.nick.clone();
        match coop.connection.clone() {
            CoopConnection::Host { port, password } => self.host_coop(port, password, nick),
            CoopConnection::Join { addr, password } => self.join_coop(addr, password, nick),
        }
    }

    pub fn begin_coop_reload(&mut self) {
        if let Some(coop) = self.coop.as_mut() {
            coop.paused = true;
        }

        self.lock_coop_documents();
    }

    pub fn finish_coop_reload(&mut self) {
        if let Some(coop) = self.coop.as_mut() {
            coop.paused = false;
        }

        self.lock_coop_documents();
    }

    pub(super) fn check_coop_codebase(&mut self, environment: &Environment) {
        if let Some(coop) = self.coop.as_mut() {
            let local = environment.fingerprint().map(|hash| CodebaseId {
                hash,
                git_hint: environment.git_hint(),
            });
            if matches!(coop.status, CoopStatus::Connecting | CoopStatus::Connected) {
                match &local {
                    Ok(local) if local.hash != coop.codebase.hash => coop.mismatch(coop.codebase.clone()),
                    Ok(_) if environment.root != coop.environment_root => {
                        coop.end(String::from("codebase switched; co-op disconnected"));
                    },
                    Err(error) => coop.end(format!("cannot verify the loaded codebase: {error}")),
                    _ => {},
                }
            }

            if let Ok(local) = local {
                coop.codebase = local;
            }
        }

        self.finish_coop_reload();
        self.clean_ended_coop();
    }

    fn clean_ended_coop(&mut self) {
        let Some(coop) = self.coop.as_mut().filter(|coop| coop.needs_cleanup) else {
            return;
        };

        coop.needs_cleanup = false;
        let pending = mem::take(&mut coop.shared_maps)
            .into_values()
            .filter_map(|map| map.pending_document)
            .collect::<Vec<_>>();
        coop.prepared = mpsc::channel();

        for id in pending {
            self.close_map(id);
        }

        self.lock_coop_documents();
    }

    pub fn leave_coop(&mut self) {
        let Some(mut coop) = self.coop.take() else {
            return;
        };

        // a server waits for its peers to hear the close, keep that off the frame
        if let Some(server) = coop.server.take() {
            thread::spawn(move || drop(server));
        }

        for id in coop
            .shared_maps
            .into_values()
            .filter_map(|shared_map| shared_map.pending_document)
        {
            self.close_map(id);
        }

        self.lock_coop_documents();
    }

    pub fn comment_tool_available(&self) -> bool { self.coop.as_ref().is_some_and(Coop::can_collaborate) }

    pub fn add_coop_comment(&self, id: DocumentId, pos: [f32; 2], text: String) {
        let (Some(map), Some(document)) = (self.coop_shared_map_path(id), self.state.document(id)) else {
            return;
        };

        if let Some(client) = self
            .coop
            .as_ref()
            .filter(|coop| coop.can_collaborate())
            .and_then(|coop| coop.client.as_ref())
        {
            client.send_comment(map, document.z, pos, text);
        }
    }

    pub fn delete_coop_comment(&self, id: CommentId) {
        if let Some(client) = self
            .coop
            .as_ref()
            .filter(|coop| coop.can_collaborate())
            .and_then(|coop| coop.client.as_ref())
        {
            client.delete_comment(id);
        }
    }

    pub fn poll_coop(&mut self) {
        if self.state.tool == Tool::Comment && !self.comment_tool_available() {
            self.state.tool = Tool::Select;
        }

        let codebase = self.codebase_dir().map(Path::to_path_buf);
        let Some(coop) = self.coop.as_mut() else {
            return;
        };

        if coop
            .stopping_server
            .as_ref()
            .is_some_and(|stopping| !matches!(stopping.try_recv(), Err(mpsc::TryRecvError::Empty)))
        {
            coop.stopping_server = None;
        }

        if coop.paused {
            return;
        }

        let events = coop
            .client
            .as_ref()
            .map(|client| client.poll().collect::<Vec<_>>())
            .unwrap_or_default();

        for event in events {
            coop.apply(event, codebase.as_deref());
        }

        if !coop.is_connected() {
            self.clean_ended_coop();
            return;
        }

        let prepared = coop.prepared.1.try_iter().collect::<Vec<_>>();

        let now = Instant::now();
        let elapsed = now.duration_since(coop.last_poll).as_secs_f32();
        coop.last_poll = now;
        for peer in coop.peers.values_mut() {
            peer.follow(elapsed);
        }

        let mut waiting = Vec::new();
        for shared_map in coop.shared_maps.values_mut() {
            let SharedState::Waiting(received) = &shared_map.state else {
                continue;
            };

            let loading = SharedState::Loading(received.generation);
            if let SharedState::Waiting(received) = mem::replace(&mut shared_map.state, loading) {
                waiting.push(*received);
            }
        }

        for received in waiting {
            self.open_shared_map(received);
        }

        for prepared in prepared {
            match prepared {
                Prepared::Upload { path, base, bytes } => {
                    if let Some(client) = self
                        .coop
                        .as_ref()
                        .filter(|coop| coop.can_collaborate())
                        .and_then(|coop| coop.client.as_ref())
                    {
                        client.share_map(path, base, bytes);
                    }
                },
                Prepared::Snapshot(received) => self.open_shared_map(received),
                Prepared::Unreadable { path, generation } => {
                    if let Some(shared_map) = self.coop.as_mut().and_then(|coop| coop.shared_maps.get_mut(&path))
                        && matches!(shared_map.state, SharedState::Loading(loading) if loading == generation)
                    {
                        shared_map.restart(SharedState::Closed);
                    }
                },
            }
        }

        self.follow_shared_documents();
        self.apply_coop_edits();
        self.send_coop_edits();
    }

    fn follow_shared_documents(&mut self) {
        self.follow_pending_documents();
        let Some(coop) = self.coop.as_ref() else {
            return;
        };

        let mut closed = Vec::new();
        let mut reopened = Vec::new();
        for (path, shared_map) in &coop.shared_maps {
            match (&shared_map.state, self.shared_document(path).is_some()) {
                (SharedState::Ready, false) => closed.push(path.clone()),
                (SharedState::Closed, true) => reopened.push(path.clone()),
                _ => {},
            }
        }

        if let Some(coop) = self.coop.as_mut() {
            for path in &closed {
                if let Some(shared_map) = coop.shared_maps.get_mut(path) {
                    shared_map.restart(SharedState::Closed);
                }
            }
        }

        for path in reopened {
            self.resync_coop_map(&path);
        }

        self.lock_coop_documents();
    }

    fn follow_pending_documents(&mut self) {
        let Some(codebase) = self.codebase_dir().map(Path::to_path_buf) else {
            return;
        };

        let Some(coop) = self.coop.as_mut() else {
            return;
        };

        let is_connected = coop.is_connected();
        let mut stale = Vec::new();
        let mut gone = Vec::new();
        let mut wanted = Vec::new();
        for (path, shared_map) in &mut coop.shared_maps {
            match shared_map.pending_document {
                // the user closed it before the map arrived
                Some(id) if self.state.document(id).is_none() => {
                    shared_map.pending_document = None;
                    if shared_map.state.is_arriving() {
                        shared_map.restart(SharedState::Closed);
                    }
                },
                // stale map
                Some(id) if !is_connected || !shared_map.state.is_arriving() => {
                    shared_map.pending_document = None;
                    stale.push(id);
                    if shared_map.is_abandoned() {
                        gone.push(path.clone());
                    }
                },
                None if is_connected && shared_map.state.is_arriving() => wanted.push(path.clone()),
                _ => {},
            }
        }

        if is_connected {
            for path in gone {
                coop.shared_maps.remove(&path);
            }
        } else {
            coop.shared_maps.clear();
        }

        for id in stale {
            self.close_map(id);
        }

        for path in wanted {
            // an open copy shows the progress itself
            if self.shared_document(&path).is_some() {
                continue;
            }

            let mut document = MapDocument::open(codebase.join(&path), dmm::Map::new(Size { x: 1, y: 1, z: 1 }), 1);
            document.set_read_only(true);
            let id = self.activate_document(document);
            if let Some(shared_map) = self.coop.as_mut().and_then(|coop| coop.shared_maps.get_mut(&path)) {
                shared_map.pending_document = Some(id);
            }
        }
    }

    fn lock_coop_documents(&mut self) {
        let locks = self
            .state
            .document_ids()
            .into_iter()
            .map(|id| {
                (
                    id,
                    self.coop_receiving(id).is_some()
                        || self.coop_out_of_date(id).is_some()
                        || self.coop.as_ref().is_some_and(|coop| coop.paused) && self.coop_shared_map(id).is_some(),
                )
            })
            .collect::<Vec<_>>();

        for (id, locked) in locks {
            if let Some(document) = self.state.document_mut(id) {
                document.set_read_only(locked);
            }
        }
    }

    fn coop_shared_map(&self, id: DocumentId) -> Option<&SharedMap> {
        self.coop.as_ref()?.shared_maps.get(&self.coop_map_path(id)?)
    }

    // the map is on its way and will replace this document
    pub fn coop_receiving(&self, id: DocumentId) -> Option<&SharedState> {
        let shared_map = self.coop_shared_map(id)?;
        let is_replaced = shared_map.pending_document == Some(id)
            || shared_map.received
            || self.state.document(id).is_some_and(|document| !document.is_dirty());

        (shared_map.state.is_arriving() && is_replaced).then_some(&shared_map.state)
    }

    // a newer snapshot from this peer waits on the document's unsaved changes
    pub fn coop_out_of_date(&self, id: DocumentId) -> Option<PeerId> {
        let shared_map = self.coop_shared_map(id)?;
        // saved or handed over, the next poll replaces it
        let is_waiting = matches!(shared_map.state, SharedState::Waiting(_))
            && !shared_map.received
            && self.state.document(id).is_some_and(MapDocument::is_dirty);

        is_waiting.then_some(shared_map.by)
    }

    // hands the unsaved copy over to the session, so the waiting snapshot replaces it
    pub fn discard_for_coop_map(&mut self, id: DocumentId) {
        let Some(path) = self.coop_map_path(id) else {
            return;
        };

        if let Some(shared_map) = self.coop.as_mut().and_then(|coop| coop.shared_maps.get_mut(&path))
            && matches!(shared_map.state, SharedState::Waiting(_))
        {
            shared_map.received = true;
        }
    }

    pub fn coop_transfer(&self, id: DocumentId) -> Option<&SharedState> {
        let shared_map = self.coop_shared_map(id)?;
        let is_sending = matches!(shared_map.state, SharedState::Sending { .. });

        (is_sending || shared_map.state.is_arriving()).then_some(&shared_map.state)
    }

    pub fn is_coop_shared_file(&self, file: &Path) -> bool {
        self.coop_file_path(file)
            .zip(self.coop.as_ref())
            .is_some_and(|(path, coop)| coop.shared_maps.contains_key(&path))
    }

    pub fn open_coop_file(&mut self, file: &Path) -> bool {
        let Some(path) = self.coop_file_path(file).filter(|_| self.is_coop_shared_file(file)) else {
            return false;
        };

        self.open_coop_map(&path)
    }

    pub fn open_coop_map(&mut self, path: &str) -> bool {
        if let Some(id) = self.shared_document(path) {
            return self.set_active_document(id);
        }

        if self
            .coop
            .as_ref()
            .and_then(|coop| coop.shared_maps.get(path))
            .is_some_and(|shared_map| matches!(shared_map.state, SharedState::Closed))
        {
            self.resync_coop_map(path);
            self.follow_pending_documents();
        }

        self.shared_document(path).is_some()
    }

    fn resync_coop_map(&mut self, path: &str) {
        let Some(coop) = self.coop.as_mut() else {
            return;
        };

        let Some(shared_map) = coop.shared_maps.get_mut(path) else {
            return;
        };

        shared_map.restart(SharedState::RECEIVING);
        // the snapshot replaces whatever copy is open, unsaved or not
        shared_map.received = true;
        if let Some(client) = coop.client.as_ref() {
            client.resync(path.to_owned());
        }
    }

    fn shared_document(&self, path: &str) -> Option<DocumentId> {
        self.state.document_for_path(&self.codebase_dir()?.join(path))
    }

    fn apply_coop_edits(&mut self) {
        let Some(coop) = self.coop.as_mut() else {
            return;
        };

        // we need ourselves here, this function is also used to keep us synced with the server
        // I WANT YOU TO KEEP IN SYNC!
        let you = coop.you;

        let mut incoming = Vec::new();
        for (path, shared_map) in coop
            .shared_maps
            .iter_mut()
            .filter(|(_, shared_map)| shared_map.is_ready())
        {
            while let Some((by, edit)) = shared_map.inbox.remove(&shared_map.next_seq) {
                shared_map.next_seq = shared_map.next_seq.next();
                incoming.push((path.clone(), by, edit));
            }
        }

        for (path, by, edit) in incoming {
            let tiles = match patch::decode(&edit.patch, edit.coords.len()) {
                Ok(tiles) => tiles,
                Err(e) => {
                    log::warn!("{path}: dropping an edit that does not parse: {e}");
                    continue;
                },
            };

            let coords = edit.coords.iter().map(|&[x, y, z]| Coord::new(x, y, z));
            let Some(shared_map) = self.coop.as_mut().and_then(|coop| coop.shared_maps.get_mut(&path)) else {
                continue;
            };

            // a tile we have in flight keeps our version until the server echoes it back,
            // anything else applies, including our own edits replayed after a resync
            let mut applied = Vec::new();
            for (coord, tile) in coords.zip(tiles) {
                let Some(count) = shared_map.in_flight.get_mut(&coord) else {
                    applied.push((coord, tile));
                    continue;
                };

                // bookkeeping
                if Some(by) == you {
                    *count -= 1;
                    if *count == 0 {
                        shared_map.in_flight.remove(&coord);
                    }
                }
            }

            let Some(id) = self.shared_document(&path).filter(|_| !applied.is_empty()) else {
                continue;
            };

            let Some(document) = self.state.document_mut(id) else {
                continue;
            };

            let affected = document.apply_remote(applied);
            self.update_document_instances(id, &affected);
            if let Some(cache) = self.caches.get_mut(&id) {
                cache.map_revision = cache.map_revision.wrapping_add(1);
            }
        }
    }

    fn send_coop_edits(&mut self) {
        let Some(coop) = self.coop.as_ref() else {
            return;
        };

        let ready = coop
            .shared_maps
            .iter()
            .filter(|(_, shared_map)| shared_map.is_ready())
            .filter_map(|(path, shared_map)| Some((path.clone(), shared_map.generation?)))
            .collect::<Vec<_>>();

        for (path, generation) in ready {
            let Some(id) = self.shared_document(&path) else {
                continue;
            };

            let Some(journal) = self.state.document_mut(id).and_then(|document| document.take_journal()) else {
                continue;
            };

            if journal.reshaped {
                self.share_coop_document(id);
                continue;
            }

            let Some((coords, patch)) = self
                .state
                .document(id)
                .and_then(|document| patch::encode(&document.map, journal.coords))
            else {
                continue;
            };

            let Some(coop) = self.coop.as_mut() else {
                return;
            };

            if let Some(shared_map) = coop.shared_maps.get_mut(&path) {
                for coord in &coords {
                    *shared_map.in_flight.entry(*coord).or_insert(0) += 1;
                }
            }

            if let Some(client) = coop.client.as_ref() {
                client.send_edit(MapEdit {
                    path,
                    generation,
                    coords: coords.iter().map(|coord| [coord.x, coord.y, coord.z]).collect(),
                    patch,
                });
            }
        }
    }

    pub fn can_share_coop_map(&self) -> bool {
        self.comment_tool_available() && self.state.active().and_then(|id| self.coop_map_path(id)).is_some()
    }

    pub fn share_coop_file(&mut self, file: &Path) -> bool {
        let Some(id) = self
            .state
            .document_for_path(file)
            .filter(|_| self.comment_tool_available())
        else {
            return false;
        };

        self.set_active_document(id);
        self.share_coop_document(id);

        true
    }

    pub fn share_coop_map(&mut self) {
        if let Some(id) = self.state.active() {
            self.share_coop_document(id);
        }
    }

    fn share_coop_document(&mut self, id: DocumentId) {
        if !self.coop.as_ref().is_some_and(Coop::can_collaborate) {
            return;
        }

        let Some(path) = self.coop_map_path(id) else {
            return;
        };

        let Some(document) = self.state.document_mut(id) else {
            return;
        };

        // edits so far are part of the snapshot, the journal only carries what comes after it
        document.start_journal();
        document.take_journal();

        let Some(coop) = self.coop.as_mut() else {
            return;
        };

        let you = coop.you.unwrap_or(PeerId(0));
        let shared_map = coop
            .shared_maps
            .entry(path.clone())
            .or_insert_with(|| SharedMap::new(you, SharedState::Closed));

        // only a copy that follows its generation has the edits before `next_seq`
        let base = shared_map
            .generation
            .filter(|_| shared_map.is_ready())
            .map(|generation| (generation, shared_map.next_seq));

        // hold back new edits until the server numbers this share
        shared_map.state = SharedState::Sending { done: 0, total: 0 };

        let map = document.map.clone();
        let prepared = coop.prepared.0.clone();
        thread::spawn(move || {
            let bytes = writer::write(&map).into_bytes();
            let _ = prepared.send(Prepared::Upload { path, base, bytes });
        });
    }

    fn open_shared_map(&mut self, received: ReceivedMap) {
        let Some(shared_map) = self
            .coop
            .as_mut()
            .and_then(|coop| coop.shared_maps.get_mut(&received.path))
            .filter(|shared_map| {
                matches!(shared_map.state, SharedState::Loading(loading) if loading == received.generation)
            })
        else {
            return;
        };

        if shared_map
            .generation
            .is_some_and(|current| received.generation < current)
        {
            // a newer share is on its way
            shared_map.state = SharedState::RECEIVING;
            return;
        }

        let open = self.state.document_for_path(&received.file);
        // a local copy with unsaved work waits until it is saved or closed, our copy of the session's map does not
        if !shared_map.received && open.is_some_and(|id| self.state.document(id).is_some_and(MapDocument::is_dirty)) {
            shared_map.state = SharedState::Waiting(Box::new(received));
            return;
        }

        let ReceivedMap {
            generation,
            file,
            map,
            errors,
            is_modified,
            ..
        } = received;

        // the snapshot of a new share can beat its announcement
        shared_map.renumber(generation);
        shared_map.state = SharedState::Ready;
        shared_map.in_flight.clear();
        shared_map.pending_document = None;
        shared_map.received = true;

        let repo = self.git_enabled.then(|| git::discover(&file)).flatten();
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
            if is_modified {
                document.mark_unsaved();
            }
        }
    }

    pub fn coop_map_path(&self, id: DocumentId) -> Option<String> {
        self.coop_file_path(self.state.document(id)?.path.as_deref()?)
    }

    // peers only line up on a map they all have the shared copy of
    pub fn coop_shared_map_path(&self, id: DocumentId) -> Option<String> {
        let path = self.coop_map_path(id)?;
        let is_ready = self.coop.as_ref()?.shared_maps.get(&path)?.is_ready();

        is_ready.then_some(path)
    }

    fn coop_file_path(&self, file: &Path) -> Option<String> {
        let relative = file.strip_prefix(self.codebase_dir()?).ok()?;
        let path = relative
            .components()
            .map(|part| part.as_os_str().to_str())
            .collect::<Option<Vec<_>>>()?
            .join("/");

        net::is_map_path(&path).then_some(path)
    }

    pub fn coop_cursor(&self, cursor: Option<Cursor>) {
        if let Some(client) = self
            .coop
            .as_ref()
            .filter(|coop| coop.can_collaborate())
            .and_then(|coop| coop.client.as_ref())
        {
            client.send_cursor(cursor);
        }
    }
}

#[cfg(test)]
mod tests {
    use core::path::TreePath;
    use std::{
        env,
        process,
        time::{Duration, Instant},
    };

    use dmm::Prefab;
    use editor::command::Edit;
    use net::{CodebaseHash, Impairment, LossyProxy};

    use super::*;
    use crate::session::fixtures;

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
            sessions[0].coop().is_some_and(
                |coop| matches!(&coop.status, CoopStatus::Ended(reason) if reason.contains("wrong password")),
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

    const OTHER: PeerId = PeerId(99);

    fn hosting(name: &str) -> (PathBuf, Session) {
        let (dir, mut session) = codebase_with_map(name, "aa");
        session
            .host_coop(0, String::from("hunter2"), String::from("host"))
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
                    .is_some_and(|document| document.map.size.x == 2)
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
                    .is_some_and(|document| document.map.size.x == 3)
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
            .host_coop(0, String::from("hunter2"), String::from("host"))
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

        host.host_coop(0, String::from("hunter2"), String::from("host"))
            .unwrap();
        poll_until(&mut [&mut host], |sessions| connected(sessions[0]));
        assert!(host.can_share_coop_map());
        host.share_coop_map();
        poll_until(&mut [&mut host], |sessions| {
            sessions[0]
                .coop()
                .is_some_and(|coop| coop.shared_maps.contains_key("_maps/a.dmm"))
        });

        let port = host.coop().and_then(Coop::host).unwrap().0.port();
        guest
            .join_coop(
                format!("127.0.0.1:{port}"),
                String::from("hunter2"),
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
    fn an_out_of_date_copy_has_no_cursors_or_comments() {
        let (host_dir, host, guest_dir, guest, local) = guest_out_of_date("cursor-out-of-date");
        let host_map = host.state.document_for_path(&host_dir.join("_maps/a.dmm")).unwrap();
        assert_eq!(host.coop_shared_map_path(host_map).as_deref(), Some("_maps/a.dmm"));
        assert_eq!(guest.coop_shared_map_path(local), None);

        let _ = fs::remove_dir_all(host_dir);
        let _ = fs::remove_dir_all(guest_dir);
    }

    fn width_on_disk(file: &Path) -> u32 { parser::parse(&fs::read_to_string(file).unwrap()).0.size.x }

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
        assert_eq!(received.map.size.x, 2);
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
                .is_some_and(|document| document.map.size.x == 2)
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
        assert_eq!(document.map.size.x, 1);
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
                .is_some_and(|document| document.map.size.x == 2)
        });

        let id = guest.state.document_for_path(&file).unwrap();
        assert!(!guest.state.document(id).unwrap().is_read_only());
        assert!(guest.coop().unwrap().shared_maps["_maps/a.dmm"].is_ready());
        assert_eq!(width_on_disk(&file), 1);

        let _ = fs::remove_dir_all(&host_dir);
        let _ = fs::remove_dir_all(&guest_dir);
    }

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
        let port = host.coop().unwrap().host().unwrap().0.port();
        guest
            .join_coop(format!("127.0.0.1:{port}"), "hunter2".into(), "guest".into())
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
        assert_eq!(document.map.size.x, 1);
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
        assert_eq!(guest.state.document(local).unwrap().map.size.x, 1);
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
        assert_eq!(guest.state.document(local).unwrap().map.size.x, 1);
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
        let port = host.coop().unwrap().host().unwrap().0.port();
        guest
            .join_coop(format!("127.0.0.1:{port}"), "hunter2".into(), "guest".into())
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
                })
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

        host.host_coop(0, String::from("hunter2"), String::from("host"))
            .unwrap();
        poll_until(&mut [&mut host], |sessions| connected(sessions[0]));
        host.share_coop_map();
        let port = host.coop().and_then(Coop::host).unwrap().0.port();
        let join = |session: &mut Session, nick: &str| {
            session
                .join_coop(format!("127.0.0.1:{port}"), String::from("hunter2"), String::from(nick))
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

        host.host_coop(0, String::from("hunter2"), String::from("host"))
            .unwrap();
        poll_until(&mut [&mut host], |sessions| connected(sessions[0]));
        host.share_coop_map();
        let port = host.coop().and_then(Coop::host).unwrap().0.port();
        guest
            .join_coop(
                format!("127.0.0.1:{port}"),
                String::from("hunter2"),
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
    fn a_closed_shared_map_resyncs_when_reopened() {
        let (host_dir, mut host) = codebase_with_map("resync-host", "aa");
        let (guest_dir, mut guest) = codebase_with_map("resync-guest", "aa");
        let host_file = host_dir.join("_maps/a.dmm");
        let guest_file = guest_dir.join("_maps/a.dmm");
        let (left, right) = (Coord::new(1, 1, 1), Coord::new(2, 1, 1));
        open_local(&mut host, host_file.clone());

        host.host_coop(0, String::from("hunter2"), String::from("host"))
            .unwrap();
        poll_until(&mut [&mut host], |sessions| connected(sessions[0]));
        host.share_coop_map();
        let port = host.coop().and_then(Coop::host).unwrap().0.port();
        guest
            .join_coop(
                format!("127.0.0.1:{port}"),
                String::from("hunter2"),
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
    fn edits_converge_through_a_lossy_link() {
        let server = Server::spawn(ServerConfig {
            codebase: None,
            bind: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
            password: String::from("hunter2"),
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
                    String::from("hunter2"),
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
    fn coop_needs_a_codebase() {
        assert!(
            Session::new()
                .join_coop(String::from("127.0.0.1:1"), String::from("pw"), String::from("me"))
                .is_err()
        );
    }
}
