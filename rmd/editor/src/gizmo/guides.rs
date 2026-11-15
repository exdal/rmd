use std::collections::HashSet;

use dmm::Coord;
use render::GuideLine;

use crate::{document::PrefabInstanceId, frame::visual, session::Session};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct GuideBadge {
    pub position: [f32; 2],
    pub z: u32,
}

#[derive(Debug, Default)]
pub(crate) struct PickGuides {
    pub lines: Vec<GuideLine>,
    pub badges: Vec<GuideBadge>,
    pub connected: Vec<PrefabInstanceId>,
}

impl Session {
    pub(crate) fn picked_offset_guide(&self) -> Option<GuideLine> {
        let environment = self.state.environment.as_ref()?;
        let document = self.state.active_document()?;
        let picked = document.picked_instance()?;
        let instance = document.prefab_instance(picked)?;
        let prefab = instance.prefab();
        let location = instance.location;
        if location.coord.z != document.z {
            return None;
        }

        let id = environment.tree.id_of(&prefab.path)?;
        let appearance = visual::resolve_id(&environment.tree, id, prefab);
        let displacement = [
            appearance
                .step_x
                .saturating_add(appearance.pixel_x)
                .saturating_add(appearance.pixel_w),
            appearance
                .step_y
                .saturating_add(appearance.pixel_y)
                .saturating_add(appearance.pixel_z),
        ];
        if displacement == [0, 0] {
            return None;
        }

        let sprite = self.instances()?.sprite(picked)?;
        let tile_size = self.options.tile_size.max(1) as f32;

        Some(GuideLine {
            origin: [
                (location.coord.x as f32 - 0.5) * tile_size,
                (location.coord.y as f32 - 0.5) * tile_size,
            ],
            target: [sprite.x + sprite.width * 0.5, sprite.y + sprite.height * 0.5],
        })
    }

    pub(crate) fn picked_guides(&self) -> PickGuides {
        let mut guides = PickGuides::default();
        if let Some(offset) = self.picked_offset_guide() {
            guides.lines.push(offset);
        }

        let Some(document_id) = self.state.active() else {
            return guides;
        };
        let Some(document) = self.state.document(document_id) else {
            return guides;
        };
        let Some(picked) = document.picked_instance() else {
            return guides;
        };
        let Some(picked_location) = document.instance_location(picked) else {
            return guides;
        };

        if picked_location.coord.z != document.z {
            return guides;
        }

        let Some(cache) = self.caches.get(&document_id) else {
            return guides;
        };

        let Some(bake) = cache.bake.as_ref() else {
            return guides;
        };

        let tile_size = self.options.tile_size.max(1) as f32;
        let center = |id: PrefabInstanceId, coord: Coord| {
            cache.instances.sprite(id).map_or_else(
                || [(coord.x as f32 - 0.5) * tile_size, (coord.y as f32 - 0.5) * tile_size],
                |sprite| [sprite.x + sprite.width * 0.5, sprite.y + sprite.height * 0.5],
            )
        };

        let pick_center = center(picked, picked_location.coord);
        let mut badged = HashSet::new();

        for connected in bake.connections(picked.get()) {
            let Some(connected) = PrefabInstanceId::from_raw(connected) else {
                continue;
            };
            let Some(location) = document.instance_location(connected) else {
                continue;
            };
            let target = center(connected, location.coord);
            guides.lines.push(GuideLine {
                origin: pick_center,
                target,
            });
            guides.connected.push(connected);

            if location.coord.z != document.z && badged.insert((location.coord.x, location.coord.y, location.coord.z)) {
                guides.badges.push(GuideBadge {
                    position: target,
                    z: location.coord.z,
                });
            }
        }

        guides
    }
}

#[cfg(test)]
mod tests;
