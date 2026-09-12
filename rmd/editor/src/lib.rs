pub mod clipboard;
pub mod command;
pub mod document;
pub mod environment;
pub mod error;
pub mod focus;
pub mod frame;
pub mod icons;
pub mod progress;
pub mod tool;
pub mod visual;

use core::types::Value;
use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
};

use dmi::{IconFile, error::IconError, metadata::Metadata};
use objtree::ObjectTree;

use crate::{
    clipboard::TileBlock,
    document::{DocumentId, MapDocument},
    environment::LoadDiagnostics,
    error::LoadError,
    progress::{Progress, Stage},
    tool::Tool,
};

pub const RECENT_PREFAB_CAPACITY: usize = 10;

pub struct Environment {
    pub root: PathBuf,
    pub tree: ObjectTree,
    pub icons: HashMap<String, Metadata>,
    pub maps: Vec<PathBuf>,
    pub files: Vec<PathBuf>,
    /// `#define FILE_DIR "icons"`
    pub resource_dirs: Vec<PathBuf>,
}

impl Environment {
    pub fn new(root: impl Into<PathBuf>, tree: ObjectTree) -> Self {
        Self {
            root: root.into(),
            tree,
            icons: HashMap::new(),
            maps: Vec::new(),
            files: Vec::new(),
            resource_dirs: Vec::new(),
        }
    }

    pub fn load(entry: impl AsRef<Path>) -> Result<(Self, LoadDiagnostics), LoadError> {
        Self::load_with_progress(entry, &Progress::new())
    }

    pub fn load_with_progress(
        entry: impl AsRef<Path>, progress: &Progress,
    ) -> Result<(Self, LoadDiagnostics), LoadError> {
        let entry = entry.as_ref();
        let (tree, compiled) = environment::compile(entry, progress)?;

        let mut environment = Self {
            root: compiled.root,
            tree,
            icons: HashMap::new(),
            maps: compiled.maps,
            files: compiled.files,
            resource_dirs: compiled.resource_dirs,
        };

        let search_dirs = environment.resource_dirs.clone();
        let icons = environment.load_icons(&search_dirs, progress);
        if progress.is_cancelled() {
            return Err(LoadError::Cancelled);
        }

        Ok((
            environment,
            LoadDiagnostics {
                preprocess: compiled.errors,
                sema: compiled.sema_errors,
                icons,
            },
        ))
    }

    pub fn base_dir(&self) -> &Path { self.root.parent().unwrap_or(Path::new(".")) }

    pub fn file(&self, id: core::location::FileId) -> Option<&Path> {
        self.files.get(id.0 as usize).map(PathBuf::as_path)
    }

    pub fn icon(&self, name: &str) -> Option<&Metadata> { self.icons.get(name) }

    /// `'icons/obj/items.dmi'`
    pub fn icon_paths(&self) -> BTreeSet<&str> {
        self.tree
            .iter()
            .flat_map(|decl| decl.vars.values())
            .filter_map(|var| match &var.value {
                Value::Resource(s) if s.to_ascii_lowercase().ends_with(".dmi") => Some(s.as_str()),
                _ => None,
            })
            .collect()
    }

    /// `#define FILE_DIR "icons"`
    pub fn load_icons(&mut self, search_dirs: &[PathBuf], progress: &Progress) -> Vec<(String, IconError)> {
        let names: Vec<String> = self.icon_paths().into_iter().map(String::from).collect();
        let base = self.base_dir().to_path_buf();
        let mut failures = Vec::new();

        progress.enter(Stage::Icons, names.len());
        for name in names {
            if progress.is_cancelled() {
                break;
            }
            progress.advance(&name);
            // TODO: BYOND resolves resource paths case-insensitively
            let mut candidates = std::iter::once(base.join(&name)).chain(search_dirs.iter().map(|dir| dir.join(&name)));

            let found = candidates
                .find(|path| path.is_file())
                .unwrap_or_else(|| base.join(&name));

            match IconFile::load_metadata(&found) {
                Ok(metadata) => {
                    self.icons.insert(name, metadata);
                },
                Err(e) => failures.push((name, e)),
            }
        }

        failures
    }
}

pub struct EditorState {
    pub environment: Option<Environment>,
    documents: Vec<MapDocument>,
    active: Option<DocumentId>,
    pub tool: Tool,
    pub palette: Option<dmm::Prefab>,
    recent_prefabs: Vec<dmm::Prefab>,
    clipboard: Option<TileBlock>,
}

impl Default for EditorState {
    fn default() -> Self { Self::new() }
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
        }
    }

    pub fn documents(&self) -> &[MapDocument] { &self.documents }

    pub fn document_ids(&self) -> Vec<DocumentId> { self.documents.iter().map(MapDocument::id).collect() }

    pub fn is_empty(&self) -> bool { self.documents.is_empty() }

    pub fn document(&self, id: DocumentId) -> Option<&MapDocument> {
        self.documents.iter().find(|document| document.id() == id)
    }

    pub fn document_mut(&mut self, id: DocumentId) -> Option<&mut MapDocument> {
        self.documents.iter_mut().find(|document| document.id() == id)
    }

    pub fn document_for_path(&self, path: &Path) -> Option<DocumentId> {
        self.documents
            .iter()
            .find(|document| document.path.as_deref() == Some(path))
            .map(MapDocument::id)
    }

    pub fn active(&self) -> Option<DocumentId> { self.active }

    pub fn set_active(&mut self, id: DocumentId) {
        if self.document(id).is_some() {
            self.active = Some(id);
        }
    }

    pub fn active_document(&self) -> Option<&MapDocument> { self.document(self.active?) }

    pub fn active_document_mut(&mut self) -> Option<&mut MapDocument> { self.document_mut(self.active?) }

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

    pub fn has_unsaved_changes(&self) -> bool { self.documents.iter().any(MapDocument::is_dirty) }

    pub fn active_pair_mut(&mut self) -> Option<(&Environment, &mut MapDocument)> {
        let Self {
            environment,
            documents,
            active,
            ..
        } = self;
        let environment = environment.as_ref()?;
        let id = (*active)?;
        let document = documents.iter_mut().find(|document| document.id() == id)?;

        Some((environment, document))
    }

    pub fn active_pair(&self) -> Option<(&Environment, &MapDocument)> {
        Some((self.environment.as_ref()?, self.active_document()?))
    }

    pub fn active_paste_mut(&mut self) -> Option<(&Environment, &mut MapDocument, &TileBlock)> {
        let Self {
            environment,
            documents,
            active,
            clipboard,
            ..
        } = self;
        let environment = environment.as_ref()?;
        let block = clipboard.as_ref()?;
        let id = (*active)?;
        let document = documents.iter_mut().find(|document| document.id() == id)?;

        Some((environment, document, block))
    }

    pub fn clipboard(&self) -> Option<&TileBlock> { self.clipboard.as_ref() }

    pub fn set_clipboard(&mut self, block: TileBlock) { self.clipboard = Some(block); }

    pub fn recent_prefabs(&self) -> &[dmm::Prefab] { &self.recent_prefabs }

    pub fn choose_prefab(&mut self, prefab: dmm::Prefab) {
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

    pub fn replace_palette(&mut self, prefab: dmm::Prefab) -> bool {
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
    use core::{
        location::Location,
        path::TreePath,
        types::{Identifier, Value, VarModifiers},
    };
    use std::path::Path;

    use objtree::{ObjectTree, VarDecl};

    use crate::{EditorState, Environment, RECENT_PREFAB_CAPACITY, progress::Progress};

    fn tree_with_vars(vars: &[(&str, Value)]) -> ObjectTree {
        let mut tree = ObjectTree::new();
        let id = tree.register(&TreePath::parse("/obj/item"), Location::default());

        if let Some(decl) = tree.get_mut(id) {
            for (name, value) in vars {
                let name = Identifier(String::from(*name));
                decl.vars.insert(
                    name.clone(),
                    VarDecl {
                        name,
                        declared_type: None,
                        modifiers: VarModifiers::default(),
                        value: value.clone(),
                        location: Location::default(),
                    },
                );
            }
        }

        tree
    }

    #[test]
    fn collects_dmi_resource_literals_only() {
        let tree = tree_with_vars(&[
            ("icon", Value::Resource(String::from("icons/obj/items.dmi"))),
            ("sound", Value::Resource(String::from("sound/bang.ogg"))),
            ("name", Value::Text(String::from("not a resource.dmi"))),
        ]);
        let environment = Environment::new("/project/game/tgstation.dme", tree);

        assert_eq!(
            environment.icon_paths().into_iter().collect::<Vec<_>>(),
            ["icons/obj/items.dmi"]
        );
    }

    #[test]
    fn load_icons_reports_misses_instead_of_aborting() {
        let tree = tree_with_vars(&[("icon", Value::Resource(String::from("icons/nope.dmi")))]);
        let mut environment = Environment::new("/project/game/tgstation.dme", tree);

        let failures = environment.load_icons(&[], &Progress::new());
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].0, "icons/nope.dmi");
        assert!(environment.icons.is_empty());
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
    fn recent_prefabs_are_exact_deduplicated_and_bounded() {
        let mut state = EditorState::new();
        for index in 0..RECENT_PREFAB_CAPACITY + 2 {
            state.choose_prefab(dmm::Prefab::new(TreePath::parse(&format!("/obj/item/{index}"))));
        }

        assert_eq!(state.recent_prefabs().len(), RECENT_PREFAB_CAPACITY);
        assert_eq!(state.recent_prefabs()[0].path, TreePath::parse("/obj/item/11"));
        assert_eq!(state.recent_prefabs()[9].path, TreePath::parse("/obj/item/2"));

        let mut overridden = dmm::Prefab::new(TreePath::parse("/obj/item/11"));
        overridden.set_var("name".into(), Value::Text("custom".into()));
        state.choose_prefab(overridden.clone());

        assert_eq!(state.recent_prefabs()[0], overridden);
        assert_eq!(state.recent_prefabs()[1].path, TreePath::parse("/obj/item/11"));
    }

    #[test]
    fn choosing_an_old_recent_entry_promotes_it() {
        let mut state = EditorState::new();
        for name in ["one", "two", "three"] {
            state.choose_prefab(dmm::Prefab::new(TreePath::parse(&format!("/obj/{name}"))));
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
        let older = dmm::Prefab::new(TreePath::parse("/obj/older"));
        let current = dmm::Prefab::new(TreePath::parse("/obj/current"));
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
