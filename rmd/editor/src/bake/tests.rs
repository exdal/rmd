use base::{path::TreePath, types::Value};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use dmm::{Map, Size};
use objtree::Builtins;

use super::*;
use crate::{
    document::command::Edit,
    environment::world::Perspective,
    frame::{self, fixtures, visual},
    session::Session,
    testing,
};

fn examples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env")
}

fn environment(profile: &'static str) -> Environment {
    let mut environment = testing::compile_environment(&[
        ("<test-profile.dm>", profile),
        ("<test-profile-default.dm>", "/datum/demir/test\n    default = TRUE\n"),
    ]);
    let failures = environment.load_icons(&[], &crate::environment::progress::Progress::new());
    assert!(failures.is_empty(), "{failures:?}");

    environment
}

fn options<'a>(
    visibility: &'a frame::options::TypeVisibility, bake: &'a Bake,
) -> frame::options::FrameRenderOptions<'a> {
    frame::options::FrameRenderOptions {
        perspective: Perspective::TopDown,
        node_types: None,
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
    let definition = vm::profile::ProfileDefinition::resolve(&program.tree, &program.module.index, profile);
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
    assert!(frame::sprite_texture(&textures, &smoothed_appearance).is_some());
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
    let disabled = crate::environment::compile::BakeOptions {
        enabled: false,
        ..Default::default()
    };
    let (disabled, _) = Environment::load_with(
        root.join("test.dme"),
        disabled,
        &crate::environment::progress::Progress::new(),
    )
    .expect("codebase with baking off");
    let phi_only = crate::environment::compile::BakeOptions {
        optimizations_enabled: false,
        ..Default::default()
    };
    let (phi_only, _) = Environment::load_with(
        root.join("test.dme"),
        phi_only,
        &crate::environment::progress::Progress::new(),
    )
    .expect("codebase with optional optimizations off");

    assert!(profiled.bake_program.is_some());
    assert!(bare.bake_program.is_none());
    assert!(disabled.bake_program.is_none());
    assert!(phi_only.bake_program.is_some());
    assert_eq!(profiled.optimization_timings.samples(), 1);
    assert_eq!(disabled.optimization_timings.samples(), 0);
    assert_eq!(phi_only.optimization_timings.samples(), 1);

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
fn overlays_follow_the_owner_dir_and_sort_by_layer() -> Result<(), &'static str> {
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
    let prefab = document
        .prefab_instance(table)
        .ok_or("prefab instance is missing")?
        .prefab();
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
    let visibility = frame::options::TypeVisibility::default();
    let instances = fixtures::build_with_options(&environment.tree, &textures, &document, options(&visibility, &bake));
    let depths = instances.live_sprites().map(|sprite| sprite.depth).collect::<Vec<_>>();

    assert_eq!(depths, [1.0, owner.layer, owner.layer]);
    Ok(())
}

#[test]
fn overlays_below_float_layer_draw_under_their_owner() {
    let environment = environment(
        r##"
/datum/demir/test/bake(atom/target)
    if(!istype(target, /obj/structure/table))
        return
    var/image/above = new
    above.icon_state = "floor"
    above.layer = -1
    above.color = "#0000ff"
    var/image/below = new
    below.icon_state = "light"
    below.layer = -1.01
    below.color = "#ff0000"
    target.overlays += above
    target.overlays += below
"##,
    );
    let mut map = Map::new(Size { x: 1, y: 1, z: 1 });
    let key = map.intern_tile(vec![Prefab::new(TreePath::parse("/obj/structure/table"))]);
    map.grid[0] = vec![vec![key]];
    let document = MapDocument::new(map, 1);
    let bake = build(&environment, &document).expect("baking is on");
    assert_eq!(bake.diagnostics.count(), 0, "{:?}", bake.diagnostics);

    let mut textures = render::texture::TextureCatalog::new();
    textures
        .insert(
            "icons/test.dmi",
            &dmi::IconFile::load(examples().join("icons/test.dmi")).expect("test icon"),
        )
        .expect("pack test icon");
    let visibility = frame::options::TypeVisibility::default();
    let instances = fixtures::build_with_options(&environment.tree, &textures, &document, options(&visibility, &bake));
    let colors = instances
        .live_sprites()
        .filter(|sprite| sprite.depth > 1.0)
        .map(|sprite| sprite.color)
        .collect::<Vec<_>>();

    assert_eq!(colors, [[1.0, 0.0, 0.0, 1.0], [1.0; 4], [0.0, 0.0, 1.0, 1.0]]);
}

#[test]
fn derived_sprites_follow_edits_and_history_without_changing_map_bytes() -> Result<(), &'static str> {
    let environment = environment(WALLS);
    let mut map = Map::new(Size { x: 3, y: 3, z: 1 });
    let key = map.intern_tile(vec![Prefab::new(TreePath::parse("/turf/closed/wall"))]);
    map.grid[0] = vec![vec![key; 3]; 3];
    let mut document = MapDocument::new(map, 1);
    let bytes = dmm::writer::MapWriter::new(&document.to_map()).write();

    let mut bake = build(&environment, &document).expect("baking is on");
    let before = bake.appearances.clone();
    let center = document.instance_ids_at(Coord::new(2, 2, 1))[0];
    let south = document.instance_ids_at(Coord::new(2, 1, 1))[0];

    assert_eq!(bake.diagnostics.count(), 0, "{:?}", bake.diagnostics);
    assert_eq!(dmm::writer::MapWriter::new(&document.to_map()).write(), bytes);

    let prefab = document
        .prefab_instance(center)
        .ok_or("prefab instance is missing")?
        .prefab();
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
    let visibility = frame::options::TypeVisibility::default();
    let mut instances =
        fixtures::build_with_options(&environment.tree, &textures, &document, options(&visibility, &bake));

    // Every wall draws itself and its `light` overlay.
    assert_eq!(instances.live_sprites().count(), 18);

    let mut edit = Edit::new("remove wall");
    edit.change(&document, Coord::new(2, 2, 1), vec![]);
    assert!(document.apply(edit));
    let affected = update(&mut bake, &environment, &document, &[center]);

    assert_eq!(var(&bake, south, "name"), Some("12"));

    fixtures::update_prefabs_with_options(
        &mut instances,
        &environment.tree,
        &textures,
        &document,
        &affected.appearances,
        options(&visibility, &bake),
    );
    let rebuilt = fixtures::build_with_options(&environment.tree, &textures, &document, options(&visibility, &bake));

    assert!(instances.live_sprites().eq(rebuilt.live_sprites()));

    let affected = document.undo_with_affected().expect("undo");
    update(&mut bake, &environment, &document, &affected);

    assert_eq!(bake.appearances, before);

    let affected = document.redo_with_affected().expect("redo");
    update(&mut bake, &environment, &document, &affected);

    assert!(!bake.appearances.contains_key(&center.get()));
    Ok(())
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
fn undo_over_a_remote_edit_leaves_no_ghost_for_smoothing() {
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
    let mut map = Map::new(Size { x: 2, y: 1, z: 1 });
    let occupied = map.intern_tile(vec![floor.clone(), table.clone()]);
    let empty = map.intern_tile(vec![floor.clone()]);
    map.grid[0][0] = vec![occupied, empty];
    let mut document = MapDocument::new(map, 1);
    let left = document.instance_ids_at(Coord::new(1, 1, 1))[1];
    let mut bake = build(&environment, &document).expect("baking is on");

    let coord = Coord::new(2, 1, 1);
    let mut after = document.placed_tile(coord).expect("placed floor");
    after.push(document.instantiate(table.clone()));
    let mut edit = Edit::new("add table");
    edit.change(&document, coord, after);
    let affected = edit.affected_instances();
    assert!(document.apply(edit));
    update(&mut bake, &environment, &document, &affected);

    let affected = document.apply_remote(vec![(coord, vec![floor, table])]);
    update(&mut bake, &environment, &document, &affected);
    assert_eq!(var(&bake, left, "icon_state"), Some("table-4"));

    let affected = document.undo_with_affected().expect("undo");
    update(&mut bake, &environment, &document, &affected);

    assert_eq!(var(&bake, left, "icon_state"), Some("table-0"));
    let fresh = build(&environment, &document).expect("baking is on");
    assert_eq!(bake.appearances, fresh.appearances);
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

const GENERATED_ICONS: &str = r#"
/datum/demir/test/bake(atom/target)
    if(!istype(target, /obj/structure/table))
        return
    target.icon = file("gen/" + "made.dmi")
    target.icon_state = "floor"
    target.overlays += image(file("gen/" + "over.dmi"), "light")
"#;

struct GeneratedIcons {
    session: Session,
    owner: PrefabInstanceId,
    dir: PathBuf,
}

impl GeneratedIcons {
    fn new(has_files: bool) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);

        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("demir-generated-icons-{}-{id}", std::process::id()));
        std::fs::create_dir_all(dir.join("gen")).expect("temp dir");
        if has_files {
            for name in ["made.dmi", "over.dmi"] {
                std::fs::copy(examples().join("icons/test.dmi"), dir.join("gen").join(name)).expect("icon");
            }
        }

        let mut environment = environment(GENERATED_ICONS);
        environment.root = dir.join("test.dme");

        let mut map = Map::new(Size { x: 1, y: 1, z: 1 });
        let key = map.intern_tile(vec![Prefab::new(TreePath::parse("/obj/structure/table"))]);
        map.grid[0] = vec![vec![key]];
        let document = MapDocument::new(map, 1);
        let owner = document.instance_ids_at(Coord::new(1, 1, 1))[0];
        let bake = build(&environment, &document).expect("baking is on");
        assert_eq!(bake.diagnostics.count(), 0, "{:?}", bake.diagnostics);

        let mut session = Session::new();
        session.textures = crate::environment::session::build_textures(&environment, &Progress::new());
        session.textures.insert_missing_icon().expect("missing icon");
        session.state.environment = Some(Arc::new(environment));
        let id = session.state.open_document(document);
        session.caches.entry(id).or_default().bake = Some(bake);
        session.rebuild_instances(id);

        Self { session, owner, dir }
    }

    fn rebuild(&mut self) {
        let id = self.session.state.active().expect("document");
        self.session.rebuild_instances(id);
    }

    fn is_missing(&self) -> bool {
        let sprite = self
            .session
            .instances()
            .and_then(|instances| instances.sprite(self.owner));

        sprite.is_some_and(|sprite| self.session.textures.is_missing_icon(sprite.texture))
    }
}

impl Drop for GeneratedIcons {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

#[test]
fn icons_written_during_a_bake_register_when_the_bake_lands() {
    let mut generated = GeneratedIcons::new(true);
    let revision = generated.session.texture_revision();

    assert!(!generated.is_missing());
    assert!(generated.session.textures.metadata("gen/made.dmi").is_some());
    assert!(generated.session.textures.metadata("gen/over.dmi").is_some());
    assert_ne!(generated.session.texture_revision(), 0);

    generated.rebuild();

    assert_eq!(generated.session.texture_revision(), revision);
    assert!(!generated.is_missing());
}

#[test]
fn absent_generated_icons_stay_missing_and_are_retried() {
    let mut generated = GeneratedIcons::new(false);
    let revision = generated.session.texture_revision();

    assert!(generated.is_missing());
    assert!(generated.session.textures.metadata("gen/made.dmi").is_none());

    std::fs::copy(examples().join("icons/test.dmi"), generated.dir.join("gen/made.dmi")).expect("icon");
    generated.rebuild();

    assert!(!generated.is_missing());
    assert_ne!(generated.session.texture_revision(), revision);
}

#[test]
fn icon_names_follow_overlays_and_underlays_and_keep_only_dmi() {
    let builtins = Builtins::default();
    let icon = |name: &str| (builtins.icon.clone(), Value::Resource(name.into()));
    let delta = AppearanceDelta {
        vars: vec![icon("a.dmi")],
        overlays: vec![AppearanceDelta {
            vars: vec![icon("B/B.DMI")],
            underlays: vec![AppearanceDelta {
                vars: vec![icon("c.dmi")],
                ..Default::default()
            }],
            ..Default::default()
        }],
        underlays: vec![
            AppearanceDelta {
                vars: vec![icon("d.png")],
                ..Default::default()
            },
            AppearanceDelta {
                vars: vec![(builtins.icon_state.clone(), Value::Text("e.dmi".into()))],
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    assert_eq!(
        icon_names([&delta]),
        BTreeSet::from(["B/B.DMI".to_string(), "a.dmi".to_string(), "c.dmi".to_string()])
    );
}
