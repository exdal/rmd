use core::{arena::StrArena, location::FileId, path::TreePath, source::SourceMap};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use codegen::CodegenError;
use dmi::error::IconError;
use preprocessor::error::PreprocessError;
use sema::error::SemaError;
use serde::{Deserialize, Serialize};

use crate::{
    BakeProgram,
    LoadError,
    ObjectTree,
    Profiles,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BundledProfile {
    Cmss13,
    Goonstation,
    Monkestation,
    Tgstation,
    Vanderlin,
}

impl BundledProfile {
    pub const ALL: [Self; 5] = [
        Self::Cmss13,
        Self::Goonstation,
        Self::Monkestation,
        Self::Tgstation,
        Self::Vanderlin,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Cmss13 => "CMSS13",
            Self::Goonstation => "Goonstation",
            Self::Monkestation => "Monkestation",
            Self::Tgstation => "tgstation",
            Self::Vanderlin => "Vanderlin",
        }
    }

    pub const fn type_path(self) -> &'static str {
        match self {
            Self::Cmss13 => "/datum/demir/cmss13",
            Self::Goonstation => "/datum/demir/goonstation",
            Self::Monkestation => "/datum/demir/monkestation",
            Self::Tgstation => "/datum/demir/tgstation",
            Self::Vanderlin => "/datum/demir/vanderlin",
        }
    }

    const fn source_name(self) -> &'static str {
        match self {
            Self::Cmss13 => "<bundled-profile-cmss13.dm>",
            Self::Goonstation => "<bundled-profile-goonstation.dm>",
            Self::Monkestation => "<bundled-profile-monkestation.dm>",
            Self::Tgstation => "<bundled-profile-tgstation.dm>",
            Self::Vanderlin => "<bundled-profile-vanderlin.dm>",
        }
    }

    const fn source(self) -> &'static str {
        match self {
            Self::Cmss13 => include_str!("../../../examples/profiles/cmss13.dm"),
            Self::Goonstation => include_str!("../../../examples/profiles/goonstation.dm"),
            Self::Monkestation => include_str!("../../../examples/profiles/monkestation.dm"),
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
            Self::Cmss13 | Self::Monkestation | Self::Tgstation | Self::Vanderlin => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BakeOptions {
    pub enabled: bool,
    pub optimizations_enabled: bool,
    pub profile: Option<TreePath>,
    pub forced_profile: Option<BundledProfile>,
    pub limits: vm::Limits,
}

impl Default for BakeOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            optimizations_enabled: true,
            profile: None,
            forced_profile: None,
            limits: vm::Limits::default(),
        }
    }
}

/// `DM_BAKE=0` turns baking off for one run without touching the saved setting.
pub fn baking_enabled(setting: bool) -> bool { std::env::var_os("DM_BAKE").map_or(setting, |value| value != "0") }

pub(crate) fn compile(entry: &Path, options: &BakeOptions, progress: &Progress) -> Result<Compiled, LoadError> {
    let arena = StrArena::new();
    let editor = compile_view(
        &arena,
        entry,
        false,
        false,
        options,
        preprocessor::SourceCache::default(),
        progress,
    )?;

    let bake = if options.enabled {
        Some(compile_view(
            &arena,
            entry,
            true,
            true,
            options,
            editor.source_cache.clone(),
            progress,
        )?)
    } else {
        None
    };

    let mut optimization_timings = editor.optimization_timings;
    let mut resource_dirs = editor.resource_dirs;
    if let Some(view) = &bake {
        optimization_timings.merge(view.optimization_timings);
        for path in &view.resource_dirs {
            if !resource_dirs.contains(path) {
                resource_dirs.push(path.clone());
            }
        }
    }

    let (bake_program, profiles, profile_error, bake_files, bake_errors, bake_sema_errors, codegen_error) = match bake {
        Some(view) => {
            let files: Arc<[PathBuf]> = Arc::from(view.files);
            let path = |id| {
                view.tree
                    .get(id)
                    .map(|declaration| declaration.path.to_string())
                    .unwrap_or_default()
            };
            let (selection, profile_error) = match options.forced_profile {
                Some(forced) => match view.tree.id_of(&TreePath::parse(forced.type_path())) {
                    Some(active) => {
                        let active_path = path(active);
                        let profiles = Profiles {
                            available: vec![active_path.clone()],
                            default: active_path.clone(),
                            active: active_path,
                        };

                        (Some((active, profiles)), None)
                    },
                    None => (None, Some(vm::bake::ProfileError::Missing)),
                },
                None => match vm::bake::profile_catalog(&view.tree) {
                    Ok(catalog) => {
                        let active = catalog.select(&view.tree, options.profile.as_ref());
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
            let profiles = selection.as_ref().map(|(_, profiles)| profiles.clone());
            let (program, codegen_error) = match (view.module, selection) {
                (Some(module), Some((profile, _))) => {
                    let definition = vm::profile::ProfileDefinition::resolve(&view.tree, profile);
                    let roots = definition.entry_points();
                    match codegen::generate_reachable(&module, &view.tree, &roots) {
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

fn compile_view<'a>(
    arena: &'a StrArena, entry: &Path, baking: bool, generate: bool, options: &BakeOptions,
    source_cache: preprocessor::SourceCache<'a>, progress: &'a Progress,
) -> Result<CompiledView<'a>, LoadError> {
    progress.enter(Stage::Preprocess, 0);
    let mut prelude = preprocessor::prelude_files();
    if let Some((name, source)) = options.forced_profile.and_then(BundledProfile::defines) {
        prelude.push(preprocessor::PreludeFile::Embedded(name, source));
    }
    let postlude = options
        .forced_profile
        .map(|profile| preprocessor::PreludeFile::Embedded(profile.source_name(), profile.source()));
    let preprocessed = preprocessor::Preprocessor::new(arena)
        .with_source_cache(source_cache)
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
    drop(preprocessed.tokens);
    drop(preprocessed.defines);
    if progress.is_cancelled() {
        return Err(LoadError::Cancelled);
    }

    progress.enter(Stage::Analyze, 0);
    let (tree, module, sema_errors, optimization_timings) =
        sema::analyze_with_optimizations(&ast, options.optimizations_enabled);
    if progress.is_cancelled() {
        return Err(LoadError::Cancelled);
    }

    let module = generate.then_some(module);

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

    Ok(CompiledView {
        tree,
        module,
        root,
        files,
        maps,
        resource_dirs: preprocessed.resource_dirs,
        errors: preprocessed.errors,
        sema_errors,
        optimization_timings,
        source_cache: preprocessed.source_cache,
    })
}

pub(crate) struct Compiled {
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
    maps: Vec<PathBuf>,
    resource_dirs: Vec<PathBuf>,
    errors: Vec<PreprocessError>,
    sema_errors: Vec<SemaError>,
    optimization_timings: ir::opt::OptimizationTimings,
    source_cache: preprocessor::SourceCache<'a>,
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
        assert_eq!(default.optimization_timings.samples(), 2);
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
            forced_profile: Some(BundledProfile::Cmss13),
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
        assert_eq!(environment.bake_options.forced_profile, Some(BundledProfile::Cmss13));

        std::fs::remove_dir_all(entry.parent().expect("fixture parent")).expect("remove fixture");
    }

    #[test]
    fn bundled_profile_metadata_covers_every_embedded_example() {
        assert_eq!(
            BundledProfile::ALL.map(BundledProfile::type_path),
            [
                "/datum/demir/cmss13",
                "/datum/demir/goonstation",
                "/datum/demir/monkestation",
                "/datum/demir/tgstation",
                "/datum/demir/vanderlin",
            ]
        );
        for profile in BundledProfile::ALL {
            assert!(profile.source().contains(profile.type_path()));
        }
        assert_eq!(
            BundledProfile::Goonstation.defines().map(|(_, source)| source),
            Some("#define USE_PERSPECTIVE_EDITOR_WALLS\n")
        );
    }
}
