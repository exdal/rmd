import common;

struct Sprite {
    f32x4 position_size;
    f32x4 color;
    i32 texture;
};

[[vk::binding(0, 1)]]
Sampler2D textures[];

[[vk::binding(1, 0)]]
StructuredBuffer<Sprite> sprites;

struct VSOutput {
    f32x4 position : SV_Position;
    f32x2 uv;
    nointerpolation u32 texture_index;
    f32x4 color;
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
    uniform u32 base
) -> VSOutput {
    let sprite = sprites[instance_id + base];
    let corner = CORNERS[vertex_id];

    let map = sprite.position_size.xy + corner * sprite.position_size.zw;
    let view = (map - center) * zoom;
    let clip = view / (viewport * 0.5);

    VSOutput output;
    output.position = f32x4(clip.x, -clip.y, 0.0, 1.0);
    output.uv = f32x2(corner.x, 1.0 - corner.y);
    output.texture_index = sprite.texture;
    output.color = sprite.color;

    return output;
}

[shader("fragment")]
func fs_main(VSOutput input) -> f32x4 {
    let texture_index = NonUniformResourceIndex(input.texture_index);
    let texel = textures[texture_index].Sample(input.uv);

    // The tint RGB already includes sprite opacity; premultiply only by texture alpha.
    return f32x4(texel.rgb * input.color.rgb * texel.a, texel.a * input.color.a);
}
