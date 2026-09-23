use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{Receiver, TryRecvError, channel},
    },
    thread,
};

use dmm::{Map, Prefab};
use editor::{
    Environment,
    conflict::ConflictData,
    environment::{BakeOptions, LoadDiagnostics},
    error::LoadError,
    git::{self, RepoPath},
    progress::{Progress, Snapshot, Stage},
    visual,
};
use objtree::TypeId;
use render::texture::TextureCatalog;

use crate::session::{
    PrefabThumbnail,
    build_textures,
    discover_maps,
    prefab_thumbnail_for,
    prefab_thumbnail_or_missing,
    validate_level,
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
    pub fn new() -> Self { Self::default() }

    pub fn is_busy(&self) -> bool { self.active.is_some() }

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
    let repo = git_enabled.then(|| git::discover(path)).flatten();
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
    let mut standalone = editor::bake::Standalone::default();
    let mut derived = 0usize;

    progress.enter(Stage::Thumbnails, environment.tree.len());
    for declaration in environment.tree.iter() {
        if progress.is_cancelled() {
            break;
        }
        progress.advance(&declaration.path.to_string());

        let prefab = Prefab::new(declaration.path.clone());
        let appearance = visual::resolve_id(&environment.tree, declaration.id, &prefab);
        let thumbnail = match prefab_thumbnail_for(textures, environment, &appearance) {
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

                standalone.or_else(|| prefab_thumbnail_or_missing(textures, environment, &appearance))
            },
        };
        thumbnails.insert(declaration.id, thumbnail);
    }

    log::info!("{derived} thumbnails derived by baking");

    thumbnails
}

fn standalone_thumbnail(
    standalone: &mut editor::bake::Standalone, environment: &Environment, textures: &TextureCatalog, id: TypeId,
    prefab: &Prefab, appearance: &visual::Appearance,
) -> Option<PrefabThumbnail> {
    let tree = &environment.tree;
    if appearance.icon.is_none() || !tree.roots().atom.is_some_and(|atom| tree.is_subtype_of(id, atom)) {
        return None;
    }

    let delta = standalone.appearance(environment, prefab)?;
    let appearance = visual::resolve_delta(tree, id, prefab, &delta);

    prefab_thumbnail_for(textures, environment, &appearance)
}

#[cfg(test)]
mod tests {
    use std::{
        path::{Path, PathBuf},
        time::Instant,
    };

    use editor::{
        environment::BakeOptions,
        progress::{Progress, Stage},
    };

    use super::{Job, Loader, Outcome, load_codebase, load_map};

    fn fixture_git(dir: &Path, args: &[&str]) -> std::process::Output {
        std::process::Command::new("git")
            .args([
                "-c",
                "user.name=rmd",
                "-c",
                "user.email=rmd@example.com",
                "-c",
                "core.autocrlf=false",
            ])
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git fixture command")
    }

    #[test]
    fn unmerged_map_loads_from_stages_and_can_be_saved_and_staged() {
        if std::process::Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let dir = std::env::temp_dir().join(format!(
            "rmd-loader-merge-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(fixture_git(&dir, &["init", "-q", "-b", "main"]).status.success());
        let path = dir.join("map.dmm");
        let map = |tile: &str| format!("\"a\" = ({tile})\n\n(1,1,1) = {{\"\na\n\"}}\n");
        std::fs::write(&path, map("/turf/floor")).unwrap();
        assert!(fixture_git(&dir, &["add", "map.dmm"]).status.success());
        assert!(fixture_git(&dir, &["commit", "-q", "-m", "base"]).status.success());
        assert!(fixture_git(&dir, &["checkout", "-q", "-b", "feature"]).status.success());
        std::fs::write(&path, map("/turf/lava")).unwrap();
        assert!(fixture_git(&dir, &["commit", "-qam", "incoming"]).status.success());
        assert!(fixture_git(&dir, &["checkout", "-q", "main"]).status.success());
        std::fs::write(&path, map("/turf/wall")).unwrap();
        assert!(fixture_git(&dir, &["commit", "-qam", "ours"]).status.success());
        assert!(!fixture_git(&dir, &["merge", "feature"]).status.success());

        let loaded = super::load_map_with_git(&path, 1, &editor::progress::Progress::new(), true).unwrap();
        assert_eq!(loaded.conflict.as_ref().unwrap().conflicts.len(), 1);
        assert_eq!(
            loaded.map.tile_at(dmm::Coord::new(1, 1, 1)).unwrap()[0]
                .path
                .to_string(),
            "/turf/wall"
        );
        let mut session = crate::session::Session::new();
        session.apply_map(loaded);
        let id = session.state.active().unwrap();
        assert!(session.state.document(id).unwrap().is_dirty());
        assert!(session.resolve_conflict(id, &[dmm::Coord::new(1, 1, 1)], editor::conflict::Side::Theirs));
        session.save_map().unwrap();
        assert!(session.mark_resolved(id));
        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        while session.git_state(id).unwrap().staging && Instant::now() < deadline {
            session.poll_git();
            std::thread::yield_now();
        }
        assert!(!session.git_state(id).unwrap().staging);
        assert!(
            session.git_state(id).unwrap().error.is_none(),
            "{:?}",
            session.git_state(id).unwrap().error
        );
        assert!(
            editor::git::discover(&path)
                .unwrap()
                .open()
                .unwrap()
                .unmerged()
                .unwrap()
                .is_none()
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            dmm::writer::write(&session.state.document(id).unwrap().map)
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_unreadable_repository_does_not_stop_a_map_from_opening() {
        if std::process::Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let dir = std::env::temp_dir().join(format!(
            "rmd-loader-broken-index-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(fixture_git(&dir, &["init", "-q", "-b", "main"]).status.success());
        let path = dir.join("map.dmm");
        std::fs::write(&path, "\"a\" = (/turf/floor)\n\n(1,1,1) = {\"\na\n\"}\n").unwrap();
        assert!(fixture_git(&dir, &["add", "map.dmm"]).status.success());
        assert!(fixture_git(&dir, &["commit", "-q", "-m", "base"]).status.success());
        // Too short for a checksum, and long enough to reach the checksum test
        for index in [b"not an index".to_vec(), vec![b'x'; 256]] {
            std::fs::write(dir.join(".git").join("index"), index).unwrap();

            let loaded = super::load_map_with_git(&path, 1, &editor::progress::Progress::new(), true)
                .expect("the map opens without conflict detection");

            assert!(loaded.conflict.is_none());
            assert!(loaded.errors.is_empty());
            assert!(loaded.repo.is_some(), "the git panel can still report the problem");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_unmerged_file_with_identical_tiles_still_needs_a_save() {
        if std::process::Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let dir = std::env::temp_dir().join(format!(
            "rmd-loader-identical-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(fixture_git(&dir, &["init", "-q", "-b", "main"]).status.success());
        let path = dir.join("map.dmm");
        let map =
            |number: &str| format!("\"a\" = (/obj/item{{damage = {number}}},/turf/floor)\n\n(1,1,1) = {{\"\na\n\"}}\n");
        std::fs::write(&path, map("1.0")).unwrap();
        assert!(fixture_git(&dir, &["add", "map.dmm"]).status.success());
        assert!(fixture_git(&dir, &["commit", "-q", "-m", "base"]).status.success());
        assert!(fixture_git(&dir, &["checkout", "-q", "-b", "feature"]).status.success());
        std::fs::write(&path, map("1.3")).unwrap();
        assert!(fixture_git(&dir, &["commit", "-qam", "incoming"]).status.success());
        assert!(fixture_git(&dir, &["checkout", "-q", "main"]).status.success());
        std::fs::write(&path, map("1.30")).unwrap();
        assert!(fixture_git(&dir, &["commit", "-qam", "ours"]).status.success());
        assert!(!fixture_git(&dir, &["merge", "feature"]).status.success());

        let loaded = super::load_map_with_git(&path, 1, &editor::progress::Progress::new(), true).unwrap();
        assert!(loaded.conflict.as_ref().unwrap().conflicts.is_empty());
        let mut session = crate::session::Session::new();
        session.apply_map(loaded);
        let id = session.state.active().unwrap();
        assert!(session.state.document(id).unwrap().is_dirty());
        assert!(!session.mark_resolved(id));
        session.save_map().unwrap();
        assert!(session.mark_resolved(id));
        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        while session.git_state(id).unwrap().staging && Instant::now() < deadline {
            session.poll_git();
            std::thread::yield_now();
        }
        assert!(session.git_state(id).unwrap().error.is_none());
        assert!(
            editor::git::discover(&path)
                .unwrap()
                .open()
                .unwrap()
                .unmerged()
                .unwrap()
                .is_none()
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    fn examples() -> PathBuf {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env");

        path.canonicalize().unwrap_or(path)
    }

    /// Waits for the job in flight, since a worker thread has no deterministic finish time.
    fn settle(loader: &mut Loader) -> Outcome {
        let deadline = Instant::now() + std::time::Duration::from_secs(60);

        loop {
            if let Some(outcome) = loader.poll() {
                return outcome;
            }

            assert!(Instant::now() < deadline, "the worker never reported back");
            std::thread::yield_now();
        }
    }

    #[test]
    fn a_codebase_load_reports_every_stage_and_ends_on_map_discovery() {
        let root = examples();
        let progress = Progress::new();
        let loaded = load_codebase(&root.join("test.dme"), &BakeOptions::default(), &progress)
            .expect("load the example codebase");

        assert_eq!(progress.snapshot().stage, Stage::FindMaps);
        assert!(!loaded.textures.is_empty());
        assert!(!loaded.thumbnails.is_empty());
        assert!(loaded.maps.iter().any(|map| map.ends_with("test.dmm")));
        assert_eq!(loaded.environment.tree.len(), loaded.thumbnails.len());
    }

    #[test]
    fn a_cancelled_codebase_load_gives_up_instead_of_compiling() {
        let root = examples();
        let progress = Progress::new();
        progress.cancel();

        assert!(load_codebase(&root.join("test.dme"), &BakeOptions::default(), &progress).is_err());
    }

    #[test]
    fn a_map_load_rejects_a_level_the_map_does_not_have() {
        let root = examples();
        let progress = Progress::new();
        let loaded = load_map(&root.join("test.dmm"), 1, &progress).expect("parse the example map");

        assert_eq!(loaded.z, 1);
        assert!(loaded.map.size.z >= 1);
        assert!(load_map(&root.join("test.dmm"), 99, &progress).is_err());
        assert!(load_map(&root.join("nope.dmm"), 1, &progress).is_err());
    }

    #[test]
    fn a_worker_delivers_the_map_it_was_asked_for() {
        let map = examples().join("test.dmm");
        let mut loader = Loader::new();
        assert!(loader.poll().is_none());
        assert!(loader.view().is_none());

        loader.start(Job::Map {
            path: map.clone(),
            z: 1,
            git_enabled: false,
        });
        assert!(loader.is_busy());

        // A second request while one is in flight is dropped rather than queued.
        loader.start(Job::Codebase {
            path: examples().join("test.dme"),
            bake: BakeOptions::default(),
        });

        match settle(&mut loader) {
            Outcome::Map(loaded) => assert_eq!(loaded.path, map),
            _ => panic!("expected the map job to come back"),
        }

        assert!(!loader.is_busy());
        assert!(loader.poll().is_none());
    }

    #[test]
    fn a_failed_job_comes_back_with_the_job_that_failed() {
        let missing = examples().join("missing.dmm");
        let mut loader = Loader::new();
        loader.start(Job::Map {
            path: missing.clone(),
            z: 1,
            git_enabled: false,
        });

        match settle(&mut loader) {
            Outcome::Failed { job, error } => {
                assert_eq!(job.path(), missing);
                assert_eq!(job.title(), "Opening map");
                assert!(!error.is_empty());
            },
            _ => panic!("expected the missing map to fail"),
        }
    }
}
