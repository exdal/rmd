use base::{coord::Coord, vars};
use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet},
};

use defines::{EFFECTS_LAYER, FLOAT_LAYER, KEEP_APART, KEEP_TOGETHER};
use objtree::{ObjectTree, TypeId};
use prefab::{Prefab, PrefabInstanceId};
use render::{SpriteInstance, SpriteTexture, texture::TextureCatalog};
use vm::bake::SharedAppearance;

use crate::{environment::world::Perspective, maploader::preview::MapView};

use super::{
    PlacementOrder, RenderedPrefab, TypeVisibility,
    area::{area_edges, area_outline},
    instance_for, sprite_texture, sprite_texture_or_missing,
    visual::{self, Appearance},
};

const FLAT_ROW: Reverse<i32> = Reverse(i32::MIN);

// an offset moves the row only once it carries the sprite's center onto the next tile, mappers nudge
// things a few pixels toward the viewer without changing what covers them
pub(super) fn side_row(coord: Coord, appearance: &Appearance, tile_size: u32) -> Reverse<i32> {
    let tile_size = i64::from(tile_size.max(1));
    let offset = i64::from(appearance.pixel_y) + i64::from(appearance.step_y);
    let tiles = (offset.abs() + tile_size / 2) / tile_size * offset.signum();
    let row = i64::from(coord.y) + tiles;

    Reverse(row.clamp(i64::from(i32::MIN) + 1, i64::from(i32::MAX)) as i32)
}

const AREA_PLANE: f32 = f32::INFINITY;

struct SpriteGroup {
    plane: f32,
    layer: f32,
    // the parent's own icon floats at `FLOAT_LAYER`, so `FLOAT_LAYER - 1` sorts under it
    float: f32,
    keep_apart: bool,
    sprites: Vec<SpriteInstance>,
}

pub(super) struct RenderContext<'a> {
    pub(super) node_types: Option<&'a HashSet<TypeId>>,
    pub(super) appearances: &'a HashMap<u64, SharedAppearance>,
    pub(super) tree: &'a ObjectTree,
    pub(super) textures: &'a TextureCatalog,
    pub(super) map: MapView<'a>,
    pub(super) area: Option<TypeId>,
    pub(super) visibility: &'a TypeVisibility,
    pub(super) tile_size: u32,
    pub(super) perspective: Perspective,
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
        if let Some(texture) = sprite_texture(self.textures, &appearance) {
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

    pub(super) fn prefab(
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
        let texture = sprite_texture(self.textures, &appearance);
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
            && let Some(fallback) = sprite_texture_or_missing(self.textures, &appearance)
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
