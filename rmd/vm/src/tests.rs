use core::{
    arena::StrArena,
    path::TreePath,
    types::{Identifier, Value},
};
use std::{
    collections::{HashMap, HashSet},
    sync::atomic::{AtomicUsize, Ordering},
};

use codegen::ProcIndex;
use defines::{EAST, NORTH, NORTHEAST, SOUTH, SOUTHWEST, WEST};
use objtree::{ObjectTree, ResolvedVarType, TypeId, VarTypeKind};

use crate::{
    Fault, FaultKind, GenericValue, IconStates, Limits, Runtime,
    bake::{
        Atom, Bake, BakeUpdate, HIGHLIGHT_ALWAYS, HIGHLIGHT_EDGE_NORTH, HIGHLIGHT_EDGE_SOUTH, HIGHLIGHT_EDGE_WEST,
        HIGHLIGHT_SELECTED, HighlightTile, ModuleRoot, PendingBake, ProfileError, Stage, has_profile, host_reads,
        profile_catalog, profile_type, selected_profile_type,
    },
    eval::Evaluator,
    heap::{Object, ObjectId},
    profile::{ProfileDefinition, ProfileHook},
    ui::{Command, Feedback, Rebake, Value as UiValue},
    world::Position,
};

macro_rules! fixture {
    ($path:literal) => {
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/", $path))
    };
}

fn analyze_fixture(source: &str) -> (ObjectTree, ir::Module) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("rmd-vm-fixture-{}-{id}.dm", std::process::id()));
    let source = if source.contains("/datum/demir/test") {
        format!("{source}\n/datum/demir/test\n    default = TRUE\n")
    } else {
        source.to_owned()
    };
    std::fs::write(&path, source).expect("write fixture");
    let arena = StrArena::new();
    let preprocessed = preprocessor::preprocess(&arena, &path).expect("fixture should preprocess");
    let _ = std::fs::remove_file(path);
    assert!(preprocessed.errors.is_empty(), "{:?}", preprocessed.errors);
    let ast = ast::parse(&preprocessed.tokens).expect("fixture should parse");
    let (tree, errors) = sema::analyze(&ast);
    assert!(errors.is_empty(), "{errors:?}");
    let (module, _) = ir::lower(&ast, &tree, true);

    (tree, module)
}

fn compile(source: &str) -> (ObjectTree, codegen::Module) {
    let (tree, module) = analyze_fixture(source);
    let module = codegen::generate(&module).expect("fixture should compile");
    (tree, module)
}

fn proc(tree: &ObjectTree, index: &ProcIndex, name: &str) -> core::types::ProcId {
    index
        .inherited_body(tree, TypeId::ROOT, &name.into())
        .expect("fixture proc should exist")
}

fn hook(tree: &ObjectTree, index: &ProcIndex, hook: ProfileHook) -> core::types::ProcId {
    let profile = profile_type(tree).expect("fixture should declare one profile");
    ProfileDefinition::resolve(tree, index, profile)
        .procedure(hook)
        .expect("fixture hook should exist")
}

fn run(source: &str, name: &str) -> GenericValue {
    let (tree, module) = compile(source);
    Runtime::default()
        .run(
            &tree,
            &module,
            proc(&tree, &module.index, name),
            None,
            None,
            Vec::new(),
            Limits::default(),
        )
        .expect("fixture should execute")
}

#[test]
fn negative_spawn_delays_run_before_the_continuation() {
    let source = fixture!("programs/immediate_spawns.dm");
    assert_eq!(
        run(source, "spawn_order"),
        GenericValue::Text(
            "before,delay,spawn,after;before,delay,spawn,after;before,delay,after;before,delay,after".into()
        )
    );
    assert_eq!(
        run(source, "spawn_default_delays"),
        GenericValue::Text("before,after".into())
    );
}

#[test]
fn immediate_spawns_copy_locals_parameters_and_the_return_value() {
    let source = fixture!("programs/immediate_spawns.dm");
    assert_eq!(run(source, "spawn_frame"), GenericValue::Num(79959.0));
    assert_eq!(run(source, "spawn_nested"), GenericValue::Text("33,22,11".into()));
}

#[test]
fn immediate_spawn_returns_and_loop_controls_stay_inside_the_spawn() {
    assert_eq!(
        run(fixture!("programs/immediate_spawns.dm"), "spawn_returns_in_loops"),
        GenericValue::Num(43.0)
    );
}

#[test]
fn immediate_spawns_preserve_shared_mutations_across_exception_scopes() {
    let source = fixture!("programs/immediate_spawns.dm");
    assert_eq!(run(source, "spawn_throws"), GenericValue::Num(7165.0));
    assert_eq!(run(source, "spawn_sleeps"), GenericValue::Text("a,after".into()));
    assert_eq!(
        run(source, "spawn_returns_in_try"),
        GenericValue::Text("after,parent caught".into())
    );
    assert_eq!(run(source, "spawn_dot_in_try"), GenericValue::Text("5,5,1".into()));
    assert_eq!(run(source, "spawn_super_uses_copies"), GenericValue::Num(5.0));
    assert_eq!(run(source, "spawn_try_inside"), GenericValue::Num(87.0));
    assert_eq!(run(source, "spawn_static"), GenericValue::Num(1.0));
    assert_eq!(run(source, "spawn_object"), GenericValue::Num(123.0));
}

#[test]
fn immediate_spawns_do_not_swallow_sandbox_faults() {
    let (tree, module) = compile(fixture!("programs/immediate_spawns.dm"));
    let fault = Runtime::default()
        .run(
            &tree,
            &module,
            proc(&tree, &module.index, "spawn_shell_fault"),
            None,
            None,
            Vec::new(),
            Limits::default(),
        )
        .expect_err("sandbox operation should fault");
    assert_eq!(fault.kind, FaultKind::Blocked("shell".into()));
}

const WALLS: &str = fixture!("programs/profile-wall-bake.dm");

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

fn baked_alpha(bake: &Bake, id: u64) -> Option<f32> {
    bake.appearances
        .get(&id)?
        .vars
        .iter()
        .find(|(name, _)| name.as_str() == "alpha")?
        .1
        .as_num()
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
fn prelude_flags_resolve_to_booleans_through_the_whole_pipeline() {
    let (tree, _) = analyze_fixture(fixture!(
        "programs/prelude_flags_resolve_to_booleans_through_the_whole_pipeline.dm"
    ));
    let crate_id = tree.id_of(&TreePath::parse("/obj/crate")).expect("crate type");
    let kind = |name: &str| {
        tree.resolved_var_type(crate_id, &name.into())
            .and_then(ResolvedVarType::single)
    };

    assert_eq!(kind("density"), Some(VarTypeKind::Bool));
    assert_eq!(kind("opacity"), Some(VarTypeKind::Bool));
    assert_eq!(kind("anchored"), Some(VarTypeKind::Bool));
    assert_eq!(kind("weight"), Some(VarTypeKind::Number));
}

#[test]
fn only_a_demir_subtype_makes_a_profile() {
    let (bare, _) = compile(fixture!("programs/only_a_demir_subtype_makes_a_profile-bare.dm"));
    let (profiled, _) = compile(WALLS);
    // A subtype with no hook bodies at all is still a profile: it bakes nothing, which is what a
    // codebase asking for the editor without appearances wants.
    let (empty, _) = compile(fixture!("programs/only_a_demir_subtype_makes_a_profile-empty.dm"));

    assert_eq!(profile_type(&bare), Err(ProfileError::Missing));
    assert!(!has_profile(&bare));
    assert!(has_profile(&profiled));
    assert!(has_profile(&empty));
}

#[test]
fn profile_definition_resolves_the_complete_hook_abi() {
    let (tree, module) = analyze_fixture(fixture!(
        "programs/profile_definition_resolves_the_complete_hook_abi.dm"
    ));
    let profile = profile_type(&tree).expect("default profile");
    let definition = ProfileDefinition::resolve(&tree, &module.index, profile);
    let resolved = ProfileHook::ALL
        .into_iter()
        .filter(|hook| definition.declaration(&tree, *hook).is_some())
        .collect::<Vec<_>>();

    assert_eq!(resolved, ProfileHook::ALL);
}

#[test]
fn an_explicit_default_is_not_inherited_by_its_variants() {
    let (tree, _) = compile(fixture!(
        "programs/an_explicit_default_is_not_inherited_by_its_variants.dm"
    ));
    let catalog = profile_catalog(&tree).expect("valid profile catalog");
    let chosen = profile_type(&tree).expect("default profile");
    let debug = selected_profile_type(&tree, Some(&TreePath::parse("/datum/demir/base/debug")))
        .expect("selected debug profile");
    let stale = selected_profile_type(&tree, Some(&TreePath::parse("/datum/demir/base/missing")))
        .expect("a stale selection falls back to the default");

    assert_eq!(
        tree.get(chosen).map(|decl| decl.path.to_string()),
        Some(String::from("/datum/demir/base"))
    );
    assert_eq!(
        tree.get(debug).map(|decl| decl.path.to_string()),
        Some(String::from("/datum/demir/base/debug"))
    );
    assert_eq!(catalog.profiles.len(), 2);
    assert_eq!(stale, chosen);
}

#[test]
fn profiles_require_one_explicit_default() {
    let (tree, _) = compile(fixture!("programs/profiles_require_one_explicit_default.dm"));

    assert_eq!(
        profile_type(&tree),
        Err(ProfileError::MissingDefault(vec![
            String::from("/datum/demir/goon"),
            String::from("/datum/demir/tg"),
        ]))
    );
    assert!(!has_profile(&tree));
}

#[test]
fn multiple_explicit_defaults_are_rejected() {
    let (tree, _) = compile(fixture!("programs/multiple_explicit_defaults_are_rejected.dm"));

    assert_eq!(
        profile_type(&tree),
        Err(ProfileError::MultipleDefaults(vec![
            String::from("/datum/demir/goon"),
            String::from("/datum/demir/tg"),
        ]))
    );
}

/// `..()` is half the reason the hooks are methods: a variant profile adjusts the base rather than
/// copying it.
#[test]
fn a_profile_hook_chains_to_its_parent() {
    let (tree, module) = compile(fixture!("programs/a_profile_hook_chains_to_its_parent.dm"));
    let profile =
        selected_profile_type(&tree, Some(&TreePath::parse("/datum/demir/base/debug"))).expect("debug profile");
    let bake = Bake::new_with_profile(
        &tree,
        &module,
        profile,
        vec![Atom {
            instance: 1,
            ty: tree.id_of(&TreePath::parse("/turf/wall")).expect("wall type"),
            position: Position::new(1, 1, 1),
            vars: Vec::new(),
        }],
        [1, 1, 1],
        Limits::default(),
        IconStates::default(),
        None,
    );

    assert_eq!(baked_state(&bake, 1), Some("base-debug"));
}

/// A proc that is not itself a hook has no `src` of its own to read the profile from.
#[test]
fn demir_profile_reaches_the_instance_from_an_ordinary_proc() {
    let (tree, module) = compile(fixture!(
        "programs/demir_profile_reaches_the_instance_from_an_ordinary_proc.dm"
    ));
    let bake = Bake::new(
        &tree,
        &module,
        vec![Atom {
            instance: 1,
            ty: tree.id_of(&TreePath::parse("/turf/wall")).expect("wall type"),
            position: Position::new(1, 1, 1),
            vars: Vec::new(),
        }],
        [1, 1, 1],
        Limits::default(),
        IconStates::default(),
    );

    assert_eq!(baked_state(&bake, 1), Some("lit"));
}

/// Memo safety is measured against the atom a hook was called about, not against `src`. A profile
/// has no position, so seeding from it would mark every neighbourhood read uncacheable and silently
/// cost every cache hit in a real bake.
#[test]
fn a_hook_reading_its_neighbours_still_caches() {
    let (tree, module) = compile(fixture!("programs/a_hook_reading_its_neighbours_still_caches.dm"));
    let ty = tree.id_of(&TreePath::parse("/turf/wall")).expect("wall type");
    let atoms = (1..=20)
        .map(|x| Atom {
            instance: x as u64,
            ty,
            position: Position::new(x, 1, 1),
            vars: Vec::new(),
        })
        .collect::<Vec<_>>();
    let bake = Bake::new(
        &tree,
        &module,
        atoms,
        [20, 1, 1],
        Limits::default(),
        IconStates::default(),
    );

    assert_eq!(bake.attempted, 20);
    assert_eq!(baked_state(&bake, 10), Some("12"));
    assert!(
        bake.cache_hits > 10,
        "the interior walls share a neighbourhood, so all but the ends come from the cache: {}",
        bake.cache_hits
    );
}

#[test]
fn profile_connections_match_complementary_roles_from_either_endpoint() {
    let (tree, module) = compile(fixture!(
        "programs/profile_connections_match_complementary_roles_from_either_endpoint.dm"
    ));
    let atom = |instance, path: &str, channel: &str| Atom {
        instance,
        ty: tree.id_of(&TreePath::parse(path)).expect("connection fixture type"),
        position: Position::new(instance as i32, 1, 1),
        vars: vec![(Identifier::from("channel"), Value::Text(channel.into()))],
    };
    let source_a = atom(1, "/obj/source", "a");
    let source_b = atom(2, "/obj/source", "b");
    let target_a = atom(3, "/obj/target", "a");
    let both_a = atom(4, "/obj/both", "a");
    let mut bake = Bake::new(
        &tree,
        &module,
        vec![source_a.clone(), source_b, target_a.clone(), both_a],
        [4, 1, 1],
        Limits::default(),
        IconStates::default(),
    );

    assert_eq!(bake.connections(1), vec![3, 4]);
    assert!(bake.connections(2).is_empty());
    assert_eq!(bake.connections(3), vec![1, 4]);
    assert_eq!(bake.connections(4), vec![1, 3]);
    let source_object = bake.object(1).expect("source runtime object");
    assert_eq!(
        bake.runtime
            .heap
            .object(source_object)
            .and_then(|object| object.vars.get(&Identifier::from("name"))),
        None,
    );

    let target_b = atom(3, "/obj/target", "b");
    bake.update(&tree, &module, vec![target_b], &[4]);
    assert!(bake.connections(1).is_empty());
    assert_eq!(bake.connections(2), vec![3]);
    assert_eq!(bake.connections(3), vec![2]);
}

#[test]
fn profile_highlights_are_clipped_edged_and_restored_across_edits() {
    let (tree, module) = compile(fixture!(
        "programs/profile_highlights_are_clipped_edged_and_restored_across_edits.dm"
    ));
    let port = |instance, position, span: f32| Atom {
        instance,
        ty: tree
            .id_of(&TreePath::parse("/obj/port"))
            .expect("highlight fixture type"),
        position,
        vars: vec![(Identifier::from("span"), Value::Num(span))],
    };
    let centered = port(1, Position::new(3, 3, 1), 3.0);
    // A rectangle reaching past the south-west corner is clipped, not rejected.
    let cornered = port(2, Position::new(1, 1, 1), 3.0);
    let mut bake = Bake::new(
        &tree,
        &module,
        vec![centered.clone(), cornered],
        [5, 5, 1],
        Limits::default(),
        IconStates::default(),
    );

    assert_eq!(bake.diagnostics.count(), 0, "{:#?}", bake.diagnostics);
    let [rect, tiles] = bake.highlights(1) else {
        panic!("both highlights should survive");
    };
    assert_eq!(rect.tiles.len(), 9);
    assert_eq!(rect.color, [0.0, 1.0, 0.0]);
    assert_eq!(rect.fill, 0.5);
    assert_eq!(rect.when, HIGHLIGHT_ALWAYS);
    assert_eq!(rect.label.as_deref(), Some("span"));
    assert_eq!(
        rect.tiles.first(),
        Some(&HighlightTile {
            position: [2, 2],
            edges: HIGHLIGHT_EDGE_SOUTH | HIGHLIGHT_EDGE_WEST,
        })
    );
    // An interior tile of a full rectangle faces nothing outward.
    assert_eq!(
        rect.tiles
            .iter()
            .find(|tile| tile.position == [3, 3])
            .map(|tile| tile.edges),
        Some(0)
    );
    assert_eq!(tiles.when, HIGHLIGHT_SELECTED, "the default is selection only");
    assert_eq!(tiles.tiles.len(), 2);
    assert_eq!(
        tiles.tiles.first(),
        Some(&HighlightTile {
            position: [3, 5],
            edges: HIGHLIGHT_EDGE_NORTH | HIGHLIGHT_EDGE_SOUTH | HIGHLIGHT_EDGE_WEST,
        })
    );

    assert_eq!(bake.highlights(2).first().map(|rect| rect.tiles.len()), Some(4));

    let port_object = bake.object(1).expect("port runtime object");
    assert_eq!(
        bake.runtime
            .heap
            .object(port_object)
            .and_then(|object| object.vars.get(&Identifier::from("name"))),
        None,
    );

    bake.update(&tree, &module, Vec::new(), &[1]);
    assert!(bake.highlights(1).is_empty());

    bake.update(&tree, &module, vec![centered], &[]);
    assert_eq!(bake.highlights(1).len(), 2);
}

#[test]
fn profile_ui_draws_widgets_and_reads_back_what_the_editor_remembers() {
    let (tree, module) = compile(fixture!(
        "programs/profile_ui_draws_widgets_and_reads_back_what_the_editor_remembers.dm"
    ));
    let mut bake = Bake::new(
        &tree,
        &module,
        vec![Atom {
            instance: 1,
            ty: tree.id_of(&TreePath::parse("/obj/panel")).expect("ui fixture type"),
            position: Position::new(1, 1, 1),
            vars: Vec::new(),
        }],
        [3, 3, 1],
        Limits::default(),
        IconStates::default(),
    );

    let commands = bake
        .ui(&tree, &module, Some(1), 7, Feedback::default())
        .expect("the hook should draw")
        .commands;
    let [
        Command::Begin { key, dock, .. },
        Command::Text { .. },
        Command::Button { .. },
        Command::Checkbox { value: true, .. },
        Command::Slider { key: slider, value, .. },
        Command::End,
    ] = commands.as_slice()
    else {
        panic!("unexpected command stream: {commands:#?}");
    };
    assert_eq!(key, "Panel", "the window keys off its own label");
    assert_eq!(slider, "Panel/glow", "widgets key off the windows enclosing them");
    assert_eq!(*dock, None, "a window only docks when the profile asks");
    assert_eq!(*value, 40.0, "the profile's own value starts the widget");

    let object = bake.object(1).expect("panel runtime object");
    assert_eq!(
        bake.runtime
            .heap
            .object(object)
            .and_then(|object| object.vars.get(&Identifier::from("name"))),
        None,
    );

    let feedback = Feedback {
        values: HashMap::from([
            (String::from("Panel/glow"), UiValue::Num(200.0)),
            (String::from("Panel/lit"), UiValue::Bool(false)),
        ]),
        clicks: HashSet::from([String::from("Panel/Reset")]),
        closed: HashSet::new(),
        interacted: true,
        ..Default::default()
    };
    let commands = bake
        .ui(&tree, &module, Some(1), 7, feedback)
        .expect("second frame")
        .commands;
    let [
        Command::Begin { .. },
        Command::Text { .. },
        Command::Button { .. },
        Command::Text { text, .. },
        Command::Checkbox { value: false, .. },
        Command::End,
    ] = commands.as_slice()
    else {
        panic!("unexpected command stream: {commands:#?}");
    };
    assert_eq!(text, "reset", "a press is answered on the frame after");

    let closed = Feedback {
        closed: HashSet::from([String::from("Panel")]),
        ..Default::default()
    };
    let commands = bake
        .ui(&tree, &module, Some(1), 7, closed)
        .expect("collapsed frame")
        .commands;
    assert!(
        matches!(commands.as_slice(), [Command::Begin { .. }, Command::End]),
        "a collapsed window draws nothing inside",
    );
}

#[test]
fn prepared_contents_are_visible_and_cleaned_up_with_their_placement() {
    let (tree, module) = compile(fixture!(
        "programs/prepared_contents_are_visible_and_cleaned_up_with_their_placement.dm"
    ));
    let atom = |instance: u64, path: &str, x: i32| Atom {
        instance,
        ty: tree.id_of(&TreePath::parse(path)).expect("fixture type"),
        position: Position::new(x, 1, 1),
        vars: Vec::new(),
    };
    let mut bake = Bake::new(
        &tree,
        &module,
        vec![atom(1, "/obj/container", 1), atom(2, "/obj/container", 2)],
        [2, 1, 1],
        Limits::default(),
        IconStates::default(),
    );

    let draw = |bake: &mut Bake| {
        bake.ui(&tree, &module, Some(1), 7, Feedback::default())
            .expect("the hook should draw")
            .commands
    };
    let expected = [
        Command::Begin {
            key: "Contents".into(),
            label: "Contents".into(),
            dock: None,
            size: None,
        },
        Command::Text {
            text: "1".into(),
            color: None,
        },
        Command::Text {
            text: "prepared item".into(),
            color: None,
        },
        Command::End,
    ];
    assert_eq!(draw(&mut bake), expected);

    bake.rebake(
        &tree,
        &module,
        Rebake {
            appearance: Some(0),
            ..Default::default()
        },
    );
    assert_eq!(draw(&mut bake), expected, "rebaking must not repopulate the container");

    let generated = [1, 2]
        .into_iter()
        .flat_map(|instance| {
            let container = bake.object(instance).expect("container runtime object");
            let item = bake.runtime.heap.object(container).expect("container").contents[0];
            let nested = bake.runtime.heap.object(item).expect("generated item").contents[0];
            [item, nested]
        })
        .collect::<Vec<_>>();

    bake.update(&tree, &module, vec![atom(1, "/obj/empty", 1)], &[2]);

    assert!(generated.into_iter().all(|id| bake.runtime.heap.object(id).is_none()));
    let replacement = bake.object(1).expect("replacement runtime object");
    assert!(
        bake.runtime
            .heap
            .object(replacement)
            .expect("replacement")
            .contents
            .is_empty()
    );
    assert!(bake.object(2).is_none());
}

#[test]
fn profile_ui_rejects_drawing_outside_a_window() {
    let (tree, module) = compile(fixture!("programs/profile_ui_rejects_drawing_outside_a_window.dm"));
    let mut bake = Bake::new(
        &tree,
        &module,
        Vec::new(),
        [1, 1, 1],
        Limits::default(),
        IconStates::default(),
    );

    let fault = bake
        .ui(&tree, &module, None, 0, Feedback::default())
        .expect_err("a widget outside imgui_begin is a fault");
    assert!(
        matches!(&fault.kind, FaultKind::InvalidOperation(message) if message.contains("outside of imgui_begin")),
        "{fault:?}",
    );
}

#[test]
fn imgui_procs_are_blocked_outside_the_ui_hook() {
    let (tree, module) = compile(fixture!("programs/imgui_procs_are_blocked_outside_the_ui_hook.dm"));
    let fault = Runtime::default()
        .run(
            &tree,
            &module,
            hook(&tree, &module.index, ProfileHook::Bake),
            None,
            None,
            Vec::new(),
            Limits::default(),
        )
        .expect_err("imgui only runs inside ui()");
    assert!(
        matches!(&fault.kind, FaultKind::Blocked(message) if message.contains("outside ui()")),
        "{fault:?}",
    );
}

#[test]
fn the_modular_preview_request_is_kept_only_with_an_interacted_frame() {
    let (tree, module) = compile(fixture!(
        "programs/demir_modular_preview_is_read_and_requested_from_ui.dm"
    ));
    let mut bake = Bake::new(
        &tree,
        &module,
        Vec::new(),
        [1, 1, 1],
        Limits::default(),
        IconStates::default(),
    );
    let clicked = |is_shown: bool, is_interacted: bool| Feedback {
        values: HashMap::from([(String::from("Panel/Flip"), UiValue::Bool(true))]),
        modular_preview: is_shown,
        interacted: is_interacted,
        ..Default::default()
    };

    let quiet = bake
        .ui(
            &tree,
            &module,
            None,
            0,
            Feedback {
                modular_preview: true,
                ..Default::default()
            },
        )
        .expect("a frame without a request draws");
    assert_eq!(quiet.modular_preview, None);
    assert!(!quiet.committed);

    let untouched = bake
        .ui(&tree, &module, None, 0, clicked(true, false))
        .expect("an untouched frame draws");
    assert_eq!(untouched.modular_preview, None, "a rolled-back frame requests nothing");
    assert!(!untouched.committed);

    let hidden = bake
        .ui(&tree, &module, None, 0, clicked(true, true))
        .expect("the interaction frame draws");
    assert!(hidden.committed, "the request alone counts as a write");
    assert_eq!(hidden.modular_preview, Some(false), "the last call in the frame wins");

    let after = bake
        .ui(
            &tree,
            &module,
            None,
            0,
            Feedback {
                interacted: true,
                ..Default::default()
            },
        )
        .expect("a frame draws");
    assert_eq!(after.modular_preview, None, "the request does not carry over");
}

#[test]
fn the_modular_preview_read_returns_the_editor_state() {
    let (tree, module) = compile(fixture!(
        "programs/demir_modular_preview_is_read_and_requested_from_ui.dm"
    ));
    let mut bake = Bake::new(
        &tree,
        &module,
        Vec::new(),
        [1, 1, 1],
        Limits::default(),
        IconStates::default(),
    );

    for (is_shown, requested) in [(true, false), (false, true)] {
        let frame = bake
            .ui(
                &tree,
                &module,
                None,
                0,
                Feedback {
                    values: HashMap::from([(String::from("Panel/Flip"), UiValue::Bool(true))]),
                    modular_preview: is_shown,
                    interacted: true,
                    ..Default::default()
                },
            )
            .expect("the frame draws");
        assert_eq!(frame.modular_preview, Some(requested));
    }
}

#[test]
fn the_modular_preview_procs_are_blocked_outside_the_ui_hook() {
    for name in [
        fixture!("programs/demir_modular_preview_outside_ui_is_blocked.dm"),
        fixture!("programs/demir_set_modular_preview_outside_ui_is_blocked.dm"),
    ] {
        let (tree, module) = compile(name);
        let fault = Runtime::default()
            .run(
                &tree,
                &module,
                hook(&tree, &module.index, ProfileHook::Bake),
                None,
                None,
                Vec::new(),
                Limits::default(),
            )
            .expect_err("the procs only run inside ui()");
        assert!(
            matches!(&fault.kind, FaultKind::Blocked(message) if message.contains("outside ui()")),
            "{fault:?}",
        );
    }
}

#[test]
fn profile_ui_state_survives_on_the_profile_and_a_committed_frame_rebakes() {
    let (tree, module) = compile(fixture!(
        "programs/profile_ui_state_survives_on_the_profile_and_a_committed_frame_rebakes.dm"
    ));
    let mut bake = Bake::new(
        &tree,
        &module,
        vec![Atom {
            instance: 1,
            ty: tree.id_of(&TreePath::parse("/obj/lamp")).expect("lamp type"),
            position: Position::new(2, 2, 1),
            vars: Vec::new(),
        }],
        [3, 3, 1],
        Limits::default(),
        IconStates::default(),
    );
    assert_eq!(baked_state(&bake, 1), Some("on"));
    let lit = bake.lighting.clone().expect("the light hook exports a lightmap");
    assert!(lit.tile(Position::new(2, 2, 1)).expect("lamp tile").corners[0][0] > 0.0);

    let unchecked = Feedback {
        values: HashMap::from([(String::from("Panel/lit"), UiValue::Bool(false))]),
        ..Default::default()
    };
    let quiet = bake
        .ui(&tree, &module, None, 0, unchecked.clone())
        .expect("a quiet frame still draws");
    assert!(!quiet.committed, "only the frame the viewer touched keeps its writes");
    assert!(
        quiet.rebake.is_empty(),
        "a rolled-back frame asked on behalf of state that no longer exists",
    );
    assert_eq!(baked_state(&bake, 1), Some("on"));

    let committed = bake
        .ui(
            &tree,
            &module,
            None,
            0,
            Feedback {
                interacted: true,
                ..unchecked
            },
        )
        .expect("the interaction frame draws");
    assert!(committed.committed);
    assert_eq!(
        committed.rebake,
        Rebake {
            appearance: Some(2),
            light: Some(2),
            highlight: None,
        },
        "the profile names the stages and the group its option drives",
    );

    let update = bake.rebake(&tree, &module, committed.rebake);
    assert_eq!(update.appearances, vec![1], "the lamp is re-derived from the new state");
    assert_eq!(baked_state(&bake, 1), Some("off"));
    assert_eq!(
        update.lighting,
        Some(0..9),
        "the source the panel switched off is re-solved"
    );
    assert!(
        bake.lighting
            .as_ref()
            .expect("lightmap")
            .tile(Position::new(2, 2, 1))
            .expect("lamp tile")
            .corners
            .iter()
            .all(|corner| corner[0] == 0.0),
        "clearing the neutral schema is what drops the range the lamp used to have",
    );
}

#[test]
fn profile_ui_radios_pick_one_mode_and_the_rebake_follows_it() {
    let (tree, module) = compile(fixture!(
        "programs/profile_ui_radios_pick_one_mode_and_the_rebake_follows_it.dm"
    ));
    let mut bake = Bake::new(
        &tree,
        &module,
        vec![Atom {
            instance: 1,
            ty: tree.id_of(&TreePath::parse("/obj/pipe")).expect("pipe type"),
            position: Position::new(1, 1, 1),
            vars: Vec::new(),
        }],
        [1, 1, 1],
        Limits::default(),
        IconStates::default(),
    );
    assert_eq!(
        baked_alpha(&bake, 1),
        Some(0.0),
        "the profile's own default wins at load"
    );

    let frame = bake
        .ui(&tree, &module, None, 0, Feedback::default())
        .expect("the hook should draw");
    let [
        _,
        Command::Radio {
            key: hidden,
            active: true,
            ..
        },
        Command::Radio {
            key: shown,
            active: false,
            ..
        },
        _,
    ] = frame.commands.as_slice()
    else {
        panic!("unexpected command stream: {:#?}", frame.commands);
    };
    assert_eq!(hidden, "Panel/Hidden");
    assert_eq!(shown, "Panel/Shown");

    let picked = bake
        .ui(
            &tree,
            &module,
            None,
            0,
            Feedback {
                clicks: HashSet::from([String::from("Panel/Shown")]),
                interacted: true,
                ..Default::default()
            },
        )
        .expect("the interaction frame draws");
    assert!(picked.committed, "picking a radio is an edit like any other");

    assert_eq!(
        picked.rebake,
        Rebake {
            appearance: Some(0),
            ..Default::default()
        }
    );
    assert_eq!(bake.rebake(&tree, &module, picked.rebake).appearances, vec![1]);
    assert_eq!(baked_alpha(&bake, 1), Some(128.0));

    let settled = bake
        .ui(&tree, &module, None, 0, Feedback::default())
        .expect("the frame after draws");
    let [
        _,
        Command::Radio { active: false, .. },
        Command::Radio { active: true, .. },
        _,
    ] = settled.commands.as_slice()
    else {
        panic!("the pick should hold: {:#?}", settled.commands);
    };
}

#[test]
fn a_rebake_group_narrows_the_pass_to_the_placements_that_carry_it() {
    let (tree, module) = compile(fixture!(
        "programs/a_rebake_group_narrows_the_pass_to_the_placements_that_carry_it.dm"
    ));
    let atom = |instance: u64, path: &str, x: i32| Atom {
        instance,
        ty: tree.id_of(&TreePath::parse(path)).expect("fixture type"),
        position: Position::new(x, 1, 1),
        vars: Vec::new(),
    };
    let mut bake = Bake::new(
        &tree,
        &module,
        vec![atom(1, "/obj/cable", 1), atom(2, "/obj/pipe", 3)],
        [3, 1, 1],
        Limits::default(),
        IconStates::default(),
    );

    let frame = bake
        .ui(
            &tree,
            &module,
            None,
            0,
            Feedback {
                values: HashMap::from([(String::from("Panel/tint"), UiValue::Num(80.0))]),
                interacted: true,
                ..Default::default()
            },
        )
        .expect("the interaction frame draws");
    assert_eq!(
        frame.rebake,
        Rebake {
            appearance: Some(1),
            ..Default::default()
        }
    );

    assert_eq!(
        bake.rebake(&tree, &module, frame.rebake).appearances,
        vec![1],
        "only the placement carrying the group is re-derived",
    );
    assert_eq!(baked_alpha(&bake, 1), Some(80.0));
    assert_eq!(
        baked_alpha(&bake, 2),
        Some(0.0),
        "the pipe keeps the appearance the load gave it",
    );

    assert_eq!(
        bake.rebake(&tree, &module, Rebake::default()),
        BakeUpdate::default(),
        "a frame that asks for nothing re-derives nothing",
    );
}

#[test]
fn a_rebake_group_covers_the_subtypes_of_what_it_named() {
    let (tree, module) = compile(fixture!(
        "programs/a_rebake_group_covers_the_subtypes_of_what_it_named.dm"
    ));
    let atom = |instance: u64, path: &str, x: i32| Atom {
        instance,
        ty: tree.id_of(&TreePath::parse(path)).expect("fixture type"),
        position: Position::new(x, 1, 1),
        vars: Vec::new(),
    };
    let mut bake = Bake::new(
        &tree,
        &module,
        vec![
            atom(1, "/obj/cable", 1),
            atom(2, "/obj/cable/layered", 2),
            atom(3, "/obj/pipe", 3),
        ],
        [3, 1, 1],
        Limits::default(),
        IconStates::default(),
    );

    // Nothing has changed, so a re-derivation reports nothing; what it walked is what matters.
    assert_eq!(bake.attempted, 3);
    bake.rebake(
        &tree,
        &module,
        Rebake {
            appearance: Some(1),
            ..Default::default()
        },
    );
    assert_eq!(
        bake.attempted, 5,
        "the named type and its subtype are redone, and the unrelated one is not",
    );
}

#[test]
fn defining_a_rebake_group_outside_initialize_is_blocked() {
    let (tree, module) = compile(fixture!(
        "programs/defining_a_rebake_group_outside_initialize_is_blocked.dm"
    ));
    let fault = Runtime::default()
        .run(
            &tree,
            &module,
            hook(&tree, &module.index, ProfileHook::Bake),
            None,
            None,
            Vec::new(),
            Limits::default(),
        )
        .expect_err("groups are declared once, at initialization");
    assert!(
        matches!(&fault.kind, FaultKind::Blocked(message) if message.contains("outside a profile's New()")),
        "{fault:?}",
    );
}

#[test]
fn node_groups_accept_blocker_lists_merge_and_allow_an_unblocked_group() {
    let (tree, module) = compile(fixture!(
        "programs/node_groups_accept_blocker_lists_merge_and_allow_an_unblocked_group.dm"
    ));
    let bake = Bake::new(
        &tree,
        &module,
        Vec::new(),
        [1, 1, 1],
        Limits::default(),
        IconStates::default(),
    );
    let cable = tree.id_of(&TreePath::parse("/obj/cable")).unwrap();
    let pipe = tree.id_of(&TreePath::parse("/obj/pipe")).unwrap();
    let closed = tree.id_of(&TreePath::parse("/turf/closed")).unwrap();
    let grille = tree.id_of(&TreePath::parse("/obj/grille")).unwrap();
    let window = tree.id_of(&TreePath::parse("/obj/window")).unwrap();

    assert_eq!(bake.node_groups().len(), 2);
    let cable_group = bake.node_groups().iter().find(|group| group.subtype == cable).unwrap();
    assert_eq!(cable_group.blockers, vec![closed, grille, window]);
    assert!(
        bake.node_groups()
            .iter()
            .find(|group| group.subtype == pipe)
            .unwrap()
            .blockers
            .is_empty()
    );
}

#[test]
fn node_groups_accept_matching_vars() {
    let (tree, module) = compile(fixture!("programs/node_groups_accept_matching_vars.dm"));
    let bake = Bake::new(
        &tree,
        &module,
        Vec::new(),
        [1, 1, 1],
        Limits::default(),
        IconStates::default(),
    );
    let matching = |path: &str| {
        let ty = tree.id_of(&TreePath::parse(path)).unwrap();
        bake.node_groups()
            .iter()
            .find(|group| group.subtype == ty)
            .unwrap()
            .matching
            .clone()
    };

    assert_eq!(matching("/obj/cable"), vec![Identifier::from("cable_layer")]);
    assert_eq!(
        matching("/obj/pipe"),
        vec![
            Identifier::from("piping_layer"),
            Identifier::from("pipe_color"),
            Identifier::from("hidden"),
        ]
    );
}

#[test]
fn node_groups_reject_invalid_arguments() {
    let (tree, module) = compile(fixture!("programs/node_groups_reject_invalid_arguments.dm"));
    for (path, message) in [
        (
            "/datum/demir/subtype",
            "demir_node_group subtype must be a type, got not a type",
        ),
        (
            "/datum/demir/blocker",
            "demir_node_group blocker must be a type, got not a type",
        ),
        (
            "/datum/demir/orientable",
            "demir_node_group orientable_subtype must be a descendant of the subtype, got /obj/other",
        ),
        (
            "/datum/demir/matching",
            "demir_node_group matching must name vars as text, got 2",
        ),
    ] {
        let profile = tree.id_of(&TreePath::parse(path)).expect("profile");
        let bake = Bake::new_with_profile(
            &tree,
            &module,
            profile,
            Vec::new(),
            [1, 1, 1],
            Limits::default(),
            IconStates::default(),
            None,
        );

        assert_eq!(bake.diagnostics.count(), 1, "{path}");
        assert_eq!(
            bake.diagnostics.entries[0].fault.kind,
            FaultKind::InvalidOperation(message.into()),
            "{path}"
        );
        assert!(bake.node_groups().is_empty(), "{path}");
    }
}

#[test]
fn node_groups_keep_valid_orientable_subtypes_and_ignore_conflicts() {
    let (tree, module) = compile(fixture!("programs/node_groups_accept_orientable_subtypes.dm"));
    let bake = Bake::new(
        &tree,
        &module,
        Vec::new(),
        [1, 1, 1],
        Limits::default(),
        IconStates::default(),
    );
    let group = &bake.node_groups()[0];
    assert_eq!(bake.node_groups().len(), 1);
    assert_eq!(group.subtype, tree.id_of(&TreePath::parse("/obj/pipe")).unwrap());
    assert_eq!(
        group.orientable_subtype,
        tree.id_of(&TreePath::parse("/obj/pipe/segment"))
    );
    assert_eq!(
        group.blockers,
        vec![
            tree.id_of(&TreePath::parse("/turf/closed")).unwrap(),
            tree.id_of(&TreePath::parse("/obj/grille")).unwrap(),
        ],
    );
    assert_eq!(group.orientations.len(), 1);
    assert_eq!(
        group.orientations[0].subtype,
        tree.id_of(&TreePath::parse("/obj/pipe/segment")).unwrap()
    );
    assert_eq!(group.orientations[0].direction, 1);
    assert_eq!(group.orientations[0].openings, 12);
}

#[test]
fn node_orientations_follow_profile_type_rules() {
    let (tree, module) = compile(fixture!("programs/node_orientations_follow_profile_type_rules.dm"));
    let bake = Bake::new(
        &tree,
        &module,
        Vec::new(),
        [1, 1, 1],
        Limits::default(),
        IconStates::default(),
    );
    assert_eq!(bake.diagnostics.count(), 0);
    let group = &bake.node_groups()[0];
    let openings = |path: &str| {
        let subtype = tree.id_of(&TreePath::parse(path)).unwrap();
        group
            .orientations
            .iter()
            .find(|rule| rule.subtype == subtype && rule.direction == 1)
            .map(|rule| rule.openings)
    };
    assert_eq!(openings("/obj/link"), Some(1));
    assert_eq!(openings("/obj/link/segment"), Some(3));
    assert_eq!(openings("/obj/link/junction"), Some(9));
}

#[test]
fn defining_a_node_group_outside_initialize_is_blocked() {
    let (tree, module) = compile(fixture!(
        "programs/defining_a_node_group_outside_initialize_is_blocked.dm"
    ));
    let fault = Runtime::default()
        .run(
            &tree,
            &module,
            hook(&tree, &module.index, ProfileHook::Bake),
            None,
            None,
            Vec::new(),
            Limits::default(),
        )
        .expect_err("node groups are declared once, at initialization");
    assert!(
        matches!(&fault.kind, FaultKind::Blocked(message) if message.contains("outside a profile's New()")),
        "{fault:?}",
    );
}

#[test]
fn rotations_follow_the_most_specific_registered_type() {
    let (tree, module) = compile(fixture!(
        "programs/rotations_follow_the_most_specific_registered_type.dm"
    ));
    let bake = Bake::new(
        &tree,
        &module,
        Vec::new(),
        [1, 1, 1],
        Limits::default(),
        IconStates::default(),
    );
    assert_eq!(bake.diagnostics.count(), 0);
    assert_eq!(bake.rotations().len(), 2);
    let directions = |path: &str| {
        let ty = tree.id_of(&TreePath::parse(path)).unwrap();
        crate::bake::rotation_for(&tree, bake.rotations(), ty).map(|rotation| rotation.directions.clone())
    };
    assert_eq!(directions("/obj/machine"), Some(vec![NORTH, SOUTH, EAST, WEST]));
    assert_eq!(directions("/obj/machine/tank"), Some(vec![NORTH, SOUTH, EAST, WEST]));
    assert_eq!(directions("/obj/machine/pump/fast"), Some(vec![NORTHEAST, SOUTHWEST]));
    assert_eq!(directions("/obj"), None);
}

#[test]
fn declaring_a_rotation_outside_initialize_is_blocked() {
    let (tree, module) = compile(fixture!(
        "programs/declaring_a_rotation_outside_initialize_is_blocked.dm"
    ));
    let fault = Runtime::default()
        .run(
            &tree,
            &module,
            hook(&tree, &module.index, ProfileHook::Bake),
            None,
            None,
            Vec::new(),
            Limits::default(),
        )
        .expect_err("rotations are declared once, at initialization");
    assert!(
        matches!(&fault.kind, FaultKind::Blocked(message) if message.contains("outside a profile's New()")),
        "{fault:?}",
    );
}

#[test]
fn a_ui_interaction_the_profile_ignores_keeps_nothing() {
    let (tree, module) = compile(fixture!(
        "programs/a_ui_interaction_the_profile_ignores_keeps_nothing.dm"
    ));
    let mut bake = Bake::new(
        &tree,
        &module,
        Vec::new(),
        [1, 1, 1],
        Limits::default(),
        IconStates::default(),
    );

    let frame = bake
        .ui(
            &tree,
            &module,
            None,
            0,
            Feedback {
                clicks: HashSet::from([String::from("Panel/Does nothing")]),
                interacted: true,
                ..Default::default()
            },
        )
        .expect("the hook should draw");

    assert!(
        !frame.committed,
        "a press the profile drops leaves nothing to keep, and so nothing to re-derive",
    );
}

#[test]
fn profile_ui_streams_stay_balanced_when_the_profile_leaves_them_open() {
    let (tree, module) = compile(fixture!(
        "programs/profile_ui_streams_stay_balanced_when_the_profile_leaves_them_open.dm"
    ));
    let mut bake = Bake::new(
        &tree,
        &module,
        Vec::new(),
        [1, 1, 1],
        Limits::default(),
        IconStates::default(),
    );

    let open = bake
        .ui(&tree, &module, None, 0, Feedback::default())
        .expect("the hook should draw");
    assert!(
        matches!(
            open.commands.as_slice(),
            [
                Command::Begin { .. },
                Command::Tree { .. },
                Command::Text { .. },
                Command::TreeEnd,
                Command::Text { .. },
                Command::End,
            ],
        ),
        "a missing imgui_end() is closed for the profile: {:#?}",
        open.commands,
    );

    let collapsed = Feedback {
        closed: HashSet::from([String::from("Panel/Nested")]),
        ..Default::default()
    };
    let closed = bake
        .ui(&tree, &module, None, 0, collapsed)
        .expect("the collapsed frame draws");
    assert!(
        matches!(
            closed.commands.as_slice(),
            [
                Command::Begin { .. },
                Command::Tree { .. },
                Command::TreeEnd,
                Command::Text { .. },
                Command::End,
            ],
        ),
        "a collapsed node closes itself, since the profile only pairs an open one: {:#?}",
        closed.commands,
    );
}

#[test]
fn profile_mouse_popup_opens_only_for_a_selection() {
    let (tree, module) = compile(fixture!("programs/profile_mouse_popup_opens_only_for_a_selection.dm"));
    let mut bake = Bake::new(
        &tree,
        &module,
        Vec::new(),
        [1, 1, 1],
        Limits::default(),
        IconStates::default(),
    );

    let requested = bake
        .ui(
            &tree,
            &module,
            None,
            0,
            Feedback {
                mouse_popup_requested: true,
                ..Default::default()
            },
        )
        .expect("the selection frame should draw");
    assert!(matches!(
        requested.commands.as_slice(),
        [
            Command::OpenPopup(crate::ui::PopupId::Mouse),
            Command::BeginPopup(crate::ui::PopupId::Mouse),
            Command::Text { .. },
            Command::EndPopup,
        ]
    ));

    let retained = bake
        .ui(
            &tree,
            &module,
            None,
            0,
            Feedback {
                open_popups: HashSet::from([crate::ui::PopupId::Mouse]),
                ..Default::default()
            },
        )
        .expect("an open popup should keep drawing");
    assert!(matches!(
        retained.commands.as_slice(),
        [
            Command::BeginPopup(crate::ui::PopupId::Mouse),
            Command::Text { .. },
            Command::EndPopup,
        ]
    ));

    let dismissed = bake
        .ui(&tree, &module, None, 0, Feedback::default())
        .expect("a dismissed popup should leave a balanced stream");
    assert!(matches!(
        dismissed.commands.as_slice(),
        [Command::BeginPopup(crate::ui::PopupId::Mouse), Command::EndPopup]
    ));
}

#[test]
fn malformed_connection_metadata_does_not_block_appearance_baking() {
    let (tree, module) = compile(fixture!(
        "programs/malformed_connection_metadata_does_not_block_appearance_baking.dm"
    ));
    let atom = Atom {
        instance: 1,
        ty: tree.id_of(&TreePath::parse("/obj/source")).expect("source type"),
        position: Position::new(1, 1, 1),
        vars: Vec::new(),
    };
    let bake = Bake::new(
        &tree,
        &module,
        vec![atom],
        [1, 1, 1],
        Limits::default(),
        IconStates::default(),
    );

    assert_eq!(bake.diagnostics.count(), 1);
    assert!(bake.connections(1).is_empty());
    assert_eq!(baked_state(&bake, 1), Some("baked"));
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
    let (tree, module) = compile(fixture!(
        "programs/lighting_hooks_build_and_incrementally_restore_the_lightmap.dm"
    ));
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
    let (tree, module) = compile(fixture!(
        "programs/lighting_hook_keeps_zero_radius_quadratic_sources.dm"
    ));
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
fn lighting_hook_offsets_source_origins_in_tile_units() {
    let (tree, module) = compile(fixture!(
        "programs/lighting_hook_offsets_source_origins_in_tile_units.dm"
    ));
    let lamp = tree.id_of(&TreePath::parse("/obj/lamp")).expect("lamp type");
    let bake = Bake::new(
        &tree,
        &module,
        vec![Atom {
            instance: 1,
            ty: lamp,
            position: Position::new(3, 2, 1),
            vars: Vec::new(),
        }],
        [5, 3, 1],
        Limits::default(),
        IconStates::default(),
    );
    let lighting = bake.lighting.as_ref().expect("offset source exports a lightmap");
    let west = lighting.tile(Position::new(1, 2, 1)).unwrap().corners[1][0];
    let east = lighting.tile(Position::new(4, 2, 1)).unwrap().corners[1][0];

    assert!(
        east > west,
        "a positive x offset must move the light east: {east} <= {west}"
    );
}

#[test]
fn appearance_offset_moves_a_wall_fixture_without_leaking_through_its_wall() {
    let (tree, module) = compile(fixture!(
        "programs/appearance_offset_moves_a_wall_fixture_without_leaking_through_its_wall.dm"
    ));
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
    let (tree, module) = compile(fixture!("programs/range_and_orange_spiral_out_with_areas_once.dm"));
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

/// What `view(2)` and `oview(2)` see from the centre of a 5 by 5 map.
fn view_from_the_centre(walls: &[(i32, i32)], curtains: &[(i32, i32)]) -> String {
    let (tree, module) = compile(fixture!("programs/view_from_the_centre.dm"));
    let ty = |path: &str| tree.id_of(&TreePath::parse(path)).expect("fixture type");
    let mut atoms = Vec::new();
    for y in 1..=5 {
        for x in 1..=5 {
            let turf = if walls.contains(&(x, y)) {
                "/turf/wall"
            } else {
                "/turf/floor"
            };
            for path in ["/area/zone", turf] {
                atoms.push(Atom {
                    instance: atoms.len() as u64 + 1,
                    ty: ty(path),
                    position: Position::new(x, y, 1),
                    vars: Vec::new(),
                });
            }
        }
    }
    for &(x, y) in curtains {
        atoms.push(Atom {
            instance: atoms.len() as u64 + 1,
            ty: ty("/obj/curtain"),
            position: Position::new(x, y, 1),
            vars: Vec::new(),
        });
    }
    let bake = Bake::new(
        &tree,
        &module,
        atoms,
        [5, 5, 1],
        Limits::default(),
        IconStates::default(),
    );

    assert_eq!(bake.diagnostics.count(), 0, "{:?}", bake.diagnostics);
    bake.appearances[&26]
        .vars
        .iter()
        .find(|(name, _)| name.as_str() == "name")
        .and_then(|(_, value)| value.as_text())
        .expect("the centre turf names what it saw")
        .to_owned()
}

/// The wall and the curtain hide what is behind them, and together they close the diagonal between them.
#[test]
fn view_hides_cells_behind_opaque_atoms() {
    assert_eq!(
        view_from_the_centre(&[(4, 3)], &[(3, 4)]),
        "3,3 2,2 2,3 2,4 3,2 3,4 4,2 4,3 1,1 1,2 1,3 1,4 1,5 2,1 2,5 3,1 4,1 5,1 5,2 | 2,2 2,3 2,4 3,2 3,4 4,2 4,3 \
         1,1 1,2 1,3 1,4 1,5 2,1 2,5 3,1 4,1 5,1 5,2"
    );
}

/// 3,5 is behind the pillar at 3,4, but both of its wall neighbours are seen.
#[test]
fn view_reveals_walls_between_seen_walls() {
    assert_eq!(
        view_from_the_centre(&[(3, 4), (2, 5), (3, 5), (4, 5)], &[]),
        "3,3 2,2 2,3 2,4 3,2 3,4 4,2 4,3 4,4 1,1 1,2 1,3 1,4 1,5 2,1 2,5 3,1 3,5 4,1 4,5 5,1 5,2 5,3 5,4 5,5 | 2,2 \
         2,3 2,4 3,2 3,4 4,2 4,3 4,4 1,1 1,2 1,3 1,4 1,5 2,1 2,5 3,1 3,5 4,1 4,5 5,1 5,2 5,3 5,4 5,5"
    );
}

#[test]
fn block_lists_turfs_inside_the_map() {
    let (tree, module) = compile(fixture!("programs/block_lists_turfs_inside_the_map.dm"));
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
    let bake = Bake::new(
        &tree,
        &module,
        atoms,
        [3, 3, 1],
        Limits::default(),
        IconStates::default(),
    );

    assert_eq!(bake.diagnostics.count(), 0, "{:?}", bake.diagnostics);
    let corner = bake.appearances[&2]
        .vars
        .iter()
        .find(|(name, _)| name.as_str() == "name")
        .and_then(|(_, value)| value.as_text())
        .expect("the corner turf names what it found");
    assert_eq!(corner, "1,1 2,1 1,2 2,2 | 2,1 3,1 2,2 3,2 2,3 3,3 | 3,2 | ");
}

#[test]
fn implicit_locate_searches_the_world() {
    let (tree, module) = compile(fixture!("programs/implicit_locate_searches_the_world.dm"));
    let ty = |path: &str| tree.id_of(&TreePath::parse(path)).expect("fixture type");
    let placements = [
        ("/area/zone", 1),
        ("/turf/floor", 1),
        ("/obj/decoy", 1),
        ("/area/zone", 2),
        ("/turf/floor", 2),
        ("/obj/wanted", 2),
    ];
    let atoms = placements
        .iter()
        .enumerate()
        .map(|(index, (path, x))| Atom {
            instance: index as u64 + 1,
            ty: ty(path),
            position: Position::new(*x, 1, 1),
            vars: Vec::new(),
        })
        .collect();
    let bake = Bake::new(
        &tree,
        &module,
        atoms,
        [2, 1, 1],
        Limits::default(),
        IconStates::default(),
    );

    assert_eq!(bake.diagnostics.count(), 0, "{:?}", bake.diagnostics);
    let name = bake.appearances[&2]
        .vars
        .iter()
        .find(|(name, _)| name.as_str() == "name")
        .and_then(|(_, value)| value.as_text())
        .expect("the first turf names what it found");
    assert_eq!(name, "2 1 1");
}

#[test]
fn baking_exports_icon_objects_and_ignores_timed_effects() {
    let (tree, module) = compile(fixture!(
        "programs/baking_exports_icon_objects_and_ignores_timed_effects.dm"
    ));
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
fn baking_exports_icon_and_icon_state_as_a_pair() {
    let (tree, module) = compile(fixture!("programs/baking_exports_icon_and_icon_state_as_a_pair.dm"));
    let state_only = tree
        .id_of(&TreePath::parse("/obj/state_only"))
        .expect("state-only type");
    let icon_only = tree.id_of(&TreePath::parse("/obj/icon_only")).expect("icon-only type");
    let bake = Bake::new(
        &tree,
        &module,
        vec![
            Atom {
                instance: 1,
                ty: state_only,
                position: Position::new(1, 1, 1),
                vars: Vec::new(),
            },
            Atom {
                instance: 2,
                ty: icon_only,
                position: Position::new(2, 1, 1),
                vars: Vec::new(),
            },
        ],
        [2, 1, 1],
        Limits::default(),
        IconStates::default(),
    );

    assert_eq!(bake.diagnostics.count(), 0, "{:?}", bake.diagnostics);
    assert_eq!(
        bake.appearances[&1].vars,
        [
            (Identifier::from("icon"), Value::Resource("state.dmi".into())),
            (Identifier::from("icon_state"), Value::Text("on".into())),
        ]
    );
    assert_eq!(
        bake.appearances[&2].vars,
        [
            (Identifier::from("icon"), Value::Resource("new.dmi".into())),
            (Identifier::from("icon_state"), Value::Text("steady".into())),
        ]
    );
}

/// The hook never reads `transform`, so `/obj/moved`'s initializer only runs because the profile
/// roots it for the export.
#[test]
fn baking_exports_transforms_the_compat_view_cannot_evaluate() {
    let (tree, module) = analyze_fixture(fixture!(
        "programs/baking_exports_transforms_the_compat_view_cannot_evaluate.dm"
    ));
    let profile = profile_type(&tree).expect("fixture profile");
    let roots = ProfileDefinition::resolve(&tree, &module.index, profile).entry_points();
    let module = codegen::generate_reachable(&module, &tree, &roots, &host_reads()).expect("fixture should compile");
    let atoms = ["/obj/moved", "/obj/turned", "/obj/plain"]
        .into_iter()
        .enumerate()
        .map(|(index, path)| Atom {
            instance: index as u64 + 1,
            ty: tree.id_of(&TreePath::parse(path)).expect("fixture type"),
            position: Position::new(index as i32 + 1, 1, 1),
            vars: Vec::new(),
        })
        .collect();
    let bake = Bake::new(
        &tree,
        &module,
        atoms,
        [3, 1, 1],
        Limits::default(),
        IconStates::default(),
    );
    let transform = |instance: u64| {
        bake.appearances.get(&instance).and_then(|appearance| {
            appearance
                .vars
                .iter()
                .find(|(name, _)| name.as_str() == "transform")
                .and_then(|(_, value)| match value {
                    Value::List(entries) => entries
                        .iter()
                        .map(|entry| entry.key.as_num())
                        .collect::<Option<Vec<_>>>(),
                    _ => None,
                })
        })
    };

    assert_eq!(bake.diagnostics.count(), 0, "{:?}", bake.diagnostics);
    assert_eq!(transform(1), Some(vec![1.0, 0.0, -4.0, 0.0, 1.0, -4.0]));
    let turned = transform(2).expect("turned transform");
    let expected = [0.0, 1.0, 0.0, -1.0, 0.0, 0.0];
    assert!(
        turned.iter().zip(expected).all(|(got, want)| (got - want).abs() < 1e-6),
        "{turned:?}"
    );
    assert_eq!(transform(3), None);
}

/// Nothing in the hook reads `icon_state`, so its initializer only exists because the host reads are
/// roots.
#[test]
fn baking_exports_appearance_vars_set_by_runtime_initializers() {
    let (tree, module) = analyze_fixture(fixture!(
        "programs/baking_exports_appearance_vars_set_by_runtime_initializers.dm"
    ));
    let profile = profile_type(&tree).expect("fixture profile");
    let roots = ProfileDefinition::resolve(&tree, &module.index, profile).entry_points();
    let module = codegen::generate_reachable(&module, &tree, &roots, &host_reads()).expect("fixture should compile");
    let atom = Atom {
        instance: 1,
        ty: tree.id_of(&TreePath::parse("/obj/picked")).expect("fixture type"),
        position: Position::new(1, 1, 1),
        vars: Vec::new(),
    };
    let bake = Bake::new(
        &tree,
        &module,
        vec![atom],
        [1, 1, 1],
        Limits::default(),
        IconStates::default(),
    );

    assert_eq!(bake.diagnostics.count(), 0, "{:?}", bake.diagnostics);
    assert!(
        bake.appearances[&1]
            .vars
            .contains(&(Identifier::from("icon_state"), Value::Text("only".into()))),
        "{:?}",
        bake.appearances[&1]
    );
}

#[test]
fn baking_preserves_prepared_scalar_appearance() {
    let (tree, module) = compile(fixture!("programs/baking_preserves_prepared_scalar_appearance.dm"));
    let atoms = [
        ("/obj/prepared", Vec::new()),
        ("/obj/prepared/changed_state", Vec::new()),
        ("/obj/prepared/changed_sheet", Vec::new()),
        (
            "/obj/prepared",
            vec![
                ("dir".into(), Value::Num(WEST as f32)),
                ("alpha".into(), Value::Num(64.0)),
                ("pixel_x".into(), Value::Num(-8.0)),
            ],
        ),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (path, vars))| Atom {
        instance: index as u64 + 1,
        ty: tree.id_of(&TreePath::parse(path)).expect("prepared type"),
        position: Position::new(index as i32 + 1, 1, 1),
        vars,
    })
    .collect::<Vec<_>>();
    let mut bake = Bake::new(
        &tree,
        &module,
        atoms.clone(),
        [4, 1, 1],
        Limits::default(),
        IconStates::default(),
    );

    assert_eq!(bake.diagnostics.count(), 0, "{:?}", bake.diagnostics);
    for (id, icon, state, dir) in [
        (1, "mapping.dmi", "prepared", NORTH),
        (2, "mapping.dmi", "baked", NORTH),
        (3, "prepared.dmi", "map", NORTH),
        (4, "mapping.dmi", "prepared", EAST),
    ] {
        let appearance = &bake.appearances[&id];
        for (name, expected) in [
            ("icon", Value::Resource(icon.into())),
            ("icon_state", Value::Text(state.into())),
            ("dir", Value::Num(dir as f32)),
            ("alpha", Value::Num(128.0)),
            ("pixel_x", Value::Num(4.0)),
            ("pixel_y", Value::Num(-3.0)),
            ("color", Value::Null),
        ] {
            assert!(
                appearance.vars.contains(&(name.into(), expected)),
                "placement {id} lost {name}: {appearance:?}"
            );
        }
        let object = bake.runtime.heap.object(bake.object(id).unwrap()).unwrap();
        assert_eq!(object.vars.get(&"bookkeeping".into()), None);
        assert_eq!(
            object.vars.get(&"dir".into()),
            Some(&GenericValue::Num(dir as f32)),
            "preparation persists while bake mutations roll back"
        );
    }
    let object = bake.runtime.heap.object(bake.object(2).unwrap()).unwrap();
    assert_eq!(
        object.vars.get(&"icon_state".into()).and_then(GenericValue::text),
        Some("prepared"),
        "the preview's icon state must roll back"
    );

    let initial = bake.appearances.clone();
    bake.update(&tree, &module, Vec::new(), &[4]);
    bake.update(&tree, &module, vec![atoms[3].clone()], &[]);
    assert_eq!(bake.diagnostics.count(), 0, "{:?}", bake.diagnostics);
    assert_eq!(
        bake.appearances, initial,
        "recreating a placement preserves prepared appearance"
    );
}

#[test]
fn baking_exports_sprite_lighting_roles() {
    let (tree, module) = compile(fixture!("programs/baking_exports_sprite_lighting_roles.dm"));
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
    let source = fixture!("programs/bake_cache_uses_instance_variables.dm");
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
    let (tree, module) = compile(fixture!(
        "programs/initialization_runs_once_for_a_bake_and_not_for_updates.dm"
    ));
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
    let (tree, module) = compile(fixture!("programs/initialization_fault_stops_all_object_hooks.dm"));
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
    let (tree, module) = compile(fixture!(
        "programs/neighbor_overlay_changes_are_exported_and_rolled_back.dm"
    ));
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
fn assigning_appearance_lists_empties_or_copies_them() {
    let (tree, module) = compile(fixture!(
        "programs/assigning_appearance_lists_empties_or_copies_them.dm"
    ));
    let ty = tree.id_of(&TreePath::parse("/obj/machine")).unwrap();
    let atoms = vec![Atom {
        instance: 1,
        ty,
        position: Position::new(1, 1, 1),
        vars: Vec::new(),
    }];
    let bake = Bake::new(
        &tree,
        &module,
        atoms,
        [1, 1, 1],
        Limits::default(),
        IconStates::default(),
    );

    assert_eq!(bake.diagnostics.count(), 0);
    let appearance = &bake.appearances[&1];
    assert_eq!(appearance.underlays.len(), 1, "a list assignment copies the list");
    assert_eq!(appearance.overlays.len(), 1, "a null assignment leaves an empty list");
    assert_eq!(
        appearance.overlays[0]
            .vars
            .iter()
            .find(|(name, _)| name.as_str() == "icon_state")
            .map(|(_, value)| value),
        Some(&core::types::Value::Text("edge".into()))
    );
}

#[test]
fn shared_overlay_graphs_respect_the_export_budget() {
    let (tree, module) = compile(fixture!("programs/shared_overlay_graphs_respect_the_export_budget.dm"));
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
            fixture!("programs/executes_arithmetic_direct_calls_and_defaults.dm"),
            "test",
        ),
        7.into()
    );
}

#[test]
fn nonconstant_defaults_can_read_earlier_parameters() {
    assert_eq!(
        run(
            fixture!("programs/nonconstant_defaults_can_read_earlier_parameters.dm"),
            "test",
        ),
        75.into()
    );
}

#[test]
fn executes_phi_lowered_fibonacci_loop() {
    assert_eq!(
        run(fixture!("programs/executes_phi_lowered_fibonacci_loop.dm"), "test",),
        55.into()
    );
}

#[test]
fn conditional_fallthrough_preserves_both_phi_edges() {
    assert_eq!(
        run(
            fixture!("programs/conditional_fallthrough_preserves_both_phi_edges.dm"),
            "test",
        ),
        12.into()
    );
}

#[test]
fn associative_iteration_preserves_keys_and_values() {
    assert_eq!(
        run(
            fixture!("programs/associative_iteration_preserves_keys_and_values.dm"),
            "test",
        ),
        14.into()
    );
}

#[test]
fn labelled_break_leaves_a_plain_block() {
    assert_eq!(
        run(fixture!("programs/labelled_break_leaves_a_plain_block.dm"), "test",),
        1.into()
    );
}

#[test]
fn labelled_break_inside_do_while_leaves_the_block() {
    assert_eq!(
        run(
            fixture!("programs/labelled_break_inside_do_while_leaves_the_block.dm"),
            "test",
        ),
        1.into()
    );
}

#[test]
fn repeated_local_labels_create_distinct_blocks() {
    assert_eq!(
        run(
            fixture!("programs/repeated_local_labels_create_distinct_blocks.dm"),
            "test",
        ),
        2.into()
    );
}

#[test]
fn repeated_local_labels_preserve_values_from_their_predecessors() {
    assert_eq!(
        run(
            fixture!("programs/repeated_local_labels_preserve_values_from_their_predecessors.dm"),
            "test",
        ),
        7.into()
    );
}

#[test]
fn descending_ranges_use_the_steps_sign() {
    assert_eq!(
        run(fixture!("programs/descending_ranges_use_the_steps_sign.dm"), "test",),
        9.into()
    );
}

#[test]
fn switch_cases_match_every_listed_value() {
    assert_eq!(
        run(fixture!("programs/switch_cases_match_every_listed_value.dm"), "test",),
        GenericValue::from("ns ns ew ew none")
    );
}

#[test]
fn null_arguments_take_the_default() {
    assert_eq!(
        run(fixture!("programs/null_arguments_take_the_default.dm"), "test",),
        GenericValue::from("5 7 3")
    );
}

#[test]
fn virtual_dispatch_super_dot_and_defaults() {
    assert_eq!(
        run(fixture!("programs/virtual_dispatch_super_dot_and_defaults.dm"), "test",),
        9.into()
    );
}

#[test]
fn sandbox_faults_are_not_catchable() {
    let (tree, module) = compile(fixture!("programs/sandbox_faults_are_not_catchable.dm"));
    let fault = Runtime::default()
        .run(
            &tree,
            &module,
            proc(&tree, &module.index, "test"),
            None,
            None,
            Vec::new(),
            Limits::default(),
        )
        .expect_err("sandbox operation should fault");
    assert_eq!(fault.kind, FaultKind::Blocked("shell".into()));
}

#[test]
fn world_log_output_uses_a_field_reference() {
    let (tree, module) = compile(fixture!("programs/world_log_output_uses_a_field_reference.dm"));
    let mut runtime = Runtime::default();
    let result = runtime
        .run(
            &tree,
            &module,
            proc(&tree, &module.index, "test"),
            None,
            None,
            Vec::new(),
            Limits::default(),
        )
        .expect("fixture should execute");

    assert_eq!(result, GenericValue::Null);
    assert_eq!(runtime.output(), &[String::from("hello")]);
}

#[test]
fn locate_in_searches_the_container() {
    let result = run(fixture!("programs/locate_in_searches_the_container.dm"), "test");

    assert_eq!(result, GenericValue::from("1 1 1 1 1"));
}

#[test]
fn implicit_locate_uses_the_declared_type() {
    let result = run(fixture!("programs/implicit_locate_uses_the_declared_type.dm"), "test");

    assert_eq!(result, GenericValue::from("1 1 1 1 1"));
}

#[test]
fn initial_reads_the_builtin_type_vars() {
    let result = run(fixture!("programs/initial_reads_the_builtin_type_vars.dm"), "test");

    assert_eq!(result, GenericValue::from("/obj/parent /obj/parent/child /obj/parent"));
}

#[test]
fn a_proc_that_does_not_wait_still_runs() {
    let result = run(fixture!("programs/a_proc_that_does_not_wait_still_runs.dm"), "test");

    assert_eq!(result, GenericValue::Num(3.0));
}

#[test]
fn vars_reads_show_current_values() {
    let result = run(fixture!("programs/vars_reads_show_current_values.dm"), "test");

    assert_eq!(result, GenericValue::from("after kept same"));
}

#[test]
fn a_scope_read_of_an_undeclared_var_faults() {
    let (tree, module) = compile(fixture!("programs/a_scope_read_of_an_undeclared_var_faults.dm"));
    let fault = Runtime::default()
        .run(
            &tree,
            &module,
            proc(&tree, &module.index, "test"),
            None,
            None,
            Vec::new(),
            Limits::default(),
        )
        .expect_err("the path's type has no such var");
    assert_eq!(fault.kind, FaultKind::MissingVariable("only_child".into()));
}

#[test]
fn a_var_and_a_proc_can_share_a_name() {
    let result = run(fixture!("programs/a_var_and_a_proc_can_share_a_name.dm"), "test");

    assert_eq!(result, GenericValue::from("2 2 south"));
}

#[test]
fn datums_keep_their_own_coordinate_vars() {
    let result = run(fixture!("programs/datums_keep_their_own_coordinate_vars.dm"), "test");

    assert_eq!(result, GenericValue::from("4.5 2"));
}

#[test]
fn sized_type_vars_start_as_lists() {
    let result = run(fixture!("programs/sized_type_vars_start_as_lists.dm"), "test");

    assert_eq!(result, GenericValue::from("3 8 0  2"));
}

#[test]
fn icon_states_answer_from_the_host_table() {
    let (tree, module) = compile(fixture!("programs/icon_states_answer_from_the_host_table.dm"));
    let mut runtime = Runtime::default();
    runtime.icons = IconStates::new([(
        String::from("icons/walls.dmi"),
        vec![String::from("wall0"), String::from("wall1"), String::from("wall0")],
        [32, 32],
    )]);
    let result = runtime
        .run(
            &tree,
            &module,
            proc(&tree, &module.index, "test"),
            None,
            None,
            Vec::new(),
            Limits::default(),
        )
        .expect("fixture should execute");

    assert_eq!(result, GenericValue::from("2 wall0 wall1 2 0"));
}

#[test]
fn unsupported_output_targets_are_blocked() {
    let (tree, module) = compile(fixture!("programs/unsupported_output_targets_are_blocked.dm"));
    let fault = Runtime::default()
        .run(
            &tree,
            &module,
            proc(&tree, &module.index, "test"),
            None,
            None,
            Vec::new(),
            Limits::default(),
        )
        .expect_err("unsupported output target should fault");

    assert_eq!(fault.kind, FaultKind::Blocked("output".into()));
}

#[test]
fn instruction_budget_stops_infinite_control_flow() {
    let (tree, module) = compile(fixture!("programs/instruction_budget_stops_infinite_control_flow.dm"));
    let fault = Runtime::default()
        .run(
            &tree,
            &module,
            proc(&tree, &module.index, "test"),
            None,
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
        run(fixture!("programs/dm_throw_is_caught_with_its_value.dm"), "test",),
        49.into()
    );
}

#[test]
fn values_live_across_try_and_catch_merge_correctly() {
    assert_eq!(
        run(
            fixture!("programs/values_live_across_try_and_catch_merge_correctly.dm"),
            "test",
        ),
        11.into()
    );
}

#[test]
fn try_inside_a_loop_preserves_frame_locals() {
    assert_eq!(
        run(fixture!("programs/try_inside_a_loop_preserves_frame_locals.dm"), "test",),
        6.into()
    );
}

#[test]
fn parameter_defaults_are_applied_before_try_frame_storage() {
    assert_eq!(
        run(
            fixture!("programs/parameter_defaults_are_applied_before_try_frame_storage.dm"),
            "test",
        ),
        5.into()
    );
}

#[test]
fn same_type_super_observes_reassigned_arguments() {
    assert_eq!(
        run(
            fixture!("programs/same_type_super_observes_reassigned_arguments.dm"),
            "test",
        ),
        9.into()
    );
}

#[test]
fn static_state_is_shared_and_dynamic_dm_calls_work() {
    assert_eq!(
        run(
            fixture!("programs/static_state_is_shared_and_dynamic_dm_calls_work.dm"),
            "test",
        ),
        14.into()
    );
}

#[test]
fn reachable_codegen_executes_like_full_codegen() {
    let source = fixture!("programs/reachable_codegen_executes_like_full_codegen.dm");
    let (tree, ir_module) = analyze_fixture(source);
    let entry = proc(&tree, &ir_module.index, "entry");
    let full = codegen::generate(&ir_module).expect("full codegen");
    let selected = codegen::generate_reachable(&ir_module, &tree, &[entry], &[]).expect("reachable codegen");

    let execute = |module: &codegen::Module| {
        let mut runtime = Runtime::default();
        let value = runtime
            .run(&tree, module, entry, None, None, Vec::new(), Limits::default())
            .expect("fixture should execute");
        (value, runtime.take_output())
    };

    assert_eq!(execute(&selected), execute(&full));
    let unreachable = proc(&tree, &ir_module.index, "unreachable");
    assert!(selected.function_for_proc(unreachable).is_none());
}

#[test]
fn named_and_arglist_arguments_keep_their_layout() {
    assert_eq!(
        run(
            fixture!("programs/named_and_arglist_arguments_keep_their_layout.dm"),
            "test",
        ),
        123.into()
    );
}

#[test]
fn positional_arguments_fill_parameters_left_open_by_named_arguments() {
    assert_eq!(
        run(
            fixture!("programs/positional_arguments_fill_parameters_left_open_by_named_arguments.dm"),
            "test",
        ),
        123.into()
    );
}

#[test]
fn args_is_a_dm_list_with_the_supplied_length() {
    assert_eq!(
        run(
            fixture!("programs/args_is_a_dm_list_with_the_supplied_length.dm"),
            "test",
        ),
        3.into()
    );
}

#[test]
fn trailing_list_comma_does_not_add_an_entry() {
    assert_eq!(
        run(
            fixture!("programs/trailing_list_comma_does_not_add_an_entry.dm"),
            "test",
        ),
        4.into()
    );
}

#[test]
fn type_paths_remain_types_as_associative_list_keys() {
    assert_eq!(
        run(
            fixture!("programs/type_paths_remain_types_as_associative_list_keys.dm"),
            "test",
        ),
        3.into()
    );
}

#[test]
fn a_sized_local_list_declaration_allocates_its_entries() {
    assert_eq!(
        run(
            fixture!("programs/a_sized_local_list_declaration_allocates_its_entries.dm"),
            "test",
        ),
        34.into()
    );
}

#[test]
fn list_iteration_uses_a_stable_entry_snapshot() {
    assert_eq!(
        run(
            fixture!("programs/list_iteration_uses_a_stable_entry_snapshot.dm"),
            "test",
        ),
        66.into()
    );
}

#[test]
fn compound_list_operators_mutate_aliases_but_plain_operators_copy() {
    assert_eq!(
        run(
            fixture!("programs/compound_list_operators_mutate_aliases_but_plain_operators_copy.dm"),
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
        run(fixture!("programs/null_is_the_identity_for_addition.dm"), "test",),
        "ab".into()
    );

    assert_eq!(
        run(fixture!("programs/null_is_the_identity_for_addition-2.dm"), "test",),
        "x".into()
    );

    assert_eq!(
        run(fixture!("programs/null_is_the_identity_for_addition-3.dm"), "test",),
        "closed".into()
    );
}

#[test]
fn rgb2num_splits_channels_and_keeps_spelled_alpha() {
    assert_eq!(
        run(
            fixture!("programs/rgb2num_splits_channels_and_keeps_spelled_alpha.dm"),
            "test",
        ),
        "243,255,250,196|0,255,136|3".into()
    );
}

#[test]
fn typed_iteration_filters_the_iterated_value() {
    assert_eq!(
        run(
            fixture!("programs/typed_iteration_filters_the_iterated_value.dm"),
            "test",
        ),
        2.into()
    );
}

#[test]
fn one_argument_istype_uses_the_declared_type() {
    assert_eq!(
        run(
            fixture!("programs/one_argument_istype_uses_the_declared_type.dm"),
            "test",
        ),
        0.into()
    );
}

#[test]
fn initial_reads_the_declaration_instead_of_the_current_field() {
    assert_eq!(
        run(
            fixture!("programs/initial_reads_the_declaration_instead_of_the_current_field.dm"),
            "test",
        ),
        5.into()
    );
}

#[test]
fn initial_of_a_local_falls_back_to_its_current_value() {
    assert_eq!(
        run(
            fixture!("programs/initial_of_a_local_falls_back_to_its_current_value.dm"),
            "test",
        ),
        9.into()
    );
}

#[test]
fn safe_calls_on_null_return_null() {
    assert_eq!(
        run(fixture!("programs/safe_calls_on_null_return_null.dm"), "test",),
        GenericValue::Null
    );
}

#[test]
fn safe_index_writes_on_null_are_discarded() {
    assert_eq!(
        run(fixture!("programs/safe_index_writes_on_null_are_discarded.dm"), "test",),
        3.into()
    );
}

#[test]
fn call_depth_stops_recursive_functions() {
    let (tree, module) = compile(fixture!("programs/call_depth_stops_recursive_functions.dm"));
    let fault = Runtime::default()
        .run(
            &tree,
            &module,
            proc(&tree, &module.index, "test"),
            None,
            None,
            Vec::new(),
            Limits {
                call_depth: 4,
                ..Limits::default()
            },
        )
        .expect_err("call depth should stop recursion");
    assert_eq!(fault.kind, FaultKind::CallDepth);
    assert_eq!(fault.frames.len(), 4);
    assert!(
        fault.frames.iter().all(|frame| frame.location.begin.line == 2),
        "{fault:?}"
    );
}

#[test]
fn a_bake_fault_reports_each_frame_line() {
    let (tree, module) = compile(fixture!("programs/a_bake_fault_reports_each_frame_line.dm"));
    let atom = Atom {
        instance: 1,
        ty: tree.id_of(&TreePath::parse("/turf/floor")).expect("floor should exist"),
        position: Position::new(1, 1, 1),
        vars: Vec::new(),
    };
    let bake = Bake::new(
        &tree,
        &module,
        vec![atom],
        [1, 1, 1],
        Limits::default(),
        IconStates::default(),
    );

    assert_eq!(bake.diagnostics.count(), 1);
    let fault = &bake.diagnostics.entries[0].fault;
    let frames = fault
        .frames
        .iter()
        .map(|frame| (crate::proc_name(&module, frame.proc), frame.location.begin.line))
        .collect::<Vec<_>>();
    assert_eq!(
        frames,
        [("/datum/demir/test::reboot_from", 10), ("/datum/demir/test::bake", 6)]
    );
    assert_eq!(fault.location.begin.line, 10);
    assert_eq!(bake.hook_of(fault), Some(ProfileHook::Bake));
}

#[test]
fn a_ui_fault_clears_when_the_target_changes() {
    let (tree, module) = compile(fixture!("programs/a_ui_fault_clears_when_the_target_changes.dm"));
    let atom = Atom {
        instance: 1,
        ty: tree.id_of(&TreePath::parse("/turf/floor")).expect("floor should exist"),
        position: Position::new(1, 1, 1),
        vars: Vec::new(),
    };
    let mut bake = Bake::new(
        &tree,
        &module,
        vec![atom],
        [1, 1, 1],
        Limits::default(),
        IconStates::default(),
    );

    bake.ui(&tree, &module, Some(1), 0, Feedback::default())
        .expect_err("ui() reboots with a target");
    let fault = bake.ui_fault.as_ref().expect("the failed frame is kept");
    assert_eq!(fault.location.begin.line, 5);
    assert_eq!(bake.hook_of(fault), Some(ProfileHook::Ui));

    bake.ui(&tree, &module, None, 0, Feedback::default())
        .expect("ui() draws nothing without a target");
    assert!(bake.ui_fault.is_none());
}

#[test]
fn a_ui_fault_from_an_interaction_stays_listed() {
    let (tree, module) = compile(fixture!("programs/a_ui_fault_from_an_interaction_stays_listed.dm"));
    let atom = Atom {
        instance: 1,
        ty: tree.id_of(&TreePath::parse("/turf/floor")).expect("floor should exist"),
        position: Position::new(1, 1, 1),
        vars: Vec::new(),
    };
    let mut bake = Bake::new(
        &tree,
        &module,
        vec![atom],
        [1, 1, 1],
        Limits::default(),
        IconStates::default(),
    );
    let slider = |hours: f32| Feedback {
        values: HashMap::from([(String::from("Panel/Hour"), UiValue::Num(hours))]),
        interacted: true,
        ..Default::default()
    };

    bake.ui(&tree, &module, None, 0, Feedback::default())
        .expect("an idle frame draws");
    assert!(bake.ui_fault.is_none());

    bake.ui(&tree, &module, None, 0, slider(14.0))
        .expect_err("moving the slider reboots");
    let fault = bake.ui_fault.as_ref().expect("the failed frame is kept");
    assert_eq!(fault.location.begin.line, 13);

    bake.ui(&tree, &module, None, 0, Feedback::default())
        .expect("the idle frame after the fault draws");
    assert!(
        bake.ui_fault.is_some(),
        "an idle frame must not forget what the interaction raised"
    );

    bake.ui(&tree, &module, None, 0, slider(10.0))
        .expect("an interaction that changes nothing draws");
    assert!(bake.ui_fault.is_none());
}

#[test]
fn an_instruction_fault_rolls_back_heap_changes() {
    let (tree, module) = compile(fixture!("programs/an_instruction_fault_rolls_back_heap_changes.dm"));
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
            proc(&tree, &module.index, "test"),
            None,
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
    let (tree, module) = compile(fixture!(
        "programs/evaluation_reports_randomness_and_nonlocal_world_reads.dm"
    ));
    let mut runtime = Runtime::default();
    runtime.global = Some(
        runtime
            .heap
            .alloc_object(Object::new(TypeId::ROOT))
            .expect("global should allocate"),
    );
    let mut evaluator = Evaluator::new(&mut runtime, &tree, &module, Limits::default(), None);
    evaluator
        .call(proc(&tree, &module.index, "test"), None, Vec::new())
        .expect("fixture should execute");
    assert!(evaluator.position_sensitive);
    assert!(!evaluator.memo_safe);
}

#[test]
fn intrinsic_procs_run_in_rust_instead_of_their_body() {
    let (tree, module) = compile(fixture!(
        "programs/intrinsic_procs_run_in_rust_instead_of_their_body.dm"
    ));

    let root = std::env::temp_dir().join(format!("dmed-intrinsic-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("temp dir");
    std::fs::write(root.join("tips.txt"), "first\nsecond\n").expect("fixture file");

    let mut runtime = Runtime::default();
    runtime.world.root = Some(root.clone());

    let value = runtime
        .run(
            &tree,
            &module,
            proc(&tree, &module.index, "lines"),
            None,
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
                proc(&tree, &module.index, "banned"),
                None,
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
            proc(&tree, &module.index, "reboot"),
            None,
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
    let (tree, module) = compile(fixture!("programs/file2list_without_a_root_is_blocked.dm"));

    let fault = Runtime::default()
        .run(
            &tree,
            &module,
            proc(&tree, &module.index, "lines"),
            None,
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
            fixture!("programs/world_vars_are_readable_and_writable_through_world_procs.dm"),
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
            fixture!("programs/image_constructor_fills_the_object_new_allocated.dm"),
            "test",
        ),
        "state".into()
    );
}

/// `matrix(M, ...)` is the in-place form every `/matrix` method in `stddef.dm` routes through, so
/// it must not write the argument list onto `a`/`b`/`c` the way the six-number form does.
#[test]
fn matrix_in_place_form_leaves_the_matrix_alone() {
    assert_eq!(
        run(
            fixture!("programs/matrix_in_place_form_leaves_the_matrix_alone.dm"),
            "test",
        ),
        1.into()
    );
}

#[test]
fn matrix_operations_follow_byond() {
    assert_eq!(
        run(fixture!("programs/matrix_operations_follow_byond.dm"), "test"),
        "2,6,10;1;12;0,1;1;1;-4".into()
    );
}

/// `/alist` is a list, not a plain object, so `new` has to take the same path `/list` does.
#[test]
fn new_alist_is_a_list_seeded_from_its_pairs() {
    assert_eq!(
        run(
            fixture!("programs/new_alist_is_a_list_seeded_from_its_pairs.dm"),
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
        run(fixture!("programs/sound_constructor_keeps_its_file.dm"), "test",),
        GenericValue::Resource("beep.ogg".into())
    );
}

/// `PROC_REF(X)` is `nameof(.proc/X)`, and tgstation spells almost every callback that way.
#[test]
fn nameof_resolves_a_proc_path_to_its_name() {
    assert_eq!(
        run(fixture!("programs/nameof_resolves_a_proc_path_to_its_name.dm"), "test",),
        "work".into()
    );
}

#[test]
fn proc_macro_names_the_enclosing_proc() {
    assert_eq!(
        run(fixture!("programs/proc_macro_names_the_enclosing_proc.dm"), "test"),
        "global_name work work".into()
    );
}

#[test]
fn proc_macro_is_the_proc_path() {
    assert_eq!(
        run(fixture!("programs/proc_macro_is_the_proc_path.dm"), "test"),
        3.into()
    );
}

#[test]
fn proc_macro_text_keeps_the_declared_keyword() {
    assert_eq!(
        run(
            fixture!("programs/proc_macro_text_keeps_the_declared_keyword.dm"),
            "test"
        ),
        "/mob/proc/plain,/mob/verb/spoken,/mob/child/plain,/mob/child/spoken,/mob/proc/plain,/mob/verb/spoken,1".into()
    );
}

#[test]
fn proc_macro_calls_back_into_a_proc_block_proc() {
    assert_eq!(
        run(
            fixture!("programs/proc_macro_calls_back_into_a_proc_block_proc.dm"),
            "test"
        ),
        3.into()
    );
}

/// Non-constant var defaults lower as hidden procs, which must not count as the current proc.
#[test]
fn proc_macro_is_null_in_a_var_initializer() {
    assert_eq!(
        run(fixture!("programs/proc_macro_is_null_in_a_var_initializer.dm"), "test"),
        GenericValue::Null
    );
}

#[test]
fn type_macro_is_the_declaring_type() {
    assert_eq!(
        run(fixture!("programs/type_macro_is_the_declaring_type.dm"), "test"),
        3.into()
    );
}

#[test]
fn type_macro_is_null_in_a_global_proc() {
    assert_eq!(
        run(fixture!("programs/type_macro_is_null_in_a_global_proc.dm"), "test"),
        GenericValue::Null
    );
}

#[test]
fn type_macro_can_be_instantiated() {
    assert_eq!(
        run(fixture!("programs/type_macro_can_be_instantiated.dm"), "test"),
        7.into()
    );
}

#[test]
fn type_macro_names_the_owner_in_a_var_initializer() {
    assert_eq!(
        run(
            fixture!("programs/type_macro_names_the_owner_in_a_var_initializer.dm"),
            "test"
        ),
        1.into()
    );
}

#[test]
fn type_macro_is_an_identifier_after_undef() {
    assert_eq!(
        run(fixture!("programs/type_macro_is_an_identifier_after_undef.dm"), "test"),
        5.into()
    );
}

/// A global proc called from inside a method binds to the global. `find_proc` would reach the same
/// body, but only after failing a lookup on the runtime type of `src` at every call.
#[test]
fn a_global_proc_called_from_a_method_reaches_the_global() {
    assert_eq!(
        run(
            fixture!("programs/a_global_proc_called_from_a_method_reaches_the_global.dm"),
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
            fixture!("programs/a_method_of_the_same_name_still_shadows_the_global.dm"),
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
            fixture!("programs/appearance_arguments_follow_the_declared_signature.dm"),
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
        run(fixture!("programs/mutable_appearance_takes_an_appearance.dm"), "test",),
        "src".into()
    );
}

/// `RemoveAll` drops every occurrence and answers how many went, which is how `list_clear_nulls`
/// on a tgstation downstream asks whether the list held any nulls.
#[test]
fn list_remove_all_reports_how_many_it_dropped() {
    assert_eq!(
        run(
            fixture!("programs/list_remove_all_reports_how_many_it_dropped.dm"),
            "test",
        ),
        331.into()
    );
}

/// `Splice(Start, End, Item...)` cuts the range and drops the items in its place.
#[test]
fn list_splice_replaces_a_range() {
    assert_eq!(
        run(fixture!("programs/list_splice_replaces_a_range.dm"), "test",),
        "axyc".into()
    );
}

/// List methods use the object tree like datum methods. A user declaration replaces the prelude
/// intrinsic and receives the list itself as `src`.
#[test]
fn list_methods_are_overridable_dm_procs_with_list_src() {
    assert_eq!(
        run(
            fixture!("programs/list_methods_are_overridable_dm_procs_with_list_src.dm"),
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
            fixture!("programs/list_method_overrides_can_call_the_intrinsic_super_proc.dm"),
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
            fixture!("programs/alists_inherit_list_intrinsics_through_proc_dispatch.dm"),
            "test",
        ),
        12.into()
    );
}

/// Every object var and list entry on a bake heap holds one of these, a million atoms' worth on a
/// large map.
#[test]
fn a_runtime_value_stays_three_words() {
    assert!(size_of::<GenericValue>() <= 24, "{} bytes", size_of::<GenericValue>());
}

fn root_atom(tree: &ObjectTree, instance: u64, path: &str, x: i32, key: &str) -> Atom {
    Atom {
        instance,
        ty: tree.id_of(&TreePath::parse(path)).expect("fixture type"),
        position: Position::new(x, 1, 1),
        vars: vec![("key".into(), Value::Text(key.into()))],
    }
}

fn profile_at(tree: &ObjectTree, path: &str) -> TypeId {
    tree.id_of(&TreePath::parse(path)).expect("profile type")
}

fn pending<'a>(
    tree: &'a ObjectTree, module: &'a codegen::Module, profile: TypeId, atoms: Vec<Atom>,
) -> PendingBake<'a> {
    let size = [atoms.len() as i32 + 8, 1, 1];

    Bake::initialize_with_profile_and_progress(
        tree,
        module,
        profile,
        atoms,
        size,
        Limits::default(),
        IconStates::default(),
        None,
        None,
        |_, _, _| {},
    )
}

fn module_files<'a>(modules: impl Iterator<Item = (u64, &'a ModuleRoot)>) -> Vec<(u64, Vec<&'a str>)> {
    let mut files = modules
        .map(|(id, root)| (id, root.files.iter().map(String::as_str).collect()))
        .collect::<Vec<_>>();
    files.sort();

    files
}

fn profile_var(bake: &mut Bake, tree: &ObjectTree, module: &codegen::Module, name: &str) -> GenericValue {
    let profile = bake.runtime.profile.expect("profile instance");

    Evaluator::new(&mut bake.runtime, tree, module, Limits::default(), None)
        .read_field(GenericValue::Object(profile), &name.into())
        .expect("profile var")
}

#[test]
fn process_modular_root_runs_once_per_pass_over_the_roots_only() {
    let (tree, module) = compile(fixture!("programs/process_modular_root_runs_once_per_pass.dm"));
    let atoms = vec![
        root_atom(&tree, 1, "/obj/root", 1, "a"),
        root_atom(&tree, 2, "/obj/root/child", 2, "b"),
        root_atom(&tree, 3, "/obj/plain", 3, "c"),
    ];

    let pending = pending(&tree, &module, profile_at(&tree, "/datum/demir/test"), atoms);
    assert_eq!(
        module_files(pending.modules()),
        [(1, vec!["a:2:1"]), (2, vec!["b:2:1"])]
    );
    assert!(pending.module_fault(1).is_none());

    let mut bake = pending.finish_with_progress(|_, _, _| {});
    assert_eq!(bake.diagnostics.count(), 0, "{:?}", bake.diagnostics);
    assert_eq!(bake.modules().count(), 2);
    assert_eq!(profile_var(&mut bake, &tree, &module, "task_calls"), 0.into());
}

#[test]
fn one_batched_call_and_several_calls_place_the_same_roots() {
    let (tree, module) = compile(fixture!("programs/process_modular_root_runs_once_per_pass.dm"));
    let atoms = vec![
        root_atom(&tree, 1, "/obj/root", 1, "a"),
        root_atom(&tree, 2, "/obj/root", 2, "b"),
    ];

    let batched = pending(&tree, &module, profile_at(&tree, "/datum/demir/batched"), atoms);
    assert_eq!(module_files(batched.modules()), [(1, vec!["a:2"]), (2, vec!["b:2"])]);
}

#[test]
fn a_profile_without_a_process_modular_root_places_nothing() {
    let (tree, module) = compile(fixture!("programs/process_modular_root_runs_once_per_pass.dm"));
    let atoms = vec![root_atom(&tree, 1, "/obj/root", 1, "a")];

    let bake =
        pending(&tree, &module, profile_at(&tree, "/datum/demir/no_task"), atoms).finish_with_progress(|_, _, _| {});
    assert_eq!(bake.modules().count(), 0);
    assert_eq!(bake.module_errors().count(), 0);
    assert_eq!(bake.module_faults().count(), 0);
    assert_eq!(bake.diagnostics.count(), 0);
}

#[test]
fn a_process_modular_root_fault_reaches_every_root_of_its_pass() {
    let (tree, module) = compile(fixture!("programs/process_modular_root_runs_once_per_pass.dm"));
    let atoms = vec![
        root_atom(&tree, 1, "/obj/root", 1, "a"),
        root_atom(&tree, 2, "/obj/root", 2, "b"),
    ];

    let mut bake =
        pending(&tree, &module, profile_at(&tree, "/datum/demir/crashing"), atoms).finish_with_progress(|_, _, _| {});
    assert_eq!(bake.modules().count(), 0, "entries of a failed task are dropped");
    for id in [1, 2] {
        assert_eq!(
            bake.module_fault(id).map(|fault| fault.kind.clone()),
            Some(FaultKind::InvalidOperation("task failure".into())),
            "{id}"
        );
    }
    assert_eq!(bake.diagnostics.count(), 1);

    bake.update(&tree, &module, Vec::new(), &[1]);
    assert!(bake.module_fault(1).is_none());
    assert!(bake.module_fault(2).is_some());
    assert_eq!(bake.diagnostics.count(), 1);

    bake.update(&tree, &module, Vec::new(), &[2]);
    assert_eq!(bake.diagnostics.count(), 0);
}

#[test]
fn placement_entries_become_module_roots_and_errors() {
    let (tree, module) = compile(fixture!("programs/placement_entries_become_module_roots.dm"));
    let keys = [
        "none",
        "empty",
        "full",
        "single_skip",
        "error",
        "offset_2d",
        "offset_3d",
        "offset_null",
    ];
    let atoms = keys
        .iter()
        .enumerate()
        .map(|(index, key)| root_atom(&tree, index as u64 + 1, "/obj/root", index as i32 + 1, key))
        .collect();
    let bake = pending(&tree, &module, profile_type(&tree).expect("default profile"), atoms)
        .finish_with_progress(|_, _, _| {});
    let id = |key: &str| keys.iter().position(|current| *current == key).expect("key") as u64 + 1;
    let ty = |path: &str| tree.id_of(&TreePath::parse(path)).expect("type");
    let modules = bake.modules().collect::<HashMap<_, _>>();

    assert!(!modules.contains_key(&id("none")));
    assert!(bake.module_fault(id("none")).is_none());
    assert_eq!(
        modules.get(&id("empty")),
        Some(&&ModuleRoot {
            files: Vec::new(),
            anchor: None,
            skip: Vec::new(),
            offset: [0; 3],
        })
    );
    assert_eq!(
        modules.get(&id("full")),
        Some(&&ModuleRoot {
            files: vec!["a.dmm".into(), "b.dmm".into(), "a.dmm".into()],
            anchor: Some(ty("/obj/connector")),
            skip: vec![ty("/turf/hole"), ty("/area/hole")],
            offset: [0; 3],
        })
    );
    assert_eq!(modules.get(&id("offset_2d")).map(|root| root.offset), Some([-3, 4, 0]));
    assert_eq!(modules.get(&id("offset_3d")).map(|root| root.offset), Some([1, -2, 5]));
    assert_eq!(modules.get(&id("offset_null")).map(|root| root.offset), Some([0; 3]));
    assert_eq!(
        modules.get(&id("single_skip")).map(|root| root.skip.clone()),
        Some(vec![ty("/turf/hole")])
    );

    assert!(!modules.contains_key(&id("error")));
    assert_eq!(
        bake.module_errors().collect::<Vec<_>>(),
        [(id("error"), "no room in config")]
    );
    assert_eq!(bake.module_faults().count(), 0);
    assert_eq!(bake.diagnostics.count(), 0, "{:?}", bake.diagnostics);
}

#[test]
fn invalid_placement_entries_fault_the_task_for_every_root() {
    let (tree, module) = compile(fixture!("programs/placement_entries_are_validated.dm"));
    let profile = profile_type(&tree).expect("default profile");

    for (key, message) in [
        ("foreign", "demir_place_modular() root is not an atom of this pass"),
        (
            "no_root",
            "demir_place_modular() entry needs a \"root\" atom or an \"at\" coordinate",
        ),
        (
            "duplicate_in_call",
            "demir_place_modular() root already has an entry in this pass",
        ),
        (
            "duplicate_across_calls",
            "demir_place_modular() root already has an entry in this pass",
        ),
        (
            "bad_entry",
            "each demir_place_modular() entry must be an associative list",
        ),
        ("bad_maps", "demir_place_modular() maps must be a list of text"),
        ("bad_map_entry", "each demir_place_modular() map must be text"),
        ("bad_anchor", "demir_place_modular() anchor must be a type"),
        (
            "undefined_anchor",
            "demir_place_modular() anchor must be a defined type",
        ),
        (
            "bad_skip",
            "demir_place_modular() skip must be a type or a list of types",
        ),
        (
            "bad_offset_text",
            "demir_place_modular() offset must be list(dx, dy) or list(dx, dy, dz)",
        ),
        (
            "bad_offset_len",
            "demir_place_modular() offset must be list(dx, dy) or list(dx, dy, dz)",
        ),
        (
            "bad_offset_entry",
            "demir_place_modular() offset must be list(dx, dy) or list(dx, dy, dz)",
        ),
        ("bad_error", "demir_place_modular() error must be text"),
        (
            "unknown_kind",
            "demir_place_modular() entry needs \"maps\" or \"error\"",
        ),
    ] {
        let atoms = vec![
            root_atom(&tree, 1, "/obj/root", 1, key),
            root_atom(&tree, 2, "/obj/root", 2, key),
            root_atom(&tree, 3, "/obj/plain", 3, key),
        ];
        let bake = pending(&tree, &module, profile, atoms).finish_with_progress(|_, _, _| {});

        assert_eq!(bake.modules().count(), 0, "{key}");
        assert_eq!(bake.module_errors().count(), 0, "{key}");
        assert_eq!(bake.diagnostics.count(), 1, "{key}: {:?}", bake.diagnostics);
        for id in [1, 2] {
            assert_eq!(
                bake.module_fault(id).map(|fault| fault.kind.clone()),
                Some(FaultKind::InvalidOperation(message.into())),
                "{key} {id}"
            );
        }
    }
}

fn place_from_hook(profile: &str) -> (ObjectTree, codegen::Module, TypeId) {
    let (tree, module) = compile(fixture!("programs/demir_place_from_any_hook.dm"));
    let profile = profile_at(&tree, profile);

    (tree, module, profile)
}

#[test]
fn demir_place_from_new_is_kept_before_the_first_pass() {
    let (tree, module, profile) = place_from_hook("/datum/demir/from_new");
    let atoms = vec![root_atom(&tree, 1, "/obj/thing", 1, "a")];

    let pending = pending(&tree, &module, profile, atoms);
    assert_eq!(module_files(pending.modules()), [(1, vec!["new.dmm"])]);

    let bake = pending.finish_with_progress(|_, _, _| {});
    assert_eq!(module_files(bake.modules()), [(1, vec!["new.dmm"])]);
    assert_eq!(bake.diagnostics.count(), 0, "{:?}", bake.diagnostics);
}

#[test]
fn demir_place_from_prepare_is_kept_after_the_bake() {
    let (tree, module, profile) = place_from_hook("/datum/demir/from_prepare");
    let atoms = vec![root_atom(&tree, 1, "/obj/thing", 1, "a")];

    let pending = pending(&tree, &module, profile, atoms);
    assert_eq!(pending.modules().count(), 0);

    let mut bake = pending.finish_with_progress(|_, _, _| {});
    assert_eq!(module_files(bake.modules()), [(1, vec!["prepare.dmm"])]);

    bake.update(&tree, &module, vec![root_atom(&tree, 2, "/obj/thing", 2, "b")], &[1]);
    assert_eq!(module_files(bake.modules()), [(2, vec!["prepare.dmm"])]);
    assert_eq!(bake.diagnostics.count(), 0, "{:?}", bake.diagnostics);
}

#[test]
fn demir_place_from_bake_and_highlights_is_discarded() {
    for name in ["from_bake", "from_highlights"] {
        let (tree, module, profile) = place_from_hook(&format!("/datum/demir/{name}"));
        let atoms = vec![root_atom(&tree, 1, "/obj/thing", 1, "a")];

        let bake = pending(&tree, &module, profile, atoms).finish_with_progress(|_, _, _| {});
        assert_eq!(bake.modules().count(), 0, "{name}");
        assert_eq!(bake.module_errors().count(), 0, "{name}");
        assert_eq!(bake.diagnostics.count(), 0, "{name}: {:?}", bake.diagnostics);
    }
}

#[test]
fn a_later_demir_place_replaces_an_earlier_one_for_the_same_atom() {
    let (tree, module, profile) = place_from_hook("/datum/demir/replacing");
    let atoms = vec![root_atom(&tree, 1, "/obj/thing", 1, "a")];

    let pending = pending(&tree, &module, profile, atoms);
    assert_eq!(pending.modules().count(), 0, "the error entry replaced the maps");
    assert_eq!(pending.module_errors().collect::<Vec<_>>(), [(1, "second")]);

    let bake = pending.finish_with_progress(|_, _, _| {});
    assert_eq!(module_files(bake.modules()), [(1, vec!["third.dmm"])]);
    assert_eq!(bake.module_errors().count(), 0);
}

#[test]
fn demir_place_on_an_atom_that_is_not_placed_faults() {
    let (tree, module, profile) = place_from_hook("/datum/demir/unplaced");
    let atoms = vec![root_atom(&tree, 1, "/obj/thing", 1, "a")];

    let bake = pending(&tree, &module, profile, atoms).finish_with_progress(|_, _, _| {});
    assert_eq!(bake.modules().count(), 0);
    assert_eq!(bake.diagnostics.count(), 1, "{:?}", bake.diagnostics);
    assert_eq!(
        bake.diagnostics.entries[0].fault.kind,
        FaultKind::InvalidOperation("demir_place_modular() root must be an atom placed on the map".into())
    );
}

#[test]
fn demir_place_from_ui_is_kept_only_on_a_committed_frame() {
    let (tree, module, profile) = place_from_hook("/datum/demir/from_ui");
    let feedback = |is_interaction: bool| Feedback {
        clicks: HashSet::from([String::from("Panel/Place")]),
        interacted: is_interaction,
        ..Default::default()
    };
    let atoms = || vec![root_atom(&tree, 1, "/obj/thing", 1, "a")];

    let mut bake = pending(&tree, &module, profile, atoms()).finish_with_progress(|_, _, _| {});
    let frame = bake
        .ui(&tree, &module, Some(1), 0, feedback(false))
        .expect("idle frame");
    assert!(!frame.committed);
    assert_eq!(bake.modules().count(), 0);

    let frame = bake
        .ui(&tree, &module, Some(1), 0, feedback(true))
        .expect("committed frame");
    assert!(frame.committed);
    assert_eq!(module_files(bake.modules()), [(1, vec!["ui.dmm"])]);

    let mut bake = pending(&tree, &module, profile, atoms()).finish_with_progress(|_, _, _| {});
    bake.ui(&tree, &module, Some(1), 0, feedback(true)).expect("frame");
    bake.update(&tree, &module, Vec::new(), &[1]);
    assert_eq!(bake.modules().count(), 0, "removing the atom drops its entry");
}

fn place_clicked() -> Feedback {
    Feedback {
        clicks: HashSet::from([String::from("Panel/Place")]),
        interacted: true,
        ..Default::default()
    }
}

#[test]
fn a_ui_placement_survives_its_atom_being_reinserted_but_not_removed() {
    let (tree, module, profile) = place_from_hook("/datum/demir/from_ui");
    let atoms = vec![root_atom(&tree, 1, "/obj/thing", 1, "a")];

    let mut bake = pending(&tree, &module, profile, atoms).finish_with_progress(|_, _, _| {});
    bake.ui(&tree, &module, Some(1), 0, place_clicked()).expect("frame");
    bake.update(&tree, &module, vec![root_atom(&tree, 1, "/obj/thing", 1, "b")], &[]);
    assert_eq!(module_files(bake.modules()), [(1, vec!["ui.dmm"])]);

    bake.update(&tree, &module, Vec::new(), &[1]);
    assert_eq!(bake.modules().count(), 0);
}

#[test]
fn a_new_placement_survives_its_atom_being_reinserted_but_not_removed() {
    let (tree, module, profile) = place_from_hook("/datum/demir/from_new");
    let atoms = vec![root_atom(&tree, 1, "/obj/thing", 1, "a")];

    let mut bake = pending(&tree, &module, profile, atoms).finish_with_progress(|_, _, _| {});
    bake.update(&tree, &module, vec![root_atom(&tree, 1, "/obj/thing", 1, "b")], &[]);
    assert_eq!(module_files(bake.modules()), [(1, vec!["new.dmm"])]);

    bake.update(&tree, &module, Vec::new(), &[1]);
    assert_eq!(bake.modules().count(), 0);
}

#[test]
fn a_prepare_placement_is_decided_again_when_its_atom_is_reinserted() {
    let (tree, module, profile) = place_from_hook("/datum/demir/conditional_prepare");
    let atoms = vec![root_atom(&tree, 1, "/obj/thing", 1, "place")];

    let mut bake = pending(&tree, &module, profile, atoms).finish_with_progress(|_, _, _| {});
    assert_eq!(module_files(bake.modules()), [(1, vec!["prepare.dmm"])]);

    bake.update(&tree, &module, vec![root_atom(&tree, 1, "/obj/thing", 1, "skip")], &[]);
    assert_eq!(bake.modules().count(), 0);

    bake.update(&tree, &module, vec![root_atom(&tree, 1, "/obj/thing", 1, "place")], &[]);
    assert_eq!(module_files(bake.modules()), [(1, vec!["prepare.dmm"])]);
}

#[test]
fn a_ui_placement_on_a_modular_root_survives_a_pass_that_places_nothing() {
    let (tree, module, profile) = place_from_hook("/datum/demir/ui_on_a_root");
    let atoms = vec![root_atom(&tree, 1, "/obj/thing", 1, "skip")];

    let mut bake = pending(&tree, &module, profile, atoms).finish_with_progress(|_, _, _| {});
    assert_eq!(bake.modules().count(), 0);

    bake.ui(&tree, &module, Some(1), 0, place_clicked()).expect("frame");
    assert_eq!(module_files(bake.modules()), [(1, vec!["ui.dmm"])]);

    bake.update(&tree, &module, vec![root_atom(&tree, 1, "/obj/thing", 1, "skip2")], &[]);
    assert_eq!(module_files(bake.modules()), [(1, vec!["ui.dmm"])]);

    bake.update(&tree, &module, vec![root_atom(&tree, 1, "/obj/thing", 1, "pass")], &[]);
    assert_eq!(module_files(bake.modules()), [(1, vec!["pass.dmm"])]);

    bake.update(&tree, &module, vec![root_atom(&tree, 1, "/obj/thing", 1, "skip")], &[]);
    assert_eq!(bake.modules().count(), 0, "the pass-owned entry is cleared again");
}

fn coordinate_entries<'a>(
    modules: impl Iterator<Item = (Position, Result<&'a ModuleRoot, &'a str>)>,
) -> Vec<(Position, Result<&'a ModuleRoot, &'a str>)> {
    modules.collect()
}

#[test]
fn at_entries_from_new_are_kept_per_coordinate_and_outlive_atoms() {
    let (tree, module) = compile(fixture!("programs/placement_at_entries_come_from_new.dm"));
    let profile = profile_type(&tree).expect("default profile");
    let ty = |path: &str| tree.id_of(&TreePath::parse(path)).expect("type");
    let expected = [
        (Position::new(3, 2, 1), Err("no room")),
        (
            Position::new(5, 2, 1),
            Ok(ModuleRoot {
                files: vec!["a.dmm".into()],
                anchor: None,
                skip: Vec::new(),
                offset: [0; 3],
            }),
        ),
        (
            Position::new(9, 1, 2),
            Ok(ModuleRoot {
                files: vec!["b.dmm".into(), "c.dmm".into()],
                anchor: Some(ty("/obj/connector")),
                skip: vec![ty("/turf/hole")],
                offset: [1, -2, 3],
            }),
        ),
        (
            Position::new(40, 40, 40),
            Ok(ModuleRoot {
                files: Vec::new(),
                anchor: None,
                skip: Vec::new(),
                offset: [0; 3],
            }),
        ),
    ];
    let check = |entries: Vec<(Position, Result<&ModuleRoot, &str>)>| {
        let entries = entries
            .into_iter()
            .map(|(position, module)| (position, module.cloned().map_err(String::from)))
            .collect::<Vec<_>>();
        let expected = expected
            .iter()
            .map(|(position, module)| (*position, module.clone().map_err(String::from)))
            .collect::<Vec<_>>();

        assert_eq!(entries, expected);
    };
    let atoms = vec![root_atom(&tree, 1, "/obj/root", 1, "a")];

    let pending = pending(&tree, &module, profile, atoms);
    check(coordinate_entries(pending.coordinate_modules()));

    let mut bake = pending.finish_with_progress(|_, _, _| {});
    check(coordinate_entries(bake.coordinate_modules()));
    assert_eq!(module_files(bake.modules()), [(1, vec!["root.dmm"])]);

    bake.update(&tree, &module, vec![root_atom(&tree, 2, "/obj/root", 2, "b")], &[1]);
    check(coordinate_entries(bake.coordinate_modules()));

    bake.update(&tree, &module, Vec::new(), &[2]);
    assert_eq!(bake.modules().count(), 0);
    check(coordinate_entries(bake.coordinate_modules()));
    assert_eq!(bake.diagnostics.count(), 0, "{:?}", bake.diagnostics);
}

#[test]
fn invalid_at_entries_fault_new() {
    let (tree, module) = compile(fixture!("programs/placement_at_entries_are_validated.dm"));
    let bad_at = "demir_place_modular() at must be list(x, y, z)";

    for (profile, message) in [
        (
            "neither",
            "demir_place_modular() entry needs a \"root\" atom or an \"at\" coordinate",
        ),
        ("both", "demir_place_modular() entry takes \"root\" or \"at\", not both"),
        ("two_numbers", bad_at),
        ("not_a_list", bad_at),
        ("not_a_number", bad_at),
        ("zero", bad_at),
        ("fractional", bad_at),
        ("duplicate_in_call", "demir_place_modular() at already has an entry"),
        (
            "duplicate_across_calls",
            "demir_place_modular() at already has an entry",
        ),
    ] {
        let profile_type = profile_at(&tree, &format!("/datum/demir/{profile}"));
        let pending = pending(&tree, &module, profile_type, Vec::new());
        let bake = pending.finish_with_progress(|_, _, _| {});

        assert_eq!(bake.diagnostics.count(), 1, "{profile}: {:?}", bake.diagnostics);
        assert_eq!(
            bake.diagnostics.entries[0].fault.kind,
            FaultKind::InvalidOperation(message.into()),
            "{profile}"
        );
        assert_eq!(bake.coordinate_modules().count(), 0, "{profile}");
    }
}

#[test]
fn at_entries_outside_new_are_blocked() {
    let (tree, module) = compile(fixture!("programs/placement_at_outside_new_is_blocked.dm"));
    let is_blocked = |fault: &Fault| {
        matches!(
            &fault.kind,
            FaultKind::Blocked(message) if message == "demir_place_modular() at outside a profile's New()"
        )
    };
    let atoms = || vec![root_atom(&tree, 1, "/obj/root", 1, "a")];

    for profile in ["from_pass", "from_prepare"] {
        let profile = profile_at(&tree, &format!("/datum/demir/{profile}"));
        let bake = pending(&tree, &module, profile, atoms()).finish_with_progress(|_, _, _| {});

        assert_eq!(bake.diagnostics.count(), 1, "{:?}", bake.diagnostics);
        assert!(is_blocked(&bake.diagnostics.entries[0].fault), "{:?}", bake.diagnostics);
        assert_eq!(bake.coordinate_modules().count(), 0);
    }

    let profile = profile_at(&tree, "/datum/demir/from_ui");
    let mut bake = pending(&tree, &module, profile, atoms()).finish_with_progress(|_, _, _| {});
    let feedback = Feedback {
        clicks: HashSet::from([String::from("Panel/Place")]),
        interacted: true,
        ..Default::default()
    };
    let fault = bake.ui(&tree, &module, Some(1), 0, feedback).expect_err("blocked");

    assert!(is_blocked(&fault), "{fault:?}");
    assert_eq!(bake.coordinate_modules().count(), 0);
}

#[test]
fn an_uninitialized_bake_has_no_coordinate_modules() {
    let (tree, module) = compile(fixture!("programs/placement_at_entries_vanish_when_new_faults.dm"));
    let profile = profile_type(&tree).expect("default profile");
    let pending = pending(&tree, &module, profile, Vec::new());

    assert_eq!(pending.coordinate_modules().count(), 0);

    let bake = pending.finish_with_progress(|_, _, _| {});
    assert_eq!(bake.diagnostics.count(), 1, "{:?}", bake.diagnostics);
    assert_eq!(bake.coordinate_modules().count(), 0);
}

#[test]
fn demir_modular_root_outside_new_faults() {
    let (tree, module) = compile(fixture!("programs/demir_modular_root_outside_new_faults.dm"));
    let atoms = vec![root_atom(&tree, 1, "/obj/root", 1, "a")];
    let bake = pending(&tree, &module, profile_type(&tree).expect("default profile"), atoms)
        .finish_with_progress(|_, _, _| {});

    assert_eq!(bake.diagnostics.count(), 1, "{:?}", bake.diagnostics);
    assert!(matches!(
        bake.diagnostics.entries[0].fault.kind,
        FaultKind::Blocked(ref message) if message.contains("outside a profile's New()")
    ));
}

#[test]
fn a_pending_bake_runs_a_second_module_pass_for_inserted_roots_only() {
    let (tree, module) = compile(fixture!("programs/process_modular_root_runs_once_per_pass.dm"));
    let atoms = vec![
        root_atom(&tree, 1, "/obj/root", 1, "a"),
        root_atom(&tree, 2, "/obj/root", 2, "b"),
    ];
    let mut pending = pending(&tree, &module, profile_at(&tree, "/datum/demir/test"), atoms);

    pending.replace_atoms(
        vec![
            root_atom(&tree, 4, "/obj/root", 4, "c"),
            root_atom(&tree, 5, "/obj/plain", 5, "p"),
        ],
        &[],
        |_, _, _| {},
    );
    assert_eq!(
        module_files(pending.modules()),
        [(1, vec!["a:2:1"]), (2, vec!["b:2:1"]), (4, vec!["c:1:1"])]
    );

    pending.replace_atoms(Vec::new(), &[1], |_, _, _| {});
    assert_eq!(
        module_files(pending.modules()),
        [(2, vec!["b:2:1"]), (4, vec!["c:1:1"])]
    );

    pending.replace_atoms(vec![root_atom(&tree, 4, "/obj/root", 4, "d")], &[], |_, _, _| {});
    assert_eq!(
        module_files(pending.modules()),
        [(2, vec!["b:2:1"]), (4, vec!["d:1:1"])]
    );
}

#[test]
fn update_runs_module_passes_for_inserted_roots_and_drops_removed_ones() {
    let (tree, module) = compile(fixture!("programs/process_modular_root_runs_once_per_pass.dm"));
    let atoms = vec![
        root_atom(&tree, 1, "/obj/root", 1, "a"),
        root_atom(&tree, 2, "/obj/root", 2, "b"),
    ];
    let mut bake =
        pending(&tree, &module, profile_at(&tree, "/datum/demir/test"), atoms).finish_with_progress(|_, _, _| {});

    bake.update(
        &tree,
        &module,
        vec![
            root_atom(&tree, 3, "/obj/root", 3, "c"),
            root_atom(&tree, 4, "/obj/plain", 4, "p"),
        ],
        &[2],
    );
    assert_eq!(module_files(bake.modules()), [(1, vec!["a:2:1"]), (3, vec!["c:1:1"])]);
    assert_eq!(bake.diagnostics.count(), 0, "{:?}", bake.diagnostics);
}

#[test]
fn demir_toml_decode_converts_to_dm_values() {
    let source = fixture!("programs/demir_toml_decode_converts_to_dm_values.dm");

    assert_eq!(run(source, "scalars").display(), "x|1|0|3|0.5",);
    assert_eq!(run(source, "nested").display(), "3|2|b.dmm");

    let (tree, module) = compile(source);
    let fault = Runtime::default()
        .run(
            &tree,
            &module,
            proc(&tree, &module.index, "invalid"),
            None,
            None,
            Vec::new(),
            Limits::default(),
        )
        .expect_err("uncaught invalid TOML should fault");
    assert!(
        matches!(&fault.kind, FaultKind::Thrown(message) if !message.is_empty()),
        "{:?}",
        fault.kind
    );
    assert_eq!(run(source, "caught").display(), "caught: 1|1");
}

#[test]
fn demir_map_path_is_the_value_the_bake_was_built_with() {
    let (tree, module) = compile(fixture!("programs/demir_map_path_reaches_every_hook.dm"));
    let baked = |map_path: Option<&str>| {
        let atoms = vec![Atom {
            instance: 1,
            ty: tree.id_of(&TreePath::parse("/turf/wall")).expect("wall type"),
            position: Position::new(1, 1, 1),
            vars: Vec::new(),
        }];
        let bake = Bake::with_profile_and_progress(
            &tree,
            &module,
            profile_type(&tree).expect("profile type"),
            atoms,
            [1, 1, 1],
            Limits::default(),
            IconStates::default(),
            None,
            map_path.map(str::to_owned),
            |_, _, _| {},
        );

        baked_state(&bake, 1).map(str::to_owned)
    };

    let path = "_maps/map_files/Test/Test.dmm";
    assert_eq!(baked(Some(path)), Some(format!("{path}|{path}")));
    assert_eq!(baked(None), Some("null|null".to_owned()));
}

#[test]
fn file_reads_cannot_leave_the_codebase_root() {
    let (tree, module) = compile(fixture!("programs/filesystem_reads_stay_inside_the_root.dm"));
    let root = std::env::temp_dir().join(format!("rmd-sandbox-{}", std::process::id()));
    std::fs::create_dir_all(root.join("sub")).expect("temp dir");
    std::fs::write(root.join("tips.txt"), "inside").expect("fixture file");
    std::fs::write(root.join("sub/inner.txt"), "inner").expect("fixture file");

    let mut runtime = Runtime::default();
    runtime.world.root = Some(root.clone());
    let mut call = |name: &str| {
        runtime.run(
            &tree,
            &module,
            proc(&tree, &module.index, name),
            None,
            None,
            Vec::new(),
            Limits::default(),
        )
    };

    assert_eq!(call("inside").expect("inside").display(), "inside");
    assert_eq!(call("dotted").expect("dotted").display(), "inside");
    assert_eq!(call("backslash").expect("backslash").display(), "inner");
    for name in ["escaping", "deep_escaping", "absolute"] {
        let fault = call(name).expect_err(name);
        assert_eq!(fault.kind, FaultKind::Blocked("filesystem".into()), "{name}");
    }

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn modules_are_inserted_before_atom_hooks() {
    let (tree, module) = compile(fixture!("programs/modules_are_inserted_before_atom_hooks.dm"));
    let atom = |instance, path, x| Atom {
        instance,
        ty: tree.id_of(&TreePath::parse(path)).unwrap(),
        position: Position::new(x, 1, 1),
        vars: Vec::new(),
    };
    let source = vec![
        atom(1, "/turf/staged_floor", 1),
        atom(2, "/turf/staged_floor", 2),
        atom(3, "/obj/staged_root", 2),
        atom(4, "/obj/staged_kept", 1),
        atom(5, "/area/staged_source", 1),
        atom(6, "/area/staged_source", 2),
    ];
    let mut progress = Vec::new();
    let mut pending = Bake::initialize_with_profile_and_progress(
        &tree,
        &module,
        profile_type(&tree).unwrap(),
        source,
        [2, 1, 1],
        Limits::default(),
        IconStates::default(),
        None,
        None,
        |stage, done, total| progress.push((stage, done, total)),
    );
    assert_eq!(module_files(pending.modules()), [(3, vec!["staged.dmm"])]);
    pending.replace_atoms(
        vec![
            atom(7, "/turf/staged_wall", 2),
            atom(8, "/obj/staged_module", 2),
            atom(9, "/area/staged_module", 2),
        ],
        &[2, 3, 6],
        |stage, done, total| progress.push((stage, done, total)),
    );
    assert!(
        progress
            .iter()
            .all(|(stage, ..)| matches!(stage, Stage::Instantiate | Stage::Initialize))
    );
    assert_eq!(pending.modules().count(), 0);
    let mut bake = pending.finish_with_progress(|stage, done, total| progress.push((stage, done, total)));
    assert_eq!(bake.diagnostics.count(), 0, "{:?}", bake.diagnostics);
    assert_eq!(bake.modules().count(), 0);
    assert_eq!(
        bake.take_output(),
        [
            "constructor",
            "prepare:/turf/staged_floor",
            "prepare:/obj/staged_kept",
            "prepare:/area/staged_source",
            "prepare:/turf/staged_wall",
            "prepare:/obj/staged_module",
            "prepare:/area/staged_module",
        ]
    );
    assert_eq!(baked_state(&bake, 4), Some("1-1-constructor:prepared-1"));
    for id in [2, 3, 6] {
        assert!(bake.object(id).is_none());
        assert!(!bake.appearances.contains_key(&id));
    }
    assert_eq!(bake.runtime.world.turf_at(Position::new(2, 1, 1)), bake.object(7));
    assert_eq!(
        bake.runtime.world.area_of(&bake.runtime.heap, bake.object(8).unwrap()),
        bake.object(9)
    );
    let profile = bake.runtime.heap.object(bake.runtime.profile.unwrap()).unwrap();
    assert_eq!(
        profile.vars.get(&"original_turf".into()),
        Some(&GenericValue::Object(bake.object(1).unwrap()))
    );
    for stage in [Stage::Initialize, Stage::Prepare, Stage::Light, Stage::Smooth] {
        assert_eq!(
            progress
                .iter()
                .filter(|(current, done, _)| *current == stage && *done == 0)
                .count(),
            1,
            "{stage:?}"
        );
    }
    assert_eq!(progress.last(), Some(&(Stage::Smooth, 6, 6)));
}
