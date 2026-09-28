module sprite_shared;

import common;

public static const u32 SPRITE_FLAG_AREA = 1;
public static const u32 SPRITE_AREA_EDGE_NORTH = 1 << 1;
public static const u32 SPRITE_AREA_EDGE_EAST = 1 << 2;
public static const u32 SPRITE_AREA_EDGE_SOUTH = 1 << 3;
public static const u32 SPRITE_AREA_EDGE_WEST = 1 << 4;
public static const u32 SPRITE_AREA_EDGE_MASK =
    SPRITE_AREA_EDGE_NORTH | SPRITE_AREA_EDGE_EAST | SPRITE_AREA_EDGE_SOUTH | SPRITE_AREA_EDGE_WEST;
public static const u32 SPRITE_FLAG_EMISSIVE_MASK = 1 << 5;
public static const u32 SPRITE_FLAG_EMISSIVE_BLOCKER = 1 << 6;
public static const u32 SPRITE_FLAG_OVERLAY_LIGHT = 1 << 7;
public static const u32 SPRITE_FLAG_OVERLAY_LIGHT_SUBTRACT = 1 << 8;
public static const u32 SPRITE_FLAG_FULLBRIGHT = 1 << 9;
public static const u32 SPRITE_FLAG_CLICK_THROUGH = 1 << 10;
public static const u32 SPRITE_FLAGS_SHIFT = 21;
public static const u32 SPRITE_TEXTURE_MASK = (1 << SPRITE_FLAGS_SHIFT) - 1;

public struct Sprite {
    public u32x2 owner;
    public u32x2 area_owner;
    public f32x2 position;
    public u32 z;
    public u32 packed_size;
    public u32 packed_color;
    public u32x2 packed_source_position_size;
    public u32 packed_texture_flags;
    public u32x2 packed_transform;

    public func position_size() -> f32x4 {
        return f32x4(this.position, unpack_half2(this.packed_size));
    }

    /// `a`, `b`, `d`, `e`, about the sprite's center
    public func transform() -> f32x4 {
        return f32x4(unpack_half2(this.packed_transform.x), unpack_half2(this.packed_transform.y));
    }

    public func turned(f32x2 offset) -> f32x2 {
        let transform = this.transform();

        return f32x2(
            transform.x * offset.x + transform.y * offset.y,
            transform.z * offset.x + transform.w * offset.y
        );
    }

    public func color() -> f32x4 {
        return unpack_unorm4x8(this.packed_color);
    }

    public func source_position() -> u32x2 {
        return unpack_u16x2(this.packed_source_position_size.x);
    }

    public func source_size() -> u32x2 {
        return unpack_u16x2(this.packed_source_position_size.y);
    }

    public func texture() -> u32 {
        return this.packed_texture_flags & SPRITE_TEXTURE_MASK;
    }

    public func flags() -> u32 {
        return this.packed_texture_flags >> SPRITE_FLAGS_SHIFT;
    }
};
