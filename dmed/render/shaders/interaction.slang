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

func owner_at(i32x2 pixel, u32 width, u32 height) -> u32x2 {
    if (pixel.x < 0 || pixel.y < 0 || pixel.x >= i32(width) || pixel.y >= i32(height)) {
        return u32x2(0);
    }

    return owner_for(visibility.Load(i32x3(pixel, 0)));
}

func is_highlighted_owner(u32x2 owner, u32x2 selected_owner, u32x2 hovered_owner) -> bool {
    if (!any(owner != u32x2(0))) {
        return false;
    }

    return (any(selected_owner != u32x2(0)) && same_owner(owner, selected_owner)) ||
        (any(hovered_owner != u32x2(0)) && same_owner(owner, hovered_owner));
}

func marching_color(f32 path_position, f32 stripe_offset, u32 interaction_mode) -> f32x4 {
    let stripe_position = path_position - stripe_offset;
    let wrapped = stripe_position - floor(stripe_position / 12.0) * 12.0;
    let accent = interaction_mode != 0 ? f32x4(1.0, 0.0, 0.0, 1.0) : f32x4(0.0, 1.0, 0.0, 1.0);

    return wrapped < 6.0 ? f32x4(1.0) : accent;
}

func guide_color(f32 path_position, f32 stripe_offset) -> f32x4 {
    let stripe_position = path_position - stripe_offset;
    let wrapped = stripe_position - floor(stripe_position / 12.0) * 12.0;

    return wrapped < 6.0 ? f32x4(1.0) : f32x4(1.0, 0.5, 0.0, 1.0);
}

[shader("fragment")]
func fs_main(
    VSOutput input,
    uniform u32x2 cursor,
    uniform u32 cursor_valid,
    uniform u32x2 selected_owner,
    uniform f32 stripe_offset,
    uniform f32x2 guide_origin,
    uniform f32x2 guide_target,
    uniform u32 guide_valid,
    uniform u32 interaction_mode,
    uniform u32x2 focused_area_owner
) -> f32x4 {
    let color = scene.Sample(input.uv);
    let pixel = i32x2(input.position.xy);
    let fragment = input.position.xy;
    u32 width;
    u32 height;
    visibility.GetDimensions(width, height);

    let guide_active = guide_valid != 0 && any(selected_owner != u32x2(0));
    var on_guide = false;
    var guide_path_position = 0.0;
    if (guide_active) {
        let segment = guide_target - guide_origin;
        let segment_length_squared = dot(segment, segment);
        if (segment_length_squared > 0.0001) {
            let along = saturate(dot(fragment - guide_origin, segment) / segment_length_squared);
            let nearest_guide = guide_origin + segment * along;
            on_guide = distance(fragment, nearest_guide) <= 1.5;
            guide_path_position = along * sqrt(segment_length_squared);
        }
    }

    var hovered_owner = u32x2(0);
    if (cursor_valid != 0) {
        let hovered = visibility.Load(i32x3(i32x2(cursor), 0));
        // a focused area makes every tool inert outside it, so nothing out there highlights
        let reachable = !any(focused_area_owner != u32x2(0)) ||
            (hovered != 0 && all(sprites[hovered - 1].area_owner == focused_area_owner));
        if (reachable) {
            hovered_owner = owner_for(hovered);
        }
    }

    if (!any(selected_owner != u32x2(0)) && !any(hovered_owner != u32x2(0)) && !guide_active) {
        return color;
    }

    let owner = owner_at(pixel, width, height);
    let on_selected = any(selected_owner != u32x2(0)) && same_owner(owner, selected_owner);
    if (on_selected) {
        return color;
    }

    // origin needs to stay under sprite, otherwise it looks annoying
    if (guide_active && distance(fragment, guide_origin) <= 6.0) {
        return f32x4(1.0, 0.5, 0.0, 1.0);
    }

    let on_hovered = any(hovered_owner != u32x2(0)) && same_owner(owner, hovered_owner);
    if (on_hovered) {
        return on_guide ? guide_color(guide_path_position, stripe_offset) : color;
    }

    var nearest = i32x2(0);
    var nearest_distance = 5;
    for (i32 y = -2; y <= 2; y++) {
        for (i32 x = -2; x <= 2; x++) {
            let distance = x * x + y * y;
            if (distance == 0 || distance > 4 || distance >= nearest_distance) {
                continue;
            }

            let neighbor = owner_at(pixel + i32x2(x, y), width, height);
            if (is_highlighted_owner(neighbor, selected_owner, hovered_owner)) {
                nearest = i32x2(x, y);
                nearest_distance = distance;
            }
        }
    }

    if (nearest_distance != 5) {
        var path_position = 0.0;
        if (abs(nearest.y) >= abs(nearest.x)) {
            path_position = nearest.y > 0 ? f32(pixel.x) : -f32(pixel.x);
        }
        else {
            path_position = nearest.x < 0 ? f32(pixel.y) : -f32(pixel.y);
        }

        return marching_color(path_position, stripe_offset, interaction_mode);
    }

    if (on_guide) {
        return guide_color(guide_path_position, stripe_offset);
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
