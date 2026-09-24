use std::{collections::HashMap, ops::Range, sync::OnceLock};

use dmm::{Coord, Prefab, PrefabInstanceId};
pub use vm::{
    AppearanceDelta,
    bake::{
        Bake,
        HIGHLIGHT_ALWAYS,
        HIGHLIGHT_EDGE_EAST,
        HIGHLIGHT_EDGE_NORTH,
        HIGHLIGHT_EDGE_SOUTH,
        HIGHLIGHT_EDGE_WEST,
        HIGHLIGHT_HOVERED,
        HIGHLIGHT_SELECTED,
        Highlight,
        HighlightTile,
        highlight_tiles,
    },
    ui::{
        Command as UiCommand,
        Feedback as UiFeedback,
        Frame as UiFrame,
        PopupId as UiPopupId,
        Rebake as UiRebake,
        Value as UiValue,
    },
};

use crate::{
    BakeProgram,
    Environment,
    document::MapDocument,
    progress::{Progress, Stage},
};

fn atom(program: &BakeProgram, prefab: &Prefab, instance: u64, coord: Coord) -> Option<vm::bake::Atom> {
    Some(vm::bake::Atom {
        instance,
        ty: program.tree.id_of(&prefab.path)?,
        position: vm::world::Position::new(coord.x as i32, coord.y as i32, coord.z as i32),
        vars: prefab
            .vars
            .iter()
            .map(|(name, value)| (name.clone(), value.value.clone()))
            .collect(),
    })
}

fn placed(environment: &Environment, document: &MapDocument, id: PrefabInstanceId) -> Option<vm::bake::Atom> {
    let program = environment.bake_program.as_ref()?;
    let (prefab, location) = document.prefab_instance(id)?;

    atom(program, prefab, id.get(), location.coord)
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
            )
        });

        world.update(&program.tree, &program.module, vec![atom], &[]);
        let delta = world.appearances.get(&Self::INSTANCE).cloned();
        world.update(&program.tree, &program.module, Vec::new(), &[Self::INSTANCE]);

        for line in world.take_output() {
            log::info!("DM: {line}");
        }

        delta
    }
}

pub fn atoms(environment: &Environment, document: &MapDocument) -> Vec<vm::bake::Atom> {
    let map = &document.map;
    let mut atoms = Vec::new();
    for z in 1..=map.size.z {
        for y in 1..=map.size.y {
            for x in 1..=map.size.x {
                for id in document.instance_ids_at(Coord::new(x, y, z)) {
                    if let Some(atom) = placed(environment, document, *id) {
                        atoms.push(atom);
                    }
                }
            }
        }
    }

    atoms
}

pub fn size(document: &MapDocument) -> [i32; 3] {
    let size = document.map.size;

    [size.x as i32, size.y as i32, size.z as i32]
}

pub fn build_atoms(
    environment: &Environment, atoms: Vec<vm::bake::Atom>, size: [i32; 3], progress: &Progress,
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
        |stage, done, total| {
            if current != Some(stage) {
                current = Some(stage);
                progress.enter(Stage::from_bake(stage), total);
            }

            progress.set_done(done);
        },
    ))
}

pub fn build(environment: &Environment, document: &MapDocument) -> Option<Bake> {
    build_atoms(
        environment,
        atoms(environment, document),
        size(document),
        &Progress::new(),
    )
}

pub fn update(
    bake: &mut Bake, environment: &Environment, document: &MapDocument, affected: &[PrefabInstanceId],
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
        match placed(environment, document, *id) {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BakeUpdate {
    pub appearances: Vec<PrefabInstanceId>,
    pub lighting: Option<Range<usize>>,
}

pub fn appearances(bake: Option<&Bake>) -> &HashMap<u64, AppearanceDelta> {
    static EMPTY: OnceLock<HashMap<u64, AppearanceDelta>> = OnceLock::new();

    bake.map(|bake| &bake.appearances)
        .unwrap_or_else(|| EMPTY.get_or_init(HashMap::new))
}

#[cfg(test)]
mod tests {
    use core::{arena::StrArena, path::TreePath};
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use dmm::{Map, Size};

    use super::*;
    use crate::{command::Edit, frame, visual};

    fn examples() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env") }

    fn environment(profile: &'static str) -> Environment {
        let root = examples();
        let compile = |baking| {
            let arena = StrArena::new();
            let prelude = preprocessor::prelude_files().into_iter().chain([
                preprocessor::PreludeFile::Embedded("<test-profile.dm>", profile),
                preprocessor::PreludeFile::Embedded(
                    "<test-profile-default.dm>",
                    "/datum/demir/test\n    default = TRUE\n",
                ),
            ]);
            let preprocessed = preprocessor::Preprocessor::new(&arena)
                .with_prelude(prelude)
                .with_baking(baking)
                .run(root.join("test.dm"))
                .expect("preprocess");
            assert!(preprocessed.is_ok(), "{:?}", preprocessed.errors);

            let ast = ast::parse(&preprocessed.tokens).expect("parse");
            let (tree, module, errors) = sema::analyze(&ast, baking);
            assert!(errors.is_empty(), "{errors:?}");

            (tree, module)
        };
        let (editor_tree, _) = compile(false);
        let (bake_tree, module) = compile(true);
        let profile = vm::bake::profile_type(&bake_tree).expect("default profile");
        let mut environment = Environment::new(root.join("test.dme"), editor_tree);
        environment.bake_program = Some(BakeProgram {
            tree: bake_tree,
            module: codegen::generate(&module).expect("codegen"),
            profile,
            files: Default::default(),
            icon_states: Default::default(),
        });
        let failures = environment.load_icons(&[], &crate::progress::Progress::new());
        assert!(failures.is_empty(), "{failures:?}");

        environment
    }

    fn options<'a>(visibility: &'a frame::TypeVisibility, bake: &'a Bake) -> frame::FrameRenderOptions<'a> {
        frame::FrameRenderOptions {
            visibility,
            tile_size: 32,
            appearances: appearances(Some(bake)),
            lighting: bake.lighting.as_ref(),
        }
    }

    fn var<'a>(bake: &'a Bake, id: PrefabInstanceId, name: &str) -> Option<&'a str> {
        bake.appearances
            .get(&id.get())
            .and_then(|delta| delta.vars.iter().find(|(current, _)| current.as_str() == name))
            .and_then(|(_, value)| value.as_text())
    }

    const WALLS: &str = r#"
/datum/demir/test/bake(atom/target)
    if(!istype(target, /turf/closed/wall))
        return
    var/junction = 0
    for(var/direction in list(1, 2, 4, 8))
        if(istype(get_step(target, direction), /turf/closed/wall))
            junction |= direction
    target.name = "[junction]"
    target.icon_state = junction ? "wall" : "floor"
    target.overlays += "light"
"#;

    #[test]
    fn compatibility_appearances_and_types_survive_runtime_baking() {
        static NEXT: AtomicUsize = AtomicUsize::new(0);

        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("demir-compat-bake-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&root).expect("temp dir");
        std::fs::copy(examples().join("icons/test.dmi"), root.join("editor.dmi")).expect("editor icon");
        std::fs::copy(examples().join("icons/test.dmi"), root.join("runtime.dmi")).expect("runtime icon");
        let entry = root.join("test.dm");
        std::fs::write(
            &entry,
            r#"
#ifdef __DEMIR_BAKE__
#define RUNTIME_BUILD
#endif

/obj/plain
#ifdef RUNTIME_BUILD
    icon = 'runtime.dmi'
    icon_state = "table"
#else
    icon = 'editor.dmi'
    icon_state = "floor"
#endif

/obj/smoothed
#ifdef RUNTIME_BUILD
    icon = 'runtime.dmi'
    icon_state = "table"
#else
    icon = 'editor.dmi'
    icon_state = "floor"
#endif

#ifdef FASTDMM
/obj/map_helper
    icon_state = "helper"
#endif

#ifdef __DEMIR_BAKE__
#include "profile.dm"
#endif
"#,
        )
        .expect("fixture");
        std::fs::write(
            root.join("profile.dm"),
            r#"
/datum/demir/test
    default = TRUE
/datum/demir/test/bake(atom/target)
    if(istype(target, /obj/smoothed))
        target.icon_state = "wall"
"#,
        )
        .expect("profile");

        let (mut environment, diagnostics) = Environment::load(&entry).expect("environment");
        std::fs::remove_dir_all(&root).expect("remove temp dir");
        assert!(diagnostics.is_empty(), "unexpected diagnostics");

        let plain = Prefab::new(TreePath::parse("/obj/plain"));
        let smoothed = Prefab::new(TreePath::parse("/obj/smoothed"));
        let helper = Prefab::new(TreePath::parse("/obj/map_helper"));
        let plain_id = environment.tree.id_of(&plain.path).expect("editor plain");
        let smoothed_id = environment.tree.id_of(&smoothed.path).expect("editor smoothed");
        let helper_id = environment.tree.id_of(&helper.path).expect("editor helper");
        let program = environment.bake_program.as_ref().expect("bake program");
        let profile = vm::bake::profile_type(&program.tree).expect("bake profile");
        let definition = vm::profile::ProfileDefinition::resolve(&program.tree, profile);
        let hook = definition
            .declaration(&program.tree, vm::profile::ProfileHook::Bake)
            .expect("bake hook");

        assert_eq!(
            visual::resolve_id(&environment.tree, plain_id, &plain).icon.as_deref(),
            Some("editor.dmi")
        );
        assert_eq!(
            visual::resolve_id(&environment.tree, plain_id, &plain)
                .icon_state
                .as_deref(),
            Some("floor")
        );
        assert_eq!(
            visual::resolve_id(
                &program.tree,
                program.tree.id_of(&plain.path).expect("runtime plain"),
                &plain
            )
            .icon
            .as_deref(),
            Some("runtime.dmi")
        );
        assert_eq!(
            visual::resolve_id(
                &program.tree,
                program.tree.id_of(&plain.path).expect("runtime plain"),
                &plain
            )
            .icon_state
            .as_deref(),
            Some("table")
        );
        assert!(program.tree.id_of(&helper.path).is_none());
        assert_eq!(
            visual::resolve_id(&environment.tree, helper_id, &helper)
                .icon_state
                .as_deref(),
            Some("helper")
        );
        assert_eq!(
            environment
                .bake_file(hook.location.file)
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str()),
            Some("profile.dm")
        );

        let mut standalone = Standalone::default();
        let plain_delta = standalone.appearance(&environment, &plain).expect("plain bake result");
        let plain_appearance = visual::resolve_delta(&environment.tree, plain_id, &plain, &plain_delta);
        assert_eq!(plain_appearance.icon.as_deref(), Some("editor.dmi"));
        assert_eq!(plain_appearance.icon_state.as_deref(), Some("floor"));
        assert!(plain_delta.vars.is_empty());

        let smoothed_delta = standalone
            .appearance(&environment, &smoothed)
            .expect("smoothed bake result");
        let smoothed_appearance = visual::resolve_delta(&environment.tree, smoothed_id, &smoothed, &smoothed_delta);
        assert_eq!(smoothed_appearance.icon.as_deref(), Some("runtime.dmi"));
        assert_eq!(smoothed_appearance.icon_state.as_deref(), Some("wall"));
        environment
            .icons
            .get_mut("editor.dmi")
            .expect("editor icon metadata")
            .states
            .retain(|state| state.name != "wall");
        let file = dmi::IconFile::load(examples().join("icons/test.dmi")).expect("test icon");
        let mut textures = render::texture::TextureCatalog::new();
        textures.insert("editor.dmi", &file).expect("pack editor icon");
        textures.insert("runtime.dmi", &file).expect("pack runtime icon");
        assert!(frame::sprite_texture(&environment.icons, &textures, &smoothed_appearance).is_some());
        assert!(standalone.appearance(&environment, &helper).is_none());

        let mut map = Map::new(Size { x: 1, y: 1, z: 1 });
        let key = map.intern_tile(vec![plain, smoothed, helper]);
        map.grid[0][0][0] = key;
        let document = MapDocument::new(map, 1);
        assert_eq!(atoms(&environment, &document).len(), 2);
    }

    #[test]
    fn only_a_codebase_with_a_profile_bakes() {
        let root = examples();
        let (profiled, _) = Environment::load(root.join("test.dme")).expect("profiled codebase");
        let (bare, _) = Environment::load(root.join("test.dm")).expect("codebase without a profile");
        let disabled = crate::environment::BakeOptions {
            enabled: false,
            ..Default::default()
        };
        let (disabled, _) = Environment::load_with(root.join("test.dme"), disabled, &crate::progress::Progress::new())
            .expect("codebase with baking off");
        let phi_only = crate::environment::BakeOptions {
            optimizations_enabled: false,
            ..Default::default()
        };
        let (phi_only, _) = Environment::load_with(root.join("test.dme"), phi_only, &crate::progress::Progress::new())
            .expect("codebase with optional optimizations off");

        assert!(profiled.bake_program.is_some());
        assert!(bare.bake_program.is_none());
        assert!(disabled.bake_program.is_none());
        assert!(phi_only.bake_program.is_some());
        assert_eq!(profiled.optimization_timings.samples(), 2);
        assert_eq!(disabled.optimization_timings.samples(), 1);
        assert_eq!(phi_only.optimization_timings.samples(), 2);

        let source = std::fs::read_to_string(root.join("test.dmm")).expect("example map");
        let (map, errors) = dmm::parser::parse(&source);
        assert!(errors.is_empty(), "{errors:?}");
        let document = MapDocument::new(map, 1);
        let bake = build(&profiled, &document).expect("the example profile bakes");
        let named = bake
            .appearances
            .values()
            .flat_map(|delta| &delta.vars)
            .filter(|(name, _)| name.as_str() == "name")
            .filter_map(|(_, value)| value.as_text())
            .collect::<Vec<_>>();

        assert!(named.iter().any(|name| name.starts_with("wall ")), "{named:?}");
        assert!(!named.contains(&"wall"), "{named:?}");
        assert!(build(&bare, &document).is_none());
    }

    #[test]
    fn overlays_follow_the_owner_dir_and_sort_by_layer() {
        let environment = environment(
            r#"
/datum/demir/test/bake(atom/target)
    if(!istype(target, /obj/structure/table))
        return
    target.dir = 4
    var/image/above = new
    above.icon_state = "light"
    above.layer = -1
    var/image/below = new
    below.icon_state = "floor"
    below.layer = 1
    target.overlays += above
    target.overlays += below
"#,
        );
        let mut map = Map::new(Size { x: 1, y: 1, z: 1 });
        let key = map.intern_tile(vec![Prefab::new(TreePath::parse("/obj/structure/table"))]);
        map.grid[0] = vec![vec![key]];
        let document = MapDocument::new(map, 1);
        let bake = build(&environment, &document).expect("baking is on");
        assert_eq!(bake.diagnostics.count(), 0, "{:?}", bake.diagnostics);

        let table = document.instance_ids_at(Coord::new(1, 1, 1))[0];
        let (prefab, _) = document.prefab_instance(table).expect("table");
        let ty = environment.tree.id_of(&prefab.path).expect("table type");
        let delta = &bake.appearances[&table.get()];
        let owner = visual::resolve_delta(&environment.tree, ty, prefab, delta);
        let above = visual::resolve_overlay(&environment.tree, &owner, &delta.overlays[0]);

        assert_eq!(owner.dir, 4);
        assert_eq!(above.dir, 4);

        let mut textures = render::texture::TextureCatalog::new();
        textures
            .insert(
                "icons/test.dmi",
                &dmi::IconFile::load(examples().join("icons/test.dmi")).expect("test icon"),
            )
            .expect("pack test icon");
        let visibility = frame::TypeVisibility::default();
        let instances = frame::build_with_options(
            &environment.tree,
            &environment.icons,
            &textures,
            &document,
            options(&visibility, &bake),
        );
        let depths = instances.sprites.iter().map(|sprite| sprite.depth).collect::<Vec<_>>();

        assert_eq!(depths, [1.0, owner.layer, owner.layer]);
    }

    #[test]
    fn derived_sprites_follow_edits_and_history_without_changing_map_bytes() {
        let environment = environment(WALLS);
        let mut map = Map::new(Size { x: 3, y: 3, z: 1 });
        let key = map.intern_tile(vec![Prefab::new(TreePath::parse("/turf/closed/wall"))]);
        map.grid[0] = vec![vec![key; 3]; 3];
        let mut document = MapDocument::new(map, 1);
        let bytes = dmm::writer::MapWriter::new(&document.map).write();

        let mut bake = build(&environment, &document).expect("baking is on");
        let before = bake.appearances.clone();
        let center = document.instance_ids_at(Coord::new(2, 2, 1))[0];
        let south = document.instance_ids_at(Coord::new(2, 1, 1))[0];

        assert_eq!(bake.diagnostics.count(), 0, "{:?}", bake.diagnostics);
        assert_eq!(dmm::writer::MapWriter::new(&document.map).write(), bytes);

        let (prefab, _) = document.prefab_instance(center).expect("center wall");
        let ty = environment.tree.id_of(&prefab.path).expect("wall type");
        let appearance = visual::resolve_delta(&environment.tree, ty, prefab, &bake.appearances[&center.get()]);

        assert_eq!(appearance.name.as_deref(), Some("15"));

        let mut textures = render::texture::TextureCatalog::new();
        textures
            .insert(
                "icons/test.dmi",
                &dmi::IconFile::load(examples().join("icons/test.dmi")).expect("test icon"),
            )
            .expect("pack test icon");
        let visibility = frame::TypeVisibility::default();
        let mut instances = frame::build_with_options(
            &environment.tree,
            &environment.icons,
            &textures,
            &document,
            options(&visibility, &bake),
        );

        // Every wall draws itself and its `light` overlay.
        assert_eq!(instances.sprites.len(), 18);

        let mut edit = Edit::new("remove wall");
        edit.change(&document, Coord::new(2, 2, 1), vec![]);
        assert!(document.apply(edit));
        let affected = update(&mut bake, &environment, &document, &[center]);

        assert_eq!(var(&bake, south, "name"), Some("12"));

        frame::update_prefabs_with_options(
            &mut instances,
            &environment.tree,
            &environment.icons,
            &textures,
            &document,
            &affected.appearances,
            options(&visibility, &bake),
        );
        let rebuilt = frame::build_with_options(
            &environment.tree,
            &environment.icons,
            &textures,
            &document,
            options(&visibility, &bake),
        );

        assert_eq!(instances.sprites, rebuilt.sprites);

        let affected = document.undo_with_affected().expect("undo");
        update(&mut bake, &environment, &document, &affected);

        assert_eq!(bake.appearances, before);

        let affected = document.redo_with_affected().expect("redo");
        update(&mut bake, &environment, &document, &affected);

        assert!(!bake.appearances.contains_key(&center.get()));
    }

    #[test]
    fn movable_smoothing_follows_object_add_undo_and_redo() {
        let environment = environment(
            r#"
/obj/structure/table/proc/smooth_icon()
    var/junction = 0
    for(var/direction in list(1, 2, 4, 8))
        var/turf/neighbor = get_step(src, direction)
        for(var/obj/structure/table/table in neighbor)
            junction |= direction
    icon_state = "table-[junction]"

/datum/demir/test/bake(atom/target)
    if(istype(target, /obj/structure/table))
        var/obj/structure/table/table = target
        table.smooth_icon()
"#,
        );
        let floor = Prefab::new(TreePath::parse("/turf/open/floor"));
        let table = Prefab::new(TreePath::parse("/obj/structure/table"));
        let mut map = Map::new(Size { x: 3, y: 1, z: 1 });
        let occupied = map.intern_tile(vec![floor.clone(), table.clone()]);
        let empty = map.intern_tile(vec![floor]);
        map.grid[0][0] = vec![occupied, empty, empty];
        let mut document = MapDocument::new(map, 1);
        let left = document.instance_ids_at(Coord::new(1, 1, 1))[1];
        let mut bake = build(&environment, &document).expect("baking is on");

        assert_eq!(var(&bake, left, "icon_state"), Some("table-0"));

        let coord = Coord::new(2, 1, 1);
        let right = document.instantiate(table);
        let right_id = right.id();
        let mut after = document.placed_tile(coord).expect("placed floor");
        after.push(right);
        let mut edit = Edit::new("add table");
        edit.change(&document, coord, after);
        assert!(document.apply(edit));
        update(&mut bake, &environment, &document, &[right_id]);
        let connected = bake.appearances.clone();

        assert_eq!(var(&bake, left, "icon_state"), Some("table-4"));
        assert_eq!(var(&bake, right_id, "icon_state"), Some("table-8"));

        let affected = document.undo_with_affected().expect("undo");
        update(&mut bake, &environment, &document, &affected);

        assert_eq!(var(&bake, left, "icon_state"), Some("table-0"));
        assert!(!bake.appearances.contains_key(&right_id.get()));

        let affected = document.redo_with_affected().expect("redo");
        update(&mut bake, &environment, &document, &affected);

        assert_eq!(bake.appearances, connected);
    }

    #[test]
    fn a_standalone_bake_runs_the_profile_against_an_empty_one_cell_world() {
        let mut environment = environment(WALLS);
        let prefab = Prefab::new(TreePath::parse("/turf/closed/wall"));
        let ty = environment.tree.id_of(&prefab.path).expect("wall type");
        let mut standalone = Standalone::default();
        let delta = standalone
            .appearance(&environment, &prefab)
            .expect("standalone appearance");
        let appearance = visual::resolve_delta(&environment.tree, ty, &prefab, &delta);

        assert_eq!(appearance.name.as_deref(), Some("0"));

        let missing = Prefab::new(TreePath::parse("/turf/closed/nonexistent"));

        assert!(standalone.appearance(&environment, &missing).is_none());

        // the world is shared, so a second prefab must still bake against an otherwise empty cell
        let repeated = standalone
            .appearance(&environment, &prefab)
            .expect("standalone appearance");

        assert_eq!(repeated, delta);

        environment.bake_program = None;

        assert!(standalone.appearance(&environment, &prefab).is_none());
    }
}
