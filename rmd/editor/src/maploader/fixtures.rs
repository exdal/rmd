use base::{coord::Coord, types::Value};
use std::{error::Error, fs, sync::Arc};

use dmm::{Map, Size, writer};
use prefab::Prefab;

use crate::{
    document::DocumentId,
    environment::{progress::Progress, session::build_textures},
    session::{
        Session,
        fixtures::{assert_render_cache_matches_rebuild, node_environment_with_profile, settle_bake},
    },
    testing::{TempDir, prefab},
    tool::Tool,
};

pub(crate) type TestResult<T = ()> = Result<T, Box<dyn Error>>;

pub(crate) const PROFILE: &str = include_str!("../../tests/fixtures/programs/module_preview_profile.dm");
pub(crate) const COORDINATE_PROFILE: &str = include_str!("../../tests/fixtures/programs/module_coordinate_profile.dm");
pub(crate) const GENERATE_PROFILE: &str = include_str!("../../tests/fixtures/programs/module_generate_profile.dm");
pub(crate) const WALL: &str = "/turf/closed/wall";
pub(crate) const MODULE: &str = "/obj/module";
pub(crate) const ROOT: &str = "/obj/modular_map_root";

pub(crate) struct Fixture {
    pub(crate) dir: TempDir,
}

impl Fixture {
    pub(crate) fn new(roots: &[(u32, &str)]) -> TestResult<Self> {
        Self::with_variants(roots, "'a.dmm', 'b.dmm'")
    }

    pub(crate) fn with_variants(roots: &[(u32, &str)], variants: &str) -> TestResult<Self> {
        let fixture = Self {
            dir: TempDir::new("module-preview"),
        };
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

    pub(crate) fn write_main(&self, roots: &[(u32, Prefab)]) -> TestResult {
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

    pub(crate) fn write_map(&self, name: &str, map: &Map) -> TestResult {
        fs::write(self.dir.join(name), writer::write(map))?;

        Ok(())
    }

    pub(crate) fn session(&self) -> TestResult<Session> {
        self.session_with(PROFILE)
    }

    pub(crate) fn session_with(&self, profile: &'static str) -> TestResult<Session> {
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

pub(crate) fn viewport_session() -> TestResult<(impl Sized, Session)> {
    let fixture = Fixture::with_variants(&[(2, "outer")], "'missing.dmm', 'a.dmm', 'b.dmm'")?;
    let session = fixture.session()?;

    Ok((fixture, session))
}

pub(crate) fn coordinate_viewport_session() -> TestResult<(impl Sized, Session)> {
    let fixture = Fixture::new(&[])?;
    let session = fixture.session_with(COORDINATE_PROFILE)?;

    Ok((fixture, session))
}

pub(crate) fn keyed(key: &str) -> Prefab {
    let mut root = prefab(ROOT);
    if key != "outer" {
        root.set_var("key".into(), Value::Text(key.into()));
    }

    root
}

pub(crate) fn map(tiles: &[Vec<Prefab>]) -> Map {
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

pub(crate) fn coord(x: u32) -> Coord {
    Coord::new(x, 1, 1)
}

pub(crate) fn effective_paths(session: &Session, x: u32) -> Vec<String> {
    let id = session.state.active().unwrap();

    session
        .effective_grid(id)
        .and_then(|grid| grid.tile_at(coord(x)))
        .map(|tile| tile.iter().map(|prefab| prefab.path.to_string()).collect())
        .unwrap_or_default()
}

pub(crate) fn has(session: &Session, x: u32, path: &str) -> bool {
    effective_paths(session, x).iter().any(|found| found == path)
}

pub(crate) fn assert_settled(session: &Session) {
    assert!(!session.baker.is_busy());
    assert!(session.bake_view().is_none());
    assert_render_cache_matches_rebuild(session);
}

pub(crate) fn open_document(session: &mut Session, fixture: &Fixture, name: &str) -> TestResult<DocumentId> {
    session.open_map(&fixture.dir.join(name), 1)?;
    settle_bake(session);

    Ok(session.state.active().ok_or("no active document")?)
}

pub(crate) fn place_in(session: &mut Session, id: DocumentId, x: u32, path: &str) -> TestResult {
    session.set_active_document(id);
    session.set_tool(Tool::Place);
    session.state.choose_prefab(prefab(path));
    session.place_at(coord(x), None).ok_or("nothing was placed")?;

    Ok(())
}

pub(crate) fn show(session: &mut Session, id: DocumentId) {
    session.set_active_document(id);
    session.sync_open_modules();
}
