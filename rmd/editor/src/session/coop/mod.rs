use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    net::SocketAddr,
    path::PathBuf,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use dmm::{Coord, error::MapError};
use editor::document::DocumentId;
use net::{
    Client,
    CodebaseId,
    Comment,
    CommentId,
    Cursor,
    GenerationId,
    InsertAt,
    InsertLevel,
    LevelLog,
    LevelOp,
    MapEdit,
    PRESENCE_REFRESH,
    PasswordHash,
    PeerId,
    PeerInfo,
    Selection,
    SeqId,
    Server,
    View,
};

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
// map pixels, close enough to the received position to stop drawing at full rate
const CURSOR_SETTLED: f32 = 0.1;

const MAX_ACTIVITY: usize = 8;

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum CoopStatus {
    Connecting,
    Connected,
    Ended(String),
    CodebaseMismatch { expected: CodebaseId },
}

pub(crate) enum Activity {
    Joined(PeerInfo),
    Left(PeerInfo),
    Unshared {
        path: String,
        by: PeerId,
        nick: Option<String>,
    },
}

const PRESENCE_TIMEOUT: Duration = Duration::from_secs(3 * PRESENCE_REFRESH.as_secs());

struct Received<T> {
    value: T,
    at: Instant,
}

impl<T> Received<T> {
    fn now(value: T) -> Self {
        Self {
            value,
            at: Instant::now(),
        }
    }
}

pub(crate) struct RemotePeer {
    pub info: PeerInfo,
    cursor: Option<Received<Cursor>>,
    pub shown: [f32; 2],
    view: Option<Received<View>>,
    selection: Option<Received<Selection>>,
}

impl RemotePeer {
    fn new(info: PeerInfo) -> Self {
        Self {
            info,
            cursor: None,
            shown: [0.0; 2],
            view: None,
            selection: None,
        }
    }

    pub fn cursor(&self) -> Option<&Cursor> { self.cursor.as_ref().map(|cursor| &cursor.value) }

    pub fn view(&self) -> Option<&View> { self.view.as_ref().map(|view| &view.value) }

    pub fn selection(&self) -> Option<&Selection> { self.selection.as_ref().map(|selection| &selection.value) }

    pub fn is_cursor_easing(&self) -> bool {
        self.cursor().is_some_and(|cursor| {
            cursor
                .pos
                .iter()
                .zip(self.shown)
                .any(|(target, shown)| (target - shown).abs() > CURSOR_SETTLED)
        })
    }

    fn set_cursor(&mut self, cursor: Option<Cursor>) {
        if let Some(next) = cursor.as_ref()
            && self
                .cursor()
                .is_none_or(|previous| previous.map != next.map || previous.z != next.z)
        {
            self.shown = next.pos;
        }

        self.cursor = cursor.map(Received::now);
    }

    fn follow(&mut self, elapsed: f32) {
        let Some(cursor) = self.cursor.as_ref() else {
            return;
        };

        let blend = 1.0 - (-elapsed * CURSOR_SMOOTHING).exp();
        for (shown, target) in self.shown.iter_mut().zip(cursor.value.pos) {
            *shown += (target - *shown) * blend;
        }
    }

    fn expire(&mut self, now: Instant) {
        let is_stale = |at: Instant| now.duration_since(at) >= PRESENCE_TIMEOUT;
        self.cursor.take_if(|cursor| is_stale(cursor.at));
        self.view.take_if(|view| is_stale(view.at));
        self.selection.take_if(|selection| is_stale(selection.at));
    }
}

pub(crate) struct Coop {
    pub status: CoopStatus,
    pub nick: String,
    pub you: Option<PeerId>,
    pub peers: BTreeMap<PeerId, RemotePeer>,
    pub comments: BTreeMap<CommentId, Comment>,
    pub shared_maps: BTreeMap<String, SharedMap>,
    pub activity: VecDeque<(Instant, Activity)>,
    following: Option<Following>,
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

struct Following {
    peer: PeerId,
    document: Option<DocumentId>,
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
    // a level change we asked for, it happens for everyone once the server sends it back
    level_request: Option<LevelRequest>,
    level_log: LevelLog,
}

enum LevelRequest {
    Queued(LevelOp, LevelStep),
    Sent(LevelStep),
}

impl LevelRequest {
    fn step(&self) -> LevelStep {
        match self {
            Self::Queued(_, step) | Self::Sent(step) => *step,
        }
    }
}

// how our own level change lands in history once the server sends it back
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::session) enum LevelStep {
    Change,
    Undo,
    Redo,
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
            level_request: None,
            level_log: LevelLog::default(),
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
            // an unsent append still applies, it lands on top of any map,
            // but other level changes picked their z on the old snapshot
            let queued = self.level_request.take().filter(|request| {
                matches!(
                    request,
                    LevelRequest::Queued(
                        LevelOp::Insert(InsertLevel { at: InsertAt::Top, .. }),
                        LevelStep::Change
                    )
                )
            });
            self.forget_edits();
            self.level_request = queued;
        }
    }

    fn forget_edits(&mut self) {
        self.inbox.clear();
        self.next_seq = SeqId(0);
        self.in_flight.clear();
        self.level_request = None;
        self.level_log = LevelLog::default();
    }

    fn send_level_request(&mut self) -> Option<LevelOp> {
        match self.level_request.take()? {
            LevelRequest::Queued(op, step) => {
                self.level_request = Some(LevelRequest::Sent(step));

                Some(op)
            },
            LevelRequest::Sent(step) => {
                self.level_request = Some(LevelRequest::Sent(step));

                None
            },
        }
    }
}

enum Prepared {
    Upload {
        path: String,
        base: Option<(GenerationId, SeqId)>,
        levels: u32,
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
    Host { port: u16, password: PasswordHash },
    Join { addr: String, password: PasswordHash },
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

    pub fn host(&self) -> Option<SocketAddr> { self.server.as_ref().map(Server::local_addr) }

    fn mismatch(&mut self, expected: CodebaseId) {
        self.end(String::from("different codebase"));
        self.status = CoopStatus::CodebaseMismatch { expected };
    }

    fn record(&mut self, activity: Activity) {
        self.activity.push_back((Instant::now(), activity));
        if self.activity.len() > MAX_ACTIVITY {
            self.activity.pop_front();
        }
    }

    // shared maps stay until the session closes their pending documents
    fn end(&mut self, reason: String) {
        self.status = CoopStatus::Ended(reason);
        self.peers.clear();
        self.activity.clear();
        self.comments.clear();
        self.following = None;
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

    fn collaborating_client(&self) -> Option<&Client> {
        self.coop
            .as_ref()
            .filter(|coop| coop.can_collaborate())?
            .client
            .as_ref()
    }
}
