import common;

[[vk::binding(0, 0)]]
Texture2D<f32x4> scene;

struct LightTile {
    u32 southwest;
    u32 southeast;
    u32 northwest;
    u32 northeast;
};

[[vk::binding(1, 0)]]
StructuredBuffer<LightTile> light_tiles;

[[vk::binding(2, 0)]]
Texture2D<f32> emissive_mask;

[[vk::binding(3, 0)]]
Texture2D<f32x4> overlay_lightmap;

[[vk::binding(4, 0)]]
Texture2D<f32> rendered_levels;

struct VSOutput {
    f32x4 position : SV_Position;
    f32x2 uv;
};

[shader("vertex")]
func vs_main(u32 vertex_id: SV_VertexID) -> VSOutput {
    let uv = f32x2((vertex_id << 1) & 2, vertex_id & 2);

    VSOutput output;
    output.position = f32x4(uv * 2.0 - 1.0, 0.0, 1.0);
    output.uv = uv;
    return output;
}

[shader("fragment")]
func fs_main(
    VSOutput input,
    uniform f32x2 center,
    uniform f32x2 viewport,
    uniform f32 zoom,
    uniform f32 tile_size,
    uniform u32x2 map_size,
    uniform u32 level_count,
    uniform u32 base,
    uniform f32 minimum_brightness
) -> f32x4 {
    let pixel = i32x3(i32x2(input.position.xy), 0);
    let color = scene.Load(pixel);
    let emission = emissive_mask.Load(pixel);
    let overlay_light = overlay_lightmap.Load(pixel).rgb;
    let level = u32(round(rendered_levels.Load(pixel) * LEVEL_DEPTH_SCALE));
    if (zoom <= 0.0 || tile_size <= 0.0 || level == 0 || level > level_count) {
        return color;
    }

    let world = center + f32x2((input.uv.x - 0.5) * viewport.x / zoom,
                                (0.5 - input.uv.y) * viewport.y / zoom);
    let tile = i32x2(floor(world / tile_size));
    if (tile.x < 0 || tile.y < 0 || tile.x >= i32(map_size.x) ||
        tile.y >= i32(map_size.y)) {
        return color;
    }

    let plane = map_size.x * map_size.y;
    let index =
        base + (level - 1) * plane + u32(tile.y) * map_size.x + u32(tile.x);
    let light_tile = light_tiles[index];
    let southwest = unpack_unorm4x8(light_tile.southwest).rgb;
    let southeast = unpack_unorm4x8(light_tile.southeast).rgb;
    let northwest = unpack_unorm4x8(light_tile.northwest).rgb;
    let northeast = unpack_unorm4x8(light_tile.northeast).rgb;
    let local = frac(world / tile_size);
    let south = lerp(southwest, southeast, local.x);
    let north = lerp(northwest, northeast, local.x);
    let light = lerp(south, north, local.y);
    let combined_light = saturate(light + overlay_light);
    let luminance = dot(combined_light, f32x3(0.2126, 0.7152, 0.0722));
    var displayed_light = combined_light;
    if (luminance < minimum_brightness) {
        displayed_light = lerp(combined_light, f32x3(1.0), (minimum_brightness - luminance) / (1.0 - luminance));
    }
    let factor = lerp(displayed_light, f32x3(1.0), emission);
    let straight = color.a > 0.0 ? color.rgb / color.a : f32x3(0.0);
    let lit = srgb_to_linear(linear_to_srgb(straight) * factor) * color.a;

    return f32x4(lit, color.a);
}
