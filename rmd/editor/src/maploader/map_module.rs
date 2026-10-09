use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use dmm::{Coord, Map};
use objtree::ObjectTree;

use crate::maploader::{Diagnostic, ErrorKind, resolve, resolved_path};

#[derive(Debug, Default)]
pub struct Discovery {
    pub roots: Vec<Root>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootLocation {
    pub source_map: PathBuf,
    pub coord: Coord,
    pub prefab_index: usize,
    pub config_file: Option<PathBuf>,
    pub key: Option<String>,
}

#[derive(Debug)]
pub struct Root {
    pub location: RootLocation,
    pub variants: Vec<Variant>,
    pub error: Option<Diagnostic>,
}

#[derive(Debug)]
pub struct Variant {
    pub path: PathBuf,
    pub module: Result<MapModule, Diagnostic>,
}

#[derive(Debug)]
pub struct MapModule {
    pub map: Arc<Map>,
    pub connector: Coord,
    pub translation: [i64; 3],
    pub roots: Vec<Root>,
}

pub fn discover(codebase: &Path, tree: &ObjectTree, source_map: &Path, map: &Map, loader: Option<&str>) -> Discovery {
    let Some(name) = loader else {
        return Discovery::default();
    };

    match resolve(name) {
        Some(loader) => loader.discover_modules(codebase, tree, source_map, map),
        None => Discovery {
            roots: Vec::new(),
            diagnostics: vec![Diagnostic {
                root: None,
                module_file: None,
                chain: vec![resolved_path(&codebase.join(source_map))],
                kind: ErrorKind::UnsupportedLoader(name.to_owned()),
            }],
        },
    }
}
