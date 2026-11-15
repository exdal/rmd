use dmm::{Coord, Prefab, Size};
use objtree::{Roots, TypeId};
use render::{Frame, GuideLine, MapViewFrame, MapViewInteraction, MapViewRect, SpritePreview};

use crate::session::Session;
use crate::{
    document::{DocumentId, PrefabInstanceId, Selection},
    focus::AreaFocus,
    frame::{
        self, FrameInstances, PrefabUpdate,
        options::{FrameRenderOptions, HiddenTypes},
    },
    tool::session::context_placement_group,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeLayer {
    Area,
    Turf,
    Obj,
    Mob,
}

impl TypeLayer {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Area => "Areas",
            Self::Turf => "Turfs",
            Self::Obj => "Objects",
            Self::Mob => "Mobs",
        }
    }

    const fn root(self, roots: Roots) -> Option<TypeId> {
        match self {
            Self::Area => roots.area,
            Self::Turf => roots.turf,
            Self::Obj => roots.obj,
            Self::Mob => roots.mob,
        }
    }
}

impl Session {
    pub(crate) fn instances(&self) -> Option<&FrameInstances> {
        self.caches.get(&self.state.active()?).map(|cache| &cache.instances)
    }

    pub fn toggle_areas(&mut self) {
        self.options.show_areas = !self.options.show_areas;
    }

    pub fn toggle_area_outlines(&mut self) {
        self.options.show_area_outlines = !self.options.show_area_outlines;
    }

    pub fn toggle_lighting(&mut self) {
        self.options.show_lighting = !self.options.show_lighting;
    }

    pub fn is_type_visible(&self, id: TypeId) -> bool {
        self.tree().is_some_and(|tree| tree.get(id).is_some()) && self.type_visibility.is_visible(id)
    }

    pub fn is_layer_visible(&self, layer: TypeLayer) -> bool {
        self.layer_root(layer)
            .is_some_and(|root| self.type_visibility.is_visible(root))
    }

    pub fn toggle_layer(&mut self, layer: TypeLayer) -> bool {
        self.layer_root(layer)
            .is_some_and(|root| self.toggle_type_visibility(root))
    }

    fn layer_root(&self, layer: TypeLayer) -> Option<TypeId> {
        layer.root(self.tree()?.roots())
    }

    pub fn hides_types(&self) -> bool {
        self.type_visibility.hides_any()
    }

    pub fn show_all_types(&mut self) -> bool {
        let changed = self.type_visibility.show_all();
        if changed {
            self.apply_type_visibility();
        }

        changed
    }

    pub(crate) fn hidden_types(&self) -> HiddenTypes {
        let Some(environment) = self.state.environment.as_ref() else {
            return HiddenTypes::default();
        };

        let hidden = self.type_visibility.hidden_types(&environment.tree);
        if self.options.show_areas {
            return hidden;
        }

        hidden.with_areas(environment.area_paths())
    }

    pub fn toggle_type_visibility(&mut self, id: TypeId) -> bool {
        let visible = !self.type_visibility.is_visible(id);
        let changed = {
            let Some(environment) = self.state.environment.as_ref() else {
                return false;
            };
            if environment.tree.get(id).is_none() {
                return false;
            }

            self.type_visibility.set_subtree(&environment.tree, id, visible)
        };

        if changed {
            self.apply_type_visibility();
        }

        changed
    }

    fn apply_type_visibility(&mut self) {
        for id in self.state.document_ids() {
            let update = self.caches.get_mut(&id).map_or(PrefabUpdate::Unchanged, |cache| {
                cache.instances.apply_visibility(&self.type_visibility)
            });
            self.publish_frame_update(id, update);
        }

        self.revalidate_focus();
    }

    pub fn set_underlay_depth(&mut self, depth: u32) {
        if depth == self.options.underlay_depth {
            return;
        }

        self.options.underlay_depth = depth;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn map_view_frame<'a>(
        &'a self, id: DocumentId, rect: MapViewRect, camera: render::Camera, interaction: MapViewInteraction,
        guide_lines: &'a [GuideLine], connected: &'a [PrefabInstanceId], picked: &'a [PrefabInstanceId],
    ) -> Option<MapViewFrame<'a>> {
        let document = self.state.document(id)?;
        let cache = self.caches.get(&id)?;

        let is_node_pickable = |owner: &PrefabInstanceId| {
            cache
                .instances
                .sprite(*owner)
                .is_some_and(|sprite| sprite.node_pickable)
        };
        let picked = if interaction.mode.is_node() && !picked.iter().all(is_node_pickable) {
            &picked[..0]
        } else {
            picked
        };

        Some(MapViewFrame {
            rect,
            camera,
            sprite_instances: &cache.instances.sprites,
            focused_area: document.focus().map(AreaFocus::component),
            active_z: document.z,
            level_count: document.map.size().z.max(1),
            revision: cache.revision,
            pending_updates: &cache.frame_updates,
            lighting: (self.options.show_lighting && !cache.instances.light_tiles.is_empty()).then_some(
                render::LightingFrame {
                    size: cache.instances.lighting_size,
                    tiles: &cache.instances.light_tiles,
                    tile_size: self.options.tile_size,
                    minimum_brightness: self.options.minimum_light_brightness_percent.min(100) as f32 / 100.0,
                    revision: cache.lighting_revision,
                    pending_updates: &cache.lighting_updates,
                },
            ),
            guide_lines,
            connected,
            picked,
            interaction,
            preview: cache
                .preview
                .as_ref()
                .filter(|preview| {
                    preview.visible
                        && preview.destination.z == document.z
                        && preview.key.document_revision == cache.revision
                })
                .map(|preview| SpritePreview {
                    sprites: &preview.sprites,
                    revision: preview.revision,
                    offset: [
                        preview.destination.x.saturating_sub(1) as f32 * self.options.tile_size.max(1) as f32,
                        preview.destination.y.saturating_sub(1) as f32 * self.options.tile_size.max(1) as f32,
                    ],
                }),
        })
    }

    pub fn frame<'a>(&self, map_views: &'a [MapViewFrame<'a>], picking: Option<usize>) -> Frame<'a> {
        Frame {
            map_views,
            underlay_depth: self.options.underlay_depth,
            show_areas: self.options.show_areas,
            show_area_outlines: self.options.show_area_outlines,
            picking,
        }
    }

    pub fn texture_revision(&self) -> u64 {
        self.texture_revision
    }

    pub fn extent_px(&self) -> (f32, f32) {
        self.state
            .active()
            .map_or_else(|| self.tile_extent(None), |id| self.extent_px_of(id))
    }

    pub fn capture_region(&self, id: DocumentId, selection: Option<Selection>) -> Option<([u32; 2], [u32; 2])> {
        // bottom left origin

        let size = self.state.document(id)?.map.size();
        let tile = self.options.tile_size.max(1);
        let (min_x, min_y) = selection.map_or((1, 1), |selection| (selection.min.x, selection.min.y));
        let (max_x, max_y) = selection.map_or((size.x, size.y), |selection| {
            (selection.max.x.min(size.x), selection.max.y.min(size.y))
        });
        if min_x < 1 || min_y < 1 || min_x > max_x || min_y > max_y {
            return None;
        }

        Some((
            [(min_x - 1) * tile, (min_y - 1) * tile],
            [(max_x - min_x + 1) * tile, (max_y - min_y + 1) * tile],
        ))
    }

    pub fn extent_px_of(&self, id: DocumentId) -> (f32, f32) {
        self.tile_extent(self.state.document(id).map(|document| document.map.size()))
    }

    fn tile_extent(&self, size: Option<Size>) -> (f32, f32) {
        let tile = self.options.tile_size.max(1) as f32;

        size.map_or((tile, tile), |size| {
            (size.x.max(1) as f32 * tile, size.y.max(1) as f32 * tile)
        })
    }

    pub(crate) fn rebuild_instances(&mut self, id: DocumentId) {
        if let Some(cache) = self.caches.get_mut(&id) {
            drop(std::mem::take(&mut cache.instances));
        }

        let bake = self.caches.get(&id).and_then(|cache| cache.bake.as_ref());
        let node_types = crate::node::session::registered_types(self.state.environment.as_deref(), bake);
        let instances = match (self.state.environment.as_ref(), self.effective_grid(id)) {
            (Some(environment), Some(grid)) => frame::build_view_with_options(
                &environment.tree,
                &environment.icons,
                &self.textures,
                grid,
                FrameRenderOptions {
                    perspective: environment.world.perspective,
                    node_types: Some(&node_types),
                    visibility: &self.type_visibility,
                    tile_size: self.options.tile_size,
                    appearances: crate::bake::appearances(bake),
                    lighting: bake.and_then(|bake| bake.lighting.as_ref()),
                },
            ),
            _ => FrameInstances::default(),
        };

        let revision = self.bump_revision();
        let cache = self.caches.entry(id).or_default();
        cache.instances = instances;
        cache.node_types = node_types;
        cache.revision = revision;
        cache.frame_updates.clear();
        cache.lighting_revision = revision;
        cache.lighting_updates.clear();
        self.revalidate_focus();
    }

    pub(crate) fn bump_revision(&mut self) -> u64 {
        let revision = self.next_revision;
        self.next_revision = self.next_revision.wrapping_add(1).max(1);

        revision
    }

    pub(crate) fn refresh_reordered_from_affected(&mut self, affected: &[PrefabInstanceId]) {
        let coord = self.state.active_document().and_then(|document| {
            affected
                .iter()
                .find_map(|id| document.instance_location(*id).map(|location| location.coord))
        });
        if let Some(coord) = coord {
            self.refresh_reordered_tile(coord);
        }
    }

    pub(crate) fn refresh_reordered_tile(&mut self, coord: Coord) {
        let Some(id) = self.state.active() else {
            return;
        };

        let sources = self
            .state
            .document(id)
            .map(|document| document.instance_ids_at(coord).to_vec())
            .unwrap_or_default();
        if let Some(changed) = self.sync_module_source(id, &sources) {
            self.settle_modules(id, changed);
        }

        let Some(ordered) = self.placement_group_at(id, coord) else {
            return;
        };

        // drawn in their new tile order once the render cache takes their indices again
        self.apply_bake_update(
            id,
            crate::bake::BakeUpdate {
                appearances: ordered,
                lighting: None,
            },
        );
    }

    pub(crate) fn apply_remote_tiles(&mut self, id: DocumentId, tiles: Vec<(Coord, Vec<Prefab>)>) -> bool {
        let Some(document) = self.state.document_mut(id) else {
            return false;
        };

        let affected = document.apply_remote(tiles);
        self.update_document_instances(id, &affected);

        true
    }

    fn placement_group_at(&self, id: DocumentId, coord: Coord) -> Option<Vec<PrefabInstanceId>> {
        let grid = self.effective_grid(id)?;
        let tree = self.tree()?;

        Some(
            grid.ids_at(coord)
                .iter()
                .copied()
                .filter(|owner| {
                    grid.prefab_instance(*owner)
                        .is_some_and(|instance| context_placement_group(tree, &instance.prefab().path) == Some(0))
                })
                .collect(),
        )
    }

    pub(crate) fn update_instance(&mut self, selected: PrefabInstanceId) {
        self.update_instances(&[selected]);
    }

    pub(crate) fn update_instances(&mut self, affected: &[PrefabInstanceId]) {
        if let Some(id) = self.state.active() {
            self.update_document_instances(id, affected);
        }
    }

    pub(crate) fn update_document_instances(&mut self, id: DocumentId, affected: &[PrefabInstanceId]) {
        let changed = self
            .sync_module_source(id, affected)
            .unwrap_or_else(|| affected.to_vec());
        self.bake_edits(id, affected, changed);
    }
}

#[cfg(test)]
mod tests;
