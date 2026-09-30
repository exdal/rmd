use std::{
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use bytes::Bytes;
use protocol::{
    FrameReader,
    Hello,
    Service,
    coop::{
        ClientHello,
        ClientMessage,
        CodebaseId,
        Comment,
        CommentId,
        Cursor,
        Datagram,
        GenerationId,
        MapEdit,
        PeerId,
        PeerInfo,
        Relayed,
        SeqId,
        ServerMessage,
        Transfer,
    },
};
use quinn::{Connection, Endpoint};
use tokio::{
    net::lookup_host,
    sync::{mpsc as channel, oneshot, watch},
    task::JoinSet,
    time::{self, MissedTickBehavior},
};

use crate::{
    Error,
    fail,
    stream::{read_message, read_payload, read_transfer_header, send_transfer, write_message},
    tls,
};

const CURSOR_INTERVAL: Duration = Duration::from_millis(50);
const CLOSE_GRACE: Duration = Duration::from_millis(500);
const RESET_POLL: Duration = Duration::from_millis(5);
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Connected {
        you: PeerId,
        peers: Vec<PeerInfo>,
        comments: Vec<Comment>,
    },
    Rejected(String),
    PeerJoined(PeerInfo),
    PeerLeft(PeerId),
    Cursor {
        from: PeerId,
        cursor: Option<Cursor>,
    },
    Comment(Comment),
    CommentDeleted(CommentId),
    MapShared {
        path: String,
        by: PeerId,
        generation: GenerationId,
    },
    MapSnapshot {
        path: String,
        generation: GenerationId,
        bytes: Vec<u8>,
    },
    Edit {
        by: PeerId,
        seq: SeqId,
        edit: MapEdit,
    },
    MapIncoming {
        path: String,
        by: PeerId,
        len: u64,
    },
    MapCancelled {
        path: String,
        by: PeerId,
    },
    Progress {
        path: String,
        direction: Direction,
        done: u64,
        total: u64,
    },
    TransferFailed {
        path: String,
        direction: Direction,
        reason: String,
    },
    Disconnected(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Sending,
    Receiving,
}

struct Share {
    path: String,
    base: Option<(GenerationId, SeqId)>,
    bytes: Vec<u8>,
}

pub struct Client {
    events: mpsc::Receiver<Event>,
    cursor: watch::Sender<Option<Cursor>>,
    outbox: channel::UnboundedSender<ClientMessage>,
    shares: channel::UnboundedSender<Share>,
    shutdown: Option<oneshot::Sender<()>>,
}

impl Client {
    pub fn connect(addr: String, password: String, nick: String, codebase: CodebaseId) -> Self {
        let (events_tx, events) = mpsc::channel();
        let (cursor, cursor_rx) = watch::channel(None);
        let (outbox, inbox) = channel::unbounded_channel();
        let (shares, shares_rx) = channel::unbounded_channel();
        let (shutdown, stop) = oneshot::channel();
        let hello = ClientHello {
            nick,
            password,
            codebase,
        };

        let failed = events_tx.clone();
        let spawned = thread::Builder::new().name(String::from("rmd-coop")).spawn(move || {
            let result = crate::runtime()
                .and_then(|runtime| runtime.block_on(run(&addr, hello, cursor_rx, inbox, shares_rx, &events_tx, stop)));

            if let Err(e) = result {
                let _ = events_tx.send(Event::Disconnected(e.to_string()));
            }
        });

        if let Err(e) = spawned {
            let _ = failed.send(Event::Disconnected(e.to_string()));
        }

        Self {
            events,
            cursor,
            outbox,
            shares,
            shutdown: Some(shutdown),
        }
    }

    pub fn poll(&self) -> impl Iterator<Item = Event> + '_ { self.events.try_iter() }

    pub fn send_comment(&self, map: String, z: u32, pos: [f32; 2], text: String) {
        self.send(ClientMessage::Comment { map, z, pos, text });
    }

    pub fn delete_comment(&self, id: CommentId) { self.send(ClientMessage::DeleteComment(id)); }

    // `base` is the generation our copy tracks and the next edit of it the snapshot lacks,
    // so the server can carry peers' later edits into the new generation
    pub fn share_map(&self, path: String, base: Option<(GenerationId, SeqId)>, bytes: Vec<u8>) {
        let _ = self.shares.send(Share { path, base, bytes });
    }

    pub fn send_edit(&self, edit: MapEdit) { self.send(ClientMessage::Edit(edit)); }

    pub fn resync(&self, path: String) { self.send(ClientMessage::Resync { path }); }

    fn send(&self, message: ClientMessage) { let _ = self.outbox.send(message); }

    pub fn send_cursor(&self, cursor: Option<Cursor>) {
        self.cursor.send_if_modified(|current| {
            let is_changed = *current != cursor;
            *current = cursor;

            is_changed
        });
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

async fn run(
    addr: &str, hello: ClientHello, cursor: watch::Receiver<Option<Cursor>>,
    outbox: channel::UnboundedReceiver<ClientMessage>, shares: channel::UnboundedReceiver<Share>,
    events: &mpsc::Sender<Event>, mut stop: oneshot::Receiver<()>,
) -> Result<(), Error> {
    let remote = lookup_host(addr)
        .await
        .map_err(|e| fail(format!("could not resolve {addr}: {e}")))?
        .next()
        .ok_or_else(|| fail(format!("could not resolve {addr}")))?;

    let bind = match remote {
        SocketAddr::V4(_) => SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)),
        SocketAddr::V6(_) => SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0)),
    };

    let mut endpoint = Endpoint::client(bind).map_err(fail)?;
    endpoint.set_default_client_config(tls::client_config()?);

    let connecting = endpoint.connect(remote, tls::SERVER_NAME).map_err(fail)?;
    let connection = tokio::select! {
        connection = connecting => connection.map_err(fail)?,
        _ = &mut stop => return Ok(()),
    };

    let mut uploads = JoinSet::new();
    let result = tokio::select! {
        result = session(&connection, hello, cursor, outbox, shares, events, &mut uploads) => result,
        _ = &mut stop => Ok(()),
    };

    // quinn holds the close back while anything else is queued behind a full congestion window,
    // so the resets of unfinished uploads go out first
    while uploads.try_join_next().is_some() {}
    let resets = connection.stats().frame_tx.reset_stream + uploads.len() as u64;
    uploads.shutdown().await;
    let _ = time::timeout(CLOSE_GRACE, async {
        while connection.stats().frame_tx.reset_stream < resets {
            time::sleep(RESET_POLL).await;
        }
    })
    .await;

    connection.close(0u32.into(), b"");
    let _ = time::timeout(CLOSE_GRACE, endpoint.wait_idle()).await;

    result
}

async fn session(
    connection: &Connection, hello: ClientHello, mut cursor: watch::Receiver<Option<Cursor>>,
    mut outbox: channel::UnboundedReceiver<ClientMessage>, mut shares: channel::UnboundedReceiver<Share>,
    events: &mpsc::Sender<Event>, uploads: &mut JoinSet<()>,
) -> Result<(), Error> {
    let emit = |event| {
        let _ = events.send(event);
    };

    let (mut send, mut recv) = connection.open_bi().await.map_err(fail)?;
    write_message(&mut send, &Hello::new(Service::Coop)).await?;
    write_message(&mut send, &ClientMessage::Hello(hello)).await?;

    let mut reader = FrameReader::new();
    match read_message(&mut recv, &mut reader).await? {
        Some(ServerMessage::Welcome { you, peers, comments }) => emit(Event::Connected { you, peers, comments }),
        Some(ServerMessage::Reject { reason }) => {
            emit(Event::Rejected(reason));
            return Ok(());
        },
        Some(_) => return Err(fail("the host skipped the handshake")),
        None => return Err(fail("the host closed the connection")),
    }

    // a write stalled on flow control must not stop us reading, or both ends can wait on each other
    let writer = async {
        while let Some(message) = outbox.recv().await {
            write_message(&mut send, &message).await?;
        }

        Ok::<_, Error>(())
    };

    let reader = async {
        let mut throttle = time::interval(CURSOR_INTERVAL);
        throttle.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                message = read_message(&mut recv, &mut reader) => match message? {
                    Some(ServerMessage::PeerJoined(peer)) => emit(Event::PeerJoined(peer)),
                    Some(ServerMessage::PeerLeft(id)) => emit(Event::PeerLeft(id)),
                    Some(ServerMessage::Comment(comment)) => emit(Event::Comment(comment)),
                    Some(ServerMessage::CommentDeleted(id)) => emit(Event::CommentDeleted(id)),
                    Some(ServerMessage::MapShared { path, by, generation }) => emit(Event::MapShared { path, by, generation }),
                    Some(ServerMessage::Edit { by, seq, edit }) => emit(Event::Edit { by, seq, edit }),
                    Some(ServerMessage::MapIncoming { path, by, len }) => emit(Event::MapIncoming { path, by, len }),
                    Some(ServerMessage::MapCancelled { path, by }) => emit(Event::MapCancelled { path, by }),
                    Some(other) => log::warn!("unexpected message from the host: {other:?}"),
                    None => return Err(fail("the host ended the session")),
                },
                Some(share) = shares.recv() => {
                    uploads.spawn(upload(connection.clone(), share, events.clone()));
                },
                Some(_) = uploads.join_next() => {},
                stream = connection.accept_uni() => {
                    tokio::spawn(download(stream.map_err(fail)?, events.clone()));
                },
                datagram = connection.read_datagram() => {
                    let datagram = datagram.map_err(fail)?;
                    match protocol::decode::<Relayed>(&datagram) {
                        Ok(Relayed { from, datagram: Datagram::Cursor(cursor) }) => emit(Event::Cursor { from, cursor }),
                        Err(e) => log::debug!("dropping a datagram: {e}"),
                    }
                },
                _ = throttle.tick() => {
                    if !cursor.has_changed().unwrap_or(false) {
                        continue;
                    }

                    let datagram = Datagram::Cursor(cursor.borrow_and_update().clone());
                    let bytes = protocol::encode(&datagram).map_err(fail)?;
                    if let Err(e) = connection.send_datagram(Bytes::from(bytes)) {
                        log::warn!("could not send the cursor: {e}");
                    }
                },
            }
        }
    };

    tokio::select! {
        result = writer => result,
        result = reader => result,
    }
}

struct Throttle {
    path: String,
    direction: Direction,
    total: u64,
    last: Option<Instant>,
    events: mpsc::Sender<Event>,
}

impl Throttle {
    fn new(path: String, direction: Direction, total: u64, events: mpsc::Sender<Event>) -> Self {
        Self {
            path,
            direction,
            total,
            last: None,
            events,
        }
    }

    fn report(&mut self, done: u64) {
        let now = Instant::now();
        if done < self.total && self.last.is_some_and(|last| now - last < PROGRESS_INTERVAL) {
            return;
        }

        self.last = Some(now);
        let _ = self.events.send(Event::Progress {
            path: self.path.clone(),
            direction: self.direction,
            done,
            total: self.total,
        });
    }

    fn fail(&self, reason: String) {
        let _ = self.events.send(Event::TransferFailed {
            path: self.path.clone(),
            direction: self.direction,
            reason,
        });
    }
}

async fn upload(connection: Connection, Share { path, base, bytes }: Share, events: mpsc::Sender<Event>) {
    let (generation, next_seq) = base.unwrap_or((GenerationId(0), SeqId(0)));
    let header = Transfer::Map {
        path: path.clone(),
        generation,
        next_seq,
        len: bytes.len() as u64,
    };

    let mut throttle = Throttle::new(path, Direction::Sending, bytes.len() as u64, events);
    if let Err(e) = send_transfer(&connection, &header, &bytes, |done| throttle.report(done)).await {
        log::warn!("could not share {}: {e}", throttle.path);
        throttle.fail(e.to_string());
    }
}

async fn download(mut stream: quinn::RecvStream, events: mpsc::Sender<Event>) {
    let (path, generation, len) = match read_transfer_header(&mut stream).await {
        Ok(Transfer::Map {
            path, generation, len, ..
        }) => (path, generation, len),
        Err(e) => {
            log::warn!("could not receive a map: {e}");
            return;
        },
    };

    let mut throttle = Throttle::new(path, Direction::Receiving, len, events);
    match read_payload(&mut stream, len, |done| throttle.report(done)).await {
        Ok(bytes) => {
            let _ = throttle.events.send(Event::MapSnapshot {
                path: throttle.path,
                generation,
                bytes,
            });
        },
        Err(e) => {
            log::warn!("could not receive {}: {e}", throttle.path);
            throttle.fail(e.to_string());
        },
    }
}
