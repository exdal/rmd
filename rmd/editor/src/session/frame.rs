use dmm::{Coord, Prefab, Size};
use editor::{
    document::{DocumentId, PrefabInstanceId, Selection},
    focus::AreaFocus,
    frame::{self, FrameInstances, FrameRenderOptions, PrefabUpdate},
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
            self.apply_type_visibility();
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

    pub(super) fn rebuild_instances(&mut self, id: DocumentId) {
        if let Some(cache) = self.caches.get_mut(&id) {
            drop(std::mem::take(&mut cache.instances));
        }

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
        cache.frame_updates.clear();
        cache.lighting_revision = revision;
        cache.lighting_updates.clear();
        self.revalidate_focus();
    }

    pub(super) fn bump_revision(&mut self) -> u64 {
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

        let Some(ordered) = self.placement_group_at(id, coord) else {
            return;
        };

        // drawn in their new tile order once the render cache takes their indices again
        self.apply_bake_update(
            id,
            editor::bake::BakeUpdate {
                appearances: ordered,
                lighting: None,
            },
        );
    }

    pub(super) fn apply_remote_tiles(&mut self, id: DocumentId, tiles: Vec<(Coord, Vec<Prefab>)>) -> bool {
        let Some(document) = self.state.document_mut(id) else {
            return false;
        };

        let affected = document.apply_remote(tiles);
        self.update_document_instances(id, &affected);

        true
    }

    fn placement_group_at(&self, id: DocumentId, coord: Coord) -> Option<Vec<PrefabInstanceId>> {
        let document = self.state.document(id)?;
        let tree = self.tree()?;

        Some(
            document
                .instance_ids_at(coord)
                .iter()
                .copied()
                .filter(|owner| {
                    document
                        .prefab_instance(*owner)
                        .is_some_and(|instance| context_placement_group(tree, &instance.prefab().path) == Some(0))
                })
                .collect(),
        )
    }

    pub(super) fn update_instance(&mut self, selected: PrefabInstanceId) { self.update_instances(&[selected]); }

    pub(super) fn update_instances(&mut self, affected: &[PrefabInstanceId]) {
        if let Some(id) = self.state.active() {
            self.update_document_instances(id, affected);
        }
    }

    pub(super) fn update_document_instances(&mut self, id: DocumentId, affected: &[PrefabInstanceId]) {
        let Self {
            state, caches, baker, ..
        } = self;
        let cache = caches.entry(id).or_default();
        let bake_update = match (cache.bake.as_mut(), state.environment.as_ref(), state.document(id)) {
            (Some(bake), Some(environment), Some(document)) => {
                // a level the bake doesn't have yet is baked whole by its extension
                let levels = editor::bake::levels(bake);
                let (baked, later) = affected.iter().copied().partition::<Vec<_>, _>(|instance| {
                    document
                        .prefab_instance(*instance)
                        .is_none_or(|instance| instance.location.coord.z <= levels)
                });
                let mut update = editor::bake::update(bake, environment, document, &baked);
                report_bake_output(bake);
                update.appearances.extend(later);

                update
            },
            _ => {
                if baker.extending(id) {
                    cache.unbaked.extend_from_slice(affected);
                } else {
                    baker.invalidate(id);
                }

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
    use core::{path::TreePath, types::Value};

    use dmm::{Coord, Map, Prefab, Size};
    use editor::{
        document::{MapDocument, Selection},
        tool::Tool,
    };

    use crate::session::{
        Session,
        TypeLayer,
        bake::MAX_PENDING_UPDATES,
        fixtures::{assert_render_cache_matches_rebuild, examples, flat_session, settle_bake},
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
    fn remote_edits_update_the_render_cache_like_a_rebuild() {
        let mut session = flat_session(3, 1);
        let id = session.state.active().unwrap();
        let (from, to) = (Coord::new(1, 1, 1), Coord::new(2, 1, 1));
        session.set_tool(Tool::Place);
        session
            .state
            .choose_prefab(Prefab::new(TreePath::parse("/obj/structure/table")));
        session.place_at(from, None).unwrap();

        let tile = |session: &Session, coord| session.map().unwrap().tile_at(coord).unwrap().clone();
        let mut source = tile(&session, from);
        let table = source.remove(source.len() - 1);
        let mut destination = tile(&session, to);
        destination.push(table);
        let mut repainted = tile(&session, Coord::new(3, 1, 1));
        let mut engineering = Prefab::new(TreePath::parse("/area/station"));
        engineering.set_var("name".into(), Value::Text(String::from("Engineering")));
        *repainted.last_mut().unwrap() = engineering;
        let affected = session.state.document_mut(id).unwrap().apply_remote(vec![
            (from, source),
            (to, destination),
            (Coord::new(3, 1, 1), repainted),
        ]);
        session.update_document_instances(id, &affected);

        assert_eq!(affected.len(), 4, "the moved table and the repainted area, old and new");
        assert_render_cache_matches_rebuild(&session);
    }

    fn named_table(name: &str) -> Prefab {
        let mut table = Prefab::new(TreePath::parse("/obj/structure/table"));
        table.set_var("name".into(), Value::Text(name.into()));

        table
    }

    fn tables_session(names: &[&str]) -> Session {
        let mut session = flat_session(1, 1);
        session.set_tool(Tool::Place);
        for name in names {
            session.state.choose_prefab(named_table(name));
            session.place_at(Coord::new(1, 1, 1), None).unwrap();
        }

        session
    }

    fn drawn_tables(session: &Session) -> Vec<String> {
        let document = session.state.active_document().unwrap();
        session
            .instances()
            .unwrap()
            .sprites
            .iter()
            .filter_map(|sprite| document.prefab_instance(sprite.owner))
            .filter(|instance| instance.prefab().path == TreePath::parse("/obj/structure/table"))
            .map(|instance| match instance.prefab().var(&"name".into()) {
                Some(Value::Text(name)) => name.clone(),
                _ => String::new(),
            })
            .collect()
    }

    fn apply_remote_tile(session: &mut Session, tile: Vec<Prefab>) {
        let id = session.state.active().unwrap();
        assert!(session.apply_remote_tiles(id, vec![(Coord::new(1, 1, 1), tile)]));
    }

    #[test]
    fn a_remote_reorder_changes_equal_layer_draw_order() {
        let mut session = tables_session(&["a", "b"]);
        assert_eq!(drawn_tables(&session), ["a", "b"]);
        let mut tile = session.map().unwrap().tile_at(Coord::new(1, 1, 1)).unwrap().clone();
        let a = tile.iter().position(|prefab| *prefab == named_table("a")).unwrap();
        let b = tile.iter().position(|prefab| *prefab == named_table("b")).unwrap();
        tile.swap(a, b);

        apply_remote_tile(&mut session, tile);

        assert_eq!(drawn_tables(&session), ["b", "a"]);
        assert_render_cache_matches_rebuild(&session);
    }

    #[test]
    fn a_remote_insert_between_equal_layers_draws_between_them() {
        let mut session = tables_session(&["a", "b"]);
        let mut tile = session.map().unwrap().tile_at(Coord::new(1, 1, 1)).unwrap().clone();
        let b = tile.iter().position(|prefab| *prefab == named_table("b")).unwrap();
        tile.insert(b, named_table("c"));

        apply_remote_tile(&mut session, tile);

        assert_eq!(drawn_tables(&session), ["a", "c", "b"]);
        assert_render_cache_matches_rebuild(&session);
    }

    #[test]
    fn edits_between_frames_chain_their_updates_up_to_a_cap() {
        let mut session = flat_session(12, 1);
        session.set_tool(Tool::Place);
        session
            .state
            .choose_prefab(Prefab::new(TreePath::parse("/obj/structure/table")));
        let before = session.active_cache().revision;

        session.place_at(Coord::new(1, 1, 1), None).unwrap();
        session.place_at(Coord::new(2, 1, 1), None).unwrap();

        let updates = &session.active_cache().frame_updates;
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].previous_revision, before);

        for x in 3..=12 {
            session.place_at(Coord::new(x, 1, 1), None).unwrap();
        }
        assert_eq!(session.active_cache().frame_updates.len(), MAX_PENDING_UPDATES);
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
        for id in &ids {
            assert!(session.caches[id].instances.live_sprites().next().is_some());
        }
        assert_ne!(frame.map_views[0].sprite_instances, frame.map_views[1].sprite_instances);
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

        let revision = session.active_cache().revision;
        assert!(session.toggle_type_visibility(table));
        assert_eq!(
            session.active_cache().bake.as_ref().map(|bake| bake.cache_hits),
            Some(usize::MAX)
        );
        assert_eq!(
            session
                .active_cache()
                .frame_updates
                .last()
                .map(|update| update.previous_revision),
            Some(revision),
            "the renderer patches the pages it touched instead of uploading a rebuilt cache"
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
        assert_ne!(first.sprite_instances, second.sprite_instances);
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
