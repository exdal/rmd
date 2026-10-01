use std::{
    collections::{BTreeMap, HashMap},
    net::SocketAddr,
    path::PathBuf,
    sync::mpsc,
    thread,
    time::Instant,
};

use dmm::{Coord, error::MapError};
use editor::document::DocumentId;
use net::{Client, CodebaseId, Comment, CommentId, Cursor, GenerationId, MapEdit, PeerId, PeerInfo, SeqId, Server};

use super::Session;

mod connection;
mod edits;
mod event;
mod maps;
mod presence;
#[cfg(test)]
mod tests;

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

    fn mismatch(&mut self, expected: CodebaseId) {
        self.end(String::from("different codebase"));
        self.status = CoopStatus::CodebaseMismatch { expected };
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
}
