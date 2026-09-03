use std::path::{Path, PathBuf};

use crate::{
    arena::StrArena,
    location::{FileId, Position},
};

pub struct SourceFile<'a> {
    pub path: PathBuf,
    pub contents: &'a str,
    /// Byte offset of the start of every line, for turning offsets back into positions.
    line_starts: Vec<usize>,
}

impl<'a> SourceFile<'a> {
    fn new(path: PathBuf, contents: &'a str) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(contents.match_indices('\n').map(|(i, _)| i + 1));

        Self {
            path,
            contents,
            line_starts,
        }
    }

    pub fn position_of(&self, offset: usize) -> Position {
        let line = self
            .line_starts
            .partition_point(|&start| start <= offset)
            .saturating_sub(1);
        let col = offset - self.line_starts[line];

        Position::new(line + 1, col + 1)
    }

    /// One-based, to match [`Position::line`].
    pub fn line(&self, line: usize) -> Option<&'a str> {
        let start = *self.line_starts.get(line.checked_sub(1)?)?;
        let end = self.line_starts.get(line).copied().unwrap_or(self.contents.len());

        Some(self.contents[start..end].trim_end_matches(['\r', '\n']))
    }
}

/// Every file the compilation pulled in, addressed by [`FileId`].
///
/// File contents live in a [`StrArena`] so that tokens can borrow them for the whole run - see the
/// note on `StrArena` for why.
#[derive(Default)]
pub struct SourceMap<'a> {
    files: Vec<SourceFile<'a>>,
}

impl<'a> SourceMap<'a> {
    pub fn new() -> Self { Self { files: Vec::new() } }

    pub fn add(&mut self, arena: &'a StrArena, path: impl Into<PathBuf>, contents: String) -> FileId {
        let id = FileId(self.files.len() as u32);
        self.files.push(SourceFile::new(path.into(), arena.alloc(contents)));

        id
    }

    pub fn load(&mut self, arena: &'a StrArena, path: impl AsRef<Path>) -> std::io::Result<FileId> {
        let path = path.as_ref();
        let contents = std::fs::read_to_string(path)?;

        Ok(self.add(arena, path, contents))
    }

    pub fn get(&self, id: FileId) -> Option<&SourceFile<'a>> { self.files.get(id.0 as usize) }

    pub fn contents(&self, id: FileId) -> Option<&'a str> { self.get(id).map(|f| f.contents) }

    pub fn path(&self, id: FileId) -> Option<&Path> { self.get(id).map(|f| f.path.as_path()) }

    pub fn find(&self, path: impl AsRef<Path>) -> Option<FileId> {
        let path = path.as_ref();
        self.files.iter().position(|f| f.path == path).map(|i| FileId(i as u32))
    }

    pub fn len(&self) -> usize { self.files.len() }

    pub fn is_empty(&self) -> bool { self.files.is_empty() }
}
