use std::path::{Component, Path, PathBuf};

use base::{
    file_path,
    location::{FileId, Location},
    source::read_source,
};
use lexer::{Lexer, token::Token};

use crate::{IncludeState, Preprocessor, diagnostic::Code};

impl<'a> Preprocessor<'a> {
    pub(super) fn directive_include(&mut self, location: Location) {
        let Some(raw) = self.next_significant() else {
            return;
        };
        let token = raw.token;
        let token_location = raw.location;

        let (Token::StringLiteral(raw) | Token::RawStringLiteral(raw)) = token else {
            self.diagnose(
                Code::InvalidInclusion,
                token_location,
                format!("'{token}' is not a valid include path"),
            );
            return;
        };

        let path = self.resolve_include(raw);
        self.include_file(&path, location);
    }

    /// `#include "dir\\file.dm"`
    fn resolve_include(&self, raw: &str) -> PathBuf {
        let base = self
            .include_stack
            .last()
            .map_or(Path::new(""), |state| state.dir.as_path());
        base.join(file_path::forward_slashes(raw).as_ref())
    }

    pub(super) fn include_file(&mut self, path: &Path, location: Location) {
        let path = normalize(path);

        if self.included.contains(&path) {
            self.diagnose(
                Code::FileAlreadyIncluded,
                location,
                format!("file \"{}\" was already included", path.display()),
            );
            return;
        }

        let extension = path.extension().and_then(|e| e.to_str());
        if extension.is_some_and(|e| e.eq_ignore_ascii_case("dms")) {
            return;
        }

        let is_resource =
            extension.is_some_and(|e| ["dmm", "dmp", "dmf", "dmi"].iter().any(|r| e.eq_ignore_ascii_case(r)));

        if is_resource {
            if !path.is_file() {
                self.include_ignoring_case(&path, location);
                return;
            }

            self.included.insert(path.clone());
            self.resources.push(path);
            return;
        }

        let contents = match self.source_cache.files.get(&path).copied() {
            Some(contents) => contents,
            None => match read_source(&path) {
                Ok(contents) => {
                    let contents = self.arena.alloc(contents);
                    self.source_cache.files.insert(path.clone(), contents);
                    contents
                },
                Err(_) if !path.is_file() => {
                    self.include_ignoring_case(&path, location);
                    return;
                },
                Err(error) => {
                    self.included.insert(path.clone());
                    self.diagnose(
                        Code::MissingIncludedFile,
                        location,
                        format!("could not read \"{}\": {error}", path.display()),
                    );

                    return;
                },
            },
        };

        self.included.insert(path.clone());
        let file = self.sources.add_borrowed(&path, contents);

        if let Some(progress) = self.progress.as_mut()
            && !progress(&path)
        {
            self.aborted = true;

            return;
        }

        self.push_file(file, &path);
    }

    fn include_ignoring_case(&mut self, path: &Path, location: Location) {
        match file_path::find_ignoring_case(path) {
            Some(found) => self.include_file(&found, location),
            None => self.diagnose_missing(path, location),
        }
    }

    fn diagnose_missing(&mut self, path: &Path, location: Location) {
        self.diagnose(
            Code::MissingIncludedFile,
            location,
            format!("could not find included file \"{}\"", path.display()),
        );
    }

    pub(super) fn open_embedded(&mut self, name: &str, contents: &'static str) {
        let arena = self.arena;
        let file = self.sources.add(arena, name, contents.to_string());
        self.builtin_files.push(file);

        self.push_file(file, Path::new(""));
    }

    fn push_file(&mut self, file: FileId, path: &Path) {
        let contents = self.sources.contents(file).unwrap_or_default();
        let mut lexer = Lexer::with_file(contents, file);
        lexer.emit_suppressed_newlines();

        // `#include "inner.dm"`
        lexer.keep_indents_at_eof();

        if let Some(state) = self.include_stack.last() {
            lexer.restore_indent(state.lexer.indent_state().without_errors());

            // `new /datum/tgs_version(\n #include "version.dm"\n)`
            if state.lexer.layout_suppressed() {
                lexer.disable_layout();
            }
        }

        self.include_stack.push(IncludeState {
            lexer,
            dir: path.parent().map(Path::to_path_buf).unwrap_or_default(),
            peeked: None,
        });
    }
}

/// `./dir/../file.dm`
pub(super) fn normalize(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };

    let mut out = PathBuf::with_capacity(absolute.as_os_str().len());
    for component in absolute.components() {
        match component {
            Component::CurDir => {},
            Component::ParentDir => {
                out.pop();
            },
            component => out.push(component),
        }
    }

    out
}
