pub mod error;
pub mod module;
pub mod preview;

use std::{
    fs,
    path::{Path, PathBuf},
};

pub use error::{Diagnostic, ErrorKind};
pub use module::{LoadedModule, ModuleFiles, RootSpecs, resolve_module_path, root_specs};

pub const MAX_NESTING: usize = 128;

pub(crate) fn resolved_path(path: &Path) -> PathBuf { fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()) }
