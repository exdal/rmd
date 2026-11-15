use std::collections::{HashMap, HashSet};

use dmm::{Coord, Prefab};
use objtree::{ObjectTree, TypeId};
use render::{AREA_EDGE_EAST, AREA_EDGE_NORTH, AREA_EDGE_SOUTH, AREA_EDGE_WEST, SpriteInstance, SpriteTexture};

use crate::{document::PrefabInstanceId, maploader::preview::MapView};

use super::{FrameInstances, instance_for, visual::Appearance};

pub(super) fn area_outline(
    owner: PrefabInstanceId, appearance: &Appearance, texture: SpriteTexture, coord: Coord, tile_size: u32,
    area_owner: Option<PrefabInstanceId>,
) -> SpriteInstance {
    let mut outline = instance_for(owner, appearance, texture, coord, tile_size, true);
    outline.area_owner = area_owner;
    outline.x = (coord.x.saturating_sub(1) * tile_size) as f32;
    outline.y = (coord.y.saturating_sub(1) * tile_size) as f32;
    outline.width = tile_size as f32;
    outline.height = tile_size as f32;
    outline.color = [1.0; 4];

    outline
}

pub(super) fn add_area_neighborhood(coords: &mut HashSet<Coord>, coord: Coord, map: MapView<'_>) {
    coords.insert(coord);
    if coord.x > 1 {
        coords.insert(Coord::new(coord.x - 1, coord.y, coord.z));
    }
    if coord.x < map.size().x {
        coords.insert(Coord::new(coord.x + 1, coord.y, coord.z));
    }
    if coord.y > 1 {
        coords.insert(Coord::new(coord.x, coord.y - 1, coord.z));
    }
    if coord.y < map.size().y {
        coords.insert(Coord::new(coord.x, coord.y + 1, coord.z));
    }
}

pub(super) fn area_owners_at(tree: &ObjectTree, map: MapView<'_>, coord: Coord) -> Vec<PrefabInstanceId> {
    let Some(area) = tree.roots().area else {
        return Vec::new();
    };
    let Some(tile) = map.tile_at(coord) else {
        return Vec::new();
    };

    tile.iter()
        .zip(map.ids_at(coord))
        .filter_map(|(prefab, owner)| {
            let id = tree.id_of(&prefab.path)?;

            tree.is_subtype_of(id, area).then_some(*owner)
        })
        .collect()
}

pub(super) fn remove_area_owner(instances: &mut FrameInstances, coord: Coord, owner: PrefabInstanceId) {
    let Some(owners) = instances.area_owners_by_coord.get_mut(&coord) else {
        return;
    };
    owners.retain(|candidate| *candidate != owner);
    if owners.is_empty() {
        instances.area_owners_by_coord.remove(&coord);
    }
}

pub(super) fn repair_area_components(
    instances: &mut FrameInstances, tree: &ObjectTree, map: MapView<'_>, changed: &HashSet<Coord>,
) -> HashSet<Coord> {
    let Some(area) = tree.roots().area else {
        return HashSet::new();
    };
    if changed.is_empty() {
        return HashSet::new();
    }

    let mut impacted = HashSet::new();
    for coord in changed {
        if let Some(component) = instances.area_components.get(coord) {
            impacted.insert(*component);
        }
        let Some((prefab, _)) = area_instance_at(tree, area, map, *coord) else {
            continue;
        };
        for neighbor in cardinal_neighbors(*coord, map) {
            if area_prefab_at(tree, area, map, neighbor).is_some_and(|candidate| same_area(prefab, candidate))
                && let Some(component) = instances.area_components.get(&neighbor)
            {
                impacted.insert(*component);
            }
        }
    }

    let mut candidates = changed.clone();
    for component in &impacted {
        if let Some(tiles) = instances.area_component_tiles.get(component) {
            candidates.extend(tiles.iter().copied());
        }
    }
    let previous = candidates
        .iter()
        .map(|coord| (*coord, instances.area_components.get(coord).copied()))
        .collect::<HashMap<_, _>>();

    for component in impacted {
        if let Some(tiles) = instances.area_component_tiles.remove(&component) {
            for coord in tiles {
                instances.area_components.remove(&coord);
            }
        }
    }
    for coord in &candidates {
        instances.area_components.remove(coord);
    }

    let mut seeds = candidates.iter().copied().collect::<Vec<_>>();
    seeds.sort_unstable_by_key(|coord| (coord.z, coord.y, coord.x));
    let mut visited = HashSet::new();
    for seed in seeds {
        if !visited.insert(seed) {
            continue;
        }
        let Some((prefab, component)) = area_instance_at(tree, area, map, seed) else {
            continue;
        };
        let prefab = prefab.clone();
        let mut members = HashSet::new();
        let mut pending = vec![seed];
        while let Some(coord) = pending.pop() {
            members.insert(coord);
            for neighbor in cardinal_neighbors(coord, map) {
                if !candidates.contains(&neighbor)
                    || visited.contains(&neighbor)
                    || !area_prefab_at(tree, area, map, neighbor).is_some_and(|candidate| same_area(&prefab, candidate))
                {
                    continue;
                }

                visited.insert(neighbor);
                pending.push(neighbor);
            }
        }

        for coord in &members {
            instances.area_components.insert(*coord, component);
        }
        instances.area_component_tiles.insert(component, members);
    }

    candidates
        .into_iter()
        .filter(|coord| previous.get(coord).copied().flatten() != instances.area_components.get(coord).copied())
        .collect()
}

pub(super) fn build_area_components(tree: &ObjectTree, map: MapView<'_>) -> HashMap<Coord, PrefabInstanceId> {
    let mut components = HashMap::new();
    let Some(area) = tree.roots().area else {
        return components;
    };
    let mut visited = HashSet::new();

    for z in 1..=map.size().z.max(1) {
        for y in 1..=map.size().y {
            for x in 1..=map.size().x {
                let seed = Coord::new(x, y, z);
                if visited.contains(&seed) {
                    continue;
                }
                let Some((prefab, component_owner)) = area_instance_at(tree, area, map, seed) else {
                    visited.insert(seed);

                    continue;
                };
                let prefab = prefab.clone();
                let mut pending = vec![seed];
                visited.insert(seed);

                while let Some(coord) = pending.pop() {
                    components.insert(coord, component_owner);
                    for neighbor in cardinal_neighbors(coord, map) {
                        if visited.contains(&neighbor)
                            || !area_prefab_at(tree, area, map, neighbor)
                                .is_some_and(|candidate| same_area(&prefab, candidate))
                        {
                            continue;
                        }

                        visited.insert(neighbor);
                        pending.push(neighbor);
                    }
                }
            }
        }
    }

    components
}

pub(super) fn index_area_component_tiles(
    components: &HashMap<Coord, PrefabInstanceId>,
) -> HashMap<PrefabInstanceId, HashSet<Coord>> {
    let mut tiles = HashMap::<PrefabInstanceId, HashSet<Coord>>::new();
    for (coord, component) in components {
        tiles.entry(*component).or_default().insert(*coord);
    }

    tiles
}

pub(super) fn area_instance_at<'a>(
    tree: &ObjectTree, area: TypeId, map: MapView<'a>, coord: Coord,
) -> Option<(&'a Prefab, PrefabInstanceId)> {
    let tile = map.tile_at(coord)?;

    tile.iter().zip(map.ids_at(coord)).find_map(|(prefab, owner)| {
        let id = tree.id_of(&prefab.path)?;

        tree.is_subtype_of(id, area).then_some((prefab, *owner))
    })
}

fn cardinal_neighbors(coord: Coord, map: MapView<'_>) -> impl Iterator<Item = Coord> {
    [
        (coord.x > 1).then(|| Coord::new(coord.x - 1, coord.y, coord.z)),
        (coord.x < map.size().x).then(|| Coord::new(coord.x + 1, coord.y, coord.z)),
        (coord.y > 1).then(|| Coord::new(coord.x, coord.y - 1, coord.z)),
        (coord.y < map.size().y).then(|| Coord::new(coord.x, coord.y + 1, coord.z)),
    ]
    .into_iter()
    .flatten()
}

pub(super) fn area_edges(tree: &ObjectTree, area: TypeId, map: MapView<'_>, prefab: &Prefab, coord: Coord) -> u32 {
    let matches = |coord| area_prefab_at(tree, area, map, coord).is_some_and(|other| same_area(prefab, other));
    let mut edges = 0;

    if coord.y >= map.size().y || !matches(Coord::new(coord.x, coord.y + 1, coord.z)) {
        edges |= AREA_EDGE_NORTH;
    }
    if coord.x >= map.size().x || !matches(Coord::new(coord.x + 1, coord.y, coord.z)) {
        edges |= AREA_EDGE_EAST;
    }
    if coord.y <= 1 || !matches(Coord::new(coord.x, coord.y - 1, coord.z)) {
        edges |= AREA_EDGE_SOUTH;
    }
    if coord.x <= 1 || !matches(Coord::new(coord.x - 1, coord.y, coord.z)) {
        edges |= AREA_EDGE_WEST;
    }

    edges
}

pub(super) fn area_prefab_at<'a>(
    tree: &ObjectTree, area: TypeId, map: MapView<'a>, coord: Coord,
) -> Option<&'a Prefab> {
    map.tile_at(coord)?.iter().find(|prefab| {
        tree.id_of(&prefab.path)
            .is_some_and(|candidate| tree.is_subtype_of(candidate, area))
    })
}

/// Source spelling and variable order do not split otherwise identical area definitions.
pub fn same_area(left: &Prefab, right: &Prefab) -> bool {
    left.path == right.path
        && left.vars.len() == right.vars.len()
        && left
            .vars
            .iter()
            .all(|(name, value)| right.var(name).is_some_and(|other| other == &value.value))
}
