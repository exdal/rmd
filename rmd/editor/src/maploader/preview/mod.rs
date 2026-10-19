use std::{
    collections::{HashMap, HashSet, hash_map::Entry},
    path::{Path, PathBuf},
    sync::Arc,
};

use dmm::{Coord, Prefab, PrefabInstanceId};

use crate::{
    grid::Grid,
    maploader::{Diagnostic, ErrorKind},
};

mod compose;
mod state;
#[cfg(test)]
mod tests;
mod view;
use compose::ModuleLayer;
pub use compose::{ComposeContext, compose};
pub use state::ModulePreviewState;
pub use view::MapView;
use view::OverlayTiles;

pub const PREVIEW_ID_START: u64 = 1 << 63;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModuleVariantKey {
    pub path: PathBuf,
    pub occurrence: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModuleRootKey {
    pub instance: PrefabInstanceId,
    pub ancestry: Vec<(ModuleVariantKey, Coord, usize)>,
}

pub type ModuleChoices = HashMap<ModuleRootKey, ModuleVariantKey>;

#[derive(Debug, Clone)]
pub struct ModuleVariantPreview {
    pub choice: ModuleVariantKey,
    pub error: Option<Diagnostic>,
}

#[derive(Debug, Clone)]
pub struct ModuleRootPreview {
    pub key: ModuleRootKey,
    pub coord: Coord,
    pub depth: usize,
    pub marker: Option<Prefab>,
    pub variants: Vec<ModuleVariantPreview>,
    pub selected: Option<usize>,
    pub error: Option<Diagnostic>,
}

impl ModuleRootPreview {
    pub fn adjacent(&self, forward: bool) -> Option<usize> {
        let selected = self.selected?;
        if forward {
            (selected + 1..self.variants.len()).find(|index| self.variants[*index].error.is_none())
        } else {
            (0..selected).rev().find(|index| self.variants[*index].error.is_none())
        }
    }
}

type PreviewInstanceKey = (ModuleRootKey, ModuleVariantKey, Coord, usize);

#[derive(Debug)]
pub struct PreviewIdAllocator {
    next: Option<u64>,
    entries: HashMap<PreviewInstanceKey, PrefabInstanceId>,
}

impl Default for PreviewIdAllocator {
    fn default() -> Self {
        Self {
            next: Some(PREVIEW_ID_START),
            entries: HashMap::new(),
        }
    }
}

impl PreviewIdAllocator {
    fn get(
        &mut self, root: &ModuleRootKey, choice: &ModuleVariantKey, coord: Coord, index: usize,
    ) -> Result<PrefabInstanceId, ErrorKind> {
        let key = (root.clone(), choice.clone(), coord, index);

        match self.entries.entry(key) {
            Entry::Occupied(entry) => Ok(*entry.get()),
            Entry::Vacant(entry) => {
                let id = self
                    .next
                    .and_then(PrefabInstanceId::from_raw)
                    .ok_or(ErrorKind::PreviewIdsExhausted)?;
                self.next = id.get().checked_add(1);
                entry.insert(id);
                Ok(id)
            },
        }
    }
}

#[derive(Debug)]
pub struct ModuleOverlay {
    tiles: OverlayTiles,
    layers: Vec<Arc<ModuleLayer>>,
    pub covered: HashSet<Coord>,
    pub roots: Vec<ModuleRootPreview>,
    pub diagnostics: Vec<Diagnostic>,
    pub choices: ModuleChoices,
}

impl ModuleOverlay {
    pub fn uses_file(&self, path: &Path) -> bool { self.layers.iter().any(|layer| layer.uses_file(path)) }

    pub fn view<'a>(&'a self, source: &'a Grid) -> MapView<'a> { MapView::new(source, &self.tiles) }

    pub fn changes(source: &Grid, previous: Option<&Self>, current: Option<&Self>) -> Vec<PrefabInstanceId> {
        let before = previous.map_or_else(|| MapView::from(source), |preview| preview.view(source));
        let after = current.map_or_else(|| MapView::from(source), |preview| preview.view(source));

        let coords = previous
            .into_iter()
            .chain(current)
            .flat_map(|preview| preview.covered.iter().copied())
            .collect::<HashSet<_>>();
        let coords = coords.into_iter().filter(|coord| {
            !previous.zip(current).is_some_and(|(old, new)| {
                old.tiles
                    .cells
                    .get(coord)
                    .zip(new.tiles.cells.get(coord))
                    .is_some_and(|(old, new)| Arc::ptr_eq(old, new))
            })
        });

        let candidates = coords
            .flat_map(|coord| before.ids_at(coord).iter().chain(after.ids_at(coord)).copied())
            .collect::<HashSet<_>>();

        candidates
            .into_iter()
            .filter(|id| before.prefab_instance(*id) != after.prefab_instance(*id))
            .collect::<Vec<_>>()
    }
}
