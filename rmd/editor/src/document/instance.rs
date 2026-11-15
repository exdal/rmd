use base::{
    path::TreePath,
    types::{Identifier, Value},
};

use dmm::{Coord, Prefab, PrefabInstanceId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrefabLocation {
    pub coord: Coord,
    pub prefab_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PrefabInstance<'a> {
    pub id: PrefabInstanceId,
    pub location: PrefabLocation,
    prefab: &'a Prefab,
}

impl<'a> PrefabInstance<'a> {
    pub(crate) fn new(id: PrefabInstanceId, prefab: &'a Prefab, location: PrefabLocation) -> Self {
        Self { id, location, prefab }
    }

    pub fn prefab(&self) -> &'a Prefab {
        self.prefab
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlacedPrefab {
    pub(super) id: PrefabInstanceId,
    pub(super) prefab: Prefab,
}

impl PlacedPrefab {
    pub fn id(&self) -> PrefabInstanceId {
        self.id
    }

    pub fn prefab(&self) -> &Prefab {
        &self.prefab
    }

    pub fn prefab_mut(&mut self) -> &mut Prefab {
        &mut self.prefab
    }
}

pub type PlacedTile = Vec<PlacedPrefab>;

#[derive(Debug, Clone, PartialEq)]
pub enum VarMutation {
    Set(Identifier, Value),
    Remove(Identifier),
}

#[derive(Debug, Clone, PartialEq)]
pub struct InstanceChange {
    pub id: PrefabInstanceId,
    pub to_coord: Option<Coord>,
    pub path: Option<TreePath>,
    pub mutations: Vec<VarMutation>,
}
