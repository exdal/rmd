use core::{path::TreePath, types::Value};
use std::{
    env,
    error::Error,
    fs,
    path::PathBuf,
    process,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Instant,
};

use dmm::{Coord, Map, MapFormat, Prefab, PrefabInstanceId, Size, writer};
use editor::{
    document::{DocumentId, MapDocument},
    environment::BakeOptions,
    maploader::{ErrorKind, preview::PREVIEW_ID_START},
    progress::Progress,
    tool::Tool,
};

use crate::{
    loader::load_codebase,
    session::{
        ModuleBake,
        Session,
        build_textures,
        fixtures::{assert_render_cache_matches_rebuild, node_environment_with_profile, settle_bake},
    },
};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const PROFILE: &str = include_str!("../../../tests/fixtures/programs/module_preview_profile.dm");
const PLACEMENTS_PROFILE: &str = include_str!("../../../tests/fixtures/programs/module_placements_profile.dm");
const WALL: &str = "/turf/closed/wall";
const MODULE: &str = "/obj/module";
const ROOT: &str = "/obj/modular_map_root";

struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(roots: &[(u32, &str)]) -> TestResult<Self> { Self::with_variants(roots, "'a.dmm', 'b.dmm'") }

    fn with_variants(roots: &[(u32, &str)], variants: &str) -> TestResult<Self> {
        static NEXT: AtomicUsize = AtomicUsize::new(0);

        let dir = env::temp_dir().join(format!(
            "rmd-module-preview-{}-{}",
            process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir)?;
        let fixture = Self { dir };
        fs::write(
            fixture.dir.join("config.toml"),
            format!(
                "directory = './'\n[rooms.outer]\nmodules = [{variants}]\n[rooms.child]\nmodules = \
                 ['child.dmm']\n[rooms.alt]\nmodules = ['b.dmm']\n"
            ),
        )?;
        fixture.write_map(
            "a.dmm",
            &map(&[
                vec![
                    prefab("/obj/modular_map_connector"),
                    prefab(WALL),
                    prefab("/area/station"),
                ],
                vec![
                    keyed("child"),
                    prefab("/turf/template_noop"),
                    prefab("/area/template_noop"),
                ],
            ]),
        )?;
        fixture.write_map(
            "b.dmm",
            &map(&[vec![
                prefab("/obj/modular_map_connector"),
                prefab(MODULE),
                prefab("/turf/open/floor"),
                prefab("/area/station"),
            ]]),
        )?;
        fixture.write_map(
            "child.dmm",
            &map(&[vec![
                prefab("/obj/modular_map_connector"),
                prefab(WALL),
                prefab("/area/station"),
            ]]),
        )?;

        let roots = roots.iter().map(|(x, key)| (*x, keyed(key))).collect::<Vec<_>>();
        fixture.write_main(&roots)?;

        Ok(fixture)
    }

    fn write_main(&self, roots: &[(u32, Prefab)]) -> TestResult {
        let tiles = (1..=6)
            .map(|x| {
                let mut tile = vec![prefab("/turf/open/floor"), prefab("/area/station")];
                if let Some((_, root)) = roots.iter().find(|(root, _)| *root == x) {
                    tile.insert(0, root.clone());
                }

                tile
            })
            .collect::<Vec<_>>();

        self.write_map("main.dmm", &map(&tiles))
    }

    fn write_map(&self, name: &str, map: &Map) -> TestResult {
        fs::write(self.dir.join(name), writer::write(map))?;

        Ok(())
    }

    fn session(&self) -> TestResult<Session> { self.session_with(PROFILE) }

    fn session_with(&self, profile: &'static str) -> TestResult<Session> {
        let mut environment = node_environment_with_profile(profile);
        let progress = Progress::new();
        assert!(environment.load_icons(&[], &progress).is_empty());
        let textures = build_textures(&environment, &progress);
        environment.root = self.dir.join("fixture.dme");
        let mut session = Session::new();
        session.sync_git_enabled(false);
        session.textures = textures;
        session.state.environment = Some(Arc::new(environment));
        session.open_map(&self.dir.join("main.dmm"), 1)?;
        settle_bake(&mut session);
        assert!(!session.baker.is_busy());

        Ok(session)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.dir); }
}

pub(crate) fn viewport_session() -> TestResult<(impl Sized, Session)> {
    let fixture = Fixture::with_variants(&[(2, "outer")], "'missing.dmm', 'a.dmm', 'b.dmm'")?;
    let session = fixture.session()?;

    Ok((fixture, session))
}

fn prefab(path: &str) -> Prefab { Prefab::new(TreePath::parse(path)) }

fn keyed(key: &str) -> Prefab {
    let mut root = prefab(ROOT);
    if key != "outer" {
        root.set_var("key".into(), Value::Text(key.into()));
    }

    root
}

fn map(tiles: &[Vec<Prefab>]) -> Map {
    let mut map = Map::new(Size {
        x: tiles.len() as u32,
        y: 1,
        z: 1,
    });
    map.grid[0][0] = tiles
        .iter()
        .map(|tile| map.intern_tile(tile.clone()))
        .collect::<Vec<_>>();

    map
}

fn coord(x: u32) -> Coord { Coord::new(x, 1, 1) }

fn effective_paths(session: &Session, x: u32) -> Vec<String> {
    let id = session.state.active().unwrap();

    session
        .effective_grid(id)
        .and_then(|grid| grid.tile_at(coord(x)))
        .map(|tile| tile.iter().map(|prefab| prefab.path.to_string()).collect())
        .unwrap_or_default()
}

fn source_paths(session: &Session, x: u32) -> Vec<String> {
    session
        .map()
        .and_then(|map| map.tile_at(coord(x)))
        .map(|tile| tile.iter().map(|prefab| prefab.path.to_string()).collect())
        .unwrap_or_default()
}

fn has(session: &Session, x: u32, path: &str) -> bool { effective_paths(session, x).iter().any(|found| found == path) }

fn preview_ids(session: &Session) -> Vec<PrefabInstanceId> {
    let id = session.state.active().unwrap();
    let grid = session.effective_grid(id).unwrap();

    (1..=grid.size().x)
        .flat_map(|x| grid.ids_at(coord(x)).iter().copied())
        .filter(|id| id.get() >= PREVIEW_ID_START)
        .collect()
}

fn is_baked(session: &Session, id: PrefabInstanceId) -> bool {
    session
        .active_cache()
        .bake
        .as_ref()
        .is_some_and(|bake| bake.appearances.contains_key(&id.get()))
}

fn root_at(session: &Session, x: u32) -> PrefabInstanceId {
    let id = session.state.active().unwrap();
    let document = session.state.document(id).unwrap();

    *document
        .instance_ids_at(coord(x))
        .iter()
        .find(|owner| {
            document
                .prefab_instance(**owner)
                .is_some_and(|instance| instance.prefab().path.to_string() == ROOT)
        })
        .expect("a root is placed there")
}

fn baked_position(session: &Session, id: PrefabInstanceId) -> Option<vm::world::Position> {
    session.active_cache().bake.as_ref()?.position(id.get())
}

fn is_deferred(session: &Session) -> bool { matches!(session.active_cache().module_bake, ModuleBake::Deferred { .. }) }

fn assert_settled(session: &Session) {
    assert!(!session.baker.is_busy());
    assert!(session.bake_view().is_none());
    assert_render_cache_matches_rebuild(session);
}

#[test]
fn opening_a_map_with_a_root_bakes_the_composite() -> TestResult {
    let fixture = Fixture::new(&[(2, "outer")])?;
    let session = fixture.session()?;
    let id = session.state.active().ok_or("no active document")?;

    assert_eq!(session.module_roots().len(), 2);
    assert_eq!(session.module_roots()[0].selected, Some(0));
    assert_eq!(session.module_roots()[1].depth, 1);
    assert!(has(&session, 2, WALL));
    assert!(has(&session, 3, WALL));
    assert!(has(&session, 2, ROOT));
    assert_eq!(source_paths(&session, 2)[0], ROOT);
    assert!(!has(&session, 2, "/turf/open/floor"));
    assert!(source_paths(&session, 2).contains(&String::from("/turf/open/floor")));
    assert!(!session.state.document(id).ok_or("document is missing")?.is_dirty());

    let previews = preview_ids(&session);
    assert!(!previews.is_empty());
    assert!(previews.iter().all(|id| is_baked(&session, *id)));
    assert!(
        session
            .active_cache()
            .instances
            .sprites
            .iter()
            .any(|sprite| sprite.owner.get() >= PREVIEW_ID_START)
    );
    assert!(session.module_roots().iter().all(|root| {
        root.marker
            .as_ref()
            .and_then(|marker| session.module_marker(marker))
            .is_some()
    }));
    assert_settled(&session);

    Ok(())
}

#[test]
fn placements_made_while_baking_show_up_when_the_bake_lands() -> TestResult {
    let fixture = Fixture::new(&[(2, "prepared"), (4, "outer")])?;
    let session = fixture.session_with(PLACEMENTS_PROFILE)?;

    assert_eq!(session.module_roots().len(), 1);
    assert!(has(&session, 2, WALL));
    assert!(!has(&session, 4, WALL));
    assert!(preview_ids(&session).iter().all(|id| is_baked(&session, *id)));
    assert_settled(&session);

    Ok(())
}

#[test]
fn a_ui_button_can_place_a_module_on_the_selected_atom() -> TestResult {
    let fixture = Fixture::new(&[(2, "outer")])?;
    let mut session = fixture.session_with(PLACEMENTS_PROFILE)?;
    let id = session.state.active().ok_or("no active document")?;
    assert!(session.module_roots().is_empty());

    let root = root_at(&session, 2);
    session
        .state
        .document_mut(id)
        .ok_or("document is missing")?
        .select_instance(Some(root));
    let frame = session.dm_ui(
        0,
        editor::bake::UiFeedback {
            clicks: ["Panel/Place".to_owned()].into(),
            interacted: true,
            ..Default::default()
        },
    );

    assert!(frame.committed);
    assert_eq!(session.module_roots().len(), 1);
    assert!(has(&session, 2, WALL));
    assert!(preview_ids(&session).iter().all(|id| is_baked(&session, *id)));
    assert_settled(&session);

    Ok(())
}

#[test]
fn choosing_a_variant_changes_the_view_and_the_bake_without_editing() -> TestResult {
    let fixture = Fixture::new(&[(2, "outer")])?;
    let mut session = fixture.session()?;
    let id = session.state.active().ok_or("no active document")?;
    let undo_label = session.undo_label().map(str::to_owned);
    let before = preview_ids(&session);
    let key = session.module_roots()[0].key.clone();

    session.choose_module_variant(&key, 1);

    assert!(has(&session, 2, MODULE));
    assert!(!has(&session, 2, WALL));
    assert!(!has(&session, 3, WALL));
    assert_eq!(session.module_roots().len(), 1);
    assert_eq!(session.module_roots()[0].selected, Some(1));
    assert!(before.iter().all(|id| !is_baked(&session, *id)));
    assert!(preview_ids(&session).iter().all(|id| is_baked(&session, *id)));
    assert!(!session.state.document(id).ok_or("document is missing")?.is_dirty());
    assert_eq!(session.undo_label().map(str::to_owned), undo_label);
    assert_settled(&session);

    session.choose_module_variant(&key, 0);

    assert!(has(&session, 2, WALL));
    assert!(has(&session, 3, WALL));
    assert_eq!(session.module_roots().len(), 2);
    assert!(preview_ids(&session).iter().all(|id| is_baked(&session, *id)));
    assert!(!session.state.document(id).ok_or("document is missing")?.is_dirty());
    assert_settled(&session);

    Ok(())
}

#[test]
fn toggling_previews_off_restores_the_source_and_on_recomposes() -> TestResult {
    let fixture = Fixture::new(&[(2, "outer")])?;
    let mut session = fixture.session()?;
    let previews = preview_ids(&session);

    session.set_modules_enabled(false);

    assert!(!session.modules_enabled());
    assert_eq!(effective_paths(&session, 2), source_paths(&session, 2));
    assert!(preview_ids(&session).is_empty());
    assert!(previews.iter().all(|id| !is_baked(&session, *id)));
    assert_settled(&session);

    session.set_modules_enabled(true);

    assert!(session.modules_enabled());
    assert!(has(&session, 3, WALL));
    assert_eq!(preview_ids(&session).len(), previews.len());
    assert!(preview_ids(&session).iter().all(|id| is_baked(&session, *id)));
    assert_settled(&session);

    Ok(())
}

#[test]
fn the_profile_checkbox_toggles_previews() -> TestResult {
    let fixture = Fixture::new(&[(2, "outer")])?;
    let mut session = fixture.session()?;
    let toggle = |session: &mut Session, is_shown: bool| {
        session.dm_ui(
            0,
            editor::bake::UiFeedback {
                values: [(
                    "Panel/Preview modules".to_owned(),
                    editor::bake::UiValue::Bool(is_shown),
                )]
                .into(),
                modular_preview: session.modules_enabled(),
                interacted: true,
                ..Default::default()
            },
        )
    };
    assert!(session.modules_enabled());
    assert!(!session.can_edit_at(coord(2)));

    let quiet = session.dm_ui(
        0,
        editor::bake::UiFeedback {
            values: [("Panel/Preview modules".to_owned(), editor::bake::UiValue::Bool(false))].into(),
            modular_preview: true,
            ..Default::default()
        },
    );
    assert!(!quiet.committed);
    assert!(session.modules_enabled(), "a frame the editor drops changes nothing");

    let hidden = toggle(&mut session, false);
    assert!(hidden.committed);
    assert!(!session.modules_enabled());
    assert!(session.can_edit_at(coord(2)));
    assert!(preview_ids(&session).is_empty());

    let shown = toggle(&mut session, true);
    assert!(shown.committed);
    assert!(session.modules_enabled());
    assert!(!session.can_edit_at(coord(2)));
    assert!(has(&session, 3, WALL));

    Ok(())
}

#[test]
fn covered_tiles_reject_edits_but_undo_still_applies() -> TestResult {
    let fixture = Fixture::new(&[(2, "outer")])?;
    let mut session = fixture.session()?;
    session.set_modules_enabled(false);
    assert!(session.can_edit_at(coord(3)));

    session.set_tool(Tool::Place);
    session.state.choose_prefab(prefab("/obj/structure/table"));
    session.place_at(coord(3), None).ok_or("placement was rejected")?;
    assert!(source_paths(&session, 3).contains(&String::from("/obj/structure/table")));

    session.set_modules_enabled(true);
    assert!(!session.can_edit_at(coord(2)));
    assert!(!session.can_edit_at(coord(3)));
    assert!(session.can_edit_at(coord(4)));
    assert!(session.place_at(coord(3), None).is_none());

    assert!(session.undo());
    assert!(!source_paths(&session, 3).contains(&String::from("/obj/structure/table")));
    assert_settled(&session);

    session.set_modules_enabled(false);
    assert!(session.can_edit_at(coord(2)));
    assert!(session.can_edit_at(coord(3)));

    Ok(())
}

#[test]
fn a_source_root_stays_editable_under_its_own_module() -> TestResult {
    let fixture = Fixture::new(&[(2, "outer")])?;
    let mut session = fixture.session()?;
    let root = root_at(&session, 2);

    assert!(!session.can_edit_at(coord(2)));
    assert!(session.can_edit_instance(root));

    session.set_modules_enabled(false);
    assert!(session.can_edit_instance(root));

    Ok(())
}

#[test]
fn moving_a_selected_root_moves_its_module() -> TestResult {
    let fixture = Fixture::new(&[(2, "outer")])?;
    let mut session = fixture.session()?;
    let root = root_at(&session, 2);
    session.select_instance(Some(root));

    assert_eq!(session.move_selected_instance(coord(3), "move", &[], None), Some(true));
    assert!(session.can_edit_instance(root));
    assert_eq!(session.module_roots()[0].coord, coord(3));
    assert!(has(&session, 3, ROOT));
    assert!(has(&session, 3, WALL));
    assert!(has(&session, 4, WALL));
    assert!(!has(&session, 2, WALL));
    assert!(preview_ids(&session).iter().all(|id| is_baked(&session, *id)));
    assert_settled(&session);

    assert_eq!(session.move_selected_instance(coord(6), "move", &[], None), Some(true));
    assert!(has(&session, 6, ROOT));
    assert!(!has(&session, 3, WALL));
    assert_settled(&session);

    Ok(())
}

#[test]
fn a_deferred_move_shows_the_module_without_baking_it_until_flushed() -> TestResult {
    let fixture = Fixture::new(&[(2, "outer")])?;
    let mut session = fixture.session()?;
    let root = root_at(&session, 2);
    let previews = preview_ids(&session);
    let before = previews
        .iter()
        .map(|id| baked_position(&session, *id))
        .collect::<Vec<_>>();
    session.select_instance(Some(root));

    session.set_module_bake_deferred(true);
    assert!(is_deferred(&session));

    assert_eq!(session.move_selected_instance(coord(3), "move", &[], None), Some(true));
    assert!(has(&session, 3, ROOT));
    assert!(has(&session, 3, WALL));
    assert!(has(&session, 4, WALL));
    assert!(!has(&session, 2, WALL));
    assert_eq!(preview_ids(&session), previews);
    assert!(previews.iter().all(|id| is_baked(&session, *id)));
    assert_eq!(
        previews
            .iter()
            .map(|id| baked_position(&session, *id))
            .collect::<Vec<_>>(),
        before
    );
    assert!(
        session
            .instances()
            .ok_or("no instances")?
            .sprites
            .iter()
            .any(|sprite| previews.contains(&sprite.owner))
    );
    assert_eq!(session.module_roots()[0].coord, coord(3));
    assert!(
        session
            .state
            .document(session.state.active().ok_or("no active document")?)
            .ok_or("document is missing")?
            .is_dirty()
    );

    session.set_module_bake_deferred(false);

    assert!(!is_deferred(&session));
    assert!(
        previews
            .iter()
            .map(|id| baked_position(&session, *id))
            .ne(before.iter().copied())
    );
    assert!(preview_ids(&session).iter().all(|id| is_baked(&session, *id)));
    assert_settled(&session);

    session.set_module_bake_deferred(false);
    assert_settled(&session);

    Ok(())
}

#[test]
fn a_variant_switch_while_deferred_settles_on_flush() -> TestResult {
    let fixture = Fixture::new(&[(2, "outer")])?;
    let mut session = fixture.session()?;
    let key = session.module_roots()[0].key.clone();

    session.set_module_bake_deferred(true);
    session.choose_module_variant(&key, 1);
    assert!(has(&session, 2, MODULE));
    assert!(!has(&session, 2, WALL));

    session.set_module_bake_deferred(false);

    assert!(preview_ids(&session).iter().all(|id| is_baked(&session, *id)));
    assert_settled(&session);

    session.set_module_bake_deferred(true);
    session.choose_module_variant(&key, 0);
    session.set_module_bake_deferred(false);

    assert!(has(&session, 3, WALL));
    assert_eq!(session.module_roots().len(), 2);
    assert!(preview_ids(&session).iter().all(|id| is_baked(&session, *id)));
    assert_settled(&session);

    Ok(())
}

#[test]
fn a_rebake_while_deferred_drops_the_pending_ids() -> TestResult {
    let fixture = Fixture::new(&[(2, "outer")])?;
    let mut session = fixture.session()?;
    let root = root_at(&session, 2);
    session.select_instance(Some(root));
    session.set_module_bake_deferred(true);
    assert_eq!(session.move_selected_instance(coord(3), "move", &[], None), Some(true));

    let id = session.state.active().ok_or("no active document")?;
    session.rebake(id);
    settle_bake(&mut session);

    assert!(!is_deferred(&session));
    assert!(has(&session, 3, WALL));
    assert!(preview_ids(&session).iter().all(|id| is_baked(&session, *id)));
    assert_settled(&session);

    Ok(())
}

#[test]
fn deleting_a_root_removes_its_module() -> TestResult {
    let fixture = Fixture::new(&[(2, "outer")])?;
    let mut session = fixture.session()?;
    let root = root_at(&session, 2);

    assert!(session.delete_context_instance(root));
    assert!(session.module_roots().is_empty());
    assert!(preview_ids(&session).is_empty());
    assert!(!has(&session, 2, ROOT));
    assert!(!has(&session, 3, WALL));
    assert_settled(&session);

    assert!(session.undo());
    assert!(has(&session, 3, WALL));
    assert_settled(&session);

    Ok(())
}

#[test]
fn editing_a_root_var_switches_its_module() -> TestResult {
    let fixture = Fixture::new(&[(2, "outer")])?;
    let mut session = fixture.session()?;
    session.set_modules_enabled(false);
    let root = root_at(&session, 2);

    session.select_instance(Some(root));
    assert_eq!(
        session.set_selected_instance_var("key".into(), Value::Text("alt".into())),
        Some(true)
    );

    assert_eq!(session.module_roots().len(), 1);
    assert_eq!(session.module_roots()[0].variants.len(), 1);
    assert!(session.module_roots()[0].variants[0].choice.path.ends_with("b.dmm"));
    assert_settled(&session);

    session.set_modules_enabled(true);

    assert!(has(&session, 2, MODULE));
    assert!(!has(&session, 3, WALL));
    assert!(preview_ids(&session).iter().all(|id| is_baked(&session, *id)));
    assert_settled(&session);

    Ok(())
}

#[test]
fn remote_edits_to_a_covered_root_recompose_its_module() -> TestResult {
    let fixture = Fixture::new(&[(2, "outer")])?;
    let mut session = fixture.session()?;
    let id = session.state.active().ok_or("no active document")?;

    assert!(session.apply_remote_tiles(
        id,
        vec![(
            coord(2),
            vec![keyed("alt"), prefab("/turf/open/floor"), prefab("/area/station")]
        )]
    ));

    assert!(has(&session, 2, MODULE));
    assert!(!has(&session, 3, WALL));
    assert_eq!(session.module_roots().len(), 1);
    assert!(preview_ids(&session).iter().all(|id| is_baked(&session, *id)));
    assert_settled(&session);

    Ok(())
}

#[test]
fn a_placement_error_belongs_to_its_root_only() -> TestResult {
    let fixture = Fixture::new(&[(2, "outer"), (5, "missing")])?;
    let session = fixture.session()?;
    let roots = session.module_roots();
    let healthy = roots.iter().find(|root| root.coord == coord(2)).ok_or("missing root")?;
    let broken = roots.iter().find(|root| root.coord == coord(5)).ok_or("missing root")?;

    assert!(healthy.error.is_none());
    assert_eq!(healthy.selected, Some(0));
    assert!(matches!(
        broken.error.as_ref().map(|error| &error.kind),
        Some(ErrorKind::Hook(text)) if text == "no room missing in config.toml"
    ));
    assert!(broken.selected.is_none());
    assert!(has(&session, 3, WALL));
    assert!(!has(&session, 5, WALL));
    assert_settled(&session);

    Ok(())
}

#[test]
fn an_unreadable_config_is_an_error_on_its_roots_only() -> TestResult {
    let fixture = Fixture::new(&[])?;
    let mut unreadable = keyed("outer");
    unreadable.set_var("config_file".into(), Value::Text("absent.toml".into()));
    fixture.write_main(&[(2, keyed("outer")), (5, unreadable)])?;
    let session = fixture.session()?;
    let roots = session.module_roots();
    let healthy = roots.iter().find(|root| root.coord == coord(2)).ok_or("missing root")?;
    let broken = roots.iter().find(|root| root.coord == coord(5)).ok_or("missing root")?;

    assert!(healthy.error.is_none());
    assert!(matches!(
        broken.error.as_ref().map(|error| &error.kind),
        Some(ErrorKind::Hook(text)) if text == "cannot read absent.toml"
    ));
    assert!(has(&session, 3, WALL));
    assert!(!has(&session, 5, WALL));
    assert_settled(&session);

    Ok(())
}

#[test]
fn an_invalid_config_is_an_error_for_its_roots_only() -> TestResult {
    let fixture = Fixture::new(&[])?;
    fs::write(fixture.dir.join("bad.toml"), "rooms = [\n")?;
    let mut invalid = keyed("outer");
    invalid.set_var("config_file".into(), Value::Text("bad.toml".into()));
    fixture.write_main(&[(2, invalid), (5, keyed("outer"))])?;
    let session = fixture.session()?;
    let roots = session.module_roots();

    let failed = roots
        .iter()
        .filter(|root| {
            matches!(
                root.error.as_ref().map(|error| &error.kind),
                Some(ErrorKind::Hook(text)) if text.contains("bad.toml") && text.contains("unclosed array")
            )
        })
        .count();
    assert_eq!(failed, 1);
    assert!(roots.iter().any(|root| root.error.is_none() && root.selected.is_some()));

    Ok(())
}

#[test]
fn saving_writes_the_source_without_preview_atoms() -> TestResult {
    let fixture = Fixture::new(&[(2, "outer")])?;
    let mut session = fixture.session()?;
    let original = fs::read(fixture.dir.join("main.dmm"))?;
    let key = session.module_roots()[0].key.clone();
    session.choose_module_variant(&key, 1);

    let target = fixture.dir.join("saved.dmm");
    session.save_map_as(&target, MapFormat::Standard)?;

    let saved = fs::read(&target)?;
    assert_eq!(saved, original);
    let text = String::from_utf8(saved)?;
    assert!(!text.contains(MODULE));
    assert!(!text.contains(WALL));

    Ok(())
}

#[test]
fn choices_survive_a_rebake() -> TestResult {
    let fixture = Fixture::new(&[(2, "outer")])?;
    let mut session = fixture.session()?;
    let id = session.state.active().ok_or("no active document")?;
    let key = session.module_roots()[0].key.clone();
    session.choose_module_variant(&key, 1);

    session.rebake(id);
    settle_bake(&mut session);

    assert_eq!(session.module_roots()[0].selected, Some(1));
    assert!(has(&session, 2, MODULE));
    assert!(!has(&session, 3, WALL));
    assert_settled(&session);

    Ok(())
}

#[test]
fn toggling_previews_off_while_baking_lands_the_source_view() -> TestResult {
    let fixture = Fixture::new(&[(2, "outer")])?;
    let mut session = fixture.session()?;
    let id = session.state.active().ok_or("no active document")?;

    session.rebake(id);
    session.set_modules_enabled(false);
    settle_bake(&mut session);

    assert!(!session.modules_enabled());
    assert_eq!(effective_paths(&session, 2), source_paths(&session, 2));
    assert!(preview_ids(&session).is_empty());
    assert!(session.can_edit_at(coord(2)));
    assert_settled(&session);

    session.set_modules_enabled(true);

    assert!(has(&session, 3, WALL));
    assert_settled(&session);

    Ok(())
}

#[test]
#[ignore = "requires the local target/tgstation checkout"]
fn tramstation_loads_its_modules_through_the_tgstation_profile() -> TestResult {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/tgstation");
    let path = root.join("_maps/map_files/tramstation/tramstation.dmm");
    let (map, errors) = dmm::parser::load(&path)?;
    assert!(errors.is_empty(), "{errors:?}");

    let options = BakeOptions {
        forced_profile: Some(String::from("tgstation")),
        ..Default::default()
    };
    let loaded = load_codebase(&root.join("tgstation.dme"), &options, &Progress::new())?;
    assert!(loaded.diagnostics.profile.is_none(), "{:?}", loaded.diagnostics.profile);

    let mut session = Session::new();
    session.apply_codebase(loaded);
    session.activate_document(MapDocument::open(path, map, 1));
    settle_bake(&mut session);

    let modules = &session.active_cache().modules;
    let preview = modules.preview.as_ref().ok_or("no module preview")?;
    assert!(preview.diagnostics.is_empty(), "{:?}", preview.diagnostics);

    let medsci = preview
        .roots
        .iter()
        .find(|root| {
            root.depth == 0
                && root
                    .variants
                    .first()
                    .is_some_and(|variant| variant.choice.path.ends_with("medsciupper_1.dmm"))
        })
        .ok_or("medsciupper root")?;
    assert_eq!(medsci.variants.len(), 3);
    assert!(medsci.variants.iter().all(|variant| variant.error.is_none()));

    let attachments = preview
        .roots
        .iter()
        .filter(|root| root.key.instance == medsci.key.instance && root.depth == 1)
        .count();
    assert_eq!(attachments, 2);

    Ok(())
}

#[test]
#[ignore = "requires the local target/TerraGov-Marine-Corps checkout"]
fn lv624_loads_its_modules_through_the_tgmc_profile() -> TestResult {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/TerraGov-Marine-Corps");
    let path = root.join("_maps/map_files/LV624/LV624.dmm");
    let (map, errors) = dmm::parser::load(&path)?;
    assert!(errors.is_empty(), "{errors:?}");

    let options = BakeOptions {
        forced_profile: Some(String::from("tgmc")),
        ..Default::default()
    };
    let loaded = load_codebase(&root.join("tgmc.dme"), &options, &Progress::new())?;
    assert!(loaded.diagnostics.profile.is_none(), "{:?}", loaded.diagnostics.profile);

    let mut session = Session::new();
    session.apply_codebase(loaded);
    session.activate_document(MapDocument::open(path, map, 1));
    settle_bake(&mut session);

    let modules = &session.active_cache().modules;
    let preview = modules.preview.as_ref().ok_or("no module preview")?;
    assert!(preview.diagnostics.is_empty(), "{:?}", preview.diagnostics);
    assert!(!preview.roots.is_empty());
    for root in &preview.roots {
        eprintln!(
            "root at {:?}: {} variants, selected {:?}, error {:?}",
            root.coord,
            root.variants.len(),
            root.selected,
            root.error
        );
        assert!(root.error.is_none(), "{:?}", root.error);
        assert!(root.selected.is_some());
    }

    Ok(())
}

#[test]
#[ignore = "timing harness for moving tramstation roots"]
fn tramstation_root_move_timing() -> TestResult {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/tgstation");
    let path = root.join("_maps/map_files/tramstation/tramstation.dmm");
    let (map, _) = dmm::parser::load(&path)?;
    let options = BakeOptions {
        forced_profile: Some(String::from("tgstation")),
        ..Default::default()
    };
    let loaded = load_codebase(&root.join("tgstation.dme"), &options, &Progress::new())?;
    let mut session = Session::new();
    session.apply_codebase(loaded);
    session.activate_document(MapDocument::open(path, map, 1));
    settle_bake(&mut session);

    let roots = session
        .module_roots()
        .iter()
        .filter(|root| root.key.ancestry.is_empty())
        .map(|root| (root.key.instance, root.coord))
        .collect::<Vec<_>>();
    for (instance, coord) in roots {
        session.select_instance(Some(instance));
        let next = Coord::new(coord.x + 1, coord.y, coord.z);
        let started = Instant::now();
        session.move_selected_instance(next, "move", &[], None);
        let moved = started.elapsed();
        let started = Instant::now();
        session.move_selected_instance(coord, "move", &[], None);
        let returned = started.elapsed();

        session.set_module_bake_deferred(true);
        let started = Instant::now();
        session.move_selected_instance(next, "move", &[], None);
        let deferred_moved = started.elapsed();
        let started = Instant::now();
        session.move_selected_instance(coord, "move", &[], None);
        let deferred_returned = started.elapsed();
        let started = Instant::now();
        session.set_module_bake_deferred(false);
        let flushed = started.elapsed();

        eprintln!(
            "root {} at {:?}: live {moved:?} / {returned:?}, deferred {deferred_moved:?} / {deferred_returned:?}, \
             flush {flushed:?}",
            instance.get(),
            coord,
        );
    }

    Ok(())
}

fn open_document(session: &mut Session, fixture: &Fixture, name: &str) -> TestResult<DocumentId> {
    session.open_map(&fixture.dir.join(name), 1)?;
    settle_bake(session);

    Ok(session.state.active().ok_or("no active document")?)
}

fn place_in(session: &mut Session, id: DocumentId, x: u32, path: &str) -> TestResult {
    session.set_active_document(id);
    session.set_tool(Tool::Place);
    session.state.choose_prefab(prefab(path));
    session.place_at(coord(x), None).ok_or("nothing was placed")?;

    Ok(())
}

fn show(session: &mut Session, id: DocumentId) {
    session.set_active_document(id);
    session.sync_open_modules();
}

#[test]
fn edits_in_an_open_module_show_up_in_maps_that_place_it() -> TestResult {
    let fixture = Fixture::new(&[(2, "outer")])?;
    let mut session = fixture.session()?;
    let main = session.state.active().ok_or("no active document")?;
    let module = open_document(&mut session, &fixture, "a.dmm")?;

    place_in(&mut session, module, 1, MODULE)?;
    show(&mut session, main);

    assert!(has(&session, 2, MODULE));
    assert!(has(&session, 3, WALL), "the nested root still composes");
    assert!(preview_ids(&session).iter().all(|id| is_baked(&session, *id)));
    assert_settled(&session);

    session.set_active_document(module);
    assert!(session.undo());
    show(&mut session, main);

    assert!(!has(&session, 2, MODULE));
    assert!(has(&session, 2, WALL));
    assert_settled(&session);

    Ok(())
}

#[test]
fn closing_an_unsaved_module_reverts_maps_to_the_file_on_disk() -> TestResult {
    let fixture = Fixture::new(&[(2, "outer")])?;
    let mut session = fixture.session()?;
    let main = session.state.active().ok_or("no active document")?;
    let module = open_document(&mut session, &fixture, "a.dmm")?;
    place_in(&mut session, module, 1, MODULE)?;
    show(&mut session, main);
    assert!(has(&session, 2, MODULE));

    assert!(session.close_map(module));
    show(&mut session, main);

    assert!(!has(&session, 2, MODULE));
    assert!(has(&session, 2, WALL));
    assert!(preview_ids(&session).iter().all(|id| is_baked(&session, *id)));
    assert_settled(&session);

    Ok(())
}

#[test]
fn live_module_edits_survive_a_rebake_of_the_containing_map() -> TestResult {
    let fixture = Fixture::new(&[(2, "outer")])?;
    let mut session = fixture.session()?;
    let main = session.state.active().ok_or("no active document")?;
    let module = open_document(&mut session, &fixture, "a.dmm")?;
    place_in(&mut session, module, 1, MODULE)?;
    show(&mut session, main);

    session.rebake(main);
    settle_bake(&mut session);

    assert!(has(&session, 2, MODULE));
    assert!(preview_ids(&session).iter().all(|id| is_baked(&session, *id)));
    assert_settled(&session);

    Ok(())
}

#[test]
fn edits_in_a_nested_module_show_up_through_its_parent_module() -> TestResult {
    let fixture = Fixture::new(&[(2, "outer")])?;
    let mut session = fixture.session()?;
    let main = session.state.active().ok_or("no active document")?;
    let child = open_document(&mut session, &fixture, "child.dmm")?;

    place_in(&mut session, child, 1, MODULE)?;
    show(&mut session, main);

    assert!(has(&session, 3, MODULE));
    assert!(!has(&session, 2, MODULE));
    assert!(preview_ids(&session).iter().all(|id| is_baked(&session, *id)));
    assert_settled(&session);

    Ok(())
}

#[test]
fn a_map_opened_after_the_edit_shows_the_live_module() -> TestResult {
    let fixture = Fixture::new(&[(2, "outer")])?;
    let mut session = fixture.session()?;
    let main = session.state.active().ok_or("no active document")?;
    let module = open_document(&mut session, &fixture, "a.dmm")?;
    place_in(&mut session, module, 1, MODULE)?;
    assert!(session.close_map(main));

    let main = open_document(&mut session, &fixture, "main.dmm")?;
    assert!(!has(&session, 2, MODULE), "the bake starts from the file on disk");
    show(&mut session, main);

    assert!(has(&session, 2, MODULE));
    assert!(preview_ids(&session).iter().all(|id| is_baked(&session, *id)));
    assert_settled(&session);

    Ok(())
}
