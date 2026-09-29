use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{
        Arc,
        Mutex,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use bytes::Bytes;
use protocol::{
    FrameReader,
    Hello,
    Service,
    live::{
        ClientHello,
        ClientMessage,
        Comment,
        CommentId,
        Datagram,
        MAX_COMMENT_LEN,
        MAX_NICK_LEN,
        PeerId,
        PeerInfo,
        Relayed,
        ServerMessage,
    },
};
use quinn::{Connection, Endpoint};
use ring::rand::SecureRandom;
use tokio::sync::{mpsc, oneshot};

use crate::{
    Error,
    fail,
    stream::{read_message, write_message},
};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_CONTROL_FRAME: usize = 64 * 1024;
const PASSWORD_ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
const PASSWORD_LEN: usize = 8;

pub fn random_password() -> String {
    let mut bytes = [0; PASSWORD_LEN];
    ring::rand::SystemRandom::new()
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
}

pub struct Server {
    addr: SocketAddr,
    password: String,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    pub fn spawn(config: ServerConfig) -> Result<Self, Error> {
        let runtime = crate::runtime()?;
        let endpoint = {
            let _context = runtime.enter();
            Endpoint::server(crate::tls::server_config()?, config.bind).map_err(fail)?
        };

        let addr = endpoint.local_addr().map_err(fail)?;
        let shared = Arc::new(Shared {
            password: config.password.clone(),
            state: Mutex::default(),
            next_id: AtomicU32::new(1),
        });

        let (shutdown, stop) = oneshot::channel();
        let thread = std::thread::Builder::new()
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
    peers: HashMap<PeerId, Peer>,
    comments: Vec<Comment>,
    last_comment: u32,
}

struct Peer {
    info: PeerInfo,
    connection: Connection,
    outbox: mpsc::UnboundedSender<ServerMessage>,
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
        state.last_comment += 1;
        let comment = Comment {
            id: CommentId(state.last_comment),
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

    endpoint.close(0u32.into(), b"server shutting down");
    endpoint.wait_idle().await;
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

async fn session(connection: &Connection, shared: &Shared) -> Result<(), Error> {
    let (mut send, mut recv) = tokio::time::timeout(HANDSHAKE_TIMEOUT, connection.accept_bi())
        .await
        .map_err(|_| fail("handshake timed out"))?
        .map_err(fail)?;

    let mut reader = FrameReader::with_max(MAX_CONTROL_FRAME);
    let hello = tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake(&mut recv, &mut reader))
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
            let _ = tokio::time::timeout(Duration::from_secs(2), connection.closed()).await;

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
    {
        let mut state = shared.state.lock().unwrap();
        let mut others = state.peers.values().map(|peer| peer.info.clone()).collect::<Vec<_>>();
        others.sort_by_key(|peer| peer.id);

        let _ = outbox.send(ServerMessage::Welcome {
            you: id,
            peers: others,
            comments: state.comments.clone(),
        });
        for peer in state.peers.values() {
            let _ = peer.outbox.send(ServerMessage::PeerJoined(info.clone()));
        }

        state.peers.insert(
            id,
            Peer {
                info: info.clone(),
                connection: connection.clone(),
                outbox,
            },
        );
    }

    log::info!("{} joined as {id:?} from {}", info.nick, connection.remote_address());

    let result = async {
        loop {
            tokio::select! {
                message = inbox.recv() => {
                    let Some(message) = message else {
                        return Ok(());
                    };

                    write_message(&mut send, &message).await?;
                },
                message = read_message::<ClientMessage>(&mut recv, &mut reader) => match message? {
                    Some(ClientMessage::Hello(_)) => return Err(fail("sent a second hello")),
                    Some(ClientMessage::Comment { map, z, pos, text }) => shared.comment(id, map, z, pos, &text),
                    Some(ClientMessage::DeleteComment(comment)) => shared.delete_comment(comment),
                    None => return Ok(()),
                },
                datagram = connection.read_datagram() => shared.relay(id, &datagram.map_err(fail)?),
            }
        }
    }
    .await;

    shared.state.lock().unwrap().peers.remove(&id);
    shared.broadcast(id, &ServerMessage::PeerLeft(id));
    log::info!("{} left", info.nick);

    result
}

async fn handshake(recv: &mut quinn::RecvStream, reader: &mut FrameReader) -> Result<ClientHello, Error> {
    let hello = read_message::<Hello>(recv, reader)
        .await?
        .ok_or_else(|| fail("closed before the handshake"))?;

    hello.check(Service::LiveShare).map_err(fail)?;
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
    use super::*;

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
