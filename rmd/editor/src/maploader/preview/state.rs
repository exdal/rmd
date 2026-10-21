use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use dmm::{Coord, PrefabInstanceId, Size};

use super::{
    ComposeContext, MapView, ModuleChoices, ModuleOverlay, ModuleRootKey, PreviewIdAllocator, compose::update,
};
use crate::{
    grid::Grid,
    maploader::{ModuleFiles, OpenModule, RootSpecs},
};

pub struct ModulePreviewState {
    pub enabled: bool,
    pub choices: ModuleChoices,
    pub preview: Option<ModuleOverlay>,
    pub specs: RootSpecs,
    pub(super) ids: PreviewIdAllocator,
    files: ModuleFiles,
    covered_source: HashMap<PrefabInstanceId, Coord>,
    size: Option<Size>,
}

impl Default for ModulePreviewState {
    fn default() -> Self {
        Self {
            enabled: true,
            choices: ModuleChoices::new(),
            preview: None,
            specs: RootSpecs::default(),
            ids: PreviewIdAllocator::default(),
            files: ModuleFiles::default(),
            covered_source: HashMap::new(),
            size: None,
        }
    }
}

impl ModulePreviewState {
    pub fn view<'a>(&'a self, source: &'a Grid) -> MapView<'a> {
        self.preview
            .as_ref()
            .filter(|_| self.enabled)
            .map_or_else(|| source.into(), |preview| preview.view(source))
    }

    pub fn fresh(&self) -> Self {
        Self {
            enabled: self.enabled,
            choices: self.choices.clone(),
            files: self.files.open_only(),
            ..Self::default()
        }
    }

    pub fn replace_module(
        &mut self, ctx: ComposeContext<'_>, path: &Path, open: Option<OpenModule>,
    ) -> Vec<PrefabInstanceId> {
        self.files.set_open(path, open);

        self.compose(ctx, &HashSet::new())
    }

    pub fn has_module_files(&self) -> bool {
        !self.files.is_empty() || self.preview.as_ref().is_some_and(|preview| !preview.layers.is_empty())
    }

    pub fn uses_file(&self, path: &Path) -> bool {
        self.files.is_loaded(path) || self.preview.as_ref().is_some_and(|preview| preview.uses_file(path))
    }

    pub fn open_module(&self, path: &Path) -> Option<&OpenModule> {
        self.files.open_module(path)
    }

    pub fn open_modules(&self) -> impl Iterator<Item = (&Path, &OpenModule)> {
        self.files.open_modules()
    }

    pub fn needs_recompose(&self, specs: &RootSpecs) -> bool {
        self.specs != *specs
    }

    pub fn reset(&mut self) {
        self.preview = None;
        self.specs = RootSpecs::default();
        self.ids = PreviewIdAllocator::default();
        self.files.clear();
        self.covered_source.clear();
        self.size = None;
    }

    pub fn recompose(
        &mut self, ctx: ComposeContext<'_>, specs: RootSpecs, dirty: &HashSet<Coord>,
    ) -> Vec<PrefabInstanceId> {
        self.specs = specs;

        self.compose(ctx, dirty)
    }

    fn compose(&mut self, ctx: ComposeContext<'_>, dirty: &HashSet<Coord>) -> Vec<PrefabInstanceId> {
        let next = update(
            ctx,
            &self.specs,
            &mut self.files,
            &self.choices,
            &mut self.ids,
            self.preview.as_ref(),
            dirty,
        );
        let changed = ModuleOverlay::changes(
            ctx.source,
            self.preview.as_ref().filter(|_| self.enabled),
            self.enabled.then_some(&next),
        );

        self.choices = next.choices.clone();
        self.covered_source = next
            .covered
            .iter()
            .flat_map(|coord| ctx.source.ids_at(*coord).iter().map(|id| (*id, *coord)))
            .collect();
        self.preview = Some(next);
        self.size = Some(ctx.source.size());

        changed
    }

    pub fn source_changed(
        &mut self, ctx: ComposeContext<'_>, affected: &[PrefabInstanceId],
    ) -> Option<Vec<PrefabInstanceId>> {
        let source = ctx.source;
        let is_affected = affected.iter().any(|id| {
            self.covered_source.contains_key(id)
                || self.specs.atoms.contains_key(id)
                || source.location(*id).is_some_and(|location| {
                    self.preview
                        .as_ref()
                        .is_some_and(|preview| preview.covered.contains(&location.coord))
                })
        });

        if !is_affected && self.size == Some(source.size()) {
            return None;
        }

        let dirty = affected
            .iter()
            .flat_map(|id| {
                self.covered_source
                    .get(id)
                    .copied()
                    .into_iter()
                    .chain(source.location(*id).map(|location| location.coord))
            })
            .collect::<HashSet<_>>();
        let mut changes = affected.to_vec();
        changes.extend(self.compose(ctx, &dirty));

        Some(changes)
    }

    pub fn choose(&mut self, ctx: ComposeContext<'_>, key: &ModuleRootKey, index: usize) -> Vec<PrefabInstanceId> {
        let Some(variant) = self
            .preview
            .as_ref()
            .and_then(|preview| preview.roots.iter().find(|root| root.key == *key))
            .and_then(|root| root.variants.get(index))
            .filter(|variant| variant.error.is_none())
        else {
            return Vec::new();
        };

        if self.choices.get(key) == Some(&variant.choice) {
            return Vec::new();
        }

        self.choices.insert(key.clone(), variant.choice.clone());

        self.compose(ctx, &HashSet::new())
    }

    pub fn set_enabled(&mut self, source: &Grid, enabled: bool) -> Vec<PrefabInstanceId> {
        if self.enabled == enabled {
            return Vec::new();
        }

        self.enabled = enabled;

        ModuleOverlay::changes(source, self.preview.as_ref(), None)
    }

    pub fn apply_choices(&mut self, ctx: ComposeContext<'_>, choices: ModuleChoices) -> Vec<PrefabInstanceId> {
        self.choices = choices;

        self.compose(ctx, &HashSet::new())
    }
}
