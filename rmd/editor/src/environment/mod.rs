pub(crate) mod compile;
pub(crate) mod error;
mod fingerprint;
pub(crate) mod panel;
pub(crate) mod profile;
pub(crate) mod progress;
pub(crate) mod session;
pub(crate) mod worker;
pub(crate) mod world;

use base::{
    location::FileId,
    source::SourceMap,
    types::{Identifier, Value},
};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

use codegen::CodegenError;
use dmi::{IconFile, IconInfo, error::IconError, metadata::Metadata};
use net::CodebaseHash;
use objtree::ObjectTree;
use preprocessor::error::PreprocessError;
use sema::error::SemaError;

use crate::git::repo;

use compile::{BakeOptions, compile};
use error::LoadError;
use progress::{Progress, Stage};
use world::World;

pub struct BakeProgram {
    pub tree: ObjectTree,
    pub module: codegen::Module,
    pub profile: objtree::TypeId,
    pub icon_states: vm::IconStates,
    pub uses_external_libraries: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profiles {
    pub available: Vec<String>,
    pub default: String,
    pub active: String,
}

pub struct Environment {
    pub root: PathBuf,
    /// The compatibility view used by the editor and static renderer.
    pub tree: ObjectTree,
    pub world: World,
    /// Runtime declarations and bytecode compiled without mapping compatibility defines.
    pub bake_program: Option<BakeProgram>,
    /// Profiles discovered in the runtime view, even when bytecode generation failed.
    pub profiles: Option<Profiles>,
    /// Runtime-view sources, retained even when bytecode generation fails.
    pub bake_files: Arc<[PathBuf]>,
    pub bake_options: BakeOptions,
    pub optimization_timings: ir::opt::OptimizationTimings,
    pub icons: HashMap<String, Metadata>,
    pub icon_files: Vec<PathBuf>,
    pub icon_info: HashMap<String, IconInfo>,
    pub maps: Vec<PathBuf>,
    pub files: Vec<PathBuf>,
    pub builtin_files: Vec<FileId>,
    /// `#define FILE_DIR "icons"`
    pub resource_dirs: Vec<PathBuf>,
    fingerprint: Option<CodebaseHash>,
    area_paths: OnceLock<Arc<HashSet<Vec<Identifier>>>>,
}

impl Environment {
    #[cfg(test)]
    pub fn new(root: impl Into<PathBuf>, tree: ObjectTree) -> Self {
        Self {
            root: root.into(),
            world: World::new(&tree),
            tree,
            bake_program: None,
            profiles: None,
            bake_files: Arc::default(),
            bake_options: BakeOptions::default(),
            optimization_timings: ir::opt::OptimizationTimings::default(),
            icons: HashMap::new(),
            icon_files: Vec::new(),
            icon_info: HashMap::new(),
            maps: Vec::new(),
            files: Vec::new(),
            builtin_files: Vec::new(),
            resource_dirs: Vec::new(),
            fingerprint: None,
            area_paths: OnceLock::new(),
        }
    }

    pub fn area_paths(&self) -> Arc<HashSet<Vec<Identifier>>> {
        Arc::clone(self.area_paths.get_or_init(|| {
            let paths = self.tree.roots().area.map(|area| {
                self.tree
                    .descendants(area)
                    .into_iter()
                    .filter_map(|id| self.tree.get(id))
                    .map(|decl| decl.path.segments.clone())
                    .collect()
            });

            Arc::new(paths.unwrap_or_default())
        }))
    }

    #[cfg(test)]
    pub fn load(entry: impl AsRef<Path>) -> Result<(Self, LoadDiagnostics), LoadError> {
        Self::load_with(entry, BakeOptions::default(), &Progress::new())
    }

    pub fn load_with(
        entry: impl AsRef<Path>, options: BakeOptions, progress: &Progress,
    ) -> Result<(Self, LoadDiagnostics), LoadError> {
        let entry = entry.as_ref();
        let compiled = compile(entry, &options, progress)?;

        let mut environment = Self {
            root: compiled.root,
            world: World::new(&compiled.tree),
            tree: compiled.tree,
            bake_program: compiled.bake_program,
            profiles: compiled.profiles,
            bake_files: compiled.bake_files,
            bake_options: options,
            optimization_timings: compiled.optimization_timings,
            icons: HashMap::new(),
            icon_files: Vec::new(),
            icon_info: HashMap::new(),
            maps: compiled.maps,
            files: compiled.files,
            builtin_files: compiled.builtin_files,
            resource_dirs: compiled.resource_dirs,
            fingerprint: Some(compiled.fingerprint),
            area_paths: OnceLock::new(),
        };

        let search_dirs = environment.resource_dirs.clone();
        let icons = environment.load_icons(&search_dirs, progress);
        if progress.is_cancelled() {
            return Err(LoadError::Cancelled);
        }

        Ok((
            environment,
            LoadDiagnostics {
                preprocess: compiled.errors,
                sema: compiled.sema_errors,
                bake_preprocess: compiled.bake_errors,
                bake_sema: compiled.bake_sema_errors,
                codegen: compiled.codegen_error,
                profile: compiled.profile_error,
                icons,
                bake: Vec::new(),
            },
        ))
    }

    pub fn base_dir(&self) -> &Path {
        self.root.parent().unwrap_or(Path::new("."))
    }

    pub fn is_builtin_file(&self, id: FileId) -> bool {
        self.builtin_files.contains(&id)
    }

    pub fn file(&self, id: base::location::FileId) -> Option<&Path> {
        self.files.get(id.0 as usize).map(PathBuf::as_path)
    }

    pub fn bake_file(&self, id: base::location::FileId) -> Option<&Path> {
        self.bake_files.get(id.0 as usize).map(PathBuf::as_path)
    }

    pub fn relative_bake_file(&self, id: base::location::FileId) -> Option<&Path> {
        let path = self.bake_file(id)?;

        Some(path.strip_prefix(self.base_dir()).unwrap_or(path))
    }

    pub fn fault_report(&self, fault: &vm::Fault) -> String {
        match &self.bake_program {
            Some(program) => fault.format_fault_report(&program.module, |file| self.relative_bake_file(file)),
            None => fault.to_string(),
        }
    }

    /// `'icons/obj/items.dmi'`
    pub fn icon_paths(&self) -> BTreeSet<&str> {
        self.tree
            .iter()
            .flat_map(|decl| decl.vars.values())
            .map(|var| &var.value)
            .chain(
                self.bake_program
                    .iter()
                    .flat_map(|program| program.tree.iter())
                    .flat_map(|decl| decl.vars.values())
                    .map(|var| &var.value),
            )
            .chain(self.bake_program.iter().flat_map(|program| &program.module.constants))
            .fold(BTreeSet::new(), |mut paths, value| {
                collect_icon_paths(value, &mut paths);
                paths
            })
    }

    /// `#define FILE_DIR "icons"`
    pub fn load_icons(&mut self, search_dirs: &[PathBuf], progress: &Progress) -> Vec<(String, IconError)> {
        let names: Vec<String> = self.icon_paths().into_iter().map(String::from).collect();
        let base = self.base_dir().to_path_buf();
        let mut failures = Vec::new();

        progress.enter(Stage::Icons, names.len());
        for name in names {
            if progress.is_cancelled() {
                break;
            }
            progress.advance(&name);
            let found = std::iter::once(base.as_path())
                .chain(search_dirs.iter().map(PathBuf::as_path))
                .find_map(|dir| resolve_resource_path(dir, &name))
                .unwrap_or_else(|| base.join(&name));

            match IconFile::load_info(&found) {
                Ok(info) => {
                    self.icons.insert(name.clone(), info.metadata.clone());
                    self.icon_info.insert(name, info);
                    self.icon_files.push(found);
                },
                Err(e) => failures.push((name, e)),
            }
        }

        if let Some(program) = self.bake_program.as_mut() {
            program.icon_states = vm::IconStates::new(self.icons.iter().map(|(name, metadata)| {
                let states = metadata.states.iter().map(|state| state.name.clone()).collect();

                (name.clone(), states, [metadata.width, metadata.height])
            }));
        }

        failures
    }

    pub fn fingerprint(&self) -> std::io::Result<CodebaseHash> {
        self.fingerprint
            .ok_or_else(|| std::io::Error::other("the codebase was not loaded from disk"))
    }

    pub fn git_hint(&self) -> Option<String> {
        let head = repo::discover(&self.root)?.open().ok()?.head_ref()?;

        Some(head.short)
    }
}

/// `var/static/list/light_overlays = list("32" = 'icons/effects/light_32.dmi')`
fn collect_icon_paths<'a>(value: &'a Value, paths: &mut BTreeSet<&'a str>) {
    match value {
        Value::Resource(path) if path.to_ascii_lowercase().ends_with(".dmi") => {
            paths.insert(path.as_str());
        },
        Value::List(entries) => {
            for entry in entries {
                collect_icon_paths(&entry.key, paths);
                if let Some(value) = &entry.value {
                    collect_icon_paths(value, paths);
                }
            }
        },
        _ => {},
    }
}

fn resolve_resource_path(dir: &Path, relative: &str) -> Option<PathBuf> {
    let mut current = dir.to_path_buf();
    for component in relative.split('/').filter(|part| !part.is_empty()) {
        let entry = std::fs::read_dir(&current).ok()?.filter_map(Result::ok).find(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.eq_ignore_ascii_case(component))
        })?;

        current = entry.path();
    }

    current.is_file().then_some(current)
}

#[derive(Debug, Default)]
pub struct LoadDiagnostics {
    pub preprocess: Vec<PreprocessError>,
    pub sema: Vec<SemaError>,
    pub bake_preprocess: Vec<PreprocessError>,
    pub bake_sema: Vec<SemaError>,
    pub codegen: Option<CodegenError>,
    pub profile: Option<vm::bake::ProfileError>,
    pub icons: Vec<(String, IconError)>,
    pub bake: Vec<vm::Diagnostic>,
}

impl LoadDiagnostics {
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.preprocess.is_empty()
            && self.sema.is_empty()
            && self.bake_preprocess.is_empty()
            && self.bake_sema.is_empty()
            && self.codegen.is_none()
            && self.profile.is_none()
            && self.icons.is_empty()
            && self.bake.is_empty()
    }
}

pub(crate) fn is_map(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("dmm"))
}

pub(crate) fn source_root<'a>(sources: &'a SourceMap<'_>, entry: Option<FileId>, path: &'a Path) -> &'a Path {
    entry
        .and_then(|entry| sources.path(entry))
        .and_then(Path::parent)
        .or_else(|| path.parent())
        .unwrap_or_else(|| Path::new(""))
}

#[cfg(test)]
mod tests;
