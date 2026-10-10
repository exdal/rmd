use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

pub const MAX_NICK_LEN: usize = 32;
pub const MAX_COMMENT_LEN: usize = 500;
pub const MAX_MAP_LEN: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PeerId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CodebaseHash(pub [u8; 32]);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PasswordHash(pub [u8; 32]);

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
    pub password: PasswordHash,
    pub codebase: CodebaseId,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CommentId(pub u32);

impl CommentId {
    pub fn next(self) -> Self { Self(self.0 + 1) }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
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
    Unshare {
        path: String,
    },
}

impl ClientMessage {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Hello(_) => "hello",
            Self::Comment { .. } => "comment",
            Self::DeleteComment(_) => "delete comment",
            Self::Edit(_) => "edit",
            Self::Resync { .. } => "resync",
            Self::Unshare { .. } => "unshare",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapEdit {
    pub path: String,
    pub generation: GenerationId,
    pub coords: Vec<[u32; 3]>,
    /// the changed tiles as a one row map, in `coords` order
    pub patch: String,
    /// applied after `coords`
    pub level: Option<LevelOp>,
    /// the author's copy had every edit before this one, level changes from here on are rebased away
    pub seen: SeqId,
}

pub const MAX_LEVELS: u32 = 255;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LevelOp {
    Insert(InsertLevel),
    Delete(DeleteLevel),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InsertLevel {
    pub at: InsertAt,
    pub contents: LevelContents,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InsertAt {
    /// above the top, dropped when another append landed since `seen`, so racing appends add one level
    Top,
    /// the level here and the ones above it move up one
    Z(u32),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LevelContents {
    /// one tile in every cell, as a one tile map
    Fill(String),
    /// every cell as a one row map, x then y
    Tiles(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteLevel {
    pub z: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LevelChange {
    Appended(u32),
    Inserted(u32),
    Deleted(u32),
}

impl LevelChange {
    /// where level `z` ends up, none when it was deleted
    pub fn level(self, z: u32) -> Option<u32> {
        match self {
            Self::Appended(inserted) | Self::Inserted(inserted) => Some(if z >= inserted { z + 1 } else { z }),
            Self::Deleted(deleted) => match z.cmp(&deleted) {
                Ordering::Less => Some(z),
                Ordering::Equal => None,
                Ordering::Greater => Some(z - 1),
            },
        }
    }

    /// where a level inserted at `z` ends up
    pub fn slot(self, z: u32) -> u32 {
        match self {
            Self::Appended(inserted) | Self::Inserted(inserted) if z >= inserted => z + 1,
            Self::Deleted(deleted) if z > deleted => z - 1,
            _ => z,
        }
    }
}

// the relay and every copy run the same edits through this, so they agree on which level ops apply
#[derive(Debug, Clone, Default)]
pub struct LevelLog {
    changes: Vec<(SeqId, LevelChange)>,
}

impl LevelLog {
    pub fn changes(&self) -> impl Iterator<Item = (SeqId, LevelChange)> + '_ { self.changes.iter().copied() }

    fn since(&self, seen: SeqId) -> impl Iterator<Item = LevelChange> + '_ {
        self.changes
            .iter()
            .filter(move |(seq, _)| *seq >= seen)
            .map(|(_, change)| *change)
    }

    pub fn rebase(&self, seen: SeqId, z: u32) -> Option<u32> {
        self.since(seen).try_fold(z, |z, change| change.level(z))
    }

    pub fn rebase_slot(&self, seen: SeqId, z: u32) -> u32 { self.since(seen).fold(z, |z, change| change.slot(z)) }

    /// the change the op makes to a map of `levels` levels, none when it lost a race
    pub fn sequence(&mut self, levels: u32, seq: SeqId, seen: SeqId, op: &LevelOp) -> Option<LevelChange> {
        let change = match op {
            LevelOp::Insert(InsertLevel { at: InsertAt::Top, .. }) => {
                let is_beaten = self
                    .since(seen)
                    .any(|change| matches!(change, LevelChange::Appended(_)));
                (!is_beaten && levels < MAX_LEVELS).then_some(LevelChange::Appended(levels + 1))?
            },
            LevelOp::Insert(InsertLevel { at: InsertAt::Z(z), .. }) => {
                let z = self.rebase_slot(seen, *z);
                (levels < MAX_LEVELS && (1..=levels + 1).contains(&z)).then_some(LevelChange::Inserted(z))?
            },
            LevelOp::Delete(DeleteLevel { z }) => {
                let z = self
                    .rebase(seen, *z)
                    .filter(|z| levels > 1 && (1..=levels).contains(z))?;
                LevelChange::Deleted(z)
            },
        };

        self.changes.push((seq, change));

        Some(change)
    }
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
    MapUnshared {
        path: String,
        by: PeerId,
        generation: Option<GenerationId>,
    },
}

impl ServerMessage {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Welcome { .. } => "welcome",
            Self::Reject { .. } => "reject",
            Self::CodebaseMismatch { .. } => "codebase mismatch",
            Self::PeerJoined(_) => "peer joined",
            Self::PeerLeft(_) => "peer left",
            Self::Comment(_) => "comment",
            Self::CommentDeleted(_) => "comment deleted",
            Self::MapShared { .. } => "map shared",
            Self::Edit { .. } => "edit",
            Self::MapIncoming { .. } => "map incoming",
            Self::MapCancelled { .. } => "map cancelled",
            Self::MapUnshared { .. } => "map unshared",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Transfer {
    Map {
        path: String,
        generation: GenerationId,
        next_seq: SeqId,
        levels: u32,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tool {
    Place,
    Select,
    Node,
    BlockSelect,
    Delete,
    Replace,
    Fill,
    Comment,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cursor {
    /// `_maps/map_files/Station/station.dmm`
    pub map: String,
    pub z: u32,
    /// map pixels from the bottom left corner
    pub pos: [f32; 2],
    pub tool: Tool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct View {
    /// `_maps/map_files/Station/station.dmm`
    pub map: String,
    pub z: u32,
    /// map pixels from the bottom left corner
    pub center: [f32; 2],
    pub zoom: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectionMode {
    Full,
    Hollow { line_width: u32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Selection {
    /// `_maps/map_files/Station/station.dmm`
    pub map: String,
    pub z: u32,
    pub min: [u32; 2],
    pub max: [u32; 2],
    pub mode: SelectionMode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Datagram {
    Cursor(Option<Cursor>),
    View(Option<View>),
    Selection(Option<Selection>),
}

impl Datagram {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Cursor(_) => "cursor",
            Self::View(_) => "view",
            Self::Selection(_) => "selection",
        }
    }
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
            password: PasswordHash([1; 32]),
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
            levels: 2,
            len: 1234,
        });

        let edit = MapEdit {
            path: String::from("_maps/test.dmm"),
            generation: GenerationId(2),
            coords: vec![[1, 2, 1], [3, 4, 1]],
            patch: String::from("\"a\" = (/turf,/area)\n"),
            level: Some(LevelOp::Delete(DeleteLevel { z: 1 })),
            seen: SeqId(5),
        };
        round_trip(ClientMessage::Edit(MapEdit {
            level: Some(LevelOp::Insert(InsertLevel {
                at: InsertAt::Z(2),
                contents: LevelContents::Tiles(String::from("\"a\" = (/turf,/area)\n")),
            })),
            ..edit.clone()
        }));
        round_trip(ClientMessage::Edit(edit.clone()));
        round_trip(ServerMessage::Edit {
            by: PeerId(1),
            seq: SeqId(7),
            edit,
        });
        round_trip(ClientMessage::Resync {
            path: String::from("_maps/test.dmm"),
        });
        round_trip(ClientMessage::Unshare {
            path: String::from("_maps/test.dmm"),
        });
        round_trip(ServerMessage::MapUnshared {
            path: String::from("_maps/test.dmm"),
            by: PeerId(1),
            generation: Some(GenerationId(3)),
        });
    }

    fn delete(z: u32) -> LevelOp { LevelOp::Delete(DeleteLevel { z }) }

    fn insert(at: InsertAt) -> LevelOp {
        LevelOp::Insert(InsertLevel {
            at,
            contents: LevelContents::Fill(String::new()),
        })
    }

    #[test]
    fn rebasing_follows_levels_through_deletions_and_insertions() {
        let mut log = LevelLog::default();
        assert_eq!(
            log.sequence(4, SeqId(2), SeqId(0), &delete(2)),
            Some(LevelChange::Deleted(2))
        );

        assert_eq!(log.rebase(SeqId(0), 1), Some(1));
        assert_eq!(log.rebase(SeqId(0), 2), None);
        assert_eq!(log.rebase(SeqId(0), 4), Some(3));
        assert_eq!(log.rebase(SeqId(3), 4), Some(4), "the author already had the deletion");

        assert_eq!(
            log.sequence(3, SeqId(3), SeqId(3), &insert(InsertAt::Z(1))),
            Some(LevelChange::Inserted(1))
        );
        assert_eq!(log.rebase(SeqId(3), 1), Some(2));
        assert_eq!(
            log.rebase(SeqId(0), 3),
            Some(3),
            "down past the deletion, up past the insertion"
        );
        assert_eq!(
            log.rebase_slot(SeqId(0), 2),
            3,
            "the deleted level's slot stays, then moves up past the insertion"
        );
        assert_eq!(log.rebase_slot(SeqId(0), 4), 4);
    }

    #[test]
    fn racing_deletions_of_one_level_delete_it_once() {
        let mut log = LevelLog::default();
        assert_eq!(
            log.sequence(3, SeqId(0), SeqId(0), &delete(2)),
            Some(LevelChange::Deleted(2))
        );
        assert_eq!(log.sequence(2, SeqId(1), SeqId(0), &delete(2)), None);
        assert_eq!(
            log.sequence(2, SeqId(2), SeqId(0), &delete(3)),
            Some(LevelChange::Deleted(2))
        );
        assert_eq!(
            log.changes().collect::<Vec<_>>(),
            [(SeqId(0), LevelChange::Deleted(2)), (SeqId(2), LevelChange::Deleted(2))]
        );
    }

    #[test]
    fn racing_deletions_keep_the_last_level() {
        let mut log = LevelLog::default();
        assert_eq!(
            log.sequence(2, SeqId(0), SeqId(0), &delete(1)),
            Some(LevelChange::Deleted(1))
        );
        assert_eq!(log.sequence(1, SeqId(1), SeqId(0), &delete(2)), None);
        assert_eq!(log.sequence(1, SeqId(2), SeqId(1), &delete(1)), None);
    }

    #[test]
    fn racing_appends_add_one_level_but_insertions_always_apply() {
        let mut log = LevelLog::default();
        assert_eq!(
            log.sequence(3, SeqId(0), SeqId(0), &delete(1)),
            Some(LevelChange::Deleted(1))
        );
        assert_eq!(
            log.sequence(2, SeqId(1), SeqId(0), &insert(InsertAt::Top)),
            Some(LevelChange::Appended(3)),
            "a deletion doesn't beat an append"
        );
        assert_eq!(log.sequence(3, SeqId(2), SeqId(0), &insert(InsertAt::Top)), None);
        assert_eq!(
            log.sequence(3, SeqId(3), SeqId(0), &insert(InsertAt::Z(2))),
            Some(LevelChange::Inserted(1)),
            "the insertion follows its slot down past the deletion"
        );
        assert_eq!(
            log.sequence(4, SeqId(4), SeqId(2), &insert(InsertAt::Top)),
            Some(LevelChange::Appended(5)),
            "an insertion doesn't beat an append"
        );
    }

    #[test]
    fn insertions_stop_at_the_level_limit() {
        let mut log = LevelLog::default();
        assert_eq!(
            log.sequence(MAX_LEVELS, SeqId(0), SeqId(0), &insert(InsertAt::Top)),
            None
        );
        assert_eq!(
            log.sequence(MAX_LEVELS, SeqId(0), SeqId(0), &insert(InsertAt::Z(1))),
            None
        );
        assert_eq!(log.sequence(3, SeqId(0), SeqId(0), &insert(InsertAt::Z(5))), None);
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
            tool: Tool::Fill,
        };

        round_trip(Datagram::Cursor(Some(cursor.clone())));
        round_trip(Datagram::Cursor(None));
        round_trip(Datagram::View(Some(View {
            map: cursor.map.clone(),
            z: 3,
            center: [512.0, 96.5],
            zoom: 1.5,
        })));
        round_trip(Datagram::View(None));
        round_trip(Datagram::Selection(Some(Selection {
            map: cursor.map.clone(),
            z: 2,
            min: [3, 4],
            max: [10, 12],
            mode: SelectionMode::Hollow { line_width: 2 },
        })));
        round_trip(Datagram::Selection(None));
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
                tool: Tool::BlockSelect,
            })),
        };

        let view = Relayed {
            from: PeerId(u32::MAX),
            datagram: Datagram::View(Some(View {
                map: "m".repeat(200),
                z: u32::MAX,
                center: [f32::MAX, f32::MIN],
                zoom: f32::MAX,
            })),
        };

        // QUIC guarantees at least 1200 byte packets, minus header overhead
        assert!(encode(&relayed).unwrap().len() < 1100);
        assert!(encode(&view).unwrap().len() < 1100);
    }
}
