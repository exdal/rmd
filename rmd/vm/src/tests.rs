use core::{
    arena::StrArena,
    path::TreePath,
    types::{Identifier, Value},
};
use std::sync::atomic::{AtomicUsize, Ordering};

use objtree::{ObjectTree, TypeId};

use crate::{
    FaultKind,
    GenericValue,
    IconStates,
    Limits,
    Runtime,
    bake::{Atom, Bake, has_profile},
    eval::Evaluator,
    heap::{Object, ObjectId},
    world::Position,
};

fn compile(source: &str) -> (ObjectTree, codegen::Module) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("rmd-vm-fixture-{}-{id}.dm", std::process::id()));
    std::fs::write(&path, source).expect("write fixture");
    let arena = StrArena::new();
    let preprocessed = preprocessor::preprocess(&arena, &path).expect("fixture should preprocess");
    let _ = std::fs::remove_file(path);
    assert!(preprocessed.errors.is_empty(), "{:?}", preprocessed.errors);
    let ast = ast::parse(&preprocessed.tokens).expect("fixture should parse");
    let (tree, module, errors) = sema::analyze(&ast);
    assert!(errors.is_empty(), "{errors:?}");
    let module = codegen::generate(&module).expect("fixture should compile");
    (tree, module)
}

fn proc(tree: &ObjectTree, name: &str) -> core::types::ProcId {
    tree.proc_inherited(TypeId::ROOT, &name.into())
        .and_then(|proc| proc.body)
        .expect("fixture proc should exist")
}

fn run(source: &str, name: &str) -> GenericValue {
    let (tree, module) = compile(source);
    Runtime::default()
        .run(&tree, &module, proc(&tree, name), None, Vec::new(), Limits::default())
        .expect("fixture should execute")
}

const WALLS: &str = r#"
/turf/wall
    icon_state = "static"
/proc/demir_bake(atom/target)
    if(!istype(target, /turf/wall))
        return
    var/junction = 0
    for(var/direction in list(1, 2, 4, 8))
        if(istype(get_step(target, direction), /turf/wall))
            junction |= direction
    target.icon_state = "wall-[junction]"
"#;

fn wall_patch(tree: &ObjectTree) -> Vec<Atom> {
    let ty = tree
        .id_of(&TreePath::parse("/turf/wall"))
        .expect("wall type should exist");
    let mut atoms = Vec::new();
    for y in 1..=3 {
        for x in 1..=3 {
            atoms.push(Atom {
                instance: ((y - 1) * 3 + x) as u64,
                ty,
                position: Position::new(x, y, 1),
                vars: Vec::new(),
            });
        }
    }

    atoms
}

fn baked_state(bake: &Bake, id: u64) -> Option<&str> {
    bake.appearances
        .get(&id)?
        .vars
        .iter()
        .find(|(name, _)| name.as_str() == "icon_state")?
        .1
        .as_text()
}

#[test]
fn only_hook_bodies_make_a_profile() {
    let (bare, _) = compile("/turf/wall\n    proc/Initialize(mapload)\n        return\n");
    let (profiled, _) = compile(WALLS);

    assert!(!has_profile(&bare));
    assert!(has_profile(&profiled));
}

#[test]
fn baking_updates_a_neighborhood_and_restores_removed_atoms() {
    let (tree, module) = compile(WALLS);
    let atoms = wall_patch(&tree);
    let original = atoms.clone();
    let mut bake = Bake::new(
        &tree,
        &module,
        atoms,
        [3, 3, 1],
        Limits::default(),
        IconStates::default(),
    );

    assert_eq!(bake.diagnostics.count(), 0, "{:?}", bake.diagnostics);
    assert_eq!(baked_state(&bake, 5), Some("wall-15"));
    assert_eq!(baked_state(&bake, 1), Some("wall-5"));
    assert_eq!(baked_state(&bake, 9), Some("wall-10"));

    bake.update(&tree, &module, Vec::new(), &[5]);
    assert_eq!(baked_state(&bake, 2), Some("wall-12"));
    assert_eq!(baked_state(&bake, 4), Some("wall-3"));

    bake.update(&tree, &module, vec![original[4].clone()], &[]);
    assert_eq!(baked_state(&bake, 5), Some("wall-15"));
    assert_eq!(baked_state(&bake, 2), Some("wall-13"));
}

#[test]
fn lighting_hooks_build_and_incrementally_restore_the_lightmap() {
    let (tree, module) = compile(
        r##"
/obj/lamp
/obj/ambient
/proc/demir_light(atom/target)
    if(istype(target, /obj/lamp))
        target.demir_light_range = 3
        target.demir_light_power = 1
        target.demir_light_color = "#ff4000"
        target.demir_light_height = 0
    if(istype(target, /obj/ambient))
        target.demir_ambient_color = "#0020ff"
        target.demir_ambient_power = 0.25
"##,
    );
    let lamp = tree.id_of(&TreePath::parse("/obj/lamp")).expect("lamp type");
    let ambient = tree.id_of(&TreePath::parse("/obj/ambient")).expect("ambient type");
    let atoms = vec![
        Atom {
            instance: 1,
            ty: lamp,
            position: Position::new(2, 2, 1),
            vars: Vec::new(),
        },
        Atom {
            instance: 2,
            ty: ambient,
            position: Position::new(3, 2, 1),
            vars: Vec::new(),
        },
    ];
    let mut bake = Bake::new(
        &tree,
        &module,
        atoms.clone(),
        [4, 3, 1],
        Limits::default(),
        IconStates::default(),
    );
    let original = bake.lighting.clone().expect("light hook exports a lightmap");
    let center = original.tile(Position::new(2, 2, 1)).expect("center tile");
    let ambient_tile = original.tile(Position::new(3, 2, 1)).expect("ambient tile");
    assert!(center.corners[0][0] > center.corners[0][1]);
    assert!(ambient_tile.corners[0][2] > center.corners[0][2]);

    let removed = bake.update(&tree, &module, Vec::new(), &[1]);
    assert_eq!(removed.lighting, Some(0..12));
    assert!(
        bake.lighting
            .as_ref()
            .unwrap()
            .tile(Position::new(2, 2, 1))
            .unwrap()
            .corners
            .iter()
            .all(|corner| corner[0] == 0.0)
    );

    let restored = bake.update(&tree, &module, vec![atoms[0].clone()], &[]);
    assert_eq!(restored.lighting, Some(0..12));
    assert_eq!(bake.lighting.as_ref(), Some(&original));

    let mut moved_lamp = atoms[0].clone();
    moved_lamp.position = Position::new(3, 2, 1);
    let moved = bake.update(&tree, &module, vec![atoms[0].clone(), moved_lamp.clone()], &[]);
    let expected = Bake::new(
        &tree,
        &module,
        vec![moved_lamp.clone(), atoms[1].clone()],
        [4, 3, 1],
        Limits::default(),
        IconStates::default(),
    );
    assert_eq!(moved.lighting, Some(0..12));
    assert_eq!(bake.lighting, expected.lighting);

    let mut pixel_shifted_lamp = moved_lamp;
    pixel_shifted_lamp.vars = vec![(Identifier::from("pixel_x"), Value::Num(16.0))];
    bake.update(&tree, &module, vec![pixel_shifted_lamp.clone()], &[]);
    let expected = Bake::new(
        &tree,
        &module,
        vec![pixel_shifted_lamp, atoms[1].clone()],
        [4, 3, 1],
        Limits::default(),
        IconStates::default(),
    );
    assert_eq!(bake.lighting, expected.lighting);
}

#[test]
fn lighting_hook_keeps_zero_radius_quadratic_sources() {
    let (tree, module) = compile(
        r##"
/obj/runway_light
/proc/demir_light(atom/target)
    if(istype(target, /obj/runway_light))
        target.demir_light_range = 0
        target.demir_light_power = 0.5
        target.demir_light_color = "#ffffff"
        target.demir_light_height = 5.76
        target.demir_light_quadratic = 1.1
        target.demir_light_constant = -0.11
"##,
    );
    let runway_light = tree
        .id_of(&TreePath::parse("/obj/runway_light"))
        .expect("runway light type");
    let atoms = vec![Atom {
        instance: 1,
        ty: runway_light,
        position: Position::new(2, 2, 1),
        vars: Vec::new(),
    }];

    let bake = Bake::new(
        &tree,
        &module,
        atoms,
        [3, 3, 1],
        Limits::default(),
        IconStates::default(),
    );
    let center = bake
        .lighting
        .as_ref()
        .expect("quadratic source exports a lightmap")
        .tile(Position::new(2, 2, 1))
        .expect("source tile");
    assert!(center.corners.iter().all(|corner| corner[0] > 0.0));
}

#[test]
fn appearance_offset_moves_a_wall_fixture_without_leaking_through_its_wall() {
    let (tree, module) = compile(
        r##"
/obj/fixture
    pixel_y = 21
/obj/wall
    opacity = 1
/proc/demir_light(atom/target)
    if(istype(target, /obj/fixture))
        target.demir_light_range = 3
        target.demir_light_power = 1.6
        target.demir_light_color = "#ffffff"
        target.demir_light_height = 5.76
        target.demir_light_quadratic = 3.52
        target.demir_light_constant = -0.11
"##,
    );
    let fixture = tree.id_of(&TreePath::parse("/obj/fixture")).expect("fixture type");
    let wall = tree.id_of(&TreePath::parse("/obj/wall")).expect("wall type");
    let mut atoms = vec![Atom {
        instance: 1,
        ty: fixture,
        position: Position::new(3, 2, 1),
        vars: Vec::new(),
    }];
    atoms.extend((1..=5).map(|x| Atom {
        instance: x as u64 + 1,
        ty: wall,
        position: Position::new(x, 3, 1),
        vars: Vec::new(),
    }));

    let mut bake = Bake::new(
        &tree,
        &module,
        atoms.clone(),
        [5, 5, 1],
        Limits::default(),
        IconStates::default(),
    );
    let lighting = bake.lighting.as_ref().expect("fixture exports a lightmap");
    assert!(
        lighting
            .tile(Position::new(3, 2, 1))
            .unwrap()
            .corners
            .iter()
            .any(|corner| corner[0] > 0.0)
    );
    assert_eq!(lighting.tile(Position::new(3, 4, 1)).unwrap().corners, [[0.0; 3]; 4]);

    let original = bake.lighting.clone();
    let mut moved = atoms[0].clone();
    moved.vars = vec![(Identifier::from("pixel_y"), Value::Num(10.0))];
    assert!(bake.update(&tree, &module, vec![moved.clone()], &[]).lighting.is_some());
    assert_ne!(bake.lighting, original, "changing pixel_y must move the light origin");

    let mut expected_atoms = atoms;
    expected_atoms[0] = moved;
    let expected = Bake::new(
        &tree,
        &module,
        expected_atoms.clone(),
        [5, 5, 1],
        Limits::default(),
        IconStates::default(),
    );
    assert_eq!(bake.lighting, expected.lighting);

    let before_tile_move = bake.lighting.clone();
    let mut tile_moved = expected_atoms[0].clone();
    tile_moved.position = Position::new(3, 1, 1);
    assert!(
        bake.update(&tree, &module, vec![tile_moved.clone()], &[])
            .lighting
            .is_some()
    );
    assert_ne!(bake.lighting, before_tile_move, "changing y must move the light source");

    expected_atoms[0] = tile_moved;
    let expected = Bake::new(
        &tree,
        &module,
        expected_atoms,
        [5, 5, 1],
        Limits::default(),
        IconStates::default(),
    );
    assert_eq!(bake.lighting, expected.lighting);
}

#[test]
fn range_and_orange_spiral_out_with_areas_once() {
    let (tree, module) = compile(
        r#"
/area/zone
/turf/floor
/obj/thing
/proc/describe(list/found)
    var/list/parts = list()
    for(var/atom/entry in found)
        if(isturf(entry))
            parts += "[entry.x],[entry.y]"
        else if(isarea(entry))
            parts += "A"
        else
            parts += "O"
    return jointext(parts, " ")

/proc/demir_bake(atom/target)
    if(!istype(target, /turf/floor) || target.x != 2 || target.y != 2)
        return
    target.name = "[describe(orange(1, target))] | [describe(range(target, 1))]"
"#,
    );
    let ty = |path: &str| tree.id_of(&TreePath::parse(path)).expect("fixture type");
    let mut atoms = Vec::new();
    for y in 1..=3 {
        for x in 1..=3 {
            for path in ["/area/zone", "/turf/floor"] {
                atoms.push(Atom {
                    instance: atoms.len() as u64 + 1,
                    ty: ty(path),
                    position: Position::new(x, y, 1),
                    vars: Vec::new(),
                });
            }
        }
    }
    atoms.push(Atom {
        instance: atoms.len() as u64 + 1,
        ty: ty("/obj/thing"),
        position: Position::new(1, 1, 1),
        vars: Vec::new(),
    });
    let bake = Bake::new(
        &tree,
        &module,
        atoms,
        [3, 3, 1],
        Limits::default(),
        IconStates::default(),
    );

    assert_eq!(bake.diagnostics.count(), 0, "{:?}", bake.diagnostics);
    let center = bake.appearances[&10]
        .vars
        .iter()
        .find(|(name, _)| name.as_str() == "name")
        .and_then(|(_, value)| value.as_text())
        .expect("the centre turf names what it found");
    assert_eq!(
        center,
        "1,1 A O 1,2 1,3 2,1 2,3 3,1 3,2 3,3 | 2,2 A 1,1 O 1,2 1,3 2,1 2,3 3,1 3,2 3,3"
    );
}

#[test]
fn baking_exports_icon_objects_and_ignores_timed_effects() {
    let (tree, module) = compile(
        r#"
/obj/panel
    icon = 'old.dmi'
/proc/demir_bake(atom/target)
    flick("opening", target)
    animate(target, alpha = 0, time = 10)
    target.icon = icon(icon('panels.dmi', "on"))
    target.icon_state = "on"
"#,
    );
    let panel = tree.id_of(&TreePath::parse("/obj/panel")).expect("panel type");
    let bake = Bake::new(
        &tree,
        &module,
        vec![Atom {
            instance: 1,
            ty: panel,
            position: Position::new(1, 1, 1),
            vars: Vec::new(),
        }],
        [1, 1, 1],
        Limits::default(),
        IconStates::default(),
    );

    assert_eq!(bake.diagnostics.count(), 0, "{:?}", bake.diagnostics);
    let vars = &bake.appearances[&1].vars;
    assert!(
        vars.contains(&(Identifier::from("icon"), Value::Resource("panels.dmi".into()))),
        "{vars:?}"
    );
    assert!(
        vars.contains(&(Identifier::from("icon_state"), Value::Text("on".into()))),
        "{vars:?}"
    );
}

#[test]
fn baking_exports_sprite_lighting_roles() {
    let (tree, module) = compile(
        r#"
/obj/light
/proc/demir_bake(atom/target)
    if(!istype(target, /obj/light))
        return
    target.demir_emissive = TRUE
    var/image/glow = new
    glow.icon_state = "glow"
    glow.demir_emissive = TRUE
    var/image/blocker = new
    blocker.icon_state = "blocker"
    blocker.demir_emissive_blocker = TRUE
    glow.overlays += blocker
    var/image/overlay_light = new
    overlay_light.icon_state = "overlay-light"
    overlay_light.demir_overlay_light = 1
    glow.overlays += overlay_light
    var/image/darkness = new
    darkness.icon_state = "darkness"
    darkness.demir_overlay_light = -1
    glow.overlays += darkness
    target.overlays += glow
"#,
    );
    let light = tree.id_of(&TreePath::parse("/obj/light")).expect("light type");
    let bake = Bake::new(
        &tree,
        &module,
        vec![Atom {
            instance: 1,
            ty: light,
            position: Position::new(1, 1, 1),
            vars: Vec::new(),
        }],
        [1, 1, 1],
        Limits::default(),
        IconStates::default(),
    );

    assert_eq!(bake.diagnostics.count(), 0, "{:?}", bake.diagnostics);
    let appearance = &bake.appearances[&1];
    assert_eq!(appearance.lighting, crate::AppearanceLighting::Emissive);
    assert_eq!(appearance.overlays[0].lighting, crate::AppearanceLighting::Emissive);
    assert_eq!(
        appearance.overlays[0].overlays[0].lighting,
        crate::AppearanceLighting::Blocker
    );
    assert_eq!(
        appearance.overlays[0].overlays[1].lighting,
        crate::AppearanceLighting::OverlayLight
    );
    assert_eq!(
        appearance.overlays[0].overlays[2].lighting,
        crate::AppearanceLighting::OverlayLightSubtract
    );
}

#[test]
fn baking_rolls_back_overlays_and_randomness_is_repeatable() {
    let source = WALLS.replace(
        "target.icon_state = \"wall-[junction]\"",
        "target.icon_state = \"wall-[rand(1, 100)]-[pick(1, 2, 3)]\"\n    target.overlays += \"edge\"",
    );
    let (tree, module) = compile(&source);
    let atoms = wall_patch(&tree);
    let mut bake = Bake::new(
        &tree,
        &module,
        atoms.clone(),
        [3, 3, 1],
        Limits::default(),
        IconStates::default(),
    );
    let before = bake.appearances.clone();

    bake.update(&tree, &module, vec![atoms[4].clone()], &[]);

    assert_eq!(bake.appearances, before);
    assert_eq!(bake.appearances[&5].overlays.len(), 1);
}

#[test]
fn bake_cache_uses_instance_variables() {
    let source = r#"
/turf/styled
    icon_state = "static"
    var/style = 0
/proc/demir_bake(atom/target)
    if(istype(target, /turf/styled))
        target.icon_state = "[target.style]"
"#;
    let (tree, module) = compile(source);
    let ty = tree
        .id_of(&TreePath::parse("/turf/styled"))
        .expect("styled turf should exist");
    let atoms = (1..=20)
        .map(|x| Atom {
            instance: x as u64,
            ty,
            position: Position::new(x, 1, 1),
            vars: Vec::new(),
        })
        .collect::<Vec<_>>();
    let mut bake = Bake::new(
        &tree,
        &module,
        atoms.clone(),
        [20, 1, 1],
        Limits::default(),
        IconStates::default(),
    );
    assert!(bake.cache_hits > 10);

    let mut edited = atoms[9].clone();
    edited.vars.push(("style".into(), core::types::Value::Num(7.0)));
    bake.update(&tree, &module, vec![edited], &[]);

    assert_eq!(baked_state(&bake, 10), Some("7"));
    assert_eq!(baked_state(&bake, 11), Some("0"));
}

#[test]
fn initialization_runs_once_for_a_bake_and_not_for_updates() {
    let (tree, module) = compile(
        r#"
var/global/demir_init_count = 0
/turf/initialized
    icon_state = "static"
    var/demir_prepared = 0
/proc/demir_initialize()
    world.log << "hello world"
    demir_init_count += 1
/proc/demir_prepare(atom/target)
    if(istype(target, /turf/initialized))
        target.demir_prepared += 1
/proc/demir_bake(atom/target)
    if(istype(target, /turf/initialized))
        target.icon_state = "[demir_init_count]-[target.demir_prepared]"
"#,
    );
    let ty = tree
        .id_of(&TreePath::parse("/turf/initialized"))
        .expect("initialized turf should exist");
    let atoms = (1..=2)
        .map(|x| Atom {
            instance: x as u64,
            ty,
            position: Position::new(x, 1, 1),
            vars: Vec::new(),
        })
        .collect::<Vec<_>>();
    let mut progress = Vec::new();
    let mut bake = Bake::with_progress(
        &tree,
        &module,
        atoms.clone(),
        [2, 1, 1],
        Limits::default(),
        IconStates::default(),
        |stage, done, total| {
            if stage == crate::bake::Stage::Initialize {
                progress.push((done, total));
            }
        },
    );

    assert_eq!(progress, vec![(0, 1), (1, 1)]);
    assert_eq!(bake.take_output(), vec![String::from("hello world")]);
    assert_eq!(baked_state(&bake, 1), Some("1-1"));
    assert_eq!(baked_state(&bake, 2), Some("1-1"));

    bake.update(&tree, &module, vec![atoms[0].clone()], &[]);
    assert!(bake.take_output().is_empty());
    assert_eq!(baked_state(&bake, 1), Some("1-1"));
}

#[test]
fn initialization_fault_stops_all_object_hooks() {
    let (tree, module) = compile(
        r#"
/turf/initialized
    icon_state = "static"
/proc/demir_initialize()
    world.Reboot()
/proc/demir_bake(atom/target)
    target.icon_state = "baked"
"#,
    );
    let atom = Atom {
        instance: 1,
        ty: tree
            .id_of(&TreePath::parse("/turf/initialized"))
            .expect("initialized turf should exist"),
        position: Position::new(1, 1, 1),
        vars: Vec::new(),
    };
    let mut bake = Bake::new(
        &tree,
        &module,
        vec![atom.clone()],
        [1, 1, 1],
        Limits::default(),
        IconStates::default(),
    );

    assert_eq!(bake.diagnostics.count(), 1);
    assert_eq!(bake.attempted, 0);
    assert!(bake.appearances.is_empty());

    bake.update(&tree, &module, vec![atom], &[]);
    assert_eq!(bake.diagnostics.count(), 1);
    assert_eq!(bake.attempted, 0);
    assert!(bake.appearances.is_empty());
}

#[test]
fn neighbor_overlay_changes_are_exported_and_rolled_back() {
    let (tree, module) = compile(
        r#"
/turf/overlay_test
    var/list/overlays = list()
    proc/Initialize(mapload)
        overlays = list("base")
/proc/demir_bake(atom/target)
    if(istype(target, /turf/overlay_test))
        target.Initialize(TRUE)
        var/turf/east = get_step(target, 4)
        if(east)
            east.overlays.Add("edge")
"#,
    );
    let ty = tree
        .id_of(&TreePath::parse("/turf/overlay_test"))
        .expect("overlay turf should exist");
    let atoms = (1..=2)
        .map(|x| Atom {
            instance: x as u64,
            ty,
            position: Position::new(x, 1, 1),
            vars: Vec::new(),
        })
        .collect::<Vec<_>>();
    let mut bake = Bake::new(
        &tree,
        &module,
        atoms,
        [2, 1, 1],
        Limits::default(),
        IconStates::default(),
    );

    assert_eq!(bake.appearances[&2].overlays.len(), 2);
    bake.update(&tree, &module, Vec::new(), &[1]);
    assert_eq!(bake.appearances[&2].overlays.len(), 1);
}

#[test]
fn shared_overlay_graphs_respect_the_export_budget() {
    let (tree, module) = compile(
        r#"
/turf/export_test
    icon_state = "static"
    overlays = list()
/proc/demir_bake(atom/target)
    if(istype(target, /turf/export_test))
        var/image/previous = new
        for(var/i = 1 to 24)
            var/image/next = new
            next.overlays = list(previous, previous)
            previous = next
        target.overlays += previous
"#,
    );
    let atom = Atom {
        instance: 1,
        ty: tree
            .id_of(&TreePath::parse("/turf/export_test"))
            .expect("export turf should exist"),
        position: Position::new(1, 1, 1),
        vars: Vec::new(),
    };
    let bake = Bake::new(
        &tree,
        &module,
        vec![atom],
        [1, 1, 1],
        Limits {
            allocations: 1000,
            ..Default::default()
        },
        IconStates::default(),
    );

    assert!(bake.appearances.is_empty());
    assert_eq!(bake.diagnostics.count(), 1);
    assert_eq!(bake.diagnostics.entries[0].fault.kind, FaultKind::Memory);
}

#[test]
fn executes_arithmetic_direct_calls_and_defaults() {
    assert_eq!(
        run(
            r#"
/proc/add(a, b = 4)
    return a + b
/proc/test()
    return add(3)
"#,
            "test",
        ),
        7.into()
    );
}

#[test]
fn nonconstant_defaults_can_read_earlier_parameters() {
    assert_eq!(
        run(
            r#"
/proc/value(a = 2, b = a + 3)
    return b
/proc/test()
    return value(4) * 10 + value()
"#,
            "test",
        ),
        75.into()
    );
}

#[test]
fn executes_phi_lowered_fibonacci_loop() {
    assert_eq!(
        run(
            r#"
/proc/fib(n)
    var/a = 0
    var/b = 1
    for(var/i = 0; i < n; i++)
        var/tmp = b
        b = a + b
        a = tmp
    return a
/proc/test()
    return fib(10)
"#,
            "test",
        ),
        55.into()
    );
}

#[test]
fn associative_iteration_preserves_keys_and_values() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/list/values = list("a" = 2, "b" = 5)
    var/total = 0
    for(var/key, var/value in values)
        total += values[key] + value
    return total
"#,
            "test",
        ),
        14.into()
    );
}

#[test]
fn labelled_break_leaves_a_plain_block() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/value = 0
    done: {
        value = 1
        break done
        value = 2
    }
    return value
"#,
            "test",
        ),
        1.into()
    );
}

#[test]
fn labelled_break_inside_do_while_leaves_the_block() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/value = 0
    do {
        done: {
            value = 1
            break done
            value = 2
        }
    } while(FALSE)
    return value
"#,
            "test",
        ),
        1.into()
    );
}

#[test]
fn repeated_local_labels_create_distinct_blocks() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/value = 0
    do {
        done: {
            value += 1
            break done
        }
    } while(FALSE)
    do {
        done: {
            value += 1
            break done
        }
    } while(FALSE)
    return value
"#,
            "test",
        ),
        2.into()
    );
}

#[test]
fn repeated_local_labels_preserve_values_from_their_predecessors() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/value = 7
    do {
        done: {
            break done
        }
    } while(FALSE)
    var/after_first = value
    do {
        done: {
            break done
        }
    } while(FALSE)
    return after_first
"#,
            "test",
        ),
        7.into()
    );
}

#[test]
fn descending_ranges_use_the_steps_sign() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/total = 0
    for(var/i = 5 to 1 step -2)
        total += i
    return total
"#,
            "test",
        ),
        9.into()
    );
}

#[test]
fn virtual_dispatch_super_dot_and_defaults() {
    assert_eq!(
        run(
            r#"
/datum/base
    proc/value(a = 3)
        return a * 2
/datum/base/child
    value(a = 4)
        . = ..()
        . += 1
/proc/test()
    var/datum/base/child/object = new
    return object.value()
"#,
            "test",
        ),
        9.into()
    );
}

#[test]
fn sandbox_faults_are_not_catchable() {
    let (tree, module) = compile(
        r#"
/proc/test()
    try
        shell("echo no")
    catch
        return 1
    return 0
"#,
    );
    let fault = Runtime::default()
        .run(&tree, &module, proc(&tree, "test"), None, Vec::new(), Limits::default())
        .expect_err("sandbox operation should fault");
    assert_eq!(fault.kind, FaultKind::Blocked("shell".into()));
}

#[test]
fn world_log_output_uses_a_field_reference() {
    let (tree, module) = compile(
        r#"
/proc/test()
    world.log << "hello"
    return world.log
"#,
    );
    let mut runtime = Runtime::default();
    let result = runtime
        .run(&tree, &module, proc(&tree, "test"), None, Vec::new(), Limits::default())
        .expect("fixture should execute");

    assert_eq!(result, GenericValue::Null);
    assert_eq!(runtime.output(), &[String::from("hello")]);
}

#[test]
fn locate_in_searches_the_container() {
    let result = run(
        r#"
/datum/a
/datum/b

/proc/test()
    var/datum/b/wanted = new
    var/list/things = list(new /datum/a, wanted)
    var/datum/b/found = locate(/datum/b) in things
    var/missing = locate(/datum/b) in list(new /datum/a)
    return "[found == wanted] [isnull(missing)] [istype(things ? locate(/datum/a) in things : null, /datum/a)]"
"#,
        "test",
    );

    assert_eq!(result, GenericValue::from("1 1 1"));
}

#[test]
fn a_var_and_a_proc_can_share_a_name() {
    let result = run(
        r#"
/datum/rock
    var/list/edges = null

/datum/rock/proc/edges()
    src.edges = list()
    src.edges += "north"
    edges += "south"
    return length(src.edges)

/proc/test()
    var/datum/rock/rock = new
    return "[rock.edges()] [length(rock.edges)] [rock.edges[2]]"
"#,
        "test",
    );

    assert_eq!(result, GenericValue::from("2 2 south"));
}

#[test]
fn datums_keep_their_own_coordinate_vars() {
    let result = run(
        r#"
/datum/light
    var/x = 1
    var/y

/proc/test()
    var/datum/light/light = new
    light.x = 4.5
    light.y = 2
    return "[light.x] [light.y]"
"#,
        "test",
    );

    assert_eq!(result, GenericValue::from("4.5 2"));
}

#[test]
fn sized_type_vars_start_as_lists() {
    let result = run(
        r#"
/obj/thing
    var/global/shared[8]
    var/sized[3]
    var/list/empty[]

/proc/test()
    var/obj/thing/first = new
    var/obj/thing/second = new
    first.sized[1] = 1
    first.shared[1] = 2
    return "[length(first.sized)] [length(first.shared)] [length(first.empty)] [second.sized[1]] [second.shared[1]]"
"#,
        "test",
    );

    assert_eq!(result, GenericValue::from("3 8 0  2"));
}

#[test]
fn icon_states_answer_from_the_host_table() {
    let (tree, module) = compile(
        r#"
/proc/test()
    var/icon/wrapped = icon('Icons\Walls.dmi')
    var/list/found = icon_states('icons/walls.dmi')
    return "[length(found)] [found[1]] [found[2]] [length(wrapped.IconStates())] [length(icon_states('missing.dmi'))]"
"#,
    );
    let mut runtime = Runtime::default();
    runtime.icons = IconStates::new([(
        String::from("icons/walls.dmi"),
        vec![String::from("wall0"), String::from("wall1"), String::from("wall0")],
    )]);
    let result = runtime
        .run(&tree, &module, proc(&tree, "test"), None, Vec::new(), Limits::default())
        .expect("fixture should execute");

    assert_eq!(result, GenericValue::from("2 wall0 wall1 2 0"));
}

#[test]
fn unsupported_output_targets_are_blocked() {
    let (tree, module) = compile(
        r#"
/proc/test()
    null << 1
"#,
    );
    let fault = Runtime::default()
        .run(&tree, &module, proc(&tree, "test"), None, Vec::new(), Limits::default())
        .expect_err("unsupported output target should fault");

    assert_eq!(fault.kind, FaultKind::Blocked("output".into()));
}

#[test]
fn instruction_budget_stops_infinite_control_flow() {
    let (tree, module) = compile(
        r#"
/proc/test()
    while(1)
        . = 1
"#,
    );
    let fault = Runtime::default()
        .run(
            &tree,
            &module,
            proc(&tree, "test"),
            None,
            Vec::new(),
            Limits {
                instruction_budget: 20,
                ..Limits::default()
            },
        )
        .expect_err("instruction budget should stop the loop");
    assert_eq!(fault.kind, FaultKind::InstructionBudget);
}

#[test]
fn dm_throw_is_caught_with_its_value() {
    assert_eq!(
        run(
            r#"
/proc/raiser()
    throw 42
/proc/test()
    try
        raiser()
    catch(var/value)
        return value + pick(list(7))
"#,
            "test",
        ),
        49.into()
    );
}

#[test]
fn values_live_across_try_and_catch_merge_correctly() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/value = 1
    try
        value = 2
        throw 9
    catch(var/error)
        value += error
    return value
"#,
            "test",
        ),
        11.into()
    );
}

#[test]
fn try_inside_a_loop_preserves_frame_locals() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/list/values = list(1, 2, 3)
    var/total = 0
    for(var/value in values)
        var/current = value
        try
            total += current
        catch
            total = -100
        current = 100
    return total
"#,
            "test",
        ),
        6.into()
    );
}

#[test]
fn parameter_defaults_are_applied_before_try_frame_storage() {
    assert_eq!(
        run(
            r#"
/proc/value(number = 4)
    try
        number += 1
    catch
        number = -100
    return number
/proc/test()
    return value()
"#,
            "test",
        ),
        5.into()
    );
}

#[test]
fn same_type_super_observes_reassigned_arguments() {
    assert_eq!(
        run(
            r#"
/datum/test/proc/value(a = 1)
    return a
/datum/test/value(a = 2)
    args[1] += 3
    a += 4
    return ..()
/proc/test()
    var/datum/test/object = new
    return object.value()
"#,
            "test",
        ),
        9.into()
    );
}

#[test]
fn static_state_is_shared_and_dynamic_dm_calls_work() {
    assert_eq!(
        run(
            r#"
/datum/test
    var/static/list/items = list()
    proc/value(a = 9, b = 2)
        var/static/count = 0
        count++
        items += count
        return a + b + length(items)
/proc/test()
    var/datum/test/a = new
    var/datum/test/b = new
    a.value()
    return call(b, "value")(, 3)
"#,
            "test",
        ),
        14.into()
    );
}

#[test]
fn named_and_arglist_arguments_keep_their_layout() {
    assert_eq!(
        run(
            r#"
/proc/value(a, b, c)
    return a * 100 + b * 10 + c
/proc/forward(list/arguments)
    return value(arglist(arguments))
/proc/test()
    return forward(list("c" = 3, "a" = 1, "b" = 2))
"#,
            "test",
        ),
        123.into()
    );
}

#[test]
fn positional_arguments_fill_parameters_left_open_by_named_arguments() {
    assert_eq!(
        run(
            r#"
/proc/value(a, b, c)
    return a * 100 + b * 10 + c
/proc/test()
    return value(c = 3, 1, 2)
"#,
            "test",
        ),
        123.into()
    );
}

#[test]
fn args_is_a_dm_list_with_the_supplied_length() {
    assert_eq!(
        run(
            r#"
/proc/value(a, b)
    return args.len + islist(args)
/proc/test()
    return value(1, 2)
"#,
            "test",
        ),
        3.into()
    );
}

#[test]
fn a_sized_local_list_declaration_allocates_its_entries() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/device_type = 3
    var/list/node_connects[device_type]
    node_connects[1] = 1
    node_connects[2] = 2
    node_connects[3] = 4
    return node_connects.len * 10 + node_connects[3]
"#,
            "test",
        ),
        34.into()
    );
}

#[test]
fn list_iteration_uses_a_stable_entry_snapshot() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/list/values = list(1, 2, 3)
    var/total = 0
    for(var/value in values)
        total += value
        values.Add(value + 10)
    return total * 10 + length(values)
"#,
            "test",
        ),
        66.into()
    );
}

#[test]
fn compound_list_operators_mutate_aliases_but_plain_operators_copy() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/list/original = list(1, 2)
    var/list/alias = original
    alias += 3
    var/list/copy = original + 4
    return length(original) * 100 + length(alias) * 10 + length(copy)
"#,
            "test",
        ),
        334.into()
    );
}

/// `var/x; x += thing` and `L[key] += thing` on an absent key, which real codebases spell through
/// `LAZYADD`-style macros
#[test]
fn null_is_the_identity_for_addition() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/text
    text += "a"
    text += "b"
    return text
"#,
            "test",
        ),
        "ab".into()
    );

    assert_eq!(
        run(
            r#"
/datum/thing
    var/tag_name = "x"
/proc/test()
    var/list/entries = list()
    var/datum/thing/thing = new
    entries["4"] += thing
    var/datum/thing/stored = entries["4"]
    return stored.tag_name
"#,
            "test",
        ),
        "x".into()
    );
}

#[test]
fn typed_iteration_filters_the_iterated_value() {
    assert_eq!(
        run(
            r#"
/datum/base
/datum/base/wanted
/datum/other
/proc/test()
    var/list/values = list(new /datum/base/wanted, new /datum/other, new /datum/base/wanted)
    var/count = 0
    for(var/datum/base/value in values)
        count++
    return count
"#,
            "test",
        ),
        2.into()
    );
}

#[test]
fn one_argument_istype_uses_the_declared_type() {
    assert_eq!(
        run(
            r#"
/datum/base
/datum/base/child
/datum/other
/proc/test()
    var/datum/base/value = new /datum/other
    return istype(value)
"#,
            "test",
        ),
        0.into()
    );
}

#[test]
fn initial_reads_the_declaration_instead_of_the_current_field() {
    assert_eq!(
        run(
            r#"
/datum/test
    var/value = 4
/proc/test()
    var/datum/test/object = new
    object.value = 9
    return initial(object.value) + issaved(object.value)
"#,
            "test",
        ),
        5.into()
    );
}

#[test]
fn initial_of_a_local_falls_back_to_its_current_value() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/value = 4
    value = 9
    return initial(value)
"#,
            "test",
        ),
        9.into()
    );
}

#[test]
fn safe_calls_on_null_return_null() {
    assert_eq!(
        run(
            r#"
/datum/test
    proc/value()
        return 1
/proc/test()
    var/datum/test/object
    return object?.value()
"#,
            "test",
        ),
        GenericValue::Null
    );
}

#[test]
fn safe_index_writes_on_null_are_discarded() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/list/value
    value?[1] = 2
    return 3
"#,
            "test",
        ),
        3.into()
    );
}

#[test]
fn call_depth_stops_recursive_functions() {
    let (tree, module) = compile("/proc/test()\n    return test()\n");
    let fault = Runtime::default()
        .run(
            &tree,
            &module,
            proc(&tree, "test"),
            None,
            Vec::new(),
            Limits {
                call_depth: 4,
                ..Limits::default()
            },
        )
        .expect_err("call depth should stop recursion");
    assert_eq!(fault.kind, FaultKind::CallDepth);
}

#[test]
fn an_instruction_fault_rolls_back_heap_changes() {
    let (tree, module) = compile(
        r#"
/datum
    var/value = 1
    var/list/items = list("old")
/proc/test(datum/target)
    target.value = 2
    target.items.Add("new")
    new /datum
    while(1)
        target.value++
"#,
    );
    let mut runtime = Runtime::default();
    let object = runtime
        .heap
        .alloc_object(Object::new(
            tree.id_of(&TreePath::parse("/datum")).expect("type should exist"),
        ))
        .expect("object should allocate");
    let fault = runtime
        .run(
            &tree,
            &module,
            proc(&tree, "test"),
            None,
            vec![GenericValue::Object(object)],
            Limits {
                instruction_budget: 100,
                ..Limits::default()
            },
        )
        .expect_err("instruction budget should stop the loop");
    assert_eq!(fault.kind, FaultKind::InstructionBudget);
    assert!(
        runtime
            .heap
            .object(object)
            .expect("object should survive")
            .vars
            .is_empty()
    );
    // The fixture's datum plus the global and world singletons, both allocated ahead of the journal.
    assert_eq!(runtime.heap.objects().count(), 3);
    assert!(runtime.heap.object(ObjectId(1)).is_some());
}

#[test]
fn evaluation_reports_randomness_and_nonlocal_world_reads() {
    let (tree, module) = compile(
        r#"
/datum
/proc/test()
    rand()
    locate(/datum)
"#,
    );
    let mut runtime = Runtime::default();
    runtime.global = Some(
        runtime
            .heap
            .alloc_object(Object::new(TypeId::ROOT))
            .expect("global should allocate"),
    );
    let mut evaluator = Evaluator::new(&mut runtime, &tree, &module, Limits::default(), None);
    evaluator
        .call(proc(&tree, "test"), None, Vec::new())
        .expect("fixture should execute");
    assert!(evaluator.position_sensitive);
    assert!(!evaluator.memo_safe);
}

#[test]
fn intrinsic_procs_run_in_rust_instead_of_their_body() {
    let (tree, module) = compile(
        r#"
/world
    proc/file2list(File, Separator)
        set __demir_intrin = 122
    proc/IsBanned(key, address, computer_id, type)
        set __demir_intrin = 112
    proc/Reboot(reason)
        set __demir_intrin = 104
/proc/lines()
    return world.file2list("tips.txt")
/proc/banned()
    return world.IsBanned("key")
/proc/reboot()
    return world.Reboot()
"#,
    );

    let root = std::env::temp_dir().join(format!("dmed-intrinsic-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("temp dir");
    std::fs::write(root.join("tips.txt"), "first\nsecond\n").expect("fixture file");

    let mut runtime = Runtime::default();
    runtime.world.root = Some(root.clone());

    let value = runtime
        .run(
            &tree,
            &module,
            proc(&tree, "lines"),
            None,
            Vec::new(),
            Limits::default(),
        )
        .expect("file2list should execute");
    let GenericValue::List(id) = value else {
        panic!("file2list should return a list, got {value:?}");
    };
    let entries = runtime
        .heap
        .list(id)
        .expect("list")
        .entries
        .iter()
        .map(|(value, _)| value.display())
        .collect::<Vec<_>>();
    assert_eq!(entries, ["first", "second", ""]);

    assert_eq!(
        runtime
            .run(
                &tree,
                &module,
                proc(&tree, "banned"),
                None,
                Vec::new(),
                Limits::default()
            )
            .expect("IsBanned should execute"),
        false.into()
    );

    let fault = runtime
        .run(
            &tree,
            &module,
            proc(&tree, "reboot"),
            None,
            Vec::new(),
            Limits::default(),
        )
        .expect_err("Reboot should be blocked");
    assert_eq!(fault.kind, FaultKind::Blocked("world.Reboot".into()));

    std::fs::remove_dir_all(&root).ok();
}

/// Without a root there is no filesystem to reach, rather than an ambient one.
#[test]
fn file2list_without_a_root_is_blocked() {
    let (tree, module) = compile(
        r#"
/world
    proc/file2list(File, Separator)
        set __demir_intrin = 122
/proc/lines()
    return world.file2list("tips.txt")
"#,
    );

    let fault = Runtime::default()
        .run(
            &tree,
            &module,
            proc(&tree, "lines"),
            None,
            Vec::new(),
            Limits::default(),
        )
        .expect_err("file2list should be blocked");
    assert_eq!(fault.kind, FaultKind::Blocked("filesystem".into()));
}

/// `/world` procs run with the world as `src`, so assignments land on it rather than on globals.
#[test]
fn world_vars_are_readable_and_writable_through_world_procs() {
    assert_eq!(
        run(
            r#"
/world
    var/booted = 0
    proc/boot()
        booted = 7
/proc/test()
    world.boot()
    return world.booted
"#,
            "test",
        ),
        7.into()
    );
}

/// `new /image(...)` has to fill the object `new` already allocated. An intrinsic constructor that
/// allocated its own would leave the caller holding an empty one.
#[test]
fn image_constructor_fills_the_object_new_allocated() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/image/I = new('thing.dmi', null, "state")
    return I.icon_state
"#,
            "test",
        ),
        "state".into()
    );
}

/// `matrix(M, ...)` is the in-place form every `/matrix` method in `stddef.dm` routes through.
/// Transforms are unimplemented, so it must hand the matrix back rather than write the argument
/// list onto `a`/`b`/`c`.
#[test]
fn matrix_in_place_form_leaves_the_matrix_alone() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/matrix/M = new(2)
    return M.a
"#,
            "test",
        ),
        1.into()
    );
}

/// `/alist` is a list, not a plain object, so `new` has to take the same path `/list` does.
#[test]
fn new_alist_is_a_list_seeded_from_its_pairs() {
    assert_eq!(
        run(
            r#"
/alist
    var/len
    proc/New(items)
/proc/islist(L)
    set __demir_intrin = 279
/proc/test()
    var/alist/A = new(list("a", "b"))
    return islist(A) + A.len
"#,
            "test",
        ),
        3.into()
    );
}

/// `/sound/New` copies its file into the resource cache first, so a blocked `fcopy_rsc` would make
/// every `new /sound(...)` fault.
#[test]
fn sound_constructor_keeps_its_file() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/sound/S = new('beep.ogg')
    return S.file
"#,
            "test",
        ),
        GenericValue::Resource("beep.ogg".into())
    );
}

/// `PROC_REF(X)` is `nameof(.proc/X)`, and tgstation spells almost every callback that way.
#[test]
fn nameof_resolves_a_proc_path_to_its_name() {
    assert_eq!(
        run(
            r#"
/proc/nameof(X)
    set __demir_intrin = 400
/proc/work()
    return 1
/proc/test()
    return nameof(/proc/work)
"#,
            "test",
        ),
        "work".into()
    );
}

/// A global proc called from inside a method binds to the global. `find_proc` would reach the same
/// body, but only after failing a lookup on the runtime type of `src` at every call.
#[test]
fn a_global_proc_called_from_a_method_reaches_the_global() {
    assert_eq!(
        run(
            r#"
/proc/helper(n)
    return n * 2
/datum/thing
    proc/work()
        return helper(21)
/proc/test()
    var/datum/thing/T = new
    return T.work()
"#,
            "test",
        ),
        42.into()
    );
}

/// When a type declares the same name, the method wins and the call has to stay dynamic, or a
/// subtype's override would be linked away.
#[test]
fn a_method_of_the_same_name_still_shadows_the_global() {
    assert_eq!(
        run(
            r#"
/proc/helper(n)
    return 1
/datum/thing
    proc/helper(n)
        return 2
    proc/work()
        return helper(0)
/datum/thing/special
    helper(n)
        return 3
/proc/test()
    var/datum/thing/T = new /datum/thing/special
    return T.work()
"#,
            "test",
        ),
        3.into()
    );
}

/// The positional argument names come from the prelude's own signature, so `image()`'s last two
/// parameters land even though nothing in the VM lists them.
#[test]
fn appearance_arguments_follow_the_declared_signature() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/image/I = image('a.dmi', null, "s", 3, 1, 4, 5, 6, 7)
    return I.pixel_w * 10 + I.pixel_z
"#,
            "test",
        ),
        67.into()
    );
}

/// `mutable_appearance(appearance)` copies an appearance onto the new one, which is what BYOND's
/// signature says and what the prelude declares. The old hardcoded list read that argument as
/// `icon` instead.
#[test]
fn mutable_appearance_takes_an_appearance() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/image/source = new
    source.icon_state = "src"
    var/mutable_appearance/MA = mutable_appearance(source)
    return MA.icon_state
"#,
            "test",
        ),
        "src".into()
    );
}

/// `RemoveAll` drops every occurrence and answers how many went, which is how `list_clear_nulls`
/// on a tgstation downstream asks whether the list held any nulls.
#[test]
fn list_remove_all_reports_how_many_it_dropped() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/list/L = list(1, null, 2, null, null, 3)
    var/dropped = L.RemoveAll(null)
    return dropped * 100 + L.len * 10 + L[1]
"#,
            "test",
        ),
        331.into()
    );
}

/// `Splice(Start, End, Item...)` cuts the range and drops the items in its place.
#[test]
fn list_splice_replaces_a_range() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/list/L = list("a", "b", "c")
    L.Splice(2, 3, "x", "y")
    return L.Join("")
"#,
            "test",
        ),
        "axyc".into()
    );
}

/// List methods use the object tree like datum methods. A user declaration replaces the prelude
/// intrinsic and receives the list itself as `src`.
#[test]
fn list_methods_are_overridable_dm_procs_with_list_src() {
    assert_eq!(
        run(
            r#"
/list/Add(Item1)
    return src.len * 10 + Item1
/proc/test()
    var/list/L = list(1, 2)
    return L.Add(7) * 10 + L.len
"#,
            "test",
        ),
        272.into()
    );
}

/// The prelude declaration remains in the normal override chain, so `..()` reaches the intrinsic
/// implementation and keeps the original list receiver.
#[test]
fn list_method_overrides_can_call_the_intrinsic_super_proc() {
    assert_eq!(
        run(
            r#"
/list/Add(Item1)
    ..()
    return src.len
/proc/test()
    var/list/L = list(1, 2)
    return L.Add(7) * 10 + L.len
"#,
            "test",
        ),
        33.into()
    );
}

/// `/alist` has its own runtime kind but inherits the ordinary `/list` proc declarations.
#[test]
fn alists_inherit_list_intrinsics_through_proc_dispatch() {
    assert_eq!(
        run(
            r#"
/proc/test()
    var/alist/A = alist("first")
    A.Add("value")
    return istype(A, /alist) * 10 + A.len
"#,
            "test",
        ),
        12.into()
    );
}
