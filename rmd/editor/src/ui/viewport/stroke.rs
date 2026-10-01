use std::collections::HashSet;

use dmm::{Coord, Prefab, PrefabInstanceId};
use editor::{command::EditGroupId, tool::Tool};
use render::PlacementFlash;

const PLACEMENT_FLASH_DURATION: f64 = 0.25;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::ui) struct ActivePlacementFlash {
    pub(super) owner: PrefabInstanceId,
    pub(super) coord: Coord,
    pub(super) started_at: f64,
}

impl ActivePlacementFlash {
    pub(super) fn sample(self, now: f64) -> Option<PlacementFlash> {
        let elapsed = (now - self.started_at).max(0.0);
        if !elapsed.is_finite() || elapsed >= PLACEMENT_FLASH_DURATION {
            return None;
        }

        Some(PlacementFlash {
            owner: self.owner,
            strength: (1.0 - elapsed / PLACEMENT_FLASH_DURATION) as f32,
        })
    }
}

pub(super) fn active_placement_flash(
    active: &mut Option<ActivePlacementFlash>, now: f64, enabled: bool,
) -> Option<(Coord, PlacementFlash)> {
    if !enabled {
        *active = None;

        return None;
    }

    let placement = (*active)?;
    let Some(flash) = placement.sample(now) else {
        *active = None;

        return None;
    };

    Some((placement.coord, flash))
}

#[derive(Debug)]
struct TileStroke {
    z: u32,
    group: EditGroupId,
    visited: HashSet<Coord>,
}

impl TileStroke {
    fn new(z: u32) -> Self {
        Self {
            z,
            group: EditGroupId::new(),
            visited: HashSet::new(),
        }
    }

    fn visit(&mut self, coord: Coord) -> Option<EditGroupId> {
        (coord.z == self.z && self.visited.insert(coord)).then_some(self.group)
    }
}

#[derive(Debug)]
pub(in crate::ui) struct PlacementStroke {
    prefab: Prefab,
    tiles: TileStroke,
}

impl PlacementStroke {
    pub(super) fn new(prefab: Prefab, z: u32) -> Self {
        Self {
            prefab,
            tiles: TileStroke::new(z),
        }
    }

    pub(super) fn matches_context(&self, tool: Tool, prefab: Option<&Prefab>, z: u32) -> bool {
        tool == Tool::Place && prefab == Some(&self.prefab) && z == self.tiles.z
    }

    pub(super) fn visit(&mut self, coord: Coord) -> Option<EditGroupId> { self.tiles.visit(coord) }
}

#[derive(Debug)]
pub(in crate::ui) struct PickStroke {
    cursor: [u32; 2],
    tiles: TileStroke,
}

impl PickStroke {
    pub(super) fn new(cursor: [u32; 2], z: u32) -> Self {
        Self {
            cursor,
            tiles: TileStroke::new(z),
        }
    }

    pub(super) fn clear(&mut self, coord: Coord) -> Option<EditGroupId> { self.tiles.visit(coord) }

    pub(super) fn move_to(&mut self, cursor: [u32; 2]) -> bool {
        let moved = self.cursor != cursor;
        self.cursor = cursor;

        moved
    }
}
