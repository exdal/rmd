use std::{
    collections::HashMap,
    io,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use bytes::Bytes;
use fastrand::Rng;
use tokio::{
    net::UdpSocket,
    sync::{mpsc, oneshot},
    time::{self, Instant},
};

use crate::{Error, fail, socket};

const MAX_DATAGRAM: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Impairment {
    pub latency: Duration,
    // each packet waits a random extra 0..jitter, but never overtakes the packet before it
    pub jitter: Duration,
    pub loss: f64,
    pub seed: u64,
}

impl Impairment {
    pub fn is_none(&self) -> bool { self.latency.is_zero() && self.jitter.is_zero() && self.loss <= 0.0 }
}

pub struct LossyProxy {
    addr: SocketAddr,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl LossyProxy {
    pub fn spawn(bind: SocketAddr, upstream: SocketAddr, impairment: Impairment) -> Result<Self, Error> {
        let runtime = crate::runtime()?;
        let front = socket::bind_udp(bind).map_err(fail)?;
        front.set_nonblocking(true).map_err(fail)?;
        let front = {
            let _context = runtime.enter();
            UdpSocket::from_std(front).map_err(fail)?
        };
        let addr = front.local_addr().map_err(fail)?;

        let (shutdown, stop) = oneshot::channel();
        let thread = thread::Builder::new()
            .name(String::from("rmd-proxy"))
            .spawn(move || {
                runtime.block_on(async move {
                    tokio::select! {
                        _ = stop => {},
                        _ = forward(Arc::new(front), upstream, impairment) => {},
                    }
                });
            })
            .map_err(fail)?;

        Ok(Self {
            addr,
            shutdown: Some(shutdown),
            thread: Some(thread),
        })
    }

    pub fn local_addr(&self) -> SocketAddr { self.addr }
}

impl Drop for LossyProxy {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }

        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Packet {
    due: Instant,
    bytes: Bytes,
    socket: Arc<UdpSocket>,
    to: Option<SocketAddr>,
}

// one direction of the link, delivering packets in the order they were pushed
struct Lane {
    impairment: Impairment,
    state: Mutex<(Rng, Instant)>,
    queue: mpsc::UnboundedSender<Packet>,
}

impl Lane {
    fn spawn(impairment: Impairment, seed: u64) -> Arc<Self> {
        let (queue, packets) = mpsc::unbounded_channel();
        tokio::spawn(deliver(packets));

        Arc::new(Self {
            impairment,
            state: Mutex::new((Rng::with_seed(seed), Instant::now())),
            queue,
        })
    }

    fn push(&self, bytes: &[u8], socket: &Arc<UdpSocket>, to: Option<SocketAddr>) {
        let mut state = self.state.lock().unwrap();
        let (rng, last) = &mut *state;
        if rng.f64() < self.impairment.loss {
            return;
        }

        let due = (Instant::now() + self.impairment.latency + self.impairment.jitter.mul_f64(rng.f64())).max(*last);
        *last = due;

        let _ = self.queue.send(Packet {
            due,
            bytes: Bytes::copy_from_slice(bytes),
            socket: socket.clone(),
            to,
        });
    }
}

async fn deliver(mut packets: mpsc::UnboundedReceiver<Packet>) {
    while let Some(packet) = packets.recv().await {
        time::sleep_until(packet.due).await;
        let _ = match packet.to {
            Some(to) => packet.socket.send_to(&packet.bytes, to).await,
            None => packet.socket.send(&packet.bytes).await,
        };
    }
}

async fn forward(front: Arc<UdpSocket>, upstream: SocketAddr, impairment: Impairment) {
    let up = Lane::spawn(impairment, impairment.seed);
    let down = Lane::spawn(impairment, impairment.seed.wrapping_add(1));

    // one upstream socket per client, so the server sees each client as its own peer
    let mut clients = HashMap::<SocketAddr, Arc<UdpSocket>>::new();
    let mut buf = vec![0; MAX_DATAGRAM];
    loop {
        let (len, client) = match front.recv_from(&mut buf).await {
            Ok(received) => received,
            Err(e) => {
                log::debug!("proxy receive failed: {e}");
                continue;
            },
        };

        let socket = match clients.get(&client) {
            Some(socket) => socket.clone(),
            None => {
                let socket = match connect(upstream).await {
                    Ok(socket) => Arc::new(socket),
                    Err(e) => {
                        log::warn!("proxy could not reach {upstream}: {e}");
                        continue;
                    },
                };

                tokio::spawn(reply(socket.clone(), front.clone(), client, down.clone()));
                clients.insert(client, socket.clone());

                socket
            },
        };

        up.push(&buf[..len], &socket, None);
    }
}

async fn reply(upstream: Arc<UdpSocket>, front: Arc<UdpSocket>, client: SocketAddr, down: Arc<Lane>) {
    let mut buf = vec![0; MAX_DATAGRAM];
    while let Ok(len) = upstream.recv(&mut buf).await {
        down.push(&buf[..len], &front, Some(client));
    }
}

async fn connect(upstream: SocketAddr) -> io::Result<UdpSocket> {
    let bind = match upstream {
        SocketAddr::V4(_) => SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)),
        SocketAddr::V6(_) => SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0)),
    };

    let socket = UdpSocket::bind(bind).await?;
    socket.connect(upstream).await?;

    Ok(socket)
}
