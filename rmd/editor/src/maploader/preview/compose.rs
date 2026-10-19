use core::file_path;
use std::{
    array,
    cmp::Reverse,
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
};

use dmm::{Coord, Map, Prefab, PrefabInstanceId, Size};
use objtree::{ObjectTree, TypeId};
use vm::bake::ModuleRoot;

use super::{
    ModuleChoices,
    ModuleOverlay,
    ModuleRootKey,
    ModuleRootPreview,
    ModuleVariantKey,
    ModuleVariantPreview,
    PREVIEW_ID_START,
    PreviewIdAllocator,
};
use crate::{
    grid::Grid,
    maploader::{
        Diagnostic,
        ErrorKind,
        LoadedModule,
        MAX_NESTING,
        ModuleFiles,
        RootSpecs,
        module::Anchor,
        resolve_module_path,
        resolved_path,
    },
};

#[derive(Clone, Copy)]
pub struct ComposeContext<'a> {
    pub source: &'a Grid,
    pub tree: &'a ObjectTree,
    pub base_dir: &'a Path,
    pub document_path: Option<&'a Path>,
}

#[derive(Debug)]
pub(super) struct ModuleLayer {
    key: ModuleRootKey,
    choice: ModuleVariantKey,
    map: Arc<Map>,
    offset: [i64; 3],
    size: Size,
    cells: HashMap<Coord, Vec<(PrefabInstanceId, Prefab)>>,
    origins: HashMap<PrefabInstanceId, (Coord, usize)>,
    marker: (Coord, PrefabInstanceId),
    skip: Vec<TypeId>,
    anchor: Anchor,
    clipped: usize,
}

impl ModuleLayer {
    pub(super) fn uses_file(&self, path: &Path) -> bool { self.choice.path == path }
}

pub fn compose(
    ctx: ComposeContext<'_>, specs: &RootSpecs, files: &mut ModuleFiles, choices: &ModuleChoices,
    ids: &mut PreviewIdAllocator,
) -> ModuleOverlay {
    update(ctx, specs, files, choices, ids, None, &HashSet::new())
}

pub(super) fn update(
    ctx: ComposeContext<'_>, specs: &RootSpecs, files: &mut ModuleFiles, choices: &ModuleChoices,
    ids: &mut PreviewIdAllocator, previous: Option<&ModuleOverlay>, dirty_source: &HashSet<Coord>,
) -> ModuleOverlay {
    let ComposeContext { source, tree, .. } = ctx;
    let cached = previous
        .into_iter()
        .flat_map(|preview| &preview.layers)
        .map(|layer| (layer.key.clone(), layer.clone()))
        .collect::<HashMap<_, _>>();
    let mut composer = ModuleComposer {
        preview: ModuleOverlay {
            tiles: previous.map(|preview| preview.tiles.clone()).unwrap_or_default(),
            covered: HashSet::new(),
            roots: Vec::new(),
            diagnostics: Vec::new(),
            choices: choices.clone(),
            layers: Vec::new(),
        },
        cached,
        ctx,
        specs,
        document: ctx.document_path.map(resolved_path),
        files,
        ids,
    };

    let mut sources = specs
        .keys()
        .filter(|id| id.get() < PREVIEW_ID_START)
        .filter_map(|id| source.prefab_instance(*id))
        .collect::<Vec<_>>();
    sources.sort_by_key(|instance| {
        let coord = instance.location.coord;

        (coord.z, Reverse(coord.y), coord.x, instance.location.prefab_index)
    });
    for instance in sources {
        composer.root(
            ModuleRootKey {
                instance: instance.id,
                ancestry: Vec::new(),
            },
            instance.location.coord,
            instance.id,
            instance.prefab().clone(),
        );
    }

    let mut preview = composer.preview;
    let mut dirty = dirty_source.clone();
    // If roots were reordered, their overlap order may have changed.
    let is_reordered = previous.is_some_and(|old| {
        let old_order = old
            .layers
            .iter()
            .map(|layer| &layer.key)
            .filter(|key| preview.layers.iter().any(|next| &next.key == *key))
            .collect::<Vec<_>>();
        let new_order = preview
            .layers
            .iter()
            .map(|layer| &layer.key)
            .filter(|key| old.layers.iter().any(|previous| &previous.key == *key))
            .collect::<Vec<_>>();

        old_order != new_order
    });

    for layer in previous
        .into_iter()
        .flat_map(|preview| &preview.layers)
        .chain(&preview.layers)
    {
        let is_unchanged = !is_reordered
            && previous.is_some_and(|old| old.layers.iter().any(|previous| Arc::ptr_eq(previous, layer)))
            && preview.layers.iter().any(|next| Arc::ptr_eq(next, layer));
        if !is_unchanged {
            dirty.extend(layer.cells.keys().copied());
            dirty.insert(layer.marker.0);
        }
    }

    for layer in &preview.layers {
        preview.covered.extend(layer.cells.keys().copied());
        preview.covered.insert(layer.marker.0);
    }

    for coord in dirty {
        preview.tiles.remove(coord);
        if !preview.covered.contains(&coord) || source.tile_at(coord).is_none() {
            continue;
        }

        let mut entries = source
            .ids_at(coord)
            .iter()
            .filter_map(|id| {
                source
                    .prefab_instance(*id)
                    .map(|instance| (*id, instance.prefab().clone()))
            })
            .collect::<Vec<_>>();
        for layer in &preview.layers {
            if let Some(content) = layer.cells.get(&coord) {
                for (id, prefab) in content {
                    let kind = category(tree, prefab);
                    if kind != 0 {
                        entries.retain(|(_, existing)| category(tree, existing) != kind);
                    }

                    entries.push((*id, prefab.clone()));
                }
            }
        }

        preview.tiles.replace(coord, entries);
    }

    preview
}

struct ModuleComposer<'a> {
    preview: ModuleOverlay,
    cached: HashMap<ModuleRootKey, Arc<ModuleLayer>>,
    ctx: ComposeContext<'a>,
    specs: &'a RootSpecs,
    document: Option<PathBuf>,
    files: &'a mut ModuleFiles,
    ids: &'a mut PreviewIdAllocator,
}

impl ModuleComposer<'_> {
    fn root(&mut self, key: ModuleRootKey, coord: Coord, marker: PrefabInstanceId, prefab: Prefab) {
        let specs = self.specs;
        let Some(spec) = specs.get(&marker) else {
            return;
        };

        let spec = match spec {
            Ok(spec) => spec,
            Err(text) => {
                let diagnostic = Diagnostic {
                    coord: Some(coord),
                    module_file: None,
                    chain: Vec::new(),
                    kind: ErrorKind::Hook(text.clone()),
                };
                self.preview.diagnostics.push(diagnostic.clone());
                self.preview.roots.push(ModuleRootPreview {
                    depth: key.ancestry.len(),
                    key,
                    coord,
                    marker: Some(prefab),
                    variants: Vec::new(),
                    selected: None,
                    error: Some(diagnostic),
                });
                return;
            },
        };

        let mut occurrences = HashMap::<PathBuf, usize>::new();
        let mut variants = Vec::new();
        let mut loaded = Vec::new();
        for file in &spec.files {
            let resolved = resolve_module_path(self.ctx.base_dir, file);
            let path = resolved.as_ref().map_or_else(
                |_| PathBuf::from(file_path::forward_slashes(file).as_ref()),
                Clone::clone,
            );
            let result = resolved.and_then(|path| self.open(&key, spec, &path));
            let error = result
                .as_ref()
                .err()
                .map(|kind| self.diagnostic(&key, coord, &path, kind.clone()));
            let occurrence = occurrences.entry(path.clone()).or_insert(0);
            variants.push(ModuleVariantPreview {
                choice: ModuleVariantKey {
                    path,
                    occurrence: *occurrence,
                },
                error: error.clone(),
            });
            *occurrence += 1;
            self.preview.diagnostics.extend(error);
            loaded.push(result.ok());
        }

        let selected = self
            .preview
            .choices
            .get(&key)
            .and_then(|choice| {
                variants
                    .iter()
                    .position(|variant| variant.choice == *choice && variant.error.is_none())
            })
            .or_else(|| variants.iter().position(|variant| variant.error.is_none()));
        let root_index = self.preview.roots.len();
        self.preview.roots.push(ModuleRootPreview {
            key: key.clone(),
            coord,
            depth: key.ancestry.len(),
            marker: Some(prefab),
            variants: variants.clone(),
            selected,
            error: None,
        });
        let Some((selected, Some((module, anchor)))) = selected.map(|index| (index, loaded[index].clone())) else {
            return;
        };

        let choice = variants[selected].choice.clone();
        self.preview.choices.insert(key.clone(), choice.clone());
        let offset = [
            i64::from(coord.x) - i64::from(anchor.0.x) + i64::from(spec.offset[0]),
            i64::from(coord.y) - i64::from(anchor.0.y) + i64::from(spec.offset[1]),
            i64::from(coord.z) - 1 + i64::from(spec.offset[2]),
        ];
        let marker = (coord, marker);
        let cached = self
            .cached
            .get(&key)
            .filter(|layer| {
                layer.choice == choice
                    && Arc::ptr_eq(&layer.map, &module.map)
                    && layer.offset == offset
                    && layer.size == self.ctx.source.size()
                    && layer.marker == marker
                    && layer.skip == spec.skip
                    && layer.anchor == anchor
            })
            .cloned();
        let layer = match cached {
            Some(layer) => layer,
            None => match self.layer(&module, spec, &key, &choice, offset, marker, anchor) {
                Ok(layer) => Arc::new(layer),
                Err(kind) => {
                    let diagnostic = self.diagnostic(&key, coord, &choice.path, kind);
                    self.preview.roots[root_index].error = Some(diagnostic.clone());
                    self.preview.diagnostics.push(diagnostic);
                    return;
                },
            },
        };

        if layer.clipped > 0 {
            let diagnostic = self.diagnostic(&key, coord, &choice.path, ErrorKind::Clipped { tiles: layer.clipped });
            self.preview.diagnostics.push(diagnostic);
        }

        self.preview.layers.push(layer.clone());

        let mut nested = layer
            .cells
            .iter()
            .flat_map(|(cell, entries)| entries.iter().map(move |(id, prefab)| (*cell, *id, prefab)))
            .filter(|(_, id, _)| specs.contains_key(id))
            .filter_map(|(cell, id, prefab)| Some((*layer.origins.get(&id)?, cell, id, prefab.clone())))
            .collect::<Vec<_>>();
        nested.sort_by_key(|((local, index), ..)| (local.z, Reverse(local.y), local.x, *index));
        for ((local, index), cell, id, prefab) in nested {
            let mut ancestry = key.ancestry.clone();
            ancestry.push((choice.clone(), local, index));
            self.root(
                ModuleRootKey {
                    instance: key.instance,
                    ancestry,
                },
                cell,
                id,
                prefab,
            );
        }
    }

    fn open(
        &mut self, key: &ModuleRootKey, spec: &ModuleRoot, path: &Path,
    ) -> Result<(Arc<LoadedModule>, Anchor), ErrorKind> {
        let is_cycle =
            self.document.as_deref() == Some(path) || key.ancestry.iter().any(|(variant, ..)| variant.path == path);
        if is_cycle {
            return Err(ErrorKind::Cycle);
        }

        if key.ancestry.len() > MAX_NESTING {
            return Err(ErrorKind::NestingLimit);
        }

        let module = self.files.load(path)?;
        let anchor = module.anchor(self.ctx.tree, spec.anchor)?;

        Ok((module, anchor))
    }

    fn diagnostic(&self, key: &ModuleRootKey, coord: Coord, path: &Path, kind: ErrorKind) -> Diagnostic {
        let chain = self
            .document
            .iter()
            .cloned()
            .chain(key.ancestry.iter().map(|(variant, ..)| variant.path.clone()))
            .chain([path.to_path_buf()])
            .collect();

        Diagnostic {
            coord: Some(coord),
            module_file: Some(path.to_path_buf()),
            chain,
            kind,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn layer(
        &mut self, module: &LoadedModule, spec: &ModuleRoot, key: &ModuleRootKey, choice: &ModuleVariantKey,
        offset: [i64; 3], marker: (Coord, PrefabInstanceId), anchor: Anchor,
    ) -> Result<ModuleLayer, ErrorKind> {
        let ComposeContext { source, tree, .. } = self.ctx;
        let mut layer = ModuleLayer {
            key: key.clone(),
            choice: choice.clone(),
            map: module.map.clone(),
            offset,
            size: source.size(),
            cells: HashMap::new(),
            origins: HashMap::new(),
            marker,
            skip: spec.skip.clone(),
            anchor,
            clipped: 0,
        };
        for z in 1..=module.map.size.z {
            for y in 1..=module.map.size.y {
                for x in 1..=module.map.size.x {
                    let local = Coord::new(x, y, z);
                    let Some(tile) = module.map.tile_at(local) else {
                        continue;
                    };

                    let content = tile
                        .iter()
                        .enumerate()
                        .filter(|(index, prefab)| {
                            let is_skipped = tree.id_of(&prefab.path).is_some_and(|ty| spec.skip.contains(&ty));
                            let is_anchor = anchor == (local, Some(*index));

                            !(is_skipped || is_anchor)
                        })
                        .collect::<Vec<_>>();
                    if content.is_empty() {
                        continue;
                    }

                    let Some(coord) = translated(local, offset).filter(|coord| source.tile_at(*coord).is_some()) else {
                        layer.clipped += 1;
                        continue;
                    };

                    let mut entries = Vec::with_capacity(content.len());
                    for (index, prefab) in content {
                        let id = self.ids.get(key, choice, local, index)?;
                        layer.origins.insert(id, (local, index));
                        entries.push((id, prefab.clone()));
                    }

                    layer.cells.insert(coord, entries);
                }
            }
        }

        Ok(layer)
    }
}

fn category(tree: &ObjectTree, prefab: &Prefab) -> u8 {
    if let Some(id) = tree.id_of(&prefab.path) {
        let roots = tree.roots();
        if roots.turf.is_some_and(|root| tree.is_subtype_of(id, root)) {
            return 1;
        }

        if roots.area.is_some_and(|root| tree.is_subtype_of(id, root)) {
            return 2;
        }
    }

    match prefab.path.to_string().split('/').nth(1) {
        Some("turf") => 1,
        Some("area") => 2,
        _ => 0,
    }
}

fn translated(coord: Coord, offset: [i64; 3]) -> Option<Coord> {
    let values = [i64::from(coord.x), i64::from(coord.y), i64::from(coord.z)];
    let [x, y, z] = array::from_fn(|axis| u32::try_from(values[axis] + offset[axis]).ok().filter(|v| *v > 0));

    Some(Coord::new(x?, y?, z?))
}
