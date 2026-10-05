use core::{arena::StrArena, location::FileId, path::TreePath, source::SourceMap};
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use codegen::CodegenError;
use dmi::error::IconError;
use net::CodebaseHash;
use objtree::TypeId;
use preprocessor::{PreludeFile, Preprocessor, SourceCache, Spanned, error::PreprocessError, prelude_files};
use sema::error::SemaError;

use crate::{
    BakeProgram,
    LoadError,
    ObjectTree,
    Profiles,
    fingerprint,
    progress::{Progress, Stage},
};

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

    pub fn len(&self) -> usize {
        self.preprocess.len()
            + self.sema.len()
            + self.bake_preprocess.len()
            + self.bake_sema.len()
            + usize::from(self.codegen.is_some())
            + usize::from(self.profile.is_some())
            + self.icons.len()
            + self.bake.len()
    }
}

pub(crate) fn is_map(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("dmm"))
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
    Cmss13,
    Goonstation,
    Monkestation,
    SecondCity,
    Tgstation,
    Vanderlin,
}

impl BundledProfile {
    pub const ALL: [Self; 6] = [
        Self::Cmss13,
        Self::Goonstation,
        Self::Monkestation,
        Self::SecondCity,
        Self::Tgstation,
        Self::Vanderlin,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Cmss13 => "CMSS13",
            Self::Goonstation => "Goonstation",
            Self::Monkestation => "Monkestation",
            Self::SecondCity => "SecondCity",
            Self::Tgstation => "tgstation",
            Self::Vanderlin => "Vanderlin",
        }
    }

    pub const fn stem(self) -> &'static str {
        match self {
            Self::Cmss13 => "cmss13",
            Self::Goonstation => "goonstation",
            Self::Monkestation => "monkestation",
            Self::SecondCity => "secondcity",
            Self::Tgstation => "tgstation",
            Self::Vanderlin => "vanderlin",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> { Self::ALL.into_iter().find(|profile| profile.stem() == name) }

    const fn source_name(self) -> &'static str {
        match self {
            Self::Cmss13 => "<bundled-profile-cmss13.dm>",
            Self::Goonstation => "<bundled-profile-goonstation.dm>",
            Self::Monkestation => "<bundled-profile-monkestation.dm>",
            Self::SecondCity => "<bundled-profile-secondcity.dm>",
            Self::Tgstation => "<bundled-profile-tgstation.dm>",
            Self::Vanderlin => "<bundled-profile-vanderlin.dm>",
        }
    }

    const fn source(self) -> &'static str {
        match self {
            Self::Cmss13 => include_str!("../../../examples/profiles/cmss13.dm"),
            Self::Goonstation => include_str!("../../../examples/profiles/goonstation.dm"),
            Self::Monkestation => include_str!("../../../examples/profiles/monkestation.dm"),
            Self::SecondCity => include_str!("../../../examples/profiles/secondcity.dm"),
            Self::Tgstation => include_str!("../../../examples/profiles/tgstation.dm"),
            Self::Vanderlin => include_str!("../../../examples/profiles/vanderlin.dm"),
        }
    }

    const fn defines(self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::Goonstation => Some((
                "<bundled-profile-goonstation-defines.dm>",
                "#define USE_PERSPECTIVE_EDITOR_WALLS\n",
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
            Self::Cmss13 | Self::Vanderlin => None,
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
pub fn baking_enabled(setting: bool) -> bool { std::env::var_os("DM_BAKE").map_or(setting, |value| value != "0") }

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
                    let definition = vm::profile::ProfileDefinition::resolve(&view.tree, profile);
                    let roots = definition.entry_points();
                    match codegen::generate_reachable(&module, &view.tree, &roots, &vm::bake::host_reads()) {
                        Ok(module) => (
                            Some(BakeProgram {
                                tree: view.tree,
                                module,
                                profile,
                                files: files.clone(),
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
    let mut tokens = preprocessed.tokens;
    tokens.clear();
    drop(preprocessed.defines);
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

    // a view without a profile to bake never reaches codegen, so its IR is not worth finishing
    let mut profile = None;
    let (tree, module, sema_errors, optimization_timings) = if generate {
        sema::analyze_if(&ast, options.optimizations_enabled, |tree| {
            let selection = select_profile(tree, &files, options, forced);
            let is_selected = selection.chosen.is_some();
            profile = Some(selection);

            is_selected
        })
    } else {
        let (tree, errors) = sema::analyze_tree(&ast);
        (tree, None, errors, ir::opt::OptimizationTimings::default())
    };
    if progress.is_cancelled() {
        return Err(LoadError::Cancelled);
    }

    let root = preprocessed
        .entry
        .and_then(|id| preprocessed.sources.path(id))
        .map(Path::to_path_buf)
        .unwrap_or_else(|| entry.to_path_buf());

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
        errors: preprocessed.errors,
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
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::Environment;

    fn fixture(source: &str) -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("rmd-profile-selection-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&root).expect("create fixture directory");
        let entry = root.join("test.dm");
        std::fs::write(&entry, source).expect("write fixture");

        entry
    }

    #[test]
    fn compilation_exposes_profiles_and_honors_valid_or_stale_requests() {
        let entry = fixture(
            r#"
#ifdef __DEMIR_BAKE__
/datum/demir/main
    default = TRUE
    bake(atom/target)
        target.name = "main"
/datum/demir/main/debug/bake(atom/target)
    target.name = "debug"
#endif
"#,
        );

        let load = |requested: Option<&str>| {
            Environment::load_with(
                &entry,
                BakeOptions {
                    profile: requested.map(TreePath::parse),
                    ..BakeOptions::default()
                },
                &Progress::new(),
            )
            .expect("load fixture")
        };
        let (default, diagnostics) = load(None);
        assert!(diagnostics.profile.is_none());
        assert_eq!(default.optimization_timings.samples(), 1);
        assert_eq!(
            default.profiles,
            Some(Profiles {
                available: vec![
                    String::from("/datum/demir/main"),
                    String::from("/datum/demir/main/debug"),
                ],
                default: String::from("/datum/demir/main"),
                active: String::from("/datum/demir/main"),
            })
        );
        let default_program = default.bake_program.as_ref().expect("default profile bytecode");
        let main = default_program
            .tree
            .id_of(&TreePath::parse("/datum/demir/main"))
            .expect("main profile");
        let main_bake = vm::profile::ProfileDefinition::resolve(&default_program.tree, main)
            .procedure(vm::profile::ProfileHook::Bake)
            .expect("main bake body");
        let debug = default_program
            .tree
            .id_of(&TreePath::parse("/datum/demir/main/debug"))
            .expect("debug profile");
        let debug_bake = vm::profile::ProfileDefinition::resolve(&default_program.tree, debug)
            .procedure(vm::profile::ProfileHook::Bake)
            .expect("debug bake body");
        assert!(default_program.module.function_for_proc(main_bake).is_some());
        assert!(default_program.module.function_for_proc(debug_bake).is_none());

        let (debug, diagnostics) = load(Some("/datum/demir/main/debug"));
        assert!(diagnostics.profile.is_none());
        assert_eq!(
            debug.profiles.as_ref().map(|profiles| profiles.active.as_str()),
            Some("/datum/demir/main/debug")
        );
        assert_eq!(
            debug
                .bake_program
                .as_ref()
                .and_then(|program| program.tree.get(program.profile))
                .map(|declaration| declaration.path.to_string()),
            Some(String::from("/datum/demir/main/debug"))
        );
        let debug_program = debug.bake_program.as_ref().expect("debug profile bytecode");
        let main = debug_program
            .tree
            .id_of(&TreePath::parse("/datum/demir/main"))
            .expect("main profile");
        let main_bake = vm::profile::ProfileDefinition::resolve(&debug_program.tree, main)
            .procedure(vm::profile::ProfileHook::Bake)
            .expect("main bake body");
        let debug = debug_program
            .tree
            .id_of(&TreePath::parse("/datum/demir/main/debug"))
            .expect("debug profile");
        let debug_bake = vm::profile::ProfileDefinition::resolve(&debug_program.tree, debug)
            .procedure(vm::profile::ProfileHook::Bake)
            .expect("debug bake body");
        assert!(debug_program.module.function_for_proc(main_bake).is_none());
        assert!(debug_program.module.function_for_proc(debug_bake).is_some());

        let (stale, diagnostics) = load(Some("/datum/demir/main/missing"));
        assert!(diagnostics.profile.is_none());
        assert_eq!(
            stale.profiles.as_ref().map(|profiles| profiles.active.as_str()),
            Some("/datum/demir/main")
        );

        std::fs::remove_dir_all(entry.parent().expect("fixture parent")).expect("remove fixture");
    }

    #[test]
    fn invalid_default_declarations_are_diagnostics_and_disable_baking() {
        let entry = fixture(
            r#"
#ifdef __DEMIR_BAKE__
/datum/demir/one
/datum/demir/two
#endif
"#,
        );
        let (environment, diagnostics) = Environment::load_with(&entry, BakeOptions::default(), &Progress::new())
            .expect("the compatibility view still loads");

        assert!(matches!(
            diagnostics.profile,
            Some(vm::bake::ProfileError::MissingDefault(_))
        ));
        assert!(environment.profiles.is_none());
        assert!(environment.bake_program.is_none());

        std::fs::remove_dir_all(entry.parent().expect("fixture parent")).expect("remove fixture");
    }

    #[test]
    fn forcing_a_bundled_profile_overrides_the_codebase_default() {
        let entry = fixture(
            r#"
#ifdef __DEMIR_BAKE__
/datum/demir/native
    default = TRUE
#endif
"#,
        );
        let options = BakeOptions {
            forced_profile: Some(String::from("cmss13")),
            ..BakeOptions::default()
        };
        let (environment, diagnostics) = Environment::load_with(&entry, options, &Progress::new())
            .expect("the bundled profile should be injected after the codebase");

        assert!(diagnostics.profile.is_none());
        assert_eq!(
            environment.profiles,
            Some(Profiles {
                available: vec![String::from("/datum/demir/cmss13")],
                default: String::from("/datum/demir/cmss13"),
                active: String::from("/datum/demir/cmss13"),
            })
        );
        assert_eq!(environment.bake_options.forced_profile.as_deref(), Some("cmss13"));

        std::fs::remove_dir_all(entry.parent().expect("fixture parent")).expect("remove fixture");
    }

    #[test]
    fn bundled_profile_metadata_covers_every_embedded_example() {
        assert_eq!(
            BundledProfile::ALL.map(BundledProfile::stem),
            [
                "cmss13",
                "goonstation",
                "monkestation",
                "secondcity",
                "tgstation",
                "vanderlin",
            ]
        );
        for profile in BundledProfile::ALL {
            assert_eq!(BundledProfile::from_name(profile.stem()), Some(profile));
            assert!(
                profile
                    .source()
                    .contains(&format!("/datum/demir/{}\n\tdefault = TRUE", profile.stem()))
            );
        }
        assert_eq!(
            BundledProfile::Goonstation.defines().map(|(_, source)| source),
            Some("#define USE_PERSPECTIVE_EDITOR_WALLS\n")
        );
        assert_eq!(
            BundledProfile::Tgstation.defines().map(|(_, source)| source),
            Some("#ifdef __DEMIR_BAKE__\n#define CBT\n#endif\n")
        );
        assert_eq!(
            BundledProfile::SecondCity.defines().map(|(_, source)| source),
            Some("#ifdef __DEMIR_BAKE__\n#define CBT\n#endif\n")
        );
        assert_eq!(
            BundledProfile::Monkestation.defines().map(|(_, source)| source),
            Some("#ifdef __DEMIR_BAKE__\n#define CBT\n#endif\n")
        );
        assert_eq!(profile_label("secondcity"), "SecondCity");
        assert_eq!(profile_label("mine"), "mine");
    }

    #[test]
    fn missing_profiles_are_written_and_edits_are_kept() {
        let dir = fixture("").with_file_name("profiles");
        write_missing_profiles(&dir).expect("write profiles");
        for profile in BundledProfile::ALL {
            let written = std::fs::read_to_string(dir.join(format!("{}.dm", profile.stem()))).expect("profile");
            assert_eq!(written, profile.source());
        }
        assert_eq!(
            std::fs::read_to_string(dir.join("tgstation.defines.dm")).expect("defines"),
            "#ifdef __DEMIR_BAKE__\n#define CBT\n#endif\n"
        );
        assert!(!dir.join("cmss13.defines.dm").exists());

        let edited = dir.join("tgstation.dm");
        std::fs::write(&edited, "// mine now").expect("edit profile");
        let deleted = dir.join("vanderlin.dm");
        std::fs::remove_file(&deleted).expect("delete profile");
        write_missing_profiles(&dir).expect("write profiles again");

        assert_eq!(std::fs::read_to_string(&edited).expect("edited"), "// mine now");
        assert_eq!(
            std::fs::read_to_string(&deleted).expect("restored"),
            BundledProfile::Vanderlin.source()
        );

        std::fs::remove_dir_all(dir.parent().expect("fixture parent")).expect("remove fixture");
    }

    #[test]
    fn scanning_lists_custom_profiles_but_not_defines() {
        let dir = fixture("").with_file_name("profiles");
        write_missing_profiles(&dir).expect("write profiles");
        std::fs::write(dir.join("mine.dm"), "").expect("custom profile");
        std::fs::write(dir.join("mine.defines.dm"), "").expect("custom defines");
        std::fs::write(dir.join("notes.txt"), "").expect("stray file");

        assert_eq!(
            scan_profiles(&dir),
            [
                "cmss13",
                "goonstation",
                "mine",
                "monkestation",
                "secondcity",
                "tgstation",
                "vanderlin",
            ]
        );

        std::fs::remove_dir_all(dir.parent().expect("fixture parent")).expect("remove fixture");
    }

    #[test]
    fn forcing_a_custom_profile_activates_its_default_and_injects_its_defines() {
        let entry = fixture(
            r#"
#ifdef MINE_DEFINED
/obj/mine_marker
#endif
#ifdef __DEMIR_BAKE__
/datum/demir/native
    default = TRUE
#endif
"#,
        );
        let dir = entry.with_file_name("profiles");
        std::fs::create_dir_all(&dir).expect("profiles directory");
        std::fs::write(
            dir.join("mine.dm"),
            "#ifdef __DEMIR_BAKE__\n/datum/demir/mine\n    default = TRUE\n/datum/demir/mine/debug\n#endif\n",
        )
        .expect("custom profile");
        std::fs::write(dir.join("mine.defines.dm"), "#define MINE_DEFINED\n").expect("custom defines");
        let options = BakeOptions {
            forced_profile: Some(String::from("mine")),
            profile_dir: Some(dir.clone()),
            ..BakeOptions::default()
        };
        let (environment, diagnostics) = Environment::load_with(&entry, options, &Progress::new()).expect("load");

        assert!(diagnostics.profile.is_none(), "{:?}", diagnostics.profile);
        assert_eq!(
            environment.profiles.as_ref().map(|profiles| profiles.active.as_str()),
            Some("/datum/demir/mine")
        );
        assert!(environment.bake_files.contains(&dir.join("mine.dm")));
        assert!(environment.tree.id_of(&TreePath::parse("/obj/mine_marker")).is_some());

        std::fs::remove_dir_all(entry.parent().expect("fixture parent")).expect("remove fixture");
    }

    #[test]
    fn a_disk_copy_replaces_the_embedded_profile_until_it_is_deleted() {
        let entry = fixture("");
        let dir = entry.with_file_name("profiles");
        std::fs::create_dir_all(&dir).expect("profiles directory");
        let copy = dir.join("cmss13.dm");
        std::fs::write(
            &copy,
            "#ifdef __DEMIR_BAKE__\n/datum/demir/edited\n    default = TRUE\n#endif\n",
        )
        .expect("edited profile");
        let load = || {
            let options = BakeOptions {
                forced_profile: Some(String::from("cmss13")),
                profile_dir: Some(dir.clone()),
                ..BakeOptions::default()
            };
            let (environment, _) = Environment::load_with(&entry, options, &Progress::new()).expect("load");

            environment.profiles.map(|profiles| profiles.active)
        };

        assert_eq!(load().as_deref(), Some("/datum/demir/edited"));
        std::fs::remove_file(&copy).expect("delete profile");
        assert_eq!(load().as_deref(), Some("/datum/demir/cmss13"));

        std::fs::remove_dir_all(entry.parent().expect("fixture parent")).expect("remove fixture");
    }

    #[test]
    fn a_custom_profile_with_two_defaults_is_a_diagnostic() {
        let entry = fixture("");
        let dir = entry.with_file_name("profiles");
        std::fs::create_dir_all(&dir).expect("profiles directory");
        std::fs::write(
            dir.join("twice.dm"),
            "#ifdef __DEMIR_BAKE__\n/datum/demir/one\n    default = TRUE\n/datum/demir/two\n    default = \
             TRUE\n#endif\n",
        )
        .expect("custom profile");
        let options = BakeOptions {
            forced_profile: Some(String::from("twice")),
            profile_dir: Some(dir),
            ..BakeOptions::default()
        };
        let (environment, diagnostics) = Environment::load_with(&entry, options, &Progress::new()).expect("load");

        assert!(matches!(
            diagnostics.profile,
            Some(vm::bake::ProfileError::MultipleDefaults(_))
        ));
        assert!(environment.bake_program.is_none());

        std::fs::remove_dir_all(entry.parent().expect("fixture parent")).expect("remove fixture");
    }
}
