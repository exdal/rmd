use core::{
    arena::StrArena,
    path::TreePath,
    types::{Identifier, Value},
};
use std::{
    collections::{HashMap, HashSet},
    sync::atomic::{AtomicUsize, Ordering},
};

use objtree::{ObjectTree, TypeId};

use crate::{
    FaultKind,
    GenericValue,
    IconStates,
    Limits,
    Runtime,
    bake::{
        Atom,
        Bake,
        BakeUpdate,
        HIGHLIGHT_ALWAYS,
        HIGHLIGHT_EDGE_NORTH,
        HIGHLIGHT_EDGE_SOUTH,
        HIGHLIGHT_EDGE_WEST,
        HIGHLIGHT_SELECTED,
        HighlightTile,
        ProfileError,
        has_profile,
        profile_catalog,
        profile_type,
        selected_profile_type,
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
    let (tree, module, errors) = sema::analyze(&ast);
    assert!(errors.is_empty(), "{errors:?}");

    (tree, module)
}

fn compile(source: &str) -> (ObjectTree, codegen::Module) {
    let (tree, module) = analyze_fixture(source);
    let module = codegen::generate(&module).expect("fixture should compile");
    (tree, module)
}

fn proc(tree: &ObjectTree, name: &str) -> core::types::ProcId {
    tree.proc_inherited(TypeId::ROOT, &name.into())
        .and_then(|proc| proc.body)
        .expect("fixture proc should exist")
}

fn hook(tree: &ObjectTree, hook: ProfileHook) -> core::types::ProcId {
    let profile = profile_type(tree).expect("fixture should declare one profile");
    ProfileDefinition::resolve(tree, profile)
        .procedure(hook)
        .expect("fixture hook should exist")
}

fn run(source: &str, name: &str) -> GenericValue {
    let (tree, module) = compile(source);
    Runtime::default()
        .run(
            &tree,
            &module,
            proc(&tree, name),
            None,
            None,
            Vec::new(),
            Limits::default(),
        )
        .expect("fixture should execute")
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
    let (tree, _) = analyze_fixture(fixture!(
        "programs/profile_definition_resolves_the_complete_hook_abi.dm"
    ));
    let profile = profile_type(&tree).expect("default profile");
    let definition = ProfileDefinition::resolve(&tree, profile);
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
            hook(&tree, ProfileHook::Bake),
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
            hook(&tree, ProfileHook::Bake),
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
    let bad = tree.id_of(&TreePath::parse("/obj/bad")).unwrap();

    assert_eq!(bake.node_groups().len(), 3);
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
    assert_eq!(
        bake.node_groups()
            .iter()
            .find(|group| group.subtype == bad)
            .unwrap()
            .blockers,
        vec![grille],
        "an invalid blocker list does not partially update an existing group"
    );
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
            hook(&tree, ProfileHook::Bake),
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
            proc(&tree, "test"),
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
            proc(&tree, "test"),
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

    assert_eq!(result, GenericValue::from("1 1 1"));
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
    )]);
    let result = runtime
        .run(
            &tree,
            &module,
            proc(&tree, "test"),
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
            proc(&tree, "test"),
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
            proc(&tree, "test"),
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
    let entry = proc(&tree, "entry");
    let full = codegen::generate(&ir_module).expect("full codegen");
    let selected = codegen::generate_reachable(&ir_module, &tree, &[entry]).expect("reachable codegen");

    let execute = |module: &codegen::Module| {
        let mut runtime = Runtime::default();
        let value = runtime
            .run(&tree, module, entry, None, None, Vec::new(), Limits::default())
            .expect("fixture should execute");
        (value, runtime.take_output())
    };

    assert_eq!(execute(&selected), execute(&full));
    let unreachable = proc(&tree, "unreachable");
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
            proc(&tree, "test"),
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
            proc(&tree, "test"),
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
        .call(proc(&tree, "test"), None, Vec::new())
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
            proc(&tree, "lines"),
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
                proc(&tree, "banned"),
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
            proc(&tree, "reboot"),
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
            proc(&tree, "lines"),
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

/// `matrix(M, ...)` is the in-place form every `/matrix` method in `stddef.dm` routes through.
/// Transforms are unimplemented, so it must hand the matrix back rather than write the argument
/// list onto `a`/`b`/`c`.
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
