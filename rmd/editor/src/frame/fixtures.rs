use std::collections::HashMap;

use objtree::ObjectTree;
use render::texture::TextureCatalog;

use prefab::PrefabInstanceId;

use super::{
    FrameInstances, FrameRenderOptions, PrefabUpdate, TypeVisibility, build_view_with_options,
    update_view_prefabs_with_options,
};
use crate::{document::MapDocument, environment::world::Perspective};

pub(crate) fn build(
    tree: &ObjectTree, textures: &TextureCatalog, document: &MapDocument, tile_size: u32,
) -> FrameInstances {
    build_with_options(
        tree,
        textures,
        document,
        FrameRenderOptions {
            perspective: Perspective::TopDown,
            node_types: None,
            visibility: &TypeVisibility::default(),
            turf_view: None,
            tile_size,
            appearances: &HashMap::new(),
            lighting: None,
        },
    )
}

pub(crate) fn build_with_options(
    tree: &ObjectTree, textures: &TextureCatalog, document: &MapDocument, options: FrameRenderOptions<'_>,
) -> FrameInstances {
    build_view_with_options(tree, textures, (&document.map).into(), options)
}

pub(crate) fn update_prefab(
    instances: &mut FrameInstances, tree: &ObjectTree, textures: &TextureCatalog, document: &MapDocument,
    owner: PrefabInstanceId, tile_size: u32,
) -> PrefabUpdate {
    update_prefabs(instances, tree, textures, document, &[owner], tile_size)
}

pub(crate) fn update_prefabs(
    instances: &mut FrameInstances, tree: &ObjectTree, textures: &TextureCatalog, document: &MapDocument,
    owners: &[PrefabInstanceId], tile_size: u32,
) -> PrefabUpdate {
    update_prefabs_with_options(
        instances,
        tree,
        textures,
        document,
        owners,
        FrameRenderOptions {
            perspective: Perspective::TopDown,
            node_types: None,
            visibility: &TypeVisibility::default(),
            turf_view: None,
            tile_size,
            appearances: &HashMap::new(),
            lighting: None,
        },
    )
}

pub(crate) fn update_prefabs_with_options(
    instances: &mut FrameInstances, tree: &ObjectTree, textures: &TextureCatalog, document: &MapDocument,
    owners: &[PrefabInstanceId], options: FrameRenderOptions<'_>,
) -> PrefabUpdate {
    update_view_prefabs_with_options(instances, tree, textures, (&document.map).into(), owners, options)
}
