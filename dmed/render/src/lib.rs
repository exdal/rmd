pub mod color;
pub mod device;
pub mod error;
pub mod frame;
pub mod imgui;
pub mod renderer;
pub mod texture;

use editor::visual::Appearance;

pub use crate::{device::Device, error::GpuError, renderer::Renderer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SpriteTexture {
    pub index: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpriteInstance {
    pub texture: SpriteTexture,
    pub x: f32,
    pub y: f32,
    /// premultiplied RGBA
    pub color: [f32; 4],
    /// sort key, from plane and layer
    pub depth: f32,
}

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

#[derive(Debug, Default)]
pub struct Frame {
    /// The active level, drawn sharp and over everything else.
    pub sprites: Vec<SpriteInstance>,
    /// The levels below the active one, deepest first.
    pub underlays: Vec<Vec<SpriteInstance>>,
    pub camera: Camera,
    pub revision: u64,
}

/// Turn a resolved [`Appearance`] into an instance. Needs a texture lookup to know the sprite's
/// descriptor index and dimensions, which is why it is not a method on `Appearance`.
///
/// An icon wider or taller than `tile_size` is anchored to the tile's bottom left and grows up and
/// to the right, the way BYOND draws one, so `pixel_x = -16` is what centres a 64 wide icon.
pub fn instance_for(
    appearance: &Appearance, texture: SpriteTexture, tile_x: u32, tile_y: u32, tile_size: u32,
) -> SpriteInstance {
    let alpha = f32::from(appearance.alpha) / 255.0;
    let tint = appearance.color.as_deref().and_then(color::parse).unwrap_or([1.0; 4]);
    let alpha = alpha * tint[3];
    let offset_x = appearance.pixel_x.saturating_add(appearance.pixel_w);
    let offset_y = appearance.pixel_y.saturating_add(appearance.pixel_z);

    SpriteInstance {
        texture,
        x: (tile_x.saturating_sub(1) * tile_size) as f32 + offset_x as f32,
        y: (tile_y.saturating_sub(1) * tile_size) as f32 + offset_y as f32,
        color: [tint[0] * alpha, tint[1] * alpha, tint[2] * alpha, alpha],
        depth: appearance.plane * 1000.0 + appearance.layer,
    }
}

#[cfg(test)]
mod tests {
    use editor::visual::Appearance;

    use crate::{SpriteTexture, instance_for};

    fn appearance(color: Option<&str>, alpha: u8) -> Appearance {
        Appearance {
            color: color.map(String::from),
            alpha,
            ..Default::default()
        }
    }

    #[test]
    fn a_tint_is_premultiplied_by_alpha() {
        let instance = instance_for(&appearance(Some("#ff0000"), 128), SpriteTexture::default(), 1, 1, 32);
        let alpha = 128.0 / 255.0;

        assert_eq!(instance.color, [alpha, 0.0, 0.0, alpha]);
    }

    #[test]
    fn an_untinted_sprite_keeps_its_alpha_on_every_channel() {
        let instance = instance_for(&appearance(None, 255), SpriteTexture::default(), 1, 1, 32);

        assert_eq!(instance.color, [1.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn an_unreadable_color_falls_back_to_no_tint() {
        let instance = instance_for(&appearance(Some("chartreuse"), 255), SpriteTexture::default(), 1, 1, 32);

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
        let instance = instance_for(&appearance, SpriteTexture::default(), 2, 3, 32);

        assert_eq!((instance.x, instance.y), (32.0 - 14.0, 64.0 + 3.0));
    }

    /// A 64x64 icon hangs off the top and the right of its tile, so `pixel_x = -16` centres it.
    #[test]
    fn a_larger_than_tile_icon_anchors_to_the_bottom_left_of_its_tile() {
        let texture = SpriteTexture {
            index: 0,
            width: 64,
            height: 64,
        };
        let centred = Appearance {
            pixel_x: -16,
            ..Default::default()
        };

        let plain = instance_for(&Appearance::default(), texture, 1, 1, 32);
        assert_eq!((plain.x, plain.y), (0.0, 0.0));

        let shifted = instance_for(&centred, texture, 1, 1, 32);
        assert_eq!((shifted.x, shifted.y), (-16.0, 0.0));
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
