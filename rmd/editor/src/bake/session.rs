use std::{collections::HashSet, mem};

use render::FrameUpdate;
use vm::profile::ProfileHook;

use crate::{
    bake::{
        self, BakeUpdate,
        worker::{self, Baked, BakedMap, Job},
    },
    document::{DocumentId, PrefabInstanceId},
    frame::{self, PrefabUpdate, options::FrameRenderOptions},
    maploader::preview::PREVIEW_ID_START,
};
use crate::{
    session::{BakeHold, Session},
    viewport::highlight::always_highlighted,
};

// updates the renderer may not have drawn yet, so a frame that missed some can still catch up
pub(crate) const MAX_PENDING_UPDATES: usize = 8;

fn push_pending<T>(updates: &mut Vec<T>, update: T) {
    if updates.len() == MAX_PENDING_UPDATES {
        updates.remove(0);
    }

    updates.push(update);
}

pub(crate) fn report_bake_output(bake: &mut crate::bake::Bake) {
    for line in bake.take_output() {
        log::info!("DM: {line}");
    }
}

pub(crate) struct RuntimeFault<'a> {
    /// `None` for faults outside any hook, like a map var that is not a constant
    pub hook: Option<ProfileHook>,
    pub fault: &'a vm::Fault,
    pub count: usize,
    pub state: FaultState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FaultState {
    Occurring,
    Stopped,
}

// the bake only tallies what its current state raises, a placement that re-runs takes its fault
// back, so this keeps every fault a map raised until the user clears it
#[derive(Debug)]
pub(crate) struct LoggedFault {
    hook: Option<ProfileHook>,
    key: vm::FaultKey,
    fault: vm::Fault,
    count: usize,
    state: FaultState,
}

fn update_fault_log(bake: &crate::bake::Bake, log: &mut Vec<LoggedFault>) {
    for logged in log.iter_mut() {
        logged.state = FaultState::Stopped;
    }

    let raised = bake
        .diagnostics
        .entries
        .iter()
        .filter(|entry| entry.count > 0)
        .map(|entry| (bake.hook_of(&entry.fault), &entry.fault, entry.count))
        .chain(bake.ui_fault.iter().map(|fault| (Some(ProfileHook::Ui), fault, 1)));
    for (hook, fault, count) in raised {
        let key = fault.key();
        match log.iter_mut().find(|logged| logged.hook == hook && logged.key == key) {
            Some(logged) => {
                logged.count = count;
                logged.state = FaultState::Occurring;
            },
            None => log.push(LoggedFault {
                hook,
                key,
                fault: fault.clone(),
                count,
                state: FaultState::Occurring,
            }),
        }
    }
}

impl Session {
    // a map whose bake is still in flight keeps its log as it was
    pub(crate) fn log_runtimes(&mut self) {
        for cache in self.caches.values_mut() {
            if let Some(bake) = cache.bake.as_ref() {
                update_fault_log(bake, &mut cache.runtimes);
            }
        }
    }

    pub(crate) fn clear_runtimes(&mut self) {
        for cache in self.caches.values_mut() {
            cache.runtimes.clear();
        }

        self.log_runtimes();
    }

    pub(crate) fn collect_bake_diagnostics(&mut self) {
        self.log_runtimes();

        self.diagnostics.bake = self
            .caches
            .values()
            .filter_map(|cache| cache.bake.as_ref())
            .flat_map(|bake| bake.diagnostics.entries.iter())
            .filter(|entry| entry.count > 0)
            .cloned()
            .collect();

        let Some(environment) = &self.state.environment else {
            return;
        };

        for entry in &self.diagnostics.bake {
            if self.logged_faults.insert(entry.fault.key()) {
                log::warn!("DM: {}", environment.fault_report(&entry.fault));
            }
        }
    }

    pub(crate) fn rebake_all(&mut self) {
        for id in self.state.document_ids() {
            self.rebake(id);
        }
    }

    pub(crate) fn rebake(&mut self, id: DocumentId) {
        // an extension in flight carries the old bake back, which this replaces
        self.baker.invalidate(id);
        let cache = self.caches.entry(id).or_default();
        cache.bake = None;
        cache.bake_hold = BakeHold::Off;
        cache.unbaked.clear();
        cache.modules.reset();
        self.sync_module_guard(id);
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
            let cache = self.caches.entry(id).or_default();
            let behind = cache.bake.take_if(|bake| crate::bake::levels(bake) < level_count);
            let job = match behind {
                Some(bake) => Job::Extend {
                    atoms: crate::bake::atoms_from_view(
                        &environment,
                        cache.modules.view(&document.map),
                        crate::bake::levels(&bake) + 1,
                    ),
                    bake: Box::new(bake),
                    level_count,
                },
                None => Job::Build {
                    source: document.map.clone(),
                    path: document.path.clone(),
                    modules: Box::new(cache.modules.fresh()),
                },
            };

            self.baker.start(worker::Request {
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
    pub(crate) fn bake_new_levels(&mut self, id: DocumentId) {
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
            .is_some_and(|bake| crate::bake::levels(bake) < level_count);
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
                    Baked::Build(built) => self.land_bake(id, built.map(|built| *built)),
                    Baked::Extend(bake, update) => self.land_extension(id, *bake, update),
                }
            }
        }

        self.start_next_bake();
    }

    fn land_extension(&mut self, id: DocumentId, mut bake: crate::bake::Bake, mut update: BakeUpdate) {
        report_bake_output(&mut bake);
        let cache = self.caches.entry(id).or_default();
        let unbaked = std::mem::take(&mut cache.unbaked);
        cache.bake = Some(bake);

        if let (Some(environment), Some(document), Some(bake)) = (
            self.state.environment.as_ref(),
            self.state.document(id),
            cache.bake.as_mut(),
        ) {
            // levels added since it left get their own extension
            let edits =
                crate::bake::update_known_levels(bake, environment, cache.modules.view(&document.map), &unbaked);
            update.merge(edits);
            update.merge(crate::bake::settle_modules(
                bake,
                environment,
                &document.map,
                document.path.as_deref(),
                &mut cache.modules,
            ));
            report_bake_output(bake);
        }

        // keep the order consistent below

        self.sync_module_guard(id);

        let is_replayed = self
            .ui_feedback
            .clone()
            .is_some_and(|feedback| self.replay_ui(id, &feedback));

        self.collect_bake_diagnostics();
        self.apply_bake_update(id, update);
        self.resize_lighting(id);

        if is_replayed {
            self.settle_modules(id, Vec::new());
        }

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

    fn land_bake(&mut self, id: DocumentId, built: Option<BakedMap>) {
        let Self { state, caches, .. } = self;
        let cache = caches.entry(id).or_default();
        cache.bake_hold = BakeHold::Off;
        if let Some(BakedMap { mut bake, modules }) = built {
            report_bake_output(&mut bake);
            let is_enabled = cache.modules.enabled;
            cache.modules = modules;
            // previews switched while the bake ran
            if let (Some(environment), Some(document)) = (state.environment.as_ref(), state.document(id)) {
                let changed = cache.modules.set_enabled(&document.map, is_enabled);
                crate::bake::update_known_levels(&mut bake, environment, cache.modules.view(&document.map), &changed);
                report_bake_output(&mut bake);
            }

            cache.bake = Some(bake);
        } else {
            cache.bake = None;
        }

        self.sync_module_guard(id);
        let cache = self.caches.entry(id).or_default();
        cache.pending_rebake = crate::bake::UiRebake::default();
        // a bake starts from the profile's own defaults, so it has to be told what the
        // panel already asked for before it is first shown
        if let Some(feedback) = self.ui_feedback.clone() {
            self.replay_ui(id, &feedback);
        }

        let cache = self.caches.entry(id).or_default();
        cache.always_highlights = always_highlighted(cache.bake.as_ref());
        self.collect_bake_diagnostics();
        self.rebuild_instances(id);
        self.settle_modules(id, Vec::new());
        self.flush_pending_rebake(id);
    }

    pub fn bake_view(&self) -> Option<crate::environment::worker::LoadView> {
        self.baker.view()
    }

    /// Faiults __PER MAP__, keep this in mind: setup faults first and then by hook
    pub(crate) fn runtime_faults(&self) -> Vec<(DocumentId, Vec<RuntimeFault<'_>>)> {
        self.state
            .document_ids()
            .into_iter()
            .filter_map(|id| {
                let mut faults = self
                    .caches
                    .get(&id)?
                    .runtimes
                    .iter()
                    .map(|logged| RuntimeFault {
                        hook: logged.hook,
                        fault: &logged.fault,
                        count: logged.count,
                        state: logged.state,
                    })
                    .collect::<Vec<_>>();
                faults.sort_by_key(|fault| fault.hook.map_or(0, |hook| hook as usize + 1));

                (!faults.is_empty()).then_some((id, faults))
            })
            .collect()
    }

    pub(crate) fn apply_bake_update(&mut self, id: DocumentId, bake_update: crate::bake::BakeUpdate) {
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
            (Some(environment), Some(document)) => frame::update_view_prefabs_with_options(
                &mut cache.instances,
                &environment.tree,
                &environment.icons,
                textures,
                cache.modules.view(&document.map),
                &affected,
                FrameRenderOptions {
                    perspective: environment.world.perspective,
                    node_types: Some(&cache.node_types),
                    visibility: type_visibility,
                    tile_size: options.tile_size,
                    appearances: crate::bake::appearances(cache.bake.as_ref()),
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

    pub(crate) fn bake_edits(&mut self, id: DocumentId, edited: &[PrefabInstanceId], changed: Vec<PrefabInstanceId>) {
        let update = self.bake_update(id, edited, changed);
        self.sync_module_guard(id);
        self.apply_bake_update(id, update);
    }

    // `edited` are the document's own changes, the rest of `changed` only follows the composite
    fn bake_update(
        &mut self, id: DocumentId, edited: &[PrefabInstanceId], changed: Vec<PrefabInstanceId>,
    ) -> BakeUpdate {
        let Self {
            state, caches, baker, ..
        } = self;
        let cache = caches.entry(id).or_default();
        let (Some(bake), Some(environment), Some(document)) =
            (cache.bake.as_mut(), state.environment.as_ref(), state.document(id))
        else {
            if baker.extending(id) {
                cache.unbaked.extend_from_slice(&changed);
            } else if !changed.is_empty() {
                baker.invalidate(id);
            }

            return BakeUpdate {
                appearances: changed,
                lighting: None,
            };
        };

        match &mut cache.bake_hold {
            BakeHold::Off => {
                let mut update =
                    bake::update_known_levels(bake, environment, cache.modules.view(&document.map), &changed);
                update.merge(bake::settle_modules(
                    bake,
                    environment,
                    &document.map,
                    document.path.as_deref(),
                    &mut cache.modules,
                ));
                report_bake_output(bake);

                update
            },
            BakeHold::Modules { pending } => {
                let sources = edited
                    .iter()
                    .copied()
                    .filter(|owner| owner.get() < PREVIEW_ID_START)
                    .collect::<Vec<_>>();
                pending.extend(changed.iter().copied().filter(|owner| !sources.contains(owner)));

                let mut update =
                    bake::update_known_levels(bake, environment, cache.modules.view(&document.map), &sources);
                update.merge(BakeUpdate {
                    appearances: changed,
                    lighting: None,
                });
                report_bake_output(bake);

                update
            },
            BakeHold::Everything { pending } => {
                pending.extend(changed.iter().copied());

                BakeUpdate {
                    appearances: changed,
                    lighting: None,
                }
            },
        }
    }

    pub(crate) fn hold_bake(&mut self, id: DocumentId) {
        let Some(cache) = self.caches.get_mut(&id) else {
            return;
        };

        let pending = match mem::take(&mut cache.bake_hold) {
            BakeHold::Off => HashSet::default(),
            BakeHold::Modules { pending } | BakeHold::Everything { pending } => pending,
        };
        cache.bake_hold = BakeHold::Everything { pending };
    }

    pub(crate) fn release_bake(&mut self, id: DocumentId) {
        let Some(cache) = self.caches.get_mut(&id) else {
            return;
        };

        let BakeHold::Everything { pending } = &mut cache.bake_hold else {
            return;
        };

        let mut pending = mem::take(pending).into_iter().collect::<Vec<_>>();
        cache.bake_hold = BakeHold::Off;
        pending.sort_unstable();

        self.settle_modules(id, pending);
    }

    pub(crate) fn publish_frame_update(&mut self, id: DocumentId, update: PrefabUpdate) {
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
        }
    }
}

#[cfg(test)]
mod tests;
