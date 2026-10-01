use serde::{Deserialize, Serialize};

pub const MAX_NICK_LEN: usize = 32;
pub const MAX_COMMENT_LEN: usize = 500;
pub const MAX_MAP_LEN: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PeerId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CodebaseHash(pub [u8; 32]);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodebaseId {
    pub hash: CodebaseHash,
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CommentId(pub u32);

impl CommentId {
    pub fn next(self) -> Self { Self(self.0 + 1) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GenerationId(pub u32);

impl GenerationId {
    pub fn next(self) -> Self { Self(self.0 + 1) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SeqId(pub u32);

impl SeqId {
    pub fn next(self) -> Self { Self(self.0 + 1) }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comment {
    pub id: CommentId,
    pub author: PeerId,
    pub map: String,
    pub z: u32,
    pub pos: [f32; 2],
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ClientMessage {
    Hello(ClientHello),
    Comment {
        map: String,
        z: u32,
        pos: [f32; 2],
        text: String,
    },
    DeleteComment(CommentId),
    Edit(MapEdit),
    Resync {
        path: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapEdit {
    pub path: String,
    pub generation: GenerationId,
    pub coords: Vec<[u32; 3]>,
    /// the changed tiles as a one row map, in `coords` order
    pub patch: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ServerMessage {
    Welcome {
        you: PeerId,
        codebase: CodebaseId,
        peers: Vec<PeerInfo>,
        comments: Vec<Comment>,
    },
    Reject {
        reason: String,
    },
    CodebaseMismatch {
        expected: CodebaseId,
    },
    PeerJoined(PeerInfo),
    PeerLeft(PeerId),
    Comment(Comment),
    CommentDeleted(CommentId),
    MapShared {
        path: String,
        by: PeerId,
        generation: GenerationId,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Transfer {
    Map {
        path: String,
        generation: GenerationId,
        next_seq: SeqId,
        len: u64,
    },
}

/// `_maps/map_files/Station/station.dmm`
pub fn is_map_path(path: &str) -> bool {
    let segments_ok = path
        .split('/')
        .all(|segment| !segment.is_empty() && segment != "." && segment != ".." && !segment.contains(['\\', ':']));

    segments_ok && path.to_ascii_lowercase().ends_with(".dmm")
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
    use std::fmt::Debug;

    use serde::de::DeserializeOwned;

    use super::*;
    use crate::{decode, encode};

    fn round_trip<T: Serialize + DeserializeOwned + PartialEq + Debug>(value: T) {
        assert_eq!(decode::<T>(&encode(&value).unwrap()).unwrap(), value);
    }

    fn peer(id: u32) -> PeerInfo {
        PeerInfo {
            id: PeerId(id),
            nick: format!("peer{id}"),
            codebase: CodebaseId {
                hash: CodebaseHash([id as u8; 32]),
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
        let comment = Comment {
            id: CommentId(4),
            author: PeerId(1),
            map: String::from("_maps/test.dmm"),
            z: 1,
            pos: [16.0, 48.5],
            text: String::from("needs more firelocks"),
        };
        round_trip(ServerMessage::Welcome {
            you: PeerId(2),
            codebase: peer(1).codebase,
            peers: vec![peer(1), peer(2)],
            comments: vec![comment.clone()],
        });
        round_trip(ClientMessage::Comment {
            map: comment.map.clone(),
            z: comment.z,
            pos: comment.pos,
            text: comment.text.clone(),
        });
        round_trip(ClientMessage::DeleteComment(comment.id));
        round_trip(ServerMessage::CommentDeleted(comment.id));
        round_trip(ServerMessage::Comment(comment));
        round_trip(ServerMessage::Reject {
            reason: String::from("wrong password"),
        });
        round_trip(ServerMessage::CodebaseMismatch {
            expected: peer(1).codebase,
        });
        round_trip(ServerMessage::PeerJoined(peer(3)));
        round_trip(ServerMessage::PeerLeft(PeerId(3)));
        round_trip(ServerMessage::MapShared {
            path: String::from("_maps/test.dmm"),
            by: PeerId(1),
            generation: GenerationId(2),
        });
        round_trip(ServerMessage::MapIncoming {
            path: String::from("_maps/test.dmm"),
            by: PeerId(1),
            len: 6 * 1024 * 1024,
        });
        round_trip(ServerMessage::MapCancelled {
            path: String::from("_maps/test.dmm"),
            by: PeerId(1),
        });
        round_trip(Transfer::Map {
            path: String::from("_maps/test.dmm"),
            generation: GenerationId(2),
            next_seq: SeqId(3),
            len: 1234,
        });

        let edit = MapEdit {
            path: String::from("_maps/test.dmm"),
            generation: GenerationId(2),
            coords: vec![[1, 2, 1], [3, 4, 1]],
            patch: String::from("\"a\" = (/turf,/area)\n"),
        };
        round_trip(ClientMessage::Edit(edit.clone()));
        round_trip(ServerMessage::Edit {
            by: PeerId(1),
            seq: SeqId(7),
            edit,
        });
        round_trip(ClientMessage::Resync {
            path: String::from("_maps/test.dmm"),
        });
    }

    #[test]
    fn map_paths_stay_inside_the_codebase() {
        assert!(is_map_path("_maps/map_files/Station/station.dmm"));
        assert!(is_map_path("Box.DMM"));

        for path in [
            "",
            "/etc/x.dmm",
            "../x.dmm",
            "_maps/../../x.dmm",
            "a//b.dmm",
            "./a.dmm",
            "C:/a.dmm",
            "a\\b.dmm",
            "a.dm",
        ] {
            assert!(!is_map_path(path), "{path}");
        }
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
