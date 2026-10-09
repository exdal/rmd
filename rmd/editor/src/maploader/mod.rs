// This shit goes completely against main point of the editor. In a perfect world this should have been handled
// through DM, but it might have massive consequences. So in my view: this is a job for plugins. When we have
// scripting/plugin support. TODO: move this into plugin system when we have it, OR if I spend more time on this, the
// entire maploader module can be moved into profiles. With some additional hooks.

pub mod error;
pub mod map_module;
pub mod tg;

use std::{
    fs,
    path::{Path, PathBuf},
};

use dmm::Map;
pub use error::{Diagnostic, ErrorKind};
use objtree::ObjectTree;

use crate::maploader::tg::TgMapLoader;

pub const MAX_NESTING: usize = 128;

pub(crate) trait MapLoader {
    fn discover_map_modules(
        &self, codebase: &Path, tree: &ObjectTree, source_map: &Path, map: &Map,
    ) -> map_module::Discovery;

    fn ignored_atom_paths() -> &'static [&'static str];
}

#[derive(Debug)]
pub enum MapLoaderKind {
    Tgstation(TgMapLoader),
}

impl MapLoaderKind {
    pub fn discover_modules(
        &self, codebase: &Path, tree: &ObjectTree, source_map: &Path, map: &Map,
    ) -> map_module::Discovery {
        match self {
            Self::Tgstation(loader) => loader.discover_map_modules(codebase, tree, source_map, map),
        }
    }

    pub fn ignored_atom_paths(&self) -> &'static [&'static str] {
        match self {
            Self::Tgstation(_) => TgMapLoader::ignored_atom_paths(),
        }
    }
}

pub fn resolve(name: &str) -> Option<MapLoaderKind> {
    match name {
        "tgstation" => Some(MapLoaderKind::Tgstation(TgMapLoader)),
        _ => None,
    }
}

pub(crate) fn resolved_path(path: &Path) -> PathBuf { fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()) }
