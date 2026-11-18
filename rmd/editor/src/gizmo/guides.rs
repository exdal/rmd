use std::collections::HashSet;

use base::coord::Coord;
use prefab::PrefabInstanceId;
use render::GuideLine;

use crate::{document::DocumentId, frame::visual, session::Session};

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
    /// Expand a selected route node into its connected collection of guide segments.
    fn add_route_guides(&self, document_id: DocumentId, selected: PrefabInstanceId, guides: &mut PickGuides) {
        let Some(document) = self.state.document(document_id) else {
            return;
        };
        let Some(bake) = self.caches.get(&document_id).and_then(|cache| cache.bake.as_ref()) else {
            return;
        };
        let edges = bake.route_connections(selected.get());
        let edge_set: HashSet<_> = edges.iter().copied().collect();
        let size = self.options.tile_size.max(1) as f32;
        let center = |raw| {
            let id = PrefabInstanceId::from_raw(raw)?;
            let coord = document.instance_location(id)?.coord;
            (coord.z == document.z).then_some([(coord.x as f32 - 0.5) * size, (coord.y as f32 - 0.5) * size])
        };
        for (source, target) in edges {
            let (Some(origin), Some(destination)) = (center(source), center(target)) else {
                continue;
            };
            guides.lines.retain(|line| {
                !((line.origin == origin && line.target == destination)
                    || (line.origin == destination && line.target == origin))
            });
            guides.lines.extend(route_segments(
                origin,
                destination,
                size,
                edge_set.contains(&(target, source)),
            ));
            for raw in [source, target] {
                if let Some(id) = PrefabInstanceId::from_raw(raw)
                    && id != selected
                    && !guides.connected.contains(&id)
                {
                    guides.connected.push(id);
                }
            }
        }
    }

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
            solid: false,
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
                solid: false,
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

        self.add_route_guides(document_id, picked, &mut guides);
        guides
    }
}

fn route_segments(mut origin: [f32; 2], mut target: [f32; 2], size: f32, reciprocal: bool) -> Vec<GuideLine> {
    let mut lines = Vec::new();
    if origin == target {
        let points = [
            origin,
            [origin[0] + size * 0.4, origin[1]],
            [origin[0] + size * 0.4, origin[1] + size * 0.4],
            [origin[0], origin[1] + size * 0.4],
        ];
        for pair in points.windows(2) {
            lines.push(GuideLine {
                solid: true,
                origin: pair[0],
                target: pair[1],
            });
        }
        origin = points[3];
    }
    let delta = [target[0] - origin[0], target[1] - origin[1]];
    let length = delta[0].hypot(delta[1]);
    let tangent = [delta[0] / length, delta[1] / length];
    let normal = [-tangent[1], tangent[0]];
    if reciprocal && lines.is_empty() {
        for axis in 0..2 {
            origin[axis] += normal[axis] * size * 0.08;
            target[axis] += normal[axis] * size * 0.08;
        }
    }
    lines.push(GuideLine {
        solid: true,
        origin,
        target,
    });
    // Arrowheads at the middle remain visible when a beacon sprite covers its tile center.
    let tip = [(origin[0] + target[0]) * 0.5, (origin[1] + target[1]) * 0.5];
    let arrow = (size * 0.22).min(length * 0.25);
    for side in [-1.0, 1.0] {
        lines.push(GuideLine {
            solid: true,
            origin: [
                tip[0] - tangent[0] * arrow + normal[0] * arrow * side * 0.6,
                tip[1] - tangent[1] * arrow + normal[1] * arrow * side * 0.6,
            ],
            target: tip,
        });
    }
    lines
}

#[cfg(test)]
mod tests;
