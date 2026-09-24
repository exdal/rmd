use editor::{
    document::DocumentId,
    frame::{self, FrameRenderOptions, PrefabUpdate},
};
use render::FrameUpdate;

use super::{Session, always_highlighted};
use crate::baker::{self};

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
        self.caches.entry(id).or_default().bake = None;
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

            self.baker.start(baker::Request {
                document: id,
                path: document
                    .path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default(),
                atoms: editor::bake::atoms(&environment, document),
                size: editor::bake::size(document),
                environment,
            });
        }
    }

    pub fn poll_bake(&mut self) {
        if let Some((id, mut bake, outdated)) = self.baker.poll() {
            let open = self.state.document(id).is_some();
            if outdated && open {
                self.rebake(id);
            } else if open {
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
        }

        self.start_next_bake();
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
            cache.lighting_update = Some(render::LightingUpdate {
                previous_revision,
                tiles: render::UpdateRange {
                    start: range.start,
                    end: range.end,
                },
            });
        }

        match update {
            PrefabUpdate::Unchanged => {},
            PrefabUpdate::Buffers { sprites, area_tiles } => {
                let previous_revision = cache.revision;
                cache.revision = *next_revision;
                *next_revision = next_revision.wrapping_add(1).max(1);
                cache.frame_update = Some(FrameUpdate {
                    previous_revision,
                    sprites,
                    area_tiles,
                });
            },
            PrefabUpdate::Rebuild => self.rebuild_instances(id),
        }
        self.revalidate_focus();
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
        fixtures::{assert_render_cache_matches_rebuild, examples, settle_bake},
    };

    #[test]
    #[ignore = "requires the local target/MonkeStation2.0 checkout"]
    fn bundled_monkestation_profile_bakes_debug_maps() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/MonkeStation2.0");
        let entry = root.join("tgstation.dme");
        let options = editor::environment::BakeOptions {
            forced_profile: Some(editor::environment::BundledProfile::Monkestation),
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
    fn a_map_draws_before_its_bake_lands() {
        let root = examples();
        let mut session = Session::new();
        session.load_environment(&root.join("test.dme")).expect("codebase");
        session.open_map(&root.join("test.dmm"), 1).expect("map");

        assert!(session.baker.is_busy());
        assert!(
            session
                .instances()
                .is_some_and(|instances| !instances.sprites.is_empty())
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
