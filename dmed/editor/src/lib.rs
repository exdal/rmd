pub mod command;
pub mod document;
pub mod environment;
pub mod error;
pub mod frame;
pub mod tool;
pub mod visual;

use core::types::Value;
use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
};

use dmi::{IconFile, error::IconError, metadata::Metadata};
use objtree::ObjectTree;

use crate::{document::MapDocument, environment::LoadDiagnostics, error::LoadError, tool::Tool};

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
        let entry = entry.as_ref();
        let (tree, compiled) = environment::compile(entry)?;

        let mut environment = Self {
            root: compiled.root,
            tree,
            icons: HashMap::new(),
            maps: compiled.maps,
            files: compiled.files,
            resource_dirs: compiled.resource_dirs,
        };

        let search_dirs = environment.resource_dirs.clone();
        let icons = environment.load_icons(&search_dirs);

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
    pub fn load_icons(&mut self, search_dirs: &[PathBuf]) -> Vec<(String, IconError)> {
        let names: Vec<String> = self.icon_paths().into_iter().map(String::from).collect();
        let base = self.base_dir().to_path_buf();
        let mut failures = Vec::new();

        for name in names {
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
    pub documents: Vec<MapDocument>,
    pub active: Option<usize>,
    pub tool: Tool,
    pub palette: Option<dmm::Prefab>,
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
        }
    }

    pub fn active_document(&self) -> Option<&MapDocument> { self.documents.get(self.active?) }

    pub fn active_document_mut(&mut self) -> Option<&mut MapDocument> {
        let index = self.active?;

        self.documents.get_mut(index)
    }

    pub fn open_document(&mut self, document: MapDocument) -> usize {
        self.documents.push(document);
        let index = self.documents.len() - 1;
        self.active = Some(index);

        index
    }

    pub fn has_unsaved_changes(&self) -> bool { self.documents.iter().any(MapDocument::is_dirty) }
}

#[cfg(test)]
mod tests {
    use core::{
        location::Location,
        path::TreePath,
        types::{Identifier, Value, VarModifiers},
    };

    use objtree::{ObjectTree, VarDecl};

    use crate::Environment;

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

        let failures = environment.load_icons(&[]);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].0, "icons/nope.dmi");
        assert!(environment.icons.is_empty());
    }
}
