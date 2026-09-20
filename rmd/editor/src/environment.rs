use core::{arena::StrArena, location::FileId, path::TreePath, source::SourceMap};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use codegen::CodegenError;
use dmi::error::IconError;
use preprocessor::error::PreprocessError;
use sema::error::SemaError;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BakeOptions {
    pub enabled: bool,
    pub editor_walls: bool,
    pub profile: Option<TreePath>,
    pub limits: vm::Limits,
}

impl Default for BakeOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            editor_walls: false,
            profile: None,
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
        options.editor_walls,
        false,
        preprocessor::SourceCache::default(),
        progress,
    )?;

    let bake = if options.enabled {
        Some(compile_view(
            &arena,
            entry,
            true,
            false,
            true,
            editor.source_cache.clone(),
            progress,
        )?)
    } else {
        None
    };

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
            let catalog = vm::bake::profile_catalog(&view.tree);
            let (selection, profile_error) = match catalog {
                Ok(catalog) => {
                    let active = catalog.select(&view.tree, options.profile.as_ref());
                    let path = |id| {
                        view.tree
                            .get(id)
                            .map(|declaration| declaration.path.to_string())
                            .unwrap_or_default()
                    };
                    let profiles = Profiles {
                        available: catalog.profiles.iter().copied().map(path).collect::<Vec<_>>(),
                        default: path(catalog.default),
                        active: path(active),
                    };

                    (Some((active, profiles)), None)
                },
                Err(vm::bake::ProfileError::Missing) => (None, None),
                Err(error) => (None, Some(error)),
            };
            let profiles = selection.as_ref().map(|(_, profiles)| profiles.clone());
            let program = match (view.module, selection) {
                (Some(module), Some((profile, _))) => Some(BakeProgram {
                    tree: view.tree,
                    module,
                    profile,
                    files: files.clone(),
                    icon_states: vm::IconStates::default(),
                }),
                _ => None,
            };

            (
                program,
                profiles,
                profile_error,
                files,
                view.errors,
                view.sema_errors,
                view.codegen_error,
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
    })
}

fn compile_view<'a>(
    arena: &'a StrArena, entry: &Path, baking: bool, editor_walls: bool, generate: bool,
    source_cache: preprocessor::SourceCache<'a>, progress: &'a Progress,
) -> Result<CompiledView<'a>, LoadError> {
    progress.enter(Stage::Preprocess, 0);
    let preprocessed = preprocessor::Preprocessor::new(arena)
        .with_source_cache(source_cache)
        .with_baking(baking)
        .with_editor_walls(editor_walls)
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
    let (tree, module, sema_errors) = sema::analyze(&ast);
    if progress.is_cancelled() {
        return Err(LoadError::Cancelled);
    }

    let should_generate = generate && vm::bake::has_profile(&tree);
    let (module, codegen_error) = match should_generate.then(|| codegen::generate(&module)) {
        Some(Ok(module)) => (Some(module), None),
        Some(Err(error)) => (None, Some(error)),
        None => (None, None),
    };

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
        codegen_error,
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
}

struct CompiledView<'a> {
    tree: ObjectTree,
    module: Option<codegen::Module>,
    root: PathBuf,
    files: Vec<PathBuf>,
    maps: Vec<PathBuf>,
    resource_dirs: Vec<PathBuf>,
    errors: Vec<PreprocessError>,
    sema_errors: Vec<SemaError>,
    codegen_error: Option<CodegenError>,
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
}
