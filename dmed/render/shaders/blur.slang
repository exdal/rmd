import common;

[[vk::binding(0, 0)]]
Sampler2D source;

struct VSOutput {
    f32x4 position : SV_Position;
    f32x2 uv;
};

[shader("vertex")]
func vs_main(
    u32 vertex_id : SV_VertexID
) -> VSOutput {
    let uv = f32x2((vertex_id << 1) & 2, vertex_id & 2);

    VSOutput output;
    output.position = f32x4(uv * 2.0 - 1.0, 0.0, 1.0);
    output.uv = uv;
    return output;
}

static const i32 SAMPLES = 8;
static const i32 LOD = 0;
static const i32 sLOD = 1 << LOD;
static const i32 TEXELS = SAMPLES / sLOD;
static const f32 SIGMA = f32(SAMPLES) * 0.25;
static const f32 BLUR_RADIUS = 0.65;
static const f32 DIM = 0.33;

func gaussian(f32x2 offset) -> f32 {
    let scaled = offset / SIGMA;
    return exp(-0.5 * dot(scaled, scaled)) / (6.28 * SIGMA * SIGMA);
}

[shader("fragment")]
func fs_main(
    VSOutput input,
    uniform f32x2 sample_step
) -> f32x4 {
    var color = f32x4(0.0);

    for (i32 i = 0; i < TEXELS * TEXELS; i++) {
        let offset = f32x2(i % TEXELS, i / TEXELS) * f32(sLOD) - f32(SAMPLES) / 2.0;
        color += gaussian(offset) * source.SampleLevel(input.uv + offset * sample_step * BLUR_RADIUS, f32(LOD));
    }

    return f32x4(color.rgb * DIM, color.a);
}
