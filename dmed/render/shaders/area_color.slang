import common;
import sprite_shared;

static const u32 WORKGROUP_SIZE = 64;

[[vk::binding(0, 1)]]
Sampler2D textures[];

[[vk::binding(1, 0)]]
RWStructuredBuffer<Sprite> sprites;

groupshared f32x4 partial[WORKGROUP_SIZE];

[shader("compute")]
[numthreads(WORKGROUP_SIZE, 1, 1)]
func cs_main(
    u32x3 group : SV_GroupID,
    u32x3 thread : SV_GroupThreadID,
    uniform u32 first_sprite,
    uniform u32 sprite_count,
    uniform u32 group_count
) {
    let lane = thread.x;
    for (u32 relative_index = group.x; relative_index < sprite_count; relative_index += group_count) {
        let sprite_index = first_sprite + relative_index;
        let sprite = sprites[sprite_index];
        if ((sprite.flags() & SPRITE_AREA_EDGE_MASK) == 0) {
            continue;
        }

        let source_position = sprite.source_position();
        let source_size = sprite.source_size();
        let pixel_count = source_size.x * source_size.y;
        var sum = f32x4(0.0);

        if (pixel_count > 0) {
            let texture_index = NonUniformResourceIndex(sprite.texture());
            u32 texture_width;
            u32 texture_height;
            textures[texture_index].GetDimensions(texture_width, texture_height);
            let texture_size = f32x2(texture_width, texture_height);
            for (u32 pixel = lane; pixel < pixel_count; pixel += WORKGROUP_SIZE) {
                let coord = u32x2(pixel % source_size.x, pixel / source_size.x);
                let uv = (f32x2(source_position + coord) + 0.5) / texture_size;
                let texel = textures[texture_index].SampleLevel(uv, 0.0);

                sum += f32x4(texel.rgb * texel.a, texel.a);
            }
        }

        partial[lane] = sum;
        GroupMemoryBarrierWithGroupSync();

        for (u32 width = WORKGROUP_SIZE / 2; width > 0; width /= 2) {
            if (lane < width) {
                partial[lane] += partial[lane + width];
            }
            GroupMemoryBarrierWithGroupSync();
        }

        if (lane == 0) {
            let total = partial[0];
            let color = total.a > 0.0 ? f32x4(total.rgb / total.a, 1.0) : f32x4(1.0);
            sprites[sprite_index].packed_color = pack_unorm4x8(color);
        }
        GroupMemoryBarrierWithGroupSync();
    }
}
