use base::{
    coord::Coord,
    path::TreePath,
    types::{Identifier, Value},
};

use prefab::{Prefab, PrefabInstanceId};

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
