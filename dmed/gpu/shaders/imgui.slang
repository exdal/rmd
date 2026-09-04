[[vk::binding(0, 0)]]
Sampler2D texture;

struct VSInput {
    float2 position : POSITION;
    float2 uv : TEXCOORD;
    uint color : COLOR;
};

struct VSOutput {
    float4 position : SV_Position;
    float4 color : COLOR;
    float2 uv : TEXCOORD;
};

float3 linear_from_srgb(float3 srgb) {
    float3 lower = srgb / 12.92;
    float3 higher = pow((srgb + 0.055) / 1.055, 2.4);
    return lerp(lower, higher, step(0.04045, srgb));
}

[shader("vertex")]
func vs_main(
    VSInput input,
    uniform float2 scale,
    uniform float2 translate
) -> VSOutput {
    float4 packed = float4(
                        float(input.color & 0xffu),
                        float((input.color >> 8) & 0xffu),
                        float((input.color >> 16) & 0xffu),
                        float((input.color >> 24) & 0xffu))
                    / 255.0;

    VSOutput output;
    output.position = float4(input.position * scale + translate, 0.0, 1.0);
    output.color = float4(linear_from_srgb(packed.rgb), packed.a);
    output.uv = input.uv;
    return output;
}

[shader("fragment")]
float4 fs_main(VSOutput input) : SV_Target {
    return input.color * texture.Sample(input.uv);
}
