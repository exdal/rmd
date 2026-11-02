use crate::{AREA_EDGES_ALL, GpuError, SpriteInstance};

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct GpuSprite {
    owner: [u32; 2],
    area_owner: [u32; 2],
    position: [f32; 2],
    z: u32,
    packed_size: u32,
    color: u32,
    source_position_size: [u32; 2],
    texture_flags: u32,
    transform: [u32; 2],
    pub(super) node_pickable: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct GpuLightTile {
    corners: [u32; 4],
}

const SPRITE_FLAG_AREA: u32 = 1;
const SPRITE_AREA_EDGE_SHIFT: u32 = 1;
const SPRITE_FLAG_EMISSIVE_MASK: u32 = 1 << 5;
const SPRITE_FLAG_EMISSIVE_BLOCKER: u32 = 1 << 6;
const SPRITE_FLAG_OVERLAY_LIGHT: u32 = 1 << 7;
const SPRITE_FLAG_OVERLAY_LIGHT_SUBTRACT: u32 = 1 << 8;
const SPRITE_FLAG_FULLBRIGHT: u32 = 1 << 9;
const SPRITE_FLAG_CLICK_THROUGH: u32 = 1 << 10;
const SPRITE_FLAGS_SHIFT: u32 = 21;
const SPRITE_TEXTURE_MASK: u32 = (1 << SPRITE_FLAGS_SHIFT) - 1;
pub(super) const SPRITE_TEXTURE_CAPACITY: u32 = SPRITE_TEXTURE_MASK + 1;
// the cull sorts visible sprites into one list per kind, so each pass draws only what it shades
pub(super) const SPRITE_LIST_SCENE: u32 = 0;
pub(super) const SPRITE_LIST_OVERLAY_LIGHT: u32 = 1;
pub(super) const SPRITE_LIST_AREA_OUTLINE: u32 = 2;
pub(super) const SPRITE_LISTS: u32 = 3;

pub(super) fn gpu_sprite_offset(start: usize) -> Result<u64, GpuError> {
    start
        .checked_mul(size_of::<GpuSprite>())
        .and_then(|offset| u64::try_from(offset).ok())
        .ok_or(GpuError::SpriteUploadTooLarge)
}

pub(super) fn gpu_sprite_range(
    sprites: &[SpriteInstance], range: crate::UpdateRange,
) -> Result<Vec<GpuSprite>, GpuError> {
    sprites[range.start..range.end]
        .iter()
        .enumerate()
        .map(|(offset, sprite)| gpu_sprite(range.start + offset, sprite))
        .collect()
}

pub(super) fn gpu_light_tile(index: usize, tile: &crate::LightTile) -> Result<GpuLightTile, GpuError> {
    let mut corners = [0; 4];
    for (packed, corner) in corners.iter_mut().zip(tile.corners) {
        *packed = pack_unorm4x8([corner[0], corner[1], corner[2], 1.0]).ok_or(GpuError::SpritePackingOutOfRange {
            sprite: index,
            field: "light color",
        })?;
    }

    Ok(GpuLightTile { corners })
}

pub(super) fn gpu_light_offset(start: usize) -> Result<u64, GpuError> {
    start
        .checked_mul(size_of::<GpuLightTile>())
        .and_then(|offset| u64::try_from(offset).ok())
        .ok_or(GpuError::SpriteUploadTooLarge)
}

pub(super) fn gpu_sprite(index: usize, sprite: &SpriteInstance) -> Result<GpuSprite, GpuError> {
    // the cull pass drops it like a spare page slot, and the cache keeps the real sprite to show it again
    if sprite.hidden {
        return gpu_sprite(
            index,
            &SpriteInstance {
                x: -1.0e9,
                y: -1.0e9,
                width: 0.0,
                height: 0.0,
                color: [0.0; 4],
                hidden: false,
                ..*sprite
            },
        );
    }

    let mut flags = if sprite.is_area {
        SPRITE_FLAG_AREA | ((sprite.area_edges & AREA_EDGES_ALL) << SPRITE_AREA_EDGE_SHIFT)
    } else {
        0
    };
    if sprite.is_area && sprite.area_edges == 0 {
        flags |= SPRITE_FLAG_FULLBRIGHT;
    }
    if sprite.click_through {
        flags |= SPRITE_FLAG_CLICK_THROUGH;
    }
    flags |= match sprite.lighting {
        crate::SpriteLighting::Normal => 0,
        crate::SpriteLighting::Emissive => SPRITE_FLAG_EMISSIVE_MASK,
        crate::SpriteLighting::Blocker => SPRITE_FLAG_EMISSIVE_BLOCKER,
        crate::SpriteLighting::OverlayLight => SPRITE_FLAG_OVERLAY_LIGHT,
        crate::SpriteLighting::OverlayLightSubtract => SPRITE_FLAG_OVERLAY_LIGHT_SUBTRACT,
    };

    let out_of_range = |field| GpuError::SpritePackingOutOfRange { sprite: index, field };
    if !sprite.x.is_finite() || !sprite.y.is_finite() {
        return Err(out_of_range("position"));
    }
    let position = [sprite.x, sprite.y];
    let size = pack_half2(sprite.width, sprite.height).ok_or_else(|| out_of_range("size"))?;
    let color = pack_unorm4x8(sprite.color).ok_or_else(|| out_of_range("color"))?;
    let source_position = pack_u16x2(sprite.texture.source_position).ok_or_else(|| out_of_range("source position"))?;
    let source_size =
        pack_u16x2([sprite.texture.width, sprite.texture.height]).ok_or_else(|| out_of_range("source size"))?;
    if sprite.texture.index > SPRITE_TEXTURE_MASK {
        return Err(out_of_range("texture index"));
    }
    let [a, b, d, e] = sprite.transform;
    let transform = [
        pack_half2(a, b).ok_or_else(|| out_of_range("transform"))?,
        pack_half2(d, e).ok_or_else(|| out_of_range("transform"))?,
    ];

    Ok(GpuSprite {
        owner: owner_words(sprite.owner),
        area_owner: sprite.area_owner.map(owner_words).unwrap_or([0; 2]),
        position,
        z: sprite.z,
        packed_size: size,
        color,
        source_position_size: [source_position, source_size],
        texture_flags: sprite.texture.index | (flags << SPRITE_FLAGS_SHIFT),
        transform,
        node_pickable: u32::from(sprite.node_pickable),
    })
}

pub(super) fn owner_words(owner: dmm::PrefabInstanceId) -> [u32; 2] {
    split_owner(owner.get())
}

fn split_owner(owner: u64) -> [u32; 2] {
    [owner as u32, (owner >> 32) as u32]
}

/// Reinterprets a slice of tightly packed `repr(C)` shader records as the bytes to upload.
pub(super) fn payload_bytes<T: Copy>(values: &[T]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), std::mem::size_of_val(values)) }
}

fn pack_half2(x: f32, y: f32) -> Option<u32> {
    let x = meshopt_quantize_half(x);
    let y = meshopt_quantize_half(y);

    ((x & 0x7c00) != 0x7c00 && (y & 0x7c00) != 0x7c00).then_some(u32::from(x) | (u32::from(y) << 16))
}

fn meshopt_quantize_half(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = (bits >> 16) & 0x8000;
    let exponent_mantissa = (bits & 0x7fff_ffff) as i32;

    let mut half = (exponent_mantissa - (112 << 23) + (1 << 12)) >> 13;
    half = if exponent_mantissa < (113 << 23) { 0 } else { half };
    half = if exponent_mantissa >= (143 << 23) { 0x7c00 } else { half };
    half = if exponent_mantissa > (255 << 23) { 0x7e00 } else { half };

    (sign | half as u32) as u16
}

#[cfg(test)]
fn meshopt_dequantize_half(half: u16) -> f32 {
    let sign = u32::from(half & 0x8000) << 16;
    let exponent_mantissa = u32::from(half & 0x7fff);

    let mut result = (exponent_mantissa + (112 << 10)) << 13;
    result = if exponent_mantissa < (1 << 10) { 0 } else { result };
    result += if exponent_mantissa >= (31 << 10) { 112 << 23 } else { 0 };

    f32::from_bits(sign | result)
}

fn pack_unorm4x8(values: [f32; 4]) -> Option<u32> {
    if !values
        .iter()
        .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
    {
        return None;
    }

    Some(
        meshopt_quantize_unorm(values[0], 8)
            | (meshopt_quantize_unorm(values[1], 8) << 8)
            | (meshopt_quantize_unorm(values[2], 8) << 16)
            | (meshopt_quantize_unorm(values[3], 8) << 24),
    )
}

fn meshopt_quantize_unorm(value: f32, bits: u32) -> u32 {
    debug_assert!((1..=16).contains(&bits));

    let scale = ((1 << bits) - 1) as f32;
    (value * scale + 0.5) as u32
}

#[cfg(test)]
fn meshopt_dequantize_unorm(value: u32, bits: u32) -> f32 {
    debug_assert!((1..=16).contains(&bits));

    value as f32 / ((1 << bits) - 1) as f32
}

fn pack_u16x2(values: [u32; 2]) -> Option<u32> {
    (values[0] <= u16::MAX.into() && values[1] <= u16::MAX.into()).then_some(values[0] | (values[1] << 16))
}

#[cfg(test)]
mod tests {
    use dmm::PrefabInstanceId;

    use super::*;
    use crate::{
        AREA_EDGE_EAST, AREA_EDGE_NORTH, LightTile, SpriteLighting, SpriteTexture, renderer::fixtures::sprite,
    };

    #[test]
    fn area_edges_and_tile_geometry_reach_the_gpu() {
        let mut area = sprite(37);
        area.area_owner = PrefabInstanceId::from_raw(0x1234_5678_9abc_def0);
        area.x = 32.0;
        area.y = 64.0;
        area.width = 32.0;
        area.height = 32.0;
        area.is_area = true;
        area.area_edges = AREA_EDGE_NORTH | AREA_EDGE_EAST;
        area.texture = SpriteTexture {
            index: 3,
            source_position: [4, 8],
            width: 16,
            height: 24,
        };
        area.color = [0.0; 4];

        let gpu = gpu_sprite(0, &area).expect("pack");

        assert_eq!(gpu.owner, [area.owner.get() as u32, 0]);
        assert_eq!(gpu.area_owner, [0x9abc_def0, 0x1234_5678]);
        assert_eq!(gpu.position, [32.0, 64.0]);
        assert_eq!(gpu.z, 37);
        assert_eq!(gpu.packed_size, 0x5000_5000);
        assert_eq!(gpu.color, 0);
        assert_eq!(gpu.source_position_size, [0x0008_0004, 0x0018_0010]);
        assert_eq!(gpu.texture_flags & SPRITE_TEXTURE_MASK, 3);
        assert_eq!(
            gpu.texture_flags >> SPRITE_FLAGS_SHIFT,
            1 | ((AREA_EDGE_NORTH | AREA_EDGE_EAST) << 1)
        );
    }

    #[test]
    fn filled_area_sprites_are_fullbright_without_outline_flags() {
        let mut area = sprite(1);
        area.is_area = true;

        let gpu = gpu_sprite(0, &area).expect("pack");

        assert_eq!(
            gpu.texture_flags >> SPRITE_FLAGS_SHIFT,
            SPRITE_FLAG_AREA | SPRITE_FLAG_FULLBRIGHT
        );
    }

    #[test]
    fn click_through_sprites_keep_the_largest_texture_index() {
        let mut openspace = sprite(1);
        openspace.click_through = true;
        openspace.texture.index = SPRITE_TEXTURE_MASK;

        let gpu = gpu_sprite(0, &openspace).expect("pack");

        assert_eq!(gpu.texture_flags & SPRITE_TEXTURE_MASK, SPRITE_TEXTURE_MASK);
        assert_eq!(gpu.texture_flags >> SPRITE_FLAGS_SHIFT, SPRITE_FLAG_CLICK_THROUGH);
    }

    #[test]
    fn node_eligibility_does_not_consume_texture_or_appearance_flags() {
        let mut node = sprite(1);
        assert_eq!(gpu_sprite(0, &node).unwrap().node_pickable, 0);
        node.node_pickable = true;
        node.texture.index = SPRITE_TEXTURE_MASK;
        node.click_through = true;
        let gpu = gpu_sprite(0, &node).unwrap();
        assert_eq!(gpu.node_pickable, 1);
        assert_eq!(gpu.texture_flags & SPRITE_TEXTURE_MASK, SPRITE_TEXTURE_MASK);
        assert_eq!(gpu.texture_flags >> SPRITE_FLAGS_SHIFT, SPRITE_FLAG_CLICK_THROUGH);
    }

    #[test]
    fn lighting_roles_reach_the_gpu_flags() {
        let mut emissive = sprite(1);
        emissive.lighting = SpriteLighting::Emissive;
        let mut blocker = sprite(1);
        blocker.lighting = SpriteLighting::Blocker;
        let mut overlay = sprite(1);
        overlay.lighting = SpriteLighting::OverlayLight;
        let mut subtract = sprite(1);
        subtract.lighting = SpriteLighting::OverlayLightSubtract;

        let emissive = gpu_sprite(0, &emissive).expect("pack emissive");
        let blocker = gpu_sprite(1, &blocker).expect("pack blocker");
        let overlay = gpu_sprite(2, &overlay).expect("pack overlay light");
        let subtract = gpu_sprite(3, &subtract).expect("pack subtractive overlay light");

        assert_eq!(emissive.texture_flags >> SPRITE_FLAGS_SHIFT, SPRITE_FLAG_EMISSIVE_MASK);
        assert_ne!(SPRITE_FLAG_EMISSIVE_MASK, SPRITE_FLAG_FULLBRIGHT);
        assert_eq!(
            blocker.texture_flags >> SPRITE_FLAGS_SHIFT,
            SPRITE_FLAG_EMISSIVE_BLOCKER
        );
        assert_eq!(overlay.texture_flags >> SPRITE_FLAGS_SHIFT, SPRITE_FLAG_OVERLAY_LIGHT);
        assert_eq!(
            subtract.texture_flags >> SPRITE_FLAGS_SHIFT,
            SPRITE_FLAG_OVERLAY_LIGHT_SUBTRACT
        );
    }

    #[test]
    fn meshopt_half_conversion_matches_reference_values() {
        assert_eq!(meshopt_quantize_half(0.0), 0x0000);
        assert_eq!(meshopt_quantize_half(-0.0), 0x8000);
        assert_eq!(meshopt_quantize_half(1.0), 0x3c00);
        assert_eq!(meshopt_quantize_half(-2.0), 0xc000);
        assert_eq!(meshopt_quantize_half(65_504.0), 0x7bff);
        assert_eq!(meshopt_quantize_half(f32::INFINITY), 0x7c00);
        assert_eq!(meshopt_quantize_half(f32::NAN), 0x7e00);
        assert_eq!(meshopt_quantize_half(f32::MIN_POSITIVE), 0x0000);

        assert_eq!(meshopt_dequantize_half(0x0000).to_bits(), 0.0f32.to_bits());
        assert_eq!(meshopt_dequantize_half(0x8000).to_bits(), (-0.0f32).to_bits());
        assert_eq!(meshopt_dequantize_half(0x3c00), 1.0);
        assert_eq!(meshopt_dequantize_half(0xc000), -2.0);
        assert_eq!(meshopt_dequantize_half(0x7bff), 65_504.0);
        assert!(meshopt_dequantize_half(0x7c00).is_infinite());
        assert!(meshopt_dequantize_half(0x7e00).is_nan());
    }

    #[test]
    fn color_uses_rgba_unorm8_order() {
        let packed = pack_unorm4x8([0.0, 0.5, 1.0, 128.0 / 255.0]).expect("pack");

        assert_eq!(packed, 0x80ff_8000);
        assert_eq!(meshopt_dequantize_unorm((packed >> 8) & 0xff, 8), 128.0 / 255.0);
        assert_eq!(meshopt_dequantize_unorm((packed >> 16) & 0xff, 8), 1.0);
    }

    #[test]
    fn light_tiles_use_four_rgb_unorm8_corners() {
        let tile = LightTile {
            corners: [[1.0, 0.0, 0.0], [0.0, 0.5, 0.0], [0.0, 0.0, 1.0], [0.25; 3]],
        };
        let packed = gpu_light_tile(0, &tile).expect("pack light tile");

        assert_eq!(size_of_val(&packed), 16);
        assert_eq!(packed.corners[0], 0xff00_00ff);
        assert_eq!(packed.corners[1], 0xff00_8000);
        assert_eq!(packed.corners[2], 0xffff_0000);
    }

    #[test]
    fn a_sprite_transform_packs_as_two_half_pairs() {
        let mut instance = sprite(1);
        let identity = gpu_sprite(0, &instance).expect("identity transform");
        assert_eq!(identity.transform, [0x0000_3c00, 0x3c00_0000]);

        instance.transform = [0.0, 1.0, -1.0, 0.0];
        let turned = gpu_sprite(0, &instance).expect("turned transform");
        assert_eq!(turned.transform, [0x3c00_0000, 0x0000_bc00]);

        instance.transform[0] = 70_000.0;
        assert_eq!(
            gpu_sprite(7, &instance),
            Err(GpuError::SpritePackingOutOfRange {
                sprite: 7,
                field: "transform"
            })
        );
    }

    #[test]
    fn sprite_packing_rejects_values_outside_the_wire_format() {
        let mut instance = sprite(1);
        instance.x = 70_000.0;
        assert_eq!(
            gpu_sprite(7, &instance).expect("f32 position").position,
            [70_000.0, 0.0]
        );

        instance.x = f32::NAN;
        assert_eq!(
            gpu_sprite(7, &instance),
            Err(GpuError::SpritePackingOutOfRange {
                sprite: 7,
                field: "position"
            })
        );

        instance.x = 0.0;
        instance.width = 70_000.0;
        assert_eq!(
            gpu_sprite(7, &instance),
            Err(GpuError::SpritePackingOutOfRange {
                sprite: 7,
                field: "size"
            })
        );

        instance.width = 0.0;
        instance.color[0] = f32::NAN;
        assert_eq!(
            gpu_sprite(7, &instance),
            Err(GpuError::SpritePackingOutOfRange {
                sprite: 7,
                field: "color"
            })
        );

        instance.color = [1.0; 4];
        instance.texture.source_position = [u16::MAX as u32 + 1, 0];
        assert_eq!(
            gpu_sprite(7, &instance),
            Err(GpuError::SpritePackingOutOfRange {
                sprite: 7,
                field: "source position"
            })
        );

        instance.texture.source_position = [0, 0];
        instance.texture.width = u16::MAX as u32 + 1;
        assert_eq!(
            gpu_sprite(7, &instance),
            Err(GpuError::SpritePackingOutOfRange {
                sprite: 7,
                field: "source size"
            })
        );

        instance.texture.width = 0;
        instance.texture.index = SPRITE_TEXTURE_MASK + 1;
        assert_eq!(
            gpu_sprite(7, &instance),
            Err(GpuError::SpritePackingOutOfRange {
                sprite: 7,
                field: "texture index"
            })
        );
    }

    #[test]
    fn sprite_positions_keep_single_pixel_precision_at_large_coordinates() {
        let mut instance = sprite(1);
        instance.x = 2_080.0;
        instance.y = -2_080.0;
        let original = gpu_sprite(0, &instance).expect("pack");

        instance.x -= 1.0;
        instance.y += 1.0;
        let shifted = gpu_sprite(0, &instance).expect("pack");

        assert_eq!(original.position, [2_080.0, -2_080.0]);
        assert_eq!(shifted.position, [2_079.0, -2_079.0]);
        assert_ne!(original.position, shifted.position);
    }

    #[test]
    fn owner_ids_keep_both_words() {
        assert_eq!(split_owner(0x1234_5678_9abc_def0), [0x9abc_def0, 0x1234_5678]);
    }
}
