use core::{path::TreePath, types::Value};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use dmm::{Coord, Map, Prefab};
use objtree::{ObjectTree, TypeId};
use serde::Deserialize;

use super::{
    MAX_NESTING,
    MapLoader,
    error::{Diagnostic, ErrorKind},
    resolved_path,
};
use crate::{maploader::map_module, visual};

// editor should never care about the codebase they(I) said
const ROOT_PATH: &str = "/obj/modular_map_root";
const CONNECTOR_PATH: &str = "/obj/modular_map_connector";
const IGNORED_ATOM_PATHS: [&str; 2] = ["/turf/template_noop", "/area/template_noop"];

#[derive(Debug, Default, Clone, Copy)]
pub struct TgMapLoader;

impl MapLoader for TgMapLoader {
    fn discover_map_modules(
        &self, codebase: &Path, tree: &ObjectTree, source_map: &Path, map: &Map,
    ) -> map_module::Discovery {
        let source_map = resolved_path(&codebase.join(source_map));
        let mut discovery = TgDiscovery {
            codebase,
            tree,
            root_type: tree.id_of(&TreePath::parse(ROOT_PATH)),
            connector_type: tree.id_of(&TreePath::parse(CONNECTOR_PATH)),
            configs: HashMap::new(),
            maps: HashMap::new(),
            diagnostics: Vec::new(),
            ancestry: vec![source_map.clone()],
        };

        map_module::Discovery {
            roots: discovery.roots(&source_map, map),
            diagnostics: discovery.diagnostics,
        }
    }

    fn ignored_atom_paths() -> &'static [&'static str] { &IGNORED_ATOM_PATHS }
}

#[derive(Debug, Deserialize)]
struct Config {
    directory: String,
    rooms: HashMap<String, toml::Value>,
}

#[derive(Deserialize)]
struct Room {
    modules: Vec<String>,
}

struct ParsedModule {
    map: Arc<Map>,
    connector: Coord,
}

struct TgDiscovery<'a> {
    codebase: &'a Path,
    tree: &'a ObjectTree,
    root_type: Option<TypeId>,
    connector_type: Option<TypeId>,
    configs: HashMap<PathBuf, Result<Arc<Config>, ErrorKind>>,
    maps: HashMap<PathBuf, Result<Arc<ParsedModule>, ErrorKind>>,
    diagnostics: Vec<Diagnostic>,
    ancestry: Vec<PathBuf>,
}

impl TgDiscovery<'_> {
    fn roots(&mut self, source: &Path, map: &Map) -> Vec<map_module::Root> {
        let Some(root_type) = self.root_type else {
            return Vec::new();
        };

        let mut roots = Vec::new();
        for (coord, prefab_index, prefab) in placements(map) {
            let Some(ty) = self
                .tree
                .id_of(&prefab.path)
                .filter(|ty| self.tree.is_subtype_of(*ty, root_type))
            else {
                continue;
            };

            let config = self.text(ty, prefab, "config_file");
            let key = self.text(ty, prefab, "key");
            let location = map_module::RootLocation {
                source_map: source.to_path_buf(),
                coord,
                prefab_index,
                config_file: config
                    .as_ref()
                    .ok()
                    .and_then(|value| value.as_ref())
                    .map(|path| resolved_path(&self.codebase.join(dm_path(path)))),
                key: key.as_ref().ok().cloned().flatten(),
            };
            let variants = match (config, key) {
                (Err(error), _) | (_, Err(error)) => Err(error),
                (Ok(Some(_)), Ok(Some(_))) => self.variants(&location),
                _ => Ok(Vec::new()),
            };
            let (variants, error) = match variants {
                Ok(variants) => (variants, None),
                Err(kind) => (Vec::new(), Some(self.diagnostic(&location, None, kind))),
            };
            roots.push(map_module::Root {
                location,
                variants,
                error,
            });
        }
        roots
    }

    fn text(&self, ty: TypeId, prefab: &Prefab, name: &'static str) -> Result<Option<String>, ErrorKind> {
        match visual::resolve_value(self.tree, ty, prefab, &name.into()).map(|resolved| resolved.value) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Text(text)) => Ok((!text.is_empty()).then(|| text.clone())),
            _ => Err(ErrorKind::InvalidField(name)),
        }
    }

    fn variants(&mut self, root: &map_module::RootLocation) -> Result<Vec<map_module::Variant>, ErrorKind> {
        let path = root.config_file.as_ref().expect("active root config");
        let config = self
            .configs
            .entry(path.clone())
            .or_insert_with(|| {
                let source = read(path)?;
                toml::from_str::<Config>(&source)
                    .map(Arc::new)
                    .map_err(|error| ErrorKind::Config(error.to_string()))
            })
            .clone()?;
        let value = config
            .rooms
            .get(root.key.as_ref().expect("active root key"))
            .ok_or(ErrorKind::MissingKey)?;
        let room = value
            .clone()
            .try_into::<Room>()
            .map_err(|error| ErrorKind::Config(error.to_string()))?;
        if room.modules.is_empty() {
            return Err(ErrorKind::EmptyModules);
        }

        let directory = self.codebase.join(dm_path(&config.directory));

        Ok(room
            .modules
            .into_iter()
            .map(|file| {
                let path = resolved_path(&directory.join(dm_path(&file)));
                let module = self
                    .module(root, &path)
                    .map_err(|kind| self.diagnostic(root, Some(&path), kind));
                map_module::Variant { path, module }
            })
            .collect::<Vec<_>>())
    }

    fn module(&mut self, root: &map_module::RootLocation, path: &Path) -> Result<map_module::MapModule, ErrorKind> {
        if self.ancestry.iter().any(|ancestor| ancestor == path) {
            return Err(ErrorKind::Cycle);
        }

        if self.ancestry.len() > MAX_NESTING {
            return Err(ErrorKind::NestingLimit);
        }

        let tree = self.tree;
        let connector_type = self.connector_type;
        let parsed = self
            .maps
            .entry(path.to_path_buf())
            .or_insert_with(|| {
                let source = read(path)?;
                let (map, errors) = dmm::parser::parse(&source);
                if !errors.is_empty() {
                    return Err(ErrorKind::Dmm(errors));
                }
                let connectors = placements(&map)
                    .filter(|(_, _, prefab)| {
                        connector_type
                            .is_some_and(|base| tree.id_of(&prefab.path).is_some_and(|ty| tree.is_subtype_of(ty, base)))
                    })
                    .map(|(coord, ..)| coord)
                    .collect::<Vec<_>>();
                let [connector] = connectors.as_slice() else {
                    return Err(ErrorKind::Connectors {
                        count: connectors.len(),
                    });
                };
                Ok(Arc::new(ParsedModule {
                    map: Arc::new(map),
                    connector: *connector,
                }))
            })
            .clone()?;

        self.ancestry.push(path.to_path_buf());
        let roots = self.roots(path, &parsed.map);
        self.ancestry.pop();

        Ok(map_module::MapModule {
            map: parsed.map.clone(),
            connector: parsed.connector,
            translation: [
                i64::from(root.coord.x) - i64::from(parsed.connector.x),
                i64::from(root.coord.y) - i64::from(parsed.connector.y),
                i64::from(root.coord.z) - 1,
            ],
            roots,
        })
    }

    fn diagnostic(&mut self, root: &map_module::RootLocation, module: Option<&Path>, kind: ErrorKind) -> Diagnostic {
        let mut chain = self.ancestry.clone();
        if let Some(module) = module {
            chain.push(module.to_path_buf());
        }

        let diagnostic = Diagnostic {
            root: Some(root.clone()),
            module_file: module.map(Path::to_path_buf),
            chain,
            kind,
        };

        self.diagnostics.push(diagnostic.clone());

        diagnostic
    }
}

fn placements(map: &Map) -> impl Iterator<Item = (Coord, usize, &Prefab)> {
    map.grid.iter().enumerate().flat_map(move |(z, level)| {
        level.iter().enumerate().flat_map(move |(row, cells)| {
            cells.iter().enumerate().flat_map(move |(x, key)| {
                let coord = Coord::new(x as u32 + 1, map.size.y - row as u32, z as u32 + 1);
                map.dictionary
                    .get(key)
                    .into_iter()
                    .flatten()
                    .enumerate()
                    .map(move |(index, prefab)| (coord, index, prefab))
            })
        })
    })
}

fn dm_path(path: &str) -> PathBuf { PathBuf::from(path.replace('\\', "/")) }

fn read(path: &Path) -> Result<String, ErrorKind> {
    fs::read_to_string(path).map_err(|error| ErrorKind::Read {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use core::arena::StrArena;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use dmm::Size;

    use super::*;
    use crate::maploader::{MapLoaderKind, map_module::discover, resolve};

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
            fs::write(&entry, include_str!("../../tests/fixtures/modular_maps/types.dm")).unwrap();
            Self {
                directory,
                tree: analyze(&entry),
            }
        }

        fn write(&self, path: &str, source: &str) {
            let path = self.directory.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, source).unwrap();
        }

        fn map(&self, path: &str, map: &Map) { self.write(path, &dmm::writer::write(map)); }

        fn path(&self, path: &str) -> PathBuf { resolved_path(&self.directory.join(path)) }

        fn discover(&self, map: &Map) -> map_module::Discovery {
            discover(
                &self.directory,
                &self.tree,
                Path::new("main.dmm"),
                map,
                Some("tgstation"),
            )
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.directory); }
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

    fn prefab(path: &str) -> Prefab { Prefab::new(TreePath::parse(path)) }

    fn root(key: &str) -> Prefab {
        let mut root = prefab("/obj/modular_map_root/fixture");
        root.set_var("key".into(), Value::Text(key.into()));
        root
    }

    fn map(size: [u32; 3], placed: &[(Coord, Prefab)]) -> Map {
        let mut map = Map::new(Size {
            x: size[0],
            y: size[1],
            z: size[2],
        });
        map.intern_tile(vec![prefab("/turf/floor"), prefab("/area/test")]);
        for (coord, prefab) in placed {
            let mut tile = map.tile_at(*coord).unwrap().clone();
            tile.push(prefab.clone());
            let key = map.intern_tile(tile);
            map.grid[coord.z as usize - 1][(map.size.y - coord.y) as usize][coord.x as usize - 1] = key;
        }
        map
    }

    fn leaf() -> Map { map([1, 1, 1], &[(Coord::new(1, 1, 1), prefab(CONNECTOR_PATH))]) }

    fn only_module(root: &map_module::Root) -> &map_module::MapModule {
        assert!(root.error.is_none(), "{:?}", root.error);
        assert_eq!(root.variants.len(), 1);
        root.variants[0].module.as_ref().unwrap()
    }

    #[test]
    fn owned_loaders_and_resolver_discover_with_fresh_caches() {
        let fixture = Fixture::new();
        fixture.write(
            "config.toml",
            "directory = 'maps'\n[rooms.leaf]\nmodules = ['leaf.dmm']",
        );
        fixture.map("maps/leaf.dmm", &leaf());
        let main = map([2, 1, 1], &[(Coord::new(2, 1, 1), root("leaf"))]);
        let loader = resolve("tgstation").expect("registered tgstation loader");
        assert!(matches!(&loader, MapLoaderKind::Tgstation(_)));
        assert!(resolve("unknown").is_none());
        assert!(resolve("").is_none());

        let first = loader.discover_modules(&fixture.directory, &fixture.tree, Path::new("main.dmm"), &main);
        assert!(first.diagnostics.is_empty());
        assert_eq!(only_module(&first.roots[0]).translation, [1, 0, 0]);

        fixture.map(
            "maps/leaf.dmm",
            &map([2, 1, 1], &[(Coord::new(2, 1, 1), prefab(CONNECTOR_PATH))]),
        );
        let second = loader.discover_modules(&fixture.directory, &fixture.tree, Path::new("main.dmm"), &main);
        assert!(second.diagnostics.is_empty());
        assert_eq!(only_module(&second.roots[0]).translation, [0, 0, 0]);
        assert_eq!(only_module(&first.roots[0]).map.size.x, 1);
        assert_eq!(only_module(&second.roots[0]).map.size.x, 2);

        let direct = TgMapLoader;
        let third = direct.discover_map_modules(&fixture.directory, &fixture.tree, Path::new("main.dmm"), &main);
        assert!(third.diagnostics.is_empty());
        assert_eq!(third.roots[0].location, second.roots[0].location);
        assert_eq!(third.roots[0].variants[0].path, second.roots[0].variants[0].path);
        assert_eq!(
            only_module(&third.roots[0]).translation,
            only_module(&second.roots[0]).translation
        );
    }

    #[test]
    fn discovery_preserves_variant_order_duplicates_and_separate_root_placements() {
        let fixture = Fixture::new();
        fixture.write(
            "config.toml",
            "directory = 'maps/'\n[rooms.leaf]\nmodules = ['b.dmm', 'a.dmm', 'b.dmm']",
        );
        fixture.map("maps/a.dmm", &leaf());
        fixture.map("maps/b.dmm", &leaf());
        let mut main = map(
            [2, 1, 1],
            &[
                (Coord::new(1, 1, 1), prefab("/obj/modular_map_root/fixture/inherited")),
                (Coord::new(1, 1, 1), root("leaf")),
                (Coord::new(2, 1, 1), root("leaf")),
            ],
        );
        main.intern_tile(vec![root("unplaced")]);
        let before = dmm::writer::write(&main);
        let result = fixture.discover(&main);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.roots.len(), 3);
        assert_eq!(result.roots[0].location.prefab_index, 2);
        assert_eq!(result.roots[1].location.prefab_index, 3);
        let variants = &result.roots[0].variants;
        assert_eq!(
            variants
                .iter()
                .map(|v| v.path.file_name().unwrap().to_str().unwrap())
                .collect::<Vec<_>>(),
            ["b.dmm", "a.dmm", "b.dmm"]
        );
        let first = variants[0].module.as_ref().unwrap();
        assert!(Arc::ptr_eq(&first.map, &variants[2].module.as_ref().unwrap().map));
        assert!(Arc::ptr_eq(
            &first.map,
            &result.roots[2].variants[0].module.as_ref().unwrap().map
        ));
        assert_eq!(dmm::writer::write(&main), before);
    }

    #[test]
    fn nested_variants_keep_their_children_and_compose_signed_offsets() {
        let fixture = Fixture::new();
        fixture.write(
            "config.toml",
            "directory = 'maps/'\n[rooms.parent]\nmodules = ['a.dmm', 'b.dmm']",
        );
        fixture.write(
            "nested/config.toml",
            "directory = 'maps/deep/'\n[rooms.middle]\nmodules = ['middle.dmm']\n[rooms.leaf]\nmodules = ['leaf.dmm']",
        );
        let mut middle_root = root("middle");
        middle_root.set_var("config_file".into(), Value::Text("nested/config.toml".into()));
        fixture.map(
            "maps/a.dmm",
            &map(
                [3, 4, 2],
                &[
                    (Coord::new(3, 2, 1), prefab("/obj/connector_alias")),
                    (Coord::new(1, 4, 2), middle_root),
                ],
            ),
        );
        fixture.map("maps/b.dmm", &leaf());
        let mut leaf_root = root("leaf");
        leaf_root.set_var("config_file".into(), Value::Text("nested/config.toml".into()));
        fixture.map(
            "maps/deep/middle.dmm",
            &map(
                [4, 3, 2],
                &[
                    (Coord::new(2, 3, 1), prefab(CONNECTOR_PATH)),
                    (Coord::new(4, 1, 2), leaf_root),
                ],
            ),
        );
        fixture.map(
            "maps/deep/leaf.dmm",
            &map([4, 4, 2], &[(Coord::new(4, 4, 1), prefab(CONNECTOR_PATH))]),
        );
        let main = map([1, 1, 3], &[(Coord::new(1, 1, 3), root("parent"))]);
        let result = fixture.discover(&main);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let parent = &result.roots[0];
        let a = parent.variants[0].module.as_ref().unwrap();
        let b = parent.variants[1].module.as_ref().unwrap();
        assert!(b.roots.is_empty());
        assert_eq!(a.connector, Coord::new(3, 2, 1));
        assert_eq!(a.translation, [-2, -1, 2]);
        let middle = only_module(&a.roots[0]);
        assert_eq!(a.roots[0].location.coord, Coord::new(1, 4, 2));
        assert_eq!(middle.translation, [-1, 1, 1]);
        let leaf = only_module(&middle.roots[0]);
        assert_eq!(leaf.translation, [0, -3, 1]);
        // Three placements compose to put the leaf's connector at (1,1,5) in the main map.
        let connector = [
            i64::from(leaf.connector.x),
            i64::from(leaf.connector.y),
            i64::from(leaf.connector.z),
        ];
        let global = std::array::from_fn::<_, 3, _>(|axis| {
            connector[axis] + a.translation[axis] + middle.translation[axis] + leaf.translation[axis]
        });
        assert_eq!(global, [1, 1, 5]);
    }

    #[test]
    fn root_overrides_and_parent_type_relationships_are_respected() {
        let fixture = Fixture::new();
        fixture.write(
            "override.toml",
            "directory = 'elsewhere'\n[rooms.override]\nmodules = ['leaf.dmm']",
        );
        fixture.map("elsewhere/leaf.dmm", &leaf());
        let mut alias = prefab("/obj/root_alias");
        alias.set_var("key".into(), Value::Text("override".into()));
        alias.set_var("config_file".into(), Value::Text("override.toml".into()));
        let result = fixture.discover(&map([1, 1, 1], &[(Coord::new(1, 1, 1), alias)]));
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert_eq!(result.roots[0].location.key.as_deref(), Some("override"));
        assert_eq!(
            result.roots[0].location.config_file,
            Some(fixture.path("override.toml"))
        );
        assert!(only_module(&result.roots[0]).roots.is_empty());
    }

    #[test]
    fn inactive_roots_and_disabled_loader_do_not_read_configs() {
        let fixture = Fixture::new();
        let mut null = root("leaf");
        null.set_var("config_file".into(), Value::Null);
        let mut empty_key = root("");
        empty_key.set_var("config_file".into(), Value::Text("missing.toml".into()));
        let mut empty_config = root("leaf");
        empty_config.set_var("config_file".into(), Value::Text(String::new()));
        let main = map(
            [1, 1, 1],
            &[
                (Coord::new(1, 1, 1), prefab(ROOT_PATH)),
                (Coord::new(1, 1, 1), prefab("/obj/modular_map_root/fixture")),
                (Coord::new(1, 1, 1), null),
                (Coord::new(1, 1, 1), empty_key),
                (Coord::new(1, 1, 1), empty_config),
            ],
        );
        let result = fixture.discover(&main);
        assert_eq!(result.roots.len(), 5);
        assert!(result.diagnostics.is_empty());
        assert!(
            result
                .roots
                .iter()
                .all(|root| root.error.is_none() && root.variants.is_empty())
        );
        let disabled = discover(&fixture.directory, &fixture.tree, Path::new("main.dmm"), &main, None);
        assert!(disabled.roots.is_empty() && disabled.diagnostics.is_empty());
        for name in ["unknown", ""] {
            let unsupported = discover(
                &fixture.directory,
                &fixture.tree,
                Path::new("main.dmm"),
                &main,
                Some(name),
            );
            assert!(unsupported.roots.is_empty());
            assert!(matches!(&unsupported.diagnostics[0].kind, ErrorKind::UnsupportedLoader(value) if value == name));
        }
    }

    #[test]
    fn invalid_fields_and_config_inputs_keep_valid_sibling_roots() {
        let fixture = Fixture::new();
        fixture.write(
            "config.toml",
            "directory = 'maps'\n[rooms.valid]\nmodules = ['leaf.dmm']\n[rooms.empty]\nmodules = \
             []\n[rooms.invalid]\nmodules = [42]",
        );
        fixture.map("maps/leaf.dmm", &leaf());
        fixture.write("syntax.toml", "directory = [");
        fixture.write("schema.toml", "directory = 42\nrooms = {}");
        let mut entries = ["valid", "missing", "empty", "invalid"]
            .into_iter()
            .map(root)
            .collect::<Vec<_>>();
        for file in ["syntax.toml", "schema.toml", "absent.toml"] {
            let mut root = root("valid");
            root.set_var("config_file".into(), Value::Text(file.into()));
            entries.push(root);
        }
        for (field, value) in [
            ("config_file", Value::Num(42.0)),
            ("key", Value::List(Vec::new())),
            ("key", Value::Unevaluated),
            ("config_file", Value::Resource("config.toml".into())),
        ] {
            let mut root = root("valid");
            root.set_var(field.into(), value);
            entries.push(root);
        }
        let main = map(
            [1, 1, 1],
            &entries
                .into_iter()
                .map(|p| (Coord::new(1, 1, 1), p))
                .collect::<Vec<_>>(),
        );
        let result = fixture.discover(&main);
        assert!(only_module(&result.roots[0]).roots.is_empty());
        assert_eq!(result.diagnostics.len(), 10);
        assert!(matches!(
            result.roots[1].error.as_ref().unwrap().kind,
            ErrorKind::MissingKey
        ));
        assert!(matches!(
            result.roots[2].error.as_ref().unwrap().kind,
            ErrorKind::EmptyModules
        ));
        for index in [3, 4, 5] {
            assert!(matches!(
                result.roots[index].error.as_ref().unwrap().kind,
                ErrorKind::Config(_)
            ));
        }
        assert!(matches!(
            result.roots[6].error.as_ref().unwrap().kind,
            ErrorKind::Read { .. }
        ));
        for root in &result.roots[7..] {
            assert!(matches!(root.error.as_ref().unwrap().kind, ErrorKind::InvalidField(_)));
        }
    }

    #[test]
    fn bad_modules_keep_valid_variants_and_report_nested_context() {
        let fixture = Fixture::new();
        fixture.write(
            "config.toml",
            "directory = 'maps'\n[rooms.outer]\nmodules = ['outer.dmm']\n[rooms.child]\nmodules = ['missing.dmm', \
             'invalid.dmm', 'zero.dmm', 'two.dmm', 'valid.dmm']",
        );
        fixture.map(
            "maps/outer.dmm",
            &map(
                [1, 1, 1],
                &[
                    (Coord::new(1, 1, 1), prefab(CONNECTOR_PATH)),
                    (Coord::new(1, 1, 1), root("child")),
                ],
            ),
        );
        fixture.write("maps/invalid.dmm", "this is not a DMM");
        let mut zero = map([1, 1, 1], &[]);
        zero.intern_tile(vec![prefab(CONNECTOR_PATH)]);
        fixture.map("maps/zero.dmm", &zero);
        fixture.map(
            "maps/two.dmm",
            &map(
                [2, 1, 1],
                &[
                    (Coord::new(1, 1, 1), prefab(CONNECTOR_PATH)),
                    (Coord::new(2, 1, 1), prefab(CONNECTOR_PATH)),
                ],
            ),
        );
        fixture.map("maps/valid.dmm", &leaf());
        let main = map([1, 1, 1], &[(Coord::new(1, 1, 1), root("outer"))]);
        let result = fixture.discover(&main);
        let child = &only_module(&result.roots[0]).roots[0];
        let failures = child.variants[..4]
            .iter()
            .map(|variant| variant.module.as_ref().unwrap_err())
            .collect::<Vec<_>>();
        assert!(matches!(failures[0].kind, ErrorKind::Read { .. }));
        assert!(matches!(failures[1].kind, ErrorKind::Dmm(_)));
        assert!(matches!(failures[2].kind, ErrorKind::Connectors { count: 0 }));
        assert!(matches!(failures[3].kind, ErrorKind::Connectors { count: 2 }));
        assert!(child.variants[4].module.is_ok());
        assert_eq!(result.diagnostics.len(), 4);
        let diagnostic = failures[0];
        let location = diagnostic.root.as_ref().unwrap();
        assert_eq!(location.source_map, fixture.path("maps/outer.dmm"));
        assert_eq!(location.coord, Coord::new(1, 1, 1));
        assert_eq!(location.prefab_index, 3);
        assert_eq!(location.config_file, Some(fixture.path("config.toml")));
        assert_eq!(location.key.as_deref(), Some("child"));
        assert_eq!(diagnostic.module_file, Some(fixture.path("maps/missing.dmm")));
        assert_eq!(
            diagnostic.chain,
            ["main.dmm", "maps/outer.dmm", "maps/missing.dmm"].map(|path| fixture.path(path))
        );
        let displayed = diagnostic.to_string();
        assert!(displayed.contains("outer.dmm") && displayed.contains("child") && displayed.contains("missing.dmm"));
    }

    #[test]
    fn cycles_use_active_ancestry_including_main_and_canonical_aliases() {
        let fixture = Fixture::new();
        fixture.write(
            "config.toml",
            "directory = 'maps'\n[rooms.start]\nmodules = ['a.dmm', 'a.dmm']\n[rooms.cycle]\nmodules = \
             ['../maps/a.dmm', '../main.dmm', 'leaf.dmm']",
        );
        fixture.map(
            "maps/a.dmm",
            &map(
                [1, 1, 1],
                &[
                    (Coord::new(1, 1, 1), prefab(CONNECTOR_PATH)),
                    (Coord::new(1, 1, 1), root("cycle")),
                ],
            ),
        );
        fixture.map("maps/leaf.dmm", &leaf());
        let main = map([1, 1, 1], &[(Coord::new(1, 1, 1), root("start"))]);
        fixture.map("main.dmm", &main);
        let result = fixture.discover(&main);
        assert_eq!(result.diagnostics.len(), 4);
        for variant in &result.roots[0].variants {
            let module = variant.module.as_ref().unwrap();
            let nested = &module.roots[0];
            for failed in &nested.variants[..2] {
                assert!(matches!(failed.module.as_ref().unwrap_err().kind, ErrorKind::Cycle));
            }
            assert!(nested.variants[2].module.is_ok());
        }
        assert!(Arc::ptr_eq(
            &result.roots[0].variants[0].module.as_ref().unwrap().map,
            &result.roots[0].variants[1].module.as_ref().unwrap().map
        ));
    }

    #[test]
    fn module_depth_limit_accepts_128_levels_and_rejects_the_next() {
        let fixture = Fixture::new();
        let mut config = String::from("directory = 'maps'\n");
        for index in 0..=MAX_NESTING {
            config.push_str(&format!("[rooms.level{index}]\nmodules = ['{index}.dmm']\n"));
            fixture.map(
                &format!("maps/{index}.dmm"),
                &map(
                    [1, 1, 1],
                    &[
                        (Coord::new(1, 1, 1), prefab(CONNECTOR_PATH)),
                        (Coord::new(1, 1, 1), root(&format!("level{}", index + 1))),
                    ],
                ),
            );
        }
        fixture.write("config.toml", &config);
        let result = fixture.discover(&map([1, 1, 1], &[(Coord::new(1, 1, 1), root("level0"))]));
        assert_eq!(result.diagnostics.len(), 1);
        assert!(matches!(result.diagnostics[0].kind, ErrorKind::NestingLimit));
        let mut nested = &result.roots[0];
        for _ in 0..MAX_NESTING {
            nested = &only_module(nested).roots[0];
        }
        let error = nested.variants[0].module.as_ref().unwrap_err();
        assert_eq!(error.chain.len(), MAX_NESTING + 2);
    }

    #[test]
    fn connector_z_does_not_shift_the_first_module_level() {
        let fixture = Fixture::new();
        fixture.write(
            "config.toml",
            "directory = 'maps'\n[rooms.leaf]\nmodules = ['leaf.dmm']",
        );
        fixture.map(
            "maps/leaf.dmm",
            &map([1, 1, 2], &[(Coord::new(1, 1, 2), prefab(CONNECTOR_PATH))]),
        );
        let result = fixture.discover(&map([1, 1, 3], &[(Coord::new(1, 1, 3), root("leaf"))]));
        assert_eq!(only_module(&result.roots[0]).translation, [0, 0, 2]);
    }

    #[test]
    #[ignore = "requires the local target/tgstation checkout"]
    fn tramstation_discovers_nested_attachments_and_duplicate_variants() {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/tgstation");
        // Analyze the checkout's root declaration; unrelated code and icons are unnecessary here.
        let mut tree = analyze(&directory.join("code/modules/mapping/map_specific_code/Tramstation.dm"));
        tree.register(&TreePath::parse(CONNECTOR_PATH), core::location::Location::default());
        let path = directory.join("_maps/map_files/tramstation/tramstation.dmm");
        let (map, errors) = dmm::parser::load(&path).unwrap();
        assert!(errors.is_empty(), "{errors:?}");
        let result = discover(&directory, &tree, &path, &map, Some("tgstation"));
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(!result.roots.is_empty());
        let medsci = result
            .roots
            .iter()
            .find(|root| root.location.key.as_deref() == Some("medsciupper"))
            .unwrap();
        assert_eq!(medsci.variants.len(), 3);
        for variant in &medsci.variants {
            let module = variant.module.as_ref().unwrap();
            assert!(
                module
                    .roots
                    .iter()
                    .any(|root| root.location.key.as_deref() == Some("medsciupper_attachment_a"))
            );
            assert!(
                module
                    .roots
                    .iter()
                    .any(|root| root.location.key.as_deref() == Some("medsciupper_attachment_b"))
            );
        }
        let repeated = result
            .roots
            .iter()
            .find(|root| root.location.key.as_deref() == Some("secservicelower"))
            .unwrap();
        assert_eq!(repeated.variants.len(), 3);
        assert!(
            repeated
                .variants
                .iter()
                .all(|variant| variant.path == repeated.variants[0].path)
        );
        assert!(Arc::ptr_eq(
            &repeated.variants[0].module.as_ref().unwrap().map,
            &repeated.variants[1].module.as_ref().unwrap().map
        ));
    }
}
