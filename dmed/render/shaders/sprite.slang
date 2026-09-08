import common;
import sprite_shared;

[[vk::binding(0, 1)]]
Sampler2D textures[];

[[vk::binding(1, 0)]]
StructuredBuffer<Sprite> sprites;

struct VSOutput {
    f32x4 position : SV_Position;
    f32x2 uv;
    f32x2 local;
    nointerpolation u32 sprite_id;
    nointerpolation f32 placement_flash_strength;
};

static const f32x2 CORNERS[4] = {
    f32x2(0.0, 0.0),
    f32x2(1.0, 0.0),
    f32x2(0.0, 1.0),
    f32x2(1.0, 1.0),
};

[shader("vertex")]
func vs_main(
    u32 vertex_id : SV_VertexID,
    u32 instance_id : SV_InstanceID,
    uniform f32x2 center,
    uniform f32x2 viewport,
    uniform f32 zoom,
    uniform u32 base,
    uniform u32 show_areas,
    uniform u32 show_area_outlines,
    uniform u32x2 placement_flash_owner,
    uniform f32 placement_flash_strength
) -> VSOutput {
    let sprite_index = instance_id + base;
    let sprite = sprites[sprite_index];
    let corner = CORNERS[vertex_id];
    let position_size = sprite.position_size();
    let flags = sprite.flags();

    let map = position_size.xy + corner * position_size.zw;
    let view = (map - center) * zoom;
    let clip = view / (viewport * 0.5);

    VSOutput output;
    output.position = f32x4(clip.x, -clip.y, 0.0, 1.0);
    let local_uv = f32x2(corner.x, 1.0 - corner.y);
    output.uv = f32x2(sprite.source_position()) + local_uv * f32x2(sprite.source_size());
    output.local = corner;
    let is_area = (flags & SPRITE_FLAG_AREA) != 0;
    let is_area_outline = (flags & SPRITE_AREA_EDGE_MASK) != 0;
    let hidden = (is_area_outline && show_area_outlines == 0) ||
        (is_area && !is_area_outline && show_areas == 0);
    output.sprite_id = hidden ? 0 : sprite_index + 1;
    output.placement_flash_strength = all(sprite.owner == placement_flash_owner)
        ? saturate(placement_flash_strength)
        : 0.0;

    return output;
}

func apply_placement_flash(f32x4 color, f32 strength) -> f32x4 {
    let green = f32x4(0.0, color.a, 0.0, color.a);

    return lerp(color, green, strength);
}

func sample_sprite(VSOutput input) -> f32x4 {
    if (input.sprite_id == 0) {
        discard;
    }

    let sprite = sprites[input.sprite_id - 1];
    let flags = sprite.flags();
    let color = sprite.color();
    if ((flags & SPRITE_AREA_EDGE_MASK) != 0) {
        let pixel = fwidth(input.local);
        let on_outline =
            ((flags & SPRITE_AREA_EDGE_NORTH) != 0 && input.local.y >= 1.0 - pixel.y) ||
            ((flags & SPRITE_AREA_EDGE_EAST) != 0 && input.local.x >= 1.0 - pixel.x) ||
            ((flags & SPRITE_AREA_EDGE_SOUTH) != 0 && input.local.y <= pixel.y) ||
            ((flags & SPRITE_AREA_EDGE_WEST) != 0 && input.local.x <= pixel.x);
        if (!on_outline) {
            discard;
        }

        return apply_placement_flash(color, input.placement_flash_strength);
    }

    let texture_index = NonUniformResourceIndex(sprite.texture());
    u32 texture_width;
    u32 texture_height;
    textures[texture_index].GetDimensions(texture_width, texture_height);
    let uv = input.uv / f32x2(texture_width, texture_height);
    let texel = textures[texture_index].Sample(uv);
    let shaded = f32x4(texel.rgb * color.rgb * texel.a, texel.a * color.a);

    if (shaded.a <= 0.0) {
        discard;
    }

    return apply_placement_flash(shaded, input.placement_flash_strength);
}

[shader("fragment")]
func fs_main(VSOutput input) -> f32x4 {
    return sample_sprite(input);
}

struct VisibilityOutput {
    f32x4 color : SV_Target0;
    u32 visibility : SV_Target1;
};

[shader("fragment")]
func fs_visibility(VSOutput input) -> VisibilityOutput {
    VisibilityOutput output;
    output.color = sample_sprite(input);
    output.visibility = input.sprite_id;

    return output;
}
