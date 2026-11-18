use base::{
    location::{FileId, Location, Position},
    path::TreePath,
    types::Value,
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use dmi::{
    IconFile,
    metadata::{IconState, Metadata},
};
use dmm::{Coord, Map, MapFormat, Prefab, Size, writer::MapWriter};
use objtree::ObjectTree;

use super::{
    DiagnosticSeverity, LoadReport, MAX_REPORTED_DIAGNOSTICS, build_textures, discover_maps, preprocess_severity,
};
use crate::{
    environment::{Environment, progress::Progress},
    session::{
        Session,
        fixtures::{assert_render_cache_matches_rebuild, examples},
    },
};

#[test]
fn type_sources_resolve_compiler_locations_to_loaded_files() {
    let location = Location::in_file(FileId(0), Position::new(42, 7), Position::new(42, 16));
    let mut tree = ObjectTree::new();
    let id = tree.register(&TreePath::parse("/obj/item/tool"), location);
    let mut environment = Environment::new("station.dme", tree);
    environment.files.push(PathBuf::from("code/items.dm"));
    let mut session = Session::new();
    session.state.environment = Some(Arc::new(environment));

    let source = session.type_source(id).expect("known type source");

    assert_eq!(source.path, PathBuf::from("code/items.dm"));
    assert_eq!(source.line, 42);
    assert_eq!(source.column, 7);
}

#[test]
fn type_sources_use_builtin_file_ids_instead_of_paths() {
    let location = Location::in_file(FileId(0), Position::new(42, 7), Position::new(42, 16));
    let mut tree = ObjectTree::new();
    let builtin = tree.register(&TreePath::parse("/obj/builtin"), location);
    let project = tree.register(&TreePath::parse("/obj/project"), location.with_file(FileId(1)));
    let mut environment = Environment::new("station.dme", tree);
    environment
        .files
        .extend(["code/embedded.dm", "code/stddef_ext.dm"].map(PathBuf::from));
    environment.builtin_files.push(FileId(0));
    let mut session = Session::new();
    session.state.environment = Some(Arc::new(environment));

    assert!(session.type_source(builtin).is_none());
    let source = session
        .type_source(project)
        .expect("project source named like a builtin");
    assert_eq!(source.path, PathBuf::from("code/stddef_ext.dm"));
}

#[test]
fn load_report_classifies_preprocessor_levels_and_caps_only_displayed_lines() {
    use preprocessor::diagnostic::Level;

    assert_eq!(preprocess_severity(Level::Notice), DiagnosticSeverity::Warning);
    assert_eq!(preprocess_severity(Level::Warning), DiagnosticSeverity::Warning);
    assert_eq!(preprocess_severity(Level::Error), DiagnosticSeverity::Error);

    let mut report = LoadReport::default();
    assert!(report.is_empty());
    for index in 0..=MAX_REPORTED_DIAGNOSTICS {
        report.record(DiagnosticSeverity::Warning, format!("warning {index}"));
    }
    report.record(DiagnosticSeverity::Error, String::from("error"));
    assert_eq!(report.warnings, MAX_REPORTED_DIAGNOSTICS + 1);
    assert_eq!(report.warning_lines.len(), MAX_REPORTED_DIAGNOSTICS);
    assert_eq!(report.errors, 1);
    assert!(!report.is_empty());
    assert_eq!(report.error_lines, ["error"]);
}

#[test]
fn map_and_failed_load_reports_keep_one_readable_path_prefix() {
    let error = dmm::error::MapError::new(
        dmm::error::MapErrorKind::RaggedGrid,
        base::location::Position::new(2, 3),
    );
    let mut map = Map::new(Size { x: 1, y: 1, z: 1 });
    map.intern_tile(Vec::new());
    map.intern_tile(vec![Prefab::new(TreePath::parse("/obj/missing"))]);
    let report = LoadReport::map("maps/level.dmm", &map, Some(&ObjectTree::new()), &[error]);
    assert_eq!(report.errors, 1);
    assert!(report.error_lines[0].starts_with("maps/level.dmm: Map error"));
    assert_eq!(report.warnings, 1);
    assert_eq!(report.warning_lines, ["maps/level.dmm: unknown type path /obj/missing"]);

    let failed = LoadReport::failure(
        "maps/level.dmm",
        Path::new("C:/game/maps/level.dmm"),
        "C:/game/maps/level.dmm: cannot read",
    );
    assert_eq!(failed.error_lines, ["maps/level.dmm: cannot read"]);
}

#[test]
fn map_reports_keep_only_unused_paths_while_placed_issues_belong_to_the_live_panel() {
    let source = r#"
"a" = (/obj/known,/obj)
"b" = (/obj/z_missing{name = "first"},/obj/known/missing,/obj/z_missing{name = "second"})
"c" = (/obj/a_unused)
(1,1,1) = {"
a
"}
(1,1,2) = {"
b
"}
"#;
    let (map, errors) = dmm::parser::parse(source);
    assert!(errors.is_empty(), "{errors:?}");
    let mut tree = ObjectTree::new();
    tree.register(&TreePath::parse("/obj/known"), Location::default());
    for format in [MapFormat::Standard, MapFormat::Tgm] {
        let source = MapWriter::new(&map).with_format(format).write();
        let (map, errors) = dmm::parser::parse(&source);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(map.format, format);
        assert_eq!(map.size.z, 2);
        let report = LoadReport::map("maps/level.dmm", &map, Some(&tree), &errors);
        assert_eq!(report.errors, 0);
        assert_eq!(report.warnings, 1);
        assert_eq!(
            report.warning_lines,
            ["maps/level.dmm: unknown type path /obj/a_unused"]
        );
        assert!(LoadReport::map("maps/level.dmm", &map, None, &errors).is_empty());
    }
}

#[test]
fn map_reports_accept_implied_types_and_do_not_validate_paths_in_vars() {
    let mut tree = ObjectTree::new();
    tree.register(&TreePath::parse("/obj/known"), Location::default());
    let mut known = Prefab::new(TreePath::parse("/obj/known"));
    known.set_var("target".into(), Value::Path(TreePath::parse("/obj/only_in_var")));
    let mut map = Map::new(Size { x: 1, y: 1, z: 1 });
    map.intern_tile(vec![known, Prefab::new(TreePath::parse("/obj"))]);

    assert!(LoadReport::map("maps/level.dmm", &map, Some(&tree), &[]).is_empty());
}

#[test]
fn unknown_type_counts_include_paths_beyond_the_display_limit() {
    let mut map = Map::new(Size { x: 1, y: 1, z: 1 });
    map.intern_tile(Vec::new());
    map.intern_tile(
        (0..=MAX_REPORTED_DIAGNOSTICS)
            .map(|index| Prefab::new(TreePath::parse(&format!("/obj/missing_{index:04}"))))
            .collect::<Vec<_>>(),
    );
    let report = LoadReport::map("maps/level.dmm", &map, Some(&ObjectTree::new()), &[]);
    assert_eq!(report.warnings, MAX_REPORTED_DIAGNOSTICS + 1);
    assert_eq!(report.warning_lines.len(), MAX_REPORTED_DIAGNOSTICS);
    assert!(report.warning_lines[0].contains("/obj/missing_0000"));
}

#[test]
fn map_discovery_finds_maps_the_environment_never_includes() {
    let maps = discover_maps(&examples());

    assert!(
        maps.iter().any(|map| map.ends_with("test.dmm")),
        "expected test.dmm in {maps:?}"
    );
    assert!(maps.iter().all(|map| map.extension().is_some_and(|e| e == "dmm")));
}

#[test]
fn map_discovery_is_sorted_and_skips_missing_roots() {
    let maps = discover_maps(&examples());
    let mut sorted = maps.clone();
    sorted.sort();

    assert_eq!(maps, sorted);
    assert!(discover_maps(&examples().join("does-not-exist")).is_empty());
}

#[test]
fn environment_loading_packs_every_cell_in_each_dmi() {
    let root = examples();
    let (environment, diagnostics) = Environment::load(root.join("test.dme")).expect("load environment");
    assert!(diagnostics.icons.is_empty(), "{:?}", diagnostics.icons);
    let roots = environment.tree.roots();
    let obj = roots.obj.expect("embedded definitions register /obj");
    let atom = roots.atom.expect("embedded definitions register /atom");
    let movable = roots.movable.expect("embedded definitions register /atom/movable");
    assert!(environment.tree.is_subtype_of(obj, atom));
    assert!(environment.tree.is_subtype_of(obj, movable));
    assert_eq!(
        environment
            .tree
            .var_inherited(obj, &"pixel_x".into())
            .map(|variable| &variable.value),
        Some(&Value::Num(0.0)),
    );
    assert_eq!(
        environment
            .tree
            .var_inherited(obj, &"step_x".into())
            .map(|variable| &variable.value),
        Some(&Value::Num(0.0)),
    );
    let file = IconFile::load(root.join("icons/test.dmi")).expect("load icon");

    let textures = build_textures(&environment, &Progress::new());

    assert_eq!(textures.len(), 2);
    assert_eq!(textures.cell_count(), file.cell_count() + 1);
    assert!(textures.missing_icon().is_some());
    assert!((0..file.cell_count()).all(|cell| textures.lookup("icons/test.dmi", cell).is_some()));
}

#[test]
fn reloading_an_environment_resets_type_visibility() {
    let root = examples();
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).unwrap();
    session.open_map(&root.join("test.dmm"), 1).unwrap();
    let table = session
        .tree()
        .unwrap()
        .id_of(&TreePath::parse("/obj/structure/table"))
        .unwrap();
    let coord = Coord::new(6, 3, 1);
    let picked = session.state.active_document().unwrap().instance_ids_at(coord)[0];

    assert!(session.toggle_type_visibility(table));
    assert!(session.instances().unwrap().sprite(picked).is_none());
    session.pick_instance(Some(picked));
    assert_eq!(
        session.set_picked_instance_var("name".into(), Value::Text(String::from("Unsaved table"))),
        Some(true)
    );
    assert!(session.state.active_document().unwrap().is_dirty());
    session.load_environment(&root.join("test.dme")).unwrap();
    let reloaded_table = session
        .tree()
        .unwrap()
        .id_of(&TreePath::parse("/obj/structure/table"))
        .unwrap();

    assert!(session.is_type_visible(reloaded_table));
    assert!(session.instances().unwrap().sprite(picked).is_some());
    assert!(session.state.active_document().unwrap().is_dirty());
    assert_eq!(session.picked_instance(), Some(picked));
    assert_render_cache_matches_rebuild(&session);
}

fn icon_session(path: &str, states: &[(&str, bool)]) -> Session {
    let mut tree = ObjectTree::new();
    tree.register(&TreePath::parse(path), Location::default());
    let mut environment = Environment::new(".", tree);
    environment.icons.insert(
        String::from("sign.dmi"),
        Metadata {
            states: states
                .iter()
                .map(|(name, movement)| IconState {
                    name: name.to_string(),
                    dirs: 1,
                    frames: 1,
                    movement: *movement,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        },
    );
    let mut session = Session::new();
    session.state.environment = Some(Arc::new(environment));

    session
}

fn with_icon_state(prefab: &Prefab, state: &str) -> Prefab {
    let mut variant = prefab.clone();
    variant.set_var("icon_state".into(), Value::Text(state.to_string()));

    variant
}

#[test]
fn icon_state_variants_list_each_state_of_the_resolved_icon_once() {
    let session = icon_session(
        "/obj/sign",
        &[("left", false), ("", false), ("right", false), ("left", true)],
    );
    let mut prefab = Prefab::new(TreePath::parse("/obj/sign"));
    prefab.set_var("icon".into(), Value::Resource(String::from("sign.dmi")));
    prefab.set_var("icon_state".into(), Value::Text(String::from("left")));

    assert_eq!(
        session.icon_state_variants(&prefab),
        ["left", "", "right"]
            .into_iter()
            .map(|state| with_icon_state(&prefab, state))
            .collect::<Vec<_>>()
    );
}

#[test]
fn icon_state_variants_are_empty_without_a_known_icon() {
    let session = icon_session("/obj/sign", &[("left", false)]);
    let mut prefab = Prefab::new(TreePath::parse("/obj/sign"));

    assert!(session.icon_state_variants(&prefab).is_empty());

    prefab.set_var("icon".into(), Value::Resource(String::from("missing.dmi")));

    assert!(session.icon_state_variants(&prefab).is_empty());
}
