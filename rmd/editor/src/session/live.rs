use std::{
    collections::BTreeMap,
    net::{Ipv4Addr, SocketAddr},
    sync::mpsc,
    time::Instant,
};

use editor::{document::DocumentId, tool::Tool};
use net::{Client, CodebaseId, Comment, CommentId, Cursor, Event, PeerId, PeerInfo, Server, ServerConfig};

use super::Session;

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
    server: Option<Server>,
    codebase: Option<CodebaseId>,
    pending: Option<PendingJoin>,
    client: Option<Client>,
    last_poll: Instant,
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

    fn apply(&mut self, event: Event) {
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
            Event::Rejected(reason) => self.end(format!("rejected: {reason}")),
            Event::Disconnected(reason) => self.end(reason),
        }
    }

    fn end(&mut self, reason: String) {
        self.status = LiveStatus::Ended(reason);
        self.peers.clear();
        self.comments.clear();
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
            live.apply(event);
        }

        let now = Instant::now();
        let elapsed = now.duration_since(live.last_poll).as_secs_f32();
        live.last_poll = now;
        for peer in live.peers.values_mut() {
            peer.follow(elapsed);
        }
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

    #[test]
    fn live_share_needs_a_codebase() {
        assert!(
            Session::new()
                .join_live(String::from("127.0.0.1:1"), String::from("pw"), String::from("me"))
                .is_err()
        );
    }
}
