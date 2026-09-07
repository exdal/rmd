import common;
import sprite_shared;

[[vk::binding(0, 0)]]
Sampler2D scene;

[[vk::binding(1, 0)]]
Texture2D<u32> visibility;

[[vk::binding(2, 0)]]
StructuredBuffer<Sprite> sprites;

[[vk::binding(3, 0)]]
RWStructuredBuffer<u32> pick_result;

struct VSOutput {
    f32x4 position : SV_Position;
    f32x2 uv;
};

[shader("vertex")]
func vs_main(u32 vertex_id : SV_VertexID) -> VSOutput {
    let uv = f32x2((vertex_id << 1) & 2, vertex_id & 2);

    VSOutput output;
    output.position = f32x4(uv * 2.0 - 1.0, 0.0, 1.0);
    output.uv = uv;
    return output;
}

func same_owner(u32x2 left, u32x2 right) -> bool {
    return all(left == right);
}

func owner_for(u32 visibility_id) -> u32x2 {
    return visibility_id == 0 ? u32x2(0) : sprites[visibility_id - 1].owner;
}

[shader("fragment")]
func fs_main(
    VSOutput input,
    uniform u32x2 cursor,
    uniform u32 cursor_valid,
    uniform u32x2 selected_owner
) -> f32x4 {
    var color = scene.Sample(input.uv);
    let pixel = u32x2(input.position.xy);
    let owner = owner_for(visibility.Load(i32x3(i32x2(pixel), 0)));
    let selected = any(selected_owner != u32x2(0)) && same_owner(owner, selected_owner);

    var hovered = false;
    if (cursor_valid != 0) {
        let hovered_owner = owner_for(visibility.Load(i32x3(i32x2(cursor), 0)));
        hovered = any(hovered_owner != u32x2(0)) && same_owner(owner, hovered_owner);
    }

    if (selected || hovered) {
        color.rgb = lerp(color.rgb, f32x3(0.0, color.a, 0.0), 0.35);
    }

    return color;
}

[shader("compute")]
[numthreads(1, 1, 1)]
func cs_pick(
    uniform u32x2 cursor,
    uniform u32 cursor_valid
) {
    pick_result[0] = cursor_valid == 0 ? 0 : visibility.Load(i32x3(i32x2(cursor), 0));
}
