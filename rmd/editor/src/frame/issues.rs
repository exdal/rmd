use std::collections::HashSet;

use base::{coord::Coord, types::Identifier};
use bitflags::bitflags;
use objtree::{ObjectTree, TypeId};
use prefab::{Prefab, PrefabInstanceId};

use super::FrameInstances;
use crate::{
    document::grid::TileView,
    maploader::preview::MapView,
    tool::place::{is_area, is_missing_base},
};

bitflags! {
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    pub struct TileIssues: u8 {
        const MISSING_BASE = 1;
        const MULTIPLE_AREAS = 2;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstanceIssue {
    UnknownType,
    UndefinedVar(Identifier),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FlaggedInstance {
    // TODO: drop this copy of the location by having bake_edits/apply_bake_update callers pass the touched coords,
    // a deleted instance's old tile is only known from here
    pub(super) coord: Coord,
    pub(super) issues: Vec<InstanceIssue>,
}

pub(super) fn check_tile(tile: TileView<'_>) -> TileIssues {
    let mut issues = TileIssues::empty();
    issues.set(TileIssues::MISSING_BASE, is_missing_base(tile));
    issues.set(
        TileIssues::MULTIPLE_AREAS,
        tile.iter().filter(|prefab| is_area(prefab)).count() > 1,
    );

    issues
}

pub(super) fn check_instance(tree: &ObjectTree, prefab: &Prefab, ty: Option<TypeId>) -> Vec<InstanceIssue> {
    let Some(ty) = ty else {
        return vec![InstanceIssue::UnknownType];
    };

    prefab
        .vars
        .iter()
        .filter(|(name, _)| tree.var_inherited(ty, name).is_none())
        .map(|(name, _)| InstanceIssue::UndefinedVar(name.clone()))
        .collect()
}

// must run before `placements` takes the new state, it holds each known owner's old coord
pub(super) fn update(instances: &mut FrameInstances, tree: &ObjectTree, map: MapView<'_>, owners: &[PrefabInstanceId]) {
    let mut coords = HashSet::new();
    for owner in owners {
        let old = instances
            .placements
            .get(owner)
            .map(|placement| placement.coord)
            .or_else(|| instances.instance_issues.get(owner).map(|flagged| flagged.coord));
        coords.extend(old);

        let Some(instance) = map.prefab_instance(*owner) else {
            instances.instance_issues.remove(owner);
            continue;
        };

        let coord = instance.location.coord;
        coords.insert(coord);
        let prefab = instance.prefab();
        let issues = check_instance(tree, prefab, tree.id_of(&prefab.path));
        if issues.is_empty() {
            instances.instance_issues.remove(owner);
        } else {
            instances
                .instance_issues
                .insert(*owner, FlaggedInstance { coord, issues });
        }
    }

    for coord in coords {
        let issues = map.tile_at(coord).map(check_tile).unwrap_or_default();
        if issues.is_empty() {
            instances.tile_issues.remove(&coord);
        } else {
            instances.tile_issues.insert(coord, issues);
        }
    }
}
