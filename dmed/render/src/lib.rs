pub mod color;
pub mod device;
pub mod error;
pub mod frame;
pub mod imgui;
pub mod renderer;
pub mod texture;

use dmm::Coord;
use editor::{document::PrefabInstanceId, visual::Appearance};

pub use crate::{device::Device, error::GpuError, renderer::Renderer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VisibilityId(std::num::NonZeroU32);

impl VisibilityId {
    pub fn from_raw(raw: u32) -> Option<Self> { std::num::NonZeroU32::new(raw).map(Self) }

    pub fn from_sprite_index(index: usize) -> Option<Self> {
        let index = u32::try_from(index).ok()?;

        index.checked_add(1).and_then(Self::from_raw)
    }

    pub const fn get(self) -> u32 { self.0.get() }

    pub const fn sprite_index(self) -> usize { self.get() as usize - 1 }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SpriteTexture {
    pub index: u32,
    /// Pixel-space top-left corner within the DMI sheet.
    pub source_position: [u32; 2],
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpriteInstance {
    pub owner: PrefabInstanceId,
    pub texture: SpriteTexture,
    pub x: f32,
    pub y: f32,
    /// Draw size in map pixels. This normally matches the texture cell, while area outlines use one map tile.
    pub width: f32,
    pub height: f32,
    /// One-based map level containing this instance.
    pub z: u32,
    /// Whether this instance belongs to an `/area` subtype.
    pub is_area: bool,
    /// Exposed tile edges for area outlines.
    pub area_edges: u32,
    /// premultiplied RGBA
    pub color: [f32; 4],
    /// sort key, from plane and layer
    pub depth: f32,
}

pub const AREA_EDGE_NORTH: u32 = 1 << 0;
pub const AREA_EDGE_EAST: u32 = 1 << 1;
pub const AREA_EDGE_SOUTH: u32 = 1 << 2;
pub const AREA_EDGE_WEST: u32 = 1 << 3;
pub const AREA_EDGES_ALL: u32 = AREA_EDGE_NORTH | AREA_EDGE_EAST | AREA_EDGE_SOUTH | AREA_EDGE_WEST;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub x: f32,
    pub y: f32,
    pub zoom: f32,
    pub viewport_width: u32,
    pub viewport_height: u32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
            viewport_width: 1280,
            viewport_height: 720,
        }
    }
}

#[derive(Debug)]
pub struct Frame<'a> {
    /// Every placed sprite in the map, ordered by z and then draw order.
    pub sprite_instances: &'a [SpriteInstance],
    /// The level drawn sharp and above the blurred underlays.
    pub active_z: u32,
    /// How many levels immediately below `active_z` to draw as underlays.
    pub underlay_depth: u32,
    /// Whether normal area sprites participate in either draw pass.
    pub show_areas: bool,
    /// Whether area outline instances participate in either draw pass.
    pub show_area_outlines: bool,
    pub camera: Camera,
    /// Changes only when `sprite_instances` changes.
    pub revision: u64,
}

/// Turn a resolved [`Appearance`] into an instance. Needs a texture lookup to know the sprite's
/// descriptor index and dimensions, which is why it is not a method on `Appearance`.
///
/// An icon wider or taller than `tile_size` is anchored to the tile's bottom left and grows up and
/// to the right, the way BYOND draws one, so `pixel_x = -16` is what centres a 64 wide icon.
pub fn instance_for(
    owner: PrefabInstanceId, appearance: &Appearance, texture: SpriteTexture, tile: Coord, tile_size: u32,
    is_area: bool,
) -> SpriteInstance {
    let alpha = f32::from(appearance.alpha) / 255.0;
    let tint = appearance.color.as_deref().and_then(color::parse).unwrap_or([1.0; 4]);
    let alpha = alpha * tint[3];
    let offset_x = appearance.pixel_x.saturating_add(appearance.pixel_w);
    let offset_y = appearance.pixel_y.saturating_add(appearance.pixel_z);

    SpriteInstance {
        owner,
        texture,
        x: (tile.x.saturating_sub(1) * tile_size) as f32 + offset_x as f32,
        y: (tile.y.saturating_sub(1) * tile_size) as f32 + offset_y as f32,
        width: texture.width as f32,
        height: texture.height as f32,
        z: tile.z,
        is_area,
        area_edges: 0,
        color: [tint[0] * alpha, tint[1] * alpha, tint[2] * alpha, alpha],
        depth: appearance.plane * 1000.0 + appearance.layer,
    }
}

#[cfg(test)]
mod tests {
    use dmm::{Map, Size};
    use editor::{document::MapDocument, visual::Appearance};

    use crate::{SpriteTexture, VisibilityId, instance_for};

    fn owner() -> editor::document::PrefabInstanceId {
        let mut map = Map::new(Size { x: 1, y: 1, z: 1 });
        let key = map.intern_tile(vec![dmm::Prefab::new(core::path::TreePath::parse("/obj/test"))]);
        map.grid[0][0][0] = key;
        let document = MapDocument::new(map, 1);

        document.instance_ids_at(dmm::Coord::new(1, 1, 1))[0]
    }

    fn appearance(color: Option<&str>, alpha: u8) -> Appearance {
        Appearance {
            color: color.map(String::from),
            alpha,
            ..Default::default()
        }
    }

    #[test]
    fn visibility_ids_are_one_based_sprite_indices() {
        assert_eq!(VisibilityId::from_raw(0), None);
        assert_eq!(VisibilityId::from_sprite_index(0).unwrap().get(), 1);
        assert_eq!(VisibilityId::from_raw(17).unwrap().sprite_index(), 16);
        assert_eq!(VisibilityId::from_sprite_index(u32::MAX as usize), None);
    }

    #[test]
    fn a_tint_is_premultiplied_by_alpha() {
        let instance = instance_for(
            owner(),
            &appearance(Some("#ff0000"), 128),
            SpriteTexture::default(),
            dmm::Coord::new(1, 1, 1),
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
            dmm::Coord::new(1, 1, 1),
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
            dmm::Coord::new(1, 1, 1),
            32,
            false,
        );

        assert_eq!(instance.color, [1.0, 1.0, 1.0, 1.0]);
    }

    /// `pixel_w`/`pixel_z` stack on `pixel_x`/`pixel_y` rather than replacing them.
    #[test]
    fn both_pairs_of_pixel_offsets_shift_the_sprite() {
        let appearance = Appearance {
            pixel_x: -16,
            pixel_y: 4,
            pixel_w: 2,
            pixel_z: -1,
            ..Default::default()
        };
        let instance = instance_for(
            owner(),
            &appearance,
            SpriteTexture::default(),
            dmm::Coord::new(2, 3, 1),
            32,
            false,
        );

        assert_eq!((instance.x, instance.y), (32.0 - 14.0, 64.0 + 3.0));
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

        let plain = instance_for(
            owner(),
            &Appearance::default(),
            texture,
            dmm::Coord::new(1, 1, 1),
            32,
            false,
        );
        assert_eq!((plain.x, plain.y), (0.0, 0.0));

        let shifted = instance_for(owner(), &centred, texture, dmm::Coord::new(1, 1, 1), 32, false);
        assert_eq!((shifted.x, shifted.y), (-16.0, 0.0));
    }

    #[test]
    fn an_instance_keeps_its_level_and_area_classification() {
        let instance = instance_for(
            owner(),
            &Appearance::default(),
            SpriteTexture::default(),
            dmm::Coord::new(1, 1, 7),
            32,
            true,
        );

        assert_eq!(instance.z, 7);
        assert!(instance.is_area);
    }
}

pub(crate) fn extent3d(extent: ash::vk::Extent2D) -> ash::vk::Extent3D {
    ash::vk::Extent3D {
        width: extent.width,
        height: extent.height,
        depth: 1,
    }
}

pub(crate) fn read_spirv(bytes: &[u8]) -> Result<Vec<u32>, GpuError> {
    let mut cursor = std::io::Cursor::new(bytes);

    ash::util::read_spv(&mut cursor).map_err(|e| GpuError::Loader(e.to_string()))
}
