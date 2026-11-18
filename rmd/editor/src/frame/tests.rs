use base::{
    coord::Coord,
    location::Location,
    path::TreePath,
    types::{Identifier, Value, VarModifiers},
};
use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};

use dmi::{
    IconFile,
    metadata::{Dir, IconState, Metadata},
};
use dmm::{Map, Size};
use objtree::{ObjectTree, VarDecl};
use prefab::{Prefab, PrefabInstanceId};
use render::{
    AREA_EDGE_EAST, AREA_EDGE_NORTH, AREA_EDGE_SOUTH, AREA_EDGE_WEST, AREA_EDGES_ALL, IDENTITY_TRANSFORM,
    SpriteInstance, SpriteTexture, UpdateRange, texture::TextureCatalog,
};
use vm::{AppearanceDelta, bake::SharedAppearance, matrix::Matrix};

use super::{
    FrameInstances, PrefabUpdate,
    context::side_row,
    fixtures::{build, build_with_options, update_prefab, update_prefabs, update_prefabs_with_options},
    instance_for,
    options::{FrameRenderOptions, HiddenTypes, TypeVisibility},
    pages::assert_pages_hold,
};
use crate::{
    document::{MapDocument, command::Edit, instance::VarMutation},
    environment::world::Perspective,
    frame::visual::Appearance,
};

const ICON: &str = "test.dmi";

pub(super) fn document(map: Map) -> MapDocument {
    MapDocument::new(map, 1)
}

fn owner() -> PrefabInstanceId {
    PrefabInstanceId::from_raw(1).expect("nonzero prefab instance ID")
}

fn appearance(color: Option<&str>, alpha: u8) -> Appearance {
    Appearance {
        color: color.map(String::from),
        alpha,
        ..Default::default()
    }
}

#[test]
fn hidden_types_union_merges_paths_and_areas() {
    let area = |name: &str| Arc::new(HashSet::from([TreePath::parse(name).segments]));
    let left = [TreePath::parse("/obj")]
        .into_iter()
        .collect::<HiddenTypes>()
        .with_areas(area("/area/a"));
    let right = [TreePath::parse("/mob")].into_iter().collect::<HiddenTypes>();

    let merged = left.union(&right);
    assert!(merged.hides(&Prefab::new(TreePath::parse("/obj"))));
    assert!(merged.hides(&Prefab::new(TreePath::parse("/mob"))));
    assert!(merged.hides(&Prefab::new(TreePath::parse("/area/a"))));

    let both = left.union(&HiddenTypes::default().with_areas(area("/area/b")));
    assert!(both.hides(&Prefab::new(TreePath::parse("/area/a"))));
    assert!(both.hides(&Prefab::new(TreePath::parse("/area/b"))));
    assert!(HiddenTypes::default().union(&HiddenTypes::default()).is_empty());
}

#[test]
fn a_tint_is_premultiplied_by_alpha() {
    let instance = instance_for(
        owner(),
        &appearance(Some("#ff0000"), 128),
        SpriteTexture::default(),
        Coord::new(1, 1, 1),
        32,
        false,
    );
    let alpha = 128.0 / 255.0;

    assert_eq!(instance.color, [alpha, 0.0, 0.0, alpha]);
}

#[test]
fn an_untinted_sprite_keeps_its_alpha_on_every_channel() {
    let instance = instance_for(
        owner(),
        &appearance(None, 255),
        SpriteTexture::default(),
        Coord::new(1, 1, 1),
        32,
        false,
    );

    assert_eq!(instance.color, [1.0, 1.0, 1.0, 1.0]);
}

#[test]
fn an_unreadable_color_falls_back_to_no_tint() {
    let instance = instance_for(
        owner(),
        &appearance(Some("chartreuse"), 255),
        SpriteTexture::default(),
        Coord::new(1, 1, 1),
        32,
        false,
    );

    assert_eq!(instance.color, [1.0, 1.0, 1.0, 1.0]);
}

/// Step and both pixel offset pairs stack rather than replacing each other.
#[test]
fn all_offset_pairs_shift_the_sprite() {
    let appearance = Appearance {
        pixel_x: -16,
        pixel_y: 4,
        pixel_w: 2,
        pixel_z: -1,
        step_x: 3,
        step_y: -2,
        ..Default::default()
    };
    let instance = instance_for(
        owner(),
        &appearance,
        SpriteTexture::default(),
        Coord::new(2, 3, 1),
        32,
        false,
    );

    assert_eq!((instance.x, instance.y), (32.0 - 11.0, 64.0 + 1.0));
}

/// A 64x64 icon hangs off the top and the right of its tile, so `pixel_x = -16` centres it.
#[test]
fn a_larger_than_tile_icon_anchors_to_the_bottom_left_of_its_tile() {
    let texture = SpriteTexture {
        index: 0,
        source_position: [0, 0],
        width: 64,
        height: 64,
    };
    let centred = Appearance {
        pixel_x: -16,
        ..Default::default()
    };

    let plain = instance_for(owner(), &Appearance::default(), texture, Coord::new(1, 1, 1), 32, false);
    assert_eq!((plain.x, plain.y), (0.0, 0.0));

    let shifted = instance_for(owner(), &centred, texture, Coord::new(1, 1, 1), 32, false);
    assert_eq!((shifted.x, shifted.y), (-16.0, 0.0));
}

/// `smoothrocks.dmi` is 40x40, so tgstation pulls it back 4 pixels to sit centered on its tile.
#[test]
fn a_transform_translates_the_sprite_and_turns_it_about_its_center() {
    let texture = SpriteTexture {
        index: 0,
        source_position: [0, 0],
        width: 40,
        height: 40,
    };
    let appearance = Appearance {
        transform: Matrix::turn(90.0).then(Matrix::translate(-4.0, -4.0)),
        ..Default::default()
    };

    let instance = instance_for(owner(), &appearance, texture, Coord::new(1, 1, 1), 32, false);
    let [a, b, d, e] = instance.transform;
    assert_eq!((instance.x, instance.y), (-4.0, -4.0));
    assert!(a.abs() < 1e-6 && (b - 1.0).abs() < 1e-6 && (d + 1.0).abs() < 1e-6 && e.abs() < 1e-6);

    let unpackable = Appearance {
        transform: Matrix([f32::NAN, 0.0, 3.0, 0.0, 1.0, 0.0]),
        ..Default::default()
    };
    let instance = instance_for(owner(), &unpackable, texture, Coord::new(1, 1, 1), 32, false);
    assert_eq!((instance.x, instance.transform), (0.0, IDENTITY_TRANSFORM));
}

#[test]
fn an_instance_keeps_its_level_and_area_classification() {
    let instance = instance_for(
        owner(),
        &Appearance::default(),
        SpriteTexture::default(),
        Coord::new(1, 1, 7),
        32,
        true,
    );

    assert_eq!(instance.z, 7);
    assert!(instance.is_area);
}

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
pub(super) fn tree(types: &[(&str, &str, f32)]) -> ObjectTree {
    let mut tree = ObjectTree::new();

    for (path, state, layer) in types {
        let id = tree.register(&TreePath::parse(path), Location::default());
        let Some(decl) = tree.get_mut(id) else {
            continue;
        };

        let mut var = |name: &str, value: Value| {
            let name = Identifier::from(String::from(name));
            decl.vars.insert(
                name.clone(),
                VarDecl {
                    name,
                    declared_type: None,
                    modifiers: VarModifiers::default(),
                    value,
                    declared: true,
                    location: Location::default(),
                    resolved_type: None,
                    shared: None,
                },
            );
        };

        var("icon", Value::Resource(String::from(ICON)));
        var("icon_state", Value::Text(String::from(*state)));
        var("layer", Value::Num(*layer));
    }

    tree
}

#[test]
fn type_visibility_toggles_complete_subtrees_without_changing_siblings() {
    let tree = tree(&[
        ("/obj/parent", "floor", 1.0),
        ("/obj/parent/child", "floor", 1.0),
        ("/obj/sibling", "floor", 1.0),
    ]);
    let parent = tree.id_of(&TreePath::parse("/obj/parent")).unwrap();
    let child = tree.id_of(&TreePath::parse("/obj/parent/child")).unwrap();
    let sibling = tree.id_of(&TreePath::parse("/obj/sibling")).unwrap();
    let mut visibility = TypeVisibility::default();

    assert!(visibility.set_subtree(&tree, parent, false));
    assert!(!visibility.is_visible(parent));
    assert!(!visibility.is_visible(child));
    assert!(visibility.is_visible(sibling));

    assert!(visibility.set_subtree(&tree, child, true));
    assert!(!visibility.is_visible(parent));
    assert!(visibility.is_visible(child));
    assert!(visibility.is_visible(sibling));
}

#[test]
fn hidden_types_match_prefabs_by_path() {
    let tree = tree(&[("/obj/parent", "floor", 1.0), ("/obj/parent/child", "floor", 1.0)]);
    let parent = tree.id_of(&TreePath::parse("/obj/parent")).unwrap();
    let mut visibility = TypeVisibility::default();
    assert!(visibility.hidden_types(&tree).is_empty());

    visibility.set_subtree(&tree, parent, false);
    let hidden = visibility.hidden_types(&tree);

    assert!(hidden.hides(&Prefab::new(TreePath::parse("/obj/parent/child"))));
    assert!(!hidden.hides(&Prefab::new(TreePath::parse("/obj/other"))));
}

#[test]
fn hidden_types_keep_their_sprites_flagged() {
    let tree = tree(&[("/obj/table", "table", 2.0), ("/area/station", "floor", 1.0)]);
    let document = document(one_tile_map(&["/obj/table", "/area/station"]));
    let coord = Coord::new(1, 1, 1);
    let object_owner = document.instance_ids_at(coord)[0];
    let area_owner = document.instance_ids_at(coord)[1];
    let icons = icons(&["floor", "table"]);
    let textures = textures(&["floor", "table"]);
    let object = tree.id_of(&TreePath::parse("/obj")).unwrap();
    let area = tree.id_of(&TreePath::parse("/area")).unwrap();
    let mut visibility = TypeVisibility::default();

    visibility.set_subtree(&tree, object, false);
    let without_objects = build_with_options(
        &tree,
        &icons,
        &textures,
        &document,
        FrameRenderOptions {
            perspective: Perspective::TopDown,
            node_types: None,
            visibility: &visibility,
            tile_size: 32,
            appearances: &HashMap::new(),
            lighting: None,
        },
    );
    assert!(
        without_objects
            .live_sprites()
            .all(|sprite| sprite.owner != object_owner)
    );
    assert!(without_objects.live_sprites().any(|sprite| sprite.owner == area_owner));
    assert!(without_objects.sprite(object_owner).is_none());
    assert!(
        without_objects
            .sprites
            .iter()
            .any(|sprite| sprite.owner == object_owner && sprite.hidden)
    );

    visibility.set_subtree(&tree, object, true);
    visibility.set_subtree(&tree, area, false);
    let without_areas = build_with_options(
        &tree,
        &icons,
        &textures,
        &document,
        FrameRenderOptions {
            perspective: Perspective::TopDown,
            node_types: None,
            visibility: &visibility,
            tile_size: 32,
            appearances: &HashMap::new(),
            lighting: None,
        },
    );
    assert!(without_areas.live_sprites().any(|sprite| sprite.owner == object_owner));
    assert!(without_areas.live_sprites().all(|sprite| sprite.owner != area_owner));
}

fn column_map(prefabs: Vec<(u32, Prefab)>) -> Map {
    let mut map = Map::new(Size { x: 1, y: 3, z: 1 });
    for (y, prefab) in prefabs {
        map.grid[0][(3 - y) as usize][0] = map.intern_tile(vec![prefab]);
    }

    map
}

fn prefab_with(path: &str, vars: &[(&str, f32)]) -> Prefab {
    let mut prefab = Prefab::new(TreePath::parse(path));
    for (name, value) in vars {
        prefab.set_var(Identifier::from(String::from(*name)), Value::Num(*value));
    }

    prefab
}

fn side_options<'a>(
    visibility: &'a TypeVisibility, appearances: &'a HashMap<u64, SharedAppearance>,
) -> FrameRenderOptions<'a> {
    FrameRenderOptions {
        perspective: Perspective::Side,
        node_types: None,
        visibility,
        tile_size: 32,
        appearances,
        lighting: None,
    }
}

fn build_side(
    tree: &ObjectTree, icons: &HashMap<String, Metadata>, textures: &TextureCatalog, document: &MapDocument,
) -> FrameInstances {
    build_with_options(
        tree,
        icons,
        textures,
        document,
        side_options(&TypeVisibility::default(), &HashMap::new()),
    )
}

fn drawn_owners(
    tree: &ObjectTree, document: &MapDocument, perspective: Perspective, among: &[PrefabInstanceId],
) -> Vec<PrefabInstanceId> {
    let (icons, textures) = (icons(&["floor", "table"]), textures(&["floor", "table"]));
    let instances = match perspective {
        Perspective::Side => build_side(tree, &icons, &textures, document),
        Perspective::TopDown => build(tree, &icons, &textures, document, 32),
    };
    instances
        .live_sprites()
        .map(|sprite| sprite.owner)
        .filter(|owner| among.contains(owner))
        .collect()
}

fn owner_at(document: &MapDocument, y: u32) -> PrefabInstanceId {
    document.instance_ids_at(Coord::new(1, y, 1))[0]
}

#[test]
fn side_maps_draw_back_rows_first_regardless_of_layer() {
    let tree = tree(&[("/obj/low", "floor", 1.0), ("/obj/high", "table", 9.0)]);
    let map = column_map(vec![
        (3, prefab_with("/obj/high", &[])),
        (1, prefab_with("/obj/low", &[])),
    ]);
    let document = document(map);
    let (back, near) = (owner_at(&document, 3), owner_at(&document, 1));

    assert_eq!(
        drawn_owners(&tree, &document, Perspective::Side, &[back, near]),
        [back, near]
    );

    assert_eq!(
        drawn_owners(&tree, &document, Perspective::TopDown, &[near, back]),
        [near, back]
    );
}

#[test]
fn side_rows_follow_the_tile_under_the_sprites_center() {
    let row = |pixel_y| {
        let appearance = Appearance {
            pixel_y,
            ..Appearance::default()
        };

        side_row(Coord::new(1, 42, 1), &appearance, 32)
    };

    assert_eq!(
        [-5, 15, -15, 16, -16, 32, -32, 48].map(row),
        [42, 42, 42, 43, 41, 43, 41, 44].map(Reverse)
    );
}

#[test]
fn side_maps_sort_by_pixel_y_but_not_pixel_z() {
    let tree = tree(&[("/obj/low", "floor", 1.0), ("/obj/high", "table", 9.0)]);
    let map = column_map(vec![
        (1, prefab_with("/obj/high", &[("pixel_y", 32.0)])),
        (2, prefab_with("/obj/low", &[])),
    ]);
    let document = MapDocument::new(map, 1);
    let (shifted, row_two) = (owner_at(&document, 1), owner_at(&document, 2));
    assert_eq!(
        drawn_owners(&tree, &document, Perspective::Side, &[shifted, row_two]),
        [row_two, shifted],
        "row 2 ties, layer decides"
    );

    let map = column_map(vec![
        (1, prefab_with("/obj/high", &[("pixel_y", 64.0)])),
        (2, prefab_with("/obj/low", &[])),
    ]);
    let document = MapDocument::new(map, 1);
    let (shifted, row_two) = (owner_at(&document, 1), owner_at(&document, 2));
    assert_eq!(
        drawn_owners(&tree, &document, Perspective::Side, &[shifted, row_two]),
        [shifted, row_two]
    );

    let map = column_map(vec![
        (1, prefab_with("/obj/high", &[("pixel_z", -32.0)])),
        (2, prefab_with("/obj/low", &[])),
    ]);
    let document = MapDocument::new(map, 1);
    let (lifted, row_two) = (owner_at(&document, 1), owner_at(&document, 2));
    assert_eq!(
        drawn_owners(&tree, &document, Perspective::Side, &[row_two, lifted]),
        [row_two, lifted]
    );
}

#[test]
fn side_maps_draw_effects_layers_above_nearer_rows() {
    let tree = tree(&[("/obj/low", "floor", 1.0), ("/obj/high", "table", 9.0)]);
    let map = column_map(vec![
        (3, prefab_with("/obj/low", &[("layer", 5000.0)])),
        (1, prefab_with("/obj/high", &[])),
    ]);
    let document = document(map);
    let (effect, near) = (owner_at(&document, 3), owner_at(&document, 1));

    assert_eq!(
        drawn_owners(&tree, &document, Perspective::Side, &[near, effect]),
        [near, effect]
    );
}

#[test]
fn side_map_updates_keep_the_keys_of_a_rebuild_when_an_owner_changes_rows() {
    let tree = tree(&[("/obj/low", "floor", 1.0), ("/obj/high", "table", 9.0)]);
    let map = column_map(vec![
        (2, prefab_with("/obj/high", &[])),
        (1, prefab_with("/obj/low", &[])),
    ]);
    let mut document = document(map);
    let (icons, textures) = (icons(&["floor", "table"]), textures(&["floor", "table"]));
    let mut instances = build_side(&tree, &icons, &textures, &document);
    let (back, near) = (owner_at(&document, 2), owner_at(&document, 1));

    for pixel_y in [-64.0, 0.0, 32.0] {
        document
            .edit_instance_vars(
                back,
                "move row",
                &[VarMutation::Set("pixel_y".into(), Value::Num(pixel_y))],
                None,
            )
            .unwrap();
        update_prefabs_with_options(
            &mut instances,
            &tree,
            &icons,
            &textures,
            &document,
            &[back],
            side_options(&TypeVisibility::default(), &HashMap::new()),
        );
        assert_render_data_matches(&instances, &build_side(&tree, &icons, &textures, &document));

        let order = instances
            .live_sprites()
            .map(|sprite| sprite.owner)
            .filter(|owner| [back, near].contains(owner))
            .collect::<Vec<_>>();
        assert_eq!(order, if pixel_y < -32.0 { [near, back] } else { [back, near] });
    }
}

#[test]
fn a_shown_area_draws_above_emissive_blockers() {
    let tree = tree(&[("/obj/door", "table", 3.0), ("/area/station", "floor", 1.0)]);
    let mut map = Map::new(Size { x: 1, y: 1, z: 1 });
    let mut door = Prefab::new(TreePath::parse("/obj/door"));
    door.set_var(Identifier::from("plane"), Value::Num(13.0));
    map.grid[0][0][0] = map.intern_tile(vec![door, Prefab::new(TreePath::parse("/area/station"))]);
    let document = document(map);
    let owners = document.instance_ids_at(Coord::new(1, 1, 1));
    let icons = icons(&["floor", "table"]);
    let textures = textures(&["floor", "table"]);

    let instances = build(&tree, &icons, &textures, &document, 32);
    let drawn = instances
        .live_sprites()
        .filter(|sprite| sprite.area_edges == 0)
        .map(|sprite| sprite.owner)
        .collect::<Vec<_>>();

    assert_eq!(drawn, [owners[0], owners[1]]);
}

#[test]
fn only_turfs_with_mouse_opacity_zero_are_click_through() {
    let tree = tree(&[("/turf/openspace", "floor", 2.0), ("/obj/dirt", "table", 3.0)]);
    let mut map = Map::new(Size { x: 1, y: 1, z: 1 });
    let tile = ["/turf/openspace", "/obj/dirt"]
        .into_iter()
        .map(|path| {
            let mut prefab = Prefab::new(TreePath::parse(path));
            prefab.set_var(Identifier::from("mouse_opacity"), Value::Num(0.0));
            prefab
        })
        .collect();
    map.grid[0][0][0] = map.intern_tile(tile);
    let document = document(map);
    let owners = document.instance_ids_at(Coord::new(1, 1, 1));
    let icons = icons(&["floor", "table"]);
    let textures = textures(&["floor", "table"]);

    let instances = build(&tree, &icons, &textures, &document, 32);
    let click_through = |owner| {
        instances
            .live_sprites()
            .filter(|sprite| sprite.owner == owner)
            .map(|sprite| sprite.click_through)
            .collect::<Vec<_>>()
    };

    assert_eq!(click_through(owners[0]), [true]);
    assert_eq!(click_through(owners[1]), [false]);
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

pub(super) fn icons(states: &[&str]) -> HashMap<String, Metadata> {
    HashMap::from([(String::from(ICON), metadata(states))])
}

pub(super) fn textures(states: &[&str]) -> TextureCatalog {
    let mut textures = TextureCatalog::new();
    textures.insert(ICON, &icon_file(states)).expect("insert");

    textures
}

fn shared(appearances: HashMap<u64, AppearanceDelta>) -> HashMap<u64, SharedAppearance> {
    appearances
        .into_iter()
        .map(|(id, appearance)| (id, Arc::new(appearance)))
        .collect()
}

/// The sprites a list draws, in draw order, without the pages' spare slots.
fn drawn(instances: &FrameInstances) -> Vec<SpriteInstance> {
    instances.live_sprites().copied().collect()
}

pub(super) fn assert_render_data_matches(actual: &FrameInstances, expected: &FrameInstances) {
    assert_eq!(drawn(actual), drawn(expected), "drawn in the order of a clean build");
    assert_pages_hold(actual);
    assert_eq!(actual.area_components, expected.area_components);
    assert_eq!(actual.area_component_tiles, expected.area_component_tiles);
    assert_eq!(actual.sprite_sort_keys.len(), actual.sprites.len());
    assert_eq!(actual.primary_sprites, expected.primary_sprites);
    assert_eq!(actual.unknown_placements, expected.unknown_placements);
    for owner in actual.placements.keys().chain(expected.placements.keys()) {
        assert_eq!(actual.sprite(*owner), expected.sprite(*owner));
    }

    for (owner, sprite) in &actual.primary_sprites {
        assert_eq!(sprite.owner, *owner);
        assert_eq!(actual.sprite(*owner), Some(sprite));
    }
}

#[test]
fn orders_sprites_by_layer_not_by_prefab_order() {
    let tree = tree(&[("/turf/floor", "floor", 2.0), ("/obj/table", "table", 3.0)]);
    // The map lists the high-layer object first; the frame must still draw it last.
    let document = document(one_tile_map(&["/obj/table", "/turf/floor"]));
    let owners = document.instance_ids_at(Coord::new(1, 1, 1));

    let instances = build(
        &tree,
        &icons(&["floor", "table"]),
        &textures(&["floor", "table"]),
        &document,
        32,
    );
    let sprites = drawn(&instances);

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
fn large_equal_layer_decals_draw_above_surrounding_turfs() {
    let tree = tree(&[("/turf/floor", "floor", 2.0), ("/obj/decal", "decal", 2.0)]);
    let mut icons = icons(&["floor", "decal"]);
    let mut textures = textures(&["floor", "decal"]);
    let mut decal_icon = icon_file(&["decal"]);
    decal_icon.metadata.width = 96;
    decal_icon.metadata.height = 96;
    decal_icon.sheet_width = 96;
    decal_icon.sheet_height = 96;
    decal_icon.pixels = vec![255; 96 * 96 * 4];
    icons.insert("large.dmi".into(), decal_icon.metadata.clone());
    textures.insert("large.dmi", &decal_icon).unwrap();

    let mut map = area_map(5, 5, "/turf/floor");
    let mut decal = Prefab::new(TreePath::parse("/obj/decal"));
    decal.set_var("icon".into(), Value::Resource("large.dmi".into()));
    let key = map.intern_tile(vec![decal, Prefab::new(TreePath::parse("/turf/floor"))]);
    map.grid[0][2][2] = key;
    let mut document = document(map);
    let owner = document.instance_ids_at(Coord::new(3, 3, 1))[0];
    let mut instances = build(&tree, &icons, &textures, &document, 32);

    for (pixel_x, pixel_y) in [(0.0, 0.0), (-7.0, -9.0)] {
        document
            .edit_instance_vars(
                owner,
                "nudge decal",
                &[
                    VarMutation::Set("pixel_x".into(), Value::Num(pixel_x)),
                    VarMutation::Set("pixel_y".into(), Value::Num(pixel_y)),
                ],
                None,
            )
            .unwrap();
        update_prefab(&mut instances, &tree, &icons, &textures, &document, owner, 32);
        assert_render_data_matches(&instances, &build(&tree, &icons, &textures, &document, 32));
        let sprites = drawn(&instances);
        assert_eq!(sprites.len(), 26);
        assert_eq!(sprites.last().unwrap().owner, owner, "all floors draw before the decal");
        assert!(
            sprites[..25]
                .iter()
                .all(|sprite| (sprite.width, sprite.height) == (32.0, 32.0))
        );
        let decal = instances.sprite(owner).unwrap();
        assert_eq!((decal.width, decal.height), (96.0, 96.0));
        assert_eq!((decal.x, decal.y), (64.0 + pixel_x, 64.0 + pixel_y));
        assert_pages_hold(&instances);
    }
}

#[test]
fn equal_layer_overlays_and_underlays_keep_their_owners_turf_priority() {
    let tree = tree(&[("/obj/decal", "decal", 2.0), ("/turf/floor", "floor", 2.0)]);
    let node_types = HashSet::from([tree.id_of(&TreePath::parse("/obj/decal")).unwrap()]);
    let document = document(one_tile_map(&["/obj/decal", "/turf/floor"]));
    let owners = document.instance_ids_at(Coord::new(1, 1, 1));
    let child = |state: &str| AppearanceDelta {
        vars: vec![("icon_state".into(), Value::Text(state.into()))],
        ..Default::default()
    };
    let appearances = shared(
        owners
            .iter()
            .map(|owner| {
                (
                    owner.get(),
                    AppearanceDelta {
                        underlays: vec![child("underlay")],
                        overlays: vec![child("overlay")],
                        ..Default::default()
                    },
                )
            })
            .collect(),
    );
    let instances = build_with_options(
        &tree,
        &icons(&["floor", "decal", "underlay", "overlay"]),
        &textures(&["floor", "decal", "underlay", "overlay"]),
        &document,
        FrameRenderOptions {
            perspective: Perspective::TopDown,
            node_types: Some(&node_types),
            visibility: &TypeVisibility::default(),
            tile_size: 32,
            appearances: &appearances,
            lighting: None,
        },
    );
    assert_eq!(
        drawn(&instances).iter().map(|sprite| sprite.owner).collect::<Vec<_>>(),
        [owners[1], owners[1], owners[1], owners[0], owners[0], owners[0]]
    );
    assert!(
        instances
            .live_sprites()
            .all(|sprite| sprite.node_pickable == (sprite.owner == owners[0]))
    );
    assert!(instances.sprite(owners[0]).unwrap().node_pickable);
    assert!(!instances.sprite(owners[1]).unwrap().node_pickable);
}

#[test]
fn explicit_planes_take_priority_over_equal_layer_turf_order() {
    let tree = tree(&[("/obj/decal", "decal", 2.0), ("/turf/floor", "floor", 2.0)]);
    let mut document = document(one_tile_map(&["/obj/decal", "/turf/floor"]));
    let owners = document.instance_ids_at(Coord::new(1, 1, 1)).to_vec();
    document
        .set_instance_var(owners[1], "plane".into(), Value::Num(-7.0))
        .unwrap();
    for (plane, expected) in [
        (-8.0, [owners[0], owners[1]]),
        (-7.0, [owners[1], owners[0]]),
        (-6.0, [owners[1], owners[0]]),
    ] {
        document
            .set_instance_var(owners[0], "plane".into(), Value::Num(plane))
            .unwrap();
        let instances = build(
            &tree,
            &icons(&["floor", "decal"]),
            &textures(&["floor", "decal"]),
            &document,
            32,
        );
        assert_eq!(
            drawn(&instances).iter().map(|sprite| sprite.owner).collect::<Vec<_>>(),
            expected
        );
    }
}

#[test]
fn fractional_decal_layers_survive_edits_and_undo() {
    let tree = tree(&[
        ("/obj/decal", "decal", 2.001),
        ("/obj/fine_decal", "fine", 2.0001),
        ("/obj/equal", "equal", 2.0),
        ("/turf/floor", "floor", 2.0),
    ]);
    let icons = icons(&["floor", "decal", "fine", "equal"]);
    let textures = textures(&["floor", "decal", "fine", "equal"]);
    let mut document = document(one_tile_map(&[
        "/obj/decal",
        "/obj/fine_decal",
        "/obj/equal",
        "/turf/floor",
    ]));
    let owners = document.instance_ids_at(Coord::new(1, 1, 1)).to_vec();
    for owner in &owners {
        document
            .set_instance_var(*owner, "plane".into(), Value::Num(-13.0))
            .unwrap();
    }
    let mut instances = build(&tree, &icons, &textures, &document, 32);
    let before = drawn(&instances);
    assert_eq!(
        before.iter().map(|sprite| sprite.owner).collect::<Vec<_>>(),
        [owners[3], owners[2], owners[1], owners[0]]
    );

    document
        .set_instance_var(owners[0], "layer".into(), Value::Num(1.999))
        .unwrap();
    update_prefab(&mut instances, &tree, &icons, &textures, &document, owners[0], 32);
    assert_eq!(drawn(&instances)[0].owner, owners[0]);
    let rebuilt = build(&tree, &icons, &textures, &document, 32);
    assert_eq!(drawn(&instances), drawn(&rebuilt));
    assert_render_data_matches(&instances, &rebuilt);

    document.undo_with_affected().expect("undo decal layer edit");
    update_prefab(&mut instances, &tree, &icons, &textures, &document, owners[0], 32);
    assert_eq!(drawn(&instances), before);
    let rebuilt = build(&tree, &icons, &textures, &document, 32);
    assert_eq!(drawn(&instances), drawn(&rebuilt));
    assert_render_data_matches(&instances, &rebuilt);
}

#[test]
fn fractional_overlay_layers_sort_across_placements() {
    let tree = tree(&[("/obj/decal", "decal", 2.0001), ("/turf/floor", "floor", 2.0)]);
    let mut document = document(one_tile_map(&["/obj/decal", "/turf/floor"]));
    let owners = document.instance_ids_at(Coord::new(1, 1, 1)).to_vec();
    for owner in &owners {
        document
            .set_instance_var(*owner, "plane".into(), Value::Num(-13.0))
            .unwrap();
    }
    let appearances = shared(HashMap::from([(
        owners[1].get(),
        vm::AppearanceDelta {
            overlays: vec![vm::AppearanceDelta {
                vars: vec![
                    ("icon_state".into(), Value::Text("overlay".into())),
                    ("layer".into(), Value::Num(2.001)),
                ],
                ..Default::default()
            }],
            ..Default::default()
        },
    )]));
    let instances = build_with_options(
        &tree,
        &icons(&["floor", "decal", "overlay"]),
        &textures(&["floor", "decal", "overlay"]),
        &document,
        FrameRenderOptions {
            perspective: Perspective::TopDown,
            node_types: None,
            visibility: &TypeVisibility::default(),
            tile_size: 32,
            appearances: &appearances,
            lighting: None,
        },
    );
    assert_eq!(
        drawn(&instances).iter().map(|sprite| sprite.owner).collect::<Vec<_>>(),
        [owners[1], owners[0], owners[1]]
    );
}

#[test]
fn an_overlay_plane_sorts_across_unrelated_placements() {
    let tree = tree(&[("/turf/wall", "floor", 2.058), ("/obj/legacy", "table", 3.0)]);
    let mut document = document(one_tile_map(&["/turf/wall", "/obj/legacy"]));
    let owners = document.instance_ids_at(Coord::new(1, 1, 1)).to_vec();
    for owner in &owners {
        document
            .set_instance_var(*owner, "plane".into(), Value::Num(-10.0))
            .unwrap();
    }
    let frill = vm::AppearanceDelta {
        vars: vec![
            ("icon_state".into(), Value::Text("light".into())),
            ("plane".into(), Value::Num(-7.0)),
            ("layer".into(), Value::Num(2.67)),
        ],
        ..Default::default()
    };
    let appearances = shared(HashMap::from([(
        owners[0].get(),
        vm::AppearanceDelta {
            overlays: vec![frill],
            ..Default::default()
        },
    )]));
    let visibility = TypeVisibility::default();

    let instances = build_with_options(
        &tree,
        &icons(&["floor", "table", "light"]),
        &textures(&["floor", "table", "light"]),
        &document,
        FrameRenderOptions {
            perspective: Perspective::TopDown,
            node_types: None,
            visibility: &visibility,
            tile_size: 32,
            appearances: &appearances,
            lighting: None,
        },
    );
    let sprites = drawn(&instances);

    assert_eq!(
        sprites.iter().map(|sprite| sprite.owner).collect::<Vec<_>>(),
        [owners[0], owners[1], owners[0]]
    );
    assert!(sprites[2].depth > sprites[1].depth);
    assert_eq!(
        instances.sprite(owners[0]).unwrap().texture,
        textures(&["floor"]).lookup(ICON, 0).unwrap()
    );
}

#[test]
fn keep_together_keeps_explicit_plane_overlays_with_their_owner() {
    let tree = tree(&[("/turf/wall", "floor", 2.058), ("/obj/legacy", "table", 3.0)]);
    let mut document = document(one_tile_map(&["/turf/wall", "/obj/legacy"]));
    let owners = document.instance_ids_at(Coord::new(1, 1, 1)).to_vec();
    for owner in &owners {
        document
            .set_instance_var(*owner, "plane".into(), Value::Num(-10.0))
            .unwrap();
    }
    let appearances = shared(HashMap::from([(
        owners[0].get(),
        vm::AppearanceDelta {
            vars: vec![("appearance_flags".into(), Value::Num(32.0))],
            overlays: vec![vm::AppearanceDelta {
                vars: vec![
                    ("icon_state".into(), Value::Text("light".into())),
                    ("plane".into(), Value::Num(-7.0)),
                ],
                ..Default::default()
            }],
            ..Default::default()
        },
    )]));
    let visibility = TypeVisibility::default();

    let instances = build_with_options(
        &tree,
        &icons(&["floor", "table", "light"]),
        &textures(&["floor", "table", "light"]),
        &document,
        FrameRenderOptions {
            perspective: Perspective::TopDown,
            node_types: None,
            visibility: &visibility,
            tile_size: 32,
            appearances: &appearances,
            lighting: None,
        },
    );
    let sprites = drawn(&instances);

    assert_eq!(
        sprites.iter().map(|sprite| sprite.owner).collect::<Vec<_>>(),
        [owners[0], owners[0], owners[1]]
    );
}

#[test]
fn keep_apart_restores_global_plane_order_inside_keep_together() {
    let tree = tree(&[("/turf/wall", "floor", 2.058), ("/obj/legacy", "table", 3.0)]);
    let mut document = document(one_tile_map(&["/turf/wall", "/obj/legacy"]));
    let owners = document.instance_ids_at(Coord::new(1, 1, 1)).to_vec();
    for owner in &owners {
        document
            .set_instance_var(*owner, "plane".into(), Value::Num(-10.0))
            .unwrap();
    }
    let appearances = shared(HashMap::from([(
        owners[0].get(),
        vm::AppearanceDelta {
            vars: vec![("appearance_flags".into(), Value::Num(32.0))],
            overlays: vec![vm::AppearanceDelta {
                vars: vec![
                    ("icon_state".into(), Value::Text("light".into())),
                    ("plane".into(), Value::Num(-7.0)),
                    ("appearance_flags".into(), Value::Num(64.0)),
                ],
                ..Default::default()
            }],
            ..Default::default()
        },
    )]));
    let visibility = TypeVisibility::default();

    let instances = build_with_options(
        &tree,
        &icons(&["floor", "table", "light"]),
        &textures(&["floor", "table", "light"]),
        &document,
        FrameRenderOptions {
            perspective: Perspective::TopDown,
            node_types: None,
            visibility: &visibility,
            tile_size: 32,
            appearances: &appearances,
            lighting: None,
        },
    );
    let sprites = drawn(&instances);

    assert_eq!(
        sprites.iter().map(|sprite| sprite.owner).collect::<Vec<_>>(),
        [owners[0], owners[1], owners[0]]
    );
}

#[test]
fn visible_prefabs_with_failed_icon_lookups_use_the_missing_texture() {
    let mut tree = tree(&[
        ("/obj/valid", "valid", 2.0),
        ("/obj/machinery/computer", "missing", 2.0),
        ("/obj/no_file", "valid", 2.0),
        ("/obj/iconless", "missing", 2.0),
        ("/obj/transparent", "missing", 2.0),
    ]);
    tree.register(&TreePath::parse("/obj/machinery/computer/monitor"), Location::default());
    let mut document = document(one_tile_map(&[
        "/obj/valid",
        "/obj/machinery/computer/monitor",
        "/obj/no_file",
        "/obj/iconless",
        "/obj/transparent",
    ]));
    let owners = document.instance_ids_at(Coord::new(1, 1, 1)).to_vec();
    document
        .set_instance_var(owners[1], "color".into(), Value::Text(String::from("#00ff00")))
        .unwrap();
    document
        .set_instance_var(owners[2], "icon".into(), Value::Resource(String::from("missing.dmi")))
        .unwrap();
    document
        .set_instance_var(owners[3], "icon".into(), Value::Null)
        .unwrap();
    document
        .set_instance_var(owners[4], "alpha".into(), Value::Num(0.0))
        .unwrap();

    let icons = icons(&["valid"]);
    let mut textures = textures(&["valid"]);
    let missing = textures.insert_missing_icon().unwrap();
    let instances = build(&tree, &icons, &textures, &document, 32);

    assert_eq!(
        instances.sprite(owners[0]).unwrap().texture,
        textures.lookup(ICON, 0).unwrap()
    );
    assert_eq!(instances.sprite(owners[1]).unwrap().texture, missing);
    assert_eq!(instances.sprite(owners[1]).unwrap().color, [1.0; 4]);
    assert_eq!(instances.sprite(owners[2]).unwrap().texture, missing);
    assert!(instances.sprite(owners[3]).is_none());
    assert!(instances.sprite(owners[4]).is_none());

    let mut visibility = TypeVisibility::default();
    let monitor = tree.id_of(&TreePath::parse("/obj/machinery/computer/monitor")).unwrap();
    visibility.set_subtree(&tree, monitor, false);
    let hidden = build_with_options(
        &tree,
        &icons,
        &textures,
        &document,
        FrameRenderOptions {
            perspective: Perspective::TopDown,
            node_types: None,
            visibility: &visibility,
            tile_size: 32,
            appearances: &HashMap::new(),
            lighting: None,
        },
    );
    assert!(hidden.sprite(owners[1]).is_none());
}

#[test]
fn failed_overlays_do_not_cover_a_valid_airlock_or_overlay_only_object() {
    let tree = tree(&[
        ("/obj/machinery/door/airlock/grunge", "closed", 2.0),
        ("/obj/overlay_only", "missing", 2.0),
        ("/obj/truly_missing", "missing", 2.0),
    ]);
    let document = document(one_tile_map(&[
        "/obj/machinery/door/airlock/grunge",
        "/obj/overlay_only",
        "/obj/truly_missing",
    ]));
    let owners = document.instance_ids_at(Coord::new(1, 1, 1)).to_vec();
    let overlay = |state: &str| vm::AppearanceDelta {
        vars: vec![("icon_state".into(), Value::Text(state.into()))],
        ..Default::default()
    };
    let appearances = shared(HashMap::from([
        (
            owners[0].get(),
            vm::AppearanceDelta {
                overlays: vec![overlay("missing")],
                ..Default::default()
            },
        ),
        (
            owners[1].get(),
            vm::AppearanceDelta {
                overlays: vec![overlay("closed")],
                ..Default::default()
            },
        ),
    ]));
    let icons = icons(&["closed"]);
    let mut textures = textures(&["closed"]);
    let missing = textures.insert_missing_icon().unwrap();
    let rendered = build_with_options(
        &tree,
        &icons,
        &textures,
        &document,
        FrameRenderOptions {
            perspective: Perspective::TopDown,
            node_types: None,
            visibility: &TypeVisibility::default(),
            tile_size: 32,
            appearances: &appearances,
            lighting: None,
        },
    );
    let real = textures.lookup(ICON, 0).unwrap();

    assert_eq!(rendered.sprite(owners[0]).unwrap().texture, real);
    assert_eq!(rendered.sprite(owners[1]).unwrap().texture, real);
    assert_eq!(rendered.sprite(owners[2]).unwrap().texture, missing);
    assert_eq!(
        rendered
            .sprites
            .iter()
            .filter(|sprite| sprite.owner == owners[0])
            .count(),
        1
    );
    assert_eq!(
        rendered
            .sprites
            .iter()
            .filter(|sprite| sprite.owner == owners[1])
            .count(),
        1
    );
}

#[test]
fn sprite_textures_use_the_resolved_direction_and_first_frame() {
    let metadata = Metadata {
        version: String::from("4.0"),
        width: 32,
        height: 32,
        states: vec![IconState {
            name: String::from("animated"),
            dirs: 4,
            frames: 2,
            ..Default::default()
        }],
    };
    let file = IconFile {
        path: PathBuf::from(ICON),
        metadata: metadata.clone(),
        sheet_width: 32 * 8,
        sheet_height: 32,
        pixels: vec![255; 32 * 8 * 32 * 4],
    };
    let icons = HashMap::from([(String::from(ICON), metadata)]);
    let mut textures = TextureCatalog::new();
    textures.insert(ICON, &file).expect("insert directional icon");
    let appearance = Appearance {
        icon: Some(String::from(ICON)),
        icon_state: Some(String::from("animated")),
        dir: Dir::East.to_bits(),
        ..Default::default()
    };

    let texture = super::sprite_texture(&icons, &textures, &appearance).expect("east thumbnail");

    assert_eq!(texture.source_position, [64, 0]);
    assert_eq!(texture, textures.lookup(ICON, 2).unwrap());
}

#[test]
fn prefab_updates_touch_only_the_changed_sprite_and_preserve_sorting() {
    let tree = tree(&[("/turf/floor", "floor", 2.0), ("/obj/table", "table", 3.0)]);
    let icons = icons(&["floor", "table"]);
    let textures = textures(&["floor", "table"]);
    let mut document = document(one_tile_map(&["/turf/floor", "/obj/table"]));
    let owner = document.instance_ids_at(Coord::new(1, 1, 1))[1];
    let mut instances = build(&tree, &icons, &textures, &document, 32);
    let before = drawn(&instances);
    // the map's one level fits in one page, which is what every change uploads
    let page = PrefabUpdate::Buffers {
        sprites: vec![UpdateRange {
            start: 0,
            end: instances.sprites.len(),
        }],
    };

    document
        .set_instance_var(owner, "pixel_x".into(), Value::Num(7.0))
        .unwrap();
    assert_eq!(
        update_prefab(&mut instances, &tree, &icons, &textures, &document, owner, 32),
        page
    );
    assert_eq!(drawn(&instances)[0], before[0]);
    assert_eq!(drawn(&instances)[1].x, before[1].x + 7.0);

    document
        .set_instance_var(owner, "name".into(), Value::Text("renamed".into()))
        .unwrap();
    assert_eq!(
        update_prefab(&mut instances, &tree, &icons, &textures, &document, owner, 32),
        PrefabUpdate::Unchanged,
    );

    document
        .set_instance_var(owner, "layer".into(), Value::Num(1.0))
        .unwrap();
    assert_eq!(
        update_prefab(&mut instances, &tree, &icons, &textures, &document, owner, 32),
        page
    );
    assert_eq!(drawn(&instances)[0].owner, owner);

    document
        .set_instance_var(owner, "icon".into(), Value::Resource("missing.dmi".into()))
        .unwrap();
    assert_eq!(
        update_prefab(&mut instances, &tree, &icons, &textures, &document, owner, 32),
        page
    );
    assert_eq!(drawn(&instances).len(), 1);
}

#[test]
fn area_identity_edits_repair_components_and_cached_buffers() {
    let tree = tree(&[("/area/station", "floor", 1.0)]);
    let icons = icons(&["floor"]);
    let textures = textures(&["floor"]);
    let mut document = document(area_map(5, 5, "/area/station"));
    let owner = document.instance_ids_at(Coord::new(3, 3, 1))[0];
    let mut instances = build(&tree, &icons, &textures, &document, 32);

    document
        .set_instance_var(owner, "name".into(), Value::Text(String::from("Engineering")))
        .unwrap();
    let update = update_prefab(&mut instances, &tree, &icons, &textures, &document, owner, 32);
    assert!(matches!(update, PrefabUpdate::Buffers { ref sprites, .. } if !sprites.is_empty()));
    assert_render_data_matches(&instances, &build(&tree, &icons, &textures, &document, 32));

    document
        .set_instance_var(owner, "name".into(), Value::Text(String::from("Bridge")))
        .unwrap();
    assert_eq!(
        update_prefab(&mut instances, &tree, &icons, &textures, &document, owner, 32),
        PrefabUpdate::Unchanged,
    );
    assert_render_data_matches(&instances, &build(&tree, &icons, &textures, &document, 32));

    document
        .edit_instance_vars(owner, "reset name", &[VarMutation::Remove("name".into())], None)
        .unwrap();
    let update = update_prefab(&mut instances, &tree, &icons, &textures, &document, owner, 32);
    assert!(matches!(update, PrefabUpdate::Buffers { ref sprites, .. } if !sprites.is_empty()));
    assert_render_data_matches(&instances, &build(&tree, &icons, &textures, &document, 32));
}

#[test]
fn area_icon_edits_patch_the_sprite_buffer() {
    let tree = tree(&[("/area/station", "floor", 1.0)]);
    let icons = icons(&["floor", "alert"]);
    let textures = textures(&["floor", "alert"]);
    let mut document = document(one_tile_map(&["/area/station"]));
    let owner = document.instance_ids_at(Coord::new(1, 1, 1))[0];
    let mut instances = build(&tree, &icons, &textures, &document, 32);

    document
        .set_instance_var(owner, "icon_state".into(), Value::Text(String::from("alert")))
        .unwrap();
    let update = update_prefab(&mut instances, &tree, &icons, &textures, &document, owner, 32);
    assert!(matches!(
        update,
        PrefabUpdate::Buffers { ref sprites } if !sprites.is_empty()
    ));
    assert_render_data_matches(&instances, &build(&tree, &icons, &textures, &document, 32));
}

#[test]
fn moving_an_area_repairs_components_and_cached_buffers() {
    let tree = tree(&[("/area/station", "floor", 1.0)]);
    let icons = icons(&["floor"]);
    let textures = textures(&["floor"]);
    let mut map = Map::new(Size { x: 3, y: 1, z: 1 });
    map.intern_tile(Vec::new());
    let area = map.intern_tile(vec![Prefab::new(TreePath::parse("/area/station"))]);
    map.grid[0][0][0] = area;
    let mut document = document(map);
    let owner = document.instance_ids_at(Coord::new(1, 1, 1))[0];
    let mut instances = build(&tree, &icons, &textures, &document, 32);

    document
        .move_instance(&tree, owner, Coord::new(2, 1, 1), "move area", &[], None)
        .unwrap();
    let update = update_prefab(&mut instances, &tree, &icons, &textures, &document, owner, 32);
    assert!(matches!(
        update,
        PrefabUpdate::Buffers { ref sprites } if !sprites.is_empty()
    ));
    assert_render_data_matches(&instances, &build(&tree, &icons, &textures, &document, 32));

    document
        .set_instance_var(owner, "name".into(), Value::Text(String::from("Moved")))
        .unwrap();
    update_prefab(&mut instances, &tree, &icons, &textures, &document, owner, 32);
    assert_render_data_matches(&instances, &build(&tree, &icons, &textures, &document, 32));
}

#[test]
fn changing_between_area_and_object_repairs_cached_buffers() {
    let tree = tree(&[("/area/station", "floor", 1.0), ("/obj/table", "table", 2.0)]);
    let icons = icons(&["floor", "table"]);
    let textures = textures(&["floor", "table"]);
    let mut document = document(one_tile_map(&["/area/station"]));
    let owner = document.instance_ids_at(Coord::new(1, 1, 1))[0];
    let mut instances = build(&tree, &icons, &textures, &document, 32);

    document
        .replace_instance_path(owner, "make object", TreePath::parse("/obj/table"), &[], None)
        .unwrap();
    let update = update_prefab(&mut instances, &tree, &icons, &textures, &document, owner, 32);
    assert!(matches!(
        update,
        PrefabUpdate::Buffers { ref sprites } if !sprites.is_empty()
    ));
    assert!(instances.live_sprites().all(|sprite| !sprite.is_area));
    assert_render_data_matches(&instances, &build(&tree, &icons, &textures, &document, 32));

    document
        .replace_instance_path(owner, "make area", TreePath::parse("/area/station"), &[], None)
        .unwrap();
    update_prefab(&mut instances, &tree, &icons, &textures, &document, owner, 32);
    assert_render_data_matches(&instances, &build(&tree, &icons, &textures, &document, 32));
}

#[test]
fn includes_normal_area_sprites_and_separate_outlines() {
    let tree = tree(&[("/turf/floor", "floor", 2.0), ("/area/station", "floor", 1.0)]);
    let mut document = document(one_tile_map(&["/turf/floor", "/area/station"]));
    let turf_owner = document.instance_ids_at(Coord::new(1, 1, 1))[0];
    let area_owner = document.instance_ids_at(Coord::new(1, 1, 1))[1];
    let (icons, textures) = (icons(&["floor"]), textures(&["floor"]));

    let mut instances = build(&tree, &icons, &textures, &document, 32);
    let sprites = drawn(&instances);

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
    assert_eq!((sprites[0].is_area, sprites[0].area_edges), (false, 0));
    assert_eq!((sprites[1].is_area, sprites[1].area_edges), (true, 0));
    assert_eq!((sprites[2].is_area, sprites[2].area_edges), (true, AREA_EDGES_ALL));
    assert_eq!(instances.sprite(area_owner).unwrap().area_edges, 0);

    document
        .set_instance_var(turf_owner, "layer".into(), Value::Num(0.0))
        .unwrap();
    update_prefab(&mut instances, &tree, &icons, &textures, &document, turf_owner, 32);
    assert_eq!(instances.sprite(area_owner).unwrap().area_edges, 0);
}

#[test]
fn matching_area_tiles_only_draw_the_outer_perimeter() {
    let tree = tree(&[("/area/station", "floor", 1.0)]);
    let (icons, textures) = (icons(&["floor"]), textures(&["floor"]));

    let instances = build(&tree, &icons, &textures, &document(area_map(3, 3, "/area/station")), 48);
    let sprites = drawn(&instances);

    assert_eq!(sprites.len(), 9 + 8);
    assert_eq!(
        sprites
            .iter()
            .filter(|sprite| sprite.is_area && sprite.area_edges == 0)
            .count(),
        9
    );
    assert_eq!(sprites.iter().filter(|sprite| sprite.area_edges != 0).count(), 8);
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

    let instances = build(&tree, &icons, &textures, &document(map), 32);
    let sprites = drawn(&instances);
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

    let instances = build(&tree, &icons, &textures, &document(area_map(2, 1, "/area/station")), 32);
    let sprites = drawn(&instances);
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

    let instances = build(&tree, &icons, &textures, &document(map), 32);
    let sprites = drawn(&instances);
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

    assert!(super::area::same_area(&left, &right));
    right.set_var("name".into(), Value::Text(String::from("Engineering")));
    assert!(!super::area::same_area(&left, &right));
}

#[test]
fn area_components_join_cardinal_tiles_but_split_disconnected_regions_and_levels() {
    let tree = tree(&[("/area/station", "floor", 1.0), ("/area/other", "floor", 1.0)]);
    let mut map = Map::new(Size { x: 4, y: 1, z: 2 });
    let station = map.intern_tile(vec![Prefab::new(TreePath::parse("/area/station"))]);
    let other = map.intern_tile(vec![Prefab::new(TreePath::parse("/area/other"))]);
    map.grid[0][0] = vec![station, station, other, station];
    map.grid[1][0] = vec![station, station, other, station];
    let document = document(map);

    let instances = build(&tree, &icons(&["floor"]), &textures(&["floor"]), &document, 32);
    let first = instances.area_component_at(Coord::new(1, 1, 1)).unwrap();

    assert_eq!(instances.area_component_at(Coord::new(2, 1, 1)), Some(first));
    assert_ne!(instances.area_component_at(Coord::new(3, 1, 1)), Some(first));
    assert_ne!(instances.area_component_at(Coord::new(4, 1, 1)), Some(first));
    assert_ne!(instances.area_component_at(Coord::new(1, 1, 2)), Some(first));
}

#[test]
fn deleting_an_area_splits_components_and_patches_cached_ranges() {
    let tree = tree(&[("/area/station", "floor", 1.0)]);
    let icons = icons(&["floor"]);
    let textures = textures(&["floor"]);
    let mut document = document(area_map(3, 1, "/area/station"));
    let middle = Coord::new(2, 1, 1);
    let owner = document.instance_ids_at(middle)[0];
    let mut instances = build(&tree, &icons, &textures, &document, 32);
    let mut after = document.placed_tile(middle).unwrap();
    after.clear();
    let mut edit = Edit::new("delete middle area");
    edit.change(&document, middle, after);
    document.apply(edit);

    let update = update_prefab(&mut instances, &tree, &icons, &textures, &document, owner, 32);

    assert!(matches!(
        update,
        PrefabUpdate::Buffers { ref sprites } if !sprites.is_empty()
    ));
    assert_ne!(
        instances.area_component_at(Coord::new(1, 1, 1)),
        instances.area_component_at(Coord::new(3, 1, 1))
    );
    assert_render_data_matches(&instances, &build(&tree, &icons, &textures, &document, 32));
}

#[test]
fn batched_area_deletions_produce_one_valid_buffer_update() {
    let tree = tree(&[("/area/station", "floor", 1.0)]);
    let icons = icons(&["floor"]);
    let textures = textures(&["floor"]);
    let mut document = document(area_map(4, 1, "/area/station"));
    let coords = [Coord::new(2, 1, 1), Coord::new(3, 1, 1)];
    let owners = coords.map(|coord| document.instance_ids_at(coord)[0]);
    let mut instances = build(&tree, &icons, &textures, &document, 32);
    let mut edit = Edit::new("delete two areas");
    for coord in coords {
        edit.change(&document, coord, Vec::new());
    }
    document.apply(edit);

    let update = update_prefabs(&mut instances, &tree, &icons, &textures, &document, &owners, 32);

    let PrefabUpdate::Buffers { sprites } = update else {
        panic!("batched area deletion must stay incremental");
    };
    assert!(!sprites.is_empty());
    assert!(
        sprites
            .iter()
            .all(|range| range.start <= range.end && range.end <= instances.sprites.len())
    );
    assert_render_data_matches(&instances, &build(&tree, &icons, &textures, &document, 32));
}

#[test]
fn sprites_inherit_the_connected_area_at_their_anchor_tile() {
    let tree = tree(&[("/obj/table", "table", 2.0), ("/area/station", "floor", 1.0)]);
    let map = one_tile_map(&["/obj/table", "/area/station"]);
    let document = document(map);
    let coord = Coord::new(1, 1, 1);
    let object_owner = document.instance_ids_at(coord)[0];
    let area_owner = document.instance_ids_at(coord)[1];

    let instances = build(
        &tree,
        &icons(&["floor", "table"]),
        &textures(&["floor", "table"]),
        &document,
        32,
    );
    let component = instances.area_component_at(coord);

    assert_eq!(instances.sprite(object_owner).unwrap().area_owner, component);
    assert!(
        instances
            .sprites
            .iter()
            .filter(|sprite| sprite.owner == area_owner)
            .all(|sprite| sprite.area_owner == component)
    );
}

#[test]
fn an_area_with_an_unresolvable_icon_still_gets_a_white_outline() {
    let tree = tree(&[("/area/station", "missing", 1.0)]);
    let document = document(one_tile_map(&["/area/station"]));

    let instances = build(&tree, &icons(&["floor"]), &textures(&["floor"]), &document, 32);
    let sprites = drawn(&instances);

    assert_eq!(sprites.len(), 1);
    assert!(sprites[0].is_area);
    assert_eq!(sprites[0].area_edges, AREA_EDGES_ALL);
    assert_eq!(sprites[0].texture, Default::default());
    assert_eq!(sprites[0].color, [1.0; 4]);
    assert_eq!(instances.sprite(sprites[0].owner), Some(&sprites[0]));
}

#[test]
fn an_unresolvable_icon_state_without_a_registered_fallback_emits_nothing() {
    let tree = tree(&[("/obj/ghost", "not_in_the_sheet", 2.0)]);
    let document = document(one_tile_map(&["/obj/ghost"]));

    let instances = build(&tree, &icons(&["floor"]), &textures(&["floor"]), &document, 32);
    let sprites = drawn(&instances);

    assert!(sprites.is_empty());
    assert_eq!(document.instance_ids_at(Coord::new(1, 1, 1)).len(), 1);
}

#[test]
fn a_prefab_with_no_type_in_the_tree_emits_nothing() {
    let document = document(one_tile_map(&["/obj/never/declared"]));

    let instances = build(
        &ObjectTree::new(),
        &icons(&["floor"]),
        &textures(&["floor"]),
        &document,
        32,
    );
    let sprites = drawn(&instances);

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
    let instances = build(&tree, &icons, &textures, &document(map), 32);
    let sprites = drawn(&instances);

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

    let instances = build(&tree, &icons(&["floor"]), &textures(&["floor"]), &document(map), 32);
    let sprites = drawn(&instances);

    assert_eq!(sprites.len(), 1);
    assert_eq!((sprites[0].x, sprites[0].y), (32.0, 0.0));
}
