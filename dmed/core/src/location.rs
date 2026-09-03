use std::path::Path;

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileId(pub u32);

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub line: usize,
    pub col: usize,
}

impl Position {
    pub fn new(line: usize, col: usize) -> Self { Self { line, col } }
}

impl std::fmt::Display for Position {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{}:{}", self.line, self.col) }
}

#[derive(Default, Debug, Clone, Copy)]
pub struct Location {
    pub file: FileId,
    pub begin: Position,
    pub end: Position,
}

impl Location {
    pub fn new(begin: Position, end: Position) -> Self {
        Self {
            file: FileId::default(),
            begin,
            end,
        }
    }

    pub fn in_file(file: FileId, begin: Position, end: Position) -> Self { Self { file, begin, end } }

    pub fn with_file(self, file: FileId) -> Self { Self { file, ..self } }

    /// `code/game/objects/items.dm:22:1`, or `<file 3>:22:1` for a file the caller cannot name.
    pub fn display(self, path: Option<&Path>) -> LocationDisplay<'_> { LocationDisplay { location: self, path } }
}

impl std::fmt::Display for Location {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{}", self.begin) }
}

pub struct LocationDisplay<'a> {
    location: Location,
    path: Option<&'a Path>,
}

impl std::fmt::Display for LocationDisplay<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.path {
            Some(path) => write!(f, "{}:{}", path.display(), self.location.begin),
            None => write!(f, "<file {}>:{}", self.location.file.0, self.location.begin),
        }
    }
}
