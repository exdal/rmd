use dmm::PrefabInstanceId;

use crate::{Camera, MapViewFrame, MapViewRect, SpriteInstance, SpriteLighting, SpriteTexture};

pub(super) fn owner() -> PrefabInstanceId {
    PrefabInstanceId::from_raw(1).expect("nonzero prefab instance ID")
}

pub(super) fn sprite(z: u32) -> SpriteInstance {
    SpriteInstance {
        owner: owner(),
        area_owner: None,
        texture: SpriteTexture::default(),
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 0.0,
        z,
        is_area: false,
        hidden: false,
        click_through: false,
        node_pickable: false,
        area_edges: 0,
        lighting: SpriteLighting::Normal,
        color: [1.0; 4],
        depth: 0.0,
        transform: crate::IDENTITY_TRANSFORM,
    }
}

pub(super) fn map_view(rect: MapViewRect) -> MapViewFrame<'static> {
    MapViewFrame {
        rect,
        camera: Camera::default(),
        sprite_instances: &[],
        focused_area: None,
        active_z: 1,
        level_count: 1,
        revision: 1,
        pending_updates: &[],
        lighting: None,
        guide_lines: &[],
        connected: &[],
        interaction: Default::default(),
        preview: None,
    }
}

pub(super) fn map_view_with_sprites<'a>(rect: MapViewRect, sprites: &'a [SpriteInstance]) -> MapViewFrame<'a> {
    MapViewFrame {
        sprite_instances: sprites,
        ..map_view(rect)
    }
}
