import common;

struct Sprite {
    f32x4 position_size;
    f32x4 color;
    f32x4 uv_rect;
    u32 texture;
    u32 flags;
};

static const u32 SPRITE_FLAG_AREA = 1;

[[vk::binding(0, 1)]]
Sampler2D textures[];

[[vk::binding(1, 0)]]
StructuredBuffer<Sprite> sprites;

struct VSOutput {
    f32x4 position : SV_Position;
    f32x2 uv;
    nointerpolation u32 sprite_id;
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
    uniform u32 show_areas
) -> VSOutput {
    let sprite_index = instance_id + base;
    let sprite = sprites[sprite_index];
    let corner = CORNERS[vertex_id];

    let map = sprite.position_size.xy + corner * sprite.position_size.zw;
    let view = (map - center) * zoom;
    let clip = view / (viewport * 0.5);

    VSOutput output;
    output.position = f32x4(clip.x, -clip.y, 0.0, 1.0);
    let local_uv = f32x2(corner.x, 1.0 - corner.y);
    output.uv = lerp(sprite.uv_rect.xy, sprite.uv_rect.zw, local_uv);
    output.sprite_id = ((sprite.flags & SPRITE_FLAG_AREA) != 0 && show_areas == 0) ? 0 : sprite_index + 1;

    return output;
}

func sample_sprite(VSOutput input) -> f32x4 {
    if (input.sprite_id == 0) {
        discard;
    }

    let sprite = sprites[input.sprite_id - 1];
    let texture_index = NonUniformResourceIndex(sprite.texture);
    let texel = textures[texture_index].Sample(input.uv);
    let color = f32x4(texel.rgb * sprite.color.rgb * texel.a, texel.a * sprite.color.a);

    if (color.a <= 0.0) {
        discard;
    }

    return color;
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
