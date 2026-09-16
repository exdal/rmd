use core::types::{Identifier, ProcId, Value};
use std::{
    collections::{HashMap, HashSet, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
};

use codegen::Module;
use objtree::{ObjectTree, TypeId};

use crate::{
    AppearanceDelta,
    Diagnostics,
    Fault,
    FaultKind,
    GenericValue,
    Limits,
    Runtime,
    heap::{Object, ObjectId},
    world::Position,
};

const HOOKS: [&str; 4] = ["demir_initialize", "demir_prepare", "demir_light", "demir_bake"];

/// `#ifdef __DEMIR_BAKE__` around one or more bake hooks in the codebase.
pub fn has_profile(tree: &ObjectTree) -> bool { HOOKS.iter().any(|name| hook(tree, name).is_some()) }

fn hook(tree: &ObjectTree, name: &str) -> Option<ProcId> {
    tree.proc_inherited(TypeId::ROOT, &name.into())
        .and_then(|proc| proc.body)
}

#[derive(Debug, Clone, Copy, Default)]
struct Hooks {
    initialize: Option<ProcId>,
    prepare: Option<ProcId>,
    light: Option<ProcId>,
    bake: Option<ProcId>,
}

impl Hooks {
    fn resolve(tree: &ObjectTree) -> Self {
        Self {
            initialize: hook(tree, "demir_initialize"),
            prepare: hook(tree, "demir_prepare"),
            light: hook(tree, "demir_light"),
            bake: hook(tree, "demir_bake"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Atom {
    pub instance: u64,
    pub ty: TypeId,
    pub position: Position,
    pub vars: Vec<(Identifier, Value)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct CacheKey {
    ty: TypeId,
    vars: u64,
    neighborhood: u64,
    position: Option<Position>,
    size: [i32; 3],
}

#[derive(Debug, Default)]
struct Contributions(Vec<(u64, AppearanceDelta)>);

impl Contributions {
    fn slot(&self, id: u64) -> Result<usize, usize> { self.0.binary_search_by_key(&id, |(current, _)| *current) }

    fn insert(&mut self, id: u64, appearance: AppearanceDelta) {
        match self.slot(id) {
            Ok(index) => {
                if let Some((_, slot)) = self.0.get_mut(index) {
                    *slot = appearance;
                }
            },
            Err(index) => self.0.insert(index, (id, appearance)),
        }
    }

    fn remove(&mut self, id: u64) {
        if let Ok(index) = self.slot(id) {
            self.0.remove(index);
        }
    }

    fn get(&self, id: u64) -> Option<&AppearanceDelta> {
        self.0.get(self.slot(id).ok()?).map(|(_, appearance)| appearance)
    }

    fn iter(&self) -> impl Iterator<Item = &(u64, AppearanceDelta)> { self.0.iter() }
}

#[derive(Debug)]
struct Preview {
    values: HashMap<u64, AppearanceDelta>,
    memo_safe: bool,
    position_sensitive: bool,
}

#[derive(Debug, Default)]
pub struct Bake {
    pub runtime: Runtime,
    pub appearances: HashMap<u64, AppearanceDelta>,
    pub diagnostics: Diagnostics,
    pub attempted: usize,
    pub succeeded: usize,
    pub cache_hits: usize,
    cache: HashMap<CacheKey, AppearanceDelta>,
    epoch: u64,
    fingerprints: HashMap<u64, u64>,
    pub limits: Limits,
    atoms: HashMap<u64, Atom>,
    objects: HashMap<u64, ObjectId>,
    cells: HashMap<Position, Vec<u64>>,
    failed: HashSet<u64>,
    faults: HashMap<u64, Fault>,
    areas: HashMap<(TypeId, u64), ObjectId>,
    area_members: HashMap<ObjectId, HashSet<u64>>,
    initialized: bool,
    prepared: HashMap<ObjectId, Option<Fault>>,
    lit: HashSet<ObjectId>,
    contributions: HashMap<u64, Vec<u64>>,
    by_target: HashMap<u64, Contributions>,
    dirty_appearances: HashSet<u64>,
    hooks: Hooks,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Instantiate,
    Initialize,
    Prepare,
    Light,
    Smooth,
}

impl Bake {
    pub fn new(tree: &ObjectTree, module: &Module, atoms: Vec<Atom>, size: [i32; 3], limits: Limits) -> Self {
        Self::with_progress(tree, module, atoms, size, limits, |_, _, _| {})
    }

    pub fn with_progress(
        tree: &ObjectTree, module: &Module, atoms: Vec<Atom>, size: [i32; 3], limits: Limits,
        mut progress: impl FnMut(Stage, usize, usize),
    ) -> Self {
        let mut bake = Self {
            limits,
            hooks: Hooks::resolve(tree),
            ..Default::default()
        };

        bake.runtime.world.size = size;

        let mut atoms = atoms;
        atoms.sort_by_key(|atom| atom.instance);
        let total = atoms.len();
        for (index, atom) in atoms.into_iter().enumerate() {
            if index.is_multiple_of(4096) {
                progress(Stage::Instantiate, index, total);
            }
            bake.insert(tree, atom);
        }

        progress(Stage::Instantiate, total, total);

        let positions = bake.cells.keys().copied().collect::<Vec<_>>();
        for position in positions {
            bake.link_cell(tree, position);
        }

        progress(Stage::Initialize, 0, 1);
        let initialized = bake.initialize(tree, module);
        progress(Stage::Initialize, 1, 1);
        if let Err(fault) = initialized {
            bake.diagnostics.record(fault);

            return bake;
        }
        bake.initialized = true;

        let ids = bake.sorted_ids();

        for (index, id) in ids.iter().enumerate() {
            if index.is_multiple_of(4096) {
                progress(Stage::Prepare, index, total);
            }
            bake.prepare(tree, module, *id);
        }

        progress(Stage::Prepare, total, total);

        for (index, id) in ids.iter().enumerate() {
            if index.is_multiple_of(4096) {
                progress(Stage::Light, index, total);
            }
            bake.light(tree, module, *id);
        }

        progress(Stage::Light, total, total);

        for id in &ids {
            bake.fingerprint(*id);
        }

        for (index, id) in ids.iter().enumerate() {
            if index.is_multiple_of(4096) {
                progress(Stage::Smooth, index, total);
            }
            bake.bake_atom(tree, module, *id);
        }

        progress(Stage::Smooth, total, total);
        bake.compose();

        bake
    }

    pub fn position(&self, id: u64) -> Option<Position> { self.atoms.get(&id).map(|atom| atom.position) }

    pub fn object(&self, id: u64) -> Option<ObjectId> { self.objects.get(&id).copied() }

    pub fn take_output(&mut self) -> Vec<String> { self.runtime.take_output() }

    fn sorted_ids(&self) -> Vec<u64> {
        let mut ids = self.atoms.keys().copied().collect::<Vec<_>>();
        ids.sort_unstable();
        ids
    }

    fn insert(&mut self, tree: &ObjectTree, atom: Atom) {
        let mut hasher = DefaultHasher::new();
        hash_constants(&atom.vars, &mut hasher);
        let fingerprint = hasher.finish();

        let area_key = tree
            .roots()
            .area
            .filter(|ty| tree.is_subtype_of(atom.ty, *ty))
            .map(|_| (atom.ty, fingerprint));
        if let Some(object) = area_key.and_then(|key| self.areas.get(&key)).copied() {
            self.objects.insert(atom.instance, object);
            self.area_members.entry(object).or_default().insert(atom.instance);
            self.cells.entry(atom.position).or_default().push(atom.instance);
            self.atoms.insert(atom.instance, atom);

            return;
        }

        let mut object = Object::new(atom.ty);
        object.instance = Some(atom.instance);
        if tree.roots().turf.is_some_and(|ty| tree.is_subtype_of(atom.ty, ty)) {
            object.position = Some(atom.position);
        }

        for (name, value) in &atom.vars {
            match self.runtime.constant(value) {
                Ok(value) => {
                    object.vars.insert(name.clone(), value);
                },
                Err(kind) => {
                    self.diagnostics.record(Fault::detached(kind));
                    self.failed.insert(atom.instance);
                },
            }
        }

        match self.runtime.heap.alloc_object(object) {
            Ok(id) => {
                self.objects.insert(atom.instance, id);
                if let Some(key) = area_key {
                    self.areas.insert(key, id);
                    self.area_members.entry(id).or_default().insert(atom.instance);
                }
            },
            Err(kind) => {
                self.diagnostics.record(Fault::detached(kind));
                self.failed.insert(atom.instance);
            },
        }

        self.fingerprints.insert(atom.instance, fingerprint);
        self.cells.entry(atom.position).or_default().push(atom.instance);
        self.atoms.insert(atom.instance, atom);
    }

    fn link_cell(&mut self, tree: &ObjectTree, position: Position) {
        let ids = self.cells.get(&position).cloned().unwrap_or_default();
        let subtype = |id: &u64, base: Option<TypeId>| {
            self.atoms
                .get(id)
                .is_some_and(|atom| base.is_some_and(|base| tree.is_subtype_of(atom.ty, base)))
        };
        let turf = ids
            .iter()
            .find(|id| subtype(id, tree.roots().turf))
            .and_then(|id| self.objects.get(id))
            .copied();
        let area = ids
            .iter()
            .find(|id| subtype(id, tree.roots().area))
            .and_then(|id| self.objects.get(id))
            .copied();

        self.runtime.world.turfs.remove(&position);
        self.runtime.world.areas.remove(&position);
        if let Some(turf) = turf {
            self.runtime.world.turfs.insert(position, turf);
        }

        if let Some(area) = area {
            self.runtime.world.areas.insert(position, area);
        }

        for id in ids {
            if let Some(object) = self.objects.get(&id).copied() {
                let loc = if Some(object) == area {
                    None
                } else if Some(object) == turf {
                    area
                } else {
                    turf
                };
                if let Err(kind) = self.runtime.heap.relocate(object, loc) {
                    self.diagnostics.record(Fault::detached(kind));
                }
            }
        }
    }

    fn initialize(&mut self, tree: &ObjectTree, module: &Module) -> Result<(), Fault> {
        let Some(proc) = self.hooks.initialize else {
            return Ok(());
        };

        self.runtime
            .run(tree, module, proc, None, Vec::new(), self.limits)
            .map(|_| ())
    }

    fn prepare(&mut self, tree: &ObjectTree, module: &Module, id: u64) {
        if self.failed.contains(&id) {
            return;
        }

        let Some(object) = self.objects.get(&id).copied() else {
            return;
        };

        if let Some(fault) = self.prepared.get(&object).cloned() {
            if let Some(fault) = fault {
                self.failed.insert(id);
                self.record_fault(id, fault);
            }
            return;
        }

        let fault = self.hooks.prepare.and_then(|proc| {
            let args = vec![GenericValue::Object(object)];
            self.runtime.run(tree, module, proc, None, args, self.limits).err()
        });
        self.prepared.insert(object, fault.clone());
        if let Some(fault) = fault {
            self.failed.insert(id);
            self.record_fault(id, fault);
        }
    }

    fn light(&mut self, tree: &ObjectTree, module: &Module, id: u64) {
        if self.failed.contains(&id) {
            return;
        }

        let Some(object) = self.objects.get(&id).copied() else {
            return;
        };

        if !self.lit.insert(object) {
            return;
        }

        let Some(proc) = self.hooks.light else {
            return;
        };
        let args = vec![GenericValue::Object(object)];
        if let Err(fault) = self.runtime.run(tree, module, proc, None, args, self.limits) {
            self.record_fault(id, fault);
        }
    }

    fn record_fault(&mut self, id: u64, fault: Fault) {
        if let Some(previous) = self.faults.insert(id, fault.clone()) {
            self.diagnostics.remove(&previous);
        }
        self.diagnostics.record(fault);
    }

    fn fingerprint(&mut self, id: u64) {
        let Some(atom) = self.atoms.get(&id) else {
            return;
        };
        let Some(object) = self.object(id).and_then(|id| self.runtime.heap.object(id)) else {
            return;
        };

        let mut vars = object
            .vars
            .iter()
            .map(|(name, value)| (name, value.hash_key()))
            .collect::<Vec<_>>();
        vars.sort_unstable();

        let mut hasher = DefaultHasher::new();
        hash_constants(&atom.vars, &mut hasher);
        vars.hash(&mut hasher);
        self.fingerprints.insert(id, hasher.finish());
    }

    fn cache_key(&self, id: u64) -> Option<CacheKey> {
        let atom = self.atoms.get(&id)?;
        let mut hasher = DefaultHasher::new();
        for dx in -1..=1 {
            for dy in -1..=1 {
                let position = Position::new(
                    atom.position.x.saturating_add(dx),
                    atom.position.y.saturating_add(dy),
                    atom.position.z,
                );
                let cell = self.cells.get(&position).into_iter().flatten();
                let mut count = 0usize;
                for id in cell {
                    let Some(entry) = self
                        .atoms
                        .get(id)
                        .zip(self.fingerprints.get(id))
                        .map(|(atom, fingerprint)| (atom.ty, *fingerprint))
                    else {
                        continue;
                    };
                    entry.hash(&mut hasher);
                    count += 1;
                }
                count.hash(&mut hasher);
            }
        }

        Some(CacheKey {
            ty: atom.ty,
            vars: *self.fingerprints.get(&id)?,
            neighborhood: hasher.finish(),
            position: None,
            size: self.runtime.world.size,
        })
    }

    fn install_contribution(&mut self, id: u64, mut values: HashMap<u64, AppearanceDelta>) {
        let shared = values
            .iter()
            .filter(|(target, _)| **target != id)
            .filter_map(|(target, appearance)| {
                self.objects
                    .get(target)
                    .and_then(|object| self.area_members.get(object))
                    .map(|members| (members.clone(), appearance.clone()))
            })
            .collect::<Vec<_>>();
        for (members, appearance) in shared {
            for target in members {
                values.insert(target, appearance.clone());
            }
        }

        self.contributions
            .insert(id, values.keys().copied().collect::<Vec<_>>());
        for (target, appearance) in values {
            self.by_target.entry(target).or_default().insert(id, appearance);
            self.dirty_appearances.insert(target);
        }
    }

    fn bake_atom(&mut self, tree: &ObjectTree, module: &Module, id: u64) {
        self.remove_contribution(id);
        if self.failed.contains(&id) {
            return;
        }
        let Some(object) = self.objects.get(&id).copied() else {
            return;
        };
        let Some(proc) = self.hooks.bake else {
            return;
        };

        if let Some(fault) = self.faults.remove(&id) {
            self.diagnostics.remove(&fault);
        }
        self.attempted += 1;
        let key = (self.epoch == 0).then(|| self.cache_key(id)).flatten();
        let cached = key
            .as_ref()
            .and_then(|key| self.cache.get(key))
            .or_else(|| {
                let mut positioned = key?;
                positioned.position = self.position(id);
                self.cache.get(&positioned)
            })
            .cloned();
        if let Some(appearance) = cached {
            self.cache_hits += 1;
            self.succeeded += 1;
            self.install_contribution(id, HashMap::from([(id, appearance)]));

            return;
        }

        match self.runtime.preview(tree, module, proc, object, id, self.limits) {
            Ok(preview) => {
                self.succeeded += 1;
                if preview.memo_safe
                    && preview.values.len() == 1
                    && let Some(appearance) = preview.values.get(&id)
                    && let Some(mut key) = key
                {
                    if preview.position_sensitive {
                        key.position = self.position(id);
                    }
                    if self.cache.len() < 50_000 {
                        self.cache.insert(key, appearance.clone());
                    }
                }
                self.install_contribution(id, preview.values);
            },
            Err(fault) => self.record_fault(id, fault),
        }
    }

    fn remove_contribution(&mut self, id: u64) {
        if let Some(targets) = self.contributions.remove(&id) {
            for target in targets {
                if let Some(values) = self.by_target.get_mut(&target) {
                    values.remove(id);
                }
                self.dirty_appearances.insert(target);
            }
        }
    }

    fn compose(&mut self) {
        for id in std::mem::take(&mut self.dirty_appearances) {
            self.appearances.remove(&id);
            if self.failed.contains(&id) || !self.atoms.contains_key(&id) {
                continue;
            }
            let Some(values) = self.by_target.get(&id) else {
                continue;
            };
            let mut composed = values.get(id).cloned().unwrap_or_default();
            for (root, value) in values.iter() {
                if *root == id {
                    continue;
                }
                for (name, value) in &value.vars {
                    if let Some((_, previous)) = composed.vars.iter_mut().find(|(current, _)| current == name) {
                        *previous = value.clone();
                    } else {
                        composed.vars.push((name.clone(), value.clone()));
                    }
                }
                composed.overlays.extend(value.overlays.clone());
                composed.underlays.extend(value.underlays.clone());
            }
            self.appearances.insert(id, composed);
        }
    }

    // replaces changed inputs and reevaluates the surrounding 3 by 3 by 3 neighborhood
    pub fn update(&mut self, tree: &ObjectTree, module: &Module, replacements: Vec<Atom>, removed: &[u64]) -> Vec<u64> {
        self.epoch = self.epoch.wrapping_add(1);
        let mut dirty = HashSet::new();
        let mut inserted = Vec::new();
        let remove = removed
            .iter()
            .copied()
            .chain(replacements.iter().map(|atom| atom.instance))
            .collect::<Vec<_>>();

        for id in &remove {
            if let Some(atom) = self.atoms.remove(id) {
                dirty.insert(atom.position);
                if let Some(cell) = self.cells.get_mut(&atom.position) {
                    cell.retain(|value| value != id);
                }
            }
            if let Some(object) = self.objects.remove(id) {
                let mut shared = false;
                if let Some(members) = self.area_members.get_mut(&object) {
                    members.remove(id);
                    shared = !members.is_empty();
                    if let Ok(value) = self.runtime.heap.object_mut(object) {
                        value.instance = members.iter().copied().min();
                    }
                }
                if !shared {
                    self.area_members.remove(&object);
                    self.areas.retain(|_, value| *value != object);
                    self.prepared.remove(&object);
                    self.lit.remove(&object);
                    let _ = self.runtime.heap.relocate(object, None);
                    if let Ok(object) = self.runtime.heap.object_mut(object) {
                        object.deleted = true;
                    }
                }
            }
            self.fingerprints.remove(id);
            if let Some(fault) = self.faults.remove(id) {
                self.diagnostics.remove(&fault);
            }
            self.failed.remove(id);
            self.remove_contribution(*id);
        }

        for atom in replacements {
            dirty.insert(atom.position);
            inserted.push(atom.instance);
            self.insert(tree, atom);
        }
        for position in &dirty {
            self.link_cell(tree, *position);
        }
        if self.initialized {
            for id in inserted {
                self.prepare(tree, module, id);
                self.light(tree, module, id);
                self.fingerprint(id);
            }
        }

        let mut affected = HashSet::new();
        for position in dirty {
            for dz in -1..=1 {
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        let position = Position::new(
                            position.x.saturating_add(dx),
                            position.y.saturating_add(dy),
                            position.z.saturating_add(dz),
                        );
                        if let Some(ids) = self.cells.get(&position) {
                            affected.extend(ids.iter().copied());
                        }
                    }
                }
            }
        }

        let mut affected = affected.into_iter().collect::<Vec<_>>();
        affected.sort_unstable();
        if self.initialized {
            for id in &affected {
                self.bake_atom(tree, module, *id);
            }
        }
        affected.extend(remove);
        affected.extend(self.dirty_appearances.iter().copied());
        affected.sort_unstable();
        affected.dedup();
        self.compose();

        affected
    }
}

impl Runtime {
    fn preview(
        &mut self, tree: &ObjectTree, module: &Module, proc: ProcId, object: ObjectId, instance: u64, limits: Limits,
    ) -> Result<Preview, Fault> {
        if self.global.is_none() {
            self.global = Some(
                self.heap
                    .alloc_object(Object::new(TypeId::ROOT))
                    .map_err(|kind| Fault {
                        offset: None,
                        proc: Some(proc),
                        location: Default::default(),
                        kind,
                    })?,
            );
        }

        self.heap.begin();
        let (overlays, underlays) = NAMES.with(|names| (names.overlays.clone(), names.underlays.clone()));
        let result = (|| {
            let mut evaluator = crate::eval::Evaluator::new(self, tree, module, limits, Some(object));
            evaluator.call(proc, None, vec![(None, GenericValue::Object(object))])?;

            let mut ids = evaluator.runtime.heap.changed_objects();
            ids.extend(evaluator.appearance_reads.iter().copied().filter(|id| {
                evaluator.runtime.heap.object(*id).is_some_and(|object| {
                    [&overlays, &underlays].iter().any(|name| {
                        matches!(
                            object.vars.get(*name),
                            Some(GenericValue::List(list)) if evaluator.runtime.heap.list_changed(*list)
                        )
                    })
                })
            }));
            ids.push(object);
            ids.sort_by_key(|id| id.0);
            ids.dedup();

            let mut values = HashMap::new();
            let mut export_budget = limits.allocations.min(evaluator.instruction_budget_remaining as usize);
            for id in ids {
                if let Some(instance) = if id == object {
                    Some(instance)
                } else {
                    evaluator.runtime.heap.object(id).and_then(|object| object.instance)
                } {
                    let mut appearance = export_appearance(&evaluator.runtime.heap, tree, id, 0, &mut export_budget)
                        .map_err(|kind| evaluator.fault(kind))?;
                    if id != object
                        && let Some(before) = evaluator.runtime.heap.before_object(id)
                    {
                        appearance.vars.retain(|(name, value)| {
                            before
                                .vars
                                .get(name)
                                .and_then(|value| export_value(value).ok())
                                .or_else(|| {
                                    tree.var_inherited(before.ty, name)
                                        .map(|variable| variable.value.clone())
                                })
                                .as_ref()
                                != Some(value)
                        });
                        for (name, extra) in [
                            (&overlays, &mut appearance.overlays),
                            (&underlays, &mut appearance.underlays),
                        ] {
                            if let Some(GenericValue::List(list)) = before.vars.get(name) {
                                let count = evaluator
                                    .runtime
                                    .heap
                                    .before_list(*list)
                                    .map_or(0, |list| list.entries.len());
                                if count <= extra.len() {
                                    extra.drain(..count);
                                }
                            }
                        }
                    }
                    values.insert(instance, appearance);
                }
            }

            Ok(Preview {
                values,
                memo_safe: evaluator.memo_safe,
                position_sensitive: evaluator.position_sensitive,
            })
        })();
        self.heap.rollback();

        result
    }
}

thread_local! {
    static NAMES: Names = Names {
        appearance: APPEARANCE_VARS.iter().map(|name| Identifier::from(*name)).collect(),
        overlays: Identifier::from("overlays"),
        underlays: Identifier::from("underlays"),
        icon_state: Identifier::from("icon_state"),
        dir: Identifier::from("dir"),
        layer: Identifier::from("layer"),
        plane: Identifier::from("plane"),
    };
}

struct Names {
    appearance: Vec<Identifier>,
    overlays: Identifier,
    underlays: Identifier,
    icon_state: Identifier,
    dir: Identifier,
    layer: Identifier,
    plane: Identifier,
}

const APPEARANCE_VARS: &[&str] = &[
    "name",
    "icon",
    "icon_state",
    "dir",
    "layer",
    "plane",
    "pixel_x",
    "pixel_y",
    "pixel_w",
    "pixel_z",
    "step_x",
    "step_y",
    "color",
    "alpha",
    "invisibility",
    "appearance_flags",
];

fn export_appearance(
    heap: &crate::heap::Heap, tree: &ObjectTree, id: ObjectId, depth: usize, budget: &mut usize,
) -> Result<AppearanceDelta, FaultKind> {
    NAMES.with(|names| export_with_names(names, heap, tree, id, depth, budget))
}

fn export_with_names(
    names: &Names, heap: &crate::heap::Heap, tree: &ObjectTree, id: ObjectId, depth: usize, budget: &mut usize,
) -> Result<AppearanceDelta, FaultKind> {
    *budget = budget.checked_sub(1).ok_or(FaultKind::Memory)?;
    if depth >= 32 {
        return Err(FaultKind::Memory);
    }

    let object = heap.object(id).ok_or(FaultKind::InvalidReference)?;
    let mut appearance = AppearanceDelta::default();
    for name in &names.appearance {
        let value = object.vars.get(name).map(export_value).transpose()?.or_else(|| {
            tree.var_inherited(object.ty, name)
                .map(|variable| variable.value.clone())
        });
        if let Some(value) = value {
            let cost = value.as_text().map_or(1, |text| text.len().div_ceil(32).max(1));
            *budget = budget.checked_sub(cost).ok_or(FaultKind::Memory)?;
            appearance.vars.push((name.clone(), value));
        }
    }

    for name in [&names.overlays, &names.underlays] {
        let mut values = Vec::new();
        if let Some(GenericValue::List(list)) = object.vars.get(name)
            && let Some(list) = heap.list(*list)
        {
            for (value, _) in &list.entries {
                match value {
                    GenericValue::Object(id) => {
                        values.push(export_with_names(names, heap, tree, *id, depth + 1, budget)?)
                    },
                    GenericValue::Text(state) => {
                        *budget = budget
                            .checked_sub(state.len().div_ceil(32).saturating_add(4))
                            .ok_or(FaultKind::Memory)?;
                        values.push(AppearanceDelta {
                            vars: vec![
                                (names.icon_state.clone(), Value::Text(state.to_string())),
                                (names.dir.clone(), Value::Num(0.0)),
                                (names.layer.clone(), Value::Num(-1.0)),
                                (names.plane.clone(), Value::Num(-32767.0)),
                            ],
                            ..Default::default()
                        });
                    },
                    GenericValue::Null => {},
                    _ => return Err(FaultKind::Unsupported("overlay value".into())),
                }
            }
        }
        if name == &names.overlays {
            appearance.overlays = values;
        } else {
            appearance.underlays = values;
        }
    }

    Ok(appearance)
}

fn hash_constants(vars: &[(Identifier, Value)], hasher: &mut DefaultHasher) {
    vars.len().hash(hasher);
    for (name, value) in vars {
        name.hash(hasher);
        hash_constant(value, hasher);
    }
}

fn hash_constant(value: &Value, hasher: &mut DefaultHasher) {
    std::mem::discriminant(value).hash(hasher);
    match value {
        Value::Null | Value::Unevaluated => {},
        Value::Num(number) => number.to_bits().hash(hasher),
        Value::Text(text) | Value::Resource(text) => text.hash(hasher),
        Value::Path(path) => path.hash(hasher),
        Value::List(entries) => {
            entries.len().hash(hasher);
            for entry in entries {
                hash_constant(&entry.key, hasher);
                entry.value.is_some().hash(hasher);
                if let Some(value) = &entry.value {
                    hash_constant(value, hasher);
                }
            }
        },
    }
}

fn export_value(value: &GenericValue) -> Result<Value, FaultKind> {
    Ok(match value {
        GenericValue::Null => Value::Null,
        GenericValue::Num(number) => Value::Num(*number),
        GenericValue::Text(text) => Value::Text(text.to_string()),
        GenericValue::Resource(path) => Value::Resource(path.to_string()),
        GenericValue::Path(path) => Value::Path(path.clone()),
        _ => return Err(FaultKind::Unsupported("appearance value".into())),
    })
}
