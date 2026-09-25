use std::path::{Path, PathBuf};

use dmm::{Map, MapFormat, Prefab, Size};
use editor::{
    conflict::ConflictState,
    document::{DocumentId, MapDocument},
    tool::{Tool, default_tile_paths},
};

use super::{GitDocState, MAX_MAP_DIMENSION, Session, is_reorder_label};
use crate::loader::LoadedMap;

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

        true
    }

    pub(crate) fn set_active_document(&mut self, id: DocumentId) -> bool {
        if self.state.document(id).is_none() {
            return false;
        }
        if self.state.active() != Some(id) {
            self.cancel_node_edit();
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
            self.state.tool = Tool::Select;
        }

        closed
    }

    pub fn create_map(&mut self, path: &Path, size: Size, format: MapFormat) -> Result<(), Box<dyn std::error::Error>> {
        if size.x == 0
            || size.y == 0
            || size.z == 0
            || size.x > MAX_MAP_DIMENSION
            || size.y > MAX_MAP_DIMENSION
            || size.z > MAX_MAP_DIMENSION
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("map dimensions must each be between 1 and {MAX_MAP_DIMENSION}"),
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

        let tree = self
            .tree()
            .ok_or_else(|| std::io::Error::other("no codebase is loaded"))?;
        let (turf, area) = default_tile_paths(tree)
            .ok_or_else(|| std::io::Error::other("the codebase does not define usable /turf and /area types"))?;
        let mut map = Map::new(size);
        map.format = format;
        let key = map.intern_tile(vec![Prefab::new(turf), Prefab::new(area)]);
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

    pub fn map(&self) -> Option<&Map> { self.state.active_document().map(|document| &document.map) }

    pub fn map_path(&self) -> Option<&Path> {
        self.state
            .active_document()
            .and_then(|document| document.path.as_deref())
    }

    pub fn map_format(&self) -> Option<MapFormat> { self.map().map(|map| map.format) }

    pub fn undo_label(&self) -> Option<&str> { self.state.active_document()?.undo_label() }

    pub fn redo_label(&self) -> Option<&str> { self.state.active_document()?.redo_label() }

    pub fn undo(&mut self) -> bool {
        let reordered = self.undo_label().is_some_and(is_reorder_label);
        let resized = self
            .state
            .active_document()
            .and_then(|document| document.history.next_undo())
            .is_some_and(|edit| edit.resize().is_some());
        let Some(affected) = self
            .state
            .active_document_mut()
            .and_then(MapDocument::undo_with_affected)
        else {
            return false;
        };

        if resized {
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

    pub fn redo(&mut self) -> bool {
        let reordered = self.redo_label().is_some_and(is_reorder_label);
        let resized = self
            .state
            .active_document()
            .and_then(|document| document.history.next_redo())
            .is_some_and(|edit| edit.resize().is_some());
        let Some(affected) = self
            .state
            .active_document_mut()
            .and_then(MapDocument::redo_with_affected)
        else {
            return false;
        };

        if resized {
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
                outcome
                    .error
                    .get_or_insert_with(|| format!("{}: {error}", title.trim_end_matches(" *")));
            }
        }

        outcome
    }

    fn save_document(&mut self, id: DocumentId) -> std::io::Result<()> { self.write_document(id, None) }

    pub fn save_map_as(&mut self, path: &Path, format: MapFormat) -> std::io::Result<()> {
        let Some(id) = self.state.active() else {
            return Err(std::io::Error::other("no map is open"));
        };

        self.write_document(id, Some((path, format)))
    }

    fn write_document(&mut self, id: DocumentId, target: Option<(&Path, MapFormat)>) -> std::io::Result<()> {
        let environment = self.state.environment.clone().filter(|_| self.sanitize_vars_on_save);
        let sanitize = environment.as_deref().map(|environment| &environment.tree);
        let levels = self.state.document(id).map_or(0, |document| document.map.size.z);
        let document = self
            .state
            .document_mut(id)
            .ok_or_else(|| std::io::Error::other("no map is open"))?;
        let result = match target {
            Some((path, format)) => document.save_as_with(path, format, sanitize),
            None => document.save_with(sanitize),
        };

        if result.is_ok()
            && self
                .state
                .document(id)
                .is_some_and(|document| document.map.size.z != levels)
        {
            self.rebake(id);
        }

        result
    }

    pub fn set_sanitize_vars_on_save(&mut self, enabled: bool) { self.sanitize_vars_on_save = enabled; }

    pub(super) fn activate_document(&mut self, document: MapDocument) -> DocumentId {
        self.cancel_node_edit();
        let id = self.state.open_document(document);
        self.rebake(id);

        id
    }
}

#[cfg(test)]
mod tests {
    use core::{path::TreePath, types::Value};

    use dmm::{Coord, Map, MapFormat, Prefab, Size};
    use editor::{document::MapDocument, tool::Tool};

    use crate::session::{
        Session,
        fixtures::{assert_render_cache_matches_rebuild, examples, flat_session},
    };

    #[test]
    fn session_undo_and_redo_update_the_active_render_cache() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        session.open_map(&root.join("test.dmm"), 1).unwrap();
        let coord = Coord::new(2, 2, 1);
        let before = session.state.active_document().unwrap().placed_tile(coord).unwrap();
        let table = session
            .tree()
            .unwrap()
            .id_of(&TreePath::parse("/obj/structure/table"))
            .unwrap();

        assert_eq!(session.undo_label(), None);
        assert_eq!(session.redo_label(), None);
        assert!(session.choose_type(table));
        let placed = session.place_at(coord, None).unwrap();
        assert_eq!(session.undo_label(), Some("place /obj/structure/table"));
        assert!(session.state.active_document().unwrap().is_dirty());

        let edit_revision = session.revision();
        assert!(session.undo());
        assert_eq!(
            session.state.active_document().unwrap().placed_tile(coord),
            Some(before)
        );
        assert_eq!(session.redo_label(), Some("place /obj/structure/table"));
        assert!(!session.state.active_document().unwrap().is_dirty());
        assert_eq!(session.revision(), edit_revision.wrapping_add(1));
        assert_render_cache_matches_rebuild(&session);

        let undo_revision = session.revision();
        assert!(session.redo());
        assert_eq!(
            session
                .state
                .active_document()
                .unwrap()
                .instance_location(placed)
                .map(|location| location.coord),
            Some(coord)
        );
        assert_eq!(session.undo_label(), Some("place /obj/structure/table"));
        assert_eq!(session.redo_label(), None);
        assert!(session.state.active_document().unwrap().is_dirty());
        assert_eq!(session.revision(), undo_revision.wrapping_add(1));
        assert_render_cache_matches_rebuild(&session);
    }

    #[test]
    fn session_history_commands_only_change_the_active_document() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        session.open_map(&root.join("test.dmm"), 1).unwrap();
        let first = session.state.active().unwrap();
        let coord = Coord::new(2, 2, 1);
        let first_before = session.state.active_document().unwrap().placed_tile(coord).unwrap();
        let table = session
            .tree()
            .unwrap()
            .id_of(&TreePath::parse("/obj/structure/table"))
            .unwrap();
        assert!(session.choose_type(table));
        session.place_at(coord, None).unwrap();

        session.open_map(&root.join("test2.dmm"), 1).unwrap();
        let second = session.state.active().unwrap();
        session.place_at(coord, None).unwrap();
        let second_after = session.state.active_document().unwrap().placed_tile(coord).unwrap();

        session.state.set_active(first);
        assert!(session.undo());

        assert_eq!(
            session.state.document(first).unwrap().placed_tile(coord),
            Some(first_before)
        );
        assert_eq!(
            session.state.document(second).unwrap().placed_tile(coord),
            Some(second_after)
        );
        assert_render_cache_matches_rebuild(&session);
    }

    #[test]
    fn reopening_an_open_map_focuses_it_instead_of_duplicating_it() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).expect("codebase");
        session.open_map(&root.join("test.dmm"), 1).expect("first map");
        let first = session.state.active().expect("first is active");
        session.open_map(&root.join("test2.dmm"), 1).expect("second map");

        session
            .open_map(&root.join("test.dmm"), 1)
            .expect("reopen the first map");

        assert_eq!(session.state.document_ids().len(), 2, "no duplicate document");
        assert_eq!(session.state.active(), Some(first), "the existing tab is focused");
    }

    #[test]
    fn closing_a_map_drops_its_render_cache() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).expect("codebase");
        session.open_map(&root.join("test.dmm"), 1).expect("first map");
        let first = session.state.active().expect("first is active");
        session.open_map(&root.join("test2.dmm"), 1).expect("second map");
        let second = session.state.active().expect("second is active");

        assert!(session.close_map(first));

        assert!(!session.caches.contains_key(&first));
        assert!(session.caches.contains_key(&second));
        assert_eq!(session.state.active(), Some(second));
        assert!(
            session
                .map_view_frame(
                    first,
                    render::MapViewRect::default(),
                    render::Camera::default(),
                    Default::default(),
                    &[],
                    &[],
                )
                .is_none()
        );
    }

    #[test]
    fn a_new_map_uses_the_requested_size_and_codebase_tile_defaults() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        let path = std::env::temp_dir().join(format!("rmd-new-map-{}.dmm", std::process::id()));
        let _ = std::fs::remove_file(&path);

        session
            .create_map(&path, Size { x: 3, y: 2, z: 2 }, MapFormat::Tgm)
            .unwrap();

        let document = session.state.active_document().unwrap();
        assert_eq!(document.path.as_deref(), Some(path.as_path()));
        assert_eq!(document.map.size, Size { x: 3, y: 2, z: 2 });
        assert_eq!(document.map.format, MapFormat::Tgm);
        assert_eq!(document.z, 1);
        assert!(document.is_dirty());
        assert_eq!(document.map.dictionary.len(), 1);
        for coord in [Coord::new(1, 1, 1), Coord::new(3, 2, 2)] {
            assert_eq!(
                document
                    .map
                    .tile_at(coord)
                    .unwrap()
                    .iter()
                    .map(|prefab| prefab.path.to_string())
                    .collect::<Vec<_>>(),
                ["/turf", "/area"]
            );
        }

        session.save_map_as(&path, MapFormat::Tgm).unwrap();
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .starts_with("//MAP CONVERTED BY dmm2tgm.py")
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn sanitizing_on_save_leaves_default_values_out_of_the_file_only() {
        let mut table = Prefab::new(TreePath::parse("/obj/structure/table"));
        table.set_var("name".into(), Value::Text(String::from("table")));
        table.set_var("icon_state".into(), Value::Text(String::from("broken")));
        for sanitize in [false, true] {
            let mut session = flat_session(1, 1);
            session.state.choose_prefab(table.clone());
            session.set_tool(Tool::Place);
            assert!(session.place_at(Coord::new(1, 1, 1), None).is_some());
            let path = std::env::temp_dir().join(format!("rmd-sanitize-{sanitize}-{}.dmm", std::process::id()));

            session.set_sanitize_vars_on_save(sanitize);
            session.save_map_as(&path, MapFormat::Tgm).unwrap();

            let (saved, errors) = dmm::parser::parse(&std::fs::read_to_string(&path).unwrap());
            assert!(errors.is_empty(), "{errors:?}");
            let saved_table = saved
                .tile_at(Coord::new(1, 1, 1))
                .unwrap()
                .iter()
                .find(|prefab| prefab.path == table.path)
                .unwrap();
            assert_eq!(saved_table.var(&"name".into()).is_none(), sanitize);
            assert_eq!(
                saved_table.var(&"icon_state".into()),
                Some(&Value::Text(String::from("broken")))
            );
            assert!(
                session
                    .map()
                    .unwrap()
                    .tile_at(Coord::new(1, 1, 1))
                    .unwrap()
                    .contains(&table),
                "the open map keeps every override"
            );
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn save_all_writes_every_changed_map_with_a_file_and_names_one_that_needs_a_path() {
        let saved = std::env::temp_dir().join(format!("rmd-save-all-{}.dmm", std::process::id()));
        let unsaved = std::env::temp_dir().join(format!("rmd-save-all-new-{}.dmm", std::process::id()));
        let _ = std::fs::remove_file(&saved);
        let mut session = Session::new();
        let first = session
            .state
            .open_document(MapDocument::create(&saved, Map::new(Size { x: 1, y: 1, z: 1 }), 1));
        session.save_map_as(&saved, MapFormat::Tgm).unwrap();
        assert!(session.state.document_mut(first).unwrap().resize(2, 1, &[]));
        let second = session
            .state
            .open_document(MapDocument::create(&unsaved, Map::new(Size { x: 1, y: 1, z: 1 }), 1));

        let outcome = session.save_all();

        assert_eq!(outcome.needs_path, Some(second));
        assert_eq!(outcome.error, None);
        assert!(!session.state.document(first).unwrap().is_dirty());
        assert!(session.state.document(second).unwrap().is_dirty());
        assert!(!unsaved.exists(), "a map that never had a file waits for Save As");
        let _ = std::fs::remove_file(saved);
    }

    #[test]
    fn in_place_save_reuses_the_confirmed_path_and_format() {
        let path = std::env::temp_dir().join(format!("rmd-save-map-{}.dmm", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut session = Session::new();
        session
            .state
            .open_document(MapDocument::create(&path, Map::new(Size { x: 1, y: 1, z: 1 }), 1));

        assert!(!session.can_save_map_in_place());
        session.save_map_as(&path, MapFormat::Tgm).expect("initial save");
        assert!(session.can_save_map_in_place());

        std::fs::write(&path, "replace me").expect("replace target contents");
        session.save_map().expect("in-place save");
        assert!(
            std::fs::read_to_string(&path)
                .expect("written map")
                .starts_with("//MAP CONVERTED BY dmm2tgm.py")
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn new_map_creation_rejects_invalid_dimensions_and_existing_targets() {
        let mut session = Session::new();
        let path = std::env::temp_dir().join(format!("rmd-new-map-collision-{}.dmm", std::process::id()));

        let error = session
            .create_map(&path, Size { x: 0, y: 1, z: 1 }, MapFormat::Standard)
            .unwrap_err();
        assert!(error.to_string().contains("between 1 and 255"));

        std::fs::write(&path, "existing").unwrap();
        let error = session
            .create_map(&path, Size { x: 1, y: 1, z: 1 }, MapFormat::Standard)
            .unwrap_err();
        assert!(error.to_string().contains("already exists"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn exporting_the_open_map_as_tgm_round_trips_through_the_parser() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).unwrap();
        let source = root.join("test.dmm");
        session.open_map(&source, 1).unwrap();

        let dir = std::env::temp_dir().join(format!("rmd-session-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let exported = dir.join("exported.dmm");

        let before = session.map().cloned().unwrap();
        session.save_map_as(&exported, dmm::MapFormat::Tgm).expect("export");

        assert_eq!(session.map_path(), Some(exported.as_path()));
        assert_eq!(session.map_format(), Some(dmm::MapFormat::Tgm));

        let (reparsed, errors) = dmm::parser::parse(&std::fs::read_to_string(&exported).expect("written map"));
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(reparsed.format, dmm::MapFormat::Tgm);
        assert_eq!(reparsed.grid, before.grid);
        assert_eq!(reparsed.dictionary, before.dictionary);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
