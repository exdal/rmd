use core::{
    arena::StrArena,
    path::TreePath,
    types::{Identifier, Value},
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use dmm::{Coord, Map, Prefab, Size};
use editor::{
    BakeProgram,
    Environment,
    blame::BlameResult,
    diff::DiffSource,
    document::{DocumentId, MapDocument, PrefabInstanceId},
    git::{CommitInfo, WebLinks},
    progress::Progress,
};
use render::{FrameUpdate, SpriteInstance};

use super::{BlameState, DiffSide, DiffState, DocumentCache, Session};
use crate::git_worker::Revision;

/// The editor always loads in the background, but tests want one blocking call.
impl Session {
    pub(crate) fn load_environment(&mut self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        self.apply_codebase(crate::loader::load_codebase(
            path,
            &editor::environment::BakeOptions::default(),
            &Progress::new(),
        )?);

        Ok(())
    }

    pub(super) fn open_map(&mut self, path: &Path, z: u32) -> Result<(), Box<dyn std::error::Error>> {
        self.apply_map(crate::loader::load_map(path, z, &Progress::new())?);

        Ok(())
    }

    pub(super) fn active_cache(&self) -> &DocumentCache {
        self.state
            .active()
            .and_then(|id| self.caches.get(&id))
            .expect("a document is open")
    }

    pub(super) fn revision(&self) -> u64 { self.active_cache().revision }

    pub(super) fn frame_update(&self) -> Option<FrameUpdate> { self.active_cache().frame_update }
}

pub(super) fn conflicted_session() -> Session {
    let floor = vec![Prefab::new(TreePath::parse("/turf/floor"))];
    let wall = vec![Prefab::new(TreePath::parse("/turf/wall"))];
    let mut map = Map::new(Size { x: 3, y: 1, z: 1 });
    let key = map.intern_tile(floor.clone());
    map.grid[0][0].fill(key);
    let mut session = Session::new();
    session.apply_map(crate::loader::LoadedMap {
        path: PathBuf::from("conflicts.dmm"),
        map,
        z: 1,
        errors: Vec::new(),
        repo: Some(editor::git::RepoPath {
            root: PathBuf::from("missing-repository"),
            git_dir: PathBuf::from("missing-repository/.git"),
            rel: String::from("conflicts.dmm"),
        }),
        conflict: Some(editor::conflict::ConflictData {
            operation: None,
            conflicts: [1, 3]
                .map(|x| dmm::merge::TileConflict {
                    coord: Coord::new(x, 1, 1),
                    base: Some(floor.clone()),
                    ours: Some(floor.clone()),
                    theirs: Some(wall.clone()),
                })
                .into(),
        }),
    });

    session
}

/// Compares the working map with a `HEAD` that had a wall at 2, 1, 1, listed as the only
/// commit in the history. The map needs to be at least two tiles wide.
pub(crate) fn install_diff(session: &mut Session, id: DocumentId) {
    let mut old = session.state.document(id).unwrap().map.clone();
    let wall = old.intern_tile(vec![Prefab::new(TreePath::parse("/turf/wall"))]);
    let row = old.grid[0].len() - 1;
    old.grid[0][row][1] = wall;
    let commit = CommitInfo {
        hash: String::from("0123456789abcdef"),
        short: String::from("0123456"),
        author: String::from("someone"),
        time: 0,
        summary: String::from("add a wall (#12)"),
        pull_request: Some(12),
    };
    let git = session.caches.get_mut(&id).unwrap().git.as_mut().unwrap();
    git.conflicts = None;
    git.history = Some(vec![commit.clone()]);
    git.show_diff = true;
    git.diff = Some(DiffState::new(
        DiffSide::new(DiffSource::head(), Some(Revision { commit, map: Some(old) })),
        DiffSide::new(DiffSource::Working, None),
    ));
}

pub(crate) fn install_blame(session: &mut Session, id: DocumentId, result: BlameResult) {
    let snapshot = session.state.document(id).unwrap().map.clone();
    let cache = session.caches.get_mut(&id).unwrap();
    let git = cache.git.as_mut().unwrap();
    git.blame = Some(BlameState::new(result, snapshot, cache.map_revision));
    git.show_blame = true;
    git.web = WebLinks::from_remote("https://github.com/example/maps.git");
}

pub(super) fn examples() -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env");

    path.canonicalize().unwrap_or(path)
}

pub(super) fn node_environment() -> Environment {
    const PROFILE: &str = r#"
/obj/cable
/obj/cable/heavy
/obj/pipe/supply
/obj/pipe/scrubbers
/obj/not_node

/datum/demir/example
    default = TRUE

/datum/demir/example/New()
    demir_node_group(/obj/cable, /turf/closed)
    demir_node_group(/obj/pipe/supply, /turf/closed)
    demir_node_group(/obj/pipe/scrubbers, /turf/closed)
"#;
    node_environment_with_profile(PROFILE)
}

pub(super) fn oriented_node_environment() -> Environment {
    const PROFILE: &str = r#"
/obj/link
    dir = 0
/obj/link/segment
/obj/link/junction
/obj/link/endpoint

/datum/demir/example
    default = TRUE

/datum/demir/example/New()
    demir_node_group(/obj/link, null, /obj/link/segment)
    demir_node_orientation(/obj/link/segment, NORTH, NORTH | SOUTH)
    demir_node_orientation(/obj/link/segment, SOUTH, NORTH | SOUTH)
    demir_node_orientation(/obj/link/segment, EAST, EAST | WEST)
    demir_node_orientation(/obj/link/segment, WEST, EAST | WEST)
    demir_node_orientation(/obj/link/segment, NORTHEAST, NORTHEAST)
    demir_node_orientation(/obj/link/segment, SOUTHEAST, SOUTHEAST)
    demir_node_orientation(/obj/link/segment, NORTHWEST, NORTHWEST)
    demir_node_orientation(/obj/link/segment, SOUTHWEST, SOUTHWEST)
    demir_node_orientation(/obj/link/junction, NORTH, NORTH | EAST | SOUTH)
    demir_node_orientation(/obj/link/endpoint, SOUTH, SOUTH)
"#;
    node_environment_with_profile(PROFILE)
}

pub(super) fn node_environment_with_profile(profile: &'static str) -> Environment {
    let root = examples();
    let compile = |baking| {
        let arena = StrArena::new();
        let prelude = preprocessor::prelude_files()
            .into_iter()
            .chain([preprocessor::PreludeFile::Embedded("<test-node-profile.dm>", profile)]);
        let preprocessed = preprocessor::Preprocessor::new(&arena)
            .with_prelude(prelude)
            .with_baking(baking)
            .run(root.join("test.dm"))
            .expect("preprocess");
        assert!(preprocessed.is_ok(), "{:?}", preprocessed.errors);

        let ast = ast::parse(&preprocessed.tokens).expect("parse");
        let (tree, module, errors) = sema::analyze(&ast, baking);
        assert!(errors.is_empty(), "{errors:?}");

        (tree, module)
    };
    let (editor_tree, _) = compile(false);
    let (bake_tree, module) = compile(true);
    let profile = vm::bake::profile_type(&bake_tree).expect("default profile");
    let mut environment = Environment::new(root.join("test.dme"), editor_tree);
    environment.bake_program = Some(BakeProgram {
        tree: bake_tree,
        module: codegen::generate(&module).expect("codegen"),
        profile,
        files: Default::default(),
        icon_states: Default::default(),
    });

    environment
}

pub(crate) fn node_map(width: u32, height: u32, placements: &[(Coord, Vec<&str>)]) -> Map {
    let mut map = Map::new(Size {
        x: width,
        y: height,
        z: 1,
    });
    for y in 1..=height {
        for x in 1..=width {
            let coord = Coord::new(x, y, 1);
            let mut tile = vec![
                Prefab::new(TreePath::parse("/turf/open/floor")),
                Prefab::new(TreePath::parse("/area/station")),
            ];
            tile.extend(
                placements
                    .iter()
                    .filter(|(placed, _)| *placed == coord)
                    .flat_map(|(_, paths)| paths.iter())
                    .map(|path| Prefab::new(TreePath::parse(path))),
            );
            let key = map.intern_tile(tile);
            map.grid[0][(height - y) as usize][(x - 1) as usize] = key;
        }
    }

    map
}

pub(crate) fn node_session(map: Map, seed: Coord) -> (Session, PrefabInstanceId) {
    node_session_with_environment(map, seed, node_environment(), "/obj/cable")
}

pub(super) fn oriented_node_session(map: Map, seed: Coord) -> (Session, PrefabInstanceId) {
    node_session_with_environment(map, seed, oriented_node_environment(), "/obj/link/segment")
}

pub(super) fn node_session_with_environment(
    map: Map, seed: Coord, environment: Environment, target_path: &str,
) -> (Session, PrefabInstanceId) {
    let document = MapDocument::new(map, 1);
    let target = document
        .instance_ids_at(seed)
        .iter()
        .copied()
        .find(|instance| {
            document
                .prefab_instance(*instance)
                .is_some_and(|(prefab, _)| prefab.path.to_string().starts_with(target_path))
        })
        .expect("seed node");
    let bake = editor::bake::build(&environment, &document).expect("node profile bakes");
    let mut session = Session::new();
    session.state.environment = Some(Arc::new(environment));
    let document_id = session.state.open_document(document);
    session.caches.insert(
        document_id,
        DocumentCache {
            bake: Some(bake),
            ..Default::default()
        },
    );
    session.rebuild_instances(document_id);

    (session, target)
}

pub(super) fn oriented_node_map(width: u32, height: u32, placements: &[(Coord, &str, u32)]) -> Map {
    let mut map = Map::new(Size {
        x: width,
        y: height,
        z: 1,
    });
    for y in 1..=height {
        for x in 1..=width {
            let coord = Coord::new(x, y, 1);
            let mut tile = vec![
                Prefab::new(TreePath::parse("/turf/open/floor")),
                Prefab::new(TreePath::parse("/area/station")),
            ];
            for (_, path, dir) in placements.iter().filter(|(placed, ..)| *placed == coord) {
                let mut prefab = Prefab::new(TreePath::parse(path));
                prefab.set_var("dir".into(), Value::Num(*dir as f32));
                tile.push(prefab);
            }
            let key = map.intern_tile(tile);
            map.grid[0][(height - y) as usize][(x - 1) as usize] = key;
        }
    }
    map
}

pub(super) fn oriented_dir(session: &Session, coord: Coord) -> Option<u32> {
    session.map()?.tile_at(coord)?.iter().find_map(|prefab| {
        prefab.path.to_string().starts_with("/obj/link/segment").then(|| {
            prefab
                .var(&Identifier::from("dir"))
                .and_then(Value::as_num)
                .unwrap_or(0.0) as u32
        })
    })
}

pub(crate) fn node_tile_has_group(session: &Session, coord: Coord) -> bool {
    session.map().is_some_and(|map| {
        map.tile_at(coord).is_some_and(|tile| {
            tile.iter()
                .any(|prefab| prefab.path.to_string().starts_with("/obj/cable"))
        })
    })
}

pub(super) fn area_at(session: &Session, coord: Coord) -> Option<PrefabInstanceId> {
    session.area_at(session.state.active()?, coord)
}

pub(super) fn focus_session() -> Session {
    let root = examples();
    let mut session = Session::new();
    session.load_environment(&root.join("test.dme")).unwrap();

    let mut map = Map::new(Size { x: 4, y: 1, z: 5 });
    let base = map.intern_tile(vec![
        Prefab::new(TreePath::parse("/turf/open/floor")),
        Prefab::new(TreePath::parse("/area/station")),
    ]);
    let mut engineering = Prefab::new(TreePath::parse("/area/station"));
    engineering.set_var("name".into(), Value::Text(String::from("Engineering")));
    let other = map.intern_tile(vec![Prefab::new(TreePath::parse("/turf/open/floor")), engineering]);
    map.grid[0][0] = vec![base, base, other, base];
    for level in &mut map.grid {
        level[0] = vec![base, base, other, base];
    }
    let id = session.state.open_document(MapDocument::new(map, 1));
    session.rebuild_instances(id);

    session
}

pub(super) fn flat_session(width: u32, height: u32) -> Session {
    let mut session = focus_session();
    let mut map = Map::new(Size {
        x: width,
        y: height,
        z: 1,
    });
    let base = map.intern_tile(vec![
        Prefab::new(TreePath::parse("/turf/open/floor")),
        Prefab::new(TreePath::parse("/area/station")),
    ]);
    for row in &mut map.grid[0] {
        row.fill(base);
    }
    let id = session.state.open_document(MapDocument::new(map, 1));
    session.rebuild_instances(id);

    session
}

pub(super) fn assert_same_sprites(actual: &[SpriteInstance], expected: &[SpriteInstance]) {
    let mut remaining = expected.to_vec();
    assert_eq!(actual.len(), remaining.len());
    for sprite in actual {
        let index = remaining
            .iter()
            .position(|expected| expected == sprite)
            .expect("updated sprite must match a clean frame build");
        remaining.swap_remove(index);
    }
    assert!(remaining.is_empty());
}

/// Waits for the background bake, since a worker thread has no deterministic finish time.
pub(super) fn settle_bake(session: &mut Session) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);

    while session.baker.is_busy() && std::time::Instant::now() < deadline {
        session.poll_bake();
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

pub(super) fn assert_render_cache_matches_rebuild(session: &Session) {
    let environment = session.state.environment.as_ref().unwrap();
    let document = session.state.active_document().unwrap();
    let expected = editor::frame::build_with_options(
        &environment.tree,
        &environment.icons,
        &session.textures,
        document,
        editor::frame::FrameRenderOptions {
            visibility: &session.type_visibility,
            tile_size: session.options.tile_size,
            appearances: editor::bake::appearances(session.active_cache().bake.as_ref()),
            lighting: session
                .active_cache()
                .bake
                .as_ref()
                .and_then(|bake| bake.lighting.as_ref()),
        },
    );

    assert_same_sprites(&session.instances().unwrap().sprites, &expected.sprites);
    assert_same_sprites(&session.instances().unwrap().area_tiles, &expected.area_tiles);
}
