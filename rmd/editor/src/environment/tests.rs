use std::sync::atomic::{AtomicUsize, Ordering};

use base::{
    location::Location,
    types::{ListEntry, VarModifiers},
};
use objtree::VarDecl;

use super::*;

fn fixture(source: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!("rmd-profile-selection-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create fixture directory");
    let entry = root.join("test.dm");
    std::fs::write(&entry, source).expect("write fixture");

    entry
}

#[test]
fn compilation_exposes_profiles_and_honors_valid_or_stale_requests() {
    let entry = fixture(
        r#"
#ifdef __DEMIR_BAKE__
/datum/demir/main
    default = TRUE
    bake(atom/target)
        target.name = "main"
/datum/demir/main/debug/bake(atom/target)
    target.name = "debug"
#endif
"#,
    );

    let load = |requested: Option<&str>| {
        Environment::load_with(
            &entry,
            BakeOptions {
                profile: requested.map(TreePath::parse),
                ..BakeOptions::default()
            },
            &Progress::new(),
        )
        .expect("load fixture")
    };
    let (default, diagnostics) = load(None);
    assert!(diagnostics.profile.is_none());
    assert_eq!(default.optimization_timings.samples(), 1);
    assert_eq!(
        default.profiles,
        Some(Profiles {
            available: vec![
                String::from("/datum/demir/main"),
                String::from("/datum/demir/main/debug"),
            ],
            default: String::from("/datum/demir/main"),
            active: String::from("/datum/demir/main"),
        })
    );
    let default_program = default.bake_program.as_ref().expect("default profile bytecode");
    let main = default_program
        .tree
        .id_of(&TreePath::parse("/datum/demir/main"))
        .expect("main profile");
    let main_bake = vm::profile::ProfileDefinition::resolve(&default_program.tree, &default_program.module.index, main)
        .procedure(vm::profile::ProfileHook::Bake)
        .expect("main bake body");
    let debug = default_program
        .tree
        .id_of(&TreePath::parse("/datum/demir/main/debug"))
        .expect("debug profile");
    let debug_bake =
        vm::profile::ProfileDefinition::resolve(&default_program.tree, &default_program.module.index, debug)
            .procedure(vm::profile::ProfileHook::Bake)
            .expect("debug bake body");
    assert!(default_program.module.function_for_proc(main_bake).is_some());
    assert!(default_program.module.function_for_proc(debug_bake).is_none());

    let (debug, diagnostics) = load(Some("/datum/demir/main/debug"));
    assert!(diagnostics.profile.is_none());
    assert_eq!(
        debug.profiles.as_ref().map(|profiles| profiles.active.as_str()),
        Some("/datum/demir/main/debug")
    );
    assert_eq!(
        debug
            .bake_program
            .as_ref()
            .and_then(|program| program.tree.get(program.profile))
            .map(|declaration| declaration.path.to_string()),
        Some(String::from("/datum/demir/main/debug"))
    );
    let debug_program = debug.bake_program.as_ref().expect("debug profile bytecode");
    let main = debug_program
        .tree
        .id_of(&TreePath::parse("/datum/demir/main"))
        .expect("main profile");
    let main_bake = vm::profile::ProfileDefinition::resolve(&debug_program.tree, &debug_program.module.index, main)
        .procedure(vm::profile::ProfileHook::Bake)
        .expect("main bake body");
    let debug = debug_program
        .tree
        .id_of(&TreePath::parse("/datum/demir/main/debug"))
        .expect("debug profile");
    let debug_bake = vm::profile::ProfileDefinition::resolve(&debug_program.tree, &debug_program.module.index, debug)
        .procedure(vm::profile::ProfileHook::Bake)
        .expect("debug bake body");
    assert!(debug_program.module.function_for_proc(main_bake).is_none());
    assert!(debug_program.module.function_for_proc(debug_bake).is_some());

    let (stale, diagnostics) = load(Some("/datum/demir/main/missing"));
    assert!(diagnostics.profile.is_none());
    assert_eq!(
        stale.profiles.as_ref().map(|profiles| profiles.active.as_str()),
        Some("/datum/demir/main")
    );

    std::fs::remove_dir_all(entry.parent().expect("fixture parent")).expect("remove fixture");
}

#[test]
fn invalid_default_declarations_are_diagnostics_and_disable_baking() {
    let entry = fixture(
        r#"
#ifdef __DEMIR_BAKE__
/datum/demir/one
/datum/demir/two
#endif
"#,
    );
    let (environment, diagnostics) = Environment::load_with(&entry, BakeOptions::default(), &Progress::new())
        .expect("the compatibility view still loads");

    assert!(matches!(
        diagnostics.profile,
        Some(vm::bake::ProfileError::MissingDefault(_))
    ));
    assert!(environment.profiles.is_none());
    assert!(environment.bake_program.is_none());

    std::fs::remove_dir_all(entry.parent().expect("fixture parent")).expect("remove fixture");
}

#[test]
fn forcing_a_bundled_profile_overrides_the_codebase_default() {
    let entry = fixture(
        r#"
#ifdef __DEMIR_BAKE__
/datum/demir/native
    default = TRUE
#endif
"#,
    );
    let options = BakeOptions {
        forced_profile: Some(String::from("cmss13")),
        ..BakeOptions::default()
    };
    let (environment, diagnostics) = Environment::load_with(&entry, options, &Progress::new())
        .expect("the bundled profile should be injected after the codebase");

    assert!(diagnostics.profile.is_none());
    assert_eq!(
        environment.profiles,
        Some(Profiles {
            available: vec![String::from("/datum/demir/cmss13")],
            default: String::from("/datum/demir/cmss13"),
            active: String::from("/datum/demir/cmss13"),
        })
    );
    assert_eq!(environment.bake_options.forced_profile.as_deref(), Some("cmss13"));

    std::fs::remove_dir_all(entry.parent().expect("fixture parent")).expect("remove fixture");
}

#[test]
fn bundled_profile_metadata_covers_every_embedded_example() {
    assert_eq!(
        BundledProfile::ALL.map(BundledProfile::stem),
        [
            "beestation",
            "cmss13",
            "goonstation",
            "monkestation",
            "monkespaw",
            "novasector",
            "oculis",
            "secondcity",
            "tgmc",
            "tgstation",
            "vanderlin",
        ]
    );
    for profile in BundledProfile::ALL {
        assert_eq!(BundledProfile::from_name(profile.stem()), Some(profile));
        assert!(
            profile
                .source()
                .contains(&format!("/datum/demir/{}\n\tdefault = TRUE", profile.stem()))
        );
    }
    assert_eq!(
        BundledProfile::Goonstation.defines().map(|(_, source)| source),
        Some("#define USE_PERSPECTIVE_EDITOR_WALLS\n")
    );
    assert_eq!(
        BundledProfile::Tgstation.defines().map(|(_, source)| source),
        Some("#ifdef __DEMIR_BAKE__\n#define CBT\n#endif\n")
    );
    assert_eq!(
        BundledProfile::SecondCity.defines().map(|(_, source)| source),
        Some("#ifdef __DEMIR_BAKE__\n#define CBT\n#endif\n")
    );
    assert_eq!(
        BundledProfile::Monkestation.defines().map(|(_, source)| source),
        Some("#ifdef __DEMIR_BAKE__\n#define CBT\n#endif\n")
    );
    for profile in [
        BundledProfile::Beestation,
        BundledProfile::Monkespaw,
        BundledProfile::Novasector,
        BundledProfile::Oculis,
    ] {
        assert_eq!(
            profile.defines().map(|(_, source)| source),
            Some("#ifdef __DEMIR_BAKE__\n#define CBT\n#endif\n")
        );
    }
    assert_eq!(profile_label("beestation"), "BeeStation");
    assert_eq!(profile_label("novasector"), "Nova Sector");
    assert_eq!(profile_label("oculis"), "Oculis Station");
    assert_eq!(profile_label("monkespaw"), "Monke’s Paw");
    assert_eq!(profile_label("tgmc"), "TGMC");
    assert_eq!(profile_label("secondcity"), "SecondCity");
    assert_eq!(profile_label("mine"), "mine");
}

#[test]
fn missing_profiles_are_written_and_edits_are_kept() {
    let dir = fixture("").with_file_name("profiles");
    write_missing_profiles(&dir).expect("write profiles");
    for profile in BundledProfile::ALL {
        let written = std::fs::read_to_string(dir.join(format!("{}.dm", profile.stem()))).expect("profile");
        assert_eq!(written, profile.source());
        if let Some((_, defines)) = profile.defines() {
            let written =
                std::fs::read_to_string(dir.join(format!("{}.defines.dm", profile.stem()))).expect("profile defines");
            assert_eq!(written, defines);
        }
    }
    assert_eq!(
        std::fs::read_to_string(dir.join("tgstation.defines.dm")).expect("defines"),
        "#ifdef __DEMIR_BAKE__\n#define CBT\n#endif\n"
    );
    assert!(!dir.join("cmss13.defines.dm").exists());

    let edited = dir.join("tgstation.dm");
    std::fs::write(&edited, "// mine now").expect("edit profile");
    let deleted = dir.join("vanderlin.dm");
    std::fs::remove_file(&deleted).expect("delete profile");
    write_missing_profiles(&dir).expect("write profiles again");

    assert_eq!(std::fs::read_to_string(&edited).expect("edited"), "// mine now");
    assert_eq!(
        std::fs::read_to_string(&deleted).expect("restored"),
        BundledProfile::Vanderlin.source()
    );

    std::fs::remove_dir_all(dir.parent().expect("fixture parent")).expect("remove fixture");
}

#[test]
fn scanning_lists_custom_profiles_but_not_defines() {
    let dir = fixture("").with_file_name("profiles");
    write_missing_profiles(&dir).expect("write profiles");
    std::fs::write(dir.join("mine.dm"), "").expect("custom profile");
    std::fs::write(dir.join("mine.defines.dm"), "").expect("custom defines");
    std::fs::write(dir.join("notes.txt"), "").expect("stray file");

    assert_eq!(
        scan_profiles(&dir),
        [
            "beestation",
            "cmss13",
            "goonstation",
            "mine",
            "monkespaw",
            "monkestation",
            "novasector",
            "oculis",
            "secondcity",
            "tgmc",
            "tgstation",
            "vanderlin",
        ]
    );

    std::fs::remove_dir_all(dir.parent().expect("fixture parent")).expect("remove fixture");
}

#[test]
fn forcing_a_custom_profile_activates_its_default_and_injects_its_defines() {
    let entry = fixture(
        r#"
#ifdef MINE_DEFINED
/obj/mine_marker
#endif
#ifdef __DEMIR_BAKE__
/datum/demir/native
    default = TRUE
#endif
"#,
    );
    let dir = entry.with_file_name("profiles");
    std::fs::create_dir_all(&dir).expect("profiles directory");
    std::fs::write(
        dir.join("mine.dm"),
        "#ifdef __DEMIR_BAKE__\n/datum/demir/mine\n    default = TRUE\n/datum/demir/mine/debug\n#endif\n",
    )
    .expect("custom profile");
    std::fs::write(dir.join("mine.defines.dm"), "#define MINE_DEFINED\n").expect("custom defines");
    let options = BakeOptions {
        forced_profile: Some(String::from("mine")),
        profile_dir: Some(dir.clone()),
        ..BakeOptions::default()
    };
    let (environment, diagnostics) = Environment::load_with(&entry, options, &Progress::new()).expect("load");

    assert!(diagnostics.profile.is_none(), "{:?}", diagnostics.profile);
    assert_eq!(
        environment.profiles.as_ref().map(|profiles| profiles.active.as_str()),
        Some("/datum/demir/mine")
    );
    assert!(environment.bake_files.contains(&dir.join("mine.dm")));
    assert!(environment.tree.id_of(&TreePath::parse("/obj/mine_marker")).is_some());

    std::fs::remove_dir_all(entry.parent().expect("fixture parent")).expect("remove fixture");
}

#[test]
fn a_disk_copy_replaces_the_embedded_profile_until_it_is_deleted() {
    let entry = fixture("");
    let dir = entry.with_file_name("profiles");
    std::fs::create_dir_all(&dir).expect("profiles directory");
    let copy = dir.join("cmss13.dm");
    std::fs::write(
        &copy,
        "#ifdef __DEMIR_BAKE__\n/datum/demir/edited\n    default = TRUE\n#endif\n",
    )
    .expect("edited profile");
    let load = || {
        let options = BakeOptions {
            forced_profile: Some(String::from("cmss13")),
            profile_dir: Some(dir.clone()),
            ..BakeOptions::default()
        };
        let (environment, _) = Environment::load_with(&entry, options, &Progress::new()).expect("load");

        environment.profiles.map(|profiles| profiles.active)
    };

    assert_eq!(load().as_deref(), Some("/datum/demir/edited"));
    std::fs::remove_file(&copy).expect("delete profile");
    assert_eq!(load().as_deref(), Some("/datum/demir/cmss13"));

    std::fs::remove_dir_all(entry.parent().expect("fixture parent")).expect("remove fixture");
}

#[test]
fn a_custom_profile_with_two_defaults_is_a_diagnostic() {
    let entry = fixture("");
    let dir = entry.with_file_name("profiles");
    std::fs::create_dir_all(&dir).expect("profiles directory");
    std::fs::write(
        dir.join("twice.dm"),
        "#ifdef __DEMIR_BAKE__\n/datum/demir/one\n    default = TRUE\n/datum/demir/two\n    default = \
             TRUE\n#endif\n",
    )
    .expect("custom profile");
    let options = BakeOptions {
        forced_profile: Some(String::from("twice")),
        profile_dir: Some(dir),
        ..BakeOptions::default()
    };
    let (environment, diagnostics) = Environment::load_with(&entry, options, &Progress::new()).expect("load");

    assert!(matches!(
        diagnostics.profile,
        Some(vm::bake::ProfileError::MultipleDefaults(_))
    ));
    assert!(environment.bake_program.is_none());

    std::fs::remove_dir_all(entry.parent().expect("fixture parent")).expect("remove fixture");
}

/// `/obj/fake_area { parent_type = /area }`
#[test]
fn area_paths_follow_parent_types_and_are_built_once() {
    let mut tree = ObjectTree::new();
    tree.register(&TreePath::parse("/area/station"), Location::default());
    tree.register(&TreePath::parse("/obj/table"), Location::default());
    let fake = tree.register(&TreePath::parse("/obj/fake_area"), Location::default());
    tree.get_mut(fake).unwrap().parent_type = Some(TreePath::parse("/area"));
    tree.resolve_parent_types();
    let environment = Environment::new(".", tree);

    let paths = environment.area_paths();
    for path in ["/area", "/area/station", "/obj/fake_area"] {
        assert!(paths.contains(&TreePath::parse(path).segments), "{path}");
    }
    assert!(!paths.contains(&TreePath::parse("/obj/table").segments));
    assert!(Arc::ptr_eq(&paths, &environment.area_paths()));
}

fn tree_with_vars(vars: &[(&str, Value)]) -> ObjectTree {
    let mut tree = ObjectTree::new();
    let id = tree.register(&TreePath::parse("/obj/item"), Location::default());

    if let Some(decl) = tree.get_mut(id) {
        for (name, value) in vars {
            let name = Identifier::from(String::from(*name));
            decl.vars.insert(
                name.clone(),
                VarDecl {
                    name,
                    declared_type: None,
                    modifiers: VarModifiers::default(),
                    value: value.clone(),
                    declared: true,
                    location: Location::default(),
                    resolved_type: None,
                    shared: None,
                },
            );
        }
    }

    tree
}

#[test]
fn resource_paths_resolve_regardless_of_case() {
    let dir = std::env::temp_dir().join(format!("rmd-resource-case-{}", std::process::id()));
    let nested = dir.join("Icons").join("Obj");
    std::fs::create_dir_all(&nested).expect("temp dir");
    std::fs::write(nested.join("Items.dmi"), []).expect("write fixture");

    let found = super::resolve_resource_path(&dir, "icons/obj/items.dmi");
    assert_eq!(found, Some(nested.join("Items.dmi")));
    assert!(super::resolve_resource_path(&dir, "icons/obj/missing.dmi").is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn collects_dmi_resource_literals_only() {
    let tree = tree_with_vars(&[
        ("icon", Value::Resource(String::from("icons/obj/items.dmi"))),
        ("sound", Value::Resource(String::from("sound/bang.ogg"))),
        ("name", Value::Text(String::from("not a resource.dmi"))),
    ]);
    let environment = Environment::new("/project/game/tgstation.dme", tree);

    assert_eq!(
        environment.icon_paths().into_iter().collect::<Vec<_>>(),
        ["icons/obj/items.dmi"]
    );
}

#[test]
fn load_icons_reports_misses_instead_of_aborting() {
    let tree = tree_with_vars(&[("icon", Value::Resource(String::from("icons/nope.dmi")))]);
    let mut environment = Environment::new("/project/game/tgstation.dme", tree);

    let failures = environment.load_icons(&[], &Progress::new());
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].0, "icons/nope.dmi");
    assert!(environment.icons.is_empty());
}

#[test]
fn icon_paths_include_the_ones_a_list_holds() {
    let entry = |key, value| ListEntry { key, value };
    let lights = Value::List(vec![
        entry(
            Value::Text(String::from("32")),
            Some(Value::Resource(String::from("icons/light_32.dmi"))),
        ),
        entry(Value::Resource(String::from("icons/key.dmi")), None),
    ]);
    let environment = Environment::new(
        "/project/game/tgstation.dme",
        tree_with_vars(&[("light_overlays", lights)]),
    );

    assert_eq!(
        environment.icon_paths(),
        BTreeSet::from(["icons/key.dmi", "icons/light_32.dmi"])
    );
}

const FINGERPRINT_DME: &str = "#include \"code/a.dm\"";

const FINGERPRINT_CODE: &str =
    "/obj/a\n\tname = \"a\"\n\ticon = 'icons/a.dmi'\n\n/obj/b\n\n/obj/a/proc/act()\n\treturn 1\n";

fn write_codebase(name: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rmd-fingerprint-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (path, contents) in files {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    std::fs::create_dir_all(dir.join("icons")).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/env/icons/test.dmi"),
        dir.join("icons/a.dmi"),
    )
    .unwrap();

    dir
}

fn load_fingerprint(dir: &Path) -> CodebaseHash {
    Environment::load(dir.join("game.dme"))
        .unwrap()
        .0
        .fingerprint()
        .unwrap()
}

fn fingerprint_of(name: &str, code: &str) -> CodebaseHash {
    let dir = write_codebase(name, &[("game.dme", FINGERPRINT_DME), ("code/a.dm", code)]);
    let fingerprint = load_fingerprint(&dir);
    let _ = std::fs::remove_dir_all(&dir);

    fingerprint
}

#[test]
fn formatting_procs_and_file_layout_do_not_change_the_fingerprint() {
    let original = fingerprint_of("original", FINGERPRINT_CODE);
    assert_eq!(
        fingerprint_of("crlf", &FINGERPRINT_CODE.replace('\n', "\r\n")),
        original
    );
    assert_eq!(
        fingerprint_of("comment", &format!("// a comment\n{FINGERPRINT_CODE}/* trailing */\n")),
        original
    );
    assert_eq!(
        fingerprint_of("proc", &FINGERPRINT_CODE.replace("return 1", "return 2")),
        original
    );

    let (types, procs) = FINGERPRINT_CODE.split_at(FINGERPRINT_CODE.find("/obj/a/proc").unwrap());
    let dir = write_codebase(
        "split",
        &[
            ("game.dme", "#include \"code/a.dm\"\n#include \"code/b.dm\""),
            ("code/a.dm", types),
            ("code/b.dm", procs),
        ],
    );
    assert_eq!(load_fingerprint(&dir), original);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn types_and_defaults_change_the_fingerprint() {
    let original = fingerprint_of("defaults", FINGERPRINT_CODE);
    assert_ne!(
        fingerprint_of("default", &FINGERPRINT_CODE.replace("name = \"a\"", "name = \"b\"")),
        original
    );
    assert_ne!(fingerprint_of("type", &format!("{FINGERPRINT_CODE}/obj/c\n")), original);
    assert_ne!(
        fingerprint_of("parent", &format!("{FINGERPRINT_CODE}/obj/a\n\tparent_type = /obj/b\n")),
        original
    );
}

#[test]
fn overriding_a_builtin_var_changes_the_fingerprint() {
    assert_ne!(
        fingerprint_of(
            "builtin",
            &format!("{FINGERPRINT_CODE}/atom\n\tdemir_light_range = 2\n")
        ),
        fingerprint_of("builtin-original", FINGERPRINT_CODE)
    );
}

#[test]
fn icons_and_maps_do_not_change_the_fingerprint() {
    let dir = write_codebase(
        "assets",
        &[
            ("game.dme", FINGERPRINT_DME),
            ("code/a.dm", FINGERPRINT_CODE),
            ("_maps/a.dmm", "map"),
        ],
    );
    let original = load_fingerprint(&dir);

    std::fs::write(dir.join("_maps/a.dmm"), "edited map").unwrap();
    let mut icon = std::fs::read(dir.join("icons/a.dmi")).unwrap();
    icon.extend_from_slice(b"changed icon bytes");
    std::fs::write(dir.join("icons/a.dmi"), icon).unwrap();
    assert_eq!(load_fingerprint(&dir), original);

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn builtin_declarations_are_not_fingerprinted() {
    // the synthetic root sits in the first opened file, which is a built-in prelude file
    let root = base::location::Location::default().file;
    assert_eq!(
        fingerprint_of("empty", ""),
        crate::environment::fingerprint::object_tree(&ObjectTree::new(), &[root])
    );
}

#[test]
fn an_unloaded_environment_has_no_fingerprint() {
    assert!(Environment::new("game.dme", ObjectTree::new()).fingerprint().is_err());
}

#[test]
fn local_bake_settings_do_not_change_the_fingerprint() {
    use crate::environment::BakeOptions;

    let entry = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/env/test.dme");
    let fingerprint = |options| {
        Environment::load_with(&entry, options, &Progress::new())
            .unwrap()
            .0
            .fingerprint()
            .unwrap()
    };
    let baking = BakeOptions {
        enabled: true,
        ..Default::default()
    };
    let original = fingerprint(BakeOptions::default());
    assert_eq!(fingerprint(baking.clone()), original);

    let forced = BakeOptions {
        forced_profile: Some(String::from("tgstation")),
        ..baking
    };
    assert_eq!(fingerprint(forced), original);
}
