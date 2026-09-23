import common;
import sprite_shared;

struct GuideLine {
    f32x2 origin;
    f32x2 target;
};

[[vk::binding(0, 0)]]
StructuredBuffer<GuideLine> guide_lines;

[[vk::binding(1, 0)]]
Texture2D<u32> visibility;

[[vk::binding(2, 0)]]
StructuredBuffer<Sprite> sprites;

struct VSOutput {
    f32x4 position : SV_Position;
    nointerpolation u32 guide_id;
};

static const f32x2 CORNERS[4] = {
    f32x2(0.0, -1.0),
    f32x2(0.0, 1.0),
    f32x2(1.0, -1.0),
    f32x2(1.0, 1.0),
};

[shader("vertex")]
func vs_main(
    u32 vertex_id : SV_VertexID,
    u32 instance_id : SV_InstanceID,
    uniform f32x2 viewport,
    uniform u32x2 selected_owner,
    uniform f32 stripe_offset
) -> VSOutput {
    let guide = guide_lines[instance_id];
    let delta = guide.target - guide.origin;
    let length = max(distance(guide.origin, guide.target), 0.0001);
    let tangent = delta / length;
    let normal = f32x2(-tangent.y, tangent.x);
    let corner = CORNERS[vertex_id & 3];
    let along = lerp(guide.origin - tangent * 6.0, guide.target + tangent * 2.0, corner.x);
    let screen = along + normal * corner.y * 6.0;

    VSOutput output;
    output.position = f32x4(screen / viewport * 2.0 - 1.0, 0.0, 1.0);
    output.guide_id = instance_id;
    return output;
}

func owner_at(i32x2 pixel) -> u32x2 {
    u32 width;
    u32 height;
    visibility.GetDimensions(width, height);
    if (pixel.x < 0 || pixel.y < 0 || pixel.x >= i32(width) || pixel.y >= i32(height)) {
        return u32x2(0);
    }
    let visibility_id = visibility.Load(i32x3(pixel, 0));
    return visibility_id == 0 ? u32x2(0) : sprites[visibility_id - 1].owner;
}

func guide_color(f32 path_position, f32 stripe_offset) -> f32x4 {
    let stripe_position = path_position - stripe_offset;
    let wrapped = stripe_position - floor(stripe_position / 12.0) * 12.0;
    return wrapped < 6.0 ? f32x4(1.0) : f32x4(1.0, 0.5, 0.0, 1.0);
}

[shader("fragment")]
func fs_main(
    VSOutput input,
    uniform f32x2 viewport,
    uniform u32x2 selected_owner,
    uniform f32 stripe_offset
) -> f32x4 {
    if (any(selected_owner != u32x2(0)) && all(owner_at(i32x2(input.position.xy)) == selected_owner)) {
        discard;
    }

    let guide = guide_lines[input.guide_id];
    let fragment = input.position.xy;
    let segment = guide.target - guide.origin;
    let segment_length_squared = dot(segment, segment);
    if (segment_length_squared <= 0.0001) {
        discard;
    }

    if (distance(fragment, guide.origin) <= 6.0) {
        return f32x4(1.0, 0.5, 0.0, 1.0);
    }

    let along = saturate(dot(fragment - guide.origin, segment) / segment_length_squared);
    let nearest = guide.origin + segment * along;
    if (distance(fragment, nearest) > 1.5) {
        discard;
    }

    return guide_color(along * sqrt(segment_length_squared), stripe_offset);
}
