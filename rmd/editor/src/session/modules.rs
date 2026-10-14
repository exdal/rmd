use std::{collections::HashSet, mem};

use dmm::Prefab;
use editor::{
    bake::{self, BakeUpdate},
    document::{DocumentId, PrefabInstanceId},
    maploader::preview::{
        ComposeContext,
        MapView,
        ModulePreviewState,
        ModuleRootKey,
        ModuleRootPreview,
        PREVIEW_ID_START,
    },
    visual,
};

use super::{ModuleBake, PlacementPreview, Session, bake::report_bake_output, prefab_thumbnail_or_missing};

#[cfg(test)]
pub(super) mod tests;

impl Session {
    pub(super) fn effective_grid(&self, id: DocumentId) -> Option<MapView<'_>> {
        let source = &self.state.document(id)?.map;

        Some(
            self.caches
                .get(&id)
                .map_or_else(|| source.into(), |cache| cache.modules.view(source)),
        )
    }

    fn with_modules<R>(
        &mut self, id: DocumentId, run: impl FnOnce(&mut ModulePreviewState, ComposeContext<'_>) -> R,
    ) -> Option<R> {
        let Self { state, caches, .. } = self;
        let environment = state.environment.as_ref()?;
        let document = state.document(id)?;
        let cache = caches.entry(id).or_default();

        Some(run(
            &mut cache.modules,
            ComposeContext {
                source: &document.map,
                tree: &environment.tree,
                base_dir: environment.base_dir(),
                document_path: document.path.as_deref(),
            },
        ))
    }

    pub(super) fn sync_module_guard(&mut self, id: DocumentId) {
        let Some(document) = self.state.document(id) else {
            return;
        };
        let preview = self
            .caches
            .get(&id)
            .and_then(|cache| cache.modules.preview.as_ref().filter(|_| cache.modules.enabled));
        let covered = preview.map(|preview| preview.covered.clone()).unwrap_or_default();
        let exempt = preview
            .into_iter()
            .flat_map(|preview| &preview.roots)
            .filter(|root| root.key.ancestry.is_empty() && document.prefab_instance(root.key.instance).is_some())
            .map(|root| root.key.instance)
            .collect();
        if let Some(document) = self.state.document_mut(id) {
            document.set_preview_covered(covered, exempt);
        }
    }

    pub(super) fn sync_module_source(
        &mut self, id: DocumentId, affected: &[PrefabInstanceId],
    ) -> Option<Vec<PrefabInstanceId>> {
        let changed = self
            .with_modules(id, |modules, ctx| modules.source_changed(ctx, affected))
            .flatten();
        self.sync_module_guard(id);

        changed
    }

    pub(super) fn settle_modules(&mut self, id: DocumentId, changed: Vec<PrefabInstanceId>) {
        self.settle_edited_modules(id, &[], changed);
    }

    // `edited` are the document's own changes, the rest of `changed` only follows the composite
    pub(super) fn settle_edited_modules(
        &mut self, id: DocumentId, edited: &[PrefabInstanceId], changed: Vec<PrefabInstanceId>,
    ) {
        let Self {
            state, caches, baker, ..
        } = self;
        let cache = caches.entry(id).or_default();
        let update = match (cache.bake.as_mut(), state.environment.as_ref(), state.document(id)) {
            (Some(bake), Some(environment), Some(document))
                if let ModuleBake::Deferred { pending } = &mut cache.module_bake =>
            {
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
            (Some(bake), Some(environment), Some(document)) => {
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
            _ => {
                if baker.extending(id) {
                    cache.unbaked.extend_from_slice(&changed);
                } else if !changed.is_empty() {
                    baker.invalidate(id);
                }

                BakeUpdate {
                    appearances: changed,
                    lighting: None,
                }
            },
        };

        self.sync_module_guard(id);
        self.apply_bake_update(id, update);
    }
}

impl Session {
    pub fn set_module_bake_deferred(&mut self, is_deferred: bool) {
        let Some(id) = self.state.active() else {
            return;
        };

        self.set_document_module_bake_deferred(id, is_deferred);
    }

    pub(super) fn set_document_module_bake_deferred(&mut self, id: DocumentId, is_deferred: bool) {
        let Some(cache) = self.caches.get_mut(&id) else {
            return;
        };

        match (&cache.module_bake, is_deferred) {
            (ModuleBake::Live, true) => {
                cache.module_bake = ModuleBake::Deferred {
                    pending: HashSet::new(),
                };
            },
            (ModuleBake::Deferred { .. }, false) => {
                let ModuleBake::Deferred { pending } = mem::take(&mut cache.module_bake) else {
                    return;
                };
                let mut pending = pending.into_iter().collect::<Vec<_>>();
                pending.sort_unstable();

                self.settle_modules(id, pending);
            },
            _ => {},
        }
    }

    pub(crate) fn module_marker(&self, prefab: &Prefab) -> Option<PlacementPreview> {
        let environment = self.state.environment.as_ref()?;
        let appearance = visual::resolve(&environment.tree, prefab);

        Some(PlacementPreview {
            thumbnail: prefab_thumbnail_or_missing(&self.textures, environment, &appearance)?,
            offset: [
                appearance
                    .step_x
                    .saturating_add(appearance.pixel_x)
                    .saturating_add(appearance.pixel_w),
                appearance
                    .step_y
                    .saturating_add(appearance.pixel_y)
                    .saturating_add(appearance.pixel_z),
            ],
        })
    }

    pub fn module_roots(&self) -> &[ModuleRootPreview] {
        self.state
            .active()
            .and_then(|id| self.caches.get(&id))
            .and_then(|cache| cache.modules.preview.as_ref())
            .map_or(&[], |preview| &preview.roots)
    }

    pub fn is_source_root(&self, root: &ModuleRootPreview) -> bool {
        root.key.ancestry.is_empty()
            && self
                .state
                .active_document()
                .is_some_and(|document| document.prefab_instance(root.key.instance).is_some())
    }

    pub fn modules_enabled(&self) -> bool {
        self.state
            .active()
            .and_then(|id| self.caches.get(&id))
            .is_none_or(|cache| cache.modules.enabled)
    }

    pub fn set_modules_enabled(&mut self, is_enabled: bool) {
        let Some(id) = self.state.active() else {
            return;
        };
        let Some(document) = self.state.document(id) else {
            return;
        };
        let changed = self
            .caches
            .entry(id)
            .or_default()
            .modules
            .set_enabled(&document.map, is_enabled);

        self.settle_modules(id, changed);
    }

    pub fn choose_module_variant(&mut self, key: &ModuleRootKey, index: usize) {
        let Some(id) = self.state.active() else {
            return;
        };
        let Some(changed) = self.with_modules(id, |modules, ctx| modules.choose(ctx, key, index)) else {
            return;
        };

        self.settle_modules(id, changed);
    }
}
