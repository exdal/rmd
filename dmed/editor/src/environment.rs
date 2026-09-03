use core::{arena::StrArena, location::FileId};
use std::path::{Path, PathBuf};

use dmi::error::IconError;
use preprocessor::error::PreprocessError;
use sema::error::SemaError;

use crate::{LoadError, ObjectTree};

#[derive(Debug, Default)]
pub struct LoadDiagnostics {
    pub preprocess: Vec<PreprocessError>,
    pub sema: Vec<SemaError>,
    pub icons: Vec<(String, IconError)>,
}

impl LoadDiagnostics {
    pub fn is_empty(&self) -> bool { self.preprocess.is_empty() && self.sema.is_empty() && self.icons.is_empty() }

    pub fn len(&self) -> usize { self.preprocess.len() + self.sema.len() + self.icons.len() }
}

pub(crate) fn is_map(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("dmm"))
}

pub(crate) fn compile(entry: &Path) -> Result<(ObjectTree, Compiled), LoadError> {
    let arena = StrArena::new();
    let preprocessed = preprocessor::preprocess(&arena, entry)?;
    let ast = ast::parse(&preprocessed.tokens)?;
    let (tree, sema_errors) = sema::analyze(&ast);

    let root = preprocessed
        .entry
        .and_then(|id| preprocessed.sources.path(id))
        .map(Path::to_path_buf)
        .unwrap_or_else(|| entry.to_path_buf());

    let files = (0..preprocessed.sources.len())
        .map(|i| {
            preprocessed
                .sources
                .path(FileId(i as u32))
                .map(Path::to_path_buf)
                .unwrap_or_default()
        })
        .collect();

    let maps = preprocessed
        .resources
        .iter()
        .filter(|path| is_map(path))
        .cloned()
        .collect();

    Ok((
        tree,
        Compiled {
            root,
            files,
            maps,
            resource_dirs: preprocessed.resource_dirs,
            errors: preprocessed.errors,
            sema_errors,
        },
    ))
}

pub(crate) struct Compiled {
    pub root: PathBuf,
    pub files: Vec<PathBuf>,
    pub maps: Vec<PathBuf>,
    pub resource_dirs: Vec<PathBuf>,
    pub errors: Vec<PreprocessError>,
    pub sema_errors: Vec<SemaError>,
}
