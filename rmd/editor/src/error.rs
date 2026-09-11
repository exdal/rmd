use core::{location::FileId, source::SourceMap};
use std::path::{Path, PathBuf};

use ast::error::ParseError;
use preprocessor::error::PreprocessError;

use crate::environment;

#[derive(Debug)]
pub enum LoadError {
    Preprocess(PreprocessError),
    Parse { error: ParseError, file: Option<PathBuf> },
}

impl LoadError {
    pub fn parse(error: ParseError, sources: &SourceMap<'_>, entry: Option<FileId>, path: &Path) -> Self {
        let root = environment::source_root(sources, entry, path);
        let file = sources
            .path(error.location.file)
            .map(|file| file.strip_prefix(root).unwrap_or(file).to_path_buf());

        Self::Parse { error, file }
    }
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Preprocess(e) => write!(f, "{e}"),
            Self::Parse { error, file } => {
                write!(
                    f,
                    "Parser error at {}: {}",
                    error.location.display(file.as_deref()),
                    error.kind
                )
            },
        }
    }
}

impl std::error::Error for LoadError {}

impl From<PreprocessError> for LoadError {
    fn from(e: PreprocessError) -> Self { Self::Preprocess(e) }
}
