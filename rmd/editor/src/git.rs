use std::path::{Path, PathBuf};

use gix::{
    ObjectId,
    bstr::{BStr, ByteSlice},
    index::entry::{Flags, Mode, Stage, Stat},
    prelude::ObjectIdExt,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitError(pub String);

impl std::fmt::Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(&self.0) }
}

impl std::error::Error for GitError {}

fn fail(error: impl std::fmt::Display) -> GitError { GitError(error.to_string()) }

pub type GitResult<T> = Result<T, GitError>;

/// A file inside a repository's work tree
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoPath {
    pub root: PathBuf,
    pub git_dir: PathBuf,
    // relative to `root`, with forward slashes
    pub rel: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKind {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

impl OperationKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Merge => "Merging",
            Self::Rebase => "Rebasing",
            Self::CherryPick => "Cherry-picking",
            Self::Revert => "Reverting",
        }
    }
}

/// Git's minimum abbreviation, used when a hash cannot be looked up to shorten it properly
const MIN_ABBREV: usize = 7;

fn short_hash(hash: &str) -> String { hash.chars().take(MIN_ABBREV).collect() }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRef {
    pub hash: String,
    pub short: String,
    pub subject: String,
    // a branch whose tip is this commit
    pub name: Option<String>,
}

impl CommitRef {
    pub fn describe(&self) -> String {
        match &self.name {
            Some(name) => format!("{name} ({}) {}", self.short, self.subject),
            None => format!("{} {}", self.short, self.subject),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operation {
    pub kind: OperationKind,
    // the commit being brought in, what `--theirs` refers to
    pub theirs: CommitRef,
}

/// Blob ids of the index stages of an unmerged file
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stages {
    pub base: Option<ObjectId>,
    pub ours: Option<ObjectId>,
    pub theirs: Option<ObjectId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitInfo {
    pub hash: String,
    pub short: String,
    pub author: String,
    // unix timestamp
    pub time: i64,
    pub summary: String,
    pub pull_request: Option<u64>,
}

fn pull_request(summary: &str, body: Option<&str>) -> (Option<u64>, String) {
    fn number(digits: &str) -> Option<u64> {
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }

        digits.parse().ok()
    }

    let trimmed = summary.trim_end();
    // we only support github like pull requests for now
    if let Some(number) = trimmed
        .strip_suffix(')')
        .and_then(|rest| rest.rsplit_once("(#"))
        .and_then(|(_, digits)| number(digits))
    {
        return (Some(number), summary.to_owned());
    }

    if let Some(number) = trimmed
        .strip_prefix("Merge pull request #")
        .and_then(|rest| number(rest.split_once(' ').map_or(rest, |(digits, _)| digits)))
    {
        let title = body.and_then(|body| body.lines().map(str::trim).find(|line| !line.is_empty()));

        return match title {
            Some(title) => (Some(number), format!("{title} (#{number})")),
            None => (Some(number), summary.to_owned()),
        };
    }

    (None, summary.to_owned())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Forge {
    GitHub,
    GitLab,
    Bitbucket,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebLinks {
    // scheme://host[:port]/owner/repo
    repo: String,
    forge: Forge,
}

impl WebLinks {
    /// `git@github.com:owner/repo.git`
    pub fn from_remote(url: &str) -> Option<Self> { web_links(&gix::url::parse(url.as_bytes().as_bstr()).ok()?) }

    pub fn commit(&self, hash: &str) -> String {
        let route = match self.forge {
            Forge::GitLab => "-/commit",
            Forge::Bitbucket => "commits",
            Forge::GitHub | Forge::Other => "commit",
        };

        format!("{}/{route}/{hash}", self.repo)
    }

    pub fn pull_request(&self, number: u64) -> String {
        let route = match self.forge {
            Forge::GitHub => "pull",
            Forge::GitLab => "-/merge_requests",
            Forge::Bitbucket => "pull-requests",
            // Gitea and Forgejo
            Forge::Other => "pulls",
        };

        format!("{}/{route}/{number}", self.repo)
    }
}

/// The file as one commit left it, `blob` is `None` when that commit deleted it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileVersion {
    pub commit: CommitInfo,
    pub blob: Option<ObjectId>,
}

fn relative(root: &Path, file: &Path) -> Option<String> {
    let root = std::fs::canonicalize(root).ok()?;
    let file = std::fs::canonicalize(file).ok()?;
    let rel = file.strip_prefix(root).ok()?;
    let parts = rel
        .components()
        .map(|part| part.as_os_str().to_str())
        .collect::<Option<Vec<_>>>()?;

    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Finds the repository that holds `path`. `None` outside a work tree.
pub fn discover(path: &Path) -> Option<RepoPath> {
    let dir = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let repo = gix::discover(dir).ok()?;
    let root = repo.workdir()?.to_path_buf();
    let rel = relative(&root, path)?;

    Some(RepoPath {
        git_dir: repo.git_dir().to_path_buf(),
        root,
        rel,
    })
}

/// Which operation is waiting on conflicts, read from the pseudo refs in the git directory
pub fn detect_operation(git_dir: &Path) -> Option<(OperationKind, String)> {
    [
        ("MERGE_HEAD", OperationKind::Merge),
        ("CHERRY_PICK_HEAD", OperationKind::CherryPick),
        ("REVERT_HEAD", OperationKind::Revert),
        ("REBASE_HEAD", OperationKind::Rebase),
    ]
    .into_iter()
    .find_map(|(file, kind)| {
        let contents = std::fs::read_to_string(git_dir.join(file)).ok()?;
        // MERGE_HEAD lists one line per merged head, octopus merges have several
        let hash = contents.lines().next()?.trim();

        (!hash.is_empty()).then(|| (kind, hash.to_owned()))
    })
}

/// An opened repository, bound to one file
pub struct Repo {
    repo: gix::Repository,
    pub path: RepoPath,
}

impl RepoPath {
    pub fn open(&self) -> GitResult<Repo> {
        let mut repo = gix::open(&self.root).map_err(fail)?;
        repo.object_cache_size_if_unset(64 * 1024 * 1024);

        Ok(Repo {
            repo,
            path: self.clone(),
        })
    }

    pub fn file(&self) -> PathBuf { self.root.join(&self.rel) }
}

impl Repo {
    fn rel(&self) -> &BStr { self.path.rel.as_bytes().as_bstr() }

    pub fn web_links(&self) -> Option<WebLinks> {
        let remote = self
            .repo
            .find_fetch_remote(None)
            .ok()
            .or_else(|| self.repo.find_remote("origin").ok())?;

        web_links(remote.url(gix::remote::Direction::Fetch)?)
    }

    fn abbreviate(&self, id: ObjectId) -> String {
        id.attach(&self.repo)
            .shorten()
            .map_or_else(|_| short_hash(&id.to_string()), |prefix| prefix.to_string())
    }

    fn commit_info(&self, id: ObjectId) -> GitResult<CommitInfo> {
        let commit = self.repo.find_commit(id).map_err(fail)?;
        let author = commit.author().map_err(fail)?;
        let (pull_request, summary) = commit.message().map_or((None, String::new()), |message| {
            let body = message.body.map(|body| body.to_str_lossy());

            pull_request(&message.summary().to_str_lossy(), body.as_deref())
        });

        Ok(CommitInfo {
            hash: id.to_string(),
            short: self.abbreviate(id),
            author: author.name.to_str_lossy().into_owned(),
            time: commit.time().map_or(0, |time| time.seconds),
            summary,
            pull_request,
        })
    }

    pub fn branch(&self) -> GitResult<String> {
        if let Some(name) = self.repo.head_name().map_err(fail)? {
            return Ok(name.shorten().to_str_lossy().into_owned());
        }

        Ok(self.abbreviate(self.repo.head_id().map_err(fail)?.detach()))
    }

    pub fn head_ref(&self) -> Option<CommitRef> {
        let id = self.repo.head_id().ok()?.detach();
        self.commit_ref(&id.to_string()).ok()
    }

    /// A local or remote branch pointing at `id`
    fn branch_at(&self, id: ObjectId) -> Option<String> {
        let references = self.repo.references().ok()?;
        let local = references.local_branches().ok()?;
        let remote = references.remote_branches().ok()?;

        local.chain(remote).flatten().find_map(|mut reference| {
            (reference.peel_to_id().ok()?.detach() == id)
                .then(|| reference.name().shorten().to_str_lossy().into_owned())
        })
    }

    pub fn commit_ref(&self, hash: &str) -> GitResult<CommitRef> {
        let id = ObjectId::from_hex(hash.as_bytes()).map_err(fail)?;
        let info = self.commit_info(id)?;

        Ok(CommitRef {
            hash: info.hash,
            short: info.short,
            subject: info.summary,
            name: self.branch_at(id),
        })
    }

    pub fn operation(&self) -> Option<Operation> {
        let (kind, hash) = detect_operation(self.repo.git_dir())?;
        let theirs = self.commit_ref(&hash).unwrap_or_else(|_| CommitRef {
            short: short_hash(&hash),
            hash,
            subject: String::new(),
            name: None,
        });

        Some(Operation { kind, theirs })
    }

    /// gix-index panics instead of failing on a file too short to hold its checksum, so a
    /// truncated index has to be caught before it is read
    fn check_index(&self) -> GitResult<()> {
        const MINIMUM: u64 = 12 + 20;

        match std::fs::metadata(self.repo.index_path()) {
            Ok(metadata) if metadata.len() < MINIMUM => Err(GitError(String::from(
                "the Git index is truncated; run `git status` to see what Git reports",
            ))),
            _ => Ok(()),
        }
    }

    /// The index stages of the file, `None` when it isn't conflicted
    pub fn unmerged(&self) -> GitResult<Option<Stages>> {
        self.check_index()?;
        let index = self.repo.index_or_empty().map_err(fail)?;
        let rel = self.rel();
        let mut stages = Stages::default();
        let mut found = false;

        for entry in index.entries().iter().filter(|entry| entry.path(&index) == rel) {
            let slot = match entry.stage() {
                Stage::Unconflicted => continue,
                Stage::Base => &mut stages.base,
                Stage::Ours => &mut stages.ours,
                Stage::Theirs => &mut stages.theirs,
            };
            *slot = Some(entry.id);
            found = true;
        }

        Ok(found.then_some(stages))
    }

    pub fn blob(&self, id: ObjectId) -> GitResult<Vec<u8>> { Ok(self.repo.find_blob(id).map_err(fail)?.take_data()) }

    fn blob_at(&self, commit: ObjectId) -> GitResult<Option<ObjectId>> {
        let tree = self.repo.find_commit(commit).map_err(fail)?.tree_id().map_err(fail)?;
        let entry = self
            .repo
            .find_tree(tree)
            .map_err(fail)?
            .lookup_entry_by_path(&self.path.rel)
            .map_err(fail)?;

        Ok(entry
            .filter(|entry| entry.mode().is_blob())
            .map(|entry| entry.object_id()))
    }

    /// Versions of the file on the first parent chain of `HEAD`, newest first, one per commit that
    /// changed it. Stops after `limit` versions or when `cancelled` says so.
    pub fn file_history(&self, limit: usize, cancelled: &dyn Fn() -> bool) -> GitResult<Vec<FileVersion>> {
        let Ok(head) = self.repo.head_id() else {
            // worktree
            return Ok(Vec::new());
        };

        let walk = self
            .repo
            .rev_walk([head.detach()])
            .first_parent_only()
            .all()
            .map_err(fail)?;

        let mut versions = Vec::new();
        // The newer commit and its blob, waiting to see whether its parent differs
        let mut newer: Option<(ObjectId, Option<ObjectId>)> = None;
        for info in walk {
            if versions.len() >= limit || cancelled() {
                return Ok(versions);
            }

            let id = info.map_err(fail)?.id;
            let blob = self.blob_at(id)?;
            if let Some((newer_id, newer_blob)) = newer
                && newer_blob != blob
            {
                versions.push(FileVersion {
                    commit: self.commit_info(newer_id)?,
                    blob: newer_blob,
                });
            }
            newer = Some((id, blob));
        }

        // The root commit added whatever it has
        if let Some((id, Some(blob))) = newer
            && versions.len() < limit
        {
            versions.push(FileVersion {
                commit: self.commit_info(id)?,
                blob: Some(blob),
            });
        }

        Ok(versions)
    }

    /// Stages the work tree file in place of its conflict stages, like `git add`
    pub fn mark_resolved(&self) -> GitResult<()> {
        self.check_index()?;
        let mut index = self.repo.open_index().map_err(fail)?;
        let rel = self.rel().to_owned();
        if !index
            .entries()
            .iter()
            .any(|entry| entry.path(&index) == rel && entry.stage() != Stage::Unconflicted)
        {
            return Err(GitError(String::from("map is no longer unmerged in the Git index")));
        }
        let mode = index
            .entries()
            .iter()
            .filter(|entry| entry.path(&index) == rel)
            .find(|entry| entry.stage() == Stage::Ours)
            .map_or(Mode::FILE, |entry| entry.mode);

        let (mut pipeline, _) = self.repo.filter_pipeline(None).map_err(fail)?;
        let (id, ..) = pipeline
            .worktree_file_to_object(rel.as_bstr(), &index)
            .map_err(fail)?
            .ok_or_else(|| GitError(String::from("the saved map is missing from the worktree")))?;
        let metadata = gix::index::fs::Metadata::from_path_no_follow(&self.path.file()).map_err(fail)?;
        let stat = Stat::from_fs(&metadata).map_err(fail)?;

        index.remove_entries(|_, path, _| path == rel);
        index.dangerously_push_entry(stat, id, Flags::empty(), mode, rel.as_ref());
        index.sort_entries();
        // The cached trees would be written back as valid even though they are now stale
        index.remove_tree();
        index.write(gix::index::write::Options::default()).map_err(fail)?;

        Ok(())
    }
}

fn web_links(remote: &gix::Url) -> Option<WebLinks> {
    use gix::url::Scheme;

    let scheme = match remote.scheme {
        Scheme::Http => "http",
        Scheme::Https | Scheme::Ssh | Scheme::Git => "https",
        _ => return None,
    };
    let host = remote.host.as_deref()?;
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    let port = if matches!(remote.scheme, Scheme::Http | Scheme::Https) {
        remote.port.map_or(String::new(), |port| format!(":{port}"))
    } else {
        String::new()
    };
    let path = remote.path.to_str().ok()?;
    let path = path
        .trim_matches('/')
        .strip_suffix(".git")
        .unwrap_or(path.trim_matches('/'));
    if path.is_empty() {
        return None;
    }
    let mut encoded = String::new();
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }

    // surely this wont bite
    let forge = if host == "github.com" {
        Forge::GitHub
    } else if host == "bitbucket.org" {
        Forge::Bitbucket
    } else if host == "gitlab.com" || host.ends_with(".gitlab.com") {
        Forge::GitLab
    } else {
        Forge::Other
    };

    Some(WebLinks {
        repo: format!("{scheme}://{host}{port}/{encoded}"),
        forge,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::{Path, PathBuf};

    use super::{OperationKind, detect_operation, discover, pull_request, web_links};

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
            let links = web_links(&parsed).map(|links| (links.commit("abc"), links.pull_request(7)));
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
            let (number, text) = pull_request(summary, body);
            assert_eq!((number, text.as_str()), expected, "{summary}");
        }
    }

    /// Runs the git executable to build fixtures, `None` when it isn't installed
    pub(crate) fn git(dir: &Path, arguments: &[&str]) -> Option<String> {
        let output = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=rmd",
                "-c",
                "user.email=rmd@example.com",
                "-c",
                "core.autocrlf=false",
            ])
            .args(arguments)
            .current_dir(dir)
            .output()
            .ok()?;

        Some(String::from_utf8_lossy(&output.stdout).into_owned() + &String::from_utf8_lossy(&output.stderr))
    }

    pub(crate) fn git_available() -> bool { git(Path::new("."), &["--version"]).is_some() }

    pub(crate) fn temp_repo(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rmd-git-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q", "-b", "main"]);
        dir
    }

    pub(crate) fn commit(dir: &Path, file: &str, contents: &str, message: &str) {
        std::fs::write(dir.join(file), contents).unwrap();
        git(dir, &["add", file]);
        git(dir, &["commit", "-q", "-m", message]);
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

        let _ = std::fs::remove_dir_all(&dir);
    }
}
