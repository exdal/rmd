use std::{
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::mpsc,
    time::Duration,
};

use bytes::Bytes;
use protocol::{
    FrameReader,
    Hello,
    Service,
    live::{ClientHello, ClientMessage, CodebaseId, Cursor, Datagram, PeerId, PeerInfo, Relayed, ServerMessage},
};
use quinn::{Connection, Endpoint};
use tokio::sync::{oneshot, watch};

use crate::{
    Error,
    fail,
    stream::{read_message, write_message},
};

const CURSOR_INTERVAL: Duration = Duration::from_millis(50);
const CLOSE_GRACE: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Connected { you: PeerId, peers: Vec<PeerInfo> },
    Rejected(String),
    PeerJoined(PeerInfo),
    PeerLeft(PeerId),
    Cursor { from: PeerId, cursor: Option<Cursor> },
    Disconnected(String),
}

pub struct Client {
    events: mpsc::Receiver<Event>,
    cursor: watch::Sender<Option<Cursor>>,
    shutdown: Option<oneshot::Sender<()>>,
}

impl Client {
    pub fn connect(addr: String, password: String, nick: String, codebase: CodebaseId) -> Self {
        let (events_tx, events) = mpsc::channel();
        let (cursor, cursor_rx) = watch::channel(None);
        let (shutdown, stop) = oneshot::channel();
        let hello = ClientHello {
            nick,
            password,
            codebase,
        };

        let failed = events_tx.clone();
        let spawned = std::thread::Builder::new()
            .name(String::from("rmd-live"))
            .spawn(move || {
                let result = crate::runtime()
                    .and_then(|runtime| runtime.block_on(run(&addr, hello, cursor_rx, &events_tx, stop)));

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
            shutdown: Some(shutdown),
        }
    }

    pub fn poll(&self) -> impl Iterator<Item = Event> + '_ { self.events.try_iter() }

    pub fn send_cursor(&self, cursor: Option<Cursor>) {
        self.cursor.send_if_modified(|current| {
            let changed = *current != cursor;
            *current = cursor;

            changed
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
    addr: &str, hello: ClientHello, cursor: watch::Receiver<Option<Cursor>>, events: &mpsc::Sender<Event>,
    mut stop: oneshot::Receiver<()>,
) -> Result<(), Error> {
    let remote = tokio::net::lookup_host(addr)
        .await
        .map_err(|e| fail(format!("could not resolve {addr}: {e}")))?
        .next()
        .ok_or_else(|| fail(format!("could not resolve {addr}")))?;

    let bind = match remote {
        SocketAddr::V4(_) => SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)),
        SocketAddr::V6(_) => SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0)),
    };

    let mut endpoint = Endpoint::client(bind).map_err(fail)?;
    endpoint.set_default_client_config(crate::tls::client_config()?);

    let connecting = endpoint.connect(remote, crate::tls::SERVER_NAME).map_err(fail)?;
    let connection = tokio::select! {
        connection = connecting => connection.map_err(fail)?,
        _ = &mut stop => return Ok(()),
    };

    let result = tokio::select! {
        result = session(&connection, hello, cursor, events) => result,
        _ = &mut stop => Ok(()),
    };

    connection.close(0u32.into(), b"");
    let _ = tokio::time::timeout(CLOSE_GRACE, endpoint.wait_idle()).await;

    result
}

async fn session(
    connection: &Connection, hello: ClientHello, mut cursor: watch::Receiver<Option<Cursor>>,
    events: &mpsc::Sender<Event>,
) -> Result<(), Error> {
    let emit = |event| {
        let _ = events.send(event);
    };

    let (mut send, mut recv) = connection.open_bi().await.map_err(fail)?;
    write_message(&mut send, &Hello::new(Service::LiveShare)).await?;
    write_message(&mut send, &ClientMessage::Hello(hello)).await?;

    let mut reader = FrameReader::new();
    match read_message(&mut recv, &mut reader).await? {
        Some(ServerMessage::Welcome { you, peers }) => emit(Event::Connected { you, peers }),
        Some(ServerMessage::Reject { reason }) => {
            emit(Event::Rejected(reason));
            return Ok(());
        },
        Some(_) => return Err(fail("the host skipped the handshake")),
        None => return Err(fail("the host closed the connection")),
    }

    let mut throttle = tokio::time::interval(CURSOR_INTERVAL);
    throttle.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            message = read_message(&mut recv, &mut reader) => match message? {
                Some(ServerMessage::PeerJoined(peer)) => emit(Event::PeerJoined(peer)),
                Some(ServerMessage::PeerLeft(id)) => emit(Event::PeerLeft(id)),
                Some(other) => log::warn!("unexpected message from the host: {other:?}"),
                None => return Err(fail("the host ended the session")),
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
}
