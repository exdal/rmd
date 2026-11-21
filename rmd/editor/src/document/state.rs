use prefab::Prefab;
use std::{mem, path::Path, sync::Arc};

use crate::{
    clipboard::TileRegion,
    document::{DocumentId, MapDocument},
    environment::Environment,
    maploader::resolved_path,
    stash::Stash,
    tool::Tool,
};

pub const RECENT_PREFAB_CAPACITY: usize = 10;

#[derive(Debug, Clone, PartialEq)]
pub struct RecentPrefab {
    pub prefab: Prefab,
    pub is_locked: bool,
}

pub struct EditorState {
    pub environment: Option<Arc<Environment>>,
    documents: Vec<MapDocument>,
    active: Option<DocumentId>,
    pub tool: Tool,
    pub palette: Option<Prefab>,
    recent_prefabs: Vec<RecentPrefab>,
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

    pub fn recent_prefabs(&self) -> &[RecentPrefab] {
        &self.recent_prefabs
    }

    fn is_locked_prefab(&self, prefab: &Prefab) -> bool {
        self.recent_prefabs
            .iter()
            .any(|recent| recent.is_locked && &recent.prefab == prefab)
    }

    fn unlocked_prefabs(&self) -> Vec<Prefab> {
        self.recent_prefabs
            .iter()
            .filter(|recent| !recent.is_locked)
            .map(|recent| recent.prefab.clone())
            .collect::<Vec<_>>()
    }

    fn rebuild_recent(&mut self, unlocked: Vec<Prefab>) {
        let mut locked = mem::take(&mut self.recent_prefabs)
            .into_iter()
            .enumerate()
            .filter(|(_, recent)| recent.is_locked)
            .peekable();
        let mut unlocked = unlocked.into_iter();
        let mut rebuilt = Vec::with_capacity(RECENT_PREFAB_CAPACITY);

        while rebuilt.len() < RECENT_PREFAB_CAPACITY {
            if locked.peek().is_some_and(|(index, _)| *index == rebuilt.len()) {
                rebuilt.extend(locked.next().map(|(_, recent)| recent));
            } else if let Some(prefab) = unlocked.next() {
                rebuilt.push(RecentPrefab {
                    prefab,
                    is_locked: false,
                });
            } else if let Some((_, recent)) = locked.next() {
                rebuilt.push(recent);
            } else {
                break;
            }
        }

        self.recent_prefabs = rebuilt;
    }

    fn promote_recent(&mut self, prefab: &Prefab) {
        let mut unlocked = self.unlocked_prefabs();
        unlocked.retain(|recent| recent != prefab);
        unlocked.insert(0, prefab.clone());
        self.rebuild_recent(unlocked);
    }

    pub fn choose_prefab(&mut self, prefab: Prefab) {
        if !self.is_locked_prefab(&prefab) {
            self.promote_recent(&prefab);
        }

        self.palette = Some(prefab);
    }

    pub fn choose_recent(&mut self, index: usize) -> bool {
        let Some(recent) = self.recent_prefabs.get(index) else {
            return false;
        };

        let prefab = recent.prefab.clone();
        if !recent.is_locked {
            self.promote_recent(&prefab);
        }

        self.palette = Some(prefab);

        true
    }

    pub fn replace_palette(&mut self, prefab: Prefab) -> bool {
        let Some(previous) = self.palette.clone() else {
            return false;
        };
        if previous == prefab {
            return false;
        }

        if !self.is_locked_prefab(&prefab) {
            let mut unlocked = self.unlocked_prefabs();
            unlocked.retain(|recent| recent != &previous && recent != &prefab);
            unlocked.insert(0, prefab.clone());
            self.rebuild_recent(unlocked);
        } else if !self.is_locked_prefab(&previous) {
            let mut unlocked = self.unlocked_prefabs();
            unlocked.retain(|recent| recent != &previous);
            self.rebuild_recent(unlocked);
        }

        self.palette = Some(prefab);

        true
    }

    pub fn toggle_recent_lock(&mut self, index: usize) -> bool {
        let Some(recent) = self.recent_prefabs.get_mut(index) else {
            return false;
        };

        recent.is_locked = !recent.is_locked;

        true
    }
}

#[cfg(test)]
mod tests {
    use base::{coord::Coord, path::TreePath, types::Value};
    use prefab::Prefab;
    use std::path::Path;

    use super::{EditorState, RECENT_PREFAB_CAPACITY, RecentPrefab};
    use crate::{
        clipboard::{TileRegion, copy_region},
        document::{MapDocument, selection::Selection},
        frame::options::HiddenTypes,
        tool::RegionSelectionMode,
    };

    fn prefabs(state: &EditorState) -> Vec<Prefab> {
        state
            .recent_prefabs()
            .iter()
            .map(|recent| recent.prefab.clone())
            .collect::<Vec<_>>()
    }

    fn item(index: usize) -> Prefab {
        Prefab::new(TreePath::parse(&format!("/obj/item/{index}")))
    }

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
        assert_eq!(state.recent_prefabs()[0].prefab.path, TreePath::parse("/obj/item/11"));
        assert_eq!(state.recent_prefabs()[9].prefab.path, TreePath::parse("/obj/item/2"));

        let mut overridden = Prefab::new(TreePath::parse("/obj/item/11"));
        overridden.set_var("name".into(), Value::Text("custom".into()));
        state.choose_prefab(overridden.clone());

        assert_eq!(state.recent_prefabs()[0].prefab, overridden);
        assert_eq!(state.recent_prefabs()[1].prefab.path, TreePath::parse("/obj/item/11"));
    }

    #[test]
    fn choosing_an_old_recent_entry_promotes_it() {
        let mut state = EditorState::new();
        for name in ["one", "two", "three"] {
            state.choose_prefab(Prefab::new(TreePath::parse(&format!("/obj/{name}"))));
        }

        assert!(state.choose_recent(2));
        assert_eq!(state.palette.as_ref().unwrap().path, TreePath::parse("/obj/one"));
        assert_eq!(state.recent_prefabs()[0].prefab.path, TreePath::parse("/obj/one"));
        assert_eq!(state.recent_prefabs()[1].prefab.path, TreePath::parse("/obj/three"));
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
        assert_eq!(prefabs(&state), [rotated, older]);
        assert!(!state.replace_palette(state.palette.clone().unwrap()));
        assert!(!EditorState::new().replace_palette(current));
    }

    #[test]
    fn a_locked_slot_keeps_its_index_through_many_picks() {
        let mut state = EditorState::new();
        for index in 0..RECENT_PREFAB_CAPACITY {
            state.choose_prefab(item(index));
        }
        let locked = state.recent_prefabs()[2].prefab.clone();
        assert!(state.toggle_recent_lock(2));

        for index in 100..100 + RECENT_PREFAB_CAPACITY + 3 {
            state.choose_prefab(item(index));
            assert_eq!(state.recent_prefabs().len(), RECENT_PREFAB_CAPACITY);
            assert_eq!(state.recent_prefabs()[2].prefab, locked);
            assert!(state.recent_prefabs()[2].is_locked);
        }

        assert_eq!(state.recent_prefabs()[0].prefab, item(112));
        assert_eq!(state.recent_prefabs()[1].prefab, item(111));
        assert_eq!(state.recent_prefabs()[3].prefab, item(110));
    }

    #[test]
    fn choosing_a_locked_prefab_only_sets_the_palette() {
        let mut state = EditorState::new();
        for index in 0..4 {
            state.choose_prefab(item(index));
        }
        state.toggle_recent_lock(3);
        let before = state.recent_prefabs().to_vec();

        assert!(state.choose_recent(3));
        assert_eq!(state.palette, Some(item(0)));
        assert_eq!(state.recent_prefabs(), before);

        state.choose_prefab(item(2));
        let before = state.recent_prefabs().to_vec();
        state.choose_prefab(item(0));
        assert_eq!(state.palette, Some(item(0)));
        assert_eq!(state.recent_prefabs(), before);
    }

    #[test]
    fn replacing_a_locked_palette_keeps_the_locked_entry() {
        let mut state = EditorState::new();
        let current = item(0);
        state.choose_prefab(item(1));
        state.choose_prefab(current.clone());
        state.toggle_recent_lock(0);
        let mut rotated = current.clone();
        rotated.set_var("dir".into(), Value::Num(4.0));

        assert!(state.replace_palette(rotated.clone()));
        assert_eq!(state.palette.as_ref(), Some(&rotated));
        assert_eq!(
            state.recent_prefabs(),
            [
                RecentPrefab {
                    prefab: current,
                    is_locked: true
                },
                RecentPrefab {
                    prefab: rotated,
                    is_locked: false
                },
                RecentPrefab {
                    prefab: item(1),
                    is_locked: false
                },
            ]
        );
    }

    #[test]
    fn toggling_a_lock_flips_it_and_rejects_bad_indices() {
        let mut state = EditorState::new();
        state.choose_prefab(item(0));

        assert!(state.toggle_recent_lock(0));
        assert!(state.recent_prefabs()[0].is_locked);
        assert!(state.toggle_recent_lock(0));
        assert!(!state.recent_prefabs()[0].is_locked);
        assert!(!state.toggle_recent_lock(5));
    }

    #[test]
    fn a_fully_locked_list_ignores_new_picks() {
        let mut state = EditorState::new();
        for index in 0..RECENT_PREFAB_CAPACITY {
            state.choose_prefab(item(index));
        }
        for index in 0..RECENT_PREFAB_CAPACITY {
            state.toggle_recent_lock(index);
        }
        let before = state.recent_prefabs().to_vec();

        state.choose_prefab(item(99));
        assert_eq!(state.palette, Some(item(99)));
        assert_eq!(state.recent_prefabs(), before);
    }

    #[test]
    fn a_locked_entry_closes_gaps_it_cannot_fill() {
        let mut state = EditorState::new();
        for index in 0..3 {
            state.choose_prefab(item(index));
        }
        state.toggle_recent_lock(2);
        state.rebuild_recent(vec![item(7)]);

        assert_eq!(prefabs(&state), [item(7), item(0)]);
    }
}
