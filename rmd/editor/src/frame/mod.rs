use base::{path::TreePath, types::Identifier, vars};
use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet},
    sync::Arc,
};

use defines::{EFFECTS_LAYER, FLOAT_LAYER, KEEP_APART, KEEP_TOGETHER};
use dmi::metadata::{Dir, Metadata};
use dmm::{Coord, Prefab};
use objtree::{ObjectTree, TypeId};
use render::{
    AREA_EDGE_EAST, AREA_EDGE_NORTH, AREA_EDGE_SOUTH, AREA_EDGE_WEST, SpriteInstance, SpriteTexture, UpdateRange,
    texture::TextureCatalog,
};
use vm::{
    bake::{LightingMap, SharedAppearance},
    matrix::Matrix,
};

use crate::{document::PrefabInstanceId, environment::world::Perspective, maploader::preview::MapView};

#[cfg(test)]
pub(crate) mod fixtures;
mod pages;
pub(crate) mod session;
pub(crate) mod visual;

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

const FLAT_ROW: Reverse<i32> = Reverse(i32::MIN);

// an offset moves the row only once it carries the sprite's center onto the next tile, mappers nudge
// things a few pixels toward the viewer without changing what covers them
fn side_row(coord: Coord, appearance: &Appearance, tile_size: u32) -> Reverse<i32> {
    let tile_size = i64::from(tile_size.max(1));
    let offset = i64::from(appearance.pixel_y) + i64::from(appearance.step_y);
    let tiles = (offset.abs() + tile_size / 2) / tile_size * offset.signum();
    let row = i64::from(coord.y) + tiles;

    Reverse(row.clamp(i64::from(i32::MIN) + 1, i64::from(i32::MAX)) as i32)
}

/// rows from the top, left to right, then the prefab's index in its tile, it follows where a placement is now
type PlacementOrder = (Reverse<u32>, u32, usize);

const FIRST_PLACEMENT: PlacementOrder = (Reverse(u32::MAX), 0, 0);

fn placement_order(coord: Coord, prefab_index: usize) -> PlacementOrder {
    (Reverse(coord.y), coord.x, prefab_index)
}

const AREA_PLANE: f32 = f32::INFINITY;

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

struct SpriteGroup {
    plane: f32,
    layer: f32,
    // the parent's own icon floats at `FLOAT_LAYER`, so `FLOAT_LAYER - 1` sorts under it
    float: f32,
    keep_apart: bool,
    sprites: Vec<SpriteInstance>,
}

struct RenderContext<'a> {
    node_types: Option<&'a HashSet<TypeId>>,
    appearances: &'a HashMap<u64, SharedAppearance>,
    tree: &'a ObjectTree,
    icons: &'a HashMap<String, Metadata>,
    textures: &'a TextureCatalog,
    map: MapView<'a>,
    area: Option<TypeId>,
    visibility: &'a TypeVisibility,
    tile_size: u32,
    perspective: Perspective,
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TypeVisibility {
    hidden: HashSet<TypeId>,
}

impl TypeVisibility {
    pub fn is_visible(&self, id: TypeId) -> bool {
        !self.hidden.contains(&id)
    }

    pub fn set_subtree(&mut self, tree: &ObjectTree, root: TypeId, visible: bool) -> bool {
        if tree.get(root).is_none() {
            return false;
        }

        let mut changed = false;
        for id in tree.descendants(root) {
            changed |= if visible {
                self.hidden.remove(&id)
            } else {
                self.hidden.insert(id)
            };
        }

        changed
    }

    pub fn show_all(&mut self) -> bool {
        let changed = !self.hidden.is_empty();
        self.hidden.clear();

        changed
    }

    pub fn hides_any(&self) -> bool {
        !self.hidden.is_empty()
    }

    pub fn hidden_types(&self, tree: &ObjectTree) -> HiddenTypes {
        HiddenTypes {
            paths: self
                .hidden
                .iter()
                .filter_map(|id| tree.get(*id))
                .map(|decl| decl.path.segments.clone())
                .collect(),
            areas: None,
        }
    }
}

// this needs to have path as key, it needs to outlive map reloads
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HiddenTypes {
    paths: HashSet<Vec<Identifier>>,
    areas: Option<Arc<HashSet<Vec<Identifier>>>>,
}

impl HiddenTypes {
    pub fn with_areas(self, areas: Arc<HashSet<Vec<Identifier>>>) -> Self {
        Self {
            areas: Some(areas),
            ..self
        }
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty() && self.areas.as_ref().is_none_or(|areas| areas.is_empty())
    }

    pub fn hides(&self, prefab: &Prefab) -> bool {
        self.paths.contains(&prefab.path.segments)
            || self
                .areas
                .as_ref()
                .is_some_and(|areas| areas.contains(&prefab.path.segments))
    }
}

impl FromIterator<TreePath> for HiddenTypes {
    fn from_iter<I: IntoIterator<Item = TreePath>>(paths: I) -> Self {
        Self {
            paths: paths.into_iter().map(|path| path.segments).collect(),
            areas: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FrameRenderOptions<'a> {
    pub node_types: Option<&'a HashSet<TypeId>>,
    pub visibility: &'a TypeVisibility,
    pub tile_size: u32,
    /// What the bake made of each atom, keyed by `vm::bake` atom id, empty when baking is off
    pub appearances: &'a HashMap<u64, SharedAppearance>,
    pub lighting: Option<&'a LightingMap>,
    pub perspective: Perspective,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameOptions {
    pub show_areas: bool,
    pub show_area_outlines: bool,
    pub show_lighting: bool,
    /// Minimum displayed light luminance, from 0 to 100 percent.
    pub minimum_light_brightness_percent: u32,
    /// `world.icon_size`
    pub tile_size: u32,
    /// How many levels below the active one to draw behind it, 0 for none.
    pub underlay_depth: u32,
}

impl Default for FrameOptions {
    fn default() -> Self {
        Self {
            show_areas: false,
            show_area_outlines: true,
            show_lighting: true,
            minimum_light_brightness_percent: 0,
            tile_size: 32,
            underlay_depth: 3,
        }
    }
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

impl RenderContext<'_> {
    fn instance(
        &self, owner: PrefabInstanceId, appearance: &Appearance, texture: SpriteTexture, coord: Coord, is_area: bool,
    ) -> SpriteInstance {
        let mut sprite = instance_for(owner, appearance, texture, coord, self.tile_size, is_area);
        if self.textures.is_missing_icon(texture) {
            sprite.color = [sprite.color[3]; 4];
        }

        sprite
    }

    #[allow(clippy::too_many_arguments)]
    fn overlay_groups(
        &self, owner: PrefabInstanceId, parent: &Appearance, delta: &vm::AppearanceDelta, coord: Coord,
        area_owner: Option<PrefabInstanceId>, depth: usize,
    ) -> Vec<SpriteGroup> {
        if depth >= 32 {
            return Vec::new();
        }

        let appearance = visual::resolve_overlay(self.tree, parent, delta);
        let mut own = Vec::new();
        if let Some(texture) = sprite_texture(self.icons, self.textures, &appearance) {
            let mut sprite = self.instance(owner, &appearance, texture, coord, false);
            sprite.area_owner = area_owner;
            own.push(sprite);
        }

        let float = delta
            .vars
            .iter()
            .find(|(name, _)| name.as_str() == vars::LAYER)
            .and_then(|(_, value)| value.as_num())
            .filter(|layer| *layer < 0.0)
            .unwrap_or(FLOAT_LAYER);
        let mut groups = self.grouped(owner, &appearance, own, delta, coord, area_owner, depth + 1);
        for group in &mut groups {
            group.float = float;
        }

        groups
    }

    #[allow(clippy::too_many_arguments)]
    fn grouped(
        &self, owner: PrefabInstanceId, parent: &Appearance, own: Vec<SpriteInstance>, delta: &vm::AppearanceDelta,
        coord: Coord, area_owner: Option<PrefabInstanceId>, depth: usize,
    ) -> Vec<SpriteGroup> {
        let mut groups = Vec::with_capacity(delta.underlays.len() + delta.overlays.len() + 1);
        for extra in &delta.underlays {
            groups.extend(self.overlay_groups(owner, parent, extra, coord, area_owner, depth));
        }

        if !own.is_empty() {
            groups.push(SpriteGroup {
                plane: parent.plane,
                layer: parent.layer,
                float: FLOAT_LAYER,
                keep_apart: false,
                sprites: own,
            });
        }
        for extra in &delta.overlays {
            groups.extend(self.overlay_groups(owner, parent, extra, coord, area_owner, depth));
        }

        groups.sort_by(|left, right| {
            left.plane
                .total_cmp(&right.plane)
                .then_with(|| left.layer.total_cmp(&right.layer))
                .then_with(|| left.float.total_cmp(&right.float))
        });

        // TODO: type intrinsocs
        if parent.appearance_flags & KEEP_TOGETHER != 0 && !groups.is_empty() {
            let mut together = Vec::new();
            let mut apart = Vec::new();
            for group in groups {
                if group.keep_apart {
                    apart.push(group);
                } else {
                    together.extend(group.sprites);
                }
            }
            if !together.is_empty() {
                apart.push(SpriteGroup {
                    plane: parent.plane,
                    layer: parent.layer,
                    float: FLOAT_LAYER,
                    keep_apart: false,
                    sprites: together,
                });
            }
            groups = apart;
        }
        if parent.appearance_flags & KEEP_APART != 0 {
            for group in &mut groups {
                group.keep_apart = true;
            }
        }
        groups.sort_by(|left, right| {
            left.plane
                .total_cmp(&right.plane)
                .then_with(|| left.layer.total_cmp(&right.layer))
                .then_with(|| left.float.total_cmp(&right.float))
        });

        groups
    }

    fn prefab(
        &self, owner: PrefabInstanceId, prefab: &Prefab, id: TypeId, coord: Coord, order: PlacementOrder,
        area_owner: Option<PrefabInstanceId>,
    ) -> RenderedPrefab {
        let is_area = self.area.is_some_and(|area| self.tree.is_subtype_of(id, area));
        let is_turf = self
            .tree
            .roots()
            .turf
            .is_some_and(|turf| self.tree.is_subtype_of(id, turf));
        let delta = self.appearances.get(&owner.get());
        let appearance = delta
            .map(|delta| visual::resolve_delta(self.tree, id, prefab, delta))
            .unwrap_or_else(|| visual::resolve_id(self.tree, id, prefab));
        let texture = sprite_texture(self.icons, self.textures, &appearance);
        let mut own = Vec::with_capacity(if is_area { 2 } else { 1 });

        if is_area {
            if let Some(texture) = texture {
                let mut sprite = self.instance(owner, &appearance, texture, coord, true);
                sprite.area_owner = area_owner;
                own.push(sprite);
            }

            let edges = self
                .area
                .map_or(0, |area| area_edges(self.tree, area, self.map, prefab, coord));
            if edges != 0 {
                let mut outline = area_outline(
                    owner,
                    &appearance,
                    texture.unwrap_or_default(),
                    coord,
                    self.tile_size,
                    area_owner,
                );
                outline.area_edges = edges;
                own.push(outline);
            }
        } else if let Some(texture) = texture {
            let mut sprite = self.instance(owner, &appearance, texture, coord, false);
            sprite.area_owner = area_owner;
            own.push(sprite);
        }
        let primary = own.first().copied();

        let mut groups = if let Some(delta) = delta.filter(|_| !is_area) {
            self.grouped(owner, &appearance, own, delta, coord, area_owner, 0)
        } else if own.is_empty() {
            Vec::new()
        } else {
            vec![SpriteGroup {
                plane: if is_area { AREA_PLANE } else { appearance.plane },
                layer: appearance.layer,
                float: FLOAT_LAYER,
                keep_apart: false,
                sprites: own,
            }]
        };
        if groups.is_empty()
            && !is_area
            && let Some(fallback) = sprite_texture_or_missing(self.icons, self.textures, &appearance)
                .filter(|texture| self.textures.is_missing_icon(*texture))
        {
            let mut sprite = self.instance(owner, &appearance, fallback, coord, false);
            sprite.area_owner = area_owner;
            groups.push(SpriteGroup {
                plane: appearance.plane,
                layer: appearance.layer,
                float: FLOAT_LAYER,
                keep_apart: false,
                sprites: vec![sprite],
            });
        }

        let owner_row = match self.perspective {
            Perspective::Side => side_row(coord, &appearance, self.tile_size),
            Perspective::TopDown => FLAT_ROW,
        };
        let mut local_order = 0usize;
        let mut sprites = Vec::new();
        for group in groups {
            let row = if group.layer >= EFFECTS_LAYER {
                FLAT_ROW
            } else {
                owner_row
            };

            for sprite in group.sprites {
                let key = (
                    coord.z,
                    visual::sort_component(group.plane),
                    row,
                    visual::sort_component(group.layer),
                    !is_turf,
                    order,
                    local_order,
                );
                local_order = local_order.saturating_add(1);
                sprites.push((key, sprite));
            }
        }

        let mut primary = primary.or_else(|| sprites.first().map(|(_, sprite)| *sprite));
        let node_pickable = self.node_types.is_some_and(|types| types.contains(&id));
        for sprite in sprites.iter_mut().map(|(_, sprite)| sprite).chain(&mut primary) {
            sprite.node_pickable = node_pickable;
        }

        // `mouse_opacity = 0` on objects too would lock out mapped dirt and decals
        if is_turf && appearance.mouse_opacity == 0 {
            for sprite in sprites.iter_mut().map(|(_, sprite)| sprite).chain(&mut primary) {
                sprite.click_through = true;
            }
        }

        if !self.visibility.is_visible(id) {
            for sprite in sprites.iter_mut().map(|(_, sprite)| sprite).chain(&mut primary) {
                sprite.hidden = true;
            }
        }

        RenderedPrefab {
            is_area,
            sprites,
            primary,
        }
    }
}

fn area_outline(
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

fn add_area_neighborhood(coords: &mut HashSet<Coord>, coord: Coord, map: MapView<'_>) {
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

fn area_owners_at(tree: &ObjectTree, map: MapView<'_>, coord: Coord) -> Vec<PrefabInstanceId> {
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

fn remove_area_owner(instances: &mut FrameInstances, coord: Coord, owner: PrefabInstanceId) {
    let Some(owners) = instances.area_owners_by_coord.get_mut(&coord) else {
        return;
    };
    owners.retain(|candidate| *candidate != owner);
    if owners.is_empty() {
        instances.area_owners_by_coord.remove(&coord);
    }
}

fn repair_area_components(
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

fn build_area_components(tree: &ObjectTree, map: MapView<'_>) -> HashMap<Coord, PrefabInstanceId> {
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

fn index_area_component_tiles(
    components: &HashMap<Coord, PrefabInstanceId>,
) -> HashMap<PrefabInstanceId, HashSet<Coord>> {
    let mut tiles = HashMap::<PrefabInstanceId, HashSet<Coord>>::new();
    for (coord, component) in components {
        tiles.entry(*component).or_default().insert(*coord);
    }

    tiles
}

fn area_instance_at<'a>(
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

fn area_edges(tree: &ObjectTree, area: TypeId, map: MapView<'_>, prefab: &Prefab, coord: Coord) -> u32 {
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

fn area_prefab_at<'a>(tree: &ObjectTree, area: TypeId, map: MapView<'a>, coord: Coord) -> Option<&'a Prefab> {
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
