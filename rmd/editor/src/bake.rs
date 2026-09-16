use std::{collections::HashMap, sync::OnceLock};

use dmm::{Coord, Prefab, PrefabInstanceId};
pub use vm::{AppearanceDelta, bake::Bake};

use crate::{Environment, document::MapDocument};

fn atom(environment: &Environment, prefab: &Prefab, instance: u64, coord: Coord) -> Option<vm::bake::Atom> {
    Some(vm::bake::Atom {
        instance,
        ty: environment.tree.id_of(&prefab.path)?,
        position: vm::world::Position::new(coord.x as i32, coord.y as i32, coord.z as i32),
        vars: prefab
            .vars
            .iter()
            .map(|(name, value)| (name.clone(), value.value.clone()))
            .collect(),
    })
}

fn placed(environment: &Environment, document: &MapDocument, id: PrefabInstanceId) -> Option<vm::bake::Atom> {
    let (prefab, location) = document.prefab_instance(id)?;

    atom(environment, prefab, id.get(), location.coord)
}

pub fn standalone(environment: &Environment, prefab: &Prefab) -> Option<AppearanceDelta> {
    let module = environment.module.as_ref()?;
    let atom = atom(environment, prefab, 1, Coord::new(1, 1, 1))?;
    let mut bake = Bake::new(
        &environment.tree,
        module,
        vec![atom],
        [1, 1, 1],
        environment.bake_options.limits,
    );

    for line in bake.take_output() {
        log::info!("DM: {line}");
    }

    bake.appearances.get(&1).cloned()
}

pub fn build(environment: &Environment, document: &MapDocument) -> Option<Bake> {
    let module = environment.module.as_ref()?;
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

    Some(Bake::with_progress(
        &environment.tree,
        module,
        atoms,
        [map.size.x as i32, map.size.y as i32, map.size.z as i32],
        environment.bake_options.limits,
        |stage, done, total| {
            if done == 0 || done == total {
                log::info!("DM {stage:?}: {done}/{total}");
            }
        },
    ))
}

pub fn update(
    bake: &mut Bake, environment: &Environment, document: &MapDocument, affected: &[PrefabInstanceId],
) -> Vec<PrefabInstanceId> {
    let Some(module) = environment.module.as_ref() else {
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

    bake.update(&environment.tree, module, replacements, &removed)
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
    use std::path::PathBuf;

    use dmm::{Map, Size};

    use super::*;
    use crate::{command::Edit, frame, visual};

    fn examples() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env") }

    fn environment(profile: &'static str) -> Environment {
        let root = examples();
        let arena = StrArena::new();
        let prelude = preprocessor::prelude_files()
            .into_iter()
            .chain([preprocessor::PreludeFile::Embedded("<test-profile.dm>", profile)]);
        let preprocessed = preprocessor::Preprocessor::new(&arena)
            .with_prelude(prelude)
            .with_baking(true)
            .run(root.join("test.dm"))
            .expect("preprocess");
        assert!(preprocessed.is_ok(), "{:?}", preprocessed.errors);

        let ast = ast::parse(&preprocessed.tokens).expect("parse");
        let (tree, module, errors) = sema::analyze(&ast);
        assert!(errors.is_empty(), "{errors:?}");

        let mut environment = Environment::new(root.join("test.dme"), tree);
        environment.module = Some(codegen::generate(&module).expect("codegen"));
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

        assert!(profiled.module.is_some());
        assert!(bare.module.is_none());
        assert!(disabled.module.is_none());

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
        let delta = standalone(&environment, &prefab).expect("standalone appearance");
        let appearance = visual::resolve_delta(&environment.tree, ty, &prefab, &delta);

        assert_eq!(appearance.name.as_deref(), Some("0"));
        assert!(standalone(&environment, &Prefab::new(TreePath::parse("/turf/closed/nonexistent"))).is_none());

        environment.module = None;

        assert!(standalone(&environment, &prefab).is_none());
    }
}
