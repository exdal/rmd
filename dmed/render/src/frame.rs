use std::collections::HashMap;

use dmi::metadata::{Dir, Metadata};
use dmm::{Coord, Map, Prefab};
use editor::{
    document::MapDocument,
    visual::{self, Appearance},
};
use objtree::{ObjectTree, TypeId};

use crate::{
    AREA_EDGE_EAST,
    AREA_EDGE_NORTH,
    AREA_EDGE_SOUTH,
    AREA_EDGE_WEST,
    SpriteInstance,
    instance_for,
    texture::TextureCatalog,
};

#[derive(Debug, Default)]
pub struct FrameInstances {
    pub sprites: Vec<SpriteInstance>,
    pub area_tiles: Vec<SpriteInstance>,
}

impl std::ops::Deref for FrameInstances {
    type Target = [SpriteInstance];

    fn deref(&self) -> &Self::Target { &self.sprites }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameOptions {
    pub show_areas: bool,
    pub show_area_outlines: bool,
    /// `world.icon_size`
    pub tile_size: u32,
    /// How many levels below the active one to draw behind it, 0 for none.
    pub underlay_depth: u32,
}

impl Default for FrameOptions {
    fn default() -> Self {
        Self {
            show_areas: false,
            show_area_outlines: false,
            tile_size: 32,
            underlay_depth: 3,
        }
    }
}

pub fn build(
    tree: &ObjectTree, icons: &HashMap<String, Metadata>, textures: &TextureCatalog, document: &MapDocument,
    tile_size: u32,
) -> FrameInstances {
    let mut sprite_instances = Vec::new();
    let mut area_tiles = Vec::new();
    let area = tree.roots().area;
    let map = &document.map;

    for z in 1..=map.size.z.max(1) {
        let mut level: Vec<((i32, i32, usize), SpriteInstance)> = Vec::new();
        let mut order = 0usize;

        for y in (1..=map.size.y).rev() {
            for x in 1..=map.size.x {
                let Some(tile) = map.tile_at(Coord::new(x, y, z)) else {
                    continue;
                };

                for (prefab_index, prefab) in tile.iter().enumerate() {
                    let Some(owner) = document.instance_ids_at(Coord::new(x, y, z)).get(prefab_index).copied() else {
                        continue;
                    };
                    let Some(id) = tree.id_of(&prefab.path) else {
                        continue;
                    };

                    let is_area = area.is_some_and(|area| tree.is_subtype_of(id, area));
                    let appearance = visual::resolve_id(tree, id, prefab);
                    order = order.saturating_add(1);

                    if is_area {
                        let texture = sprite_texture(icons, textures, &appearance);
                        let sort_key = visual::sort_key(&appearance, order);

                        if let Some(texture) = texture {
                            level.push((
                                sort_key,
                                instance_for(owner, &appearance, texture, Coord::new(x, y, z), tile_size, true),
                            ));
                        }

                        let mut hover_outline = instance_for(
                            owner,
                            &appearance,
                            texture.unwrap_or_default(),
                            Coord::new(x, y, z),
                            tile_size,
                            true,
                        );
                        hover_outline.x = (x.saturating_sub(1) * tile_size) as f32;
                        hover_outline.y = (y.saturating_sub(1) * tile_size) as f32;
                        hover_outline.width = tile_size as f32;
                        hover_outline.height = tile_size as f32;
                        hover_outline.area_edges = crate::AREA_EDGES_ALL;
                        hover_outline.color = [1.0; 4];
                        area_tiles.push(hover_outline);

                        let edges = area.map_or(0, |area| area_edges(tree, area, map, prefab, Coord::new(x, y, z)));
                        if edges == 0 {
                            continue;
                        }

                        let texture = texture.unwrap_or_default();
                        let mut instance =
                            instance_for(owner, &appearance, texture, Coord::new(x, y, z), tile_size, true);
                        instance.x = (x.saturating_sub(1) * tile_size) as f32;
                        instance.y = (y.saturating_sub(1) * tile_size) as f32;
                        instance.width = tile_size as f32;
                        instance.height = tile_size as f32;
                        instance.area_edges = edges;
                        // The renderer replaces this with the resolved icon cell's averaged RGB.
                        instance.color = [1.0; 4];

                        level.push((sort_key, instance));
                        continue;
                    }

                    let Some(texture) = sprite_texture(icons, textures, &appearance) else {
                        continue;
                    };

                    level.push((
                        visual::sort_key(&appearance, order),
                        instance_for(owner, &appearance, texture, Coord::new(x, y, z), tile_size, false),
                    ));
                }
            }
        }

        level.sort_by_key(|(key, _)| *key);
        sprite_instances.extend(level.into_iter().map(|(_, instance)| instance));
    }

    FrameInstances {
        sprites: sprite_instances,
        area_tiles,
    }
}

fn area_edges(tree: &ObjectTree, area: TypeId, map: &Map, prefab: &Prefab, coord: Coord) -> u32 {
    let matches = |coord| area_prefab_at(tree, area, map, coord).is_some_and(|other| same_area(prefab, other));
    let mut edges = 0;

    if coord.y >= map.size.y || !matches(Coord::new(coord.x, coord.y + 1, coord.z)) {
        edges |= AREA_EDGE_NORTH;
    }
    if coord.x >= map.size.x || !matches(Coord::new(coord.x + 1, coord.y, coord.z)) {
        edges |= AREA_EDGE_EAST;
    }
    if coord.y <= 1 || !matches(Coord::new(coord.x, coord.y - 1, coord.z)) {
        edges |= AREA_EDGE_SOUTH;
    }
    if coord.x <= 1 || !matches(Coord::new(coord.x - 1, coord.y, coord.z)) {
        edges |= AREA_EDGE_WEST;
    }

    edges
}

fn area_prefab_at<'a>(tree: &ObjectTree, area: TypeId, map: &'a Map, coord: Coord) -> Option<&'a Prefab> {
    map.tile_at(coord)?.iter().find(|prefab| {
        tree.id_of(&prefab.path)
            .is_some_and(|candidate| tree.is_subtype_of(candidate, area))
    })
}

/// Source spelling and variable order do not split otherwise identical area definitions.
fn same_area(left: &Prefab, right: &Prefab) -> bool {
    left.path == right.path
        && left.vars.len() == right.vars.len()
        && left
            .vars
            .iter()
            .all(|(name, value)| right.var(name).is_some_and(|other| other == &value.value))
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
    use editor::document::MapDocument;
    use objtree::{ObjectTree, VarDecl};

    use crate::{
        AREA_EDGE_EAST,
        AREA_EDGE_NORTH,
        AREA_EDGE_SOUTH,
        AREA_EDGE_WEST,
        AREA_EDGES_ALL,
        frame::build,
        texture::TextureCatalog,
    };

    const ICON: &str = "test.dmi";

    fn document(map: Map) -> MapDocument { MapDocument::new(map, 1) }

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

    fn area_map(width: u32, height: u32, path: &str) -> Map {
        let mut map = Map::new(Size {
            x: width,
            y: height,
            z: 1,
        });
        let key = map.intern_tile(vec![Prefab::new(TreePath::parse(path))]);
        for row in &mut map.grid[0] {
            row.fill(key);
        }

        map
    }

    /// One tile per z level, each holding the type named for it.
    fn layered_map(levels: &[&str]) -> Map {
        let mut map = Map::new(Size {
            x: 1,
            y: 1,
            z: levels.len() as u32,
        });

        for (index, path) in levels.iter().enumerate() {
            let key = map.intern_tile(vec![Prefab::new(TreePath::parse(path))]);
            if let Some(slot) = map
                .grid
                .get_mut(index)
                .and_then(|z| z.get_mut(0))
                .and_then(|r| r.get_mut(0))
            {
                *slot = key;
            }
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
        let document = document(one_tile_map(&["/obj/table", "/turf/floor"]));
        let owners = document.instance_ids_at(dmm::Coord::new(1, 1, 1));

        let sprites = build(
            &tree,
            &icons(&["floor", "table"]),
            &textures(&["floor", "table"]),
            &document,
            32,
        );

        assert_eq!(sprites.len(), 2);
        // "floor" is cell 0 and "table" is cell 1 within the same sheet.
        assert_eq!(
            sprites[0].texture,
            textures(&["floor", "table"]).lookup(ICON, 0).unwrap()
        );
        assert_eq!(
            sprites[1].texture,
            textures(&["floor", "table"]).lookup(ICON, 1).unwrap()
        );
        assert_eq!([sprites[0].owner, sprites[1].owner], [owners[1], owners[0]]);
    }

    #[test]
    fn includes_normal_area_sprites_and_separate_outlines() {
        let tree = tree(&[("/turf/floor", "floor", 2.0), ("/area/station", "floor", 1.0)]);
        let document = document(one_tile_map(&["/turf/floor", "/area/station"]));
        let (icons, textures) = (icons(&["floor"]), textures(&["floor"]));

        let sprites = build(&tree, &icons, &textures, &document, 32);

        assert_eq!(sprites.len(), 3);
        assert_eq!(sprites.iter().filter(|sprite| sprite.is_area).count(), 2);
        assert_eq!(sprites.iter().filter(|sprite| !sprite.is_area).count(), 1);
        let area = sprites
            .iter()
            .find(|sprite| sprite.is_area && sprite.area_edges == 0)
            .unwrap();
        let outline = sprites.iter().find(|sprite| sprite.area_edges != 0).unwrap();
        assert_eq!(area.texture, textures.lookup(ICON, 0).unwrap());
        assert_eq!(outline.area_edges, AREA_EDGES_ALL);
        assert_eq!((outline.width, outline.height), (32.0, 32.0));
        assert_eq!((sprites[0].is_area, sprites[0].area_edges), (true, 0));
        assert_eq!((sprites[1].is_area, sprites[1].area_edges), (true, AREA_EDGES_ALL));
        assert_eq!(sprites.area_tiles.len(), 1);
        assert_eq!(sprites.area_tiles[0].owner, area.owner);
        assert_eq!(sprites.area_tiles[0].area_edges, AREA_EDGES_ALL);
    }

    #[test]
    fn matching_area_tiles_only_draw_the_outer_perimeter() {
        let tree = tree(&[("/area/station", "floor", 1.0)]);
        let (icons, textures) = (icons(&["floor"]), textures(&["floor"]));

        let sprites = build(&tree, &icons, &textures, &document(area_map(3, 3, "/area/station")), 48);

        assert_eq!(sprites.len(), 9 + 8);
        assert_eq!(
            sprites
                .iter()
                .filter(|sprite| sprite.is_area && sprite.area_edges == 0)
                .count(),
            9
        );
        assert_eq!(sprites.iter().filter(|sprite| sprite.area_edges != 0).count(), 8);
        assert_eq!(sprites.area_tiles.len(), 9);
        assert!(
            sprites
                .area_tiles
                .iter()
                .all(|sprite| sprite.area_edges == AREA_EDGES_ALL)
        );
        assert!(
            sprites
                .area_tiles
                .iter()
                .any(|sprite| (sprite.x, sprite.y) == (48.0, 48.0))
        );
        assert!(
            sprites
                .iter()
                .any(|sprite| sprite.area_edges == 0 && (sprite.x, sprite.y) == (48.0, 48.0))
        );
        assert!(
            !sprites
                .iter()
                .any(|sprite| sprite.area_edges != 0 && (sprite.x, sprite.y) == (48.0, 48.0))
        );

        let bottom_left = sprites
            .iter()
            .find(|sprite| sprite.area_edges != 0 && (sprite.x, sprite.y) == (0.0, 0.0))
            .unwrap();
        assert_eq!(bottom_left.area_edges, AREA_EDGE_SOUTH | AREA_EDGE_WEST);
        assert_eq!((bottom_left.width, bottom_left.height), (48.0, 48.0));

        let top_middle = sprites
            .iter()
            .find(|sprite| sprite.area_edges != 0 && (sprite.x, sprite.y) == (48.0, 96.0))
            .unwrap();
        assert_eq!(top_middle.area_edges, AREA_EDGE_NORTH);
    }

    #[test]
    fn touching_different_areas_keep_both_sides_of_the_shared_edge() {
        let tree = tree(&[("/area/one", "floor", 1.0), ("/area/two", "floor", 1.0)]);
        let mut map = Map::new(Size { x: 2, y: 1, z: 1 });
        let left = map.intern_tile(vec![Prefab::new(TreePath::parse("/area/one"))]);
        let right = map.intern_tile(vec![Prefab::new(TreePath::parse("/area/two"))]);
        map.grid[0][0] = vec![left, right];
        let (icons, textures) = (icons(&["floor"]), textures(&["floor"]));

        let sprites = build(&tree, &icons, &textures, &document(map), 32);
        let left = sprites
            .iter()
            .find(|sprite| sprite.area_edges != 0 && sprite.x == 0.0)
            .unwrap();
        let right = sprites
            .iter()
            .find(|sprite| sprite.area_edges != 0 && sprite.x == 32.0)
            .unwrap();

        assert_ne!(left.area_edges & AREA_EDGE_EAST, 0);
        assert_ne!(right.area_edges & AREA_EDGE_WEST, 0);
    }

    #[test]
    fn matching_neighbors_remove_both_sides_of_the_shared_edge() {
        let tree = tree(&[("/area/station", "floor", 1.0)]);
        let (icons, textures) = (icons(&["floor"]), textures(&["floor"]));

        let sprites = build(&tree, &icons, &textures, &document(area_map(2, 1, "/area/station")), 32);
        let left = sprites
            .iter()
            .find(|sprite| sprite.area_edges != 0 && sprite.x == 0.0)
            .unwrap();
        let right = sprites
            .iter()
            .find(|sprite| sprite.area_edges != 0 && sprite.x == 32.0)
            .unwrap();

        assert_eq!(left.area_edges, AREA_EDGE_NORTH | AREA_EDGE_SOUTH | AREA_EDGE_WEST);
        assert_eq!(right.area_edges, AREA_EDGE_NORTH | AREA_EDGE_EAST | AREA_EDGE_SOUTH);
    }

    #[test]
    fn holes_receive_an_inner_perimeter() {
        let tree = tree(&[("/area/station", "floor", 1.0)]);
        let mut map = Map::new(Size { x: 3, y: 3, z: 1 });
        let empty = map.intern_tile(Vec::new());
        let area = map.intern_tile(vec![Prefab::new(TreePath::parse("/area/station"))]);
        for row in &mut map.grid[0] {
            row.fill(area);
        }
        map.grid[0][1][1] = empty;
        let (icons, textures) = (icons(&["floor"]), textures(&["floor"]));

        let sprites = build(&tree, &icons, &textures, &document(map), 32);
        let west_of_hole = sprites
            .iter()
            .find(|sprite| sprite.area_edges != 0 && (sprite.x, sprite.y) == (0.0, 32.0))
            .unwrap();
        let east_of_hole = sprites
            .iter()
            .find(|sprite| sprite.area_edges != 0 && (sprite.x, sprite.y) == (64.0, 32.0))
            .unwrap();
        let south_of_hole = sprites
            .iter()
            .find(|sprite| sprite.area_edges != 0 && (sprite.x, sprite.y) == (32.0, 0.0))
            .unwrap();
        let north_of_hole = sprites
            .iter()
            .find(|sprite| sprite.area_edges != 0 && (sprite.x, sprite.y) == (32.0, 64.0))
            .unwrap();

        assert_ne!(west_of_hole.area_edges & AREA_EDGE_EAST, 0);
        assert_ne!(east_of_hole.area_edges & AREA_EDGE_WEST, 0);
        assert_ne!(south_of_hole.area_edges & AREA_EDGE_NORTH, 0);
        assert_ne!(north_of_hole.area_edges & AREA_EDGE_SOUTH, 0);
    }

    #[test]
    fn area_identity_uses_values_instead_of_source_order() {
        let mut left = Prefab::new(TreePath::parse("/area/station"));
        left.set_var("name".into(), Value::Text(String::from("Bridge")));
        left.set_var("icon_state".into(), Value::Text(String::from("bridge")));
        let mut right = Prefab::new(TreePath::parse("/area/station"));
        right.set_var("icon_state".into(), Value::Text(String::from("bridge")));
        right.set_var("name".into(), Value::Text(String::from("Bridge")));

        assert!(super::same_area(&left, &right));
        right.set_var("name".into(), Value::Text(String::from("Engineering")));
        assert!(!super::same_area(&left, &right));
    }

    #[test]
    fn an_area_with_an_unresolvable_icon_still_gets_a_white_outline() {
        let tree = tree(&[("/area/station", "missing", 1.0)]);
        let document = document(one_tile_map(&["/area/station"]));

        let sprites = build(&tree, &icons(&["floor"]), &textures(&["floor"]), &document, 32);

        assert_eq!(sprites.len(), 1);
        assert!(sprites[0].is_area);
        assert_eq!(sprites[0].area_edges, AREA_EDGES_ALL);
        assert_eq!(sprites[0].texture, Default::default());
        assert_eq!(sprites[0].color, [1.0; 4]);
        assert_eq!(sprites.area_tiles.len(), 1);
        assert_eq!(sprites.area_tiles[0].texture, Default::default());
        assert_eq!(sprites.area_tiles[0].color, [1.0; 4]);
    }

    #[test]
    fn an_unresolvable_icon_state_emits_nothing() {
        let tree = tree(&[("/obj/ghost", "not_in_the_sheet", 2.0)]);
        let document = document(one_tile_map(&["/obj/ghost"]));

        let sprites = build(&tree, &icons(&["floor"]), &textures(&["floor"]), &document, 32);

        assert!(sprites.is_empty());
        assert_eq!(document.instance_ids_at(dmm::Coord::new(1, 1, 1)).len(), 1);
    }

    #[test]
    fn a_prefab_with_no_type_in_the_tree_emits_nothing() {
        let document = document(one_tile_map(&["/obj/never/declared"]));

        let sprites = build(
            &ObjectTree::new(),
            &icons(&["floor"]),
            &textures(&["floor"]),
            &document,
            32,
        );

        assert!(sprites.is_empty());
    }

    /// `/turf/one` on z 1, `/turf/two` on z 2, `/turf/three` on z 3, one cell each.
    fn layered() -> (ObjectTree, HashMap<String, Metadata>, TextureCatalog, Map) {
        let states = ["one", "two", "three"];
        let tree = tree(&[
            ("/turf/one", "one", 2.0),
            ("/turf/two", "two", 2.0),
            ("/turf/three", "three", 2.0),
        ]);

        (
            tree,
            icons(&states),
            textures(&states),
            layered_map(&["/turf/one", "/turf/two", "/turf/three"]),
        )
    }

    #[test]
    fn builds_every_level_deepest_first() {
        let (tree, icons, textures, map) = layered();
        let sprites = build(&tree, &icons, &textures, &document(map), 32);

        assert_eq!(sprites.len(), 3);
        assert_eq!(sprites.iter().map(|sprite| sprite.z).collect::<Vec<_>>(), [1, 2, 3]);
        assert_eq!(
            sprites.iter().map(|sprite| sprite.texture).collect::<Vec<_>>(),
            (0..3)
                .map(|cell| textures.lookup(ICON, cell).unwrap())
                .collect::<Vec<_>>()
        );
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

        let sprites = build(&tree, &icons(&["floor"]), &textures(&["floor"]), &document(map), 32);

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
    use editor::{Environment, document::MapDocument};

    use crate::{frame::build, texture::TextureCatalog};

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

        let document = MapDocument::new(map, 1);
        let sprites = build(&environment.tree, &environment.icons, &textures, &document, 32);

        // 8x6 turfs, three tables, one light, and the 24 tiles around the area's perimeter.
        assert_eq!(sprites.len(), 48 + 4 + 24);

        // Drawable atoms found a real cell; the iconless area still uses tile-sized outline geometry.
        assert!(
            sprites
                .iter()
                .filter(|sprite| !sprite.is_area)
                .all(|sprite| sprite.texture.width == 32 && sprite.texture.height == 32)
        );
        assert!(
            sprites
                .iter()
                .filter(|sprite| sprite.is_area)
                .all(|sprite| (sprite.width, sprite.height) == (32.0, 32.0))
        );
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
        let document = MapDocument::new(map, 1);
        let sprites = build(&environment.tree, &environment.icons, &textures, &document, 32);

        // Neither `/turf` nor `/obj` declares a layer, so this is `demir.dm`'s builtin defaults
        // beating the map's own order, which lists every obj ahead of its turf. Cell 0 is "floor"
        // and cell 1 is "wall", so no turf may appear after the first object.
        let first_object_x = textures
            .lookup("icons/test.dmi", 2)
            .expect("object cell")
            .source_position[0];
        let first_object = sprites
            .iter()
            .position(|sprite| !sprite.is_area && sprite.texture.source_position[0] >= first_object_x)
            .expect("an object sprite");

        assert!(
            sprites[first_object..]
                .iter()
                .filter(|sprite| !sprite.is_area)
                .all(|sprite| sprite.texture.source_position[0] >= first_object_x)
        );
    }
}
