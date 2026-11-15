use super::{OperationKind, detect_operation, discover, parse_pull_request, web_links_from_url};
use crate::git::fixtures::{commit, git, git_available, temp_repo};

#[test]
fn commit_links_follow_the_remote_host_without_credentials() {
    for (remote, expected) in [
        (
            "https://github.com/team/map.git",
            Some((
                "https://github.com/team/map/commit/abc",
                "https://github.com/team/map/pull/7",
            )),
        ),
        (
            "git@github.com:team/map.git",
            Some((
                "https://github.com/team/map/commit/abc",
                "https://github.com/team/map/pull/7",
            )),
        ),
        (
            "https://gitlab.com/team/map.git",
            Some((
                "https://gitlab.com/team/map/-/commit/abc",
                "https://gitlab.com/team/map/-/merge_requests/7",
            )),
        ),
        (
            "ssh://git@bitbucket.org/team/map.git",
            Some((
                "https://bitbucket.org/team/map/commits/abc",
                "https://bitbucket.org/team/map/pull-requests/7",
            )),
        ),
        (
            "https://codeberg.org/team/map.git",
            Some((
                "https://codeberg.org/team/map/commit/abc",
                "https://codeberg.org/team/map/pulls/7",
            )),
        ),
        (
            "https://user:secret@github.com/team/map.git",
            Some((
                "https://github.com/team/map/commit/abc",
                "https://github.com/team/map/pull/7",
            )),
        ),
        ("file:///tmp/map.git", None),
    ] {
        let parsed = gix::url::parse(remote).unwrap();
        let links = web_links_from_url(&parsed).map(|links| (links.commit("abc"), links.pull_request(7)));
        assert_eq!(
            links.as_ref().map(|(commit, pull)| (commit.as_str(), pull.as_str())),
            expected,
            "{remote}"
        );
    }
}

#[test]
fn detects_squash_and_merge_pull_requests() {
    for (summary, body, expected) in [
        (
            "Update MetaStation.dmm (#66744)",
            None,
            (Some(66744), "Update MetaStation.dmm (#66744)"),
        ),
        (
            "Update MetaStation.dmm (#66744) ",
            None,
            (Some(66744), "Update MetaStation.dmm (#66744) "),
        ),
        (
            "Merge pull request #45 from someone/branch",
            Some("\nFixes the bar\n\nmore words"),
            (Some(45), "Fixes the bar (#45)"),
        ),
        (
            "Merge pull request #45 from someone/branch",
            None,
            (Some(45), "Merge pull request #45 from someone/branch"),
        ),
        ("Fix (#12) thing", None, (None, "Fix (#12) thing")),
        ("Fix thing (#abc)", None, (None, "Fix thing (#abc)")),
        ("Fix thing (#)", None, (None, "Fix thing (#)")),
        (
            "Merge branch 'master' of https://github.com/team/map",
            None,
            (None, "Merge branch 'master' of https://github.com/team/map"),
        ),
    ] {
        let (number, text) = parse_pull_request(summary, body);
        assert_eq!((number, text.as_str()), expected, "{summary}");
    }
}

#[test]
fn detects_the_operation_from_pseudo_refs() {
    let dir = std::env::temp_dir().join(format!("rmd-git-operation-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    assert_eq!(detect_operation(&dir), None);

    std::fs::write(dir.join("REBASE_HEAD"), "1111\n").unwrap();
    assert_eq!(detect_operation(&dir), Some((OperationKind::Rebase, "1111".into())));

    std::fs::write(dir.join("MERGE_HEAD"), "2222\n3333\n").unwrap();
    assert_eq!(detect_operation(&dir), Some((OperationKind::Merge, "2222".into())));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn resolves_revisions_and_reads_the_map_they_hold() {
    if !git_available() {
        return;
    }

    let map = |turf: &str| format!("\"a\" = (/turf/{turf})\n\n(1,1,1) = {{\"\na\n\"}}\n");
    let dir = temp_repo("revisions");
    commit(&dir, "other.txt", "x\n", "before the map");
    commit(&dir, "a.dmm", &map("floor"), "add map");
    git(&dir, &["tag", "v1"]);
    commit(&dir, "a.dmm", &map("wall"), "wall it off");
    commit(&dir, "a.dmm", "not a map {", "break it");

    let repo = discover(&dir.join("a.dmm")).unwrap().open().unwrap();
    assert_eq!(repo.resolve("HEAD").unwrap().summary, "break it");
    assert_eq!(repo.resolve("main~1").unwrap().summary, "wall it off");
    assert_eq!(repo.resolve(" v1 ").unwrap().summary, "add map");
    assert!(repo.resolve("no-such-branch").is_err());

    let turf = |spec: &str| {
        let commit = repo.resolve(spec).unwrap();
        repo.map_at(&commit.hash).map(|map| {
            map.and_then(|map| {
                map.tile_at(dmm::Coord::new(1, 1, 1))
                    .map(|tile| tile[0].path.to_string())
            })
        })
    };
    assert_eq!(turf("HEAD~1"), Ok(Some(String::from("/turf/wall"))));
    assert_eq!(turf("v1"), Ok(Some(String::from("/turf/floor"))));
    assert_eq!(turf("HEAD~3"), Ok(None), "the map didn't exist yet");
    assert!(turf("HEAD").is_err(), "an unparseable map is an error");
}

#[test]
fn reads_history_conflicts_and_marks_them_resolved() {
    if !git_available() {
        return;
    }

    let dir = temp_repo("history");
    std::fs::create_dir_all(dir.join("maps")).unwrap();
    commit(&dir, "maps/a.dmm", "one\n", "add map");
    commit(&dir, "other.txt", "x\n", "unrelated");
    commit(&dir, "maps/a.dmm", "two\n", "change map (#12)");

    let path = discover(&dir.join("maps").join("a.dmm")).expect("inside the repository");
    assert_eq!(path.rel, "maps/a.dmm");
    let repo = path.open().unwrap();
    assert_eq!(repo.branch().unwrap(), "main");

    git(
        &dir,
        &["remote", "add", "origin", "https://github.com/example/maps.git"],
    );
    assert_eq!(
        path.open()
            .unwrap()
            .web_links()
            .map(|links| links.commit("abc"))
            .as_deref(),
        Some("https://github.com/example/maps/commit/abc")
    );

    let history = repo.file_history(10, &|| false).unwrap();
    let summaries = history
        .iter()
        .map(|version| version.commit.summary.as_str())
        .collect::<Vec<_>>();
    assert_eq!(summaries, ["change map (#12)", "add map"]);
    assert_eq!(history[0].commit.pull_request, Some(12));
    assert_eq!(history[1].commit.pull_request, None);
    for version in &history {
        let expected = git(&dir, &["rev-parse", "--short", &version.commit.hash]).unwrap();
        assert_eq!(version.commit.short, expected.trim(), "abbreviated like rev-parse");
    }

    git(&dir, &["config", "core.abbrev", "12"]);
    let abbreviated = path.open().unwrap().file_history(1, &|| false).unwrap();
    assert_eq!(
        abbreviated[0].commit.short,
        abbreviated[0].commit.hash[..12],
        "core.abbrev is honoured"
    );
    assert_eq!(repo.blob(history[1].blob.unwrap()).unwrap(), b"one\n");
    assert_eq!(repo.file_history(1, &|| false).unwrap().len(), 1);

    git(&dir, &["checkout", "-q", "-b", "feature"]);
    commit(&dir, "maps/a.dmm", "theirs\n", "their change");
    git(&dir, &["checkout", "-q", "main"]);
    commit(&dir, "maps/a.dmm", "ours\n", "our change");
    git(&dir, &["merge", "feature"]);

    let repo = path.open().unwrap();
    let stages = repo.unmerged().unwrap().expect("conflicted");
    assert_eq!(repo.blob(stages.theirs.unwrap()).unwrap(), b"theirs\n");
    assert_eq!(repo.blob(stages.base.unwrap()).unwrap(), b"two\n");
    let operation = repo.operation().expect("merging");
    assert_eq!(operation.kind, OperationKind::Merge);
    assert_eq!(operation.theirs.subject, "their change");
    assert_eq!(operation.theirs.name.as_deref(), Some("feature"));

    std::fs::write(dir.join(".gitattributes"), "maps/*.dmm text eol=lf\n").unwrap();
    std::fs::write(path.file(), "resolved\r\n").unwrap();
    repo.mark_resolved().unwrap();

    assert_eq!(path.open().unwrap().unmerged().unwrap(), None);
    assert_eq!(
        git(&dir, &["diff", "--cached", "--name-only", "--diff-filter=U"]).unwrap(),
        ""
    );
    assert_eq!(git(&dir, &["show", ":maps/a.dmm"]).unwrap(), "resolved\n");
}
