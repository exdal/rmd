pub(crate) mod error;
mod fingerprint;
pub(crate) mod panel;
pub(crate) mod progress;
pub(crate) mod session;
pub(crate) mod worker;
pub(crate) mod world;

use base::{
    arena::StrArena,
    location::FileId,
    path::TreePath,
    source::SourceMap,
    types::{Identifier, Value},
};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

use codegen::CodegenError;
use dmi::{IconFile, IconInfo, error::IconError, metadata::Metadata};
use net::CodebaseHash;
use objtree::{ObjectTree, TypeId};
use preprocessor::{
    PreludeFile, Preprocessor, SourceCache, Spanned, error::PreprocessError, macros_used_before_definition,
    prelude_files,
};
use sema::error::SemaError;

use crate::git::repo;

use error::LoadError;
use progress::{Progress, Stage};
use world::World;

pub struct BakeProgram {
    pub tree: ObjectTree,
    pub module: codegen::Module,
    pub profile: objtree::TypeId,
    pub icon_states: vm::IconStates,
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

    pub fn icon(&self, name: &str) -> Option<&Metadata> {
        self.icons.get(name)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BundledProfile {
    Beestation,
    Cmss13,
    Goonstation,
    Monkestation,
    Monkespaw,
    Novasector,
    Oculis,
    SecondCity,
    Tgmc,
    Tgstation,
    Vanderlin,
}

impl BundledProfile {
    pub const ALL: [Self; 11] = [
        Self::Beestation,
        Self::Cmss13,
        Self::Goonstation,
        Self::Monkestation,
        Self::Monkespaw,
        Self::Novasector,
        Self::Oculis,
        Self::SecondCity,
        Self::Tgmc,
        Self::Tgstation,
        Self::Vanderlin,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Beestation => "BeeStation",
            Self::Cmss13 => "CMSS13",
            Self::Goonstation => "Goonstation",
            Self::Monkestation => "Monkestation",
            Self::Monkespaw => "Monke’s Paw",
            Self::Novasector => "Nova Sector",
            Self::Oculis => "Oculis Station",
            Self::SecondCity => "SecondCity",
            Self::Tgmc => "TGMC",
            Self::Tgstation => "tgstation",
            Self::Vanderlin => "Vanderlin",
        }
    }

    pub const fn stem(self) -> &'static str {
        match self {
            Self::Beestation => "beestation",
            Self::Cmss13 => "cmss13",
            Self::Goonstation => "goonstation",
            Self::Monkestation => "monkestation",
            Self::Monkespaw => "monkespaw",
            Self::Novasector => "novasector",
            Self::Oculis => "oculis",
            Self::SecondCity => "secondcity",
            Self::Tgmc => "tgmc",
            Self::Tgstation => "tgstation",
            Self::Vanderlin => "vanderlin",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|profile| profile.stem() == name)
    }

    const fn source_name(self) -> &'static str {
        match self {
            Self::Beestation => "<bundled-profile-beestation.dm>",
            Self::Cmss13 => "<bundled-profile-cmss13.dm>",
            Self::Goonstation => "<bundled-profile-goonstation.dm>",
            Self::Monkestation => "<bundled-profile-monkestation.dm>",
            Self::Monkespaw => "<bundled-profile-monkespaw.dm>",
            Self::Novasector => "<bundled-profile-novasector.dm>",
            Self::Oculis => "<bundled-profile-oculis.dm>",
            Self::SecondCity => "<bundled-profile-secondcity.dm>",
            Self::Tgmc => "<bundled-profile-tgmc.dm>",
            Self::Tgstation => "<bundled-profile-tgstation.dm>",
            Self::Vanderlin => "<bundled-profile-vanderlin.dm>",
        }
    }

    const fn source(self) -> &'static str {
        match self {
            Self::Beestation => include_str!("../../profiles/beestation.dm"),
            Self::Cmss13 => include_str!("../../profiles/cmss13.dm"),
            Self::Goonstation => include_str!("../../profiles/goonstation.dm"),
            Self::Monkestation => include_str!("../../profiles/monkestation.dm"),
            Self::Monkespaw => include_str!("../../profiles/monkespaw.dm"),
            Self::Novasector => include_str!("../../profiles/novasector.dm"),
            Self::Oculis => include_str!("../../profiles/oculis.dm"),
            Self::SecondCity => include_str!("../../profiles/secondcity.dm"),
            Self::Tgmc => include_str!("../../profiles/tgmc.dm"),
            Self::Tgstation => include_str!("../../profiles/tgstation.dm"),
            Self::Vanderlin => include_str!("../../profiles/vanderlin.dm"),
        }
    }

    const fn defines(self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::Goonstation => Some((
                "<bundled-profile-goonstation-defines.dm>",
                "#define USE_PERSPECTIVE_EDITOR_WALLS\n",
            )),
            Self::Beestation => Some((
                "<bundled-profile-beestation-defines.dm>",
                "#ifdef __DEMIR_BAKE__\n#define CBT\n#endif\n",
            )),
            // COCK AND BALL TORTURE
            Self::Tgstation => Some((
                "<bundled-profile-tgstation-defines.dm>",
                "#ifdef __DEMIR_BAKE__\n#define CBT\n#endif\n",
            )),
            Self::SecondCity => Some((
                "<bundled-profile-secondcity-defines.dm>",
                "#ifdef __DEMIR_BAKE__\n#define CBT\n#endif\n",
            )),
            Self::Monkestation => Some((
                "<bundled-profile-monkestation-defines.dm>",
                "#ifdef __DEMIR_BAKE__\n#define CBT\n#endif\n",
            )),
            Self::Monkespaw => Some((
                "<bundled-profile-monkespaw-defines.dm>",
                "#ifdef __DEMIR_BAKE__\n#define CBT\n#endif\n",
            )),
            Self::Novasector => Some((
                "<bundled-profile-novasector-defines.dm>",
                "#ifdef __DEMIR_BAKE__\n#define CBT\n#endif\n",
            )),
            Self::Oculis => Some((
                "<bundled-profile-oculis-defines.dm>",
                "#ifdef __DEMIR_BAKE__\n#define CBT\n#endif\n",
            )),
            Self::Cmss13 | Self::Tgmc | Self::Vanderlin => None,
        }
    }
}

const PROFILE_EXTENSION: &str = ".dm";
const DEFINES_EXTENSION: &str = ".defines.dm";

pub fn write_missing_profiles(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for profile in BundledProfile::ALL {
        write_new(
            &dir.join(format!("{}{PROFILE_EXTENSION}", profile.stem())),
            profile.source(),
        )?;
        if let Some((_, defines)) = profile.defines() {
            write_new(&dir.join(format!("{}{DEFINES_EXTENSION}", profile.stem())), defines)?;
        }
    }

    Ok(())
}

fn write_new(path: &Path, contents: &str) -> io::Result<()> {
    match std::fs::OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => file.write_all(contents.as_bytes()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error),
    }
}

pub fn scan_profiles(dir: &Path) -> Vec<String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            log::warn!("reading profiles from {}: {error}", dir.display());

            return BundledProfile::ALL.map(|profile| profile.stem().to_owned()).to_vec();
        },
    };

    let mut names = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|file| !file.ends_with(DEFINES_EXTENSION))
        .filter_map(|file| file.strip_suffix(PROFILE_EXTENSION).map(str::to_owned))
        .collect::<Vec<_>>();
    names.sort();

    names
}

pub fn profile_label(name: &str) -> &str {
    match BundledProfile::from_name(name) {
        Some(profile) => profile.label(),
        None => name,
    }
}

enum ForcedFile {
    Disk(PathBuf),
    Embedded(&'static str, &'static str),
}

impl ForcedFile {
    fn prelude(&self) -> PreludeFile {
        match self {
            Self::Disk(path) => PreludeFile::Disk(path.clone()),
            Self::Embedded(name, source) => PreludeFile::Embedded(name, source),
        }
    }

    fn path(&self) -> &Path {
        match self {
            Self::Disk(path) => path,
            Self::Embedded(name, _) => Path::new(name),
        }
    }
}

#[derive(Default)]
struct ForcedSources {
    defines: Option<ForcedFile>,
    profile: Option<ForcedFile>,
}

impl ForcedSources {
    fn resolve(options: &BakeOptions) -> Self {
        let Some(name) = options.forced_profile.as_deref() else {
            return Self::default();
        };
        let bundled = BundledProfile::from_name(name);
        let on_disk = |extension: &str| {
            options
                .profile_dir
                .as_ref()
                .map(|dir| dir.join(format!("{name}{extension}")))
                .filter(|path| path.is_file())
                .map(ForcedFile::Disk)
        };

        Self {
            defines: on_disk(DEFINES_EXTENSION).or_else(|| {
                bundled
                    .and_then(BundledProfile::defines)
                    .map(|(name, source)| ForcedFile::Embedded(name, source))
            }),
            profile: on_disk(PROFILE_EXTENSION)
                .or_else(|| bundled.map(|bundled| ForcedFile::Embedded(bundled.source_name(), bundled.source()))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BakeOptions {
    pub enabled: bool,
    pub optimizations_enabled: bool,
    pub profile: Option<TreePath>,
    pub forced_profile: Option<String>,
    pub profile_dir: Option<PathBuf>,
    pub limits: vm::Limits,
}

impl Default for BakeOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            optimizations_enabled: true,
            profile: None,
            forced_profile: None,
            profile_dir: None,
            limits: vm::Limits::default(),
        }
    }
}

/// `DM_BAKE=0` turns baking off for one run without touching the saved setting.
pub fn baking_enabled(setting: bool) -> bool {
    std::env::var_os("DM_BAKE").map_or(setting, |value| value != "0")
}

pub(crate) fn compile(entry: &Path, options: &BakeOptions, progress: &Progress) -> Result<Compiled, LoadError> {
    let arena = StrArena::new();
    let forced = ForcedSources::resolve(options);
    let editor = compile_view(
        &arena,
        entry,
        false,
        false,
        options,
        &forced,
        SourceCache::default(),
        Vec::new(),
        progress,
    )?;

    let bake = if options.enabled {
        Some(compile_view(
            &arena,
            entry,
            true,
            true,
            options,
            &forced,
            editor.source_cache.clone(),
            editor.tokens,
            progress,
        )?)
    } else {
        None
    };

    let optimization_timings = bake.as_ref().map(|view| view.optimization_timings).unwrap_or_default();
    let mut resource_dirs = editor.resource_dirs;
    if let Some(view) = &bake {
        for path in &view.resource_dirs {
            if !resource_dirs.contains(path) {
                resource_dirs.push(path.clone());
            }
        }
    }

    let (bake_program, profiles, profile_error, bake_files, bake_errors, bake_sema_errors, codegen_error) = match bake {
        Some(view) => {
            let files: Arc<[PathBuf]> = Arc::from(view.files);
            let ProfileSelection {
                chosen: selection,
                error: profile_error,
            } = view.profile.unwrap_or_default();
            let profiles = selection.as_ref().map(|(_, profiles)| profiles.clone());
            let (program, codegen_error) = match (view.module, selection) {
                (Some(module), Some((profile, _))) => {
                    let definition = vm::profile::ProfileDefinition::resolve(&view.tree, &module.index, profile);
                    let roots = definition.entry_points();
                    match codegen::generate_reachable(&module, &view.tree, &roots, &vm::bake::host_reads()) {
                        Ok(module) => (
                            Some(BakeProgram {
                                tree: view.tree,
                                module,
                                profile,
                                icon_states: vm::IconStates::default(),
                            }),
                            None,
                        ),
                        Err(error) => (None, Some(error)),
                    }
                },
                _ => (None, None),
            };

            (
                program,
                profiles,
                profile_error,
                files,
                view.errors,
                view.sema_errors,
                codegen_error,
            )
        },
        None => (None, None, None, Arc::default(), Vec::new(), Vec::new(), None),
    };

    Ok(Compiled {
        fingerprint: fingerprint::object_tree(&editor.tree, &editor.builtin_files),
        tree: editor.tree,
        bake_program,
        profiles,
        profile_error,
        bake_files,
        root: editor.root,
        files: editor.files,
        builtin_files: editor.builtin_files,
        maps: editor.maps,
        resource_dirs,
        errors: editor.errors,
        sema_errors: editor.sema_errors,
        bake_errors,
        bake_sema_errors,
        codegen_error,
        optimization_timings,
    })
}

#[derive(Default)]
struct ProfileSelection {
    chosen: Option<(TypeId, Profiles)>,
    error: Option<vm::bake::ProfileError>,
}

fn select_profile(
    tree: &ObjectTree, files: &[PathBuf], options: &BakeOptions, forced: &ForcedSources,
) -> ProfileSelection {
    let path = |id| {
        tree.get(id)
            .map(|declaration| declaration.path.to_string())
            .unwrap_or_default()
    };
    let (chosen, error) = match options.forced_profile {
        Some(_) => match forced
            .profile
            .as_ref()
            .and_then(|profile| files.iter().position(|file| file == profile.path()))
            .map_or(Err(vm::bake::ProfileError::Missing), |file| {
                vm::bake::profile_in_file(tree, FileId(file as u32))
            }) {
            Ok(active) => {
                let active_path = path(active);
                let profiles = Profiles {
                    available: vec![active_path.clone()],
                    default: active_path.clone(),
                    active: active_path,
                };

                (Some((active, profiles)), None)
            },
            Err(error) => (None, Some(error)),
        },
        None => match vm::bake::profile_catalog(tree) {
            Ok(catalog) => {
                let active = catalog.select(tree, options.profile.as_ref());
                let profiles = Profiles {
                    available: catalog.profiles.iter().copied().map(path).collect::<Vec<_>>(),
                    default: path(catalog.default),
                    active: path(active),
                };

                (Some((active, profiles)), None)
            },
            Err(vm::bake::ProfileError::Missing) => (None, None),
            Err(error) => (None, Some(error)),
        },
    };

    ProfileSelection { chosen, error }
}

#[allow(clippy::too_many_arguments)]
fn compile_view<'a>(
    arena: &'a StrArena, entry: &Path, baking: bool, generate: bool, options: &BakeOptions, forced: &ForcedSources,
    source_cache: SourceCache<'a>, tokens: Vec<Spanned<'a>>, progress: &'a Progress,
) -> Result<CompiledView<'a>, LoadError> {
    progress.enter(Stage::Preprocess, 0);
    let mut prelude = prelude_files();
    prelude.extend(forced.defines.as_ref().map(ForcedFile::prelude));
    let postlude = forced.profile.as_ref().map(ForcedFile::prelude);
    let preprocessed = Preprocessor::new(arena)
        .with_source_cache(source_cache)
        .with_output(tokens)
        .with_prelude(prelude)
        .with_postlude(postlude)
        .with_baking(baking)
        .with_progress(|path| {
            progress.advance(&path.display().to_string());

            !progress.is_cancelled()
        })
        .run(entry)?;
    if let Some(error) = preprocessed.errors.iter().find(|e| e.is_fatal()) {
        return Err(error.clone().into());
    }
    if progress.is_cancelled() {
        return Err(LoadError::Cancelled);
    }

    progress.enter(Stage::Parse, 0);
    progress.set_detail(&format!("{} tokens", preprocessed.tokens.len()));
    let ast = ast::parse(&preprocessed.tokens)
        .map_err(|error| LoadError::parse(error, &preprocessed.sources, preprocessed.entry, entry))?;

    // nothing preprocesses after the bake view, so only the editor view's buffer is kept for reuse
    let tokens = if baking {
        drop(preprocessed.tokens);
        Vec::new()
    } else {
        let mut tokens = preprocessed.tokens;
        tokens.clear();
        tokens
    };

    if progress.is_cancelled() {
        return Err(LoadError::Cancelled);
    }

    progress.enter(Stage::Analyze, 0);
    let files = (0..preprocessed.sources.len())
        .map(|i| {
            preprocessed
                .sources
                .path(FileId(i as u32))
                .map(Path::to_path_buf)
                .unwrap_or_default()
        })
        .collect::<Vec<_>>();

    let (tree, sema_errors) = sema::analyze(&ast);
    // a view without a profile to bake never reaches codegen, so its IR is not worth lowering
    let profile = generate.then(|| select_profile(&tree, &files, options, forced));
    let (module, optimization_timings) = match &profile {
        Some(ProfileSelection { chosen: Some(_), .. }) => {
            let (module, timings) = ir::lower(&ast, &tree, options.optimizations_enabled);
            (Some(module), timings)
        },
        _ => (None, ir::opt::OptimizationTimings::default()),
    };
    if progress.is_cancelled() {
        return Err(LoadError::Cancelled);
    }

    let root = preprocessed
        .entry
        .and_then(|id| preprocessed.sources.path(id))
        .map(Path::to_path_buf)
        .unwrap_or_else(|| entry.to_path_buf());

    let mut errors = preprocessed.errors;
    if let Some(module) = &module {
        let uses = module.procs.iter().flat_map(|procedure| {
            module
                .named_variables(procedure)
                .map(|(name, location)| (name.as_str(), location))
        });
        let source_root = root.parent().unwrap_or(&root);
        errors.extend(macros_used_before_definition(
            &preprocessed.defines,
            &preprocessed.sources,
            source_root,
            uses,
        ));
    }

    drop(preprocessed.defines);

    let maps = preprocessed
        .resources
        .iter()
        .filter(|path| is_map(path))
        .cloned()
        .collect();

    Ok(CompiledView {
        tree,
        module,
        root,
        files,
        maps,
        builtin_files: preprocessed.builtin_files,
        resource_dirs: preprocessed.resource_dirs,
        errors,
        sema_errors,
        optimization_timings,
        source_cache: preprocessed.source_cache,
        tokens,
        profile,
    })
}

pub(crate) struct Compiled {
    pub fingerprint: CodebaseHash,
    pub tree: ObjectTree,
    pub bake_program: Option<BakeProgram>,
    pub profiles: Option<Profiles>,
    pub profile_error: Option<vm::bake::ProfileError>,
    pub bake_files: Arc<[PathBuf]>,
    pub root: PathBuf,
    pub files: Vec<PathBuf>,
    pub builtin_files: Vec<FileId>,
    pub maps: Vec<PathBuf>,
    pub resource_dirs: Vec<PathBuf>,
    pub errors: Vec<PreprocessError>,
    pub sema_errors: Vec<SemaError>,
    pub bake_errors: Vec<PreprocessError>,
    pub bake_sema_errors: Vec<SemaError>,
    pub codegen_error: Option<CodegenError>,
    pub optimization_timings: ir::opt::OptimizationTimings,
}

struct CompiledView<'a> {
    tree: ObjectTree,
    module: Option<ir::Module>,
    root: PathBuf,
    files: Vec<PathBuf>,
    builtin_files: Vec<FileId>,
    maps: Vec<PathBuf>,
    resource_dirs: Vec<PathBuf>,
    errors: Vec<PreprocessError>,
    sema_errors: Vec<SemaError>,
    optimization_timings: ir::opt::OptimizationTimings,
    source_cache: SourceCache<'a>,
    tokens: Vec<Spanned<'a>>,
    profile: Option<ProfileSelection>,
}

#[cfg(test)]
mod tests;
