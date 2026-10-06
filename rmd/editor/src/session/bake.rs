use editor::{
    bake::BakeUpdate,
    document::DocumentId,
    frame::{self, FrameRenderOptions, PrefabUpdate},
};
use render::FrameUpdate;

use super::{Session, always_highlighted};
use crate::baker::{self, Baked, Job};

// updates the renderer may not have drawn yet, so a frame that missed some can still catch up
pub(super) const MAX_PENDING_UPDATES: usize = 8;

fn push_pending<T>(updates: &mut Vec<T>, update: T) {
    if updates.len() == MAX_PENDING_UPDATES {
        updates.remove(0);
    }

    updates.push(update);
}

pub(super) fn report_bake_output(bake: &mut editor::bake::Bake) {
    for line in bake.take_output() {
        log::info!("DM: {line}");
    }
}

impl Session {
    fn collect_bake_diagnostics(&mut self) {
        self.diagnostics.bake = self
            .caches
            .values()
            .filter_map(|cache| cache.bake.as_ref())
            .flat_map(|bake| bake.diagnostics.entries.iter())
            .filter(|entry| entry.count > 0)
            .cloned()
            .collect();
    }

    pub(super) fn rebake_all(&mut self) {
        for id in self.state.document_ids() {
            self.rebake(id);
        }
    }

    pub(super) fn rebake(&mut self, id: DocumentId) {
        // an extension in flight carries the old bake back, which this replaces
        self.baker.invalidate(id);
        let cache = self.caches.entry(id).or_default();
        cache.bake = None;
        cache.unbaked.clear();
        self.collect_bake_diagnostics();
        self.rebuild_instances(id);

        if !self.queued_bakes.contains(&id) {
            self.queued_bakes.push(id);
        }

        self.start_next_bake();
    }

    fn start_next_bake(&mut self) {
        while !self.baker.is_busy() {
            let Some(id) = self.queued_bakes.first().copied() else {
                return;
            };
            self.queued_bakes.remove(0);

            let Some(environment) = self.state.environment.clone() else {
                return;
            };
            let Some(document) = self.state.document(id) else {
                continue;
            };
            if environment.bake_program.is_none() {
                return;
            }

            let level_count = document.map.size().z;
            let behind = self
                .caches
                .get_mut(&id)
                .and_then(|cache| cache.bake.take_if(|bake| editor::bake::levels(bake) < level_count));
            let job = match behind {
                Some(bake) => Job::Extend {
                    atoms: editor::bake::atoms_from_level(&environment, document, editor::bake::levels(&bake) + 1),
                    bake: Box::new(bake),
                    level_count,
                },
                None => Job::Build {
                    atoms: editor::bake::atoms(&environment, document),
                    size: editor::bake::size(document),
                },
            };

            self.baker.start(baker::Request {
                document: id,
                path: document
                    .path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default(),
                environment,
                job,
            });
        }
    }

    // a level the bake doesn't have yet bakes on its own, the rest of the map keeps its bake
    pub(super) fn bake_new_levels(&mut self, id: DocumentId) {
        if self.baker.extending(id) {
            return;
        }

        // a full bake of the smaller map is on its way
        if self.baker.baking(id) {
            self.baker.invalidate(id);
            return;
        }

        let Some(level_count) = self.state.document(id).map(|document| document.map.size().z) else {
            return;
        };

        let is_behind = self
            .caches
            .get(&id)
            .and_then(|cache| cache.bake.as_ref())
            .is_some_and(|bake| editor::bake::levels(bake) < level_count);
        if !is_behind {
            return;
        }

        if !self.queued_bakes.contains(&id) {
            self.queued_bakes.push(id);
        }

        self.start_next_bake();
    }

    pub fn poll_bake(&mut self) {
        if let Some((id, baked, outdated)) = self.baker.poll() {
            let open = self.state.document(id).is_some();
            if outdated && open {
                self.rebake(id);
            } else if open {
                match baked {
                    Baked::Build(bake) => self.land_bake(id, bake.map(|bake| *bake)),
                    Baked::Extend(bake, update) => self.land_extension(id, *bake, update),
                }
            }
        }

        self.start_next_bake();
    }

    fn land_extension(&mut self, id: DocumentId, mut bake: editor::bake::Bake, mut update: BakeUpdate) {
        report_bake_output(&mut bake);
        let levels = editor::bake::levels(&bake);
        let cache = self.caches.entry(id).or_default();
        let unbaked = std::mem::take(&mut cache.unbaked);
        cache.bake = Some(bake);

        if let (Some(environment), Some(document), Some(bake)) = (
            self.state.environment.as_ref(),
            self.state.document(id),
            cache.bake.as_mut(),
        ) {
            // levels added since it left get their own extension
            let (replayed, later) = unbaked.into_iter().partition::<Vec<_>, _>(|instance| {
                document
                    .prefab_instance(*instance)
                    .is_none_or(|(_, location)| location.coord.z <= levels)
            });
            let edits = editor::bake::update(bake, environment, document, &replayed);
            report_bake_output(bake);
            update.appearances.extend(edits.appearances);
            update.appearances.extend(later);
            update.appearances.sort_unstable();
            update.appearances.dedup();
            update.lighting = match (update.lighting, edits.lighting) {
                (Some(left), Some(right)) => Some(left.start.min(right.start)..left.end.max(right.end)),
                (left, right) => left.or(right),
            };
        }

        if let Some(feedback) = self.ui_feedback.clone() {
            self.replay_ui(id, &feedback);
        }

        self.collect_bake_diagnostics();
        self.apply_bake_update(id, update);
        self.resize_lighting(id);
        self.flush_pending_rebake(id);
        self.bake_new_levels(id);
    }

    // a level with no light in it leaves the lightmap untouched, but it still grew
    fn resize_lighting(&mut self, id: DocumentId) {
        let Some(cache) = self.caches.get(&id) else {
            return;
        };

        let is_stale = cache
            .bake
            .as_ref()
            .and_then(|bake| bake.lighting.as_ref())
            .is_some_and(|lighting| lighting.size != cache.instances.lighting_size);
        if !is_stale {
            return;
        }

        let revision = self.bump_revision();
        let cache = self.caches.entry(id).or_default();
        let lighting = cache.bake.as_ref().and_then(|bake| bake.lighting.as_ref());
        cache.instances.update_lighting(lighting, None);
        cache.lighting_revision = revision;
        cache.lighting_updates.clear();
    }

    fn land_bake(&mut self, id: DocumentId, mut bake: Option<editor::bake::Bake>) {
        if let Some(bake) = bake.as_mut() {
            report_bake_output(bake);
        }

        let cache = self.caches.entry(id).or_default();
        cache.bake = bake;
        cache.pending_rebake = editor::bake::UiRebake::default();
        // a bake starts from the profile's own defaults, so it has to be told what the
        // panel already asked for before it is first shown
        if let Some(feedback) = self.ui_feedback.clone() {
            self.replay_ui(id, &feedback);
        }

        let cache = self.caches.entry(id).or_default();
        cache.always_highlights = always_highlighted(cache.bake.as_ref());
        self.collect_bake_diagnostics();
        self.rebuild_instances(id);
        self.flush_pending_rebake(id);
    }

    pub fn bake_view(&self) -> Option<crate::loader::LoadView> { self.baker.view() }

    pub(super) fn apply_bake_update(&mut self, id: DocumentId, bake_update: editor::bake::BakeUpdate) {
        let Self {
            state,
            textures,
            caches,
            type_visibility,
            options,
            next_revision,
            ..
        } = self;
        let cache = caches.entry(id).or_default();
        cache.always_highlights = always_highlighted(cache.bake.as_ref());
        let affected = bake_update.appearances;
        let update = match (state.environment.as_ref(), state.document(id)) {
            (Some(environment), Some(document)) => frame::update_prefabs_with_options(
                &mut cache.instances,
                &environment.tree,
                &environment.icons,
                textures,
                document,
                &affected,
                FrameRenderOptions {
                    visibility: type_visibility,
                    tile_size: options.tile_size,
                    appearances: editor::bake::appearances(cache.bake.as_ref()),
                    lighting: cache.bake.as_ref().and_then(|bake| bake.lighting.as_ref()),
                },
            ),
            _ => PrefabUpdate::Unchanged,
        };

        if let Some(range) = bake_update.lighting {
            cache.instances.update_lighting(
                cache.bake.as_ref().and_then(|bake| bake.lighting.as_ref()),
                Some(range.clone()),
            );
            let previous_revision = cache.lighting_revision;
            cache.lighting_revision = *next_revision;
            *next_revision = next_revision.wrapping_add(1).max(1);
            push_pending(
                &mut cache.lighting_updates,
                render::LightingUpdate {
                    previous_revision,
                    tiles: render::UpdateRange {
                        start: range.start,
                        end: range.end,
                    },
                },
            );
        }

        self.publish_frame_update(id, update);
        self.revalidate_focus();
    }

    pub(super) fn publish_frame_update(&mut self, id: DocumentId, update: PrefabUpdate) {
        match update {
            PrefabUpdate::Unchanged => {},
            PrefabUpdate::Buffers { sprites } => {
                let revision = self.bump_revision();
                let cache = self.caches.entry(id).or_default();
                push_pending(
                    &mut cache.frame_updates,
                    FrameUpdate {
                        previous_revision: std::mem::replace(&mut cache.revision, revision),
                        sprites,
                    },
                );
            },
            PrefabUpdate::Rebuild => self.rebuild_instances(id),
        }
    }
}

#[cfg(test)]
mod tests {
    use core::{path::TreePath, types::Value};
    use std::path::PathBuf;

    use dmm::{Coord, Map, Prefab, Size};
    use editor::{document::MapDocument, progress::Progress, tool::Tool};

    use crate::session::{
        Session,
        fixtures::{assert_render_cache_matches_rebuild, examples, live_bytes, settle_bake, take_peak_bytes},
    };

    #[test]
    #[ignore = "requires the local target/MonkeStation2.0 checkout"]
    fn bundled_monkestation_profile_bakes_debug_maps() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/MonkeStation2.0");
        let entry = root.join("tgstation.dme");
        let options = editor::environment::BakeOptions {
            forced_profile: Some(String::from("monkestation")),
            ..Default::default()
        };
        let loaded = crate::loader::load_codebase(&entry, &options, &Progress::new()).expect("Monkestation codebase");
        assert!(
            !loaded.diagnostics.bake_preprocess.iter().any(|error| error.is_fatal()),
            "{:?}",
            loaded.diagnostics.bake_preprocess
        );
        assert!(
            loaded.diagnostics.bake_sema.is_empty(),
            "{:?}",
            loaded.diagnostics.bake_sema
        );
        assert!(loaded.diagnostics.codegen.is_none(), "{:?}", loaded.diagnostics.codegen);
        assert!(loaded.diagnostics.profile.is_none(), "{:?}", loaded.diagnostics.profile);
        assert_eq!(
            loaded
                .environment
                .profiles
                .as_ref()
                .map(|profiles| profiles.active.as_str()),
            Some("/datum/demir/monkestation")
        );
        assert!(loaded.environment.bake_program.is_some());

        let mut session = Session::new();
        session.apply_codebase(loaded);
        let mut flashlight = Prefab::new(TreePath::parse("/obj/item/flashlight"));
        flashlight.set_var("start_on".into(), Value::Num(1.0));
        let mut light_map = Map::new(Size { x: 1, y: 1, z: 1 });
        let tile = light_map.intern_tile(vec![
            flashlight,
            Prefab::new(TreePath::parse("/turf/open/floor/iron")),
            Prefab::new(TreePath::parse("/area/station/engineering/main")),
        ]);
        light_map.grid[0][0][0] = tile;
        session.activate_document(MapDocument::new(light_map, 1));
        settle_bake(&mut session);
        let light_bake = session.active_cache().bake.as_ref().expect("lit flashlight bake");
        assert_eq!(
            light_bake.diagnostics.count(),
            0,
            "{:?}",
            light_bake.diagnostics.entries
        );
        assert!(
            light_bake
                .appearances
                .values()
                .flat_map(|appearance| &appearance.underlays)
                .any(|underlay| underlay.lighting == vm::AppearanceLighting::OverlayLight),
            "Monkestation lighting preview did not emit a light mask"
        );

        for name in ["runtimestation.dmm", "multiz.dmm"] {
            session
                .open_map(&root.join("_maps/map_files/debug").join(name), 1)
                .expect("Monkestation debug map");
            settle_bake(&mut session);
            let bake = session.active_cache().bake.as_ref().expect("completed map bake");
            assert!(bake.succeeded > 0, "{name} baked no atoms");
            let faults = bake
                .diagnostics
                .entries
                .iter()
                .map(|entry| {
                    let file = session
                        .state
                        .environment
                        .as_ref()
                        .and_then(|environment| environment.bake_file(entry.fault.location.file));
                    format!(
                        "{} atoms: {:?} at {}",
                        entry.count,
                        entry.fault.kind,
                        entry.fault.location.display(file)
                    )
                })
                .collect::<Vec<_>>();
            assert_eq!(bake.diagnostics.count(), 0, "{name}: {faults:#?}");
        }
    }

    #[test]
    #[ignore = "requires the local target/MonkeStation2.0 checkout"]
    fn monkestation_fixtures_light_corners_like_the_game() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/MonkeStation2.0");
        let options = editor::environment::BakeOptions {
            forced_profile: Some(String::from("monkestation")),
            ..Default::default()
        };
        let loaded = crate::loader::load_codebase(&root.join("tgstation.dme"), &options, &Progress::new())
            .expect("Monkestation codebase");
        assert!(loaded.diagnostics.profile.is_none(), "{:?}", loaded.diagnostics.profile);
        let mut session = Session::new();
        session.apply_codebase(loaded);

        let area = "/area/station/engineering/main";
        let floor = || {
            vec![
                Prefab::new(TreePath::parse("/turf/open/floor/iron")),
                Prefab::new(TreePath::parse(area)),
            ]
        };
        let mut map = Map::new(Size { x: 8, y: 1, z: 1 });
        let mut lit = floor();
        lit.insert(0, Prefab::new(TreePath::parse("/obj/machinery/light")));
        let lit_tile = map.intern_tile(lit);
        let floor_tile = map.intern_tile(floor());
        map.grid[0][0] = vec![floor_tile; 8];
        map.grid[0][0][0] = lit_tile;
        session.activate_document(MapDocument::new(map, 1));
        settle_bake(&mut session);

        let bake = session.active_cache().bake.as_ref().expect("fixture bake");
        assert_eq!(bake.diagnostics.count(), 0, "{:?}", bake.diagnostics.entries);
        let corners = |x| {
            bake.lighting
                .as_ref()
                .and_then(|lighting| lighting.tile(vm::world::Position::new(x, 1, 1)))
                .expect("lit tile")
                .corners
        };

        // LIGHT_COLOR_DEFAULT is #f3fffac4, and PARSE_LIGHT_COLOR drops its alpha.
        let color = [243.0 / 255.0, 1.0, 250.0 / 255.0];
        for (got, want) in corners(1)[0].iter().zip(color) {
            assert!((got - want).abs() < 1e-3, "fixture tile {:?}", corners(1));
        }

        // LUM_FALLOFF with the bulb's outer 7, inner 1.5 and curve 2.36, power 1, for the south-west
        // corner of the sixth tile.
        let distance = (4.5f32 * 4.5 + 0.5 * 0.5).sqrt();
        let falloff = ((7.0 - distance) / (7.0f32 - 1.5)).clamp(0.0, 1.0).powf(2.36);
        let got = corners(6)[0][1];
        assert!((got - falloff).abs() < 1e-3, "got {got}, the game lights {falloff}");
    }

    #[test]
    #[ignore = "requires the local target/MonkeStation2.0 checkout"]
    fn monkestation_windows_take_department_colors_and_sills() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/MonkeStation2.0");
        let options = editor::environment::BakeOptions {
            forced_profile: Some(String::from("monkestation")),
            ..Default::default()
        };
        let loaded = crate::loader::load_codebase(&root.join("tgstation.dme"), &options, &Progress::new())
            .expect("Monkestation codebase");
        assert!(loaded.diagnostics.profile.is_none(), "{:?}", loaded.diagnostics.profile);
        let mut session = Session::new();
        session.apply_codebase(loaded);

        let window = "/obj/structure/window/reinforced/fulltile";
        let spawner = "/obj/effect/spawner/structure/window/reinforced";
        let tile = |path: &str, area: &str| {
            vec![
                Prefab::new(TreePath::parse(path)),
                Prefab::new(TreePath::parse("/turf/open/floor/iron")),
                Prefab::new(TreePath::parse(area)),
            ]
        };
        let science = "/area/station/science/lab";
        let mut map = Map::new(Size { x: 4, y: 1, z: 1 });
        let science_window = map.intern_tile(tile(window, science));
        let science_spawner = map.intern_tile(tile(spawner, science));
        let hallway_window = map.intern_tile(tile(window, "/area/station/hallway/primary/central"));
        map.grid[0][0] = vec![science_window, science_window, science_spawner, hallway_window];
        session.activate_document(MapDocument::new(map, 1));
        settle_bake(&mut session);

        let bake = session.active_cache().bake.as_ref().expect("window bake");
        assert_eq!(bake.diagnostics.count(), 0, "{:?}", bake.diagnostics.entries);
        let environment = session.state.environment.as_ref().unwrap();
        let document = session.state.active_document().unwrap();
        let baked = |x, path: &str| {
            let instance = *document
                .instance_ids_at(Coord::new(x, 1, 1))
                .iter()
                .find(|id| document.prefab_instance(**id).unwrap().0.path.to_string() == path)
                .unwrap();
            let (prefab, _) = document.prefab_instance(instance).unwrap();
            let ty = environment.tree.id_of(&prefab.path).unwrap();
            let delta = bake.appearances.get(&instance.get()).expect("baked appearance");
            let owner = editor::visual::resolve_delta(&environment.tree, ty, prefab, delta);
            let overlays = delta
                .overlays
                .iter()
                .map(|overlay| editor::visual::resolve_overlay(&environment.tree, &owner, overlay))
                .collect::<Vec<_>>();
            (owner, overlays)
        };
        // Overlay colors carry an alpha byte.
        let color = |appearance: &editor::visual::Appearance| {
            appearance
                .color
                .as_deref()
                .map(|color| color.get(..7).unwrap_or(color).to_ascii_lowercase())
        };
        let has_sill = |overlays: &[editor::visual::Appearance]| {
            overlays.iter().any(|overlay| {
                overlay
                    .icon
                    .as_deref()
                    .is_some_and(|icon| icon.ends_with("window_sill.dmi"))
            })
        };

        // SSstation_coloring paints science purple.
        let (owner, overlays) = baked(1, window);
        assert_eq!(color(&owner).as_deref(), Some("#d381c9"));
        assert!(has_sill(&overlays), "{overlays:#?}");

        let (_, overlays) = baked(3, spawner);
        assert!(
            overlays
                .iter()
                .any(|overlay| color(overlay).as_deref() == Some("#d381c9"))
        );
        assert!(has_sill(&overlays), "{overlays:#?}");

        // Outside a department, the window keeps get_default_color().
        let (owner, _) = baked(4, window);
        assert!(
            ["#1a356e", "#305a6d", "#164f41"].contains(&color(&owner).as_deref().unwrap_or_default()),
            "{:?}",
            owner.color
        );
    }

    #[test]
    #[ignore = "requires the local target/MonkeStation2.0 checkout"]
    fn monkestation_computers_draw_their_powered_overlays() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/MonkeStation2.0");
        let options = editor::environment::BakeOptions {
            forced_profile: Some(String::from("monkestation")),
            ..Default::default()
        };
        let loaded = crate::loader::load_codebase(&root.join("tgstation.dme"), &options, &Progress::new())
            .expect("Monkestation codebase");
        assert!(loaded.diagnostics.profile.is_none(), "{:?}", loaded.diagnostics.profile);
        let mut session = Session::new();
        session.apply_codebase(loaded);

        let computer = "/obj/machinery/computer";
        let mut map = Map::new(Size { x: 1, y: 1, z: 1 });
        let tile = map.intern_tile(vec![
            Prefab::new(TreePath::parse(computer)),
            Prefab::new(TreePath::parse("/turf/open/floor/iron")),
            Prefab::new(TreePath::parse("/area/station/engineering/main")),
        ]);
        map.grid[0][0][0] = tile;
        session.activate_document(MapDocument::new(map, 1));
        settle_bake(&mut session);

        let bake = session.active_cache().bake.as_ref().expect("computer bake");
        assert_eq!(bake.diagnostics.count(), 0, "{:?}", bake.diagnostics.entries);
        let environment = session.state.environment.as_ref().unwrap();
        let document = session.state.active_document().unwrap();
        let instance = *document
            .instance_ids_at(Coord::new(1, 1, 1))
            .iter()
            .find(|id| document.prefab_instance(**id).unwrap().0.path.to_string() == computer)
            .unwrap();
        let (prefab, _) = document.prefab_instance(instance).unwrap();
        let ty = environment.tree.id_of(&prefab.path).unwrap();
        let delta = bake.appearances.get(&instance.get()).expect("baked computer");
        let owner = editor::visual::resolve_delta(&environment.tree, ty, prefab, delta);
        let overlays = delta
            .overlays
            .iter()
            .map(|overlay| editor::visual::resolve_overlay(&environment.tree, &owner, overlay))
            .collect::<Vec<_>>();

        // The keyboard, then the screen and its emissive copy.
        let states = overlays
            .iter()
            .filter_map(|overlay| overlay.icon_state.as_deref())
            .collect::<Vec<_>>();
        assert!(states.contains(&"generic_key"), "{states:?}");
        assert!(states.contains(&"generic"), "{states:?}");
        assert!(
            overlays
                .iter()
                .any(|overlay| overlay.lighting == vm::AppearanceLighting::Emissive),
            "{overlays:#?}"
        );
    }

    #[test]
    #[ignore = "requires the local target/tgstation checkout"]
    fn tgstation_fixture_glows_follow_the_lighting_toggle() {
        use std::collections::HashMap;

        use editor::{
            bake::{UiCommand, UiFeedback, UiValue},
            visual,
        };

        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/tgstation");
        let options = editor::environment::BakeOptions {
            forced_profile: Some(String::from("tgstation")),
            ..Default::default()
        };
        let loaded = crate::loader::load_codebase(&root.join("tgstation.dme"), &options, &Progress::new())
            .expect("tgstation codebase");
        assert!(loaded.diagnostics.profile.is_none(), "{:?}", loaded.diagnostics.profile);
        let mut session = Session::new();
        session.apply_codebase(loaded);

        type Fixture<'a> = (&'a str, &'a [(&'a str, f32)], bool);
        let fixtures: &[Fixture<'_>] = &[
            ("/obj/machinery/light", &[], true),
            ("/obj/machinery/light/small", &[], true),
            ("/obj/machinery/light/floor", &[], true),
            ("/obj/machinery/light", &[("nightshift_enabled", 1.0)], true),
            ("/obj/machinery/light", &[("major_emergency", 1.0)], true),
            ("/obj/machinery/light/broken", &[], false),
            ("/obj/machinery/light/burned", &[], false),
            ("/obj/machinery/light/empty", &[], false),
        ];
        let mut map = Map::new(Size {
            x: fixtures.len() as u32,
            y: 1,
            z: 1,
        });
        for (index, (path, vars, _)) in fixtures.iter().enumerate() {
            let mut fixture = Prefab::new(TreePath::parse(path));
            for (name, value) in *vars {
                fixture.set_var((*name).into(), Value::Num(*value));
            }
            let tile = map.intern_tile(vec![
                fixture,
                Prefab::new(TreePath::parse("/turf/open/floor/iron")),
                Prefab::new(TreePath::parse("/area/station/engineering/main")),
            ]);
            map.grid[0][0][index] = tile;
        }
        session.activate_document(MapDocument::new(map, 1));
        settle_bake(&mut session);

        let check = |session: &Session, lighting: bool| {
            let environment = session.state.environment.as_ref().unwrap();
            let document = session.state.active_document().unwrap();
            let bake = session.active_cache().bake.as_ref().expect("fixture bake");
            assert_eq!(bake.diagnostics.count(), 0, "{:?}", bake.diagnostics.entries);
            for (index, (path, _, native_on)) in fixtures.iter().enumerate() {
                let id = document.instance_ids_at(Coord::new(index as u32 + 1, 1, 1))[0];
                let (prefab, _) = document.prefab_instance(id).unwrap();
                let ty = environment.tree.id_of(&prefab.path).unwrap();
                let delta = &bake.appearances[&id.get()];
                let owner = visual::resolve_delta(&environment.tree, ty, prefab, delta);
                // The game chooses the sheet and state; the test only checks whether it draws a glow.
                let overlay_icon = environment
                    .tree
                    .var_inherited(ty, &"overlay_icon".into())
                    .and_then(|var| var.value.as_text())
                    .expect("native fixture overlay sheet");
                let glowing = delta.overlays.iter().any(|overlay| {
                    let overlay = visual::resolve_overlay(&environment.tree, &owner, overlay);
                    overlay.icon.as_deref() == Some(overlay_icon) && overlay.lighting == vm::AppearanceLighting::Normal
                });
                assert_eq!(glowing, lighting && *native_on, "{path}, lighting={lighting}");
                assert_eq!(
                    delta
                        .overlays
                        .iter()
                        .any(|overlay| overlay.lighting == vm::AppearanceLighting::Emissive),
                    lighting && *native_on,
                    "{path}: native emissive glow, lighting={lighting}",
                );
            }
            assert_render_cache_matches_rebuild(session);
        };
        check(&session, true);
        let initial = session.active_cache().bake.as_ref().unwrap().appearances.clone();
        let frame = session.dm_ui(0, UiFeedback::default());
        let key = frame
            .commands
            .iter()
            .find_map(|command| match command {
                UiCommand::Checkbox { key, label, .. } if label == "Lighting" => Some(key.clone()),
                _ => None,
            })
            .expect("Lighting checkbox");
        for lighting in [false, true, false, true] {
            let frame = session.dm_ui(
                0,
                UiFeedback {
                    values: HashMap::from([(key.clone(), UiValue::Bool(lighting))]),
                    interacted: true,
                    ..Default::default()
                },
            );
            assert!(frame.committed);
            assert!(frame.rebake.appearance.is_some_and(|group| group != 0));
            session.dm_ui_rebake(frame.rebake);
            check(&session, lighting);
            if lighting {
                assert_eq!(session.active_cache().bake.as_ref().unwrap().appearances, initial);
            }
        }
    }

    #[test]
    #[ignore = "requires the local target/tgstation checkout"]
    fn a_hidden_layer_manifold_draws_its_connections_above_the_floor() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/tgstation");
        let options = editor::environment::BakeOptions {
            forced_profile: Some(String::from("tgstation")),
            ..Default::default()
        };
        let loaded = crate::loader::load_codebase(&root.join("tgstation.dme"), &options, &Progress::new())
            .expect("tgstation codebase");
        assert!(loaded.diagnostics.profile.is_none(), "{:?}", loaded.diagnostics.profile);
        let mut session = Session::new();
        session.apply_codebase(loaded);

        let floor = || {
            vec![
                Prefab::new(TreePath::parse("/turf/open/floor/iron")),
                Prefab::new(TreePath::parse("/area/station/engineering/main")),
            ]
        };
        let with = |path: &str, dir: Option<f32>| {
            let mut prefab = Prefab::new(TreePath::parse(path));
            if let Some(dir) = dir {
                prefab.set_var("dir".into(), Value::Num(dir));
            }
            let mut tile = vec![prefab];
            tile.extend(floor());
            tile
        };
        let pipe = "/obj/machinery/atmospherics/pipe/smart/manifold4w/cyan/hidden";
        let manifold = "/obj/machinery/atmospherics/pipe/layer_manifold/cyan/hidden";
        let size = Size { x: 5, y: 3, z: 1 };
        let mut map = Map::new(size);
        let empty = map.intern_tile(floor());
        let pipe_tile = map.intern_tile(with(pipe, None));
        let vertical = map.intern_tile(with(manifold, Some(1.0)));
        let horizontal = map.intern_tile(with(manifold, Some(8.0)));
        for y in 1..=size.y {
            for x in 1..=size.x {
                let key = match (x, y) {
                    (1, 2) => vertical,
                    (4, 2) => horizontal,
                    (1, _) | (3, 2) | (5, 2) => pipe_tile,
                    _ => empty,
                };
                map.grid[0][(size.y - y) as usize][x as usize - 1] = key;
            }
        }
        session.activate_document(MapDocument::new(map, 1));
        settle_bake(&mut session);

        let bake = session.active_cache().bake.as_ref().expect("manifold bake");
        assert_eq!(bake.diagnostics.count(), 0, "{:?}", bake.diagnostics.entries);
        let environment = session.state.environment.as_ref().unwrap();
        let document = session.state.active_document().unwrap();
        let resolved = |instance: editor::document::PrefabInstanceId| {
            let (prefab, _) = document.prefab_instance(instance).unwrap();
            let ty = environment.tree.id_of(&prefab.path).unwrap();
            let delta = bake.appearances.get(&instance.get());
            let appearance = delta.map_or_else(
                || editor::visual::resolve_id(&environment.tree, ty, prefab),
                |delta| editor::visual::resolve_delta(&environment.tree, ty, prefab, delta),
            );
            (prefab.path.to_string(), appearance, delta)
        };
        let depth = |appearance: &editor::visual::Appearance| appearance.plane * 1000.0 + appearance.layer;

        for (coord, expected) in [
            (Coord::new(1, 2, 1), ["intact_1_3", "intact_2_3"]),
            (Coord::new(4, 2, 1), ["intact_4_3", "intact_8_3"]),
        ] {
            let placed = document
                .instance_ids_at(coord)
                .iter()
                .map(|id| resolved(*id))
                .collect::<Vec<_>>();
            let (_, owner, delta) = placed.iter().find(|(path, ..)| path == manifold).unwrap();
            let (_, turf, _) = placed.iter().find(|(path, ..)| path.starts_with("/turf/")).unwrap();
            let mut states = Vec::new();
            for overlay in &delta.expect("baked manifold").overlays {
                let connection = editor::visual::resolve_overlay(&environment.tree, owner, overlay);
                let Some(state) = connection
                    .icon_state
                    .clone()
                    .filter(|state| state.starts_with("intact_"))
                else {
                    continue;
                };
                assert_eq!(
                    (connection.plane, connection.layer),
                    (owner.plane, owner.layer),
                    "{coord:?}"
                );
                assert!(depth(&connection) > depth(turf), "{coord:?} draws under its floor");
                states.push(state);
            }
            states.sort();
            assert_eq!(states, expected, "{coord:?}");
        }
    }

    #[test]
    #[ignore = "requires the local target/tgstation checkout"]
    fn lava_lights_only_where_it_borders_another_turf() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/tgstation");
        let options = editor::environment::BakeOptions {
            forced_profile: Some(String::from("tgstation")),
            ..Default::default()
        };
        let loaded = crate::loader::load_codebase(&root.join("tgstation.dme"), &options, &Progress::new())
            .expect("tgstation codebase");
        assert!(loaded.diagnostics.profile.is_none(), "{:?}", loaded.diagnostics.profile);
        let mut session = Session::new();
        session.apply_codebase(loaded);

        let lava = "/turf/open/lava/plasma";
        let area = "/area/station/engineering/main";
        let tile = |turf: &str| vec![Prefab::new(TreePath::parse(turf)), Prefab::new(TreePath::parse(area))];
        let mut map = Map::new(Size { x: 3, y: 1, z: 1 });
        let lava_tile = map.intern_tile(tile(lava));
        let floor_tile = map.intern_tile(tile("/turf/open/floor/iron"));
        map.grid[0][0] = vec![lava_tile, lava_tile, floor_tile];
        session.activate_document(MapDocument::new(map, 1));
        settle_bake(&mut session);

        let floor = Coord::new(3, 1, 1);
        let floor_light = |session: &Session| {
            let bake = session.active_cache().bake.as_ref().expect("lava bake");
            assert_eq!(bake.diagnostics.count(), 0, "{:?}", bake.diagnostics.entries);
            bake.lighting
                .as_ref()
                .and_then(|lighting| lighting.tile(vm::world::Position::new(3, 1, 1)))
                .map_or(0.0, |tile| tile.corners.iter().flatten().sum::<f32>())
        };
        assert!(floor_light(&session) > 0.0, "the lava beside the floor cast no light");

        let bake = session.active_cache().bake.as_ref().unwrap();
        let lightings = bake
            .appearances
            .values()
            .flat_map(|appearance| &appearance.overlays)
            .map(|overlay| overlay.lighting)
            .collect::<Vec<_>>();
        assert!(
            lightings.contains(&vm::AppearanceLighting::OverlayLight),
            "{lightings:?}"
        );
        assert!(lightings.contains(&vm::AppearanceLighting::Emissive), "{lightings:?}");

        session.state.choose_prefab(Prefab::new(TreePath::parse(lava)));
        session.set_tool(Tool::Place);
        assert!(session.place_at(floor, None).is_some());
        settle_bake(&mut session);
        assert_eq!(floor_light(&session), 0.0, "lava surrounded by lava kept its light");
    }

    #[test]
    #[ignore = "requires the local target/tgstation checkout"]
    fn lava_lights_the_level_above_through_openspace() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/tgstation");
        let options = editor::environment::BakeOptions {
            forced_profile: Some(String::from("tgstation")),
            ..Default::default()
        };
        let loaded = crate::loader::load_codebase(&root.join("tgstation.dme"), &options, &Progress::new())
            .expect("tgstation codebase");
        assert!(loaded.diagnostics.profile.is_none(), "{:?}", loaded.diagnostics.profile);
        let mut session = Session::new();
        session.apply_codebase(loaded);

        // Every lava tile has only lava around it on its own level, so it can only light up because
        // of the level above.
        let area = "/area/station/engineering/main";
        let tile = |turf: &str| vec![Prefab::new(TreePath::parse(turf)), Prefab::new(TreePath::parse(area))];
        let mut map = Map::new(Size { x: 3, y: 3, z: 2 });
        let lava = map.intern_tile(tile("/turf/open/lava/plasma"));
        let floor = map.intern_tile(tile("/turf/open/floor/iron"));
        let openspace = map.intern_tile(tile("/turf/open/openspace"));
        map.grid[0] = vec![vec![lava; 3]; 3];
        map.grid[1] = vec![vec![floor; 3]; 3];
        map.grid[1][1][1] = openspace;
        session.activate_document(MapDocument::new(map, 1));
        settle_bake(&mut session);

        let bake = session.active_cache().bake.as_ref().expect("lava bake");
        assert_eq!(bake.diagnostics.count(), 0, "{:?}", bake.diagnostics.entries);
        let lighting = bake.lighting.as_ref().expect("lightmap");
        let hole = lighting
            .tile(vm::world::Position::new(2, 2, 2))
            .expect("openspace tile");
        assert!(
            hole.corners.iter().all(|corner| corner.iter().sum::<f32>() > 0.0),
            "{hole:?}"
        );
        let corner = lighting.tile(vm::world::Position::new(1, 1, 2)).expect("floor tile");
        assert_eq!(corner.corners[0], [0.0; 3], "light rose past the openspace tile");
    }

    #[test]
    #[ignore = "requires the local target/tgstation checkout"]
    fn an_openspace_level_added_above_lava_lights_like_a_full_bake() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/tgstation");
        let options = editor::environment::BakeOptions {
            forced_profile: Some(String::from("tgstation")),
            ..Default::default()
        };
        let load = || {
            let loaded = crate::loader::load_codebase(&root.join("tgstation.dme"), &options, &Progress::new())
                .expect("tgstation codebase");
            let mut session = Session::new();
            session.apply_codebase(loaded);
            session
        };
        let mut session = load();

        // lava with only lava around it stays dark until the level above opens up
        let area = "/area/station/engineering/main";
        let tile = |turf: &str| vec![Prefab::new(TreePath::parse(turf)), Prefab::new(TreePath::parse(area))];
        let mut map = Map::new(Size { x: 3, y: 3, z: 1 });
        let lava = map.intern_tile(tile("/turf/open/lava/plasma"));
        map.grid[0] = vec![vec![lava; 3]; 3];
        session.activate_document(MapDocument::new(map, 1));
        settle_bake(&mut session);
        let id = session.state.active().unwrap();

        assert_eq!(session.create_level(id, &tile("/turf/open/openspace")), Ok(2));
        assert!(session.baker.extending(id));
        settle_bake(&mut session);
        assert_render_cache_matches_rebuild(&session);

        let mut whole = load();
        whole.activate_document(MapDocument::new(session.state.active_document().unwrap().to_map(), 1));
        settle_bake(&mut whole);
        let bake = session.active_cache().bake.as_ref().unwrap();
        let expected = whole.active_cache().bake.as_ref().unwrap();
        assert_eq!(bake.diagnostics.count(), 0, "{:?}", bake.diagnostics.entries);
        let lighting = bake.lighting.as_ref().expect("lightmap");
        assert!(
            lighting == expected.lighting.as_ref().unwrap(),
            "the lightmap diverged from a full bake"
        );
        assert!(
            bake.appearances == expected.appearances,
            "appearances diverged from a full bake"
        );
        let hole = lighting
            .tile(vm::world::Position::new(2, 2, 2))
            .expect("openspace tile");
        assert!(
            hole.corners.iter().all(|corner| corner.iter().sum::<f32>() > 0.0),
            "{hole:?}"
        );
    }

    #[test]
    #[ignore = "requires the local target/tgstation checkout"]
    fn a_thermomachine_rotates_its_baked_pipe_with_dir() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/tgstation");
        let options = editor::environment::BakeOptions {
            forced_profile: Some(String::from("tgstation")),
            ..Default::default()
        };
        let loaded = crate::loader::load_codebase(&root.join("tgstation.dme"), &options, &Progress::new())
            .expect("tgstation codebase");
        assert!(loaded.diagnostics.profile.is_none(), "{:?}", loaded.diagnostics.profile);
        let mut session = Session::new();
        session.apply_codebase(loaded);

        let path = TreePath::parse("/obj/machinery/atmospherics/components/unary/thermomachine");
        let mut thermomachine = Prefab::new(path.clone());
        thermomachine.set_var("dir".into(), Value::Num(1.0));
        let mut map = Map::new(Size { x: 1, y: 1, z: 1 });
        let tile = map.intern_tile(vec![
            thermomachine,
            Prefab::new(TreePath::parse("/turf/open/floor/iron")),
            Prefab::new(TreePath::parse("/area/station/engineering/main")),
        ]);
        map.grid[0][0][0] = tile;
        session.activate_document(MapDocument::new(map, 1));
        settle_bake(&mut session);

        let instance = session
            .state
            .active_document()
            .unwrap()
            .instance_ids_at(Coord::new(1, 1, 1))[0];
        let pipe_dir = |session: &Session| {
            let bake = session.active_cache().bake.as_ref().expect("thermomachine bake");
            assert_eq!(bake.diagnostics.count(), 0, "{:?}", bake.diagnostics.entries);
            let environment = session.state.environment.as_ref().unwrap();
            let document = session.state.active_document().unwrap();
            let (prefab, _) = document.prefab_instance(instance).unwrap();
            let ty = environment.tree.id_of(&prefab.path).unwrap();
            let delta = &bake.appearances[&instance.get()];
            let owner = editor::visual::resolve_delta(&environment.tree, ty, prefab, delta);

            delta
                .overlays
                .iter()
                .map(|overlay| editor::visual::resolve_overlay(&environment.tree, &owner, overlay))
                .find(|overlay| overlay.icon_state.as_deref() == Some("pipe"))
                .map(|overlay| overlay.dir)
        };

        let mut cardinals = [false; 8];
        cardinals[..4].fill(true);
        assert_eq!(session.declared_directions(&path), Some(cardinals));
        assert_eq!(pipe_dir(&session), Some(1));

        session.select_instance(Some(instance));
        session.edit_selected_instance_vars(
            "set dir",
            &[editor::document::VarMutation::Set("dir".into(), Value::Num(4.0))],
            None,
        );
        settle_bake(&mut session);
        assert_eq!(pipe_dir(&session), Some(4));
    }

    #[test]
    #[ignore = "requires the local target/SecondCity checkout"]
    fn bundled_secondcity_profile_bakes_city_maps() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/SecondCity");
        let entry = root.join("tgstation.dme");
        let options = editor::environment::BakeOptions {
            forced_profile: Some(String::from("secondcity")),
            ..Default::default()
        };
        let loaded = crate::loader::load_codebase(&entry, &options, &Progress::new()).expect("SecondCity codebase");
        assert!(
            !loaded.diagnostics.bake_preprocess.iter().any(|error| error.is_fatal()),
            "{:?}",
            loaded.diagnostics.bake_preprocess
        );
        assert!(
            loaded.diagnostics.bake_sema.is_empty(),
            "{:?}",
            loaded.diagnostics.bake_sema
        );
        assert!(loaded.diagnostics.codegen.is_none(), "{:?}", loaded.diagnostics.codegen);
        assert!(loaded.diagnostics.profile.is_none(), "{:?}", loaded.diagnostics.profile);
        assert_eq!(
            loaded
                .environment
                .profiles
                .as_ref()
                .map(|profiles| profiles.active.as_str()),
            Some("/datum/demir/secondcity")
        );
        assert!(loaded.environment.bake_program.is_some());

        let mut session = Session::new();
        session.apply_codebase(loaded);

        // A lone city wall under an outdoor floor: the wall stands its frill on that floor, and the
        // floor is moonlit. Beside the wall, a low wall builds its window under a patch of grass.
        let area = Prefab::new(TreePath::parse("/area/vtm/outside"));
        let concrete = Prefab::new(TreePath::parse("/turf/open/floor/plating/concrete"));
        let mut city_map = Map::new(Size { x: 2, y: 2, z: 2 });
        let wall = city_map.intern_tile(vec![
            Prefab::new(TreePath::parse("/turf/closed/wall/vampwall/city")),
            area.clone(),
        ]);
        let floor = city_map.intern_tile(vec![concrete.clone(), area.clone()]);
        let low_wall = city_map.intern_tile(vec![
            Prefab::new(TreePath::parse("/obj/structure/platform/lowwall/city/window")),
            concrete,
            area.clone(),
        ]);
        let grass = city_map.intern_tile(vec![
            Prefab::new(TreePath::parse("/turf/open/misc/grass")),
            area.clone(),
        ]);
        let ash = city_map.intern_tile(vec![
            Prefab::new(TreePath::parse("/turf/open/misc/ashplanet/ash")),
            area,
        ]);
        // Rows run from the top, so the floor is north of the wall.
        city_map.grid[0][0] = vec![floor, grass];
        city_map.grid[0][1] = vec![wall, low_wall];
        // Large smoothing turfs above the first level, which the default level traits don't cover.
        city_map.grid[1][0] = vec![grass, grass];
        city_map.grid[1][1] = vec![ash, ash];
        session.activate_document(MapDocument::new(city_map, 1));
        settle_bake(&mut session);
        let city_bake = session.active_cache().bake.as_ref().expect("city wall bake");
        assert_eq!(city_bake.diagnostics.count(), 0, "{:?}", city_bake.diagnostics.entries);
        let smoothed = |sheet: &str, state: &str| {
            city_bake.appearances.values().any(|appearance| {
                let text = |var: &str| {
                    appearance
                        .vars
                        .iter()
                        .find(|(name, _)| name.as_str() == var)
                        .and_then(|(_, value)| value.as_text())
                };

                text("icon").is_some_and(|icon| icon.ends_with(sheet))
                    && text("icon_state").is_some_and(|icon_state| icon_state.starts_with(state))
            })
        };
        assert!(
            smoothed("floor/grass.dmi", "grass-"),
            "the grass did not smooth from its junction sheet"
        );
        assert!(
            smoothed("floors/ash.dmi", "ash-"),
            "the ash did not smooth from its junction sheet"
        );
        assert!(
            city_bake
                .appearances
                .values()
                .flat_map(|appearance| &appearance.overlays)
                .any(|overlay| overlay
                    .vars
                    .iter()
                    .any(|(name, value)| name.as_str() == "pixel_y" && value.as_num() == Some(32.0))),
            "the city wall drew no frill"
        );
        assert!(
            city_bake
                .appearances
                .values()
                .flat_map(|appearance| &appearance.overlays)
                .any(
                    |overlay| overlay.vars.iter().any(|(name, value)| name.as_str() == "icon_state"
                        && value.as_text().is_some_and(|state| state.starts_with("window-")))
                ),
            "the low wall drew no window"
        );
        assert!(
            city_bake
                .lighting
                .as_ref()
                .is_some_and(|lighting| lighting.tiles.iter().any(|tile| tile
                    .corners
                    .iter()
                    .flatten()
                    .any(|channel| *channel > 0.0))),
            "the outdoor floor was not moonlit"
        );

        for name in ["runtimetown.dmm", "san_fangsisco/sanfangsisco.dmm"] {
            session
                .open_map(&root.join("_maps/map_files/Vampire").join(name), 1)
                .expect("SecondCity map");
            settle_bake(&mut session);
            let bake = session.active_cache().bake.as_ref().expect("completed map bake");
            assert!(bake.succeeded > 0, "{name} baked no atoms");

            let faults = bake
                .diagnostics
                .entries
                .iter()
                .map(|entry| {
                    let file = session
                        .state
                        .environment
                        .as_ref()
                        .and_then(|environment| environment.bake_file(entry.fault.location.file));
                    format!(
                        "{} atoms: {:?} at {}",
                        entry.count,
                        entry.fault.kind,
                        entry.fault.location.display(file)
                    )
                })
                .collect::<Vec<_>>();
            assert_eq!(bake.diagnostics.count(), 0, "{name}: {faults:#?}");
        }
    }

    // Times each phase of one placement on the largest SecondCity map, and reports what the heap
    // holds along the way. Run with `--nocapture`.
    #[test]
    #[ignore = "requires the local target/SecondCity checkout"]
    fn secondcity_placement_timings() {
        use std::time::Instant;

        use editor::command::Edit;

        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/SecondCity");
        let options = editor::environment::BakeOptions {
            forced_profile: Some(String::from("secondcity")),
            ..Default::default()
        };
        let megabytes = |bytes: usize| bytes as f64 / (1024.0 * 1024.0);
        let report = |label: &str| {
            eprintln!(
                "{label}: {:.0} MB live, {:.0} MB peak",
                megabytes(live_bytes()),
                megabytes(take_peak_bytes())
            );
        };
        eprintln!(
            "GenericValue {} B, AppearanceDelta {} B, SpriteInstance {} B",
            size_of::<vm::GenericValue>(),
            size_of::<vm::AppearanceDelta>(),
            size_of::<render::SpriteInstance>()
        );
        take_peak_bytes();

        let loaded = crate::loader::load_codebase(&root.join("tgstation.dme"), &options, &Progress::new())
            .expect("SecondCity codebase");
        let mut session = Session::new();
        session.apply_codebase(loaded);
        report("codebase loaded");

        session
            .open_map(&root.join("_maps/map_files/Vampire/san_fangsisco/sanfangsisco.dmm"), 1)
            .expect("San Fangsisco");
        report("map opened");

        let started = Instant::now();
        while session.baker.is_busy() {
            session.poll_bake();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        eprintln!("full bake settled in {:?}", started.elapsed());
        report("bake landed");

        let id = session.state.active().expect("active map");
        let open_floors = {
            let document = session.state.active_document().unwrap();
            let size = document.map.size();
            (1..=size.z)
                .flat_map(|z| (1..=size.y).flat_map(move |y| (1..=size.x).map(move |x| Coord::new(x, y, z))))
                .filter(|coord| {
                    document.map.tile_at(*coord).is_some_and(|tile| {
                        tile.iter()
                            .any(|prefab| prefab.path.to_string().starts_with("/turf/open/floor"))
                    })
                })
                .collect::<Vec<_>>()
        };
        eprintln!("{} open floors", open_floors.len());
        let open_floor = |quarter: usize| open_floors[open_floors.len() * quarter / 4];

        let place = |session: &mut Session, label: &str, coord: Coord, replace_turf: bool, path: &str| {
            let document = session.state.active_document_mut().unwrap();
            let mut after = document.placed_tile(coord).expect("placed tile");
            let placed = document.instantiate(Prefab::new(TreePath::parse(path)));
            let mut affected = vec![placed.id()];
            if replace_turf {
                let index = after
                    .iter()
                    .position(|placed| placed.prefab().path.to_string().starts_with("/turf/"))
                    .expect("a turf");
                affected.push(after.remove(index).id());
                after.insert(index, placed);
            } else {
                after.push(placed);
            }

            let mut edit = Edit::new("place");
            edit.change(document, coord, after);

            let clock = Instant::now();
            assert!(document.apply(edit));
            let apply = clock.elapsed();

            let clock = Instant::now();
            let update = {
                let Session { state, caches, .. } = &mut *session;
                let bake = caches.get_mut(&id).and_then(|cache| cache.bake.as_mut()).expect("bake");
                editor::bake::update(
                    bake,
                    state.environment.as_ref().unwrap(),
                    state.document(id).unwrap(),
                    &affected,
                )
            };
            let bake = clock.elapsed();
            let changed = update.appearances.len();
            let lighting = update.lighting.as_ref().map_or(0, |range| range.len());

            let clock = Instant::now();
            session.apply_bake_update(id, update);
            let frame = clock.elapsed();
            let cache = session.active_cache();
            let uploaded = cache
                .frame_updates
                .last()
                .iter()
                .flat_map(|update| &update.sprites)
                .map(|range| range.end - range.start)
                .sum::<usize>();

            eprintln!(
                "{label}: apply {apply:?}, bake {bake:?} ({changed} appearances, {lighting} light tiles), frame \
                 {frame:?} ({uploaded} of {} sprites uploaded)",
                cache.instances.sprites.len()
            );
        };

        place(
            &mut session,
            "object",
            open_floor(1),
            false,
            "/obj/structure/table/wood",
        );
        place(
            &mut session,
            "wall",
            open_floor(2),
            true,
            "/turf/closed/wall/vampwall/city",
        );

        let bake = session.caches.get_mut(&id).and_then(|cache| cache.bake.as_mut());
        let lighting = bake.and_then(|bake| bake.lighting.take());
        assert!(lighting.is_some(), "the map was lit");

        place(
            &mut session,
            "wall, lighting off",
            open_floor(3),
            true,
            "/turf/closed/wall/vampwall/city",
        );
        report("placed");

        // what each part holds, measured by what freeing it gives back
        let mut last = live_bytes();
        let mut freed = |label: &str| {
            let now = live_bytes();
            eprintln!("  {label}: {:.1} MB", megabytes(last.saturating_sub(now)));
            last = now;
        };
        let cache = session.caches.get_mut(&id).expect("map cache");
        let instances = std::mem::take(&mut cache.instances);
        let bake = cache.bake.take().expect("bake");
        eprintln!("frame cache:");
        instances.release_in_stages(&mut freed);
        eprintln!("bake:");
        bake.release_in_stages(&mut freed);
        drop(lighting);
        freed("lighting map, taken earlier");
        eprintln!("session:");
        session.caches.clear();
        freed("other document caches");
        drop(session.state.close_document(id));
        freed("map document and history");
        session.state.environment = None;
        freed("codebase environment");
        drop(session);
        freed("rest of the session");
    }

    #[test]
    fn a_map_draws_before_its_bake_lands() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).expect("codebase");
        session.open_map(&root.join("test.dmm"), 1).expect("map");

        assert!(session.baker.is_busy());
        assert!(
            session
                .instances()
                .is_some_and(|instances| instances.live_sprites().next().is_some())
        );
        assert!(session.active_cache().bake.is_none());

        settle_bake(&mut session);

        assert!(session.active_cache().bake.is_some());
        assert_render_cache_matches_rebuild(&session);
    }

    #[test]
    fn an_edit_while_a_bake_is_running_throws_its_result_away_and_bakes_again() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).expect("codebase");
        session.open_map(&root.join("test.dmm"), 1).expect("map");
        let id = session.state.active().expect("active map");
        let coord = Coord::new(6, 3, 1);
        let target = session
            .state
            .active_document()
            .and_then(|document| document.instance_ids_at(coord).first())
            .copied()
            .expect("an instance to delete");

        session.set_tool(Tool::Delete);
        session.select_instance(Some(target));

        assert!(session.delete_instance(target));
        assert!(session.baker.invalidated(id));

        settle_bake(&mut session);

        // the bake that lands is the one that ran after the delete, so it knows nothing of the atom
        assert!(
            session
                .active_cache()
                .bake
                .as_ref()
                .is_some_and(|bake| bake.position(target.get()).is_none())
        );
    }
}
