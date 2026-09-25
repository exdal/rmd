use dmm::{Coord, Size};
use editor::{
    document::{DocumentId, PrefabInstanceId, Selection},
    focus::AreaFocus,
    frame::{self, FrameInstances, FrameRenderOptions},
};
use objtree::{Roots, TypeId};
use render::{Frame, GuideLine, MapViewFrame, MapViewInteraction, MapViewRect, SpritePreview};

use super::{Session, context_placement_group, report_bake_output};

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
    pub(super) fn instances(&self) -> Option<&FrameInstances> {
        self.caches.get(&self.state.active()?).map(|cache| &cache.instances)
    }

    pub fn toggle_areas(&mut self) { self.options.show_areas = !self.options.show_areas; }

    pub fn toggle_area_outlines(&mut self) { self.options.show_area_outlines = !self.options.show_area_outlines; }

    pub fn toggle_lighting(&mut self) { self.options.show_lighting = !self.options.show_lighting; }

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

    fn layer_root(&self, layer: TypeLayer) -> Option<TypeId> { layer.root(self.tree()?.roots()) }

    pub fn hides_types(&self) -> bool { self.type_visibility.hides_any() }

    pub fn show_all_types(&mut self) -> bool {
        let changed = self.type_visibility.show_all();
        if changed {
            self.rebuild_all_instances();
        }

        changed
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
            self.rebuild_all_instances();
        }

        changed
    }

    pub fn set_underlay_depth(&mut self, depth: u32) {
        if depth == self.options.underlay_depth {
            return;
        }

        self.options.underlay_depth = depth;
    }

    pub fn map_view_frame<'a>(
        &'a self, id: DocumentId, rect: MapViewRect, camera: render::Camera, interaction: MapViewInteraction,
        guide_lines: &'a [GuideLine], connected: &'a [PrefabInstanceId],
    ) -> Option<MapViewFrame<'a>> {
        let document = self.state.document(id)?;
        let cache = self.caches.get(&id)?;

        Some(MapViewFrame {
            rect,
            camera,
            sprite_instances: &cache.instances.sprites,
            area_tiles: &cache.instances.area_tiles,
            focused_area: document.focus().map(AreaFocus::component),
            active_z: document.z,
            level_count: document.map.size.z.max(1),
            revision: cache.revision,
            pending_update: cache.frame_update,
            lighting: (self.options.show_lighting && !cache.instances.light_tiles.is_empty()).then_some(
                render::LightingFrame {
                    size: cache.instances.lighting_size,
                    tiles: &cache.instances.light_tiles,
                    tile_size: self.options.tile_size,
                    minimum_brightness: self.options.minimum_light_brightness_percent.min(100) as f32 / 100.0,
                    revision: cache.lighting_revision,
                    pending_update: cache.lighting_update,
                },
            ),
            guide_lines,
            connected,
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

    pub fn texture_revision(&self) -> u64 { self.texture_revision }

    pub fn extent_px(&self) -> (f32, f32) {
        self.state
            .active()
            .map_or_else(|| self.tile_extent(None), |id| self.extent_px_of(id))
    }

    pub fn capture_region(&self, id: DocumentId, selection: Option<Selection>) -> Option<([u32; 2], [u32; 2])> {
        // bottom left origin

        let size = self.state.document(id)?.map.size;
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
        self.tile_extent(self.state.document(id).map(|document| document.map.size))
    }

    fn tile_extent(&self, size: Option<Size>) -> (f32, f32) {
        let tile = self.options.tile_size.max(1) as f32;

        size.map_or((tile, tile), |size| {
            (size.x.max(1) as f32 * tile, size.y.max(1) as f32 * tile)
        })
    }

    fn rebuild_all_instances(&mut self) {
        for id in self.state.document_ids() {
            self.rebuild_instances(id);
        }
    }

    pub(super) fn rebuild_instances(&mut self, id: DocumentId) {
        let bake = self.caches.get(&id).and_then(|cache| cache.bake.as_ref());
        let instances = match (self.state.environment.as_ref(), self.state.document(id)) {
            (Some(environment), Some(document)) => frame::build_with_options(
                &environment.tree,
                &environment.icons,
                &self.textures,
                document,
                FrameRenderOptions {
                    visibility: &self.type_visibility,
                    tile_size: self.options.tile_size,
                    appearances: editor::bake::appearances(bake),
                    lighting: bake.and_then(|bake| bake.lighting.as_ref()),
                },
            ),
            _ => FrameInstances::default(),
        };

        let revision = self.bump_revision();
        let cache = self.caches.entry(id).or_default();
        cache.instances = instances;
        cache.revision = revision;
        cache.frame_update = None;
        cache.lighting_revision = revision;
        cache.lighting_update = None;
        self.revalidate_focus();
    }

    fn bump_revision(&mut self) -> u64 {
        let revision = self.next_revision;
        self.next_revision = self.next_revision.wrapping_add(1).max(1);

        revision
    }

    pub(super) fn refresh_reordered_from_affected(&mut self, affected: &[PrefabInstanceId]) {
        let coord = self.state.active_document().and_then(|document| {
            affected
                .iter()
                .find_map(|id| document.instance_location(*id).map(|location| location.coord))
        });
        if let Some(coord) = coord {
            self.refresh_reordered_tile(coord);
        }
    }

    pub(super) fn refresh_reordered_tile(&mut self, coord: Coord) {
        let Some(id) = self.state.active() else {
            return;
        };

        let ordered = {
            let Some(document) = self.state.document(id) else {
                return;
            };
            let Some(tree) = self.tree() else {
                return;
            };
            document
                .instance_ids_at(coord)
                .iter()
                .copied()
                .filter(|owner| {
                    document
                        .prefab_instance(*owner)
                        .is_some_and(|(prefab, _)| context_placement_group(tree, &prefab.path) == Some(0))
                })
                .collect::<Vec<_>>()
        };

        if let Some(cache) = self.caches.get_mut(&id) {
            cache.instances.reorder_placements(&ordered);
        }

        self.apply_bake_update(
            id,
            editor::bake::BakeUpdate {
                appearances: ordered,
                lighting: None,
            },
        );
    }

    pub(super) fn update_instance(&mut self, selected: PrefabInstanceId) { self.update_instances(&[selected]); }

    pub(super) fn update_instances(&mut self, affected: &[PrefabInstanceId]) {
        let Some(id) = self.state.active() else {
            return;
        };

        let Self {
            state, caches, baker, ..
        } = self;
        let cache = caches.entry(id).or_default();
        let bake_update = match (cache.bake.as_mut(), state.environment.as_ref(), state.document(id)) {
            (Some(bake), Some(environment), Some(document)) => {
                let update = editor::bake::update(bake, environment, document, affected);
                report_bake_output(bake);

                update
            },
            _ => {
                baker.invalidate(id);

                editor::bake::BakeUpdate {
                    appearances: affected.to_vec(),
                    lighting: None,
                }
            },
        };

        self.apply_bake_update(id, bake_update);
    }
}

#[cfg(test)]
mod tests {
    use core::path::TreePath;

    use dmm::{Coord, Map, Size};
    use editor::document::{MapDocument, Selection};

    use crate::session::{
        Session,
        TypeLayer,
        fixtures::{examples, flat_session, settle_bake},
    };

    #[test]
    fn capture_regions_measure_from_the_bottom_left_and_clamp_to_the_map() {
        let session = flat_session(4, 3);
        let id = session.state.active().unwrap();
        let tile = session.options.tile_size;
        let block = |min: (u32, u32), max: (u32, u32)| {
            Some(Selection::from_drag(
                Coord::new(min.0, min.1, 1),
                Coord::new(max.0, max.1, 1),
            ))
        };

        assert_eq!(session.capture_region(id, None), Some(([0, 0], [4 * tile, 3 * tile])));
        assert_eq!(
            session.capture_region(id, block((2, 2), (3, 3))),
            Some(([tile, tile], [2 * tile, 2 * tile]))
        );
        assert_eq!(
            session.capture_region(id, block((3, 2), (9, 9))),
            Some(([2 * tile, tile], [2 * tile, 2 * tile]))
        );
        assert_eq!(session.capture_region(id, block((5, 1), (6, 1))), None);
    }

    #[test]
    fn layer_toggles_hide_a_base_type_and_show_all_restores_everything() {
        let mut session = flat_session(2, 1);
        let turf = session.map().unwrap().tile_at(Coord::new(1, 1, 1)).unwrap()[0].clone();
        assert!(session.is_layer_visible(TypeLayer::Turf));
        assert!(!session.hides_types());

        assert!(session.toggle_layer(TypeLayer::Turf));
        assert!(session.toggle_layer(TypeLayer::Obj));
        assert!(!session.is_layer_visible(TypeLayer::Turf));
        assert!(session.is_layer_visible(TypeLayer::Area));
        assert!(
            session.hidden_types().hides(&turf),
            "subtypes are hidden with their base type"
        );

        assert!(session.toggle_layer(TypeLayer::Turf));
        assert!(session.is_layer_visible(TypeLayer::Turf));
        assert!(!session.is_layer_visible(TypeLayer::Obj));

        assert!(session.show_all_types());
        assert!(session.is_layer_visible(TypeLayer::Obj));
        assert!(!session.hides_types());
        assert!(!session.show_all_types(), "nothing left to show");
    }

    #[test]
    fn both_open_maps_contribute_their_own_sprites_to_one_frame() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).expect("codebase");
        session.open_map(&root.join("test.dmm"), 1).expect("first map");
        session.open_map(&root.join("test2.dmm"), 1).expect("second map");
        let ids = session.state.document_ids();

        // Two maps docked side by side in one window.
        let left = render::MapViewRect {
            x: 0,
            y: 0,
            width: 400,
            height: 600,
        };
        let right = render::MapViewRect {
            x: 400,
            y: 0,
            width: 400,
            height: 600,
        };
        let map_views = [
            session
                .map_view_frame(ids[0], left, Default::default(), Default::default(), &[], &[])
                .expect("left map view"),
            session
                .map_view_frame(ids[1], right, Default::default(), Default::default(), &[], &[])
                .expect("right map view"),
        ];
        let frame = session.frame(&map_views, Some(1));

        assert_eq!(frame.map_views.len(), 2, "both maps are in the frame");
        assert_eq!(frame.picking, Some(1));
        assert_ne!(frame.map_views[0].rect, frame.map_views[1].rect);
        // Each map view carries its own map's sprites, not a shared cache.
        assert!(!frame.map_views[0].sprite_instances.is_empty());
        assert!(!frame.map_views[1].sprite_instances.is_empty());
        assert_ne!(
            frame.map_views[0].sprite_instances.len(),
            frame.map_views[1].sprite_instances.len(),
        );
    }

    #[test]
    fn hiding_a_type_redraws_from_the_cached_bake() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).expect("codebase");
        session.open_map(&root.join("test.dmm"), 1).expect("map");
        settle_bake(&mut session);
        let id = session.state.active().expect("active map");
        let table = session
            .tree()
            .and_then(|tree| tree.id_of(&TreePath::parse("/obj/structure/table")))
            .expect("table type");

        // A bake that ran again would start its counters over.
        session
            .caches
            .get_mut(&id)
            .and_then(|cache| cache.bake.as_mut())
            .expect("the example map is baked")
            .cache_hits = usize::MAX;

        assert!(session.toggle_type_visibility(table));
        assert_eq!(
            session.active_cache().bake.as_ref().map(|bake| bake.cache_hits),
            Some(usize::MAX)
        );
    }

    #[test]
    fn a_map_view_frame_follows_its_own_documents_z_level() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).expect("codebase");
        session.open_map(&root.join("test.dmm"), 1).expect("first map");
        let first = session.state.active().expect("first is active");
        // test2.dmm has two z levels.
        session.open_map(&root.join("test2.dmm"), 2).expect("second map");
        let second = session.state.active().expect("second is active");

        let rect = render::MapViewRect::default();
        let a = session
            .map_view_frame(first, rect, Default::default(), Default::default(), &[], &[])
            .expect("first map view");
        let b = session
            .map_view_frame(second, rect, Default::default(), Default::default(), &[], &[])
            .expect("second map view");

        assert_eq!(a.active_z, 1);
        assert_eq!(b.active_z, 2, "z is per document, not per editor");
    }

    #[test]
    fn two_open_maps_never_share_a_sprite_revision() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).expect("codebase");
        session.open_map(&root.join("test.dmm"), 1).expect("first map");
        session.open_map(&root.join("test2.dmm"), 1).expect("second map");

        let ids = session.state.document_ids();
        assert_eq!(ids.len(), 2, "both maps stay open");

        // The renderer holds a single `uploaded_revision` for whatever is on the
        // GPU. Two documents claiming one revision makes it skip the upload and
        // keep drawing the map that got there first.
        let revisions: Vec<u64> = ids.iter().map(|id| session.caches[id].revision).collect();
        assert_ne!(revisions[0], revisions[1]);

        // And the caches really do describe different maps.
        let first = session
            .map_view_frame(
                ids[0],
                render::MapViewRect::default(),
                render::Camera::default(),
                Default::default(),
                &[],
                &[],
            )
            .expect("first frame");
        let second = session
            .map_view_frame(
                ids[1],
                render::MapViewRect::default(),
                render::Camera::default(),
                Default::default(),
                &[],
                &[],
            )
            .expect("second frame");
        assert_ne!(first.revision, second.revision);
        assert_ne!(first.sprite_instances.len(), second.sprite_instances.len());
    }

    #[test]
    fn editing_one_open_map_leaves_the_other_cache_alone() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).expect("codebase");
        session.open_map(&root.join("test.dmm"), 1).expect("first map");
        let first = session.state.active().expect("first is active");
        session.open_map(&root.join("test2.dmm"), 1).expect("second map");
        let second = session.state.active().expect("second is active");

        let untouched = session.caches[&first].revision;
        let before = session.caches[&second].revision;

        session.choose_type(
            session
                .tree()
                .and_then(|tree| tree.id_of(&TreePath::parse("/obj/structure/table")))
                .expect("a placeable type"),
        );
        assert!(session.place_at(Coord::new(2, 2, 1), None).is_some(), "edit applied");

        assert_eq!(session.caches[&first].revision, untouched, "the other map is untouched");
        assert_ne!(session.caches[&second].revision, before, "the edited map re-uploads");
    }

    #[test]
    fn view_changes_do_not_change_the_sprite_revision() {
        let mut session = Session::new();
        let id = session
            .state
            .open_document(MapDocument::new(Map::new(Size { x: 1, y: 1, z: 3 }), 1));
        session.caches.entry(id).or_default().revision = 7;

        session.set_level(2);
        session.set_underlay_depth(1);
        session.toggle_areas();
        assert!(session.options.show_areas);
        assert!(session.options.show_area_outlines);
        session.toggle_area_outlines();

        let view = session
            .map_view_frame(
                id,
                render::MapViewRect::default(),
                Default::default(),
                Default::default(),
                &[],
                &[],
            )
            .expect("the open map has a map view");
        assert_eq!(view.revision, 7, "changing the view does not re-upload sprites");
        assert_eq!(view.active_z, 2);

        let map_views = [view];
        let frame = session.frame(&map_views, None);
        assert_eq!(frame.underlay_depth, 1);
        assert!(frame.show_areas);
        assert!(!frame.show_area_outlines);
    }

    #[test]
    fn toggling_lighting_off_withholds_the_light_grid() {
        let mut session = Session::new();
        let id = session
            .state
            .open_document(MapDocument::new(Map::new(Size { x: 1, y: 1, z: 1 }), 1));
        let cache = session.caches.entry(id).or_default();
        cache.instances.lighting_size = [1, 1, 1];
        cache.instances.light_tiles = vec![render::LightTile { corners: [[1.0; 3]; 4] }];

        let view = |session: &Session| {
            session
                .map_view_frame(
                    id,
                    render::MapViewRect::default(),
                    Default::default(),
                    Default::default(),
                    &[],
                    &[],
                )
                .expect("the open map has a map view")
                .lighting
                .map(|lighting| lighting.minimum_brightness)
        };

        assert!(session.options.show_lighting);
        assert_eq!(view(&session), Some(0.0));

        session.options.minimum_light_brightness_percent = 35;
        assert_eq!(view(&session), Some(0.35));

        session.toggle_lighting();
        assert_eq!(view(&session), None);

        session.toggle_lighting();
        assert_eq!(view(&session), Some(0.35));
    }
}
