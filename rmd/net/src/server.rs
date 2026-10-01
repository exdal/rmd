use std::{
    collections::HashMap,
    mem,
    net::SocketAddr,
    sync::{
        Arc,
        Mutex,
        atomic::{AtomicU32, Ordering},
    },
    thread,
    time::Duration,
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
        Datagram,
        GenerationId,
        MAX_COMMENT_LEN,
        MAX_MAP_LEN,
        MAX_NICK_LEN,
        MapEdit,
        PeerId,
        PeerInfo,
        Relayed,
        SeqId,
        ServerMessage,
        Transfer,
        is_map_path,
    },
};
use quinn::{Connection, Endpoint, EndpointConfig, TokioRuntime};
use ring::rand::{SecureRandom, SystemRandom};
use tokio::{
    sync::{mpsc, oneshot},
    task::{AbortHandle, JoinSet},
    time,
};

use crate::{
    Error,
    fail,
    socket,
    stream::{
        CLOSE_GRACE,
        abort_transfers,
        read_message,
        read_payload,
        read_transfer_header,
        send_transfer,
        write_message,
    },
    tls,
};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const PASSWORD_ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
const PASSWORD_LEN: usize = 8;

pub fn random_password() -> String {
    let mut bytes = [0; PASSWORD_LEN];
    SystemRandom::new()
        .fill(&mut bytes)
        .expect("the system random number generator failed");

    bytes
        .iter()
        .map(|byte| PASSWORD_ALPHABET[*byte as usize % PASSWORD_ALPHABET.len()] as char)
        .collect()
}

pub struct ServerConfig {
    pub bind: SocketAddr,
    pub password: String,
    pub codebase: Option<CodebaseId>,
}

pub struct Server {
    addr: SocketAddr,
    password: String,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Server {
    pub fn spawn(config: ServerConfig) -> Result<Self, Error> {
        let runtime = crate::runtime()?;
        let endpoint = {
            let _context = runtime.enter();
            let socket = socket::bind_udp(config.bind).map_err(fail)?;
            Endpoint::new(
                EndpointConfig::default(),
                Some(tls::server_config()?),
                socket,
                Arc::new(TokioRuntime),
            )
            .map_err(fail)?
        };

        let addr = endpoint.local_addr().map_err(fail)?;
        let shared = Arc::new(Shared {
            password: config.password.clone(),
            state: Mutex::new(State {
                codebase: config.codebase,
                ..State::default()
            }),
            next_id: AtomicU32::new(1),
        });

        let (shutdown, stop) = oneshot::channel();
        let thread = thread::Builder::new()
            .name(String::from("rmd-server"))
            .spawn(move || runtime.block_on(serve(endpoint, shared, stop)))
            .map_err(fail)?;

        Ok(Self {
            addr,
            password: config.password,
            shutdown: Some(shutdown),
            thread: Some(thread),
        })
    }

    pub fn local_addr(&self) -> SocketAddr { self.addr }

    pub fn password(&self) -> &str { &self.password }

    pub fn wait(mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }

        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Shared {
    password: String,
    state: Mutex<State>,
    next_id: AtomicU32,
}

#[derive(Default)]
struct State {
    codebase: Option<CodebaseId>,
    peers: HashMap<PeerId, Peer>,
    comments: Vec<Comment>,
    last_comment: CommentId,
    maps: HashMap<String, SharedMap>,
    uploads: HashMap<String, (PeerId, u64)>,
    // generations never repeat, even for a path shared again after it stopped
    last_generation: GenerationId,
}

impl State {
    fn pin_codebase(&mut self, candidate: &CodebaseId) -> Result<CodebaseId, CodebaseId> {
        let expected = self.codebase.get_or_insert_with(|| candidate.clone());
        if expected.hash == candidate.hash {
            Ok(expected.clone())
        } else {
            Err(expected.clone())
        }
    }
}

struct SharedMap {
    by: PeerId,
    bytes: Bytes,
    generation: GenerationId,
    replaced: Option<GenerationId>,
    edits: Vec<(PeerId, MapEdit)>,
    pushes: Vec<AbortHandle>,
}

impl SharedMap {
    fn abort_pushes(&self) {
        for push in &self.pushes {
            push.abort();
        }
    }
}

struct Peer {
    info: PeerInfo,
    connection: Connection,
    outbox: mpsc::UnboundedSender<ServerMessage>,
    pushes: JoinSet<()>,
}

impl Shared {
    fn broadcast(&self, from: PeerId, message: &ServerMessage) {
        let state = self.state.lock().unwrap();
        for (id, peer) in state.peers.iter() {
            if *id != from {
                let _ = peer.outbox.send(message.clone());
            }
        }
    }

    fn relay(&self, from: PeerId, bytes: &[u8]) {
        let Ok(datagram) = protocol::decode::<Datagram>(bytes) else {
            return;
        };

        let Ok(relayed) = protocol::encode(&Relayed { from, datagram }) else {
            return;
        };

        let relayed = Bytes::from(relayed);
        let state = self.state.lock().unwrap();
        for (id, peer) in state.peers.iter() {
            if *id != from {
                let _ = peer.connection.send_datagram(relayed.clone());
            }
        }
    }

    fn comment(&self, author: PeerId, map: String, z: u32, pos: [f32; 2], text: &str) {
        let Some(text) = clean_text(text, MAX_COMMENT_LEN) else {
            return;
        };

        let mut state = self.state.lock().unwrap();
        state.last_comment = state.last_comment.next();
        let comment = Comment {
            id: state.last_comment,
            author,
            map,
            z,
            pos,
            text,
        };

        for peer in state.peers.values() {
            let _ = peer.outbox.send(ServerMessage::Comment(comment.clone()));
        }

        state.comments.push(comment);
    }

    fn delete_comment(&self, id: CommentId) {
        let mut state = self.state.lock().unwrap();
        let Some(index) = state.comments.iter().position(|comment| comment.id == id) else {
            return;
        };

        state.comments.remove(index);
        for peer in state.peers.values() {
            let _ = peer.outbox.send(ServerMessage::CommentDeleted(id));
        }
    }

    fn upload_started(&self, by: PeerId, path: &str, len: u64) {
        self.state.lock().unwrap().uploads.insert(path.to_owned(), (by, len));
        self.broadcast(
            by,
            &ServerMessage::MapIncoming {
                path: path.to_owned(),
                by,
                len,
            },
        );
    }

    fn upload_failed(&self, by: PeerId, path: &str) {
        {
            let mut state = self.state.lock().unwrap();
            if state.uploads.get(path).is_none_or(|(uploader, _)| *uploader != by) {
                return;
            }

            state.uploads.remove(path);
        }

        self.broadcast(
            by,
            &ServerMessage::MapCancelled {
                path: path.to_owned(),
                by,
            },
        );
    }

    fn share_map(&self, by: PeerId, path: String, base: GenerationId, next_seq: SeqId, bytes: Bytes) {
        let mut state = self.state.lock().unwrap();
        if state.uploads.get(&path).is_some_and(|(uploader, _)| *uploader == by) {
            state.uploads.remove(&path);
        }

        let State {
            peers,
            maps,
            last_generation,
            ..
        } = &mut *state;
        let previous = maps.remove(&path);
        *last_generation = last_generation.next();
        let generation = *last_generation;
        // peers kept editing while the snapshot was on its way, and the sharer's own edits are already in it
        let edits = previous
            .as_ref()
            .filter(|map| map.generation == base)
            .map(|map| {
                map.edits
                    .iter()
                    .skip(next_seq.0 as usize)
                    .filter(|(author, _)| *author != by)
                    .map(|(author, edit)| {
                        (
                            *author,
                            MapEdit {
                                generation,
                                ..edit.clone()
                            },
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let replaced = previous.as_ref().map(|map| map.generation);
        // peers drop an older generation anyway
        if let Some(previous) = &previous {
            previous.abort_pushes();
        }

        let mut map_pushes = Vec::new();
        for (id, peer) in peers.iter_mut() {
            let _ = peer.outbox.send(ServerMessage::MapShared {
                path: path.clone(),
                by,
                generation,
            });
            if *id != by {
                push(
                    &mut peer.pushes,
                    &mut map_pushes,
                    peer.connection.clone(),
                    path.clone(),
                    generation,
                    bytes.clone(),
                );
            }

            for (seq, (author, edit)) in edits.iter().enumerate() {
                let _ = peer.outbox.send(ServerMessage::Edit {
                    by: *author,
                    seq: SeqId(seq as u32),
                    edit: edit.clone(),
                });
            }
        }

        maps.insert(
            path,
            SharedMap {
                by,
                bytes,
                generation,
                replaced,
                edits,
                pushes: map_pushes,
            },
        );
    }

    fn edit(&self, by: PeerId, mut edit: MapEdit) {
        let mut state = self.state.lock().unwrap();
        let State { peers, maps, .. } = &mut *state;
        let Some(map) = maps.get_mut(&edit.path) else {
            return;
        };

        // sent before the peer heard of the new share, which doesn't have it
        if map.replaced == Some(edit.generation) && by != map.by {
            edit.generation = map.generation;
        }

        if map.generation != edit.generation {
            return;
        }

        let seq = SeqId(map.edits.len() as u32);
        for peer in peers.values() {
            let _ = peer.outbox.send(ServerMessage::Edit {
                by,
                seq,
                edit: edit.clone(),
            });
        }

        map.edits.push((by, edit));
    }

    // echoed even for a map we don't have, so a peer that still shows it can drop it
    fn unshare(&self, by: PeerId, path: &str) {
        let mut state = self.state.lock().unwrap();
        let generation = state.maps.remove(path).map(|map| {
            map.abort_pushes();

            map.generation
        });

        state.comments.retain(|comment| comment.map != path);
        for peer in state.peers.values() {
            let _ = peer.outbox.send(ServerMessage::MapUnshared {
                path: path.to_owned(),
                by,
                generation,
            });
        }
    }

    fn resync(&self, id: PeerId, path: &str) {
        let mut state = self.state.lock().unwrap();
        let State { peers, maps, .. } = &mut *state;
        let (Some(peer), Some(map)) = (peers.get_mut(&id), maps.get_mut(path)) else {
            return;
        };

        replay(&peer.outbox, &peer.connection, path, map, &mut peer.pushes);
    }
}

fn replay(
    outbox: &mpsc::UnboundedSender<ServerMessage>, connection: &Connection, path: &str, map: &mut SharedMap,
    pushes: &mut JoinSet<()>,
) {
    push(
        pushes,
        &mut map.pushes,
        connection.clone(),
        path.to_owned(),
        map.generation,
        map.bytes.clone(),
    );
    for (seq, (by, edit)) in map.edits.iter().enumerate() {
        let _ = outbox.send(ServerMessage::Edit {
            by: *by,
            seq: SeqId(seq as u32),
            edit: edit.clone(),
        });
    }
}

fn push(
    pushes: &mut JoinSet<()>, map_pushes: &mut Vec<AbortHandle>, connection: Connection, path: String,
    generation: GenerationId, bytes: Bytes,
) {
    while pushes.try_join_next().is_some() {}
    map_pushes.retain(|push| !push.is_finished());
    map_pushes.push(pushes.spawn(push_map(connection, path, generation, bytes)));
}

async fn push_map(connection: Connection, path: String, generation: GenerationId, bytes: Bytes) {
    let header = Transfer::Map {
        path,
        generation,
        next_seq: SeqId(0),
        len: bytes.len() as u64,
    };

    if let Err(e) = send_transfer(&connection, &header, &bytes, |_| {}).await {
        log::debug!("could not send a map to {}: {e}", connection.remote_address());
    }
}

async fn upload(shared: Arc<Shared>, from: PeerId, mut stream: quinn::RecvStream) {
    let (path, base, next_seq, len) = match read_transfer_header(&mut stream).await {
        Ok(Transfer::Map {
            path,
            generation,
            next_seq,
            len,
        }) if is_map_path(&path) => (path, generation, next_seq, len),
        Ok(Transfer::Map { path, .. }) => {
            log::info!("{from:?} tried to share {path}, which is not a map path");
            return;
        },
        Err(e) => {
            log::info!("dropping an upload from {from:?}: {e}");
            return;
        },
    };

    if len > MAX_MAP_LEN as u64 {
        log::info!("{from:?} tried to share {path}, which is {len} bytes");
        return;
    }

    shared.upload_started(from, &path, len);
    match read_payload(&mut stream, len, |_| {}).await {
        Ok(bytes) => shared.share_map(from, path, base, next_seq, Bytes::from(bytes)),
        Err(e) => {
            log::info!("dropping an upload of {path} from {from:?}: {e}");
            shared.upload_failed(from, &path);
        },
    }
}

async fn serve(endpoint: Endpoint, shared: Arc<Shared>, mut stop: oneshot::Receiver<()>) {
    loop {
        tokio::select! {
            _ = &mut stop => break,
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else {
                    break;
                };

                tokio::spawn(handle(incoming, shared.clone()));
            },
        }
    }

    let pushes = shared
        .state
        .lock()
        .unwrap()
        .peers
        .values_mut()
        .map(|peer| (peer.connection.clone(), mem::take(&mut peer.pushes)))
        .collect::<Vec<_>>();
    abort_transfers(pushes).await;
    endpoint.close(0u32.into(), b"server shutting down");
    let _ = time::timeout(CLOSE_GRACE, endpoint.wait_idle()).await;
}

async fn handle(incoming: quinn::Incoming, shared: Arc<Shared>) {
    let remote = incoming.remote_address();
    let connection = match incoming.await {
        Ok(connection) => connection,
        Err(e) => {
            log::debug!("{remote} failed to connect: {e}");
            return;
        },
    };

    if let Err(e) = session(&connection, &shared).await {
        log::info!("{remote}: {e}");
    }

    connection.close(0u32.into(), b"");
}

async fn session(connection: &Connection, shared: &Arc<Shared>) -> Result<(), Error> {
    let (mut send, mut recv) = time::timeout(HANDSHAKE_TIMEOUT, connection.accept_bi())
        .await
        .map_err(|_| fail("handshake timed out"))?
        .map_err(fail)?;

    let mut reader = FrameReader::new();
    let hello = time::timeout(HANDSHAKE_TIMEOUT, handshake(&mut recv, &mut reader))
        .await
        .map_err(|_| fail("handshake timed out"))??;

    let nick = match admit(&shared.password, &hello) {
        Ok(nick) => nick,
        Err(reason) => {
            write_message(
                &mut send,
                &ServerMessage::Reject {
                    reason: reason.to_owned(),
                },
            )
            .await?;

            // let the client read the rejection and hang up first
            let _ = time::timeout(Duration::from_secs(2), connection.closed()).await;

            return Err(fail(reason));
        },
    };

    let id = PeerId(shared.next_id.fetch_add(1, Ordering::Relaxed));
    let info = PeerInfo {
        id,
        nick,
        codebase: hello.codebase,
    };

    let (outbox, mut inbox) = mpsc::unbounded_channel();
    let mismatch = {
        let mut state = shared.state.lock().unwrap();
        match state.pin_codebase(&info.codebase) {
            Err(expected) => Some(expected),
            Ok(codebase) => {
                let mut others = state.peers.values().map(|peer| peer.info.clone()).collect::<Vec<_>>();
                others.sort_by_key(|peer| peer.id);

                let _ = outbox.send(ServerMessage::Welcome {
                    you: id,
                    codebase,
                    peers: others,
                    comments: state.comments.clone(),
                });
                for peer in state.peers.values() {
                    let _ = peer.outbox.send(ServerMessage::PeerJoined(info.clone()));
                }

                let mut pushes = JoinSet::new();
                for (path, map) in state.maps.iter_mut() {
                    let _ = outbox.send(ServerMessage::MapShared {
                        path: path.clone(),
                        by: map.by,
                        generation: map.generation,
                    });
                    replay(&outbox, connection, path, map, &mut pushes);
                }

                for (path, (by, len)) in state.uploads.iter() {
                    let _ = outbox.send(ServerMessage::MapIncoming {
                        path: path.clone(),
                        by: *by,
                        len: *len,
                    });
                }

                state.peers.insert(
                    id,
                    Peer {
                        info: info.clone(),
                        connection: connection.clone(),
                        outbox,
                        pushes,
                    },
                );
                None
            },
        }
    };

    if let Some(expected) = mismatch {
        write_message(&mut send, &ServerMessage::CodebaseMismatch { expected }).await?;
        let _ = time::timeout(Duration::from_secs(2), connection.closed()).await;
        return Err(fail("different codebase"));
    }

    log::info!("{} joined as {id:?} from {}", info.nick, connection.remote_address());

    // a write stalled on flow control must not stop us reading, or both ends can wait on each other
    let writer = async {
        while let Some(message) = inbox.recv().await {
            write_message(&mut send, &message).await?;
        }

        Ok::<_, Error>(())
    };

    let reader = async {
        loop {
            tokio::select! {
                message = read_message::<ClientMessage>(&mut recv, &mut reader) => match message? {
                    Some(ClientMessage::Hello(_)) => return Err(fail("sent a second hello")),
                    Some(ClientMessage::Comment { map, z, pos, text }) => shared.comment(id, map, z, pos, &text),
                    Some(ClientMessage::DeleteComment(comment)) => shared.delete_comment(comment),
                    Some(ClientMessage::Edit(edit)) => shared.edit(id, edit),
                    Some(ClientMessage::Resync { path }) => shared.resync(id, &path),
                    Some(ClientMessage::Unshare { path }) => shared.unshare(id, &path),
                    None => return Ok(()),
                },
                datagram = connection.read_datagram() => shared.relay(id, &datagram.map_err(fail)?),
                stream = connection.accept_uni() => {
                    tokio::spawn(upload(shared.clone(), id, stream.map_err(fail)?));
                },
            }
        }
    };

    let result = tokio::select! {
        result = writer => result,
        result = reader => result,
    };

    shared.state.lock().unwrap().peers.remove(&id);
    shared.broadcast(id, &ServerMessage::PeerLeft(id));
    log::info!("{} left", info.nick);

    result
}

async fn handshake(recv: &mut quinn::RecvStream, reader: &mut FrameReader) -> Result<ClientHello, Error> {
    let hello = read_message::<Hello>(recv, reader)
        .await?
        .ok_or_else(|| fail("closed before the handshake"))?;

    hello.check(Service::Coop).map_err(fail)?;
    match read_message::<ClientMessage>(recv, reader).await? {
        Some(ClientMessage::Hello(hello)) => Ok(hello),
        Some(_) => Err(fail("skipped the handshake")),
        None => Err(fail("closed before the handshake")),
    }
}

fn admit(password: &str, hello: &ClientHello) -> Result<String, &'static str> {
    if hello.password != password {
        return Err("wrong password");
    }

    clean_text(&hello.nick, MAX_NICK_LEN).ok_or("nick is empty")
}

fn clean_text(text: &str, max: usize) -> Option<String> {
    let text = text.chars().filter(|c| !c.is_control()).take(max).collect::<String>();

    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

#[cfg(test)]
mod tests {
    use protocol::coop::CodebaseHash;

    use super::*;

    #[test]
    fn an_empty_relay_keeps_its_codebase_and_ignores_git_hints() {
        let original = CodebaseId {
            hash: CodebaseHash([7; 32]),
            git_hint: Some(String::from("main")),
        };
        let mut state = State::default();
        assert_eq!(state.pin_codebase(&original), Ok(original.clone()));
        state.peers.clear();
        assert_eq!(
            state.pin_codebase(&CodebaseId {
                hash: CodebaseHash([8; 32]),
                git_hint: None
            }),
            Err(original.clone())
        );
        assert_eq!(
            state.pin_codebase(&CodebaseId {
                hash: original.hash,
                git_hint: Some(String::from("other"))
            }),
            Ok(original)
        );
    }

    #[test]
    fn nicks_lose_control_characters_and_excess_length() {
        assert_eq!(clean_text("  mapper\n", MAX_NICK_LEN), Some(String::from("mapper")));
        assert_eq!(clean_text("\u{7}\t ", MAX_NICK_LEN), None);
        assert_eq!(
            clean_text(&"a".repeat(100), MAX_NICK_LEN).map(|nick| nick.len()),
            Some(MAX_NICK_LEN)
        );
    }

    #[test]
    fn random_passwords_use_the_readable_alphabet() {
        let password = random_password();

        assert_eq!(password.len(), PASSWORD_LEN);
        assert!(password.bytes().all(|byte| PASSWORD_ALPHABET.contains(&byte)));
        assert_ne!(password, random_password());
    }
}
