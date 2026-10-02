use std::{collections::BTreeMap, net::SocketAddr, time::Duration};

use protocol::coop::PeerId;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ServerStats {
    pub peers: Vec<PeerStats>,
    pub received: Traffic,
    pub sent: Traffic,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PeerStats {
    pub id: PeerId,
    pub nick: String,
    pub addr: SocketAddr,
    pub rtt: Duration,
    pub sent: Transport,
    pub received: Transport,
    pub lost_packets: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Transport {
    pub bytes: u64,
    pub packets: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Count {
    pub messages: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Traffic(pub BTreeMap<&'static str, Count>);

impl Traffic {
    pub fn record(&mut self, kind: &'static str, bytes: u64) {
        let count = self.0.entry(kind).or_default();
        count.messages += 1;
        count.bytes += bytes;
    }
}
