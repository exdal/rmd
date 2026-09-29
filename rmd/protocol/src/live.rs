use serde::{Deserialize, Serialize};

pub const MAX_NICK_LEN: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PeerId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodebaseId {
    pub hash: [u8; 32],
    pub git_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerInfo {
    pub id: PeerId,
    pub nick: String,
    pub codebase: CodebaseId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientHello {
    pub nick: String,
    pub password: String,
    pub codebase: CodebaseId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClientMessage {
    Hello(ClientHello),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServerMessage {
    Welcome { you: PeerId, peers: Vec<PeerInfo> },
    Reject { reason: String },
    PeerJoined(PeerInfo),
    PeerLeft(PeerId),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cursor {
    /// `_maps/map_files/Station/station.dmm`
    pub map: String,
    pub z: u32,
    /// map pixels from the bottom left corner
    pub pos: [f32; 2],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Datagram {
    Cursor(Option<Cursor>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Relayed {
    pub from: PeerId,
    pub datagram: Datagram,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{decode, encode};

    fn round_trip<T: Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug>(value: T) {
        assert_eq!(decode::<T>(&encode(&value).unwrap()).unwrap(), value);
    }

    fn peer(id: u32) -> PeerInfo {
        PeerInfo {
            id: PeerId(id),
            nick: format!("peer{id}"),
            codebase: CodebaseId {
                hash: [id as u8; 32],
                git_hint: id.is_multiple_of(2).then(|| String::from("abc1234")),
            },
        }
    }

    #[test]
    fn control_messages_round_trip() {
        round_trip(ClientMessage::Hello(ClientHello {
            nick: String::from("mapper"),
            password: String::from("hunter2"),
            codebase: peer(1).codebase,
        }));
        round_trip(ServerMessage::Welcome {
            you: PeerId(2),
            peers: vec![peer(1), peer(2)],
        });
        round_trip(ServerMessage::Reject {
            reason: String::from("wrong password"),
        });
        round_trip(ServerMessage::PeerJoined(peer(3)));
        round_trip(ServerMessage::PeerLeft(PeerId(3)));
    }

    #[test]
    fn datagrams_round_trip() {
        let cursor = Cursor {
            map: String::from("_maps/map_files/Station/station.dmm"),
            z: 2,
            pos: [1234.5, -8.25],
        };

        round_trip(Datagram::Cursor(Some(cursor.clone())));
        round_trip(Datagram::Cursor(None));
        round_trip(Relayed {
            from: PeerId(7),
            datagram: Datagram::Cursor(Some(cursor)),
        });
    }

    #[test]
    fn a_cursor_fits_in_a_minimum_quic_datagram() {
        let relayed = Relayed {
            from: PeerId(u32::MAX),
            datagram: Datagram::Cursor(Some(Cursor {
                map: "m".repeat(200),
                z: u32::MAX,
                pos: [f32::MAX, f32::MIN],
            })),
        };

        // QUIC guarantees at least 1200 byte packets, minus header overhead
        assert!(encode(&relayed).unwrap().len() < 1100);
    }
}
