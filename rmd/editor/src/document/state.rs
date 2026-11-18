use prefab::Prefab;
use std::{path::Path, sync::Arc};

use crate::{
    clipboard::TileRegion,
    document::{DocumentId, MapDocument},
    environment::Environment,
    maploader::resolved_path,
    stash::Stash,
    tool::Tool,
};

pub const RECENT_PREFAB_CAPACITY: usize = 10;

pub struct EditorState {
    pub environment: Option<Arc<Environment>>,
    documents: Vec<MapDocument>,
    active: Option<DocumentId>,
    pub tool: Tool,
    pub palette: Option<Prefab>,
    recent_prefabs: Vec<Prefab>,
    clipboard: Option<TileRegion>,
    stash: Stash,
}

impl Default for EditorState {
    fn default() -> Self {
        Self::new()
    }
}

impl EditorState {
    pub fn new() -> Self {
        Self {
            environment: None,
            documents: Vec::new(),
            active: None,
            tool: Tool::default(),
            palette: None,
            recent_prefabs: Vec::new(),
            clipboard: None,
            stash: Stash::default(),
        }
    }

    pub fn documents(&self) -> &[MapDocument] {
        &self.documents
    }

    pub fn document_ids(&self) -> Vec<DocumentId> {
        self.documents.iter().map(MapDocument::id).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.documents.is_empty()
    }

    pub fn stash(&self) -> &Stash {
        &self.stash
    }

    pub fn stash_mut(&mut self) -> &mut Stash {
        &mut self.stash
    }

    pub fn remove_stash_entry(&mut self, id: DocumentId) -> bool {
        if !self.stash.remove(id) {
            return false;
        }

        if self.active == Some(id) {
            self.active = self.documents.first().map(MapDocument::id);
        }

        true
    }

    pub fn document(&self, id: DocumentId) -> Option<&MapDocument> {
        self.documents
            .iter()
            .find(|document| document.id() == id)
            .or_else(|| self.stash.document(id))
    }

    pub fn document_mut(&mut self, id: DocumentId) -> Option<&mut MapDocument> {
        self.documents
            .iter_mut()
            .find(|document| document.id() == id)
            .or_else(|| self.stash.document_mut(id))
    }

    pub fn document_for_path(&self, path: &Path) -> Option<DocumentId> {
        let path = resolved_path(path);

        self.documents
            .iter()
            .find(|document| document.resolved_path() == Some(path.as_path()))
            .map(MapDocument::id)
    }

    pub fn active(&self) -> Option<DocumentId> {
        self.active
    }

    pub fn set_active(&mut self, id: DocumentId) {
        if self.document(id).is_some() {
            self.active = Some(id);
        }
    }

    pub fn active_document(&self) -> Option<&MapDocument> {
        self.document(self.active?)
    }

    pub fn active_document_mut(&mut self) -> Option<&mut MapDocument> {
        self.document_mut(self.active?)
    }

    pub fn open_document(&mut self, document: MapDocument) -> DocumentId {
        let id = document.id();
        self.documents.push(document);
        self.active = Some(id);

        id
    }

    pub fn close_document(&mut self, id: DocumentId) -> Option<MapDocument> {
        let index = self.documents.iter().position(|document| document.id() == id)?;
        let document = self.documents.remove(index);

        if self.active == Some(id) {
            self.active = self
                .documents
                .get(index)
                .or_else(|| self.documents.get(index.wrapping_sub(1)))
                .map(MapDocument::id);
        }

        Some(document)
    }

    #[cfg(test)]
    pub fn has_unsaved_changes(&self) -> bool {
        self.documents.iter().any(MapDocument::is_dirty)
    }

    pub fn active_pair_mut(&mut self) -> Option<(&Environment, &mut MapDocument)> {
        self.pair_mut((self.active)?)
    }

    pub fn pair_mut(&mut self, id: DocumentId) -> Option<(&Environment, &mut MapDocument)> {
        let Self {
            environment,
            documents,
            stash,
            ..
        } = self;
        let environment = environment.as_ref()?;
        let document = documents
            .iter_mut()
            .find(|document| document.id() == id)
            .or_else(|| stash.document_mut(id))?;

        Some((environment, document))
    }

    pub fn active_pair(&self) -> Option<(&Environment, &MapDocument)> {
        Some((self.environment.as_ref()?, self.active_document()?))
    }

    pub fn clipboard(&self) -> Option<&TileRegion> {
        self.clipboard.as_ref()
    }

    pub fn set_clipboard(&mut self, region: TileRegion) {
        self.clipboard = Some(region);
    }

    pub fn recent_prefabs(&self) -> &[Prefab] {
        &self.recent_prefabs
    }

    pub fn choose_prefab(&mut self, prefab: Prefab) {
        if let Some(index) = self.recent_prefabs.iter().position(|recent| recent == &prefab) {
            self.recent_prefabs.remove(index);
        }
        self.recent_prefabs.insert(0, prefab.clone());
        self.recent_prefabs.truncate(RECENT_PREFAB_CAPACITY);
        self.palette = Some(prefab);
    }

    pub fn choose_recent(&mut self, index: usize) -> bool {
        let Some(prefab) = (index < self.recent_prefabs.len()).then(|| self.recent_prefabs.remove(index)) else {
            return false;
        };

        self.recent_prefabs.insert(0, prefab.clone());
        self.palette = Some(prefab);

        true
    }

    pub fn replace_palette(&mut self, prefab: Prefab) -> bool {
        let Some(previous) = self.palette.as_ref() else {
            return false;
        };
        if previous == &prefab {
            return false;
        }

        self.recent_prefabs
            .retain(|recent| recent != previous && recent != &prefab);
        self.recent_prefabs.insert(0, prefab.clone());
        self.recent_prefabs.truncate(RECENT_PREFAB_CAPACITY);
        self.palette = Some(prefab);

        true
    }
}

#[cfg(test)]
mod tests {
    use base::{coord::Coord, path::TreePath, types::Value};
    use prefab::Prefab;
    use std::path::Path;

    use super::{EditorState, RECENT_PREFAB_CAPACITY};
    use crate::{
        clipboard::{TileRegion, copy_region},
        document::{MapDocument, selection::Selection},
        frame::options::HiddenTypes,
        tool::RegionSelectionMode,
    };

    fn region() -> TileRegion {
        let mut map = dmm::Map::new(dmm::Size { x: 2, y: 2, z: 1 });
        let key = map.intern_tile(vec![Prefab::new(TreePath::parse("/turf/open/floor"))]);
        for row in &mut map.grid[0] {
            row.fill(key);
        }

        let document = MapDocument::new(map, 1);
        let selection = Selection::from_drag(Coord::new(1, 1, 1), Coord::new(2, 2, 1));

        copy_region(
            (&document.map).into(),
            selection,
            RegionSelectionMode::Full,
            HiddenTypes::default(),
        )
        .unwrap()
    }

    fn document(path: &str) -> crate::document::MapDocument {
        crate::document::MapDocument::open(path, dmm::Map::new(dmm::Size { x: 1, y: 1, z: 1 }), 1)
    }

    #[test]
    fn every_open_document_gets_its_own_identity() {
        let mut state = EditorState::new();
        let first = state.open_document(document("a.dmm"));
        let second = state.open_document(document("b.dmm"));
        let third = state.open_document(document("c.dmm"));

        assert_ne!(first, second);
        assert_ne!(second, third);
        assert_eq!(state.active(), Some(third));
        assert_eq!(state.documents().len(), 3);
    }

    #[test]
    fn closing_a_document_leaves_the_others_addressable() {
        let mut state = EditorState::new();
        let first = state.open_document(document("a.dmm"));
        let second = state.open_document(document("b.dmm"));
        let third = state.open_document(document("c.dmm"));

        state.close_document(second);

        // An index would have shifted `third` out from under its handle here.
        assert!(state.document(first).is_some());
        assert!(state.document(second).is_none());
        assert!(state.document(third).is_some());
        assert_eq!(state.active(), Some(third));
    }

    #[test]
    fn closing_the_active_document_focuses_a_neighbour() {
        let mut state = EditorState::new();
        let first = state.open_document(document("a.dmm"));
        let second = state.open_document(document("b.dmm"));
        state.set_active(first);

        state.close_document(first);
        assert_eq!(state.active(), Some(second));

        state.close_document(second);
        assert_eq!(state.active(), None);
        assert!(state.is_empty());
    }

    #[test]
    fn an_already_open_map_is_found_by_path() {
        let mut state = EditorState::new();
        let first = state.open_document(document("a.dmm"));
        state.open_document(document("b.dmm"));

        assert_eq!(state.document_for_path(Path::new("a.dmm")), Some(first));
        assert_eq!(state.document_for_path(Path::new("missing.dmm")), None);

        state.close_document(first);
        assert_eq!(state.document_for_path(Path::new("a.dmm")), None);
    }

    #[test]
    fn a_closed_document_stops_reporting_unsaved_changes() {
        let mut state = EditorState::new();
        let dirty = state.open_document(crate::document::MapDocument::create(
            "new.dmm",
            dmm::Map::new(dmm::Size { x: 1, y: 1, z: 1 }),
            1,
        ));

        assert!(state.has_unsaved_changes());

        state.close_document(dirty);
        assert!(!state.has_unsaved_changes());
    }

    #[test]
    fn stash_entries_are_documents_that_are_not_listed() {
        let mut state = EditorState::new();
        let map_id = state.open_document(MapDocument::new(dmm::Map::new(dmm::Size { x: 1, y: 1, z: 1 }), 1));
        let entry = state.stash_mut().push(region());

        assert!(state.document(entry).is_some_and(MapDocument::is_stash));
        assert_eq!(state.document_ids(), vec![map_id]);
        assert!(state.close_document(entry).is_none());
        assert!(state.document(entry).is_some());

        state.set_active(entry);
        assert_eq!(state.active(), Some(entry));
        assert!(state.remove_stash_entry(entry));
        assert!(!state.remove_stash_entry(entry));
        assert_eq!(state.active(), Some(map_id));
        assert!(state.document(entry).is_none());
    }

    #[test]
    fn recent_prefabs_are_exact_deduplicated_and_bounded() {
        let mut state = EditorState::new();
        for index in 0..RECENT_PREFAB_CAPACITY + 2 {
            state.choose_prefab(Prefab::new(TreePath::parse(&format!("/obj/item/{index}"))));
        }

        assert_eq!(state.recent_prefabs().len(), RECENT_PREFAB_CAPACITY);
        assert_eq!(state.recent_prefabs()[0].path, TreePath::parse("/obj/item/11"));
        assert_eq!(state.recent_prefabs()[9].path, TreePath::parse("/obj/item/2"));

        let mut overridden = Prefab::new(TreePath::parse("/obj/item/11"));
        overridden.set_var("name".into(), Value::Text("custom".into()));
        state.choose_prefab(overridden.clone());

        assert_eq!(state.recent_prefabs()[0], overridden);
        assert_eq!(state.recent_prefabs()[1].path, TreePath::parse("/obj/item/11"));
    }

    #[test]
    fn choosing_an_old_recent_entry_promotes_it() {
        let mut state = EditorState::new();
        for name in ["one", "two", "three"] {
            state.choose_prefab(Prefab::new(TreePath::parse(&format!("/obj/{name}"))));
        }

        assert!(state.choose_recent(2));
        assert_eq!(state.palette.as_ref().unwrap().path, TreePath::parse("/obj/one"));
        assert_eq!(state.recent_prefabs()[0].path, TreePath::parse("/obj/one"));
        assert_eq!(state.recent_prefabs()[1].path, TreePath::parse("/obj/three"));
        assert!(!state.choose_recent(9));
    }

    #[test]
    fn replacing_the_palette_updates_one_recent_slot() {
        let mut state = EditorState::new();
        let older = Prefab::new(TreePath::parse("/obj/older"));
        let current = Prefab::new(TreePath::parse("/obj/current"));
        let mut rotated = current.clone();
        rotated.set_var("dir".into(), Value::Num(4.0));
        state.choose_prefab(older.clone());
        state.choose_prefab(current.clone());

        assert!(state.replace_palette(rotated.clone()));
        assert_eq!(state.palette.as_ref(), Some(&rotated));
        assert_eq!(state.recent_prefabs(), [rotated, older]);
        assert!(!state.replace_palette(state.palette.clone().unwrap()));
        assert!(!EditorState::new().replace_palette(current));
    }
}
