use std::{cell::RefCell, collections::HashMap, sync::Arc};

use dmm::Coord;
use editor::{
    command::EditGroupId,
    conflict::{ConflictRow, ConflictState, Region, Resolved, Side},
    document::DocumentId,
    git::{CommitInfo, CommitRef, Operation, RepoPath, WebLinks},
    tool::ToolEdit,
};

use super::{BlameState, DiffSide, DiffState, Session, SharedHighlights};
use crate::git_worker::Outcome as GitOutcome;

/// Each conflict row's resolution, indexed like `ConflictState::rows`
pub(crate) type ConflictStatus = Arc<[Option<Side>]>;

/// Conflict regions and overlays derived for one point in the history
pub(super) struct ConflictView {
    key: (u64, u64, usize),
    resolved: Resolved,
    unresolved: usize,
    status: ConflictStatus,
    regions: HashMap<Option<u32>, Arc<[Region]>>,
    highlights: HashMap<u32, SharedHighlights>,
}

impl ConflictView {
    fn regions(&mut self, conflicts: &ConflictState, z: Option<u32>) -> Arc<[Region]> {
        self.regions
            .entry(z)
            .or_insert_with(|| conflicts.regions_in(z, &self.resolved).into())
            .clone()
    }

    pub(super) fn highlights(&mut self, conflicts: &ConflictState, z: u32) -> SharedHighlights {
        self.highlights
            .entry(z)
            .or_insert_with(|| conflicts.highlights_in(z, &self.resolved).into())
            .clone()
    }
}

pub(crate) struct GitDocState {
    pub repo: RepoPath,
    pub conflicts: Option<ConflictState>,
    pub unmerged_on_disk: bool,
    pub pending_load: bool,
    pub error: Option<String>,
    pub branch: Option<String>,
    pub head: Option<CommitRef>,
    pub operation: Option<Operation>,
    pub web: Option<WebLinks>,
    pub blame: Option<BlameState>,
    pub show_blame: bool,
    pub staging: bool,
    /// Commits that changed the map, for picking what to compare
    pub history: Option<Vec<CommitInfo>>,
    pub(super) history_loading: bool,
    pub diff: Option<DiffState>,
    pub show_diff: bool,
    pub diff_loading: bool,
    conflict_view: RefCell<Option<ConflictView>>,
}

impl GitDocState {
    pub(super) fn new(repo: RepoPath, conflicts: Option<ConflictState>) -> Self {
        let unmerged_on_disk = conflicts.is_some();

        Self {
            repo,
            conflicts,
            unmerged_on_disk,
            pending_load: false,
            error: None,
            branch: None,
            head: None,
            operation: None,
            web: None,
            blame: None,
            show_blame: false,
            staging: false,
            history: None,
            history_loading: false,
            diff: None,
            show_diff: false,
            diff_loading: false,
            conflict_view: RefCell::new(None),
        }
    }

    /// Runs `read` against the view for the current history, rebuilding it after any change
    fn with_conflict_view<R>(
        &self, map_revision: u64, history: &editor::command::History,
        read: impl FnOnce(&mut ConflictView, &ConflictState) -> R,
    ) -> Option<R> {
        let conflicts = self.conflicts.as_ref()?;
        let key = (map_revision, conflicts.revision(), history.undo_depth());
        let mut view = self.conflict_view.borrow_mut();
        if view.as_ref().is_none_or(|view| view.key != key) {
            let resolved = conflicts.resolved(history);
            *view = Some(ConflictView {
                key,
                unresolved: conflicts.unresolved_in(&resolved).count(),
                status: conflicts.rows().iter().map(|row| resolved.side(row.coord)).collect(),
                resolved,
                regions: HashMap::new(),
                highlights: HashMap::new(),
            });
        }

        view.as_mut().map(|view| read(view, conflicts))
    }
}

impl Session {
    pub(super) fn refresh_git_document(&mut self, id: DocumentId) {
        if !self.git_enabled {
            return;
        }

        let Some(git) = self.caches.get_mut(&id).and_then(|cache| cache.git.as_mut()) else {
            return;
        };

        // commits or checkouts change which revisions touched the map
        git.history = None;
        self.git_worker.status(id, git.repo.clone());
    }

    pub(crate) fn refresh_git(&mut self) {
        for id in self.state.document_ids() {
            self.refresh_git_document(id);
        }
    }

    pub(crate) fn sync_git_enabled(&mut self, enabled: bool) {
        if self.git_enabled == enabled {
            return;
        }

        self.git_enabled = enabled;

        for id in self.state.document_ids() {
            if !enabled {
                self.git_worker.close(id);
                if let Some(git) = self.caches.get_mut(&id).and_then(|cache| cache.git.as_mut()) {
                    git.staging = false;
                }
            } else {
                let repo = self
                    .caches
                    .get(&id)
                    .and_then(|cache| cache.git.as_ref())
                    .is_none()
                    .then(|| {
                        self.state
                            .document(id)
                            .and_then(|document| document.path.as_deref())
                            .and_then(editor::git::discover)
                    })
                    .flatten();

                if let Some(repo) = repo {
                    self.caches.entry(id).or_default().git = Some(GitDocState::new(repo, None));
                }

                self.refresh_git_document(id);
            }
        }
    }

    /// Why the map cannot be marked resolved yet, `None` when it can
    pub(crate) fn mark_resolved_blocker(&self, id: DocumentId) -> Option<String> {
        let Some(git) = self.git_state(id) else {
            return Some(String::from("Not in a Git worktree"));
        };
        let Some(document) = self.state.document(id) else {
            return Some(String::from("No map selected"));
        };

        if git.staging {
            return Some(String::from("Staging..."));
        }
        if !git.unmerged_on_disk {
            return Some(String::from("Git does not list this map as unmerged"));
        }
        let Some(conflicts) = git.conflicts.as_ref() else {
            return Some(String::from("No conflicts loaded"));
        };

        match self.unresolved_conflicts(id) {
            0 if document.is_dirty() => Some(String::from("Save the map first")),
            0 => None,
            1 => Some(String::from("1 tile is still unresolved")),
            count => Some(format!("{count} of {} tiles are still unresolved", conflicts.len())),
        }
    }

    pub(crate) fn mark_resolved(&mut self, id: DocumentId) -> bool {
        if self.mark_resolved_blocker(id).is_some() {
            return false;
        }

        let Some(git) = self.caches.get_mut(&id).and_then(|cache| cache.git.as_mut()) else {
            return false;
        };

        git.staging = true;
        self.git_worker.mark_resolved(id, git.repo.clone());

        true
    }

    pub(crate) fn poll_git(&mut self) {
        let mut refresh_after = Vec::new();

        for finished in self.git_worker.poll() {
            let Some(document) = self.state.document(finished.document) else {
                continue;
            };
            let document_dirty = document.is_dirty();
            let Some(cache) = self.caches.get_mut(&finished.document) else {
                continue;
            };
            let Some(git) = cache.git.as_mut() else { continue };

            match finished.outcome {
                GitOutcome::Status(result) => match result {
                    Ok(status) => {
                        git.branch = Some(status.branch);
                        git.head = status.head;
                        git.operation = status.operation;
                        git.web = status.web;
                        git.unmerged_on_disk = status.unmerged;

                        if !status.unmerged && !document_dirty {
                            git.conflicts = None;
                        }

                        git.pending_load = status.unmerged && git.conflicts.is_none();
                        git.error = None;
                    },
                    Err(error) => git.error = Some(error.to_string()),
                },
                GitOutcome::Blame {
                    revision,
                    snapshot,
                    result,
                } => match result {
                    Ok(result) => git.blame = Some(BlameState::new(result, snapshot, revision)),
                    Err(error) if error.0 != "cancelled" => git.error = Some(error.to_string()),
                    Err(_) => {},
                },
                GitOutcome::History(result) => {
                    git.history_loading = false;
                    git.history = Some(result.unwrap_or_else(|error| {
                        git.error = Some(error.to_string());

                        Vec::new()
                    }));
                },
                GitOutcome::Diff { from, to, result } => {
                    git.diff_loading = false;
                    match result {
                        Ok(sides) => {
                            let (older, newer) = *sides;
                            git.diff = Some(DiffState::new(DiffSide::new(from, older), DiffSide::new(to, newer)));
                        },
                        Err(error) => git.error = Some(error.to_string()),
                    }
                },
                GitOutcome::MarkResolved(result) => {
                    git.staging = false;
                    match result {
                        Ok(()) => {
                            git.unmerged_on_disk = false;
                            git.conflicts = None;
                            git.pending_load = false;
                            refresh_after.push(finished.document);
                        },
                        Err(error) => git.error = Some(error.to_string()),
                    }
                },
            }
        }

        for id in refresh_after {
            self.refresh_git_document(id);
        }
    }

    pub(crate) fn git_state(&self, id: DocumentId) -> Option<&GitDocState> {
        if !self.git_enabled {
            return None;
        }

        self.caches.get(&id)?.git.as_ref()
    }

    pub(super) fn with_conflict_view<R>(
        &self, id: DocumentId, read: impl FnOnce(&mut ConflictView, &ConflictState) -> R,
    ) -> Option<R> {
        let document = self.state.document(id)?;
        let cache = self.caches.get(&id)?;
        let git = cache.git.as_ref().filter(|_| self.git_enabled)?;

        git.with_conflict_view(cache.map_revision, &document.history, read)
    }

    /// unresolved regions, cached until the history or the resolutions change
    pub(crate) fn conflict_regions(&self, id: DocumentId, z: Option<u32>) -> Arc<[Region]> {
        self.with_conflict_view(id, |view, conflicts| view.regions(conflicts, z))
            .unwrap_or_else(|| Arc::new([]))
    }

    pub(crate) fn unresolved_conflicts(&self, id: DocumentId) -> usize {
        self.with_conflict_view(id, |view, _| view.unresolved).unwrap_or(0)
    }

    /// Every conflict row with its current resolution, in the same order
    pub(crate) fn conflict_table(&self, id: DocumentId) -> Option<(&[ConflictRow], ConflictStatus)> {
        let conflicts = self.git_state(id)?.conflicts.as_ref()?;
        let status = self.with_conflict_view(id, |view, _| view.status.clone())?;

        Some((conflicts.rows(), status))
    }

    /// The map whose merge conflicts were loaded since the last call
    pub(crate) fn take_loaded_conflicts(&mut self) -> Option<DocumentId> { self.loaded_conflicts.take() }

    pub(crate) fn clear_git_error(&mut self, id: DocumentId) {
        if let Some(git) = self.caches.get_mut(&id).and_then(|cache| cache.git.as_mut()) {
            git.error = None;
        }
    }

    pub(crate) fn resolve_conflict(&mut self, id: DocumentId, coords: &[Coord], side: Side) -> bool {
        if !self.git_enabled {
            return false;
        }

        if self.state.active() != Some(id) {
            return false;
        }

        let edit = {
            let Some(conflicts) = self
                .caches
                .get(&id)
                .and_then(|cache| cache.git.as_ref())
                .and_then(|git| git.conflicts.as_ref())
            else {
                return false;
            };
            let Some(document) = self.state.document_mut(id) else {
                return false;
            };

            conflicts.resolve_edit(document, coords, side)
        };
        let Some(edit) = edit else { return false };
        let affected = edit.affected_instances();
        let mark = EditGroupId::new();
        if !self.commit(
            ToolEdit {
                edit,
                selected: None,
                affected,
            },
            Some(mark),
        ) {
            return false;
        }

        if let Some(conflicts) = self
            .caches
            .get_mut(&id)
            .and_then(|cache| cache.git.as_mut())
            .and_then(|git| git.conflicts.as_mut())
        {
            conflicts.record(coords, side, mark);
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use dmm::{Coord, Map, Size};

    use crate::session::{Session, fixtures::conflicted_session};

    #[test]
    fn the_conflict_table_and_mark_resolved_blocker_follow_resolutions() {
        let mut session = conflicted_session();
        let id = session.state.active().unwrap();
        assert_eq!(session.take_loaded_conflicts(), Some(id));
        assert_eq!(session.take_loaded_conflicts(), None);

        let (rows, status) = session.conflict_table(id).unwrap();
        assert_eq!(
            rows.iter().map(|row| (row.coord.x, row.region)).collect::<Vec<_>>(),
            [(1, 1), (3, 2)]
        );
        assert_eq!(status.as_ref(), [None, None]);
        assert_eq!(
            session.mark_resolved_blocker(id).as_deref(),
            Some("2 of 2 tiles are still unresolved")
        );

        let all = [Coord::new(1, 1, 1), Coord::new(3, 1, 1)];
        assert!(session.resolve_conflict(id, &all, editor::conflict::Side::Theirs));
        let (_, status) = session.conflict_table(id).unwrap();
        assert_eq!(status.as_ref(), [Some(editor::conflict::Side::Theirs); 2]);
        assert_eq!(session.mark_resolved_blocker(id).as_deref(), Some("Save the map first"));
        assert!(!session.mark_resolved(id));

        session.caches.get_mut(&id).unwrap().git.as_mut().unwrap().staging = true;
        assert_eq!(session.mark_resolved_blocker(id).as_deref(), Some("Staging..."));

        session.sync_git_enabled(false);
        assert!(session.conflict_table(id).is_none());
        assert_eq!(
            session.mark_resolved_blocker(id).as_deref(),
            Some("Not in a Git worktree")
        );
    }

    #[test]
    fn cached_conflict_regions_follow_resolutions_through_undo_and_redo() {
        let mut session = conflicted_session();
        let id = session.state.active().unwrap();
        assert_eq!(session.conflict_regions(id, Some(1)).len(), 2);
        assert_eq!(session.unresolved_conflicts(id), 2);

        assert!(session.resolve_conflict(id, &[Coord::new(1, 1, 1)], editor::conflict::Side::Theirs));
        assert_eq!(session.conflict_regions(id, Some(1)).len(), 1);
        assert_eq!(session.unresolved_conflicts(id), 1);

        assert!(session.undo());
        assert_eq!(session.conflict_regions(id, Some(1)).len(), 2);
        assert_eq!(session.unresolved_conflicts(id), 2);

        assert!(session.redo());
        assert_eq!(session.unresolved_conflicts(id), 1);
        let highlights = session.highlights(id, None).iter().cloned().collect::<Vec<_>>();
        assert!(
            highlights
                .iter()
                .any(|highlight| !highlight.outline && highlight.tiles.len() == 1),
            "the taken tile keeps a resolved wash"
        );
    }

    #[test]
    fn disabling_git_releases_a_document_waiting_on_staging() {
        let mut session = Session::new();
        session.apply_map(crate::loader::LoadedMap {
            path: PathBuf::from("staging.dmm"),
            map: Map::new(Size { x: 1, y: 1, z: 1 }),
            z: 1,
            errors: Vec::new(),
            repo: Some(editor::git::RepoPath {
                root: PathBuf::from("missing-repository"),
                git_dir: PathBuf::from("missing-repository/.git"),
                rel: String::from("staging.dmm"),
            }),
            conflict: None,
        });
        let id = session.state.active().unwrap();
        session.caches.get_mut(&id).unwrap().git.as_mut().unwrap().staging = true;

        session.sync_git_enabled(false);
        session.sync_git_enabled(true);

        assert!(
            !session.git_state(id).unwrap().staging,
            "the dropped job can never report back"
        );
    }
}
