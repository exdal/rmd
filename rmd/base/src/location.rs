use std::path::Path;

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileId(pub u32);

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    pub line: u32,
    pub col: u32,
}

impl Position {
    pub fn new(line: usize, col: usize) -> Self {
        Self {
            line: line as u32,
            col: col as u32,
        }
    }
}

impl std::fmt::Display for Position {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.line, self.col)
    }
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

    pub fn in_file(file: FileId, begin: Position, end: Position) -> Self {
        Self { file, begin, end }
    }

    pub fn with_file(self, file: FileId) -> Self {
        Self { file, ..self }
    }

    /// `code/game/objects/items.dm:22:1`, or `<file 3>:22:1` for a file the caller cannot name.
    pub fn display(self, path: Option<&Path>) -> LocationDisplay<'_> {
        LocationDisplay { location: self, path }
    }
}

impl std::fmt::Display for Location {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.begin)
    }
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

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceLine {
    pub file: FileId,
    pub line: u32,
}

impl SourceLine {
    pub fn of(location: Location) -> Self {
        Self {
            file: location.file,
            line: location.begin.line,
        }
    }

    pub fn location(self) -> Location {
        Location::in_file(
            self.file,
            Position {
                line: self.line,
                col: 1,
            },
            Position {
                line: self.line,
                col: 1,
            },
        )
    }
}

/// Lines keyed by ascending ids or offsets, stored only where the line changes
#[derive(Debug, Clone)]
pub struct LineTable<K> {
    runs: Vec<(K, Option<SourceLine>)>,
}

impl<K> Default for LineTable<K> {
    fn default() -> Self {
        Self { runs: Vec::new() }
    }
}

impl<K: Copy + Ord> LineTable<K> {
    pub fn push(&mut self, key: K, line: Option<SourceLine>) {
        if self.runs.last().is_some_and(|(_, last)| *last == line) {
            return;
        }

        if self.runs.last().is_some_and(|(last, _)| *last == key) {
            self.runs.pop();
            if self.runs.last().is_some_and(|(_, last)| *last == line) {
                return;
            }
        }

        self.runs.push((key, line));
    }

    pub fn get(&self, key: K) -> Option<SourceLine> {
        let index = self.runs.partition_point(|(start, _)| *start <= key);

        self.runs.get(index.checked_sub(1)?)?.1
    }

    pub fn get_near(&self, key: K, hint: &mut usize) -> Option<SourceLine> {
        const HINT_LOOKUP_DEPTH: usize = 3;

        let covers = |index: usize| {
            self.runs.get(index).is_some_and(|(start, _)| *start <= key)
                && self.runs.get(index + 1).is_none_or(|(next, _)| key < *next)
        };

        match (*hint..*hint + HINT_LOOKUP_DEPTH).find(|index| covers(*index)) {
            Some(index) => *hint = index,
            None => {
                let index = self.runs.partition_point(|(start, _)| *start <= key);
                *hint = index.checked_sub(1)?;
            },
        }

        self.runs.get(*hint)?.1
    }

    pub fn len(&self) -> usize {
        self.runs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }
}
