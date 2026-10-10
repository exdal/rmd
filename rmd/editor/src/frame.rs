use core::{path::TreePath, types::Identifier, vars};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use defines::{FLOAT_LAYER, KEEP_APART, KEEP_TOGETHER};
use dmi::metadata::{Dir, Metadata};
use dmm::{Coord, Prefab};
use objtree::{ObjectTree, TypeId};
use render::{
    AREA_EDGE_EAST,
    AREA_EDGE_NORTH,
    AREA_EDGE_SOUTH,
    AREA_EDGE_WEST,
    SpriteInstance,
    SpriteTexture,
    UpdateRange,
    texture::TextureCatalog,
};
use vm::{
    bake::{LightingMap, SharedAppearance},
    matrix::Matrix,
};

use crate::{
    document::{MapDocument, PrefabInstanceId},
    grid::Grid,
    visual::{self, Appearance},
};

mod pages;

use pages::{OwnerKeys, SpritePage, replace_owner_sprites};

/// - 0: z level
/// - 1: plane encoded with `visual::sort_component`
/// - 2: layer encoded the same way as above
/// - 3: placement order, counted by level (top row left to right)
/// - 4: sprite index within its placement, order goes like this: underlays, self, overlays
type SpriteKey = (u32, i32, i32, usize, usize);

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
    appearances: &'a HashMap<u64, SharedAppearance>,
    tree: &'a ObjectTree,
    icons: &'a HashMap<String, Metadata>,
    textures: &'a TextureCatalog,
    map: &'a Grid,
    area: Option<TypeId>,
    visibility: &'a TypeVisibility,
    tile_size: u32,
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
    placement_orders: HashMap<PrefabInstanceId, usize>,
    next_placement_order: usize,
    placements: HashMap<PrefabInstanceId, CachedPlacement>,
    area_owners_by_coord: HashMap<Coord, Vec<PrefabInstanceId>>,
}

impl FrameInstances {
    /// Frees the cache one part at a time and calls `freed` after each, so a caller that counts
    /// allocations can see what each part held.
    #[doc(hidden)]
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
        drop(std::mem::take(&mut self.placement_orders));
        freed("placements and orders");
        drop(std::mem::take(&mut self.area_components));
        drop(std::mem::take(&mut self.area_component_tiles));
        drop(std::mem::take(&mut self.area_owners_by_coord));
        freed("area components");
        drop(self);
        freed("everything else");
    }

    pub fn reorder_placements(&mut self, ids: &[PrefabInstanceId]) -> bool {
        let mut orders = ids
            .iter()
            .filter_map(|id| self.placement_orders.get(id).copied())
            .collect::<Vec<_>>();

        if orders.len() != ids.len() {
            return false;
        }

        orders.sort_unstable();

        let mut is_changed = false;
        for (id, order) in ids.iter().zip(orders) {
            is_changed |= self.placement_orders.insert(*id, order) != Some(order);
        }

        is_changed
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

fn light_tile(tile: vm::bake::LightTile) -> render::LightTile { render::LightTile { corners: tile.corners } }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrefabUpdate {
    /// the edited value has no effect on the cached render data
    Unchanged,
    /// span of cached and changed stuff
    Buffers {
        /// disjoint and in order
        sprites: Vec<UpdateRange>,
    },
    Rebuild,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TypeVisibility {
    hidden: HashSet<TypeId>,
}

impl TypeVisibility {
    pub fn is_visible(&self, id: TypeId) -> bool { !self.hidden.contains(&id) }

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

    pub fn hides_any(&self) -> bool { !self.hidden.is_empty() }

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

    pub fn is_empty(&self) -> bool { self.paths.is_empty() && self.areas.as_ref().is_none_or(|areas| areas.is_empty()) }

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
    pub visibility: &'a TypeVisibility,
    pub tile_size: u32,
    /// What the bake made of each atom, keyed by `vm::bake` atom id, empty when baking is off
    pub appearances: &'a HashMap<u64, SharedAppearance>,
    pub lighting: Option<&'a LightingMap>,
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

pub fn build(
    tree: &ObjectTree, icons: &HashMap<String, Metadata>, textures: &TextureCatalog, document: &MapDocument,
    tile_size: u32,
) -> FrameInstances {
    build_with_options(
        tree,
        icons,
        textures,
        document,
        FrameRenderOptions {
            visibility: &TypeVisibility::default(),
            tile_size,
            appearances: &HashMap::new(),
            lighting: None,
        },
    )
}

pub fn build_with_options(
    tree: &ObjectTree, icons: &HashMap<String, Metadata>, textures: &TextureCatalog, document: &MapDocument,
    options: FrameRenderOptions<'_>,
) -> FrameInstances {
    let mut keyed_sprites = Vec::new();
    let mut primary_sprites = HashMap::new();
    let mut owner_keys = HashMap::new();
    let mut placement_orders = HashMap::new();
    let mut placements = HashMap::new();
    let mut area_owners_by_coord = HashMap::<Coord, Vec<PrefabInstanceId>>::new();
    let area = tree.roots().area;
    let map = &document.map;
    let area_components = build_area_components(tree, document);
    let area_component_tiles = index_area_component_tiles(&area_components);
    let mut order = 0usize;
    let render = RenderContext {
        appearances: options.appearances,
        tree,
        icons,
        textures,
        map,
        area,
        visibility: options.visibility,
        tile_size: options.tile_size,
    };

    for z in 1..=map.size().z.max(1) {
        for y in (1..=map.size().y).rev() {
            for x in 1..=map.size().x {
                let Some(tile) = map.tile_at(Coord::new(x, y, z)) else {
                    continue;
                };

                for (prefab_index, prefab) in tile.iter().enumerate() {
                    let Some(owner) = document.instance_ids_at(Coord::new(x, y, z)).get(prefab_index).copied() else {
                        continue;
                    };
                    let Some(id) = tree.id_of(&prefab.path) else {
                        continue;
                    };

                    order = order.saturating_add(1);
                    let coord = Coord::new(x, y, z);
                    let rendered = render.prefab(owner, prefab, id, coord, order, area_components.get(&coord).copied());
                    placement_orders.insert(owner, order);
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
        placement_orders,
        next_placement_order: order.saturating_add(1),
        placements,
        area_owners_by_coord,
        owner_keys,
        ..Default::default()
    };
    instances.lay_out(keyed_sprites, map.size().z.max(1));

    instances
}

pub fn update_prefab(
    instances: &mut FrameInstances, tree: &ObjectTree, icons: &HashMap<String, Metadata>, textures: &TextureCatalog,
    document: &MapDocument, owner: PrefabInstanceId, tile_size: u32,
) -> PrefabUpdate {
    update_prefabs(instances, tree, icons, textures, document, &[owner], tile_size)
}

pub fn update_prefabs(
    instances: &mut FrameInstances, tree: &ObjectTree, icons: &HashMap<String, Metadata>, textures: &TextureCatalog,
    document: &MapDocument, owners: &[PrefabInstanceId], tile_size: u32,
) -> PrefabUpdate {
    update_prefabs_with_options(
        instances,
        tree,
        icons,
        textures,
        document,
        owners,
        FrameRenderOptions {
            visibility: &TypeVisibility::default(),
            tile_size,
            appearances: &HashMap::new(),
            lighting: None,
        },
    )
}

pub fn update_prefabs_with_options(
    instances: &mut FrameInstances, tree: &ObjectTree, icons: &HashMap<String, Metadata>, textures: &TextureCatalog,
    document: &MapDocument, owners: &[PrefabInstanceId], options: FrameRenderOptions<'_>,
) -> PrefabUpdate {
    let mut owners = owners
        .iter()
        .copied()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    owners.sort_unstable_by_key(|owner| owner.get());

    let changes = owners
        .iter()
        .copied()
        .filter_map(|owner| {
            let old = instances.placements.get(&owner).copied();
            let current = current_placement(tree, document, owner);

            (old.is_some() || current.is_some()).then_some((owner, old, current))
        })
        .collect::<Vec<_>>();
    if changes.is_empty() {
        return PrefabUpdate::Unchanged;
    }

    for (owner, old, current) in &changes {
        if old.is_none() && current.is_some() {
            let order = instances.next_placement_order;
            instances.next_placement_order = instances.next_placement_order.saturating_add(1);
            instances.placement_orders.insert(*owner, order);
        }
    }

    let map = &document.map;
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
    for coord in &dirty_coords {
        if let Some(owners) = instances.area_owners_by_coord.get(coord) {
            affected.extend(owners.iter().copied());
        }
        affected.extend(area_owners_at(tree, document, *coord));
    }
    for coord in repair_area_components(instances, tree, document, &topology_coords) {
        affected.extend(document.instance_ids_at(coord).iter().copied());
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
        tree,
        icons,
        textures,
        map,
        area: tree.roots().area,
        visibility: options.visibility,
        tile_size: options.tile_size,
    };
    let mut affected = affected.into_iter().collect::<Vec<_>>();
    affected.sort_unstable_by_key(|candidate| candidate.get());
    let rendered = affected
        .into_iter()
        .map(|affected_owner| {
            let rendered = document.prefab_instance(affected_owner).and_then(|instance| {
                let prefab = instance.prefab();
                let location = instance.location;
                let id = tree.id_of(&prefab.path)?;
                let order = instances.placement_orders.get(&affected_owner).copied()?;

                Some(render.prefab(
                    affected_owner,
                    prefab,
                    id,
                    location.coord,
                    order,
                    instances.area_component_at(location.coord),
                ))
            });

            (affected_owner, rendered)
        })
        .collect::<Vec<_>>();
    let sprite_update = replace_owner_sprites(instances, &rendered);

    for (owner, _, current) in changes {
        if current.is_none() {
            instances.placement_orders.remove(&owner);
        }
    }

    if sprite_update.is_empty() {
        return PrefabUpdate::Unchanged;
    }

    PrefabUpdate::Buffers { sprites: sprite_update }
}

fn current_placement(tree: &ObjectTree, document: &MapDocument, owner: PrefabInstanceId) -> Option<CurrentPlacement> {
    let instance = document.prefab_instance(owner)?;
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
        &self, owner: PrefabInstanceId, prefab: &Prefab, id: TypeId, coord: Coord, order: usize,
        area_owner: Option<PrefabInstanceId>,
    ) -> RenderedPrefab {
        let is_area = self.area.is_some_and(|area| self.tree.is_subtype_of(id, area));
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
        let mut local_order = 0usize;
        let mut sprites = Vec::new();
        for group in groups {
            for sprite in group.sprites {
                let key = (
                    coord.z,
                    visual::sort_component(group.plane),
                    visual::sort_component(group.layer),
                    order,
                    local_order,
                );
                local_order = local_order.saturating_add(1);
                sprites.push((key, sprite));
            }
        }

        let mut primary = primary.or_else(|| sprites.first().map(|(_, sprite)| *sprite));
        // `mouse_opacity = 0` on objects too would lock out mapped dirt and decals
        let is_turf = self
            .tree
            .roots()
            .turf
            .is_some_and(|turf| self.tree.is_subtype_of(id, turf));
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

fn add_area_neighborhood(coords: &mut HashSet<Coord>, coord: Coord, map: &Grid) {
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

fn area_owners_at(tree: &ObjectTree, document: &MapDocument, coord: Coord) -> Vec<PrefabInstanceId> {
    let Some(area) = tree.roots().area else {
        return Vec::new();
    };
    let Some(tile) = document.map.tile_at(coord) else {
        return Vec::new();
    };

    tile.iter()
        .zip(document.instance_ids_at(coord))
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
    instances: &mut FrameInstances, tree: &ObjectTree, document: &MapDocument, changed: &HashSet<Coord>,
) -> HashSet<Coord> {
    let Some(area) = tree.roots().area else {
        return HashSet::new();
    };
    if changed.is_empty() {
        return HashSet::new();
    }

    let map = &document.map;
    let mut impacted = HashSet::new();
    for coord in changed {
        if let Some(component) = instances.area_components.get(coord) {
            impacted.insert(*component);
        }
        let Some((prefab, _)) = area_instance_at(tree, area, document, *coord) else {
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
        let Some((prefab, component)) = area_instance_at(tree, area, document, seed) else {
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

fn build_area_components(tree: &ObjectTree, document: &MapDocument) -> HashMap<Coord, PrefabInstanceId> {
    let mut components = HashMap::new();
    let Some(area) = tree.roots().area else {
        return components;
    };
    let map = &document.map;
    let mut visited = HashSet::new();

    for z in 1..=map.size().z.max(1) {
        for y in 1..=map.size().y {
            for x in 1..=map.size().x {
                let seed = Coord::new(x, y, z);
                if visited.contains(&seed) {
                    continue;
                }
                let Some((prefab, component_owner)) = area_instance_at(tree, area, document, seed) else {
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
    tree: &ObjectTree, area: TypeId, document: &'a MapDocument, coord: Coord,
) -> Option<(&'a Prefab, PrefabInstanceId)> {
    let tile = document.map.tile_at(coord)?;

    tile.iter()
        .zip(document.instance_ids_at(coord))
        .find_map(|(prefab, owner)| {
            let id = tree.id_of(&prefab.path)?;

            tree.is_subtype_of(id, area).then_some((prefab, *owner))
        })
}

fn cardinal_neighbors(coord: Coord, map: &Grid) -> impl Iterator<Item = Coord> {
    [
        (coord.x > 1).then(|| Coord::new(coord.x - 1, coord.y, coord.z)),
        (coord.x < map.size().x).then(|| Coord::new(coord.x + 1, coord.y, coord.z)),
        (coord.y > 1).then(|| Coord::new(coord.x, coord.y - 1, coord.z)),
        (coord.y < map.size().y).then(|| Coord::new(coord.x, coord.y + 1, coord.z)),
    ]
    .into_iter()
    .flatten()
}

fn area_edges(tree: &ObjectTree, area: TypeId, map: &Grid, prefab: &Prefab, coord: Coord) -> u32 {
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

fn area_prefab_at<'a>(tree: &ObjectTree, area: TypeId, map: &'a Grid, coord: Coord) -> Option<&'a Prefab> {
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
mod tests {
    use core::{
        location::Location,
        path::TreePath,
        types::{Identifier, Value, VarModifiers},
    };
    use std::{collections::HashMap, path::PathBuf, sync::Arc};

    use dmi::{
        IconFile,
        metadata::{Dir, IconState, Metadata},
    };
    use dmm::{Coord, Map, Prefab, Size};
    use objtree::{ObjectTree, VarDecl};
    use render::{
        AREA_EDGE_EAST,
        AREA_EDGE_NORTH,
        AREA_EDGE_SOUTH,
        AREA_EDGE_WEST,
        AREA_EDGES_ALL,
        IDENTITY_TRANSFORM,
        SpriteInstance,
        SpriteTexture,
        UpdateRange,
        texture::TextureCatalog,
    };
    use vm::{AppearanceDelta, bake::SharedAppearance, matrix::Matrix};

    use super::{
        FrameInstances,
        FrameRenderOptions,
        PrefabUpdate,
        TypeVisibility,
        build,
        build_with_options,
        instance_for,
        pages::tests::assert_pages_hold,
        update_prefab,
        update_prefabs,
    };
    use crate::{
        command::Edit,
        document::{MapDocument, PrefabInstanceId, VarMutation},
        visual::Appearance,
    };

    const ICON: &str = "test.dmi";

    pub(super) fn document(map: Map) -> MapDocument { MapDocument::new(map, 1) }

    fn owner() -> PrefabInstanceId { PrefabInstanceId::from_raw(1).expect("nonzero prefab instance ID") }

    fn appearance(color: Option<&str>, alpha: u8) -> Appearance {
        Appearance {
            color: color.map(String::from),
            alpha,
            ..Default::default()
        }
    }

    #[test]
    fn a_tint_is_premultiplied_by_alpha() {
        let instance = instance_for(
            owner(),
            &appearance(Some("#ff0000"), 128),
            SpriteTexture::default(),
            dmm::Coord::new(1, 1, 1),
            32,
            false,
        );
        let alpha = 128.0 / 255.0;

        assert_eq!(instance.color, [alpha, 0.0, 0.0, alpha]);
    }

    #[test]
    fn an_untinted_sprite_keeps_its_alpha_on_every_channel() {
        let instance = instance_for(
            owner(),
            &appearance(None, 255),
            SpriteTexture::default(),
            dmm::Coord::new(1, 1, 1),
            32,
            false,
        );

        assert_eq!(instance.color, [1.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn an_unreadable_color_falls_back_to_no_tint() {
        let instance = instance_for(
            owner(),
            &appearance(Some("chartreuse"), 255),
            SpriteTexture::default(),
            dmm::Coord::new(1, 1, 1),
            32,
            false,
        );

        assert_eq!(instance.color, [1.0, 1.0, 1.0, 1.0]);
    }

    /// Step and both pixel offset pairs stack rather than replacing each other.
    #[test]
    fn all_offset_pairs_shift_the_sprite() {
        let appearance = Appearance {
            pixel_x: -16,
            pixel_y: 4,
            pixel_w: 2,
            pixel_z: -1,
            step_x: 3,
            step_y: -2,
            ..Default::default()
        };
        let instance = instance_for(
            owner(),
            &appearance,
            SpriteTexture::default(),
            dmm::Coord::new(2, 3, 1),
            32,
            false,
        );

        assert_eq!((instance.x, instance.y), (32.0 - 11.0, 64.0 + 1.0));
    }

    /// A 64x64 icon hangs off the top and the right of its tile, so `pixel_x = -16` centres it.
    #[test]
    fn a_larger_than_tile_icon_anchors_to_the_bottom_left_of_its_tile() {
        let texture = SpriteTexture {
            index: 0,
            source_position: [0, 0],
            width: 64,
            height: 64,
        };
        let centred = Appearance {
            pixel_x: -16,
            ..Default::default()
        };

        let plain = instance_for(
            owner(),
            &Appearance::default(),
            texture,
            dmm::Coord::new(1, 1, 1),
            32,
            false,
        );
        assert_eq!((plain.x, plain.y), (0.0, 0.0));

        let shifted = instance_for(owner(), &centred, texture, dmm::Coord::new(1, 1, 1), 32, false);
        assert_eq!((shifted.x, shifted.y), (-16.0, 0.0));
    }

    /// `smoothrocks.dmi` is 40x40, so tgstation pulls it back 4 pixels to sit centered on its tile.
    #[test]
    fn a_transform_translates_the_sprite_and_turns_it_about_its_center() {
        let texture = SpriteTexture {
            index: 0,
            source_position: [0, 0],
            width: 40,
            height: 40,
        };
        let appearance = Appearance {
            transform: Matrix::turn(90.0).then(Matrix::translate(-4.0, -4.0)),
            ..Default::default()
        };

        let instance = instance_for(owner(), &appearance, texture, dmm::Coord::new(1, 1, 1), 32, false);
        let [a, b, d, e] = instance.transform;
        assert_eq!((instance.x, instance.y), (-4.0, -4.0));
        assert!(a.abs() < 1e-6 && (b - 1.0).abs() < 1e-6 && (d + 1.0).abs() < 1e-6 && e.abs() < 1e-6);

        let unpackable = Appearance {
            transform: Matrix([f32::NAN, 0.0, 3.0, 0.0, 1.0, 0.0]),
            ..Default::default()
        };
        let instance = instance_for(owner(), &unpackable, texture, dmm::Coord::new(1, 1, 1), 32, false);
        assert_eq!((instance.x, instance.transform), (0.0, IDENTITY_TRANSFORM));
    }

    #[test]
    fn an_instance_keeps_its_level_and_area_classification() {
        let instance = instance_for(
            owner(),
            &Appearance::default(),
            SpriteTexture::default(),
            dmm::Coord::new(1, 1, 7),
            32,
            true,
        );

        assert_eq!(instance.z, 7);
        assert!(instance.is_area);
    }

    fn icon_file(states: &[&str]) -> IconFile {
        let cells = states.len() as u32;

        IconFile {
            path: PathBuf::from(ICON),
            metadata: metadata(states),
            sheet_width: 32 * cells,
            sheet_height: 32,
            pixels: vec![255; (32 * cells * 32 * 4) as usize],
        }
    }

    fn metadata(states: &[&str]) -> Metadata {
        Metadata {
            version: String::from("4.0"),
            width: 32,
            height: 32,
            states: states
                .iter()
                .enumerate()
                .map(|(index, name)| IconState {
                    name: String::from(*name),
                    dirs: 1,
                    frames: 1,
                    offset: index,
                    ..Default::default()
                })
                .collect(),
        }
    }

    /// `/obj/thing { icon = 'test.dmi'; icon_state = ...; layer = ... }`
    pub(super) fn tree(types: &[(&str, &str, f32)]) -> ObjectTree {
        let mut tree = ObjectTree::new();

        for (path, state, layer) in types {
            let id = tree.register(&TreePath::parse(path), Location::default());
            let Some(decl) = tree.get_mut(id) else {
                continue;
            };

            let mut var = |name: &str, value: Value| {
                let name = Identifier::from(String::from(name));
                decl.vars.insert(
                    name.clone(),
                    VarDecl {
                        name,
                        declared_type: None,
                        modifiers: VarModifiers::default(),
                        value,
                        declared: true,
                        location: Location::default(),
                        resolved_type: None,
                    },
                );
            };

            var("icon", Value::Resource(String::from(ICON)));
            var("icon_state", Value::Text(String::from(*state)));
            var("layer", Value::Num(*layer));
        }

        tree
    }

    #[test]
    fn type_visibility_toggles_complete_subtrees_without_changing_siblings() {
        let tree = tree(&[
            ("/obj/parent", "floor", 1.0),
            ("/obj/parent/child", "floor", 1.0),
            ("/obj/sibling", "floor", 1.0),
        ]);
        let parent = tree.id_of(&TreePath::parse("/obj/parent")).unwrap();
        let child = tree.id_of(&TreePath::parse("/obj/parent/child")).unwrap();
        let sibling = tree.id_of(&TreePath::parse("/obj/sibling")).unwrap();
        let mut visibility = TypeVisibility::default();

        assert!(visibility.set_subtree(&tree, parent, false));
        assert!(!visibility.is_visible(parent));
        assert!(!visibility.is_visible(child));
        assert!(visibility.is_visible(sibling));

        assert!(visibility.set_subtree(&tree, child, true));
        assert!(!visibility.is_visible(parent));
        assert!(visibility.is_visible(child));
        assert!(visibility.is_visible(sibling));
    }

    #[test]
    fn hidden_types_match_prefabs_by_path() {
        let tree = tree(&[("/obj/parent", "floor", 1.0), ("/obj/parent/child", "floor", 1.0)]);
        let parent = tree.id_of(&TreePath::parse("/obj/parent")).unwrap();
        let mut visibility = TypeVisibility::default();
        assert!(visibility.hidden_types(&tree).is_empty());

        visibility.set_subtree(&tree, parent, false);
        let hidden = visibility.hidden_types(&tree);

        assert!(hidden.hides(&Prefab::new(TreePath::parse("/obj/parent/child"))));
        assert!(!hidden.hides(&Prefab::new(TreePath::parse("/obj/other"))));
    }

    #[test]
    fn hidden_types_keep_their_sprites_flagged() {
        let tree = tree(&[("/obj/table", "table", 2.0), ("/area/station", "floor", 1.0)]);
        let document = document(one_tile_map(&["/obj/table", "/area/station"]));
        let coord = Coord::new(1, 1, 1);
        let object_owner = document.instance_ids_at(coord)[0];
        let area_owner = document.instance_ids_at(coord)[1];
        let icons = icons(&["floor", "table"]);
        let textures = textures(&["floor", "table"]);
        let object = tree.id_of(&TreePath::parse("/obj")).unwrap();
        let area = tree.id_of(&TreePath::parse("/area")).unwrap();
        let mut visibility = TypeVisibility::default();

        visibility.set_subtree(&tree, object, false);
        let without_objects = build_with_options(
            &tree,
            &icons,
            &textures,
            &document,
            FrameRenderOptions {
                visibility: &visibility,
                tile_size: 32,
                appearances: &HashMap::new(),
                lighting: None,
            },
        );
        assert!(
            without_objects
                .live_sprites()
                .all(|sprite| sprite.owner != object_owner)
        );
        assert!(without_objects.live_sprites().any(|sprite| sprite.owner == area_owner));
        assert!(without_objects.sprite(object_owner).is_none());
        assert!(
            without_objects
                .sprites
                .iter()
                .any(|sprite| sprite.owner == object_owner && sprite.hidden)
        );

        visibility.set_subtree(&tree, object, true);
        visibility.set_subtree(&tree, area, false);
        let without_areas = build_with_options(
            &tree,
            &icons,
            &textures,
            &document,
            FrameRenderOptions {
                visibility: &visibility,
                tile_size: 32,
                appearances: &HashMap::new(),
                lighting: None,
            },
        );
        assert!(without_areas.live_sprites().any(|sprite| sprite.owner == object_owner));
        assert!(without_areas.live_sprites().all(|sprite| sprite.owner != area_owner));
    }

    #[test]
    fn a_shown_area_draws_above_emissive_blockers() {
        let tree = tree(&[("/obj/door", "table", 3.0), ("/area/station", "floor", 1.0)]);
        let mut map = Map::new(Size { x: 1, y: 1, z: 1 });
        let mut door = Prefab::new(TreePath::parse("/obj/door"));
        door.set_var(Identifier::from("plane"), Value::Num(13.0));
        map.grid[0][0][0] = map.intern_tile(vec![door, Prefab::new(TreePath::parse("/area/station"))]);
        let document = document(map);
        let owners = document.instance_ids_at(Coord::new(1, 1, 1));
        let icons = icons(&["floor", "table"]);
        let textures = textures(&["floor", "table"]);

        let instances = build(&tree, &icons, &textures, &document, 32);
        let drawn = instances
            .live_sprites()
            .filter(|sprite| sprite.area_edges == 0)
            .map(|sprite| sprite.owner)
            .collect::<Vec<_>>();

        assert_eq!(drawn, [owners[0], owners[1]]);
    }

    #[test]
    fn only_turfs_with_mouse_opacity_zero_are_click_through() {
        let tree = tree(&[("/turf/openspace", "floor", 2.0), ("/obj/dirt", "table", 3.0)]);
        let mut map = Map::new(Size { x: 1, y: 1, z: 1 });
        let tile = ["/turf/openspace", "/obj/dirt"]
            .into_iter()
            .map(|path| {
                let mut prefab = Prefab::new(TreePath::parse(path));
                prefab.set_var(Identifier::from("mouse_opacity"), Value::Num(0.0));
                prefab
            })
            .collect();
        map.grid[0][0][0] = map.intern_tile(tile);
        let document = document(map);
        let owners = document.instance_ids_at(Coord::new(1, 1, 1));
        let icons = icons(&["floor", "table"]);
        let textures = textures(&["floor", "table"]);

        let instances = build(&tree, &icons, &textures, &document, 32);
        let click_through = |owner| {
            instances
                .live_sprites()
                .filter(|sprite| sprite.owner == owner)
                .map(|sprite| sprite.click_through)
                .collect::<Vec<_>>()
        };

        assert_eq!(click_through(owners[0]), [true]);
        assert_eq!(click_through(owners[1]), [false]);
    }

    fn one_tile_map(paths: &[&str]) -> Map {
        let mut map = Map::new(Size { x: 1, y: 1, z: 1 });
        let tile = paths.iter().map(|p| Prefab::new(TreePath::parse(p))).collect();
        let key = map.intern_tile(tile);
        if let Some(slot) = map
            .grid
            .get_mut(0)
            .and_then(|z| z.get_mut(0))
            .and_then(|r| r.get_mut(0))
        {
            *slot = key;
        }

        map
    }

    fn area_map(width: u32, height: u32, path: &str) -> Map {
        let mut map = Map::new(Size {
            x: width,
            y: height,
            z: 1,
        });
        let key = map.intern_tile(vec![Prefab::new(TreePath::parse(path))]);
        for row in &mut map.grid[0] {
            row.fill(key);
        }

        map
    }

    /// One tile per z level, each holding the type named for it.
    fn layered_map(levels: &[&str]) -> Map {
        let mut map = Map::new(Size {
            x: 1,
            y: 1,
            z: levels.len() as u32,
        });

        for (index, path) in levels.iter().enumerate() {
            let key = map.intern_tile(vec![Prefab::new(TreePath::parse(path))]);
            if let Some(slot) = map
                .grid
                .get_mut(index)
                .and_then(|z| z.get_mut(0))
                .and_then(|r| r.get_mut(0))
            {
                *slot = key;
            }
        }

        map
    }

    pub(super) fn icons(states: &[&str]) -> HashMap<String, Metadata> {
        HashMap::from([(String::from(ICON), metadata(states))])
    }

    pub(super) fn textures(states: &[&str]) -> TextureCatalog {
        let mut textures = TextureCatalog::new();
        textures.insert(ICON, &icon_file(states)).expect("insert");

        textures
    }

    fn shared(appearances: HashMap<u64, AppearanceDelta>) -> HashMap<u64, SharedAppearance> {
        appearances
            .into_iter()
            .map(|(id, appearance)| (id, Arc::new(appearance)))
            .collect()
    }

    /// The sprites a list draws, in draw order, without the pages' spare slots.
    fn drawn(instances: &FrameInstances) -> Vec<SpriteInstance> { instances.live_sprites().copied().collect() }

    pub(super) fn assert_render_data_matches(actual: &FrameInstances, expected: &FrameInstances) {
        // placements made after the build take later orders than a fresh build gives them, so
        // equal layers may draw in another order, which assert_pages_hold checks is sorted
        let mut remaining = drawn(expected);
        for sprite in actual.live_sprites() {
            let index = remaining
                .iter()
                .position(|expected| expected == sprite)
                .expect("every sprite must match a clean build");
            remaining.swap_remove(index);
        }
        assert!(remaining.is_empty(), "{} sprites missing", remaining.len());
        assert_pages_hold(actual);
        assert_eq!(actual.area_components, expected.area_components);
        assert_eq!(actual.area_component_tiles, expected.area_component_tiles);
        assert_eq!(actual.sprite_sort_keys.len(), actual.sprites.len());
        assert_eq!(actual.primary_sprites, expected.primary_sprites);
        for owner in actual.placements.keys().chain(expected.placements.keys()) {
            assert_eq!(actual.sprite(*owner), expected.sprite(*owner));
        }

        for (owner, sprite) in &actual.primary_sprites {
            assert_eq!(sprite.owner, *owner);
            assert_eq!(actual.sprite(*owner), Some(sprite));
        }
    }

    #[test]
    fn orders_sprites_by_layer_not_by_prefab_order() {
        let tree = tree(&[("/turf/floor", "floor", 2.0), ("/obj/table", "table", 3.0)]);
        // The map lists the high-layer object first; the frame must still draw it last.
        let document = document(one_tile_map(&["/obj/table", "/turf/floor"]));
        let owners = document.instance_ids_at(dmm::Coord::new(1, 1, 1));

        let instances = build(
            &tree,
            &icons(&["floor", "table"]),
            &textures(&["floor", "table"]),
            &document,
            32,
        );
        let sprites = drawn(&instances);

        assert_eq!(sprites.len(), 2);
        // "floor" is cell 0 and "table" is cell 1 within the same sheet.
        assert_eq!(
            sprites[0].texture,
            textures(&["floor", "table"]).lookup(ICON, 0).unwrap()
        );
        assert_eq!(
            sprites[1].texture,
            textures(&["floor", "table"]).lookup(ICON, 1).unwrap()
        );
        assert_eq!([sprites[0].owner, sprites[1].owner], [owners[1], owners[0]]);
    }

    #[test]
    fn fractional_decal_layers_survive_edits_and_undo() {
        let tree = tree(&[
            ("/obj/decal", "decal", 2.001),
            ("/obj/fine_decal", "fine", 2.0001),
            ("/obj/equal", "equal", 2.0),
            ("/turf/floor", "floor", 2.0),
        ]);
        let icons = icons(&["floor", "decal", "fine", "equal"]);
        let textures = textures(&["floor", "decal", "fine", "equal"]);
        let mut document = document(one_tile_map(&[
            "/obj/decal",
            "/obj/fine_decal",
            "/obj/equal",
            "/turf/floor",
        ]));
        let owners = document.instance_ids_at(Coord::new(1, 1, 1)).to_vec();
        for owner in &owners {
            document
                .set_instance_var(*owner, "plane".into(), Value::Num(-13.0))
                .unwrap();
        }
        let mut instances = build(&tree, &icons, &textures, &document, 32);
        let before = drawn(&instances);
        assert_eq!(
            before.iter().map(|sprite| sprite.owner).collect::<Vec<_>>(),
            [owners[2], owners[3], owners[1], owners[0]]
        );

        document
            .set_instance_var(owners[0], "layer".into(), Value::Num(1.999))
            .unwrap();
        update_prefab(&mut instances, &tree, &icons, &textures, &document, owners[0], 32);
        assert_eq!(drawn(&instances)[0].owner, owners[0]);
        let rebuilt = build(&tree, &icons, &textures, &document, 32);
        assert_eq!(drawn(&instances), drawn(&rebuilt));
        assert_render_data_matches(&instances, &rebuilt);

        document.undo_with_affected().expect("undo decal layer edit");
        update_prefab(&mut instances, &tree, &icons, &textures, &document, owners[0], 32);
        assert_eq!(drawn(&instances), before);
        let rebuilt = build(&tree, &icons, &textures, &document, 32);
        assert_eq!(drawn(&instances), drawn(&rebuilt));
        assert_render_data_matches(&instances, &rebuilt);
    }

    #[test]
    fn fractional_overlay_layers_sort_across_placements() {
        let tree = tree(&[("/obj/decal", "decal", 2.0001), ("/turf/floor", "floor", 2.0)]);
        let mut document = document(one_tile_map(&["/obj/decal", "/turf/floor"]));
        let owners = document.instance_ids_at(Coord::new(1, 1, 1)).to_vec();
        for owner in &owners {
            document
                .set_instance_var(*owner, "plane".into(), Value::Num(-13.0))
                .unwrap();
        }
        let appearances = shared(HashMap::from([(
            owners[1].get(),
            vm::AppearanceDelta {
                overlays: vec![vm::AppearanceDelta {
                    vars: vec![
                        ("icon_state".into(), Value::Text("overlay".into())),
                        ("layer".into(), Value::Num(2.001)),
                    ],
                    ..Default::default()
                }],
                ..Default::default()
            },
        )]));
        let instances = build_with_options(
            &tree,
            &icons(&["floor", "decal", "overlay"]),
            &textures(&["floor", "decal", "overlay"]),
            &document,
            FrameRenderOptions {
                visibility: &TypeVisibility::default(),
                tile_size: 32,
                appearances: &appearances,
                lighting: None,
            },
        );
        assert_eq!(
            drawn(&instances).iter().map(|sprite| sprite.owner).collect::<Vec<_>>(),
            [owners[1], owners[0], owners[1]]
        );
    }

    #[test]
    fn an_overlay_plane_sorts_across_unrelated_placements() {
        let tree = tree(&[("/turf/wall", "floor", 2.058), ("/obj/legacy", "table", 3.0)]);
        let mut document = document(one_tile_map(&["/turf/wall", "/obj/legacy"]));
        let owners = document.instance_ids_at(Coord::new(1, 1, 1)).to_vec();
        for owner in &owners {
            document
                .set_instance_var(*owner, "plane".into(), Value::Num(-10.0))
                .unwrap();
        }
        let frill = vm::AppearanceDelta {
            vars: vec![
                ("icon_state".into(), Value::Text("light".into())),
                ("plane".into(), Value::Num(-7.0)),
                ("layer".into(), Value::Num(2.67)),
            ],
            ..Default::default()
        };
        let appearances = shared(HashMap::from([(
            owners[0].get(),
            vm::AppearanceDelta {
                overlays: vec![frill],
                ..Default::default()
            },
        )]));
        let visibility = TypeVisibility::default();

        let instances = build_with_options(
            &tree,
            &icons(&["floor", "table", "light"]),
            &textures(&["floor", "table", "light"]),
            &document,
            FrameRenderOptions {
                visibility: &visibility,
                tile_size: 32,
                appearances: &appearances,
                lighting: None,
            },
        );
        let sprites = drawn(&instances);

        assert_eq!(
            sprites.iter().map(|sprite| sprite.owner).collect::<Vec<_>>(),
            [owners[0], owners[1], owners[0]]
        );
        assert!(sprites[2].depth > sprites[1].depth);
        assert_eq!(
            instances.sprite(owners[0]).unwrap().texture,
            textures(&["floor"]).lookup(ICON, 0).unwrap()
        );
    }

    #[test]
    fn keep_together_keeps_explicit_plane_overlays_with_their_owner() {
        let tree = tree(&[("/turf/wall", "floor", 2.058), ("/obj/legacy", "table", 3.0)]);
        let mut document = document(one_tile_map(&["/turf/wall", "/obj/legacy"]));
        let owners = document.instance_ids_at(Coord::new(1, 1, 1)).to_vec();
        for owner in &owners {
            document
                .set_instance_var(*owner, "plane".into(), Value::Num(-10.0))
                .unwrap();
        }
        let appearances = shared(HashMap::from([(
            owners[0].get(),
            vm::AppearanceDelta {
                vars: vec![("appearance_flags".into(), Value::Num(32.0))],
                overlays: vec![vm::AppearanceDelta {
                    vars: vec![
                        ("icon_state".into(), Value::Text("light".into())),
                        ("plane".into(), Value::Num(-7.0)),
                    ],
                    ..Default::default()
                }],
                ..Default::default()
            },
        )]));
        let visibility = TypeVisibility::default();

        let instances = build_with_options(
            &tree,
            &icons(&["floor", "table", "light"]),
            &textures(&["floor", "table", "light"]),
            &document,
            FrameRenderOptions {
                visibility: &visibility,
                tile_size: 32,
                appearances: &appearances,
                lighting: None,
            },
        );
        let sprites = drawn(&instances);

        assert_eq!(
            sprites.iter().map(|sprite| sprite.owner).collect::<Vec<_>>(),
            [owners[0], owners[0], owners[1]]
        );
    }

    #[test]
    fn keep_apart_restores_global_plane_order_inside_keep_together() {
        let tree = tree(&[("/turf/wall", "floor", 2.058), ("/obj/legacy", "table", 3.0)]);
        let mut document = document(one_tile_map(&["/turf/wall", "/obj/legacy"]));
        let owners = document.instance_ids_at(Coord::new(1, 1, 1)).to_vec();
        for owner in &owners {
            document
                .set_instance_var(*owner, "plane".into(), Value::Num(-10.0))
                .unwrap();
        }
        let appearances = shared(HashMap::from([(
            owners[0].get(),
            vm::AppearanceDelta {
                vars: vec![("appearance_flags".into(), Value::Num(32.0))],
                overlays: vec![vm::AppearanceDelta {
                    vars: vec![
                        ("icon_state".into(), Value::Text("light".into())),
                        ("plane".into(), Value::Num(-7.0)),
                        ("appearance_flags".into(), Value::Num(64.0)),
                    ],
                    ..Default::default()
                }],
                ..Default::default()
            },
        )]));
        let visibility = TypeVisibility::default();

        let instances = build_with_options(
            &tree,
            &icons(&["floor", "table", "light"]),
            &textures(&["floor", "table", "light"]),
            &document,
            FrameRenderOptions {
                visibility: &visibility,
                tile_size: 32,
                appearances: &appearances,
                lighting: None,
            },
        );
        let sprites = drawn(&instances);

        assert_eq!(
            sprites.iter().map(|sprite| sprite.owner).collect::<Vec<_>>(),
            [owners[0], owners[1], owners[0]]
        );
    }

    #[test]
    fn visible_prefabs_with_failed_icon_lookups_use_the_missing_texture() {
        let mut tree = tree(&[
            ("/obj/valid", "valid", 2.0),
            ("/obj/machinery/computer", "missing", 2.0),
            ("/obj/no_file", "valid", 2.0),
            ("/obj/iconless", "missing", 2.0),
            ("/obj/transparent", "missing", 2.0),
        ]);
        tree.register(&TreePath::parse("/obj/machinery/computer/monitor"), Location::default());
        let mut document = document(one_tile_map(&[
            "/obj/valid",
            "/obj/machinery/computer/monitor",
            "/obj/no_file",
            "/obj/iconless",
            "/obj/transparent",
        ]));
        let owners = document.instance_ids_at(Coord::new(1, 1, 1)).to_vec();
        document
            .set_instance_var(owners[1], "color".into(), Value::Text(String::from("#00ff00")))
            .unwrap();
        document
            .set_instance_var(owners[2], "icon".into(), Value::Resource(String::from("missing.dmi")))
            .unwrap();
        document
            .set_instance_var(owners[3], "icon".into(), Value::Null)
            .unwrap();
        document
            .set_instance_var(owners[4], "alpha".into(), Value::Num(0.0))
            .unwrap();

        let icons = icons(&["valid"]);
        let mut textures = textures(&["valid"]);
        let missing = textures.insert_missing_icon().unwrap();
        let instances = build(&tree, &icons, &textures, &document, 32);

        assert_eq!(
            instances.sprite(owners[0]).unwrap().texture,
            textures.lookup(ICON, 0).unwrap()
        );
        assert_eq!(instances.sprite(owners[1]).unwrap().texture, missing);
        assert_eq!(instances.sprite(owners[1]).unwrap().color, [1.0; 4]);
        assert_eq!(instances.sprite(owners[2]).unwrap().texture, missing);
        assert!(instances.sprite(owners[3]).is_none());
        assert!(instances.sprite(owners[4]).is_none());

        let mut visibility = TypeVisibility::default();
        let monitor = tree.id_of(&TreePath::parse("/obj/machinery/computer/monitor")).unwrap();
        visibility.set_subtree(&tree, monitor, false);
        let hidden = build_with_options(
            &tree,
            &icons,
            &textures,
            &document,
            FrameRenderOptions {
                visibility: &visibility,
                tile_size: 32,
                appearances: &HashMap::new(),
                lighting: None,
            },
        );
        assert!(hidden.sprite(owners[1]).is_none());
    }

    #[test]
    fn failed_overlays_do_not_cover_a_valid_airlock_or_overlay_only_object() {
        let tree = tree(&[
            ("/obj/machinery/door/airlock/grunge", "closed", 2.0),
            ("/obj/overlay_only", "missing", 2.0),
            ("/obj/truly_missing", "missing", 2.0),
        ]);
        let document = document(one_tile_map(&[
            "/obj/machinery/door/airlock/grunge",
            "/obj/overlay_only",
            "/obj/truly_missing",
        ]));
        let owners = document.instance_ids_at(Coord::new(1, 1, 1)).to_vec();
        let overlay = |state: &str| vm::AppearanceDelta {
            vars: vec![("icon_state".into(), Value::Text(state.into()))],
            ..Default::default()
        };
        let appearances = shared(HashMap::from([
            (
                owners[0].get(),
                vm::AppearanceDelta {
                    overlays: vec![overlay("missing")],
                    ..Default::default()
                },
            ),
            (
                owners[1].get(),
                vm::AppearanceDelta {
                    overlays: vec![overlay("closed")],
                    ..Default::default()
                },
            ),
        ]));
        let icons = icons(&["closed"]);
        let mut textures = textures(&["closed"]);
        let missing = textures.insert_missing_icon().unwrap();
        let rendered = build_with_options(
            &tree,
            &icons,
            &textures,
            &document,
            FrameRenderOptions {
                visibility: &TypeVisibility::default(),
                tile_size: 32,
                appearances: &appearances,
                lighting: None,
            },
        );
        let real = textures.lookup(ICON, 0).unwrap();

        assert_eq!(rendered.sprite(owners[0]).unwrap().texture, real);
        assert_eq!(rendered.sprite(owners[1]).unwrap().texture, real);
        assert_eq!(rendered.sprite(owners[2]).unwrap().texture, missing);
        assert_eq!(
            rendered
                .sprites
                .iter()
                .filter(|sprite| sprite.owner == owners[0])
                .count(),
            1
        );
        assert_eq!(
            rendered
                .sprites
                .iter()
                .filter(|sprite| sprite.owner == owners[1])
                .count(),
            1
        );
    }

    #[test]
    fn sprite_textures_use_the_resolved_direction_and_first_frame() {
        let metadata = Metadata {
            version: String::from("4.0"),
            width: 32,
            height: 32,
            states: vec![IconState {
                name: String::from("animated"),
                dirs: 4,
                frames: 2,
                ..Default::default()
            }],
        };
        let file = IconFile {
            path: PathBuf::from(ICON),
            metadata: metadata.clone(),
            sheet_width: 32 * 8,
            sheet_height: 32,
            pixels: vec![255; 32 * 8 * 32 * 4],
        };
        let icons = HashMap::from([(String::from(ICON), metadata)]);
        let mut textures = TextureCatalog::new();
        textures.insert(ICON, &file).expect("insert directional icon");
        let appearance = Appearance {
            icon: Some(String::from(ICON)),
            icon_state: Some(String::from("animated")),
            dir: Dir::East.to_bits(),
            ..Default::default()
        };

        let texture = super::sprite_texture(&icons, &textures, &appearance).expect("east thumbnail");

        assert_eq!(texture.source_position, [64, 0]);
        assert_eq!(texture, textures.lookup(ICON, 2).unwrap());
    }

    #[test]
    fn prefab_updates_touch_only_the_changed_sprite_and_preserve_sorting() {
        let tree = tree(&[("/turf/floor", "floor", 2.0), ("/obj/table", "table", 3.0)]);
        let icons = icons(&["floor", "table"]);
        let textures = textures(&["floor", "table"]);
        let mut document = document(one_tile_map(&["/turf/floor", "/obj/table"]));
        let owner = document.instance_ids_at(dmm::Coord::new(1, 1, 1))[1];
        let mut instances = build(&tree, &icons, &textures, &document, 32);
        let before = drawn(&instances);
        // the map's one level fits in one page, which is what every change uploads
        let page = PrefabUpdate::Buffers {
            sprites: vec![UpdateRange {
                start: 0,
                end: instances.sprites.len(),
            }],
        };

        document
            .set_instance_var(owner, "pixel_x".into(), Value::Num(7.0))
            .unwrap();
        assert_eq!(
            update_prefab(&mut instances, &tree, &icons, &textures, &document, owner, 32),
            page
        );
        assert_eq!(drawn(&instances)[0], before[0]);
        assert_eq!(drawn(&instances)[1].x, before[1].x + 7.0);

        document
            .set_instance_var(owner, "name".into(), Value::Text("renamed".into()))
            .unwrap();
        assert_eq!(
            update_prefab(&mut instances, &tree, &icons, &textures, &document, owner, 32),
            PrefabUpdate::Unchanged,
        );

        document
            .set_instance_var(owner, "layer".into(), Value::Num(1.0))
            .unwrap();
        assert_eq!(
            update_prefab(&mut instances, &tree, &icons, &textures, &document, owner, 32),
            page
        );
        assert_eq!(drawn(&instances)[0].owner, owner);

        document
            .set_instance_var(owner, "icon".into(), Value::Resource("missing.dmi".into()))
            .unwrap();
        assert_eq!(
            update_prefab(&mut instances, &tree, &icons, &textures, &document, owner, 32),
            page
        );
        assert_eq!(drawn(&instances).len(), 1);
    }

    #[test]
    fn area_identity_edits_repair_components_and_cached_buffers() {
        let tree = tree(&[("/area/station", "floor", 1.0)]);
        let icons = icons(&["floor"]);
        let textures = textures(&["floor"]);
        let mut document = document(area_map(5, 5, "/area/station"));
        let owner = document.instance_ids_at(dmm::Coord::new(3, 3, 1))[0];
        let mut instances = build(&tree, &icons, &textures, &document, 32);

        document
            .set_instance_var(owner, "name".into(), Value::Text(String::from("Engineering")))
            .unwrap();
        let update = update_prefab(&mut instances, &tree, &icons, &textures, &document, owner, 32);
        assert!(matches!(update, PrefabUpdate::Buffers { ref sprites, .. } if !sprites.is_empty()));
        assert_render_data_matches(&instances, &build(&tree, &icons, &textures, &document, 32));

        document
            .set_instance_var(owner, "name".into(), Value::Text(String::from("Bridge")))
            .unwrap();
        assert_eq!(
            update_prefab(&mut instances, &tree, &icons, &textures, &document, owner, 32),
            PrefabUpdate::Unchanged,
        );
        assert_render_data_matches(&instances, &build(&tree, &icons, &textures, &document, 32));

        document
            .edit_instance_vars(owner, "reset name", &[VarMutation::Remove("name".into())], None)
            .unwrap();
        let update = update_prefab(&mut instances, &tree, &icons, &textures, &document, owner, 32);
        assert!(matches!(update, PrefabUpdate::Buffers { ref sprites, .. } if !sprites.is_empty()));
        assert_render_data_matches(&instances, &build(&tree, &icons, &textures, &document, 32));
    }

    #[test]
    fn area_icon_edits_patch_the_sprite_buffer() {
        let tree = tree(&[("/area/station", "floor", 1.0)]);
        let icons = icons(&["floor", "alert"]);
        let textures = textures(&["floor", "alert"]);
        let mut document = document(one_tile_map(&["/area/station"]));
        let owner = document.instance_ids_at(dmm::Coord::new(1, 1, 1))[0];
        let mut instances = build(&tree, &icons, &textures, &document, 32);

        document
            .set_instance_var(owner, "icon_state".into(), Value::Text(String::from("alert")))
            .unwrap();
        let update = update_prefab(&mut instances, &tree, &icons, &textures, &document, owner, 32);
        assert!(matches!(
            update,
            PrefabUpdate::Buffers { ref sprites } if !sprites.is_empty()
        ));
        assert_render_data_matches(&instances, &build(&tree, &icons, &textures, &document, 32));
    }

    #[test]
    fn moving_an_area_repairs_components_and_cached_buffers() {
        let tree = tree(&[("/area/station", "floor", 1.0)]);
        let icons = icons(&["floor"]);
        let textures = textures(&["floor"]);
        let mut map = Map::new(Size { x: 3, y: 1, z: 1 });
        map.intern_tile(Vec::new());
        let area = map.intern_tile(vec![Prefab::new(TreePath::parse("/area/station"))]);
        map.grid[0][0][0] = area;
        let mut document = document(map);
        let owner = document.instance_ids_at(dmm::Coord::new(1, 1, 1))[0];
        let mut instances = build(&tree, &icons, &textures, &document, 32);

        document
            .move_instance(owner, dmm::Coord::new(2, 1, 1), "move area", &[], None)
            .unwrap();
        let update = update_prefab(&mut instances, &tree, &icons, &textures, &document, owner, 32);
        assert!(matches!(
            update,
            PrefabUpdate::Buffers { ref sprites } if !sprites.is_empty()
        ));
        assert_render_data_matches(&instances, &build(&tree, &icons, &textures, &document, 32));

        document
            .set_instance_var(owner, "name".into(), Value::Text(String::from("Moved")))
            .unwrap();
        let update = update_prefab(&mut instances, &tree, &icons, &textures, &document, owner, 32);
        assert!(!matches!(update, PrefabUpdate::Rebuild));
        assert_render_data_matches(&instances, &build(&tree, &icons, &textures, &document, 32));
    }

    #[test]
    fn changing_between_area_and_object_repairs_cached_buffers() {
        let tree = tree(&[("/area/station", "floor", 1.0), ("/obj/table", "table", 2.0)]);
        let icons = icons(&["floor", "table"]);
        let textures = textures(&["floor", "table"]);
        let mut document = document(one_tile_map(&["/area/station"]));
        let owner = document.instance_ids_at(dmm::Coord::new(1, 1, 1))[0];
        let mut instances = build(&tree, &icons, &textures, &document, 32);

        document
            .replace_instance_path(owner, "make object", TreePath::parse("/obj/table"), &[], None)
            .unwrap();
        let update = update_prefab(&mut instances, &tree, &icons, &textures, &document, owner, 32);
        assert!(matches!(
            update,
            PrefabUpdate::Buffers { ref sprites } if !sprites.is_empty()
        ));
        assert!(instances.live_sprites().all(|sprite| !sprite.is_area));
        assert_render_data_matches(&instances, &build(&tree, &icons, &textures, &document, 32));

        document
            .replace_instance_path(owner, "make area", TreePath::parse("/area/station"), &[], None)
            .unwrap();
        let update = update_prefab(&mut instances, &tree, &icons, &textures, &document, owner, 32);
        assert!(!matches!(update, PrefabUpdate::Rebuild));
        assert_render_data_matches(&instances, &build(&tree, &icons, &textures, &document, 32));
    }

    #[test]
    fn includes_normal_area_sprites_and_separate_outlines() {
        let tree = tree(&[("/turf/floor", "floor", 2.0), ("/area/station", "floor", 1.0)]);
        let mut document = document(one_tile_map(&["/turf/floor", "/area/station"]));
        let turf_owner = document.instance_ids_at(dmm::Coord::new(1, 1, 1))[0];
        let area_owner = document.instance_ids_at(dmm::Coord::new(1, 1, 1))[1];
        let (icons, textures) = (icons(&["floor"]), textures(&["floor"]));

        let mut instances = build(&tree, &icons, &textures, &document, 32);
        let sprites = drawn(&instances);

        assert_eq!(sprites.len(), 3);
        assert_eq!(sprites.iter().filter(|sprite| sprite.is_area).count(), 2);
        assert_eq!(sprites.iter().filter(|sprite| !sprite.is_area).count(), 1);
        let area = sprites
            .iter()
            .find(|sprite| sprite.is_area && sprite.area_edges == 0)
            .unwrap();
        let outline = sprites.iter().find(|sprite| sprite.area_edges != 0).unwrap();
        assert_eq!(area.texture, textures.lookup(ICON, 0).unwrap());
        assert_eq!(outline.area_edges, AREA_EDGES_ALL);
        assert_eq!((outline.width, outline.height), (32.0, 32.0));
        assert_eq!((sprites[0].is_area, sprites[0].area_edges), (false, 0));
        assert_eq!((sprites[1].is_area, sprites[1].area_edges), (true, 0));
        assert_eq!((sprites[2].is_area, sprites[2].area_edges), (true, AREA_EDGES_ALL));
        assert_eq!(instances.sprite(area_owner).unwrap().area_edges, 0);

        document
            .set_instance_var(turf_owner, "layer".into(), Value::Num(0.0))
            .unwrap();
        update_prefab(&mut instances, &tree, &icons, &textures, &document, turf_owner, 32);
        assert_eq!(instances.sprite(area_owner).unwrap().area_edges, 0);
    }

    #[test]
    fn matching_area_tiles_only_draw_the_outer_perimeter() {
        let tree = tree(&[("/area/station", "floor", 1.0)]);
        let (icons, textures) = (icons(&["floor"]), textures(&["floor"]));

        let instances = build(&tree, &icons, &textures, &document(area_map(3, 3, "/area/station")), 48);
        let sprites = drawn(&instances);

        assert_eq!(sprites.len(), 9 + 8);
        assert_eq!(
            sprites
                .iter()
                .filter(|sprite| sprite.is_area && sprite.area_edges == 0)
                .count(),
            9
        );
        assert_eq!(sprites.iter().filter(|sprite| sprite.area_edges != 0).count(), 8);
        assert!(
            sprites
                .iter()
                .any(|sprite| sprite.area_edges == 0 && (sprite.x, sprite.y) == (48.0, 48.0))
        );
        assert!(
            !sprites
                .iter()
                .any(|sprite| sprite.area_edges != 0 && (sprite.x, sprite.y) == (48.0, 48.0))
        );

        let bottom_left = sprites
            .iter()
            .find(|sprite| sprite.area_edges != 0 && (sprite.x, sprite.y) == (0.0, 0.0))
            .unwrap();
        assert_eq!(bottom_left.area_edges, AREA_EDGE_SOUTH | AREA_EDGE_WEST);
        assert_eq!((bottom_left.width, bottom_left.height), (48.0, 48.0));

        let top_middle = sprites
            .iter()
            .find(|sprite| sprite.area_edges != 0 && (sprite.x, sprite.y) == (48.0, 96.0))
            .unwrap();
        assert_eq!(top_middle.area_edges, AREA_EDGE_NORTH);
    }

    #[test]
    fn touching_different_areas_keep_both_sides_of_the_shared_edge() {
        let tree = tree(&[("/area/one", "floor", 1.0), ("/area/two", "floor", 1.0)]);
        let mut map = Map::new(Size { x: 2, y: 1, z: 1 });
        let left = map.intern_tile(vec![Prefab::new(TreePath::parse("/area/one"))]);
        let right = map.intern_tile(vec![Prefab::new(TreePath::parse("/area/two"))]);
        map.grid[0][0] = vec![left, right];
        let (icons, textures) = (icons(&["floor"]), textures(&["floor"]));

        let instances = build(&tree, &icons, &textures, &document(map), 32);
        let sprites = drawn(&instances);
        let left = sprites
            .iter()
            .find(|sprite| sprite.area_edges != 0 && sprite.x == 0.0)
            .unwrap();
        let right = sprites
            .iter()
            .find(|sprite| sprite.area_edges != 0 && sprite.x == 32.0)
            .unwrap();

        assert_ne!(left.area_edges & AREA_EDGE_EAST, 0);
        assert_ne!(right.area_edges & AREA_EDGE_WEST, 0);
    }

    #[test]
    fn matching_neighbors_remove_both_sides_of_the_shared_edge() {
        let tree = tree(&[("/area/station", "floor", 1.0)]);
        let (icons, textures) = (icons(&["floor"]), textures(&["floor"]));

        let instances = build(&tree, &icons, &textures, &document(area_map(2, 1, "/area/station")), 32);
        let sprites = drawn(&instances);
        let left = sprites
            .iter()
            .find(|sprite| sprite.area_edges != 0 && sprite.x == 0.0)
            .unwrap();
        let right = sprites
            .iter()
            .find(|sprite| sprite.area_edges != 0 && sprite.x == 32.0)
            .unwrap();

        assert_eq!(left.area_edges, AREA_EDGE_NORTH | AREA_EDGE_SOUTH | AREA_EDGE_WEST);
        assert_eq!(right.area_edges, AREA_EDGE_NORTH | AREA_EDGE_EAST | AREA_EDGE_SOUTH);
    }

    #[test]
    fn holes_receive_an_inner_perimeter() {
        let tree = tree(&[("/area/station", "floor", 1.0)]);
        let mut map = Map::new(Size { x: 3, y: 3, z: 1 });
        let empty = map.intern_tile(Vec::new());
        let area = map.intern_tile(vec![Prefab::new(TreePath::parse("/area/station"))]);
        for row in &mut map.grid[0] {
            row.fill(area);
        }
        map.grid[0][1][1] = empty;
        let (icons, textures) = (icons(&["floor"]), textures(&["floor"]));

        let instances = build(&tree, &icons, &textures, &document(map), 32);
        let sprites = drawn(&instances);
        let west_of_hole = sprites
            .iter()
            .find(|sprite| sprite.area_edges != 0 && (sprite.x, sprite.y) == (0.0, 32.0))
            .unwrap();
        let east_of_hole = sprites
            .iter()
            .find(|sprite| sprite.area_edges != 0 && (sprite.x, sprite.y) == (64.0, 32.0))
            .unwrap();
        let south_of_hole = sprites
            .iter()
            .find(|sprite| sprite.area_edges != 0 && (sprite.x, sprite.y) == (32.0, 0.0))
            .unwrap();
        let north_of_hole = sprites
            .iter()
            .find(|sprite| sprite.area_edges != 0 && (sprite.x, sprite.y) == (32.0, 64.0))
            .unwrap();

        assert_ne!(west_of_hole.area_edges & AREA_EDGE_EAST, 0);
        assert_ne!(east_of_hole.area_edges & AREA_EDGE_WEST, 0);
        assert_ne!(south_of_hole.area_edges & AREA_EDGE_NORTH, 0);
        assert_ne!(north_of_hole.area_edges & AREA_EDGE_SOUTH, 0);
    }

    #[test]
    fn area_identity_uses_values_instead_of_source_order() {
        let mut left = Prefab::new(TreePath::parse("/area/station"));
        left.set_var("name".into(), Value::Text(String::from("Bridge")));
        left.set_var("icon_state".into(), Value::Text(String::from("bridge")));
        let mut right = Prefab::new(TreePath::parse("/area/station"));
        right.set_var("icon_state".into(), Value::Text(String::from("bridge")));
        right.set_var("name".into(), Value::Text(String::from("Bridge")));

        assert!(super::same_area(&left, &right));
        right.set_var("name".into(), Value::Text(String::from("Engineering")));
        assert!(!super::same_area(&left, &right));
    }

    #[test]
    fn area_components_join_cardinal_tiles_but_split_disconnected_regions_and_levels() {
        let tree = tree(&[("/area/station", "floor", 1.0), ("/area/other", "floor", 1.0)]);
        let mut map = Map::new(Size { x: 4, y: 1, z: 2 });
        let station = map.intern_tile(vec![Prefab::new(TreePath::parse("/area/station"))]);
        let other = map.intern_tile(vec![Prefab::new(TreePath::parse("/area/other"))]);
        map.grid[0][0] = vec![station, station, other, station];
        map.grid[1][0] = vec![station, station, other, station];
        let document = document(map);

        let instances = build(&tree, &icons(&["floor"]), &textures(&["floor"]), &document, 32);
        let first = instances.area_component_at(Coord::new(1, 1, 1)).unwrap();

        assert_eq!(instances.area_component_at(Coord::new(2, 1, 1)), Some(first));
        assert_ne!(instances.area_component_at(Coord::new(3, 1, 1)), Some(first));
        assert_ne!(instances.area_component_at(Coord::new(4, 1, 1)), Some(first));
        assert_ne!(instances.area_component_at(Coord::new(1, 1, 2)), Some(first));
    }

    #[test]
    fn deleting_an_area_splits_components_and_patches_cached_ranges() {
        let tree = tree(&[("/area/station", "floor", 1.0)]);
        let icons = icons(&["floor"]);
        let textures = textures(&["floor"]);
        let mut document = document(area_map(3, 1, "/area/station"));
        let middle = Coord::new(2, 1, 1);
        let owner = document.instance_ids_at(middle)[0];
        let mut instances = build(&tree, &icons, &textures, &document, 32);
        let mut after = document.placed_tile(middle).unwrap();
        after.clear();
        let mut edit = Edit::new("delete middle area");
        edit.change(&document, middle, after);
        document.apply(edit);

        let update = update_prefab(&mut instances, &tree, &icons, &textures, &document, owner, 32);

        assert!(matches!(
            update,
            PrefabUpdate::Buffers { ref sprites } if !sprites.is_empty()
        ));
        assert_ne!(
            instances.area_component_at(Coord::new(1, 1, 1)),
            instances.area_component_at(Coord::new(3, 1, 1))
        );
        assert_render_data_matches(&instances, &build(&tree, &icons, &textures, &document, 32));
    }

    #[test]
    fn batched_area_deletions_produce_one_valid_buffer_update() {
        let tree = tree(&[("/area/station", "floor", 1.0)]);
        let icons = icons(&["floor"]);
        let textures = textures(&["floor"]);
        let mut document = document(area_map(4, 1, "/area/station"));
        let coords = [Coord::new(2, 1, 1), Coord::new(3, 1, 1)];
        let owners = coords.map(|coord| document.instance_ids_at(coord)[0]);
        let mut instances = build(&tree, &icons, &textures, &document, 32);
        let mut edit = Edit::new("delete two areas");
        for coord in coords {
            edit.change(&document, coord, Vec::new());
        }
        document.apply(edit);

        let update = update_prefabs(&mut instances, &tree, &icons, &textures, &document, &owners, 32);

        let PrefabUpdate::Buffers { sprites } = update else {
            panic!("batched area deletion must stay incremental");
        };
        assert!(!sprites.is_empty());
        assert!(
            sprites
                .iter()
                .all(|range| range.start <= range.end && range.end <= instances.sprites.len())
        );
        assert_render_data_matches(&instances, &build(&tree, &icons, &textures, &document, 32));
    }

    #[test]
    fn sprites_inherit_the_connected_area_at_their_anchor_tile() {
        let tree = tree(&[("/obj/table", "table", 2.0), ("/area/station", "floor", 1.0)]);
        let map = one_tile_map(&["/obj/table", "/area/station"]);
        let document = document(map);
        let coord = Coord::new(1, 1, 1);
        let object_owner = document.instance_ids_at(coord)[0];
        let area_owner = document.instance_ids_at(coord)[1];

        let instances = build(
            &tree,
            &icons(&["floor", "table"]),
            &textures(&["floor", "table"]),
            &document,
            32,
        );
        let component = instances.area_component_at(coord);

        assert_eq!(instances.sprite(object_owner).unwrap().area_owner, component);
        assert!(
            instances
                .sprites
                .iter()
                .filter(|sprite| sprite.owner == area_owner)
                .all(|sprite| sprite.area_owner == component)
        );
    }

    #[test]
    fn an_area_with_an_unresolvable_icon_still_gets_a_white_outline() {
        let tree = tree(&[("/area/station", "missing", 1.0)]);
        let document = document(one_tile_map(&["/area/station"]));

        let instances = build(&tree, &icons(&["floor"]), &textures(&["floor"]), &document, 32);
        let sprites = drawn(&instances);

        assert_eq!(sprites.len(), 1);
        assert!(sprites[0].is_area);
        assert_eq!(sprites[0].area_edges, AREA_EDGES_ALL);
        assert_eq!(sprites[0].texture, Default::default());
        assert_eq!(sprites[0].color, [1.0; 4]);
        assert_eq!(instances.sprite(sprites[0].owner), Some(&sprites[0]));
    }

    #[test]
    fn an_unresolvable_icon_state_without_a_registered_fallback_emits_nothing() {
        let tree = tree(&[("/obj/ghost", "not_in_the_sheet", 2.0)]);
        let document = document(one_tile_map(&["/obj/ghost"]));

        let instances = build(&tree, &icons(&["floor"]), &textures(&["floor"]), &document, 32);
        let sprites = drawn(&instances);

        assert!(sprites.is_empty());
        assert_eq!(document.instance_ids_at(dmm::Coord::new(1, 1, 1)).len(), 1);
    }

    #[test]
    fn a_prefab_with_no_type_in_the_tree_emits_nothing() {
        let document = document(one_tile_map(&["/obj/never/declared"]));

        let instances = build(
            &ObjectTree::new(),
            &icons(&["floor"]),
            &textures(&["floor"]),
            &document,
            32,
        );
        let sprites = drawn(&instances);

        assert!(sprites.is_empty());
    }

    /// `/turf/one` on z 1, `/turf/two` on z 2, `/turf/three` on z 3, one cell each.
    fn layered() -> (ObjectTree, HashMap<String, Metadata>, TextureCatalog, Map) {
        let states = ["one", "two", "three"];
        let tree = tree(&[
            ("/turf/one", "one", 2.0),
            ("/turf/two", "two", 2.0),
            ("/turf/three", "three", 2.0),
        ]);

        (
            tree,
            icons(&states),
            textures(&states),
            layered_map(&["/turf/one", "/turf/two", "/turf/three"]),
        )
    }

    #[test]
    fn builds_every_level_deepest_first() {
        let (tree, icons, textures, map) = layered();
        let instances = build(&tree, &icons, &textures, &document(map), 32);
        let sprites = drawn(&instances);

        assert_eq!(sprites.len(), 3);
        assert_eq!(sprites.iter().map(|sprite| sprite.z).collect::<Vec<_>>(), [1, 2, 3]);
        assert_eq!(
            sprites.iter().map(|sprite| sprite.texture).collect::<Vec<_>>(),
            (0..3)
                .map(|cell| textures.lookup(ICON, cell).unwrap())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn tiles_land_on_pixel_positions_from_their_coordinates() {
        let tree = tree(&[("/turf/floor", "floor", 2.0)]);
        let mut map = Map::new(Size { x: 2, y: 2, z: 1 });
        // The grid starts filled with the default key, so claim it for an empty tile first.
        map.intern_tile(Vec::new());
        let key = map.intern_tile(vec![Prefab::new(TreePath::parse("/turf/floor"))]);
        // (2, 1) is the bottom right tile, so grid row 1, column 1.
        if let Some(slot) = map
            .grid
            .get_mut(0)
            .and_then(|z| z.get_mut(1))
            .and_then(|r| r.get_mut(1))
        {
            *slot = key;
        }

        let instances = build(&tree, &icons(&["floor"]), &textures(&["floor"]), &document(map), 32);
        let sprites = drawn(&instances);

        assert_eq!(sprites.len(), 1);
        assert_eq!((sprites[0].x, sprites[0].y), (32.0, 0.0));
    }
}

/// End to end over `examples/env`, which is the only place the whole chain - preprocess, parse,
/// lower, pack, resolve - is exercised without a GPU.
#[cfg(test)]
mod example_environment {
    use std::path::PathBuf;

    use dmi::IconFile;
    use render::texture::TextureCatalog;

    use super::build;
    use crate::{Environment, document::MapDocument};

    fn examples() -> PathBuf {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env");

        path.canonicalize().unwrap_or(path)
    }

    #[test]
    fn the_example_map_resolves_to_sprites() {
        let root = examples();
        let (environment, diagnostics) = Environment::load(root.join("test.dme")).expect("load");

        assert!(diagnostics.icons.is_empty(), "{:?}", diagnostics.icons);
        assert!(environment.icons.contains_key("icons/test.dmi"));
        // `resources` holds paths the preprocessor already resolved against the includer.
        assert_eq!(environment.maps, vec![root.join("test.dmm"), root.join("test2.dmm")]);

        let mut textures = TextureCatalog::default();
        let file = IconFile::load(root.join("icons/test.dmi")).expect("icon");
        textures.insert("icons/test.dmi", &file).expect("insert");

        let source = std::fs::read_to_string(root.join("test.dmm")).expect("map");
        let (map, errors) = dmm::parser::parse(&source);
        assert!(errors.is_empty(), "{errors:?}");

        let document = MapDocument::new(map, 1);
        let instances = build(&environment.tree, &environment.icons, &textures, &document, 32);
        let sprites = instances.live_sprites().copied().collect::<Vec<_>>();

        // 8x6 turfs, three tables, one light, and the 24 tiles around the area's perimeter.
        assert_eq!(sprites.len(), 48 + 4 + 24);

        // Drawable atoms found a real cell; the iconless area still uses tile-sized outline geometry.
        assert!(
            sprites
                .iter()
                .filter(|sprite| !sprite.is_area)
                .all(|sprite| sprite.texture.width == 32 && sprite.texture.height == 32)
        );
        assert!(
            sprites
                .iter()
                .filter(|sprite| sprite.is_area)
                .all(|sprite| (sprite.width, sprite.height) == (32.0, 32.0))
        );
        assert!(sprites.iter().any(|s| s.color[0] > s.color[1]));
    }

    #[test]
    fn turfs_sort_under_objects() {
        let root = examples();
        let (environment, _) = Environment::load(root.join("test.dme")).expect("load");

        let mut textures = TextureCatalog::default();
        let file = IconFile::load(root.join("icons/test.dmi")).expect("icon");
        textures.insert("icons/test.dmi", &file).expect("insert");

        let source = std::fs::read_to_string(root.join("test.dmm")).expect("map");
        let (map, _) = dmm::parser::parse(&source);
        let document = MapDocument::new(map, 1);
        let instances = build(&environment.tree, &environment.icons, &textures, &document, 32);
        let sprites = instances.live_sprites().copied().collect::<Vec<_>>();

        // Neither `/turf` nor `/obj` declares a layer, so this is `demir.dm`'s builtin defaults
        // beating the map's own order, which lists every obj ahead of its turf. Cell 0 is "floor"
        // and cell 1 is "wall", so no turf may appear after the first object.
        let first_object_x = textures
            .lookup("icons/test.dmi", 2)
            .expect("object cell")
            .source_position[0];
        let first_object = sprites
            .iter()
            .position(|sprite| !sprite.is_area && sprite.texture.source_position[0] >= first_object_x)
            .expect("an object sprite");

        assert!(
            sprites[first_object..]
                .iter()
                .filter(|sprite| !sprite.is_area)
                .all(|sprite| sprite.texture.source_position[0] >= first_object_x)
        );
    }
}
