use super::*;

#[test]
fn clicking_a_blamed_tile_opens_a_popup_without_placing_a_tile() {
    struct OneVersion {
        commits: Vec<editor::git::CommitInfo>,
        map: dmm::Map,
    }
    impl editor::blame::VersionSource for OneVersion {
        fn commits(&self) -> &[editor::git::CommitInfo] { &self.commits }

        fn truncated(&self) -> bool { false }

        fn map_at(&mut self, _: usize) -> Result<editor::blame::MapVersion, editor::git::GitError> {
            Ok(editor::blame::MapVersion::Present(self.map.clone()))
        }
    }

    let _guard = IMGUI_CONTEXT.lock().unwrap();
    let mut session = Session::new();
    session
        .load_environment(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env/test.dme"))
        .unwrap();
    let mut map = dmm::Map::new(Size { x: 20, y: 20, z: 1 });
    let floor = Prefab::new(TreePath::parse("/turf/open/floor"));
    let key = map.intern_tile(vec![floor.clone()]);
    for row in &mut map.grid[0] {
        row.fill(key);
    }
    let mut source = OneVersion {
        commits: vec![editor::git::CommitInfo {
            hash: String::from("a").repeat(40),
            short: String::from("aaaaaaa"),
            author: String::from("Map author"),
            time: 1,
            summary: String::from("Paint the floor"),
            pull_request: None,
        }],
        map: map.clone(),
    };
    let blame = editor::blame::blame(&map, &mut source, &|| false, &mut |_, _| {}).unwrap();
    session.apply_map(crate::loader::LoadedMap {
        path: PathBuf::from("blame-ui-test.dmm"),
        map,
        z: 1,
        errors: vec![],
        repo: Some(editor::git::RepoPath {
            root: PathBuf::from("."),
            git_dir: PathBuf::from(".git"),
            rel: String::from("blame-ui-test.dmm"),
        }),
        conflict: None,
    });
    let id = session.state.active().unwrap();
    install_blame(&mut session, id, blame);
    session.set_tool(Tool::Place);
    session
        .state
        .choose_prefab(Prefab::new(TreePath::parse("/turf/closed/wall")));

    let mut app = RectangleUiHarness::with_session(session);
    let tile = app.tile(5, 8);
    app.pointer(tile, false);
    assert!(app.view.blame_popup.is_none(), "hovering only shows the normal tooltip");
    app.click(tile);

    let popup = app.view.blame_popup.as_ref().expect("clickable popup");
    assert_eq!(popup.coord, Coord::new(5, 8, 1));
    assert_eq!(app.session.undo_label(), None);
    assert_eq!(
        app.session.map().unwrap().tile_at(Coord::new(5, 8, 1)).unwrap()[0].path,
        floor.path
    );
}

#[test]
fn conflict_button_takes_the_incoming_tile_without_reaching_the_place_tool() {
    let _guard = IMGUI_CONTEXT.lock().unwrap();
    let mut session = Session::new();
    session
        .load_environment(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env/test.dme"))
        .unwrap();
    let mut map = dmm::Map::new(Size { x: 20, y: 20, z: 1 });
    let floor = Prefab::new(TreePath::parse("/turf/open/floor"));
    let key = map.intern_tile(vec![floor.clone()]);
    for row in &mut map.grid[0] {
        row.fill(key);
    }
    let coord = Coord::new(5, 8, 1);
    let incoming = Prefab::new(TreePath::parse("/turf/closed/wall"));
    session.apply_map(crate::loader::LoadedMap {
        path: PathBuf::from("conflict-ui-test.dmm"),
        map,
        z: 1,
        errors: vec![],
        repo: Some(editor::git::RepoPath {
            root: PathBuf::from("."),
            git_dir: PathBuf::from(".git"),
            rel: String::from("conflict-ui-test.dmm"),
        }),
        conflict: Some(editor::conflict::ConflictData {
            operation: None,
            conflicts: vec![dmm::merge::TileConflict {
                coord,
                base: Some(vec![floor.clone()]),
                ours: Some(vec![floor]),
                theirs: Some(vec![incoming.clone()]),
            }],
        }),
    });
    session.set_tool(Tool::Place);
    session
        .state
        .choose_prefab(Prefab::new(TreePath::parse("/turf/closed/wall")));
    let mut app = RectangleUiHarness::with_session(session);
    let button = app.conflict_button.expect("incoming control is on screen");
    app.click(button);
    assert_eq!(
        app.session.state.active_document().unwrap().map.tile_at(coord).unwrap()[0].path,
        incoming.path
    );
    assert_eq!(
        app.session
            .git_state(app.id)
            .unwrap()
            .conflicts
            .as_ref()
            .unwrap()
            .resolution(coord, &app.session.state.active_document().unwrap().history),
        Some(Side::Theirs)
    );
    assert_eq!(app.session.undo_label(), Some("Take theirs"));
}
