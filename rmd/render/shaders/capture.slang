import common;

[[vk::binding(0, 0)]]
Texture2D<f32x4> scene;

[[vk::binding(1, 0)]]
RWStructuredBuffer<u32> pixels;

[shader("compute")]
[numthreads(8, 8, 1)]
func cs_main(u32x3 id : SV_DispatchThreadID, uniform u32x2 extent) {
    if (any(id.xy >= extent)) {
        return;
    }

    let color = scene.Load(i32x3(i32x2(id.xy), 0));
    pixels[id.y * extent.x + id.x] = pack_unorm4x8(f32x4(linear_to_srgb(color.rgb), color.a));
}
