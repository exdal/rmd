import common;

[[vk::binding(0, 0)]]
Sampler2D texture;

struct VSInput {
    f32x2 position;
    f32x2 uv;
    u32 color;
};

struct VSOutput {
    f32x4 position : SV_Position;
    f32x4 color;
    f32x2 uv;
};

[shader("vertex")]
func vs_main(
    VSInput input,
    uniform f32x2 scale,
    uniform f32x2 translate
) -> VSOutput {
    var packed = f32x4(
        f32(input.color & 0xffu),
        f32((input.color >> 8) & 0xffu),
        f32((input.color >> 16) & 0xffu),
        f32((input.color >> 24) & 0xffu));
    packed /= 255.0;

    VSOutput output;
    output.position = f32x4(input.position * scale + translate, 0.0, 1.0);
    output.color = packed;
    output.uv = input.uv;
    output.color.rgb = srgb_to_linear(packed.rgb);
    output.color.a = packed.a;

    return output;
}

[shader("fragment")]
f32x4 fs_main(VSOutput input) : SV_Target {
    return input.color * texture.Sample(input.uv);
}
