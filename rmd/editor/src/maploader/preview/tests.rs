use core::{arena::StrArena, path::TreePath};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use dmm::{Coord, Map, Prefab, PrefabInstanceId, Size};
use objtree::{ObjectTree, TypeId};
use vm::{Fault, FaultKind, bake::ModuleRoot};

use super::{
    ComposeContext, MapView, ModuleOverlay, ModulePreviewState, ModuleRootKey, ModuleVariantKey, PreviewIdAllocator,
    compose, compose::update,
};
use crate::{
    document::DocumentId,
    grid::Grid,
    maploader::{
        ErrorKind, LoadedModule, MAX_NESTING, ModuleFiles, OpenModule, RootSpecs, resolve_module_path, root_specs,
    },
};

const ROOT: &str = "/obj/modular_map_root";
const CONNECTOR: &str = "/obj/modular_map_connector";

struct Fixture {
    directory: PathBuf,
    tree: ObjectTree,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "rmd-modular-maps-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(&directory).unwrap();
        let entry = directory.join("types.dm");
        fs::write(&entry, include_str!("../../../tests/fixtures/modular_maps/types.dm")).unwrap();

        Self {
            directory,
            tree: analyze(&entry),
        }
    }

    fn map(&self, path: &str, map: &Map) {
        let path = self.directory.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, dmm::writer::write(map)).unwrap();
    }

    fn ty(&self, path: &str) -> TypeId {
        self.tree.id_of(&TreePath::parse(path)).unwrap()
    }

    fn spec(&self, files: &[&str]) -> ModuleRoot {
        ModuleRoot {
            files: files.iter().map(|file| (*file).to_owned()).collect(),
            anchor: Some(self.ty(CONNECTOR)),
            skip: vec![self.ty("/turf/template_noop"), self.ty("/area/template_noop")],
            offset: [0; 3],
        }
    }

    fn ctx<'a>(&'a self, source: &'a Grid) -> ComposeContext<'a> {
        ComposeContext {
            source,
            tree: &self.tree,
            base_dir: &self.directory,
            document_path: None,
        }
    }

    fn compose(&self, source: &Grid, specs: &RootSpecs) -> ModuleOverlay {
        compose(
            self.ctx(source),
            specs,
            &mut ModuleFiles::default(),
            &Default::default(),
            &mut Default::default(),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn analyze(entry: &Path) -> ObjectTree {
    let arena = StrArena::new();
    let preprocessed = preprocessor::preprocess(&arena, entry).unwrap();
    assert!(preprocessed.errors.is_empty(), "{:?}", preprocessed.errors);
    let ast = ast::parse(&preprocessed.tokens).unwrap();
    let (tree, errors) = sema::analyze(&ast);
    assert!(errors.is_empty(), "{errors:?}");

    tree
}

fn prefab(path: &str) -> Prefab {
    Prefab::new(TreePath::parse(path))
}

fn map(size: [u32; 3], placed: &[(Coord, &str)]) -> Map {
    let mut map = Map::new(Size {
        x: size[0],
        y: size[1],
        z: size[2],
    });
    map.intern_tile(vec![prefab("/turf/floor"), prefab("/area/test")]);
    for (coord, path) in placed {
        let mut tile = map.tile_at(*coord).unwrap().clone();
        tile.push(prefab(path));
        let key = map.intern_tile(tile);
        map.grid[coord.z as usize - 1][(map.size.y - coord.y) as usize][coord.x as usize - 1] = key;
    }

    map
}

fn leaf() -> Map {
    map([1, 1, 1], &[(Coord::new(1, 1, 1), CONNECTOR)])
}

fn marker(source: &Grid, coord: Coord) -> PrefabInstanceId {
    markers(source, coord)[0]
}

fn markers(source: &Grid, coord: Coord) -> Vec<PrefabInstanceId> {
    source
        .ids_at(coord)
        .iter()
        .copied()
        .filter(|id| source.prefab_instance(*id).unwrap().prefab().path == TreePath::parse(ROOT))
        .collect()
}

fn specs(entries: &[(PrefabInstanceId, ModuleRoot)]) -> RootSpecs {
    entries.iter().map(|(id, root)| (*id, Ok(root.clone()))).collect()
}

fn paths(view: MapView<'_>, coord: Coord) -> Vec<String> {
    view.tile_at(coord)
        .unwrap()
        .iter()
        .map(|prefab| prefab.path.to_string())
        .collect()
}

fn preview_id(
    ids: &mut PreviewIdAllocator, root: &ModuleRootKey, choice: &ModuleVariantKey, local: Coord, index: usize,
) -> PrefabInstanceId {
    ids.get(root, choice, local, index).unwrap()
}

fn error_kind(root: &super::ModuleRootPreview, variant: usize) -> &ErrorKind {
    &root.variants[variant].error.as_ref().unwrap().kind
}

#[test]
fn preview_ids_report_exhaustion_and_preserve_existing_ids() {
    let root = ModuleRootKey {
        instance: PrefabInstanceId::from_raw(1).unwrap(),
        ancestry: Vec::new(),
    };
    let choice = ModuleVariantKey {
        path: "module.dmm".into(),
        occurrence: 0,
    };
    let coord = Coord::new(1, 1, 1);
    let mut ids = PreviewIdAllocator {
        next: Some(u64::MAX),
        ..Default::default()
    };
    let last = ids.get(&root, &choice, coord, 0).unwrap();
    assert_eq!(last.get(), u64::MAX);
    assert!(ids.next.is_none());
    assert!(matches!(
        ids.get(&root, &choice, coord, 1),
        Err(ErrorKind::PreviewIdsExhausted)
    ));
    assert_eq!(ids.get(&root, &choice, coord, 0).unwrap(), last);
    assert_eq!(ids.entries.len(), 1);
}

#[test]
fn exhausted_module_keeps_source_tiles_and_other_module_previews() {
    let fixture = Fixture::new();
    let main = map([2, 1, 1], &[(Coord::new(1, 1, 1), ROOT), (Coord::new(2, 1, 1), ROOT)]);
    for x in 1..=2u32 {
        let mut module = Map::new(Size { x: 1, y: 1, z: 1 });
        let mut tile = vec![prefab(CONNECTOR)];
        tile.extend((0..x).map(|_| prefab("/obj/module")));
        module.grid[0][0][0] = module.intern_tile(tile);
        fixture.map(&format!("module-{x}.dmm"), &module);
    }

    let source = Grid::from_map(&main);
    let good = Coord::new(1, 1, 1);
    let failed = Coord::new(2, 1, 1);
    let specs = specs(&[
        (marker(&source, good), fixture.spec(&["module-1.dmm"])),
        (marker(&source, failed), fixture.spec(&["module-2.dmm"])),
    ]);
    let mut ids = PreviewIdAllocator {
        next: Some(u64::MAX - 1),
        ..Default::default()
    };
    let mut files = ModuleFiles::default();
    let ctx = fixture.ctx(&source);
    let preview = compose(ctx, &specs, &mut files, &Default::default(), &mut ids);

    let view = preview.view(&source);
    assert_eq!(view.tile_at(failed), source.tile_at(failed));
    assert_eq!(view.ids_at(failed), source.ids_at(failed));
    assert!(!preview.covered.contains(&failed));
    assert!(preview.covered.contains(&good));
    for id in source.ids_at(good) {
        assert!(view.prefab_instance(*id).is_some());
    }

    let instances = view.prefab_instances().collect::<Vec<_>>();
    assert_eq!(instances.len(), 4 + 3);
    for instance in instances {
        assert_eq!(view.prefab_instance(instance.id), Some(instance));
        assert_eq!(
            view.ids_at(instance.location.coord).get(instance.location.prefab_index),
            Some(&instance.id)
        );
    }

    let diagnostic = preview.diagnostics.first().unwrap();
    assert!(matches!(diagnostic.kind, ErrorKind::PreviewIdsExhausted));
    assert_eq!(diagnostic.coord, Some(failed));
    assert!(matches!(
        preview.roots[1].error.as_ref().map(|error| &error.kind),
        Some(ErrorKind::PreviewIdsExhausted)
    ));

    let updated = update(
        ctx,
        &specs,
        &mut files,
        &preview.choices,
        &mut ids,
        Some(&preview),
        &Default::default(),
    );
    assert!(Arc::ptr_eq(&preview.layers[0], &updated.layers[0]));
    assert_eq!(updated.view(&source).tile_at(failed), source.tile_at(failed));
    assert_eq!(updated.diagnostics.len(), 1);
}

#[test]
fn preview_replaces_turfs_and_areas_appends_objects_and_preserves_source_ids() {
    let fixture = Fixture::new();
    fixture.map(
        "leaf.dmm",
        &map(
            [1, 1, 1],
            &[(Coord::new(1, 1, 1), CONNECTOR), (Coord::new(1, 1, 1), "/obj/module")],
        ),
    );
    let main = map(
        [2, 1, 1],
        &[(Coord::new(2, 1, 1), ROOT), (Coord::new(2, 1, 1), "/obj/source")],
    );
    let source = Grid::from_map(&main);
    let cell = Coord::new(2, 1, 1);
    let preview = fixture.compose(&source, &specs(&[(marker(&source, cell), fixture.spec(&["leaf.dmm"]))]));
    let view = preview.view(&source);

    assert_eq!(
        paths(view, cell),
        [ROOT, "/obj/source", "/turf/floor", "/area/test", "/obj/module"]
    );
    let retained = source.ids_at(cell)[3];
    let (before, after) = (
        source.prefab_instance(retained).unwrap(),
        view.prefab_instance(retained).unwrap(),
    );
    assert_eq!(before.prefab(), after.prefab());
    assert_eq!(before.location.coord, after.location.coord);
    assert_eq!(view.ids_at(Coord::new(1, 1, 1)), source.ids_at(Coord::new(1, 1, 1)));
    assert_eq!(source, Grid::from_map(&main));
    assert_eq!(preview.covered.len(), 1);
    assert!(preview.diagnostics.is_empty());
}

#[test]
fn skip_types_leave_the_destination_untouched_and_match_exactly() {
    let fixture = Fixture::new();
    let mut module = Map::new(Size { x: 2, y: 1, z: 1 });
    let anchored = module.intern_tile(vec![
        prefab(CONNECTOR),
        prefab("/turf/template_noop"),
        prefab("/area/template_noop"),
    ]);
    let subtype = module.intern_tile(vec![prefab("/turf/template_noop/sub")]);
    module.grid[0][0] = vec![anchored, subtype];
    fixture.map("noop.dmm", &module);

    let main = map([3, 1, 1], &[(Coord::new(1, 1, 1), ROOT)]);
    let source = Grid::from_map(&main);
    let preview = fixture.compose(
        &source,
        &specs(&[(marker(&source, Coord::new(1, 1, 1)), fixture.spec(&["noop.dmm"]))]),
    );
    let view = preview.view(&source);

    assert_eq!(paths(view, Coord::new(1, 1, 1)), ["/turf/floor", "/area/test", ROOT]);
    assert_eq!(
        paths(view, Coord::new(2, 1, 1)),
        ["/area/test", "/turf/template_noop/sub"]
    );
    assert!(preview.diagnostics.is_empty());
}

#[test]
fn anchor_prefab_is_dropped_and_subtypes_match() {
    let fixture = Fixture::new();
    fixture.map(
        "alias.dmm",
        &map(
            [2, 1, 1],
            &[
                (Coord::new(2, 1, 1), "/obj/connector_alias"),
                (Coord::new(1, 1, 1), "/obj/leaf"),
            ],
        ),
    );
    let main = map([3, 1, 1], &[(Coord::new(3, 1, 1), ROOT)]);
    let source = Grid::from_map(&main);
    let cell = Coord::new(3, 1, 1);
    let preview = fixture.compose(
        &source,
        &specs(&[(marker(&source, cell), fixture.spec(&["alias.dmm"]))]),
    );
    let view = preview.view(&source);

    assert_eq!(paths(view, cell), [ROOT, "/turf/floor", "/area/test"]);
    assert_eq!(
        paths(view, Coord::new(2, 1, 1)),
        ["/turf/floor", "/area/test", "/obj/leaf"]
    );
    assert!(preview.diagnostics.is_empty());
}

#[test]
fn missing_anchor_type_places_the_module_at_its_local_origin() {
    let fixture = Fixture::new();
    fixture.map(
        "plain.dmm",
        &map(
            [2, 1, 1],
            &[(Coord::new(1, 1, 1), CONNECTOR), (Coord::new(2, 1, 1), "/obj/leaf")],
        ),
    );
    let main = map([3, 1, 1], &[(Coord::new(2, 1, 1), ROOT)]);
    let source = Grid::from_map(&main);
    let root = ModuleRoot {
        anchor: None,
        ..fixture.spec(&["plain.dmm"])
    };
    let preview = fixture.compose(&source, &specs(&[(marker(&source, Coord::new(2, 1, 1)), root)]));
    let view = preview.view(&source);

    assert_eq!(
        paths(view, Coord::new(2, 1, 1)),
        [ROOT, "/turf/floor", "/area/test", "/obj/modular_map_connector"]
    );
    assert_eq!(
        paths(view, Coord::new(3, 1, 1)),
        ["/turf/floor", "/area/test", "/obj/leaf"]
    );
}

#[test]
fn offset_shifts_the_module_layer() {
    let fixture = Fixture::new();
    fixture.map(
        "plain.dmm",
        &map(
            [2, 1, 1],
            &[(Coord::new(1, 1, 1), CONNECTOR), (Coord::new(2, 1, 1), "/obj/leaf")],
        ),
    );
    let main = map([5, 1, 1], &[(Coord::new(2, 1, 1), ROOT)]);
    let source = Grid::from_map(&main);
    let root = ModuleRoot {
        offset: [1, 0, 0],
        ..fixture.spec(&["plain.dmm"])
    };
    let preview = fixture.compose(&source, &specs(&[(marker(&source, Coord::new(2, 1, 1)), root)]));
    let view = preview.view(&source);

    assert_eq!(paths(view, Coord::new(2, 1, 1)), ["/turf/floor", "/area/test", ROOT]);
    assert_eq!(paths(view, Coord::new(3, 1, 1)), ["/turf/floor", "/area/test"]);
    assert_eq!(
        paths(view, Coord::new(4, 1, 1)),
        ["/turf/floor", "/area/test", "/obj/leaf"]
    );
}

#[test]
fn preview_selects_first_valid_variant_and_retains_duplicate_occurrences() {
    let fixture = Fixture::new();
    fixture.map("leaf.dmm", &leaf());
    let main = map([1, 1, 1], &[(Coord::new(1, 1, 1), ROOT)]);
    let source = Grid::from_map(&main);
    let id = marker(&source, Coord::new(1, 1, 1));
    let spec = fixture.spec(&["missing.dmm", "leaf.dmm", "leaf.dmm"]);

    let first = fixture.compose(&source, &specs(&[(id, spec)]));
    assert_eq!(first.roots[0].selected, Some(1));
    assert!(matches!(error_kind(&first.roots[0], 0), ErrorKind::Read { .. }));
    let occurrences = first.roots[0]
        .variants
        .iter()
        .map(|variant| variant.choice.occurrence)
        .collect::<Vec<_>>();
    assert_eq!(occurrences, [0, 0, 1]);

    let mut choices = first.choices.clone();
    choices.insert(first.roots[0].key.clone(), first.roots[0].variants[2].choice.clone());
    let compose_with = |files: &[&str]| {
        compose(
            fixture.ctx(&source),
            &specs(&[(id, fixture.spec(files))]),
            &mut ModuleFiles::default(),
            &choices,
            &mut Default::default(),
        )
    };

    assert_eq!(
        compose_with(&["missing.dmm", "leaf.dmm", "leaf.dmm"]).roots[0].selected,
        Some(2)
    );
    assert_eq!(compose_with(&["leaf.dmm", "leaf.dmm"]).roots[0].selected, Some(1));
    assert_eq!(compose_with(&["leaf.dmm"]).roots[0].selected, Some(0));
}

#[test]
fn preview_clips_and_keeps_failed_sibling_roots() {
    let fixture = Fixture::new();
    fixture.map("wide.dmm", &map([3, 1, 1], &[(Coord::new(2, 1, 1), CONNECTOR)]));
    let main = map(
        [1, 1, 1],
        &[
            (Coord::new(1, 1, 1), ROOT),
            (Coord::new(1, 1, 1), ROOT),
            (Coord::new(1, 1, 1), ROOT),
        ],
    );
    let source = Grid::from_map(&main);
    let ids = markers(&source, Coord::new(1, 1, 1));
    let hook = Err("proc fault".to_owned());
    let mut specs = specs(&[
        (ids[0], fixture.spec(&["wide.dmm"])),
        (ids[1], fixture.spec(&["missing.dmm"])),
    ]);
    specs.insert(ids[2], hook);
    let preview = fixture.compose(&source, &specs);

    assert_eq!(preview.view(&source).size(), main.size);
    assert!(
        preview
            .diagnostics
            .iter()
            .any(|error| matches!(error.kind, ErrorKind::Clipped { tiles: 2 }))
    );
    assert_eq!(
        paths(preview.view(&source), Coord::new(1, 1, 1))
            .iter()
            .filter(|path| *path == ROOT)
            .count(),
        3
    );
    assert!(preview.roots[1].selected.is_none());
    let failed = preview.roots[2].error.as_ref().unwrap();
    assert!(matches!(&failed.kind, ErrorKind::Hook(text) if text == "proc fault"));
    assert_eq!(failed.coord, Some(Coord::new(1, 1, 1)));
    assert!(failed.to_string().ends_with("proc fault"));
    assert!(preview.roots[2].variants.is_empty());
}

#[test]
fn variant_errors_are_reported_per_variant() {
    let fixture = Fixture::new();
    fixture.map("none.dmm", &map([1, 1, 1], &[]));
    fixture.map(
        "two.dmm",
        &map(
            [2, 1, 1],
            &[
                (Coord::new(1, 1, 1), CONNECTOR),
                (Coord::new(2, 1, 1), "/obj/connector_alias"),
            ],
        ),
    );
    fs::write(fixture.directory.join("bad.dmm"), "not a map").unwrap();
    let main = map([1, 1, 1], &[(Coord::new(1, 1, 1), ROOT)]);
    let source = Grid::from_map(&main);
    let spec = fixture.spec(&[
        "none.dmm",
        "two.dmm",
        "missing.dmm",
        "bad.dmm",
        "../x.dmm",
        "/etc/passwd",
    ]);
    let preview = fixture.compose(&source, &specs(&[(marker(&source, Coord::new(1, 1, 1)), spec)]));
    let root = &preview.roots[0];

    assert!(matches!(error_kind(root, 0), ErrorKind::Anchors { count: 0 }));
    assert!(matches!(error_kind(root, 1), ErrorKind::Anchors { count: 2 }));
    assert!(matches!(error_kind(root, 2), ErrorKind::Read { .. }));
    assert!(matches!(error_kind(root, 3), ErrorKind::Dmm(_)));
    assert!(matches!(error_kind(root, 4), ErrorKind::OutsideCodebase));
    assert!(matches!(error_kind(root, 5), ErrorKind::OutsideCodebase));
    assert_eq!(root.selected, None);
    assert_eq!(preview.diagnostics.len(), 6);
    assert!(preview.layers.is_empty());
}

#[test]
fn variant_matching_the_document_is_a_cycle() {
    let fixture = Fixture::new();
    fixture.map("main.dmm", &leaf());
    let main = map([1, 1, 1], &[(Coord::new(1, 1, 1), ROOT)]);
    let source = Grid::from_map(&main);
    let specs = specs(&[(marker(&source, Coord::new(1, 1, 1)), fixture.spec(&["sub/../main.dmm"]))]);
    let document = fixture.directory.join("main.dmm");
    let ctx = ComposeContext {
        document_path: Some(&document),
        ..fixture.ctx(&source)
    };
    let preview = compose(
        ctx,
        &specs,
        &mut ModuleFiles::default(),
        &Default::default(),
        &mut Default::default(),
    );

    assert!(matches!(error_kind(&preview.roots[0], 0), ErrorKind::Cycle));
}

fn nested_fixture() -> (Fixture, Grid) {
    let fixture = Fixture::new();
    fixture.map(
        "outer.dmm",
        &map(
            [3, 2, 1],
            &[(Coord::new(2, 1, 1), CONNECTOR), (Coord::new(3, 2, 1), ROOT)],
        ),
    );
    fixture.map(
        "leaf.dmm",
        &map(
            [2, 2, 1],
            &[
                (Coord::new(2, 2, 1), "/obj/connector_alias"),
                (Coord::new(1, 2, 1), "/obj/leaf"),
            ],
        ),
    );
    let main = map([6, 4, 3], &[(Coord::new(4, 3, 2), ROOT)]);

    (fixture, Grid::from_map(&main))
}

#[test]
fn nested_roots_compose_at_grid_offsets_once_they_have_specs() {
    let (fixture, source) = nested_fixture();
    let outer = marker(&source, Coord::new(4, 3, 2));
    let mut specs = specs(&[(outer, fixture.spec(&["outer.dmm"]))]);
    let mut state = ModulePreviewState::default();
    let mut ctx = fixture.ctx(&source);
    state.recompose(ctx, specs.clone(), &HashSet::new());

    let preview = state.preview.as_ref().unwrap();
    assert_eq!(preview.roots.len(), 1);
    let root = &preview.roots[0];
    let choice = root.variants[0].choice.clone();
    let nested = preview_id(&mut state.ids, &root.key, &choice, Coord::new(3, 2, 1), 2);
    let nested_cell = Coord::new(5, 4, 2);
    assert_eq!(
        state.preview.as_ref().unwrap().view(&source).ids_at(nested_cell).len(),
        3
    );

    specs.insert(nested, Ok(fixture.spec(&["leaf.dmm"])));
    assert!(state.needs_recompose(&specs));
    ctx = fixture.ctx(&source);
    let changed = state.recompose(ctx, specs.clone(), &HashSet::new());
    assert!(!state.needs_recompose(&specs));

    let preview = state.preview.as_ref().unwrap();
    assert_eq!(preview.roots.len(), 2);
    assert_eq!(preview.roots[1].coord, nested_cell);
    assert_eq!(preview.roots[1].depth, 1);
    assert_eq!(preview.roots[1].key.ancestry.len(), 1);
    assert_eq!(preview.roots[1].key.ancestry[0].1, Coord::new(3, 2, 1));
    assert_eq!(preview.roots[1].key.ancestry[0].2, 2);
    assert!(preview.diagnostics.is_empty(), "{:?}", preview.diagnostics);

    let view = state.view(&source);
    assert_eq!(
        paths(view, Coord::new(4, 4, 2)),
        ["/turf/floor", "/area/test", "/obj/leaf"]
    );
    assert!(view.prefab_instance(nested).is_some());
    assert!(view.prefab_instance(outer).is_some());
    assert!(paths(view, nested_cell).iter().any(|path| path == ROOT));
    assert!(changed.contains(&nested));

    let leaf_id = view.ids_at(Coord::new(4, 4, 2))[2];
    assert!(changed.contains(&leaf_id));
}

#[test]
fn variant_matching_an_ancestor_is_a_cycle() {
    let (fixture, source) = nested_fixture();
    let outer = marker(&source, Coord::new(4, 3, 2));
    let mut files = ModuleFiles::default();
    let mut ids = PreviewIdAllocator::default();
    let ctx = fixture.ctx(&source);
    let mut specs = specs(&[(outer, fixture.spec(&["outer.dmm"]))]);
    let first = compose(ctx, &specs, &mut files, &Default::default(), &mut ids);
    let root = &first.roots[0];
    let nested = preview_id(&mut ids, &root.key, &root.variants[0].choice, Coord::new(3, 2, 1), 2);

    specs.insert(nested, Ok(fixture.spec(&["./outer.dmm"])));
    let second = compose(ctx, &specs, &mut files, &Default::default(), &mut ids);

    assert_eq!(second.roots.len(), 2);
    assert!(matches!(error_kind(&second.roots[1], 0), ErrorKind::Cycle));
    assert!(second.roots[1].variants[0].error.as_ref().unwrap().chain.len() > 1);
}

#[test]
fn nesting_beyond_the_limit_is_rejected() {
    let fixture = Fixture::new();
    for index in 0..=MAX_NESTING + 2 {
        fixture.map(
            &format!("m{index}.dmm"),
            &map(
                [1, 1, 1],
                &[(Coord::new(1, 1, 1), CONNECTOR), (Coord::new(1, 1, 1), ROOT)],
            ),
        );
    }

    let main = map([1, 1, 1], &[(Coord::new(1, 1, 1), ROOT)]);
    let source = Grid::from_map(&main);
    let mut files = ModuleFiles::default();
    let mut ids = PreviewIdAllocator::default();
    let ctx = fixture.ctx(&source);
    let mut specs = specs(&[(marker(&source, Coord::new(1, 1, 1)), fixture.spec(&["m0.dmm"]))]);
    for depth in 1..=MAX_NESTING + 2 {
        let preview = compose(ctx, &specs, &mut files, &Default::default(), &mut ids);
        let last = preview.roots.last().unwrap();
        assert_eq!(last.depth, depth - 1);
        if last.selected.is_none() {
            assert!(matches!(error_kind(last, 0), ErrorKind::NestingLimit));
            assert_eq!(depth, MAX_NESTING + 2);
            return;
        }

        let choice = &last.variants[last.selected.unwrap()].choice;
        let nested = preview_id(&mut ids, &last.key, choice, Coord::new(1, 1, 1), 3);
        specs.insert(nested, Ok(fixture.spec(&[&format!("m{depth}.dmm")])));
    }

    panic!("nesting limit was never reached");
}

#[test]
fn changes_report_added_removed_and_hidden_ids() {
    let fixture = Fixture::new();
    fixture.map(
        "leaf.dmm",
        &map(
            [1, 1, 1],
            &[(Coord::new(1, 1, 1), CONNECTOR), (Coord::new(1, 1, 1), "/obj/module")],
        ),
    );
    let main = map([1, 1, 1], &[(Coord::new(1, 1, 1), ROOT)]);
    let source = Grid::from_map(&main);
    let cell = Coord::new(1, 1, 1);
    let id = marker(&source, cell);
    let specs = specs(&[(id, fixture.spec(&["leaf.dmm"]))]);
    let ctx = fixture.ctx(&source);

    let mut state = ModulePreviewState::default();
    let added = state.recompose(ctx, specs.clone(), &HashSet::new());
    let preview_ids = state
        .view(&source)
        .ids_at(cell)
        .iter()
        .copied()
        .filter(|id| id.get() >= super::PREVIEW_ID_START)
        .collect::<Vec<_>>();
    assert_eq!(preview_ids.len(), 3);
    assert!(preview_ids.iter().all(|id| id.get() >= super::PREVIEW_ID_START));
    assert!(added.contains(&id));
    assert!(preview_ids.iter().all(|id| added.contains(id)));

    let sorted = |mut ids: Vec<PrefabInstanceId>| {
        ids.sort();
        ids
    };
    let disabled = state.set_enabled(&source, false);
    assert_eq!(sorted(disabled), sorted(added.clone()));
    assert_eq!(state.view(&source).ids_at(cell), source.ids_at(cell));
    assert_eq!(sorted(state.set_enabled(&source, true)), sorted(added.clone()));

    let removed = state.recompose(ctx, RootSpecs::default(), &HashSet::new());
    assert_eq!(sorted(removed), sorted(added));
    assert_eq!(state.view(&source).ids_at(cell), source.ids_at(cell));
}

#[test]
fn unrelated_recompose_keeps_layer_identity() {
    let fixture = Fixture::new();
    fixture.map("leaf.dmm", &leaf());
    let main = map([3, 1, 1], &[(Coord::new(1, 1, 1), ROOT)]);
    let source = Grid::from_map(&main);
    let specs = specs(&[(marker(&source, Coord::new(1, 1, 1)), fixture.spec(&["leaf.dmm"]))]);
    let mut files = ModuleFiles::default();
    let mut ids = PreviewIdAllocator::default();
    let ctx = fixture.ctx(&source);
    let first = compose(ctx, &specs, &mut files, &Default::default(), &mut ids);
    let dirty = HashSet::from([Coord::new(3, 1, 1)]);
    let second = update(ctx, &specs, &mut files, &first.choices, &mut ids, Some(&first), &dirty);

    assert!(Arc::ptr_eq(&first.layers[0], &second.layers[0]));
    assert!(ModuleOverlay::changes(&source, Some(&first), Some(&second)).is_empty());

    let mut other = specs.clone();
    other
        .get_mut(&marker(&source, Coord::new(1, 1, 1)))
        .unwrap()
        .as_mut()
        .unwrap()
        .skip
        .clear();
    let third = update(
        ctx,
        &other,
        &mut files,
        &first.choices,
        &mut ids,
        Some(&first),
        &HashSet::new(),
    );
    assert!(!Arc::ptr_eq(&first.layers[0], &third.layers[0]));
}

#[test]
fn source_changed_recomposes_only_for_covered_or_root_ids() {
    let fixture = Fixture::new();
    fixture.map("leaf.dmm", &leaf());
    let main = map(
        [3, 1, 1],
        &[(Coord::new(1, 1, 1), ROOT), (Coord::new(3, 1, 1), "/obj/far")],
    );
    let source = Grid::from_map(&main);
    let root = marker(&source, Coord::new(1, 1, 1));
    let specs = specs(&[(root, fixture.spec(&["leaf.dmm"]))]);
    let ctx = fixture.ctx(&source);
    let mut state = ModulePreviewState::default();
    state.recompose(ctx, specs, &HashSet::new());

    let far = *source.ids_at(Coord::new(3, 1, 1)).last().unwrap();
    assert!(state.source_changed(ctx, &[far]).is_none());
    assert!(state.source_changed(ctx, &[root]).is_some());

    let resized = Grid::from_map(&map([4, 1, 1], &[(Coord::new(1, 1, 1), ROOT)]));
    assert!(state.source_changed(fixture.ctx(&resized), &[]).is_some());

    state.reset();
    assert!(state.preview.is_none() && state.specs == RootSpecs::default());
    assert!(state.enabled);
}

#[test]
fn stacked_sibling_roots_respect_turf_and_area_parents() {
    let fixture = Fixture::new();
    for (file, turf, area) in [
        ("one", "/turf/template_noop", "/obj/area_alias"),
        ("two", "/obj/turf_alias", "/area/template_noop"),
    ] {
        let mut module = Map::new(Size { x: 1, y: 1, z: 1 });
        let tile = vec![
            prefab(CONNECTOR),
            prefab(&format!("/obj/{file}")),
            prefab(turf),
            prefab(area),
        ];
        module.grid[0][0][0] = module.intern_tile(tile);
        fixture.map(&format!("{file}.dmm"), &module);
    }

    let main = map([1, 1, 1], &[(Coord::new(1, 1, 1), ROOT), (Coord::new(1, 1, 1), ROOT)]);
    let source = Grid::from_map(&main);
    let ids = markers(&source, Coord::new(1, 1, 1));
    let specs = specs(&[
        (ids[0], fixture.spec(&["one.dmm"])),
        (ids[1], fixture.spec(&["two.dmm"])),
    ]);
    let preview = fixture.compose(&source, &specs);

    assert_eq!(
        paths(preview.view(&source), Coord::new(1, 1, 1)),
        [ROOT, ROOT, "/obj/one", "/obj/area_alias", "/obj/two", "/obj/turf_alias"]
    );
    assert!(preview.diagnostics.is_empty());
    assert_ne!(preview.roots[0].key, preview.roots[1].key);
    assert_eq!(preview.covered.len(), 1);
}

#[test]
fn module_paths_resolve_lexically_under_the_codebase() {
    let base = Path::new("/codebase");
    assert_eq!(
        resolve_module_path(base, "maps\\a.dmm").unwrap(),
        base.join("maps/a.dmm")
    );
    assert_eq!(
        resolve_module_path(base, "./maps/../a.dmm").unwrap(),
        base.join("a.dmm")
    );
    for name in ["../a.dmm", "maps/../../a.dmm", "/etc/passwd"] {
        assert!(
            matches!(resolve_module_path(base, name), Err(ErrorKind::OutsideCodebase)),
            "{name}"
        );
    }
}

#[test]
fn files_are_cached_by_resolved_path_and_anchors_by_type() {
    let fixture = Fixture::new();
    fixture.map("a.dmm", &leaf());
    let mut files = ModuleFiles::default();
    let path = resolve_module_path(&fixture.directory, "sub/../a.dmm").unwrap();
    let first = files.load(&path).unwrap();
    let second = files
        .load(&resolve_module_path(&fixture.directory, "a.dmm").unwrap())
        .unwrap();
    assert!(Arc::ptr_eq(&first, &second));

    assert_eq!(first.anchor(&fixture.tree, None).unwrap(), (Coord::new(1, 1, 1), None));
    let alias = Some(fixture.ty("/obj/connector_alias"));
    assert!(matches!(
        first.anchor(&fixture.tree, alias),
        Err(ErrorKind::Anchors { count: 0 })
    ));
    assert_eq!(
        first.anchor(&fixture.tree, Some(fixture.ty(CONNECTOR))).unwrap(),
        (Coord::new(1, 1, 1), Some(2))
    );

    files.clear();
    assert!(!Arc::ptr_eq(&first, &files.load(&path).unwrap()));
}

#[test]
fn root_specs_collects_modules_faults_and_errors_and_skips_invalid_ids() {
    let fixture = Fixture::new();
    let spec = fixture.spec(&["a.dmm"]);
    let fault = Fault::detached(FaultKind::InvalidReference);
    let collected = root_specs(
        [(7, &spec), (0, &spec)].into_iter(),
        [(9, &fault), (0, &fault)].into_iter(),
        [(11, "no room"), (0, "no room")].into_iter(),
    );

    assert_eq!(collected.len(), 3);
    assert_eq!(
        collected[&PrefabInstanceId::from_raw(11).unwrap()],
        Err("no room".to_string())
    );
    assert_eq!(collected[&PrefabInstanceId::from_raw(7).unwrap()], Ok(spec));
    assert_eq!(
        collected[&PrefabInstanceId::from_raw(9).unwrap()],
        Err(fault.to_string())
    );
}

#[test]
fn needs_recompose_flips_when_specs_change() {
    let fixture = Fixture::new();
    fixture.map("leaf.dmm", &leaf());
    let main = map([1, 1, 1], &[(Coord::new(1, 1, 1), ROOT)]);
    let source = Grid::from_map(&main);
    let id = marker(&source, Coord::new(1, 1, 1));
    let first = specs(&[(id, fixture.spec(&["leaf.dmm"]))]);
    let mut state = ModulePreviewState::default();

    assert!(!state.needs_recompose(&RootSpecs::default()));
    assert!(state.needs_recompose(&first));
    state.recompose(fixture.ctx(&source), first.clone(), &HashSet::new());
    assert!(!state.needs_recompose(&first));
    assert!(state.needs_recompose(&specs(&[(id, fixture.spec(&["other.dmm"]))])));
    assert!(state.needs_recompose(&RootSpecs::default()));
}

#[test]
fn composite_keeps_roots_and_is_stable_when_fed_back() {
    let (fixture, source) = nested_fixture();
    let outer = marker(&source, Coord::new(4, 3, 2));
    let mut specs = specs(&[(outer, fixture.spec(&["outer.dmm"]))]);
    let mut state = ModulePreviewState::default();
    let ctx = fixture.ctx(&source);
    state.recompose(ctx, specs.clone(), &HashSet::new());

    let root = &state.preview.as_ref().unwrap().roots[0];
    let nested = preview_id(
        &mut state.ids,
        &root.key,
        &root.variants[0].choice.clone(),
        Coord::new(3, 2, 1),
        2,
    );
    specs.insert(nested, Ok(fixture.spec(&["leaf.dmm"])));
    state.recompose(ctx, specs.clone(), &HashSet::new());

    let present = state
        .view(&source)
        .prefab_instances()
        .map(|instance| instance.id)
        .collect::<Vec<_>>();
    assert!(present.contains(&outer));
    assert!(present.contains(&nested));
    assert!(state.recompose(ctx, specs, &HashSet::new()).is_empty());
}

fn open_module(map: Map) -> OpenModule {
    OpenModule {
        document: DocumentId::new(),
        generation: 1,
        module: Arc::new(LoadedModule::new(map)),
    }
}

fn open_fixture() -> (Fixture, Grid, ModulePreviewState, PathBuf) {
    let fixture = Fixture::new();
    fixture.map(
        "a.dmm",
        &map(
            [2, 1, 1],
            &[(Coord::new(1, 1, 1), CONNECTOR), (Coord::new(2, 1, 1), "/obj/disk")],
        ),
    );
    let source = Grid::from_map(&map([3, 1, 1], &[(Coord::new(1, 1, 1), ROOT)]));
    let path = resolve_module_path(&fixture.directory, "a.dmm").unwrap();
    let specs = specs(&[(marker(&source, Coord::new(1, 1, 1)), fixture.spec(&["a.dmm"]))]);
    let mut state = ModulePreviewState::default();
    state.recompose(fixture.ctx(&source), specs, &HashSet::new());

    (fixture, source, state, path)
}

fn open_override() -> OpenModule {
    open_module(map(
        [2, 1, 1],
        &[(Coord::new(1, 1, 1), CONNECTOR), (Coord::new(2, 1, 1), "/obj/live")],
    ))
}

#[test]
fn open_module_replaces_the_disk_file_until_it_is_revoked() {
    let (fixture, source, mut state, path) = open_fixture();
    assert!(paths(state.view(&source), Coord::new(2, 1, 1)).contains(&"/obj/disk".to_owned()));

    let changed = state.replace_module(fixture.ctx(&source), &path, Some(open_override()));
    assert!(!changed.is_empty());
    let live = paths(state.view(&source), Coord::new(2, 1, 1));
    assert!(live.contains(&"/obj/live".to_owned()));
    assert!(!live.contains(&"/obj/disk".to_owned()));

    let changed = state.replace_module(fixture.ctx(&source), &path, None);
    assert!(!changed.is_empty());
    assert!(paths(state.view(&source), Coord::new(2, 1, 1)).contains(&"/obj/disk".to_owned()));
    assert!(state.open_module(&path).is_none());
}

#[test]
fn open_module_survives_reset_and_fresh() {
    let (fixture, source, mut state, path) = open_fixture();
    state.replace_module(fixture.ctx(&source), &path, Some(open_override()));

    let mut fresh = state.fresh();
    assert!(fresh.open_module(&path).is_some());
    assert_eq!(fresh.open_modules().count(), 1);

    state.reset();
    assert!(state.open_module(&path).is_some());
    let specs = specs(&[(marker(&source, Coord::new(1, 1, 1)), fixture.spec(&["a.dmm"]))]);
    state.recompose(fixture.ctx(&source), specs.clone(), &HashSet::new());
    assert!(paths(state.view(&source), Coord::new(2, 1, 1)).contains(&"/obj/live".to_owned()));

    fresh.recompose(fixture.ctx(&source), specs, &HashSet::new());
    assert!(paths(fresh.view(&source), Coord::new(2, 1, 1)).contains(&"/obj/live".to_owned()));
}

#[test]
fn uses_file_tracks_loaded_paths() {
    let (fixture, _source, state, path) = open_fixture();

    assert!(state.uses_file(&path));
    assert!(!state.uses_file(&fixture.directory.join("unrelated.dmm")));
}

#[test]
fn an_override_alone_does_not_count_as_using_a_file() {
    let (fixture, source, mut state, _path) = open_fixture();
    let other = resolve_module_path(&fixture.directory, "other.dmm").unwrap();

    state.replace_module(fixture.ctx(&source), &other, Some(open_override()));

    assert!(state.open_module(&other).is_some());
    assert!(!state.uses_file(&other));
}

#[test]
fn overriding_a_nested_file_rebuilds_its_layer() {
    let (fixture, source) = nested_fixture();
    let outer = marker(&source, Coord::new(4, 3, 2));
    let mut state = ModulePreviewState::default();
    state.recompose(
        fixture.ctx(&source),
        specs(&[(outer, fixture.spec(&["outer.dmm"]))]),
        &HashSet::new(),
    );
    let root = &state.preview.as_ref().unwrap().roots[0];
    let choice = root.variants[0].choice.clone();
    let nested = preview_id(&mut state.ids, &root.key, &choice, Coord::new(3, 2, 1), 2);
    let all = specs(&[
        (outer, fixture.spec(&["outer.dmm"])),
        (nested, fixture.spec(&["leaf.dmm"])),
    ]);
    state.recompose(fixture.ctx(&source), all, &HashSet::new());
    assert!(paths(state.view(&source), Coord::new(4, 4, 2)).contains(&"/obj/leaf".to_owned()));

    let leaf = resolve_module_path(&fixture.directory, "leaf.dmm").unwrap();
    let live = open_module(map(
        [2, 2, 1],
        &[
            (Coord::new(2, 2, 1), "/obj/connector_alias"),
            (Coord::new(1, 2, 1), "/obj/live"),
        ],
    ));
    let changed = state.replace_module(fixture.ctx(&source), &leaf, Some(live));

    assert!(!changed.is_empty());
    let view = state.view(&source);
    let tile = paths(view, Coord::new(4, 4, 2));
    assert!(tile.contains(&"/obj/live".to_owned()));
    assert!(!tile.contains(&"/obj/leaf".to_owned()));
}
