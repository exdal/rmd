use base::coord::Coord;
use std::{
    path::{Path, PathBuf},
    time::Instant,
};

use super::{Job, Loader, Outcome, load_codebase, load_map};
use crate::environment::{
    compile::BakeOptions,
    progress::{Progress, Stage},
};

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

    let loaded =
        super::load_map_with_git(&path, 1, &crate::environment::progress::Progress::new(), true, None).unwrap();
    assert_eq!(loaded.conflict.as_ref().unwrap().conflicts.len(), 1);
    assert_eq!(
        loaded.map.tile_at(Coord::new(1, 1, 1)).unwrap()[0].path.to_string(),
        "/turf/wall"
    );
    let mut session = crate::session::Session::new();
    session.apply_map(loaded);
    let id = session.state.active().unwrap();
    assert!(session.state.document(id).unwrap().is_dirty());
    assert!(session.resolve_conflict(id, &[Coord::new(1, 1, 1)], crate::git::conflict::Side::Theirs));
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
        crate::git::repo::discover(&path)
            .unwrap()
            .open()
            .unwrap()
            .unmerged()
            .unwrap()
            .is_none()
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        dmm::writer::write(&session.state.document(id).unwrap().to_map())
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

        let loaded = super::load_map_with_git(&path, 1, &crate::environment::progress::Progress::new(), true, None)
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

    let loaded =
        super::load_map_with_git(&path, 1, &crate::environment::progress::Progress::new(), true, None).unwrap();
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
        crate::git::repo::discover(&path)
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
    let loaded =
        load_codebase(&root.join("test.dme"), &BakeOptions::default(), &progress).expect("load the example codebase");

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
