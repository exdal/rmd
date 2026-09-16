use std::{collections::HashMap, sync::OnceLock};

use dmm::{Coord, Prefab, PrefabInstanceId};
pub use vm::{AppearanceDelta, bake::Bake};

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
            Bake::new(
                &program.tree,
                &program.module,
                Vec::new(),
                [1, 1, 1],
                environment.bake_options.limits,
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

    Some(Bake::with_progress(
        &program.tree,
        &program.module,
        atoms,
        size,
        environment.bake_options.limits,
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
) -> Vec<PrefabInstanceId> {
    let Some(program) = environment.bake_program.as_ref() else {
        return affected.to_vec();
    };

    let mut replacements = Vec::new();
    let mut removed = Vec::new();
    for id in affected {
        match placed(environment, document, *id) {
            Some(atom) => replacements.push(atom),
            None => removed.push(id.get()),
        }
    }

    bake.update(&program.tree, &program.module, replacements, &removed)
        .into_iter()
        .filter_map(PrefabInstanceId::from_raw)
        .collect()
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
            let prelude = preprocessor::prelude_files()
                .into_iter()
                .chain([preprocessor::PreludeFile::Embedded("<test-profile.dm>", profile)]);
            let preprocessed = preprocessor::Preprocessor::new(&arena)
                .with_prelude(prelude)
                .with_baking(baking)
                .run(root.join("test.dm"))
                .expect("preprocess");
            assert!(preprocessed.is_ok(), "{:?}", preprocessed.errors);

            let ast = ast::parse(&preprocessed.tokens).expect("parse");
            let (tree, module, errors) = sema::analyze(&ast);
            assert!(errors.is_empty(), "{errors:?}");

            (tree, module)
        };
        let (editor_tree, _) = compile(false);
        let (bake_tree, module) = compile(true);
        let mut environment = Environment::new(root.join("test.dme"), editor_tree);
        environment.bake_program = Some(BakeProgram {
            tree: bake_tree,
            module: codegen::generate(&module).expect("codegen"),
            files: Default::default(),
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
        }
    }

    fn var<'a>(bake: &'a Bake, id: PrefabInstanceId, name: &str) -> Option<&'a str> {
        bake.appearances
            .get(&id.get())
            .and_then(|delta| delta.vars.iter().find(|(current, _)| current.as_str() == name))
            .and_then(|(_, value)| value.as_text())
    }

    const WALLS: &str = r#"
/proc/demir_bake(atom/target)
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
        let entry = root.join("test.dm");
        std::fs::write(
            &entry,
            r#"
/obj/plain
#ifdef FASTDMM
    icon_state = "editor"
#else
    icon_state = "runtime"
#endif

/obj/smoothed
#ifdef FASTDMM
    icon_state = "editor"
#else
    icon_state = "runtime"
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
/proc/demir_bake(atom/target)
    if(istype(target, /obj/smoothed))
        target.icon_state = "baked"
"#,
        )
        .expect("profile");

        let (environment, diagnostics) = Environment::load(&entry).expect("environment");
        std::fs::remove_dir_all(&root).expect("remove temp dir");
        assert!(diagnostics.is_empty(), "unexpected diagnostics");

        let plain = Prefab::new(TreePath::parse("/obj/plain"));
        let smoothed = Prefab::new(TreePath::parse("/obj/smoothed"));
        let helper = Prefab::new(TreePath::parse("/obj/map_helper"));
        let plain_id = environment.tree.id_of(&plain.path).expect("editor plain");
        let smoothed_id = environment.tree.id_of(&smoothed.path).expect("editor smoothed");
        let helper_id = environment.tree.id_of(&helper.path).expect("editor helper");
        let program = environment.bake_program.as_ref().expect("bake program");
        let hook = program
            .tree
            .proc_inherited(objtree::TypeId::ROOT, &"demir_bake".into())
            .expect("bake hook");

        assert_eq!(
            visual::resolve_id(&environment.tree, plain_id, &plain)
                .icon_state
                .as_deref(),
            Some("editor")
        );
        assert_eq!(
            visual::resolve_id(
                &program.tree,
                program.tree.id_of(&plain.path).expect("runtime plain"),
                &plain
            )
            .icon_state
            .as_deref(),
            Some("runtime")
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
        assert_eq!(plain_appearance.icon_state.as_deref(), Some("editor"));
        assert!(plain_delta.vars.is_empty());

        let smoothed_delta = standalone
            .appearance(&environment, &smoothed)
            .expect("smoothed bake result");
        let smoothed_appearance = visual::resolve_delta(&environment.tree, smoothed_id, &smoothed, &smoothed_delta);
        assert_eq!(smoothed_appearance.icon_state.as_deref(), Some("baked"));
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

        assert!(profiled.bake_program.is_some());
        assert!(bare.bake_program.is_none());
        assert!(disabled.bake_program.is_none());

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
            &affected,
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

/proc/demir_bake(atom/target)
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
