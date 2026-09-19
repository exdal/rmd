import common;
import spec;
import sprite_shared;

[[vk::binding(0, 1)]]
Sampler2D textures[];

[[vk::binding(1, 0)]]
StructuredBuffer<Sprite> sprites;

[[vk::binding(2, 0)]]
StructuredBuffer<u32> visible_sprite_indices;

[[vk::binding(3, 0)]]
Texture2D<u32> rendered_levels;

struct VSOutput {
    f32x4 position : SV_Position;
    f32x2 uv;
    f32x2 local;
    nointerpolation u32 sprite_id;
    nointerpolation f32 focus_brightness;
    nointerpolation f32 placement_flash_strength;
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
    uniform u32 base,
    uniform u32x2 focused_area_owner,
    uniform u32x2 placement_flash_owner,
    uniform f32 placement_flash_strength
) -> VSOutput {
    let slot = instance_id + base;
    let sprite_index = SPRITE_INDIRECT ? visible_sprite_indices[slot] : slot;
    let sprite = sprites[sprite_index];
    let corner = CORNERS[vertex_id & 3];
    let position_size = sprite.position_size();
    let flags = sprite.flags();

    let map = position_size.xy + corner * position_size.zw;
    let view = (map - center) * zoom;
    let clip = view / (viewport * 0.5);

    VSOutput output;
    output.position = f32x4(clip.x, -clip.y, 0.0, 1.0);
    let local_uv = f32x2(corner.x, 1.0 - corner.y);
    output.uv = f32x2(sprite.source_position()) + local_uv * f32x2(sprite.source_size());
    output.local = corner;
    let is_area = (flags & SPRITE_FLAG_AREA) != 0;
    let is_area_outline = (flags & SPRITE_AREA_EDGE_MASK) != 0;
    let focus_active = any(focused_area_owner != u32x2(0));
    let inside_focus = all(sprite.area_owner == focused_area_owner);
    let hidden = (is_area_outline && (focus_active ? !inside_focus : !SHOW_AREA_OUTLINES)) ||
        (is_area && !is_area_outline && !SHOW_AREAS);
    output.sprite_id = hidden ? 0 : sprite_index + 1;
    output.focus_brightness = focus_active && !inside_focus ? 0.33 : 1.0;
    output.placement_flash_strength = all(sprite.owner == placement_flash_owner)
        ? saturate(placement_flash_strength)
        : 0.0;

    return output;
}

func apply_placement_flash(f32x4 color, f32 strength) -> f32x4 {
    let green = f32x4(0.0, color.a, 0.0, color.a);

    return lerp(color, green, strength);
}

func apply_focus_brightness(f32x4 color, f32 brightness) -> f32x4 {
    return f32x4(color.rgb * brightness, color.a);
}

func sample_sprite(VSOutput input) -> f32x4 {
    if (input.sprite_id == 0) {
        discard;
    }

    let sprite = sprites[input.sprite_id - 1];
    let flags = sprite.flags();
    let color = sprite.color();
    if ((flags & SPRITE_AREA_EDGE_MASK) != 0) {
        let pixel = fwidth(input.local);
        let on_outline =
            ((flags & SPRITE_AREA_EDGE_NORTH) != 0 && input.local.y >= 1.0 - pixel.y) ||
            ((flags & SPRITE_AREA_EDGE_EAST) != 0 && input.local.x >= 1.0 - pixel.x) ||
            ((flags & SPRITE_AREA_EDGE_SOUTH) != 0 && input.local.y <= pixel.y) ||
            ((flags & SPRITE_AREA_EDGE_WEST) != 0 && input.local.x <= pixel.x);
        if (!on_outline) {
            discard;
        }

        return apply_focus_brightness(
            apply_placement_flash(color, input.placement_flash_strength),
            input.focus_brightness
        );
    }

    let texture_index = NonUniformResourceIndex(sprite.texture());
    u32 texture_width;
    u32 texture_height;
    textures[texture_index].GetDimensions(texture_width, texture_height);
    let uv = input.uv / f32x2(texture_width, texture_height);
    let texel = textures[texture_index].Sample(uv);
    let shaded = f32x4(texel.rgb * color.rgb * texel.a, texel.a * color.a);

    if (shaded.a <= 0.0) {
        discard;
    }

    return apply_focus_brightness(
        apply_placement_flash(shaded, input.placement_flash_strength),
        input.focus_brightness
    );
}

[shader("fragment")]
func fs_main(VSOutput input) -> f32x4 {
    if (SPRITE_EDGE_ONLY) {
        if (input.sprite_id == 0) {
            discard;
        }

        let flags = sprites[input.sprite_id - 1].flags();
        if ((flags & SPRITE_AREA_EDGE_MASK) == 0) {
            discard;
        }
    }

    if (input.sprite_id != 0) {
        let flags = sprites[input.sprite_id - 1].flags();
        if ((flags & (SPRITE_FLAG_EMISSIVE_MASK | SPRITE_FLAG_EMISSIVE_BLOCKER |
                      SPRITE_FLAG_OVERLAY_LIGHT | SPRITE_FLAG_OVERLAY_LIGHT_SUBTRACT)) != 0) {
            discard;
        }
    }

    return sample_sprite(input);
}

struct VisibilityOutput {
    f32x4 color : SV_Target0;
    f32x4 emissive : SV_Target1;
    u32 visibility : SV_Target2;
    u32 level : SV_Target3;
};

[shader("fragment")]
func fs_visibility(VSOutput input) -> VisibilityOutput {
    if (input.sprite_id == 0) {
        discard;
    }

    let flags = sprites[input.sprite_id - 1].flags();
    if ((flags & SPRITE_AREA_EDGE_MASK) != 0) {
        discard;
    }

    if ((flags & (SPRITE_FLAG_OVERLAY_LIGHT | SPRITE_FLAG_OVERLAY_LIGHT_SUBTRACT)) != 0) {
        discard;
    }

    VisibilityOutput output;
    let color = sample_sprite(input);
    let mask = (flags & SPRITE_FLAG_EMISSIVE_MASK) != 0;
    let blocker = (flags & SPRITE_FLAG_EMISSIVE_BLOCKER) != 0;
    output.color = mask || blocker ? f32x4(0.0) : color;
    output.emissive = (flags & (SPRITE_FLAG_EMISSIVE_MASK | SPRITE_FLAG_FULLBRIGHT)) != 0
        ? f32x4(color.a)
        : f32x4(0.0, 0.0, 0.0, color.a);
    output.visibility = SPRITE_WRITES_ID ? input.sprite_id : 0;
    output.level = sprites[input.sprite_id - 1].z;

    return output;
}

[shader("fragment")]
func fs_overlay_light(VSOutput input) -> f32x4 {
    if (input.sprite_id == 0) {
        discard;
    }

    let flags = sprites[input.sprite_id - 1].flags();
    let add = (flags & SPRITE_FLAG_OVERLAY_LIGHT) != 0;
    let subtract = (flags & SPRITE_FLAG_OVERLAY_LIGHT_SUBTRACT) != 0;
    if (!add && !subtract) {
        discard;
    }
    if (sprites[input.sprite_id - 1].z != rendered_levels.Load(i32x3(i32x2(input.position.xy), 0))) {
        discard;
    }

    let color = sample_sprite(input);
    return f32x4(color.rgb * (subtract ? -1.0 : 1.0), color.a);
}
