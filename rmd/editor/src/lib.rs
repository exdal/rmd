pub mod bake;
pub mod clipboard;
pub mod command;
pub mod document;
pub mod environment;
pub mod error;
pub mod focus;
pub mod frame;
pub mod icons;
pub mod node;
pub mod progress;
pub mod tool;
pub mod visual;

use core::types::Value;
use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::Arc,
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

pub struct BakeProgram {
    pub tree: ObjectTree,
    pub module: codegen::Module,
    pub profile: objtree::TypeId,
    pub files: Arc<[PathBuf]>,
    pub icon_states: vm::IconStates,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profiles {
    pub available: Vec<String>,
    pub default: String,
    pub active: String,
}

pub struct Environment {
    pub root: PathBuf,
    /// The compatibility view used by the editor and static renderer.
    pub tree: ObjectTree,
    /// Runtime declarations and bytecode compiled without mapping compatibility defines.
    pub bake_program: Option<BakeProgram>,
    /// Profiles discovered in the runtime view, even when bytecode generation failed.
    pub profiles: Option<Profiles>,
    /// Runtime-view sources, retained even when bytecode generation fails.
    pub bake_files: Arc<[PathBuf]>,
    pub bake_options: environment::BakeOptions,
    pub optimization_timings: ir::opt::OptimizationTimings,
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
            bake_program: None,
            profiles: None,
            bake_files: Arc::default(),
            bake_options: environment::BakeOptions::default(),
            optimization_timings: ir::opt::OptimizationTimings::default(),
            icons: HashMap::new(),
            maps: Vec::new(),
            files: Vec::new(),
            resource_dirs: Vec::new(),
        }
    }

    pub fn load(entry: impl AsRef<Path>) -> Result<(Self, LoadDiagnostics), LoadError> {
        Self::load_with(entry, environment::BakeOptions::default(), &Progress::new())
    }

    pub fn load_with(
        entry: impl AsRef<Path>, options: environment::BakeOptions, progress: &Progress,
    ) -> Result<(Self, LoadDiagnostics), LoadError> {
        let entry = entry.as_ref();
        let compiled = environment::compile(entry, &options, progress)?;

        let mut environment = Self {
            root: compiled.root,
            tree: compiled.tree,
            bake_program: compiled.bake_program,
            profiles: compiled.profiles,
            bake_files: compiled.bake_files,
            bake_options: options,
            optimization_timings: compiled.optimization_timings,
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
                bake_preprocess: compiled.bake_errors,
                bake_sema: compiled.bake_sema_errors,
                codegen: compiled.codegen_error,
                profile: compiled.profile_error,
                icons,
                bake: Vec::new(),
            },
        ))
    }

    pub fn base_dir(&self) -> &Path { self.root.parent().unwrap_or(Path::new(".")) }

    pub fn file(&self, id: core::location::FileId) -> Option<&Path> {
        self.files.get(id.0 as usize).map(PathBuf::as_path)
    }

    pub fn bake_file(&self, id: core::location::FileId) -> Option<&Path> {
        self.bake_files.get(id.0 as usize).map(PathBuf::as_path)
    }

    pub fn icon(&self, name: &str) -> Option<&Metadata> { self.icons.get(name) }

    /// `'icons/obj/items.dmi'`
    pub fn icon_paths(&self) -> BTreeSet<&str> {
        self.tree
            .iter()
            .flat_map(|decl| decl.vars.values())
            .map(|var| &var.value)
            .chain(
                self.bake_program
                    .iter()
                    .flat_map(|program| program.tree.iter())
                    .flat_map(|decl| decl.vars.values())
                    .map(|var| &var.value),
            )
            .chain(self.bake_program.iter().flat_map(|program| &program.module.constants))
            .filter_map(|value| match value {
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
            let found = std::iter::once(base.as_path())
                .chain(search_dirs.iter().map(PathBuf::as_path))
                .find_map(|dir| resolve_resource_path(dir, &name))
                .unwrap_or_else(|| base.join(&name));

            match IconFile::load_metadata(&found) {
                Ok(metadata) => {
                    self.icons.insert(name, metadata);
                },
                Err(e) => failures.push((name, e)),
            }
        }

        if let Some(program) = self.bake_program.as_mut() {
            program.icon_states = vm::IconStates::new(self.icons.iter().map(|(name, metadata)| {
                let states = metadata.states.iter().map(|state| state.name.clone()).collect();

                (name.clone(), states)
            }));
        }

        failures
    }
}

fn resolve_resource_path(dir: &Path, relative: &str) -> Option<PathBuf> {
    let mut current = dir.to_path_buf();
    for component in relative.split('/').filter(|part| !part.is_empty()) {
        let entry = std::fs::read_dir(&current).ok()?.filter_map(Result::ok).find(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.eq_ignore_ascii_case(component))
        })?;

        current = entry.path();
    }

    current.is_file().then_some(current)
}

pub struct EditorState {
    pub environment: Option<Arc<Environment>>,
    documents: Vec<MapDocument>,
    active: Option<DocumentId>,
    pub tool: Tool,
    pub palette: Option<dmm::Prefab>,
    recent_prefabs: Vec<dmm::Prefab>,
    clipboard: Option<TileBlock>,
    clipboard_revision: u64,
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
            clipboard_revision: 0,
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

    pub fn set_clipboard(&mut self, block: TileBlock) {
        self.clipboard = Some(block);
        self.clipboard_revision = self.clipboard_revision.wrapping_add(1);
    }

    pub fn clipboard_revision(&self) -> u64 { self.clipboard_revision }

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
                let name = Identifier::from(String::from(*name));
                decl.vars.insert(
                    name.clone(),
                    VarDecl {
                        name,
                        declared_type: None,
                        modifiers: VarModifiers::default(),
                        value: value.clone(),
                        initializer: None,
                        declared: true,
                        location: Location::default(),
                    },
                );
            }
        }

        tree
    }

    #[test]
    fn resource_paths_resolve_regardless_of_case() {
        let dir = std::env::temp_dir().join(format!("rmd-resource-case-{}", std::process::id()));
        let nested = dir.join("Icons").join("Obj");
        std::fs::create_dir_all(&nested).expect("temp dir");
        std::fs::write(nested.join("Items.dmi"), []).expect("write fixture");

        let found = super::resolve_resource_path(&dir, "icons/obj/items.dmi");
        assert_eq!(found, Some(nested.join("Items.dmi")));
        assert!(super::resolve_resource_path(&dir, "icons/obj/missing.dmi").is_none());

        let _ = std::fs::remove_dir_all(&dir);
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
