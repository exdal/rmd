use base::{coord::Coord, path::TreePath, types::Value};
use std::{collections::HashSet, path::PathBuf};

use dear_imgui_rs::Key;
use dmm::{Map, Size};
use objtree::TypeId;
use prefab::Prefab;
use ui::testing::CONTEXT_LOCK;

use super::{Choice, CommandPalette, Mode, command::Command, representative_tile};
use crate::{
    environment::worker::LoadedMap, frontend::fixtures::RectangleUiHarness, session::Session, settings::Settings,
    tool::Tool,
};

#[test]
fn representative_tile_is_on_the_busiest_level_nearest_its_center() {
    let coords =
        [(1, 1, 2), (1, 1, 1), (2, 1, 1), (3, 1, 1), (4, 1, 1), (9, 1, 1)].map(|(x, y, z)| Coord::new(x, y, z));

    assert_eq!(representative_tile(&coords), Some(4));
}

fn prefab(path: &str) -> Prefab {
    Prefab::new(TreePath::parse(path))
}

fn edited(path: &str, name: &str, value: Value) -> Prefab {
    let mut prefab = prefab(path);
    prefab.set_var(name.into(), value);

    prefab
}

fn session_with(tiles: Vec<Vec<Prefab>>) -> Session {
    let mut map = Map::new(Size {
        x: tiles.len() as u32,
        y: 1,
        z: 1,
    });
    for (x, tile) in tiles.into_iter().enumerate() {
        map.grid[0][0][x] = map.intern_tile(tile);
    }

    let mut session = Session::new();
    session
        .load_environment(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env/test.dme"))
        .unwrap();
    session.apply_map(LoadedMap {
        path: PathBuf::from("command-palette-test.dmm"),
        map,
        z: 1,
        errors: vec![],
        repo: None,
        conflict: None,
    });

    session
}

fn prefabs(session: &Session, query: &str) -> (CommandPalette, Vec<(String, u32)>) {
    let mut palette = CommandPalette::new(session, &Settings::default());
    palette.rank(query);
    assert_eq!(palette.mode, Mode::Prefabs);
    let listed = palette
        .results
        .iter()
        .map(|&index| (palette.prefabs[index].path.clone(), palette.prefabs[index].count))
        .collect::<Vec<_>>();

    (palette, listed)
}

fn type_id(session: &Session, path: &str) -> TypeId {
    session.tree().unwrap().id_of(&TreePath::parse(path)).unwrap()
}

#[test]
fn an_empty_query_lists_recents_first_then_used_prefabs_by_count() {
    let floor = prefab("/turf/open/floor");
    let station = prefab("/area/station");
    let mut session = session_with(vec![
        vec![floor.clone(), station.clone(), prefab("/obj/structure/table")],
        vec![floor.clone(), station.clone()],
        vec![prefab("/turf/closed/wall"), station.clone()],
        vec![floor, station],
    ]);
    session.choose_type(type_id(&session, "/obj/structure/table"));
    session.choose_type(type_id(&session, "/turf/closed/wall"));

    let (_, listed) = prefabs(&session, "");

    assert_eq!(
        listed,
        [
            ("/turf/closed/wall".to_owned(), 1),
            ("/obj/structure/table".to_owned(), 1),
            ("/area/station".to_owned(), 4),
            ("/turf/open/floor".to_owned(), 3),
        ]
    );
}

#[test]
fn usage_count_breaks_score_ties() {
    let wall = prefab("/turf/closed/wall");
    let session = session_with(vec![vec![prefab("/turf/open/floor")], vec![wall.clone()], vec![wall]]);

    let (_, listed) = prefabs(&session, "turf");
    let position = |path: &str| listed.iter().position(|(listed, _)| listed == path).unwrap();

    assert!(position("/turf/closed/wall") < position("/turf/open/floor"));
}

#[test]
fn var_edited_map_prefabs_are_listed_with_their_own_counts() {
    let rusty = edited("/obj/structure/table", "name", Value::Text("Rusty".into()));
    let session = session_with(vec![
        vec![rusty.clone()],
        vec![rusty.clone()],
        vec![prefab("/obj/structure/table")],
    ]);

    let (palette, listed) = prefabs(&session, "rusty");

    assert_eq!(listed, [("/obj/structure/table".to_owned(), 2)]);
    assert_eq!(palette.choice(0), Some(Choice::Prefab(rusty)));
}

#[test]
fn every_command_has_a_unique_label() {
    let labels = Command::all().into_iter().map(Command::label).collect::<Vec<_>>();
    let unique = labels.iter().collect::<HashSet<_>>();

    assert!(labels.iter().all(|label| !label.is_empty()), "{labels:?}");
    assert_eq!(unique.len(), labels.len(), "{labels:?}");
}

fn press_ctrl_p(app: &mut RectangleUiHarness) {
    app.key(Key::ModCtrl, true);
    app.key(Key::P, true);
    app.key(Key::P, false);
    app.key(Key::ModCtrl, false);
    app.step();
}

fn open(app: &mut RectangleUiHarness) {
    press_ctrl_p(app);
    app.step();
}

fn type_text(app: &mut RectangleUiHarness, text: &str) {
    for character in text.chars() {
        app.context.io_mut().add_input_character(character);
    }
    app.step();
    app.step();
}

fn tap(app: &mut RectangleUiHarness, key: Key) {
    app.key(key, true);
    app.key(key, false);
}

#[test]
fn ctrl_p_toggles_the_palette() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut app = RectangleUiHarness::new();

    open(&mut app);
    assert!(app.state.command_palette.is_some());

    press_ctrl_p(&mut app);
    assert!(app.state.command_palette.is_none());
}

#[test]
fn choosing_a_prefab_sets_it_and_switches_to_place() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut app = RectangleUiHarness::new();
    open(&mut app);
    type_text(&mut app, "floor");

    tap(&mut app, Key::Enter);

    assert_eq!(app.session.palette(), Some(&prefab("/turf/open/floor")));
    assert_eq!(app.session.tool(), Tool::Place);
    assert!(app.state.command_palette.is_none());
}

#[test]
fn an_action_runs_through_the_frame_menu_actions() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut app = RectangleUiHarness::new();
    let before = app.session.options.show_lighting;
    open(&mut app);
    type_text(&mut app, ">lighting");

    tap(&mut app, Key::Enter);

    assert_ne!(app.session.options.show_lighting, before);
}

#[test]
fn choosing_an_area_centers_the_view_and_leaves_the_selection_alone() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut app = RectangleUiHarness::new();
    let turf = app.session.turf_at(app.id, Coord::new(1, 1, 1)).unwrap();
    app.session
        .state
        .document_mut(app.id)
        .unwrap()
        .pick_instance(Some(turf));
    open(&mut app);
    type_text(&mut app, "@");

    tap(&mut app, Key::Enter);

    let document = app.session.state.document(app.id).unwrap();
    assert_eq!(document.picked_instance(), Some(turf));
    let view = app.state.map_views.get(&app.id).unwrap();
    let tile_size = app.session.options.tile_size as f32;
    assert_eq!(
        (view.camera.camera.x, view.camera.camera.y),
        (9.5 * tile_size, 9.5 * tile_size)
    );
}

#[test]
fn typing_still_reaches_the_input_after_enter_finds_nothing() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let mut app = RectangleUiHarness::new();
    open(&mut app);
    type_text(&mut app, "zzzz");
    tap(&mut app, Key::Enter);
    app.step();

    type_text(&mut app, "turf");

    let palette = app.state.command_palette.as_ref().unwrap();
    assert!(palette.query.ends_with("turf"), "{:?}", palette.query);
}

#[test]
fn open_map_takes_map_names_as_arguments() {
    let _guard = CONTEXT_LOCK.lock().unwrap();
    let app = RectangleUiHarness::new();
    let map = |name: &str| {
        app.session
            .maps()
            .iter()
            .find(|path| path.file_name().is_some_and(|file| file == name))
            .unwrap()
            .clone()
    };
    let mut palette = CommandPalette::new(&app.session, &app.settings);

    palette.rank(">open map test2 test");
    assert_eq!(
        palette.choice(0),
        Some(Choice::Maps(vec![map("test2.dmm"), map("test.dmm")]))
    );

    palette.rank(">open map");
    assert_eq!(palette.choice(0), Some(Choice::Command(Command::OpenMap)));

    palette.rank(">open map zzzz");
    assert!(palette.results.is_empty());
}

#[test]
fn open_codebase_takes_one_recent_codebase_as_argument() {
    let tg = PathBuf::from("/repos/tgstation/tgstation.dme");
    let settings = Settings {
        recent_codebases: vec![PathBuf::from("/repos/other/other.dme"), tg.clone()],
        ..Settings::default()
    };
    let mut palette = CommandPalette::new(&Session::new(), &settings);

    palette.rank(">open tg");
    assert_eq!(palette.choice(0), Some(Choice::Codebase(tg)));

    palette.rank(">open tg other");
    assert!(palette.results.is_empty());
}
