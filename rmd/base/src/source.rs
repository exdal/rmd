use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

use crate::{
    arena::StrArena,
    location::{FileId, Position},
};

pub fn read_source(path: impl AsRef<Path>) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;

    Ok(match String::from_utf8(bytes) {
        Ok(contents) => contents,
        Err(error) => error.into_bytes().into_iter().map(char::from).collect::<String>(),
    })
}

pub struct SourceFile<'a> {
    pub path: PathBuf,
    pub contents: &'a str,
    line_starts: OnceLock<Vec<usize>>,
}

impl<'a> SourceFile<'a> {
    fn new(path: PathBuf, contents: &'a str) -> Self {
        Self {
            path,
            contents,
            line_starts: OnceLock::new(),
        }
    }

    fn line_starts(&self) -> &[usize] {
        self.line_starts.get_or_init(|| {
            let mut line_starts = vec![0];
            line_starts.extend(self.contents.match_indices('\n').map(|(i, _)| i + 1));

            line_starts
        })
    }

    pub fn position_of(&self, offset: usize) -> Position {
        let line_starts = self.line_starts();
        let line = line_starts.partition_point(|&start| start <= offset).saturating_sub(1);
        let col = offset - line_starts[line];

        Position::new(line + 1, col + 1)
    }

    /// One-based, to match [`Position::line`].
    pub fn line(&self, line: usize) -> Option<&'a str> {
        let line_starts = self.line_starts();
        let start = *line_starts.get(line.checked_sub(1)?)?;
        let end = line_starts.get(line).copied().unwrap_or(self.contents.len());

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
    pub fn new() -> Self {
        Self { files: Vec::new() }
    }

    pub fn add(&mut self, arena: &'a StrArena, path: impl Into<PathBuf>, contents: String) -> FileId {
        self.add_borrowed(path, arena.alloc(contents))
    }

    pub fn add_borrowed(&mut self, path: impl Into<PathBuf>, contents: &'a str) -> FileId {
        let id = FileId(self.files.len() as u32);
        self.files.push(SourceFile::new(path.into(), contents));

        id
    }

    pub fn load(&mut self, arena: &'a StrArena, path: impl AsRef<Path>) -> std::io::Result<FileId> {
        let path = path.as_ref();
        let contents = read_source(path)?;

        Ok(self.add(arena, path, contents))
    }

    pub fn get(&self, id: FileId) -> Option<&SourceFile<'a>> {
        self.files.get(id.0 as usize)
    }

    pub fn contents(&self, id: FileId) -> Option<&'a str> {
        self.get(id).map(|f| f.contents)
    }

    pub fn path(&self, id: FileId) -> Option<&Path> {
        self.get(id).map(|f| f.path.as_path())
    }

    pub fn find(&self, path: impl AsRef<Path>) -> Option<FileId> {
        let path = path.as_ref();
        self.files.iter().position(|f| f.path == path).map(|i| FileId(i as u32))
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::ErrorKind,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use super::*;

    fn with_source(contents: &[u8], check: impl FnOnce(&Path)) {
        static NEXT: AtomicUsize = AtomicUsize::new(0);

        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("rmd-read-source-{}-{id}", std::process::id()));
        fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("source.dm");
        fs::write(&path, contents).expect("write source");

        check(&path);
        fs::remove_dir_all(&dir).expect("remove temp dir");
    }

    #[test]
    fn valid_utf8_source_is_preserved() {
        for contents in ["", "var/x = 1\r\n", "var/name = \"café 日本 🦀\"\n"] {
            with_source(contents.as_bytes(), |path| {
                assert_eq!(read_source(path).expect("read UTF-8 source"), contents);
            });
        }
    }

    #[test]
    fn invalid_utf8_source_is_decoded_as_strict_latin1() {
        with_source(b"// 90\xb0\r\nvar/name = \"caf\xe9\x92\xff\"\n", |path| {
            assert_eq!(
                read_source(path).expect("read Latin-1 source"),
                "// 90°\r\nvar/name = \"café\u{0092}ÿ\"\n"
            );
        });
    }

    #[test]
    fn latin1_fallback_decodes_the_entire_file() {
        with_source(b"// caf\xc3\xa9 90\xb0\n", |path| {
            assert_eq!(read_source(path).expect("read Latin-1 source"), "// cafÃ© 90°\n");
        });
    }

    #[test]
    fn source_map_loads_decoded_text() {
        with_source(b"// 90\xb0\r\nvar/name = \"caf\xe9\"\n", |path| {
            let arena = StrArena::new();
            let mut sources = SourceMap::new();
            let file = sources.load(&arena, path).expect("load Latin-1 source");

            assert_eq!(sources.path(file), Some(path));
            assert_eq!(sources.contents(file), Some("// 90°\r\nvar/name = \"café\"\n"));
            let source = sources.get(file).expect("source file");
            assert_eq!(source.line(1), Some("// 90°"));
            assert_eq!(source.line(2), Some("var/name = \"café\""));
            assert_eq!(source.position_of("// 90°\r\n".len()), Position::new(2, 1));
        });
    }

    #[test]
    fn missing_source_keeps_the_filesystem_error() {
        with_source(b"", |path| {
            let missing = path.with_file_name("missing.dm");
            assert_eq!(
                read_source(&missing).expect_err("missing source").kind(),
                ErrorKind::NotFound
            );

            let arena = StrArena::new();
            let mut sources = SourceMap::new();
            assert_eq!(
                sources.load(&arena, &missing).expect_err("missing source").kind(),
                ErrorKind::NotFound
            );
            assert!(sources.is_empty());
        });
    }
}
