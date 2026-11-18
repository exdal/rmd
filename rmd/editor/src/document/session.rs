use std::{
    io,
    path::{Path, PathBuf},
};

use base::path::TreePath;
use dmm::{Map, MapFormat, Size};
use prefab::Prefab;

use crate::git::session::GitDocState;
use crate::{coop::LevelStep, session::Session, tool::session::is_reorder_label};
use crate::{
    document::{DocumentId, MapDocument, command::Edit, grid::Grid},
    environment::worker::LoadedMap,
    git::conflict::ConflictState,
    tool::Tool,
};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SaveAllOutcome {
    pub needs_path: Option<DocumentId>,
    pub error: Option<String>,
}

impl Session {
    pub fn apply_map(&mut self, loaded: LoadedMap) {
        let LoadedMap {
            path,
            map,
            z,
            errors,
            repo,
            conflict,
        } = loaded;

        for error in &errors {
            log::error!("{}: {error}", path.display());
        }

        if let Some(id) = self.state.document_for_path(&path) {
            self.set_active_document(id);
            self.note_loaded_issues(id);

            return;
        }

        let pending_write = conflict.is_some();
        let document = if pending_write {
            MapDocument::open_modified(path, map, z)
        } else {
            MapDocument::open(path, map, z)
        };
        let id = self.activate_document(document);
        if pending_write {
            self.loaded_conflicts = Some(id);
        }
        self.caches.entry(id).or_default().git =
            repo.map(|repo| GitDocState::new(repo, conflict.map(ConflictState::new)));

        self.refresh_git_document(id);
        self.note_loaded_issues(id);
    }

    pub fn reload_map(&mut self, id: DocumentId, loaded: LoadedMap) -> bool {
        let LoadedMap {
            path,
            map,
            errors,
            repo,
            conflict,
            ..
        } = loaded;
        if self
            .state
            .document(id)
            .is_none_or(|document| document.path.as_deref() != Some(path.as_path()))
        {
            return false;
        }

        for error in &errors {
            log::error!("{}: {error}", path.display());
        }

        let pending_write = conflict.is_some();
        if pending_write {
            self.loaded_conflicts = Some(id);
        }
        self.state.document_mut(id).unwrap().replace_map(map, pending_write);
        self.caches.entry(id).or_default().git =
            repo.map(|repo| GitDocState::new(repo, conflict.map(ConflictState::new)));
        self.caches.entry(id).or_default().map_revision = self
            .caches
            .get(&id)
            .map_or(0, |cache| cache.map_revision.wrapping_add(1));
        self.set_active_document(id);
        self.rebake(id);
        self.refresh_git_document(id);
        self.note_loaded_issues(id);

        true
    }

    pub(crate) fn set_active_document(&mut self, id: DocumentId) -> bool {
        if self.state.document(id).is_none() {
            return false;
        }
        if self.state.active() != Some(id) {
            self.cancel_node_edit();
            if let Some(previous) = self.state.active() {
                self.set_document_module_bake_held(previous, false);
            }

            self.state.set_active(id);
        }

        true
    }

    pub fn close_map(&mut self, id: DocumentId) -> bool {
        if self.node_edit.as_ref().is_some_and(|edit| edit.document == id) {
            self.cancel_node_edit();
        }
        let closed = self.state.close_document(id).is_some();
        if closed {
            self.caches.remove(&id);
            self.git_worker.close(id);
        }

        if self.state.tool == Tool::Node && !self.node_tool_available() {
            self.cancel_node_edit();
            self.state.tool = Tool::Pick;
        }

        closed
    }

    pub fn create_map(&mut self, path: &Path, size: Size, format: MapFormat) -> Result<(), Box<dyn std::error::Error>> {
        if size.x == 0 || size.y == 0 || size.z == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "map dimensions must each be at least 1",
            )
            .into());
        }
        if !path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("dmm"))
        {
            return Err(
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "map path must use the .dmm extension").into(),
            );
        }
        if path.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("{} already exists", path.display()),
            )
            .into());
        }
        if !path.parent().is_some_and(Path::is_dir) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("parent directory for {} does not exist", path.display()),
            )
            .into());
        }

        let world = self
            .world()
            .ok_or_else(|| std::io::Error::other("no codebase is loaded"))?;
        let base_tile = world
            .base_tile
            .as_ref()
            .ok_or_else(|| std::io::Error::other("the codebase does not define usable /turf and /area types"))?;
        let mut map = Map::new(size);
        map.format = format;
        let key = map.intern_tile(vec![
            Prefab::new(base_tile.turf.clone()),
            Prefab::new(base_tile.area.clone()),
        ]);
        for cell in map.grid.iter_mut().flatten().flatten() {
            *cell = key;
        }

        self.activate_document(MapDocument::create(path, map, 1));

        Ok(())
    }

    pub fn first_map(&self) -> Option<PathBuf> {
        let environment = self.state.environment.as_ref()?;

        environment
            .maps
            .iter()
            .find(|path| path.is_file())
            .or_else(|| environment.maps.first())
            .cloned()
    }

    pub fn map(&self) -> Option<&Grid> {
        self.state.active_document().map(|document| &document.map)
    }

    pub fn map_prefabs_of(&self, id: DocumentId, path: &TreePath) -> Vec<Prefab> {
        let Some(document) = self.state.document(id) else {
            return Vec::new();
        };

        let mut prefabs = document.map.prefabs_of(path).cloned().collect::<Vec<_>>();
        prefabs.sort_by_cached_key(|prefab| (!document.map.is_placed(prefab), prefab.vars.len()));
        prefabs
    }

    pub fn is_prefab_placed(&self, id: DocumentId, prefab: &Prefab) -> bool {
        self.state
            .document(id)
            .is_some_and(|document| document.map.is_placed(prefab))
    }

    pub fn pin_prefabs(&mut self, id: DocumentId, prefabs: &[Prefab]) {
        let Some(document) = self.state.document_mut(id) else {
            return;
        };

        for prefab in prefabs {
            document.map.pin(prefab);
        }
    }

    pub fn map_path(&self) -> Option<&Path> {
        self.state
            .active_document()
            .and_then(|document| document.path.as_deref())
    }

    pub fn map_format(&self) -> Option<MapFormat> {
        self.state.active_document().map(MapDocument::format)
    }

    fn is_level_history_blocked(&self, id: DocumentId) -> bool {
        self.has_loaded_conflicts(id) || !self.can_step_coop_level(id)
    }

    fn next_step(&self, step: LevelStep) -> Option<&Edit> {
        let history = &self.state.active_document()?.history;
        match step {
            LevelStep::Undo => history.next_undo(),
            LevelStep::Redo => history.next_redo(),
            LevelStep::Change => None,
        }
    }

    fn step_label(&self, step: LevelStep) -> Option<&str> {
        let id = self.state.active()?;
        let document = self.state.document(id)?;
        if self.next_step(step)?.level().is_some() && self.is_level_history_blocked(id) {
            return None;
        }

        match step {
            LevelStep::Undo => document.undo_label(),
            LevelStep::Redo => document.redo_label(),
            LevelStep::Change => None,
        }
    }

    pub fn undo_label(&self) -> Option<&str> {
        self.step_label(LevelStep::Undo)
    }

    pub fn redo_label(&self) -> Option<&str> {
        self.step_label(LevelStep::Redo)
    }

    fn step(&mut self, step: LevelStep) -> bool {
        let reordered = self.step_label(step).is_some_and(is_reorder_label);
        let is_level_edit = self.next_step(step).is_some_and(|edit| edit.level().is_some());

        if is_level_edit {
            let Some(id) = self.state.active() else {
                return false;
            };

            if self.is_level_history_blocked(id) {
                return false;
            }

            // a shared map's level edits go through the server like they were made
            if let Some(is_requested) = self.request_coop_level_step(id, step) {
                return is_requested;
            }

            self.cancel_node_edit();
        }

        let resized = self.next_step(step).is_some_and(|edit| edit.resize().is_some());
        let Some(affected) = self.state.active_document_mut().and_then(|document| match step {
            LevelStep::Undo => document.undo_with_affected(),
            LevelStep::Redo => document.redo_with_affected(),
            LevelStep::Change => None,
        }) else {
            return false;
        };

        if is_level_edit {
            if let Some(id) = self.state.active() {
                self.rebake_levels(id);
            }
        } else if resized {
            if let Some(id) = self.state.active() {
                self.rebake(id);
            }
        } else if reordered {
            self.refresh_reordered_from_affected(&affected);
        } else {
            self.update_instances(&affected);
        }

        if let Some(id) = self.state.active()
            && let Some(cache) = self.caches.get_mut(&id)
        {
            cache.map_revision = cache.map_revision.wrapping_add(1);
        }

        true
    }

    pub fn undo(&mut self) -> bool {
        self.step(LevelStep::Undo)
    }

    pub fn redo(&mut self) -> bool {
        self.step(LevelStep::Redo)
    }

    pub fn can_save_map_in_place(&self) -> bool {
        self.state
            .active_document()
            .is_some_and(|document| document.path.is_some() && !document.needs_initial_save())
    }

    pub fn save_map(&mut self) -> std::io::Result<()> {
        let Some(id) = self.state.active() else {
            return Err(std::io::Error::other("no map is open"));
        };

        self.save_document(id)
    }

    pub fn save_all(&mut self) -> SaveAllOutcome {
        let mut outcome = SaveAllOutcome::default();
        for id in self.state.document_ids() {
            let Some(document) = self.state.document(id).filter(|document| document.is_dirty()) else {
                continue;
            };
            if document.path.is_none() || document.needs_initial_save() {
                outcome.needs_path.get_or_insert(id);
                continue;
            }
            let title = document.title();
            if let Err(error) = self.save_document(id) {
                outcome.error.get_or_insert_with(|| format!("{title}: {error}"));
            }
        }

        outcome
    }

    pub fn save_document(&mut self, id: DocumentId) -> std::io::Result<()> {
        self.write_document(id, None)
    }

    pub fn save_map_as(&mut self, path: &Path, format: MapFormat) -> std::io::Result<()> {
        let Some(id) = self.state.active() else {
            return Err(std::io::Error::other("no map is open"));
        };

        self.write_document(id, Some((path, format)))
    }

    fn write_document(&mut self, id: DocumentId, target: Option<(&Path, MapFormat)>) -> io::Result<()> {
        let environment = self.state.environment.clone();
        let sanitized = environment
            .as_deref()
            .filter(|_| self.sanitize_vars_on_save)
            .map(|environment| &environment.tree);

        let document = self
            .state
            .document_mut(id)
            .filter(|document| !document.is_stash())
            .ok_or_else(|| io::Error::other("no map is open"))?;

        match target {
            Some((path, format)) => document.save_as_with(path, format, sanitized),
            None => document.save_with(sanitized),
        }
    }

    pub fn set_sanitize_vars_on_save(&mut self, enabled: bool) {
        self.sanitize_vars_on_save = enabled;
    }

    pub(crate) fn activate_document(&mut self, document: MapDocument) -> DocumentId {
        self.cancel_node_edit();
        let id = self.state.open_document(document);
        self.rebake(id);

        id
    }
}

#[cfg(test)]
mod tests;
