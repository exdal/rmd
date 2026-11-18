use base::{coord::Coord, path::TreePath};

use dmm::{Map, Size};
use prefab::{Prefab, PrefabInstanceId};

use super::{PAGE_CAPACITY, assert_pages_hold};
use crate::{
    document::{MapDocument, command::Edit},
    environment::world::Perspective,
    frame::{
        PrefabUpdate,
        fixtures::{build, build_with_options, update_prefab, update_prefabs},
        options::{FrameRenderOptions, TypeVisibility},
        tests::{assert_render_data_matches, document, icons, textures, tree},
    },
};

fn floor_map(width: u32, height: u32, levels: u32) -> Map {
    let mut map = Map::new(Size {
        x: width,
        y: height,
        z: levels,
    });
    let key = map.intern_tile(vec![Prefab::new(TreePath::parse("/turf/floor"))]);
    for level in &mut map.grid {
        for row in level {
            row.fill(key);
        }
    }

    map
}

fn place(document: &mut MapDocument, coord: Coord, path: &str) -> PrefabInstanceId {
    let placed = document.instantiate(Prefab::new(TreePath::parse(path)));
    let id = placed.id();
    let mut after = document.placed_tile(coord).unwrap();
    after.push(placed);
    let mut edit = Edit::new("place");
    edit.change(document, coord, after);
    assert!(document.apply(edit));

    id
}

#[test]
fn a_placement_uploads_only_the_pages_it_touches() {
    let tree = tree(&[("/turf/floor", "floor", 2.0), ("/obj/table", "table", 2.0)]);
    let icons = icons(&["floor", "table"]);
    let textures = textures(&["floor", "table"]);
    let mut document = document(floor_map(24, 24, 2));
    let mut instances = build(&tree, &icons, &textures, &document, 32);
    let slots = instances.sprites.len();

    assert_eq!(instances.pages.len(), 6);
    assert_pages_hold(&instances);

    let table = place(&mut document, Coord::new(5, 5, 1), "/obj/table");
    let update = update_prefab(&mut instances, &tree, &icons, &textures, &document, table, 32);
    let PrefabUpdate::Buffers { sprites, .. } = update else {
        panic!("a placement must stay incremental");
    };

    assert_eq!(sprites.len(), 1);
    assert_eq!(sprites[0].end - sprites[0].start, PAGE_CAPACITY);
    assert_eq!(instances.sprites.len(), slots);
    assert_render_data_matches(&instances, &build(&tree, &icons, &textures, &document, 32));
}

#[test]
fn equal_layer_turf_priority_follows_moves_type_changes_and_history() {
    let tree = tree(&[("/turf/floor", "floor", 2.0), ("/obj/table", "table", 2.0)]);
    let icons = icons(&["floor", "table"]);
    let textures = textures(&["floor", "table"]);
    let mut document = document(floor_map(24, 24, 1));
    let table = place(&mut document, Coord::new(5, 24, 1), "/obj/table");
    let mut instances = build(&tree, &icons, &textures, &document, 32);
    assert_eq!(instances.live_sprites().last().unwrap().owner, table);

    assert_eq!(
        document.move_instance(&tree, table, Coord::new(20, 1, 1), "move table", &[], None),
        Some(true)
    );
    update_prefab(&mut instances, &tree, &icons, &textures, &document, table, 32);
    assert_eq!(instances.live_sprites().last().unwrap().owner, table);
    assert_render_data_matches(&instances, &build(&tree, &icons, &textures, &document, 32));

    assert_eq!(
        document.edit_instances(
            &[table],
            "replace table with turf",
            Some(&TreePath::parse("/turf/floor")),
            &[],
            None,
        ),
        Some(true)
    );
    update_prefab(&mut instances, &tree, &icons, &textures, &document, table, 32);
    assert_ne!(instances.live_sprites().last().unwrap().owner, table);
    assert_render_data_matches(&instances, &build(&tree, &icons, &textures, &document, 32));

    for _ in 0..2 {
        let affected = document.undo_with_affected().expect("undo");
        update_prefabs(&mut instances, &tree, &icons, &textures, &document, &affected, 32);
        assert_eq!(instances.live_sprites().last().unwrap().owner, table);
        assert_render_data_matches(&instances, &build(&tree, &icons, &textures, &document, 32));
    }
    for _ in 0..2 {
        let affected = document.redo_with_affected().expect("redo");
        update_prefabs(&mut instances, &tree, &icons, &textures, &document, &affected, 32);
        assert_render_data_matches(&instances, &build(&tree, &icons, &textures, &document, 32));
    }
    assert_ne!(instances.live_sprites().last().unwrap().owner, table);
}

#[test]
fn toggling_visibility_flags_the_cached_sprites_and_restores_them_exactly() {
    let tree = tree(&[("/turf/floor", "floor", 2.0), ("/obj/table", "table", 3.0)]);
    let icons = icons(&["floor", "table"]);
    let textures = textures(&["floor", "table"]);
    let mut document = document(floor_map(24, 24, 2));
    let tables = [(3, 3), (20, 7)].map(|(x, y)| place(&mut document, Coord::new(x, y, 1), "/obj/table"));
    let mut instances = build(&tree, &icons, &textures, &document, 32);
    let original = instances.sprites.clone();
    let table = tree.id_of(&TreePath::parse("/obj/table")).unwrap();
    let mut visibility = TypeVisibility::default();

    visibility.set_subtree(&tree, table, false);
    let PrefabUpdate::Buffers { sprites } = instances.apply_visibility(&visibility) else {
        panic!("hiding a placed type must flag its sprites");
    };
    for range in &sprites {
        assert!(
            instances.sprites[range.start..range.end]
                .iter()
                .any(|sprite| tables.contains(&sprite.owner)),
            "only pages holding a table are uploaded"
        );
    }
    let rebuilt = build_with_options(
        &tree,
        &icons,
        &textures,
        &document,
        FrameRenderOptions {
            perspective: Perspective::TopDown,
            node_types: None,
            visibility: &visibility,
            tile_size: 32,
            appearances: &std::collections::HashMap::new(),
            lighting: None,
        },
    );
    assert_eq!(instances.sprites, rebuilt.sprites);
    assert!(tables.iter().all(|table| instances.sprite(*table).is_none()));
    assert_pages_hold(&instances);
    assert_eq!(instances.apply_visibility(&visibility), PrefabUpdate::Unchanged);

    visibility.set_subtree(&tree, table, true);
    assert!(matches!(
        instances.apply_visibility(&visibility),
        PrefabUpdate::Buffers { .. }
    ));
    assert_eq!(instances.sprites, original);
    assert!(tables.iter().all(|table| instances.sprite(*table).is_some()));
}

#[test]
fn pages_follow_placements_overflow_removals_and_undo() {
    let tree = tree(&[("/turf/floor", "floor", 2.0), ("/obj/table", "table", 2.0)]);
    let icons = icons(&["floor", "table"]);
    let textures = textures(&["floor", "table"]);
    let mut document = document(floor_map(24, 24, 2));
    let mut instances = build(&tree, &icons, &textures, &document, 32);

    // every table sorts into the same page, so this runs it out of spare slots
    for x in 1..=24 {
        for y in 1..=5 {
            let table = place(&mut document, Coord::new(x, y, 1), "/obj/table");
            update_prefab(&mut instances, &tree, &icons, &textures, &document, table, 32);
        }
    }

    assert_render_data_matches(&instances, &build(&tree, &icons, &textures, &document, 32));

    for x in 1..=24 {
        let coord = Coord::new(x, 3, 2);
        let owners = document.instance_ids_at(coord).to_vec();
        let mut edit = Edit::new("clear");
        edit.change(&document, coord, Vec::new());
        assert!(document.apply(edit));
        update_prefabs(&mut instances, &tree, &icons, &textures, &document, &owners, 32);
    }

    assert_render_data_matches(&instances, &build(&tree, &icons, &textures, &document, 32));

    for _ in 0..30 {
        let affected = document.undo_with_affected().expect("undo");
        update_prefabs(&mut instances, &tree, &icons, &textures, &document, &affected, 32);
    }

    assert_render_data_matches(&instances, &build(&tree, &icons, &textures, &document, 32));
}
