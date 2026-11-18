use base::{coord::Coord, path::TreePath, types::Value};
use std::sync::Arc;

use prefab::Prefab;

use crate::{
    environment::{progress::Progress, session::build_textures},
    session::{Session, fixtures::examples},
    tool::Tool,
};

#[test]
fn prefab_thumbnails_resolve_overrides_and_sheet_coordinates() {
    let root = examples();
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).unwrap();
    let mut prefab = Prefab::new(TreePath::parse("/obj/structure/table"));
    let inherited = session.prefab_thumbnail(&prefab).expect("inherited table thumbnail");

    prefab.set_var("icon_state".into(), Value::Text(String::from("light")));
    prefab.set_var("color".into(), Value::Text(String::from("#ff000080")));
    prefab.set_var("alpha".into(), Value::Num(128.0));
    let overridden = session.prefab_thumbnail(&prefab).expect("overridden light thumbnail");

    assert_eq!(inherited.texture.width, 32);
    assert_eq!(inherited.texture.height, 32);
    assert_ne!(inherited.uv0, overridden.uv0);
    assert_ne!(inherited.uv1, overridden.uv1);
    assert_eq!(overridden.tint[0..3], [1.0, 0.0, 0.0]);
    assert!((overridden.tint[3] - (128.0 / 255.0) * (128.0 / 255.0)).abs() < f32::EPSILON);

    prefab.set_var("icon".into(), Value::Resource(String::from("icons/missing.dmi")));
    let missing = session.prefab_thumbnail(&prefab).expect("missing icon thumbnail");
    assert_eq!(missing.texture, session.textures.missing_icon().unwrap());
    assert_eq!(missing.tint[0..3], [1.0; 3]);
    assert_eq!(missing.tint[3], overridden.tint[3]);
    assert!(
        session
            .prefab_thumbnail(&Prefab::new(TreePath::parse("/area/station")))
            .is_none()
    );
}

/// A smoothed type's static `icon_state` is only a prefix, so the sheet holds nothing under it
/// and the palette, the recent list and the placement preview would all draw nothing.
#[test]
fn palette_thumbnails_fall_back_to_a_standalone_bake_when_no_sprite_matches() {
    let root = examples();
    let profile = r#"
/turf/closed/wall/smoothed
    icon_state = "smooth"
/datum/demir/test
    default = TRUE
/datum/demir/test/bake(atom/target)
    if(istype(target, /turf/closed/wall/smoothed))
        target.icon_state = "wall"
"#;
    let compile = |baking| {
        let arena = base::arena::StrArena::new();
        let prelude = preprocessor::prelude_files()
            .into_iter()
            .chain([preprocessor::PreludeFile::Embedded("<test-standalone.dm>", profile)]);
        let preprocessed = preprocessor::Preprocessor::new(&arena)
            .with_prelude(prelude)
            .with_baking(baking)
            .run(root.join("test.dm"))
            .expect("preprocess");
        assert!(preprocessed.is_ok(), "{:?}", preprocessed.errors);

        let ast = ast::parse(&preprocessed.tokens).expect("parse");
        let (tree, errors) = sema::analyze(&ast);
        assert!(errors.is_empty(), "{errors:?}");
        let (module, _) = ir::lower(&ast, &tree, baking);

        (tree, module)
    };
    let (editor_tree, _) = compile(false);
    let (bake_tree, module) = compile(true);
    let selected_profile = vm::bake::profile_type(&bake_tree).expect("default profile");
    let mut environment = crate::environment::Environment::new(root.join("test.dme"), editor_tree);
    environment.bake_program = Some(crate::environment::BakeProgram {
        tree: bake_tree,
        module: codegen::generate(&module).expect("codegen"),
        profile: selected_profile,
        icon_states: Default::default(),
        uses_external_libraries: false,
    });
    assert!(environment.load_icons(&[], &Progress::new()).is_empty());

    let mut session = Session::new();
    session.textures = build_textures(&environment, &Progress::new());
    session.state.environment = Some(Arc::new(environment));

    let smoothed = Prefab::new(TreePath::parse("/turf/closed/wall/smoothed"));

    // "smooth" is in no sheet, so only a standalone bake makes this drawable
    assert_eq!(
        session.prefab_appearance(&smoothed).and_then(|a| a.icon_state),
        Some(String::from("wall"))
    );
    assert!(session.prefab_thumbnail(&smoothed).is_some());

    session.state.choose_prefab(smoothed.clone());
    session.set_tool(Tool::Place);

    assert!(session.placement_preview().is_some());

    // a type whose static state already matches never reaches the baker
    let plain = Prefab::new(TreePath::parse("/turf/closed/wall"));

    assert_eq!(
        session.prefab_appearance(&plain).and_then(|a| a.icon_state),
        Some(String::from("wall"))
    );
    assert_eq!(session.standalone.len(), 1);
}

#[test]
fn tree_choices_place_new_objects_incrementally_and_select_them() {
    let root = examples();
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).unwrap();
    session.open_map(&root.join("test.dmm"), 1).unwrap();
    let table = session
        .tree()
        .unwrap()
        .id_of(&TreePath::parse("/obj/structure/table"))
        .unwrap();
    let coord = Coord::new(2, 2, 1);
    let before_len = session.map().unwrap().tile_at(coord).unwrap().len();
    let revision = session.revision();

    assert_eq!(session.place_at(coord, None), None);
    assert!(session.choose_type(table));
    assert_eq!(session.tool(), Tool::Place);
    let picked = session.place_at(coord, None).unwrap();

    assert_eq!(session.picked_instance(), Some(picked));
    assert_eq!(session.map().unwrap().tile_at(coord).unwrap().len(), before_len + 1);
    assert_eq!(
        session.picked_prefab().unwrap().path,
        TreePath::parse("/obj/structure/table")
    );
    assert_eq!(session.recent_prefabs()[0], *session.picked_prefab().unwrap());
    assert!(session.instances().unwrap().sprite(picked).is_some());
    assert_eq!(session.revision(), revision.wrapping_add(1));
    assert_eq!(session.frame_update().unwrap().previous_revision, revision);
}

#[test]
fn choosing_a_prefab_sets_the_brush_and_switches_to_place() {
    let root = examples();
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).unwrap();
    let table = Prefab::new(TreePath::parse("/obj/structure/table"));

    session.set_tool(Tool::Delete);
    session.choose_prefab(table.clone());
    assert_eq!(session.palette(), Some(&table));
    assert_eq!(session.tool(), Tool::Place);
    assert_eq!(session.recent_prefabs()[0], table);

    for tool in [Tool::Fill, Tool::RectFill, Tool::Select] {
        session.set_tool(tool);
        session.choose_prefab(table.clone());
        assert_eq!(session.tool(), tool);
    }
}

#[test]
fn palette_choices_preserve_fill_and_region_select_modes() {
    let root = examples();
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).unwrap();
    let floor = session
        .tree()
        .unwrap()
        .id_of(&TreePath::parse("/turf/open/floor"))
        .unwrap();

    for tool in [Tool::Fill, Tool::RectFill, Tool::Select] {
        session.set_tool(tool);
        assert!(session.choose_type(floor));
        assert_eq!(session.tool(), tool);

        assert!(session.choose_recent(0));
        assert_eq!(session.tool(), tool);
    }
}
