use base::{arena::StrArena, location::FileId, path::TreePath};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use codegen::CodegenError;
use net::CodebaseHash;
use objtree::ObjectTree;
use preprocessor::{
    PreludeFile, Preprocessor, SourceCache, Spanned, error::PreprocessError, macros_used_before_definition,
    prelude_files,
};
use sema::error::SemaError;

use super::{
    BakeProgram, Profiles,
    error::LoadError,
    fingerprint, is_map,
    profile::{BundledProfile, DEFINES_EXTENSION, PROFILE_EXTENSION, ProfileSelection, select_profile},
    progress::{Progress, Stage},
};

pub(super) enum ForcedFile {
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

    pub(super) fn path(&self) -> &Path {
        match self {
            Self::Disk(path) => path,
            Self::Embedded(name, _) => Path::new(name),
        }
    }
}

#[derive(Default)]
pub(super) struct ForcedSources {
    defines: Option<ForcedFile>,
    pub(super) profile: Option<ForcedFile>,
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
