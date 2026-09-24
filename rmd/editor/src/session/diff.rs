use std::{cell::RefCell, collections::HashMap, sync::Arc};

use dmm::{Coord, Map, Tile};
use editor::{
    diff::{self, ChangeKind, DiffSource, MapDiff},
    document::DocumentId,
    git::CommitInfo,
    tool::ToolEdit,
};

use super::{Session, SharedHighlights};
use crate::git_worker::Revision;

/// One side of a loaded comparison
pub(crate) struct DiffSide {
    pub source: DiffSource,
    /// What the revision resolved to, `None` for the working map
    pub commit: Option<CommitInfo>,
    map: Option<Map>,
}

impl DiffSide {
    pub(super) fn new(source: DiffSource, revision: Option<Revision>) -> Self {
        let (commit, map) = revision.map_or((None, None), |revision| (Some(revision.commit), revision.map));

        Self { source, commit, map }
    }

    pub fn is_working(&self) -> bool { self.source == DiffSource::Working }

    /// `HEAD` or `main` as typed, a short hash for commits picked from the history
    pub fn label(&self) -> String {
        match (&self.source, &self.commit) {
            (DiffSource::Working, _) => String::from("working map"),
            (DiffSource::Revision(spec), Some(commit)) if *spec == commit.hash => commit.short.clone(),
            (DiffSource::Revision(spec), _) => spec.clone(),
        }
    }

    fn map<'a>(&'a self, working: &'a Map) -> Option<&'a Map> {
        if self.is_working() {
            Some(working)
        } else {
            self.map.as_ref()
        }
    }
}

pub(super) struct DiffView {
    key: u64,
    pub(super) diff: Arc<MapDiff>,
    pub(super) highlights: HashMap<u32, SharedHighlights>,
}

pub(crate) struct DiffState {
    pub from: DiffSide,
    pub to: DiffSide,
    /// Rebuilt whenever the working map changes, when one side is the working map
    view: RefCell<Option<DiffView>>,
}

impl DiffState {
    pub(super) fn new(from: DiffSide, to: DiffSide) -> Self {
        Self {
            from,
            to,
            view: RefCell::new(None),
        }
    }

    /// Whether tiles can be restored from `from` onto the map
    pub fn restorable(&self) -> bool { self.to.is_working() && !self.from.is_working() }

    fn with_view<R>(&self, map_revision: u64, working: &Map, read: impl FnOnce(&mut DiffView) -> R) -> R {
        let key = if self.from.is_working() || self.to.is_working() {
            map_revision
        } else {
            0
        };
        let mut view = self.view.borrow_mut();
        if view.as_ref().is_some_and(|view| view.key != key) {
            *view = None;
        }

        let view = view.get_or_insert_with(|| DiffView {
            key,
            diff: Arc::new(diff::diff(self.from.map(working), self.to.map(working))),
            highlights: HashMap::new(),
        });

        read(view)
    }
}

impl Session {
    /// Lists the commits that changed the map, once per refresh
    pub(crate) fn load_history(&mut self, id: DocumentId, limit: usize) {
        if !self.git_enabled {
            return;
        }

        let Some(git) = self.caches.get_mut(&id).and_then(|cache| cache.git.as_mut()) else {
            return;
        };
        if git.history.is_some() || git.history_loading {
            return;
        }

        git.history_loading = true;
        self.git_worker.history(id, git.repo.clone(), limit);
    }

    pub(crate) fn run_diff(&mut self, id: DocumentId, from: DiffSource, to: DiffSource) {
        if !self.git_enabled {
            return;
        }

        let Some(git) = self.caches.get_mut(&id).and_then(|cache| cache.git.as_mut()) else {
            return;
        };

        git.diff_loading = true;
        git.show_diff = true;
        git.show_blame = false;
        self.git_worker.diff(id, git.repo.clone(), from, to);
    }

    /// Shows or hides the diff overlay, comparing `HEAD` with the working map the first time
    pub(crate) fn toggle_diff(&mut self, id: DocumentId) {
        let Some(git) = self.caches.get_mut(&id).and_then(|cache| cache.git.as_mut()) else {
            return;
        };

        git.show_diff = !git.show_diff;
        if !git.show_diff {
            return;
        }

        git.show_blame = false;
        if git.diff.is_none() && !git.diff_loading {
            self.run_diff(id, DiffSource::head(), DiffSource::Working);
        }
    }

    pub(super) fn with_diff_view<R>(&self, id: DocumentId, read: impl FnOnce(&mut DiffView) -> R) -> Option<R> {
        let document = self.state.document(id)?;
        let cache = self.caches.get(&id)?;
        let diff = cache.git.as_ref().filter(|_| self.git_enabled)?.diff.as_ref()?;

        Some(diff.with_view(cache.map_revision, &document.map, read))
    }

    /// The loaded comparison, recomputed after edits when one side is the working map
    pub(crate) fn diff(&self, id: DocumentId) -> Option<Arc<MapDiff>> {
        self.with_diff_view(id, |view| Arc::clone(&view.diff))
    }

    /// How a tile changed, with its old and new contents, `None` outside a map
    pub(crate) fn diff_at(&self, id: DocumentId, coord: Coord) -> Option<(ChangeKind, Option<&Tile>, Option<&Tile>)> {
        let kind = self.with_diff_view(id, |view| view.diff.at(coord))??;
        let document = self.state.document(id)?;
        let diff = self.git_state(id)?.diff.as_ref()?;
        let from = diff.from.map(&document.map).and_then(|map| map.tile_at(coord));
        let to = diff.to.map(&document.map).and_then(|map| map.tile_at(coord));

        Some((kind, from, to))
    }

    /// Puts the tiles `from` had back at `coords` as one undoable edit
    pub(crate) fn restore_diff(&mut self, id: DocumentId, coords: &[Coord]) -> bool {
        if !self.git_enabled || self.state.active() != Some(id) {
            return false;
        }

        let edit = {
            let Some(diff) = self
                .caches
                .get(&id)
                .and_then(|cache| cache.git.as_ref())
                .and_then(|git| git.diff.as_ref())
                .filter(|diff| diff.restorable())
            else {
                return false;
            };
            let Some(document) = self.state.document_mut(id) else {
                return false;
            };
            let label = format!("Restore from {}", diff.from.label());

            diff::restore_edit(document, diff.from.map.as_ref(), coords, &label)
        };
        let Some(edit) = edit else { return false };
        let affected = edit.affected_instances();

        self.commit(
            ToolEdit {
                edit,
                selected: None,
                affected,
            },
            None,
        )
    }
}

#[cfg(test)]
mod tests {
    use dmm::Coord;
    use editor::diff::{ChangeKind, MODIFIED_COLOR};

    use crate::session::{
        Session,
        fixtures::{conflicted_session, install_diff},
    };

    #[test]
    fn a_working_map_diff_follows_restores_through_undo() {
        let mut session = conflicted_session();
        let id = session.state.active().unwrap();
        let coord = Coord::new(2, 1, 1);
        install_diff(&mut session, id);
        session.caches.get_mut(&id).unwrap().git.as_mut().unwrap().show_diff = false;

        let changes = |session: &Session| session.diff(id).unwrap().len();
        assert_eq!(changes(&session), 1);
        assert_eq!(
            session.diff_at(id, coord).map(|(kind, ..)| kind),
            Some(ChangeKind::Modified)
        );
        assert_eq!(session.highlights(id, None).iter().count(), 0, "hidden until shown");

        session.caches.get_mut(&id).unwrap().git.as_mut().unwrap().show_diff = true;
        let highlights = session.highlights(id, None).iter().cloned().collect::<Vec<_>>();
        assert_eq!(highlights.len(), 1);
        assert_eq!(highlights[0].color, MODIFIED_COLOR);

        assert!(session.restore_diff(id, &[coord]));
        assert_eq!(session.undo_label(), Some("Restore from HEAD"));
        assert_eq!(changes(&session), 0);
        assert!(!session.restore_diff(id, &[coord]), "nothing left to restore");

        assert!(session.undo());
        assert_eq!(changes(&session), 1);
    }
}
