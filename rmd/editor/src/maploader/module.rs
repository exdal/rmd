use core::file_path;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use dmm::{Coord, Map, Prefab, PrefabInstanceId};
use objtree::{ObjectTree, TypeId};
use vm::{Fault, bake::ModuleRoot, world::Position};

use crate::{
    document::DocumentId,
    maploader::{ErrorKind, resolved_path},
};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RootSpecs {
    pub atoms: HashMap<PrefabInstanceId, Result<ModuleRoot, String>>,
    pub coords: HashMap<Coord, Result<ModuleRoot, String>>,
}

pub type Anchor = (Coord, Option<usize>);

pub fn root_specs<'a>(
    modules: impl Iterator<Item = (u64, &'a ModuleRoot)>, faults: impl Iterator<Item = (u64, &'a Fault)>,
    errors: impl Iterator<Item = (u64, &'a str)>,
    coordinates: impl Iterator<Item = (Position, Result<&'a ModuleRoot, &'a str>)>,
) -> RootSpecs {
    let modules = modules.filter_map(|(id, root)| Some((PrefabInstanceId::from_raw(id)?, Ok(root.clone()))));
    let faults = faults.filter_map(|(id, fault)| Some((PrefabInstanceId::from_raw(id)?, Err(fault.to_string()))));

    let errors = errors.filter_map(|(id, error)| Some((PrefabInstanceId::from_raw(id)?, Err(error.to_string()))));

    let coords = coordinates
        .filter_map(|(position, spec)| {
            let coord = Coord::new(
                u32::try_from(position.x).ok()?,
                u32::try_from(position.y).ok()?,
                u32::try_from(position.z).ok()?,
            );

            Some((coord, spec.cloned().map_err(str::to_string)))
        })
        .collect();

    RootSpecs {
        atoms: modules.chain(faults).chain(errors).collect(),
        coords,
    }
}

pub fn resolve_module_path(base_dir: &Path, name: &str) -> Result<PathBuf, ErrorKind> {
    let path = file_path::within(base_dir, name).ok_or(ErrorKind::OutsideCodebase)?;

    Ok(resolved_path(&path))
}

#[derive(Debug)]
pub struct LoadedModule {
    pub map: Arc<Map>,
    anchors: Mutex<HashMap<TypeId, Result<Anchor, ErrorKind>>>,
}

impl LoadedModule {
    pub fn new(map: Map) -> Self {
        Self {
            map: Arc::new(map),
            anchors: Mutex::default(),
        }
    }

    pub fn anchor(&self, tree: &ObjectTree, ty: Option<TypeId>) -> Result<Anchor, ErrorKind> {
        let Some(ty) = ty else {
            return Ok((Coord::new(1, 1, 1), None));
        };

        let mut anchors = self.anchors.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

        anchors
            .entry(ty)
            .or_insert_with(|| {
                let found = placements(&self.map)
                    .filter(|(_, _, prefab)| tree.id_of(&prefab.path).is_some_and(|id| tree.is_subtype_of(id, ty)))
                    .map(|(coord, index, _)| (coord, Some(index)))
                    .collect::<Vec<_>>();

                match found.as_slice() {
                    [anchor] => Ok(*anchor),
                    _ => Err(ErrorKind::Anchors { count: found.len() }),
                }
            })
            .clone()
    }
}

#[derive(Debug, Clone)]
pub struct OpenModule {
    pub document: DocumentId,
    pub generation: u64,
    pub module: Arc<LoadedModule>,
}

#[derive(Debug, Default)]
pub struct ModuleFiles {
    entries: HashMap<PathBuf, Result<Arc<LoadedModule>, ErrorKind>>,
    open: HashMap<PathBuf, OpenModule>,
}

impl ModuleFiles {
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn load(&mut self, path: &Path) -> Result<Arc<LoadedModule>, ErrorKind> {
        if let Some(open) = self.open.get(path) {
            return Ok(open.module.clone());
        }

        self.entries
            .entry(path.to_path_buf())
            .or_insert_with(|| read(path))
            .clone()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.open.is_empty()
    }

    pub fn is_loaded(&self, path: &Path) -> bool {
        self.entries.contains_key(path)
    }

    pub fn open_module(&self, path: &Path) -> Option<&OpenModule> {
        self.open.get(path)
    }

    pub fn open_modules(&self) -> impl Iterator<Item = (&Path, &OpenModule)> {
        self.open.iter().map(|(path, open)| (path.as_path(), open))
    }

    pub fn set_open(&mut self, path: &Path, open: Option<OpenModule>) {
        self.entries.remove(path);

        match open {
            Some(open) => {
                self.open.insert(path.to_path_buf(), open);
            },
            None => {
                self.open.remove(path);
            },
        }
    }

    pub fn open_only(&self) -> Self {
        Self {
            entries: HashMap::new(),
            open: self.open.clone(),
        }
    }
}

fn read(path: &Path) -> Result<Arc<LoadedModule>, ErrorKind> {
    let source = fs::read_to_string(path).map_err(|error| ErrorKind::Read {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    let (map, errors) = dmm::parser::parse(&source);
    if !errors.is_empty() {
        return Err(ErrorKind::Dmm(errors));
    }

    Ok(Arc::new(LoadedModule::new(map)))
}

pub(super) fn placements(map: &Map) -> impl Iterator<Item = (Coord, usize, &Prefab)> {
    map.grid.iter().enumerate().flat_map(move |(z, level)| {
        level.iter().enumerate().flat_map(move |(row, cells)| {
            cells.iter().enumerate().flat_map(move |(x, key)| {
                let coord = Coord::new(x as u32 + 1, map.size.y - row as u32, z as u32 + 1);
                map.dictionary
                    .get(key)
                    .into_iter()
                    .flatten()
                    .enumerate()
                    .map(move |(index, prefab)| (coord, index, prefab))
            })
        })
    })
}
