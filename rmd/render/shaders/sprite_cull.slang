import common;
import spec;
import sprite_shared;

static const u32 WORKGROUP_SIZE = 64;
static const u32 SCAN_SIZE = 256;

struct CullChunk {
    u32 count;
    u32 underlay_count;
    u32 offset;
    u32 underlay_offset;
};

[[vk::binding(1, 0)]]
StructuredBuffer<Sprite> sprites;

[[vk::binding(2, 0)]]
RWStructuredBuffer<u32> visible_sprite_indices;

[[vk::binding(3, 0)]]
RWStructuredBuffer<DrawIndirectCommand> draw_commands;

[[vk::binding(4, 0)]]
RWStructuredBuffer<CullChunk> chunks;

groupshared u32 chunk_visible[WORKGROUP_SIZE];
groupshared u32 chunk_underlay[WORKGROUP_SIZE];
groupshared u32 scan_visible[SCAN_SIZE];
groupshared u32 scan_underlay[SCAN_SIZE];

func sprite_is_visible(
    Sprite sprite,
    f32x2 center,
    f32x2 viewport,
    f32 zoom,
    u32x2 focused_area_owner
) -> bool {
    let half_extent = viewport * 0.5 / max(abs(zoom), 0.000001);
    let view_min = center - half_extent;
    let view_max = center + half_extent;
    let position_size = sprite.position_size();
    let sprite_min = position_size.xy;
    let sprite_max = position_size.xy + position_size.zw;
    let intersects = !(
        sprite_max.x < view_min.x || sprite_min.x > view_max.x ||
        sprite_max.y < view_min.y || sprite_min.y > view_max.y
    );

    let flags = sprite.flags();
    let is_area = (flags & SPRITE_FLAG_AREA) != 0;
    let is_area_outline = (flags & SPRITE_AREA_EDGE_MASK) != 0;
    let focus_active = any(focused_area_owner != u32x2(0));
    let inside_focus = all(sprite.area_owner == focused_area_owner);
    let hidden = (is_area_outline && (focus_active ? !inside_focus : !SHOW_AREA_OUTLINES)) ||
        (is_area && !is_area_outline && !SHOW_AREAS);

    return intersects && !hidden;
}

func scan_chunk(u32 lane, bool visible, bool underlay) {
    chunk_visible[lane] = visible ? 1 : 0;
    chunk_underlay[lane] = visible && underlay ? 1 : 0;
    GroupMemoryBarrierWithGroupSync();

    for (u32 width = 1; width < WORKGROUP_SIZE; width *= 2) {
        let carry_visible = lane >= width ? chunk_visible[lane - width] : 0;
        let carry_underlay = lane >= width ? chunk_underlay[lane - width] : 0;
        GroupMemoryBarrierWithGroupSync();
        if (lane >= width) {
            chunk_visible[lane] += carry_visible;
            chunk_underlay[lane] += carry_underlay;
        }
        GroupMemoryBarrierWithGroupSync();
    }
}

[numthreads(WORKGROUP_SIZE, 1, 1)]
[shader("compute")]
func cs_classify(
    u32x3 group : SV_GroupID,
    u32x3 thread : SV_GroupThreadID,
    uniform f32x2 center,
    uniform f32x2 viewport,
    uniform u32x2 focused_area_owner,
    uniform f32 zoom,
    uniform u32 first_sprite,
    uniform u32 sprite_count,
    uniform u32 chunk_count,
    uniform u32 active_first_relative
) {
    let lane = thread.x;
    let relative_index = group.x * WORKGROUP_SIZE + lane;

    var visible = false;
    if (relative_index < sprite_count) {
        visible = sprite_is_visible(
            sprites[first_sprite + relative_index],
            center,
            viewport,
            zoom,
            focused_area_owner
        );
    }

    scan_chunk(lane, visible, relative_index < active_first_relative);

    if (lane == 0 && group.x < chunk_count) {
        chunks[group.x].count = chunk_visible[WORKGROUP_SIZE - 1];
        chunks[group.x].underlay_count = chunk_underlay[WORKGROUP_SIZE - 1];
    }
}

[numthreads(SCAN_SIZE, 1, 1)]
[shader("compute")]
func cs_scan(
    u32x3 thread : SV_GroupThreadID,
    uniform f32x2 center,
    uniform f32x2 viewport,
    uniform u32x2 focused_area_owner,
    uniform f32 zoom,
    uniform u32 first_sprite,
    uniform u32 sprite_count,
    uniform u32 chunk_count,
    uniform u32 active_first_relative
) {
    let lane = thread.x;
    let block = (chunk_count + SCAN_SIZE - 1) / SCAN_SIZE;
    let start = min(lane * block, chunk_count);
    let end = min(start + block, chunk_count);

    var visible = 0u;
    var underlay = 0u;
    for (u32 index = start; index < end; index++) {
        visible += chunks[index].count;
        underlay += chunks[index].underlay_count;
    }

    scan_visible[lane] = visible;
    scan_underlay[lane] = underlay;
    GroupMemoryBarrierWithGroupSync();

    for (u32 width = 1; width < SCAN_SIZE; width *= 2) {
        let carry_visible = lane >= width ? scan_visible[lane - width] : 0;
        let carry_underlay = lane >= width ? scan_underlay[lane - width] : 0;
        GroupMemoryBarrierWithGroupSync();
        if (lane >= width) {
            scan_visible[lane] += carry_visible;
            scan_underlay[lane] += carry_underlay;
        }
        GroupMemoryBarrierWithGroupSync();
    }

    var running_underlay = scan_underlay[lane] - underlay;
    var running_active = scan_visible[lane] - visible - running_underlay;
    for (u32 index = start; index < end; index++) {
        let chunk = chunks[index];
        chunks[index].underlay_offset = running_underlay;
        chunks[index].offset = running_active;
        running_underlay += chunk.underlay_count;
        running_active += chunk.count - chunk.underlay_count;
    }

    if (lane == 0) {
        let visible_total = scan_visible[SCAN_SIZE - 1];
        let visible_underlay = scan_underlay[SCAN_SIZE - 1];

        DrawIndirectCommand underlays;
        underlays.vertex_count = 4;
        underlays.instance_count = visible_underlay;
        underlays.first_vertex = 0;
        underlays.first_instance = 0;
        draw_commands[0] = underlays;

        DrawIndirectCommand active;
        active.vertex_count = 4;
        active.instance_count = visible_total - visible_underlay;
        active.first_vertex = 0;
        active.first_instance = 0;
        draw_commands[1] = active;
    }
}

[numthreads(WORKGROUP_SIZE, 1, 1)]
[shader("compute")]
func cs_compact(
    u32x3 group : SV_GroupID,
    u32x3 thread : SV_GroupThreadID,
    uniform f32x2 center,
    uniform f32x2 viewport,
    uniform u32x2 focused_area_owner,
    uniform f32 zoom,
    uniform u32 first_sprite,
    uniform u32 sprite_count,
    uniform u32 chunk_count,
    uniform u32 active_first_relative
) {
    let lane = thread.x;
    let relative_index = group.x * WORKGROUP_SIZE + lane;

    var visible = false;
    if (relative_index < sprite_count) {
        visible = sprite_is_visible(
            sprites[first_sprite + relative_index],
            center,
            viewport,
            zoom,
            focused_area_owner
        );
    }

    let underlay = relative_index < active_first_relative;
    scan_chunk(lane, visible, underlay);

    if (!visible || group.x >= chunk_count) {
        return;
    }

    let chunk = chunks[group.x];
    if (underlay) {
        visible_sprite_indices[chunk.underlay_offset + chunk_underlay[lane] - 1] = first_sprite + relative_index;
    } else {
        let rank = chunk_visible[lane] - chunk_underlay[lane];
        visible_sprite_indices[active_first_relative + chunk.offset + rank - 1] = first_sprite + relative_index;
    }
}
