use core::location::Position;

use crate::Coord;

#[derive(Clone, Debug)]
pub enum MapErrorKind {
    InvalidKey(String),
    UnknownKey(String),
    RaggedGrid,
    ExpectedDictionary,
    ExpectedGrid,
    MalformedPrefab(String),
    UnterminatedString,
    UnexpectedEof,
    Expected(char),
    DuplicateKey(String),
    InconsistentKeyLength { expected: usize, found: usize },
    MalformedGridHeader,
    MalformedValue(String),
    OverlappingBlocks(Coord),
    IncompleteGrid(Coord),
}

impl std::fmt::Display for MapErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidKey(s) => write!(f, "invalid key '{s}'"),
            Self::UnknownKey(s) => write!(f, "grid references undefined key '{s}'"),
            Self::RaggedGrid => write!(f, "grid rows have differing widths"),
            Self::ExpectedDictionary => write!(f, "expected a dictionary entry"),
            Self::ExpectedGrid => write!(f, "expected a grid block"),
            Self::MalformedPrefab(s) => write!(f, "malformed prefab '{s}'"),
            Self::UnterminatedString => write!(f, "unterminated string"),
            Self::UnexpectedEof => write!(f, "unexpected end of file"),
            Self::Expected(c) => write!(f, "expected '{c}'"),
            Self::DuplicateKey(s) => write!(f, "key '{s}' is defined twice"),
            Self::InconsistentKeyLength { expected, found } => {
                write!(f, "expected a {expected} character key, found {found}")
            },
            Self::MalformedGridHeader => write!(f, "malformed grid block header"),
            Self::MalformedValue(s) => write!(f, "malformed value '{s}'"),
            Self::OverlappingBlocks(c) => write!(f, "two grid blocks cover ({},{},{})", c.x, c.y, c.z),
            Self::IncompleteGrid(c) => write!(f, "no grid block covers ({},{},{})", c.x, c.y, c.z),
        }
    }
}

#[derive(Clone, Debug)]
pub struct MapError {
    pub position: Position,
    pub kind: MapErrorKind,
}

impl MapError {
    pub fn new(kind: MapErrorKind, position: Position) -> Self { Self { position, kind } }
}

impl std::fmt::Display for MapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Map error at {}: {}", self.position, self.kind)
    }
}

impl std::error::Error for MapError {}
