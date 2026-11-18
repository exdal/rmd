pub(crate) mod panel;
pub(crate) mod session;
pub(crate) mod worker;

use base::{coord::Coord, file_path};
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use prefab::{Prefab, PrefabInstanceId};
pub use vm::{
    AppearanceDelta,
    bake::{
        Bake, HIGHLIGHT_ALWAYS, HIGHLIGHT_EDGE_EAST, HIGHLIGHT_EDGE_NORTH, HIGHLIGHT_EDGE_SOUTH, HIGHLIGHT_EDGE_WEST,
        HIGHLIGHT_HOVERED, HIGHLIGHT_SELECTED, Highlight, PendingBake, SharedAppearance, highlight_tiles,
    },
    ui::{
        Command as UiCommand, Feedback as UiFeedback, Frame as UiFrame, PopupId as UiPopupId, Rebake as UiRebake,
        Value as UiValue,
    },
};
use vm::{
    bake::{Atom, Stage as BakeStage},
    world::Position,
};

#[cfg(test)]
use crate::document::MapDocument;
use crate::{
    document::grid::Grid,
    environment::{
        BakeProgram, Environment,
        progress::{Progress, Stage},
    },
    maploader::{
        RootSpecs,
        preview::{ComposeContext, MapView, ModulePreviewState},
        root_specs,
    },
};

fn atom(program: &BakeProgram, prefab: &Prefab, instance: u64, coord: Coord) -> Option<Atom> {
    Some(Atom {
        instance,
        ty: program.tree.id_of(&prefab.path)?,
        position: Position::new(coord.x as i32, coord.y as i32, coord.z as i32),
        vars: prefab
            .vars
            .iter()
            .map(|(name, value)| (name.clone(), value.value.clone()))
            .collect(),
    })
}

fn placed(environment: &Environment, view: MapView<'_>, id: PrefabInstanceId) -> Option<Atom> {
    let program = environment.bake_program.as_ref()?;
    let instance = view.prefab_instance(id)?;

    atom(program, instance.prefab(), instance.id.get(), instance.location.coord)
}

#[derive(Default)]
pub struct Standalone {
    world: Option<Bake>,
    baked: usize,
}

impl Standalone {
    const INSTANCE: u64 = 1;
    const LIFETIME: usize = 256;

    pub fn appearance(&mut self, environment: &Environment, prefab: &Prefab) -> Option<AppearanceDelta> {
        let program = environment.bake_program.as_ref()?;
        let atom = atom(program, prefab, Self::INSTANCE, Coord::new(1, 1, 1))?;
        if self.baked.is_multiple_of(Self::LIFETIME) {
            self.world = None;
        }
        self.baked += 1;

        let world = self.world.get_or_insert_with(|| {
            Bake::new_with_profile(
                &program.tree,
                &program.module,
                program.profile,
                Vec::new(),
                [1, 1, 1],
                environment.bake_options.limits,
                program.icon_states.clone(),
                Some(environment.base_dir().to_path_buf()),
            )
        });

        world.update(&program.tree, &program.module, vec![atom], &[]);
        let delta = world
            .appearances
            .get(&Self::INSTANCE)
            .map(|delta| AppearanceDelta::clone(delta));
        world.update(&program.tree, &program.module, Vec::new(), &[Self::INSTANCE]);

        for line in world.take_output() {
            log::info!("DM: {line}");
        }

        delta
    }
}

#[cfg(test)]
pub fn atoms(environment: &Environment, document: &MapDocument) -> Vec<Atom> {
    atoms_from_level(environment, document, 1)
}

#[cfg(test)]
pub fn atoms_from_level(environment: &Environment, document: &MapDocument, first: u32) -> Vec<Atom> {
    atoms_from_view(environment, (&document.map).into(), first)
}

pub fn atoms_from_view(environment: &Environment, view: MapView<'_>, first: u32) -> Vec<Atom> {
    let size = view.size();
    let mut atoms = Vec::new();
    for z in first..=size.z {
        for y in 1..=size.y {
            for x in 1..=size.x {
                for id in view.ids_at(Coord::new(x, y, z)) {
                    if let Some(atom) = placed(environment, view, *id) {
                        atoms.push(atom);
                    }
                }
            }
        }
    }

    atoms
}

#[cfg(test)]
pub fn size(document: &MapDocument) -> [i32; 3] {
    let size = document.map.size();

    [size.x as i32, size.y as i32, size.z as i32]
}

#[cfg(test)]
pub fn build_atoms(
    environment: &Environment, atoms: Vec<Atom>, size: [i32; 3], document_path: Option<&Path>, progress: &Progress,
) -> Option<Bake> {
    let program = environment.bake_program.as_ref()?;
    let mut current = None;

    Some(Bake::with_profile_and_progress(
        &program.tree,
        &program.module,
        program.profile,
        atoms,
        size,
        environment.bake_options.limits,
        program.icon_states.clone(),
        Some(environment.base_dir().to_path_buf()),
        document_path.and_then(|path| file_path::relative(environment.base_dir(), path)),
        report(progress, &mut current),
    ))
}

fn report(progress: &Progress, current: &mut Option<BakeStage>) -> impl FnMut(BakeStage, usize, usize) {
    move |stage, done, total| {
        if *current != Some(stage) {
            *current = Some(stage);
            progress.enter(Stage::from_bake(stage), total);
        }

        progress.set_done(done);
    }
}

pub fn levels(bake: &Bake) -> u32 {
    u32::try_from(bake.runtime.world.size[2]).unwrap_or(0)
}

// `atoms` are everything on the levels the bake doesn't have yet
pub fn extend(
    bake: &mut Bake, environment: &Environment, level_count: u32, atoms: Vec<Atom>, progress: &Progress,
) -> BakeUpdate {
    while levels(bake) < level_count {
        bake.append_level();
    }

    let ids = atoms
        .iter()
        .filter_map(|atom| PrefabInstanceId::from_raw(atom.instance))
        .collect::<Vec<_>>();
    let Some(program) = environment.bake_program.as_ref() else {
        return BakeUpdate {
            appearances: ids,
            lighting: None,
        };
    };

    let mut current = None;
    let update = bake.update_with_progress(
        &program.tree,
        &program.module,
        atoms,
        &[],
        report(progress, &mut current),
    );

    BakeUpdate {
        appearances: update
            .appearances
            .into_iter()
            .filter_map(PrefabInstanceId::from_raw)
            .collect(),
        lighting: update.lighting,
    }
}

#[cfg(test)]
pub fn build(environment: &Environment, document: &MapDocument) -> Option<Bake> {
    build_atoms(
        environment,
        atoms(environment, document),
        size(document),
        document.path.as_deref(),
        &Progress::new(),
    )
}

#[cfg(test)]
pub fn update(
    bake: &mut Bake, environment: &Environment, document: &MapDocument, affected: &[PrefabInstanceId],
) -> BakeUpdate {
    update_view(bake, environment, (&document.map).into(), affected)
}

pub fn update_view(
    bake: &mut Bake, environment: &Environment, view: MapView<'_>, affected: &[PrefabInstanceId],
) -> BakeUpdate {
    let mut affected = affected.to_vec();
    affected.sort_unstable();
    affected.dedup();
    let Some(program) = environment.bake_program.as_ref() else {
        return BakeUpdate {
            appearances: affected,
            lighting: None,
        };
    };

    let mut replacements = Vec::new();
    let mut removed = Vec::new();
    for id in &affected {
        match placed(environment, view, *id) {
            Some(atom) => replacements.push(atom),
            None => removed.push(id.get()),
        }
    }

    let update = bake.update(&program.tree, &program.module, replacements, &removed);
    BakeUpdate {
        appearances: update
            .appearances
            .into_iter()
            .filter_map(PrefabInstanceId::from_raw)
            .collect(),
        lighting: update.lighting,
    }
}

// a level the bake doesn't have yet is baked whole by its extension
pub fn update_known_levels(
    bake: &mut Bake, environment: &Environment, view: MapView<'_>, affected: &[PrefabInstanceId],
) -> BakeUpdate {
    let levels = levels(bake);
    let (baked, later) = affected
        .iter()
        .copied()
        .partition::<Vec<_>, _>(|id| view.location(*id).is_none_or(|location| location.coord.z <= levels));
    let mut update = update_view(bake, environment, view, &baked);
    update.appearances.extend(later);

    update
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BakeUpdate {
    pub appearances: Vec<PrefabInstanceId>,
    pub lighting: Option<Range<usize>>,
}

impl BakeUpdate {
    pub fn merge(&mut self, other: Self) {
        self.appearances.extend(other.appearances);
        self.appearances.sort_unstable();
        self.appearances.dedup();
        self.lighting = match (self.lighting.take(), other.lighting) {
            (Some(left), Some(right)) => Some(left.start.min(right.start)..left.end.max(right.end)),
            (left, right) => left.or(right),
        };
    }
}

pub fn appearances(bake: Option<&Bake>) -> &HashMap<u64, SharedAppearance> {
    static EMPTY: OnceLock<HashMap<u64, SharedAppearance>> = OnceLock::new();

    bake.map(|bake| &bake.appearances)
        .unwrap_or_else(|| EMPTY.get_or_init(HashMap::new))
}

trait ModuleTarget {
    fn specs(&self) -> RootSpecs;

    fn apply(&mut self, environment: &Environment, view: MapView<'_>, ids: &[PrefabInstanceId]);
}

struct Live<'a> {
    bake: &'a mut Bake,
    update: BakeUpdate,
}

impl ModuleTarget for Live<'_> {
    fn specs(&self) -> RootSpecs {
        root_specs(
            self.bake.modules(),
            self.bake.module_faults(),
            self.bake.module_errors(),
            self.bake.coordinate_modules(),
        )
    }

    fn apply(&mut self, environment: &Environment, view: MapView<'_>, ids: &[PrefabInstanceId]) {
        let update = update_known_levels(self.bake, environment, view, ids);
        self.update.merge(update);
    }
}

struct Pending<'a, 'b> {
    pending: &'a mut PendingBake<'b>,
    progress: &'a Progress,
}

impl ModuleTarget for Pending<'_, '_> {
    fn specs(&self) -> RootSpecs {
        root_specs(
            self.pending.modules(),
            self.pending.module_faults(),
            self.pending.module_errors(),
            self.pending.coordinate_modules(),
        )
    }

    fn apply(&mut self, environment: &Environment, view: MapView<'_>, ids: &[PrefabInstanceId]) {
        let mut replacements = Vec::new();
        let mut removed = Vec::new();
        for id in ids {
            match placed(environment, view, *id) {
                Some(atom) => replacements.push(atom),
                None => removed.push(id.get()),
            }
        }

        let mut current = None;
        self.pending
            .replace_atoms(replacements, &removed, report(self.progress, &mut current));
    }
}

// the bake reports which atoms are roots, and it is fed the composite those roots produce
fn settle_target(
    target: &mut impl ModuleTarget, environment: &Environment, source: &Grid, document_path: Option<&Path>,
    modules: &mut ModulePreviewState,
) -> Vec<PrefabInstanceId> {
    let mut all = HashSet::new();
    loop {
        let specs = target.specs();
        if !modules.needs_recompose(&specs) {
            break;
        }

        let ctx = ComposeContext {
            source,
            tree: &environment.tree,
            base_dir: environment.base_dir(),
            document_path,
        };
        let changed = modules.recompose(ctx, specs, &HashSet::new());
        if changed.is_empty() {
            break;
        }

        target.apply(environment, modules.view(source), &changed);
        all.extend(changed);
    }

    all.into_iter().collect()
}

pub fn settle_modules(
    bake: &mut Bake, environment: &Environment, source: &Grid, document_path: Option<&Path>,
    modules: &mut ModulePreviewState,
) -> BakeUpdate {
    let mut live = Live {
        bake,
        update: BakeUpdate::default(),
    };
    let ids = settle_target(&mut live, environment, source, document_path, modules);
    let mut update = live.update;
    update.merge(BakeUpdate {
        appearances: ids,
        lighting: None,
    });

    update
}

pub fn build_map(
    environment: &Environment, source: Grid, document_path: Option<PathBuf>, mut modules: ModulePreviewState,
    progress: &Progress,
) -> Option<(Bake, ModulePreviewState)> {
    let program = environment.bake_program.as_ref()?;
    let size = source.size();
    let mut current = None;
    let mut pending = Bake::initialize_with_profile_and_progress(
        &program.tree,
        &program.module,
        program.profile,
        atoms_from_view(environment, (&source).into(), 1),
        [size.x as i32, size.y as i32, size.z as i32],
        environment.bake_options.limits,
        program.icon_states.clone(),
        Some(environment.base_dir().to_path_buf()),
        document_path
            .as_deref()
            .and_then(|path| file_path::relative(environment.base_dir(), path)),
        report(progress, &mut current),
    );

    if pending.modules().next().is_some() || pending.module_faults().next().is_some() {
        progress.enter(Stage::Modules, 0);
    }

    let mut target = Pending {
        pending: &mut pending,
        progress,
    };
    settle_target(
        &mut target,
        environment,
        &source,
        document_path.as_deref(),
        &mut modules,
    );

    let mut current = None;
    let bake = pending.finish_with_progress(report(progress, &mut current));

    Some((bake, modules))
}

#[cfg(test)]
mod tests;
