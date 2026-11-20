use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{Receiver, TryRecvError, channel},
    },
    thread,
};

use dmm::Map;
use objtree::TypeId;
use prefab::Prefab;
use render::texture::TextureCatalog;

use crate::{
    document::level_session::validate_level,
    environment::{
        Environment, LoadDiagnostics,
        compile::BakeOptions,
        error::LoadError,
        progress::{Progress, Snapshot, Stage},
        session::{build_textures, discover_maps},
    },
    frame::visual,
    git::{
        conflict::ConflictData,
        repo::{self, RepoPath},
    },
    palette::session::{PrefabThumbnail, prefab_thumbnail_for, prefab_thumbnail_or_missing},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    Codebase { path: PathBuf, bake: BakeOptions },
    Map { path: PathBuf, z: u32, git_enabled: bool },
}

impl Job {
    pub fn path(&self) -> &Path {
        match self {
            Self::Codebase { path, .. } | Self::Map { path, .. } => path,
        }
    }

    /// Heading for the progress popup.
    pub const fn title(&self) -> &'static str {
        match self {
            Self::Codebase { .. } => "Opening codebase",
            Self::Map { .. } => "Opening map",
        }
    }
}

pub struct LoadedCodebase {
    pub environment: Environment,
    pub diagnostics: LoadDiagnostics,
    pub textures: TextureCatalog,
    pub thumbnails: HashMap<TypeId, Option<PrefabThumbnail>>,
    pub maps: Vec<PathBuf>,
}

pub struct LoadedMap {
    pub path: PathBuf,
    pub map: Map,
    pub z: u32,
    pub errors: Vec<dmm::error::MapError>,
    pub repo: Option<RepoPath>,
    pub conflict: Option<ConflictData>,
}

pub enum Outcome {
    Codebase { path: PathBuf, loaded: Box<LoadedCodebase> },
    Map(Box<LoadedMap>),
    Failed { job: Job, error: String },
    Cancelled,
}

#[derive(Default)]
pub struct Loader {
    active: Option<Active>,
}

struct Active {
    job: Job,
    progress: Arc<Progress>,
    results: Receiver<Outcome>,
}

pub struct LoadView {
    pub title: &'static str,
    pub path: String,
    pub snapshot: Snapshot,
    pub cancelling: bool,
    pub cancellable: bool,
}

impl Loader {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_busy(&self) -> bool {
        self.active.is_some()
    }

    pub fn start(&mut self, job: Job) {
        if self.is_busy() {
            return;
        }

        let progress = Arc::new(Progress::new());
        let (sender, results) = channel();
        let worker = Arc::clone(&progress);
        let running = job.clone();
        thread::spawn(move || {
            let outcome = run(&running, &worker);
            let _ = sender.send(outcome);
        });

        self.active = Some(Active { job, progress, results });
    }

    pub fn cancel(&self) {
        if let Some(active) = self.active.as_ref() {
            active.progress.cancel();
        }
    }

    pub fn poll(&mut self) -> Option<Outcome> {
        let active = self.active.as_ref()?;

        match active.results.try_recv() {
            Ok(outcome) => {
                self.active = None;

                Some(outcome)
            },
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                let job = self.active.take()?.job;
                let error = format!("loading {} failed unexpectedly", job.path().display());

                Some(Outcome::Failed { job, error })
            },
        }
    }

    pub fn view(&self) -> Option<LoadView> {
        let active = self.active.as_ref()?;

        Some(LoadView {
            title: active.job.title(),
            path: active.job.path().display().to_string(),
            snapshot: active.progress.snapshot(),
            cancelling: active.progress.is_cancelled(),
            cancellable: true,
        })
    }
}

fn run(job: &Job, progress: &Progress) -> Outcome {
    let result = match job {
        Job::Codebase { path, bake } => load_codebase(path, bake, progress).map(|loaded| Outcome::Codebase {
            path: path.clone(),
            loaded: Box::new(loaded),
        }),
        Job::Map { path, z, git_enabled } => {
            load_map_with_git(path, *z, progress, *git_enabled).map(|loaded| Outcome::Map(Box::new(loaded)))
        },
    };

    match result {
        Ok(_) if progress.is_cancelled() => Outcome::Cancelled,
        Ok(outcome) => outcome,
        Err(_) if progress.is_cancelled() => Outcome::Cancelled,
        Err(error) => Outcome::Failed {
            job: job.clone(),
            error,
        },
    }
}

pub fn load_codebase(path: &Path, bake: &BakeOptions, progress: &Progress) -> Result<LoadedCodebase, String> {
    let (environment, diagnostics) = match Environment::load_with(path, bake.clone(), progress) {
        Ok(loaded) => loaded,
        Err(LoadError::Cancelled) => return Err(String::from("cancelled")),
        Err(error) => return Err(error.to_string()),
    };

    let textures = build_textures(&environment, progress);
    if progress.is_cancelled() {
        return Err(String::from("cancelled"));
    }

    let thumbnails = build_thumbnails(&environment, &textures, progress);
    if progress.is_cancelled() {
        return Err(String::from("cancelled"));
    }

    progress.enter(Stage::FindMaps, 0);
    let maps = discover_maps(environment.base_dir());

    Ok(LoadedCodebase {
        environment,
        diagnostics,
        textures,
        thumbnails,
        maps,
    })
}

#[cfg(test)]
pub fn load_map(path: &Path, z: u32, progress: &Progress) -> Result<LoadedMap, String> {
    load_map_with_git(path, z, progress, false)
}

pub fn load_map_with_git(path: &Path, z: u32, progress: &Progress, git_enabled: bool) -> Result<LoadedMap, String> {
    let repo = git_enabled.then(|| repo::discover(path)).flatten();
    // Git is optional here: when the repository can't be read, open the file as usual
    let unmerged = repo.as_ref().and_then(|repo_path| {
        let opened = repo_path.open().and_then(|opened| Ok((opened.unmerged()?, opened)));
        match opened {
            Ok((Some(stages), opened)) => Some((stages, opened)),
            Ok((None, _)) => None,
            Err(error) => {
                log::warn!("{}: could not check for merge conflicts: {error}", path.display());
                None
            },
        }
    });
    if let Some((stages, opened)) = unmerged {
        progress.enter(Stage::GitMerge, 3);
        progress.set_detail(&path.display().to_string());
        let parse_stage = |id, name: &str| -> Result<Map, String> {
            let bytes = opened.blob(id).map_err(|error| format!("{name}: {error}"))?;
            let text = String::from_utf8(bytes).map_err(|error| format!("{name}: {error}"))?;
            let (map, errors) = dmm::parser::parse(&text);
            if !errors.is_empty() {
                return Err(format!("{name}: invalid map blob: {errors:?}"));
            }
            Ok(map)
        };
        let base = stages.base.map(|id| parse_stage(id, "base stage")).transpose()?;
        progress.advance("base");
        let ours = parse_stage(
            stages
                .ours
                .ok_or("The HEAD stage is missing. Resolve this modify/delete conflict in Git")?,
            "HEAD stage",
        )?;
        progress.advance("HEAD");
        let theirs = parse_stage(
            stages
                .theirs
                .ok_or("The incoming stage is missing. Resolve this modify/delete conflict in Git")?,
            "incoming stage",
        )?;
        progress.advance("incoming");
        if progress.is_cancelled() {
            return Err(String::from("cancelled"));
        }
        let merged = dmm::merge::merge3(base.as_ref(), &ours, &theirs);
        validate_level(z, merged.map.size.z).map_err(|error| format!("{}: {error}", path.display()))?;
        return Ok(LoadedMap {
            path: path.to_path_buf(),
            map: merged.map,
            z,
            errors: Vec::new(),
            repo,
            conflict: Some(ConflictData {
                operation: opened.operation(),
                conflicts: merged.conflicts,
            }),
        });
    }
    progress.enter(Stage::ReadMap, 0);
    progress.set_detail(&path.display().to_string());
    let source = std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    if progress.is_cancelled() {
        return Err(String::from("cancelled"));
    }

    progress.enter(Stage::ParseMap, 0);
    progress.set_detail(&path.display().to_string());
    let (map, errors) = dmm::parser::parse(&source);
    validate_level(z, map.size.z).map_err(|error| format!("{}: {error}", path.display()))?;

    Ok(LoadedMap {
        path: path.to_path_buf(),
        map,
        z,
        errors,
        repo,
        conflict: None,
    })
}

fn build_thumbnails(
    environment: &Environment, textures: &TextureCatalog, progress: &Progress,
) -> HashMap<TypeId, Option<PrefabThumbnail>> {
    let mut thumbnails = HashMap::new();
    let mut standalone = crate::bake::Standalone::default();
    let mut derived = 0usize;

    progress.enter(Stage::Thumbnails, environment.tree.len());
    for declaration in environment.tree.iter() {
        if progress.is_cancelled() {
            break;
        }
        progress.advance(&declaration.path.to_string());

        let prefab = Prefab::new(declaration.path.clone());
        let appearance = visual::resolve_id(&environment.tree, declaration.id, &prefab);
        let thumbnail = match prefab_thumbnail_for(textures, &appearance) {
            Some(thumbnail) => Some(thumbnail),
            None => {
                let standalone = standalone_thumbnail(
                    &mut standalone,
                    environment,
                    textures,
                    declaration.id,
                    &prefab,
                    &appearance,
                );
                derived += usize::from(standalone.is_some());

                standalone.or_else(|| prefab_thumbnail_or_missing(textures, &appearance))
            },
        };
        thumbnails.insert(declaration.id, thumbnail);
    }

    log::info!("{derived} thumbnails derived by baking");

    thumbnails
}

fn standalone_thumbnail(
    standalone: &mut crate::bake::Standalone, environment: &Environment, textures: &TextureCatalog, id: TypeId,
    prefab: &Prefab, appearance: &visual::Appearance,
) -> Option<PrefabThumbnail> {
    let tree = &environment.tree;
    if appearance.icon.is_none() || !tree.roots().atom.is_some_and(|atom| tree.is_subtype_of(id, atom)) {
        return None;
    }

    let delta = standalone.appearance(environment, prefab)?;
    let appearance = visual::resolve_delta(tree, id, prefab, &delta);

    prefab_thumbnail_for(textures, &appearance)
}

#[cfg(test)]
mod tests;
