use base::{path::TreePath, types::Value};
use std::path::PathBuf;

use dear_imgui_rs::Key;
use dmm::{Map, Size};
use objtree::TypeId;
use prefab::Prefab;
use ui::testing::CONTEXT_LOCK;

use super::{
    CommandPalette,
    matcher::{Haystack, Query, score},
};
use crate::{environment::worker::LoadedMap, frontend::fixtures::RectangleUiHarness, session::Session, tool::Tool};

fn path_score(query: &str, path: &str) -> Option<u32> {
    score(&Query::parse(query), &Haystack::new(path, None, None))
}

#[test]
fn tokens_score_by_where_they_land_and_all_must_match() {
    assert!(
        path_score("airlock", "/obj/machinery/door/airlock")
            > path_score("airlock", "/obj/machinery/door/airlock/engineering")
    );
    assert_eq!(
        path_score("airlock eng", "/obj/machinery/door/airlock/engineering"),
        Some(60 + 100)
    );
    assert_eq!(
        path_score("airlock eng", "/obj/machinery/door/airlock/glass_engineering"),
        Some(60 + 80)
    );
    assert_eq!(path_score("airlock eng", "/obj/machinery/door/airlock"), None);
    assert_eq!(path_score("mchnry", "/obj/machinery"), Some(10));
}

#[test]
fn names_and_overrides_match_when_the_path_does_not() {
    let haystack = Haystack::new("/obj/item/foo", Some("Welding Tool"), Some("desc = \"Rusty\""));

    assert_eq!(score(&Query::parse("weld"), &haystack), Some(70));
    assert_eq!(score(&Query::parse("rusty"), &haystack), Some(30));
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
    let mut palette = CommandPalette::new(session);
    palette.rank(query);
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
    assert_eq!(palette.choice(0), Some(rusty));
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
