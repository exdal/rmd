use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet},
};

use dmi::metadata::{Dir, Metadata};
use dmm::Coord;
use objtree::{ObjectTree, TypeId};
use render::{SpriteInstance, SpriteTexture, UpdateRange, texture::TextureCatalog};
use vm::matrix::Matrix;

use crate::{document::PrefabInstanceId, maploader::preview::MapView};

pub(crate) mod area;
mod context;
#[cfg(test)]
pub(crate) mod fixtures;
pub(crate) mod options;
mod pages;
pub(crate) mod session;
pub(crate) mod visual;

use area::{
    add_area_neighborhood, area_owners_at, build_area_components, index_area_component_tiles, remove_area_owner,
    repair_area_components,
};
use context::RenderContext;
use options::{FrameRenderOptions, TypeVisibility};
use pages::{OwnerKeys, SpritePage, replace_owner_sprites};
use visual::Appearance;

/// - 0: z level
/// - 1: plane encoded with `visual::sort_component`
/// - 2: owner's row, back rows first, `FLAT_ROW` outside side-view sorting
/// - 3: layer encoded the same way as the plane
/// - 4: whether the owner is not a turf, so equal-plane/layer turfs draw first across tiles
/// - 5: placement order, see `PlacementOrder`
/// - 6: sprite index within its placement, order goes like this: underlays, self, overlays
type SpriteKey = (u32, i32, Reverse<i32>, i32, bool, PlacementOrder, usize);

/// rows from the top, left to right, then the prefab's index in its tile, it follows where a placement is now
type PlacementOrder = (Reverse<u32>, u32, usize);

const FIRST_PLACEMENT: PlacementOrder = (Reverse(u32::MAX), 0, 0);

fn placement_order(coord: Coord, prefab_index: usize) -> PlacementOrder {
    (Reverse(coord.y), coord.x, prefab_index)
}

#[derive(Debug, Clone, Copy)]
struct CachedPlacement {
    coord: Coord,
    type_id: TypeId,
    is_area: bool,
    hidden: bool,
}

#[derive(Debug, Clone, Copy)]
struct CurrentPlacement {
    coord: Coord,
    type_id: TypeId,
    is_area: bool,
}

struct RenderedPrefab {
    is_area: bool,
    sprites: Vec<(SpriteKey, SpriteInstance)>,
    primary: Option<SpriteInstance>,
}

#[derive(Debug, Default)]
pub struct FrameInstances {
    pub sprites: Vec<SpriteInstance>,
    pub light_tiles: Vec<render::LightTile>,
    pub lighting_size: [u32; 3],
    area_components: HashMap<Coord, PrefabInstanceId>,
    area_component_tiles: HashMap<PrefabInstanceId, HashSet<Coord>>,
    sprite_sort_keys: Vec<SpriteKey>,
    pages: Vec<SpritePage>,
    owner_keys: HashMap<PrefabInstanceId, OwnerKeys>,
    /// only primaries that are not among their owner's sprites, the rest are found through `owner_keys`
    primary_sprites: HashMap<PrefabInstanceId, SpriteInstance>,
    placements: HashMap<PrefabInstanceId, CachedPlacement>,
    unknown_placements: HashMap<PrefabInstanceId, Coord>,
    area_owners_by_coord: HashMap<Coord, Vec<PrefabInstanceId>>,
}

impl FrameInstances {
    /// Frees the cache one part at a time and calls `freed` after each, so a caller that counts
    /// allocations can see what each part held.
    #[doc(hidden)]
    #[cfg(test)]
    pub fn release_in_stages(mut self, mut freed: impl FnMut(&'static str)) {
        drop(std::mem::take(&mut self.sprites));
        freed("sprites");
        drop(std::mem::take(&mut self.sprite_sort_keys));
        drop(std::mem::take(&mut self.pages));
        freed("sort keys and pages");
        drop(std::mem::take(&mut self.owner_keys));
        freed("owner keys");
        drop(std::mem::take(&mut self.primary_sprites));
        freed("primary sprites");
        drop(std::mem::take(&mut self.placements));
        drop(std::mem::take(&mut self.unknown_placements));
        freed("placements");
        drop(std::mem::take(&mut self.area_components));
        drop(std::mem::take(&mut self.area_component_tiles));
        drop(std::mem::take(&mut self.area_owners_by_coord));
        freed("area components");
        drop(self);
        freed("everything else");
    }

    pub fn sprite(&self, owner: PrefabInstanceId) -> Option<&SpriteInstance> {
        self.primary_sprites
            .get(&owner)
            .or_else(|| self.keyed_primary(owner))
            .filter(|sprite| !sprite.hidden)
    }

    pub fn area_component_at(&self, coord: Coord) -> Option<PrefabInstanceId> {
        self.area_components.get(&coord).copied()
    }

    pub fn area_component_tiles(&self, component: PrefabInstanceId) -> HashSet<Coord> {
        self.area_component_tiles.get(&component).cloned().unwrap_or_default()
    }

    pub fn unknown_type_coords(&self) -> impl Iterator<Item = Coord> + '_ {
        self.unknown_placements.values().copied()
    }

    pub fn unknown_type_instances(&self) -> impl Iterator<Item = PrefabInstanceId> + '_ {
        self.unknown_placements.keys().copied()
    }

    pub fn update_lighting(&mut self, lighting: Option<&vm::bake::LightingMap>, range: Option<std::ops::Range<usize>>) {
        let Some(lighting) = lighting else {
            self.light_tiles.clear();
            self.lighting_size = [0; 3];
            return;
        };

        let replace = self.lighting_size != lighting.size || self.light_tiles.len() != lighting.tiles.len();
        self.lighting_size = lighting.size;

        if replace {
            self.light_tiles = lighting.tiles.iter().copied().map(light_tile).collect();
            return;
        }

        let range = range.unwrap_or(0..lighting.tiles.len());
        for index in range.start.min(lighting.tiles.len())..range.end.min(lighting.tiles.len()) {
            self.light_tiles[index] = light_tile(lighting.tiles[index]);
        }
    }
}

fn light_tile(tile: vm::bake::LightTile) -> render::LightTile {
    render::LightTile { corners: tile.corners }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrefabUpdate {
    /// the edited value has no effect on the cached render data
    Unchanged,
    /// span of cached and changed stuff
    Buffers {
        /// disjoint and in order
        sprites: Vec<UpdateRange>,
    },
}

/// Turn a resolved [`Appearance`] into a renderable sprite instance.
///
/// An icon wider or taller than `tile_size` is anchored to the tile's bottom left and grows up and
/// to the right, the way BYOND draws one, so `pixel_x = -16` is what centres a 64 wide icon.
pub fn instance_for(
    owner: PrefabInstanceId, appearance: &Appearance, texture: SpriteTexture, tile: Coord, tile_size: u32,
    is_area: bool,
) -> SpriteInstance {
    let alpha = f32::from(appearance.alpha) / 255.0;
    let tint = appearance
        .color
        .as_deref()
        .and_then(render::color::parse)
        .unwrap_or([1.0; 4]);
    let alpha = alpha * tint[3];
    let offset_x = appearance
        .step_x
        .saturating_add(appearance.pixel_x)
        .saturating_add(appearance.pixel_w);
    let offset_y = appearance
        .step_y
        .saturating_add(appearance.pixel_y)
        .saturating_add(appearance.pixel_z);
    let [a, b, c, d, e, f] = Some(appearance.transform.0)
        .filter(|components| components.iter().all(|component| component.abs() <= 65_504.0))
        .unwrap_or(Matrix::IDENTITY.0);

    SpriteInstance {
        owner,
        area_owner: None,
        texture,
        x: (tile.x.saturating_sub(1) * tile_size) as f32 + offset_x as f32 + c,
        y: (tile.y.saturating_sub(1) * tile_size) as f32 + offset_y as f32 + f,
        width: texture.width as f32,
        height: texture.height as f32,
        z: tile.z,
        is_area,
        hidden: false,
        click_through: false,
        node_pickable: false,
        area_edges: 0,
        lighting: match appearance.lighting {
            vm::AppearanceLighting::Normal => render::SpriteLighting::Normal,
            vm::AppearanceLighting::Emissive => render::SpriteLighting::Emissive,
            vm::AppearanceLighting::Blocker => render::SpriteLighting::Blocker,
            vm::AppearanceLighting::OverlayLight => render::SpriteLighting::OverlayLight,
            vm::AppearanceLighting::OverlayLightSubtract => render::SpriteLighting::OverlayLightSubtract,
        },
        color: [tint[0] * alpha, tint[1] * alpha, tint[2] * alpha, alpha],
        depth: appearance.plane * 1000.0 + appearance.layer,
        transform: [a, b, d, e],
    }
}

pub fn build_view_with_options(
    tree: &ObjectTree, icons: &HashMap<String, Metadata>, textures: &TextureCatalog, map: MapView<'_>,
    options: FrameRenderOptions<'_>,
) -> FrameInstances {
    let mut keyed_sprites = Vec::new();
    let mut primary_sprites = HashMap::new();
    let mut owner_keys = HashMap::new();
    let mut placements = HashMap::new();
    let mut unknown_placements = HashMap::new();
    let mut area_owners_by_coord = HashMap::<Coord, Vec<PrefabInstanceId>>::new();
    let area = tree.roots().area;
    let area_components = build_area_components(tree, map);
    let area_component_tiles = index_area_component_tiles(&area_components);
    let render = RenderContext {
        node_types: options.node_types,
        appearances: options.appearances,
        tree,
        icons,
        textures,
        map,
        area,
        visibility: options.visibility,
        tile_size: options.tile_size,
        perspective: options.perspective,
    };

    for z in 1..=map.size().z.max(1) {
        for y in (1..=map.size().y).rev() {
            for x in 1..=map.size().x {
                let Some(tile) = map.tile_at(Coord::new(x, y, z)) else {
                    continue;
                };

                for (prefab_index, prefab) in tile.iter().enumerate() {
                    let Some(owner) = map.ids_at(Coord::new(x, y, z)).get(prefab_index).copied() else {
                        continue;
                    };
                    let Some(id) = tree.id_of(&prefab.path) else {
                        unknown_placements.insert(owner, Coord::new(x, y, z));
                        continue;
                    };

                    let coord = Coord::new(x, y, z);
                    let order = placement_order(coord, prefab_index);
                    let rendered = render.prefab(owner, prefab, id, coord, order, area_components.get(&coord).copied());
                    placements.insert(
                        owner,
                        CachedPlacement {
                            coord,
                            type_id: id,
                            is_area: rendered.is_area,
                            hidden: !options.visibility.is_visible(id),
                        },
                    );

                    if rendered.is_area {
                        area_owners_by_coord.entry(coord).or_default().push(owner);
                    }

                    let (keys, unkeyed_primary) = OwnerKeys::of(&rendered);
                    if let Some(keys) = keys {
                        owner_keys.insert(owner, keys);
                    }

                    if let Some(primary) = unkeyed_primary {
                        primary_sprites.insert(owner, primary);
                    }

                    keyed_sprites.extend(rendered.sprites);
                }
            }
        }
    }

    keyed_sprites.sort_by_key(|(key, _)| *key);
    let mut instances = FrameInstances {
        light_tiles: options
            .lighting
            .map(|lighting| lighting.tiles.iter().copied().map(light_tile).collect())
            .unwrap_or_default(),
        lighting_size: options.lighting.map_or([0; 3], |lighting| lighting.size),
        area_components,
        area_component_tiles,
        primary_sprites,
        placements,
        unknown_placements,
        area_owners_by_coord,
        owner_keys,
        ..Default::default()
    };
    instances.lay_out(keyed_sprites, map.size().z.max(1));

    instances
}

pub fn update_view_prefabs_with_options(
    instances: &mut FrameInstances, tree: &ObjectTree, icons: &HashMap<String, Metadata>, textures: &TextureCatalog,
    map: MapView<'_>, owners: &[PrefabInstanceId], options: FrameRenderOptions<'_>,
) -> PrefabUpdate {
    let mut owners = owners
        .iter()
        .copied()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    owners.sort_unstable_by_key(|owner| owner.get());

    for owner in &owners {
        match map
            .prefab_instance(*owner)
            .filter(|instance| tree.id_of(&instance.prefab().path).is_none())
        {
            Some(instance) => {
                instances.unknown_placements.insert(*owner, instance.location.coord);
            },
            None => {
                instances.unknown_placements.remove(owner);
            },
        }
    }

    let changes = owners
        .iter()
        .copied()
        .filter_map(|owner| {
            let old = instances.placements.get(&owner).copied();
            let current = current_placement(tree, map, owner);

            (old.is_some() || current.is_some()).then_some((owner, old, current))
        })
        .collect::<Vec<_>>();
    if changes.is_empty() {
        return PrefabUpdate::Unchanged;
    }

    let mut topology_coords = HashSet::new();
    let mut dirty_coords = HashSet::new();
    for (_, old, current) in &changes {
        if let Some(old) = old.filter(|placement| placement.is_area) {
            topology_coords.insert(old.coord);
            add_area_neighborhood(&mut dirty_coords, old.coord, map);
        }
        if let Some(current) = current.filter(|placement| placement.is_area) {
            topology_coords.insert(current.coord);
            add_area_neighborhood(&mut dirty_coords, current.coord, map);
        }
    }

    let mut affected = owners.into_iter().collect::<HashSet<_>>();
    // a changed tile moves the prefab indices the rest of its placements are ordered by, areas draw on a plane of
    // their own one to a tile, so theirs never matter
    for coord in changes.iter().flat_map(|(_, old, current)| {
        old.map(|old| old.coord)
            .into_iter()
            .chain(current.map(|current| current.coord))
    }) {
        affected.extend(map.ids_at(coord).iter().copied().filter(|owner| {
            instances
                .placements
                .get(owner)
                .is_some_and(|placement| !placement.is_area)
        }));
    }

    for coord in &dirty_coords {
        if let Some(owners) = instances.area_owners_by_coord.get(coord) {
            affected.extend(owners.iter().copied());
        }
        affected.extend(area_owners_at(tree, map, *coord));
    }
    for coord in repair_area_components(instances, tree, map, &topology_coords) {
        affected.extend(map.ids_at(coord).iter().copied());
    }

    for (owner, old, current) in &changes {
        if let Some(old) = old.filter(|placement| placement.is_area) {
            remove_area_owner(instances, old.coord, *owner);
        }
        match current {
            Some(current) => {
                instances.placements.insert(
                    *owner,
                    CachedPlacement {
                        coord: current.coord,
                        type_id: current.type_id,
                        is_area: current.is_area,
                        hidden: !options.visibility.is_visible(current.type_id),
                    },
                );
                if current.is_area {
                    let owners = instances.area_owners_by_coord.entry(current.coord).or_default();
                    if !owners.contains(owner) {
                        owners.push(*owner);
                    }
                }
            },
            None => {
                instances.placements.remove(owner);
            },
        }
    }

    let render = RenderContext {
        appearances: options.appearances,
        node_types: options.node_types,
        tree,
        icons,
        textures,
        map,
        area: tree.roots().area,
        visibility: options.visibility,
        tile_size: options.tile_size,
        perspective: options.perspective,
    };
    let mut affected = affected.into_iter().collect::<Vec<_>>();
    affected.sort_unstable_by_key(|candidate| candidate.get());
    let rendered = affected
        .into_iter()
        .map(|affected_owner| {
            let rendered = map.prefab_instance(affected_owner).and_then(|instance| {
                let prefab = instance.prefab();
                let location = instance.location;
                let id = tree.id_of(&prefab.path)?;

                Some(render.prefab(
                    affected_owner,
                    prefab,
                    id,
                    location.coord,
                    placement_order(location.coord, location.prefab_index),
                    instances.area_component_at(location.coord),
                ))
            });

            (affected_owner, rendered)
        })
        .collect::<Vec<_>>();
    let sprite_update = replace_owner_sprites(instances, &rendered);

    if sprite_update.is_empty() {
        return PrefabUpdate::Unchanged;
    }

    PrefabUpdate::Buffers { sprites: sprite_update }
}

fn current_placement(tree: &ObjectTree, map: MapView<'_>, owner: PrefabInstanceId) -> Option<CurrentPlacement> {
    let instance = map.prefab_instance(owner)?;
    let prefab = instance.prefab();
    let location = instance.location;
    let id = tree.id_of(&prefab.path)?;
    let is_area = tree.roots().area.is_some_and(|area| tree.is_subtype_of(id, area));

    Some(CurrentPlacement {
        coord: location.coord,
        type_id: id,
        is_area,
    })
}

pub fn sprite_texture(
    icons: &HashMap<String, Metadata>, textures: &TextureCatalog, appearance: &Appearance,
) -> Option<SpriteTexture> {
    let icon = appearance.icon.as_deref()?;
    let state = appearance.icon_state.as_deref().unwrap_or("");
    let metadata = icons.get(icon)?;
    let dir = Dir::from_bits(appearance.dir).unwrap_or(Dir::South);
    let cell = metadata.find(state)?.sprite_index(dir, 0);

    textures.lookup(icon, cell)
}

pub fn sprite_texture_or_missing(
    icons: &HashMap<String, Metadata>, textures: &TextureCatalog, appearance: &Appearance,
) -> Option<SpriteTexture> {
    sprite_texture(icons, textures, appearance).or_else(|| {
        appearance.icon.as_deref().filter(|icon| !icon.is_empty())?;
        if appearance.alpha == 0 || appearance.invisibility > 0 {
            return None;
        }
        textures.missing_icon()
    })
}

#[cfg(test)]
mod tests;

/// End to end over `examples/env`, which is the only place the whole chain - preprocess, parse,
/// lower, pack, resolve - is exercised without a GPU.
#[cfg(test)]
mod example_environment;
