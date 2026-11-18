use std::path::PathBuf;

use dmi::IconFile;
use render::texture::TextureCatalog;

use super::fixtures::build;
use crate::{document::MapDocument, environment::Environment};

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
    assert_eq!(environment.maps, vec![root.join("test.dmm"), root.join("test2.dmm")]);

    let mut textures = TextureCatalog::default();
    let file = IconFile::load(root.join("icons/test.dmi")).expect("icon");
    textures.insert("icons/test.dmi", &file).expect("insert");

    let source = std::fs::read_to_string(root.join("test.dmm")).expect("map");
    let (map, errors) = dmm::parser::parse(&source);
    assert!(errors.is_empty(), "{errors:?}");

    let document = MapDocument::new(map, 1);
    let instances = build(&environment.tree, &textures, &document, 32);
    let sprites = instances.live_sprites().copied().collect::<Vec<_>>();

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
    let instances = build(&environment.tree, &textures, &document, 32);
    let sprites = instances.live_sprites().copied().collect::<Vec<_>>();

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
