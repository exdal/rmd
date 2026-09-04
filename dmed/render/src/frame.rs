use std::collections::{BTreeMap, BTreeSet, HashMap};

use dmi::metadata::{Dir, Metadata};
use dmm::{Coord, Map};
use editor::visual::{self, Appearance};
use objtree::ObjectTree;

use crate::{SpriteInstance, instance_for, texture::TextureCatalog};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameOptions {
    pub show_areas: bool,
    /// `world.icon_size`
    pub tile_size: u32,
}

impl Default for FrameOptions {
    fn default() -> Self {
        Self {
            show_areas: false,
            tile_size: 32,
        }
    }
}

pub fn build(
    tree: &ObjectTree, icons: &HashMap<String, Metadata>, textures: &TextureCatalog, map: &Map, z: u32,
    options: &FrameOptions,
) -> Vec<SpriteInstance> {
    let mut sprites: Vec<((i32, i32, usize), SpriteInstance)> = Vec::new();
    let mut order = 0usize;
    let area = tree.roots().area;

    for y in (1..=map.size.y).rev() {
        for x in 1..=map.size.x {
            let Some(tile) = map.tile_at(Coord::new(x, y, z)) else {
                continue;
            };

            for prefab in tile {
                let Some(id) = tree.id_of(&prefab.path) else {
                    continue;
                };

                if !options.show_areas && area.is_some_and(|area| tree.is_subtype_of(id, area)) {
                    continue;
                }

                let appearance = visual::resolve_id(tree, id, prefab);
                order = order.saturating_add(1);

                let Some(texture) = sprite_texture(icons, textures, &appearance) else {
                    continue;
                };

                sprites.push((
                    visual::sort_key(&appearance, order),
                    instance_for(&appearance, texture, x, y, options.tile_size),
                ));
            }
        }
    }

    sprites.sort_by_key(|(key, _)| *key);

    sprites.into_iter().map(|(_, instance)| instance).collect()
}

pub fn icons_used(tree: &ObjectTree, map: &Map) -> BTreeMap<String, BTreeSet<String>> {
    let mut used: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for tile in map.dictionary.values() {
        for prefab in tile {
            let Some(id) = tree.id_of(&prefab.path) else {
                continue;
            };

            let appearance = visual::resolve_id(tree, id, prefab);
            let Some(icon) = appearance.icon else {
                continue;
            };

            used.entry(icon)
                .or_default()
                .insert(appearance.icon_state.unwrap_or_default());
        }
    }

    used
}

fn sprite_texture(
    icons: &HashMap<String, Metadata>, textures: &TextureCatalog, appearance: &Appearance,
) -> Option<crate::SpriteTexture> {
    let icon = appearance.icon.as_deref()?;
    let state = appearance.icon_state.as_deref().unwrap_or("");
    let metadata = icons.get(icon)?;
    let dir = Dir::from_bits(appearance.dir).unwrap_or(Dir::South);
    let cell = metadata.find(state)?.sprite_index(dir, 0);

    textures.lookup(icon, cell)
}

#[cfg(test)]
mod tests {
    use core::{
        location::Location,
        path::TreePath,
        types::{Identifier, Value, VarModifiers},
    };
    use std::{collections::HashMap, path::PathBuf};

    use dmi::{
        IconFile,
        metadata::{IconState, Metadata},
    };
    use dmm::{Map, Prefab, Size};
    use objtree::{ObjectTree, VarDecl};

    use crate::{
        frame::{FrameOptions, build},
        texture::TextureCatalog,
    };

    const ICON: &str = "test.dmi";

    fn icon_file(states: &[&str]) -> IconFile {
        let cells = states.len() as u32;

        IconFile {
            path: PathBuf::from(ICON),
            metadata: metadata(states),
            sheet_width: 32 * cells,
            sheet_height: 32,
            pixels: vec![255; (32 * cells * 32 * 4) as usize],
        }
    }

    fn metadata(states: &[&str]) -> Metadata {
        Metadata {
            version: String::from("4.0"),
            width: 32,
            height: 32,
            states: states
                .iter()
                .enumerate()
                .map(|(index, name)| IconState {
                    name: String::from(*name),
                    dirs: 1,
                    frames: 1,
                    offset: index,
                    ..Default::default()
                })
                .collect(),
        }
    }

    /// `/obj/thing { icon = 'test.dmi'; icon_state = ...; layer = ... }`
    fn tree(types: &[(&str, &str, f32)]) -> ObjectTree {
        let mut tree = ObjectTree::new();

        for (path, state, layer) in types {
            let id = tree.register(&TreePath::parse(path), Location::default());
            let Some(decl) = tree.get_mut(id) else {
                continue;
            };

            let mut var = |name: &str, value: Value| {
                let name = Identifier(String::from(name));
                decl.vars.insert(
                    name.clone(),
                    VarDecl {
                        name,
                        declared_type: None,
                        modifiers: VarModifiers::default(),
                        value,
                        location: Location::default(),
                    },
                );
            };

            var("icon", Value::Resource(String::from(ICON)));
            var("icon_state", Value::Text(String::from(*state)));
            var("layer", Value::Num(*layer));
        }

        tree
    }

    fn one_tile_map(paths: &[&str]) -> Map {
        let mut map = Map::new(Size { x: 1, y: 1, z: 1 });
        let tile = paths.iter().map(|p| Prefab::new(TreePath::parse(p))).collect();
        let key = map.intern_tile(tile);
        if let Some(slot) = map
            .grid
            .get_mut(0)
            .and_then(|z| z.get_mut(0))
            .and_then(|r| r.get_mut(0))
        {
            *slot = key;
        }

        map
    }

    fn icons(states: &[&str]) -> HashMap<String, Metadata> { HashMap::from([(String::from(ICON), metadata(states))]) }

    fn textures(states: &[&str]) -> TextureCatalog {
        let mut textures = TextureCatalog::new();
        textures.insert(ICON, &icon_file(states)).expect("insert");

        textures
    }

    #[test]
    fn orders_sprites_by_layer_not_by_prefab_order() {
        let tree = tree(&[("/turf/floor", "floor", 2.0), ("/obj/table", "table", 3.0)]);
        // The map lists the high-layer object first; the frame must still draw it last.
        let map = one_tile_map(&["/obj/table", "/turf/floor"]);

        let sprites = build(
            &tree,
            &icons(&["floor", "table"]),
            &textures(&["floor", "table"]),
            &map,
            1,
            &FrameOptions::default(),
        );

        assert_eq!(sprites.len(), 2);
        // "floor" is cell 0, "table" is cell 1.
        assert_eq!(sprites[0].texture.index, 0);
        assert_eq!(sprites[1].texture.index, 1);
    }

    #[test]
    fn skips_areas_unless_asked() {
        let tree = tree(&[("/turf/floor", "floor", 2.0), ("/area/station", "floor", 1.0)]);
        let map = one_tile_map(&["/turf/floor", "/area/station"]);
        let (icons, textures) = (icons(&["floor"]), textures(&["floor"]));

        let hidden = build(&tree, &icons, &textures, &map, 1, &FrameOptions::default());
        assert_eq!(hidden.len(), 1);

        let shown = build(
            &tree,
            &icons,
            &textures,
            &map,
            1,
            &FrameOptions {
                show_areas: true,
                ..Default::default()
            },
        );
        assert_eq!(shown.len(), 2);
    }

    #[test]
    fn an_unresolvable_icon_state_emits_nothing() {
        let tree = tree(&[("/obj/ghost", "not_in_the_sheet", 2.0)]);
        let map = one_tile_map(&["/obj/ghost"]);

        let sprites = build(
            &tree,
            &icons(&["floor"]),
            &textures(&["floor"]),
            &map,
            1,
            &FrameOptions::default(),
        );

        assert!(sprites.is_empty());
    }

    #[test]
    fn a_prefab_with_no_type_in_the_tree_emits_nothing() {
        let map = one_tile_map(&["/obj/never/declared"]);

        let sprites = build(
            &ObjectTree::new(),
            &icons(&["floor"]),
            &textures(&["floor"]),
            &map,
            1,
            &FrameOptions::default(),
        );

        assert!(sprites.is_empty());
    }

    #[test]
    fn tiles_land_on_pixel_positions_from_their_coordinates() {
        let tree = tree(&[("/turf/floor", "floor", 2.0)]);
        let mut map = Map::new(Size { x: 2, y: 2, z: 1 });
        // The grid starts filled with the default key, so claim it for an empty tile first.
        map.intern_tile(Vec::new());
        let key = map.intern_tile(vec![Prefab::new(TreePath::parse("/turf/floor"))]);
        // (2, 1) is the bottom right tile, so grid row 1, column 1.
        if let Some(slot) = map
            .grid
            .get_mut(0)
            .and_then(|z| z.get_mut(1))
            .and_then(|r| r.get_mut(1))
        {
            *slot = key;
        }

        let sprites = build(
            &tree,
            &icons(&["floor"]),
            &textures(&["floor"]),
            &map,
            1,
            &FrameOptions::default(),
        );

        assert_eq!(sprites.len(), 1);
        assert_eq!((sprites[0].x, sprites[0].y), (32.0, 0.0));
    }
}

/// End to end over `examples/env`, which is the only place the whole chain - preprocess, parse,
/// lower, pack, resolve - is exercised without a GPU.
#[cfg(test)]
mod example_environment {
    use std::path::PathBuf;

    use dmi::IconFile;
    use editor::Environment;

    use crate::{
        frame::{FrameOptions, build},
        texture::TextureCatalog,
    };

    fn examples() -> PathBuf {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/env");

        path.canonicalize().unwrap_or(path)
    }

    #[test]
    fn the_example_map_resolves_to_sprites() {
        let root = examples();
        let (environment, diagnostics) = Environment::load(root.join("test.dme")).expect("load");

        assert!(diagnostics.icons.is_empty(), "{:?}", diagnostics.icons);
        assert!(environment.icons.contains_key("icons/test.dmi"));
        // `resources` holds paths the preprocessor already resolved against the includer.
        assert_eq!(environment.maps, vec![root.join("test.dmm")]);

        let mut textures = TextureCatalog::default();
        let file = IconFile::load(root.join("icons/test.dmi")).expect("icon");
        textures.insert("icons/test.dmi", &file).expect("insert");

        let source = std::fs::read_to_string(root.join("test.dmm")).expect("map");
        let (map, errors) = dmm::parser::parse(&source);
        assert!(errors.is_empty(), "{errors:?}");

        let sprites = build(
            &environment.tree,
            &environment.icons,
            &textures,
            &map,
            1,
            &FrameOptions::default(),
        );

        // 8x6 turfs, plus three tables and one light, with the four areas skipped.
        assert_eq!(sprites.len(), 48 + 4);

        // Every sprite found a real cell, and the tinted table came through premultiplied.
        assert!(sprites.iter().all(|s| s.texture.width == 32 && s.texture.height == 32));
        assert!(sprites.iter().any(|s| s.color[0] > s.color[1]));
    }

    #[test]
    fn turfs_sort_under_objects() {
        let root = examples();
        let (environment, _) = Environment::load(root.join("test.dme")).expect("load");

        let mut textures = TextureCatalog::default();
        let file = IconFile::load(root.join("icons/test.dmi")).expect("icon");
        textures.insert("icons/test.dmi", &file).expect("insert");

        let source = std::fs::read_to_string(root.join("test.dmm")).expect("map");
        let (map, _) = dmm::parser::parse(&source);
        let sprites = build(
            &environment.tree,
            &environment.icons,
            &textures,
            &map,
            1,
            &FrameOptions::default(),
        );

        // `/turf` is layer 2 and `/obj` is 3, so the last 4 drawn are all objects. Cell 0 is
        // "floor" and cell 1 is "wall", so no turf may appear after the first object.
        let first_object = sprites
            .iter()
            .position(|sprite| sprite.texture.index >= 2)
            .expect("an object sprite");

        assert!(sprites[first_object..].iter().all(|sprite| sprite.texture.index >= 2));
    }

    /// Packing only what [`icons_used`](crate::frame::icons_used) names must not cost a sprite.
    #[test]
    fn a_selective_pack_draws_the_same_frame_as_a_whole_one() {
        use std::collections::BTreeSet;

        use crate::frame::icons_used;

        let root = examples();
        let (environment, _) = Environment::load(root.join("test.dme")).expect("load");
        let file = IconFile::load(root.join("icons/test.dmi")).expect("icon");

        let source = std::fs::read_to_string(root.join("test.dmm")).expect("map");
        let (map, errors) = dmm::parser::parse(&source);
        assert!(errors.is_empty(), "{errors:?}");

        let used = icons_used(&environment.tree, &map);
        assert!(used.contains_key("icons/test.dmi"));

        let mut whole = TextureCatalog::default();
        whole.insert("icons/test.dmi", &file).expect("insert");

        let mut selective = TextureCatalog::default();
        for (icon, states) in &used {
            let states: BTreeSet<&str> = states.iter().map(String::as_str).collect();
            selective.insert_states(icon, &file, &states).expect("insert");
        }

        assert!(selective.len() <= whole.len());

        let frame = |textures: &TextureCatalog| {
            build(
                &environment.tree,
                &environment.icons,
                textures,
                &map,
                1,
                &FrameOptions::default(),
            )
        };

        assert_eq!(frame(&selective), frame(&whole));
    }
}
