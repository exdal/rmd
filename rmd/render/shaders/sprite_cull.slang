import common;
import spec;
import sprite_shared;

// changing this requires everything below to be rewritten
static const u32 WORKGROUP_SIZE = 64;
static const u32 SCAN_SIZE = 256;

// per list
struct CullChunk {
    u32 count[SPRITE_LIST_COUNT];
    u32 underlay_count[SPRITE_LIST_COUNT];
    u32 offset[SPRITE_LIST_COUNT];
    u32 underlay_offset[SPRITE_LIST_COUNT];
};

[[vk::binding(1, 0)]]
StructuredBuffer<Sprite> sprites;

[[vk::binding(2, 0)]]
RWStructuredBuffer<u32> visible_sprite_indices;

[[vk::binding(3, 0)]]
RWStructuredBuffer<DrawIndirectCommand> draw_commands;

[[vk::binding(4, 0)]]
RWStructuredBuffer<CullChunk> chunks;

// per subgroup, one byte per list
groupshared u32 subgroup_visible[WORKGROUP_SIZE];
groupshared u32 subgroup_underlay[WORKGROUP_SIZE];
groupshared u32 scan_visible[SPRITE_LIST_COUNT * SCAN_SIZE];
groupshared u32 scan_underlay[SPRITE_LIST_COUNT * SCAN_SIZE];

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
    let half_size = position_size.zw * 0.5;
    let centre = position_size.xy + half_size;
    let transform = abs(sprite.transform());
    let extent = f32x2(
        transform.x * half_size.x + transform.y * half_size.y,
        transform.z * half_size.x + transform.w * half_size.y
    );
    let sprite_min = centre - extent;
    let sprite_max = centre + extent;
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

func list_count(u32 packed, u32 list) -> u32 {
    return (packed >> (8u * list)) & 0xffu;
}

struct ChunkScan {
    u32 visible;
    u32 underlay;
    u32 total_visible;
    u32 total_underlay;
};

// this could have totally be done in a for loop, but we scalarize
func scan_chunk(u32 group_thread_id, bool visible, bool underlay, u32 list) -> ChunkScan {
    var prefix_visible = 0u;
    var prefix_underlay = 0u;
    var subgroup_total_visible = 0u;
    var subgroup_total_underlay = 0u;
    for (u32 index = 0; index < SPRITE_LIST_COUNT; index++) {
        let in_list = visible && list == index;
        let in_underlay = in_list && underlay;
        let shift = 8u * index;

        prefix_visible += (WavePrefixCountBits(in_list) + u32(in_list)) << shift;
        prefix_underlay += (WavePrefixCountBits(in_underlay) + u32(in_underlay)) << shift;
        subgroup_total_visible += WaveActiveCountBits(in_list) << shift;
        subgroup_total_underlay += WaveActiveCountBits(in_underlay) << shift;
    }

    let subgroup_size = WaveGetLaneCount();
    let subgroup = group_thread_id / subgroup_size;
    if (WaveIsFirstLane()) {
        subgroup_visible[subgroup] = subgroup_total_visible;
        subgroup_underlay[subgroup] = subgroup_total_underlay;
    }

    GroupMemoryBarrierWithGroupSync();

    ChunkScan scan;
    scan.visible = prefix_visible;
    scan.underlay = prefix_underlay;
    scan.total_visible = 0;
    scan.total_underlay = 0;
    for (u32 index = 0; index < (WORKGROUP_SIZE + subgroup_size - 1) / subgroup_size; index++) {
        if (index < subgroup) {
            scan.visible += subgroup_visible[index];
            scan.underlay += subgroup_underlay[index];
        }

        scan.total_visible += subgroup_visible[index];
        scan.total_underlay += subgroup_underlay[index];
    }

    return scan;
}

[numthreads(WORKGROUP_SIZE, 1, 1)]
[shader("compute")]
func cs_classify(
    u32 thread_id : SV_DispatchThreadID,
    u32 group_id : SV_GroupID,
    u32 group_thread_id : SV_GroupThreadID,
    uniform f32x2 center,
    uniform f32x2 viewport,
    uniform u32x2 focused_area_owner,
    uniform f32 zoom,
    uniform u32 first_sprite,
    uniform u32 sprite_count,
    uniform u32 chunk_count,
    uniform u32 active_first_relative,
    uniform u32 list_capacity
) {
    var visible = false;
    var list = SPRITE_LIST_SCENE;
    if (thread_id < sprite_count) {
        let sprite = sprites[first_sprite + thread_id];
        visible = sprite_is_visible(sprite, center, viewport, zoom, focused_area_owner);
        list = sprite_list(sprite.flags());
    }

    let scan = scan_chunk(group_thread_id, visible, thread_id < active_first_relative, list);

    if (group_thread_id == 0 && group_id < chunk_count) {
        for (u32 index = 0; index < SPRITE_LIST_COUNT; index++) {
            chunks[group_id].count[index] = list_count(scan.total_visible, index);
            chunks[group_id].underlay_count[index] = list_count(scan.total_underlay, index);
        }
    }
}

[numthreads(SCAN_SIZE, 1, 1)]
[shader("compute")]
func cs_scan(
    u32 group_thread_id : SV_GroupThreadID,
    uniform f32x2 center,
    uniform f32x2 viewport,
    uniform u32x2 focused_area_owner,
    uniform f32 zoom,
    uniform u32 first_sprite,
    uniform u32 sprite_count,
    uniform u32 chunk_count,
    uniform u32 active_first_relative,
    uniform u32 list_capacity
) {
    let block = (chunk_count + SCAN_SIZE - 1) / SCAN_SIZE;
    let start = min(group_thread_id * block, chunk_count);
    let end = min(start + block, chunk_count);

    u32 visible[SPRITE_LIST_COUNT];
    u32 underlay[SPRITE_LIST_COUNT];
    for (u32 list = 0; list < SPRITE_LIST_COUNT; list++) {
        visible[list] = 0;
        underlay[list] = 0;

        for (u32 index = start; index < end; index++) {
            visible[list] += chunks[index].count[list];
            underlay[list] += chunks[index].underlay_count[list];
        }

        scan_visible[list * SCAN_SIZE + group_thread_id] = visible[list];
        scan_underlay[list * SCAN_SIZE + group_thread_id] = underlay[list];
    }

    GroupMemoryBarrierWithGroupSync();

    for (u32 width = 1; width < SCAN_SIZE; width *= 2) {
        u32 carry_visible[SPRITE_LIST_COUNT];
        u32 carry_underlay[SPRITE_LIST_COUNT];

        for (u32 list = 0; list < SPRITE_LIST_COUNT; list++) {
            let at = list * SCAN_SIZE + group_thread_id;
            carry_visible[list] = group_thread_id >= width ? scan_visible[at - width] : 0;
            carry_underlay[list] = group_thread_id >= width ? scan_underlay[at - width] : 0;
        }

        GroupMemoryBarrierWithGroupSync();

        if (group_thread_id >= width) {
            for (u32 list = 0; list < SPRITE_LIST_COUNT; list++) {
                let at = list * SCAN_SIZE + group_thread_id;
                scan_visible[at] += carry_visible[list];
                scan_underlay[at] += carry_underlay[list];
            }
        }

        GroupMemoryBarrierWithGroupSync();
    }

    u32 running_underlay[SPRITE_LIST_COUNT];
    u32 running_active[SPRITE_LIST_COUNT];
    for (u32 list = 0; list < SPRITE_LIST_COUNT; list++) {
        let at = list * SCAN_SIZE + group_thread_id;
        running_underlay[list] = scan_underlay[at] - underlay[list];
        running_active[list] = scan_visible[at] - visible[list] - running_underlay[list];
    }

    for (u32 index = start; index < end; index++) {
        let chunk = chunks[index];
        for (u32 list = 0; list < SPRITE_LIST_COUNT; list++) {
            chunks[index].underlay_offset[list] = running_underlay[list];
            chunks[index].offset[list] = running_active[list];
            running_underlay[list] += chunk.underlay_count[list];
            running_active[list] += chunk.count[list] - chunk.underlay_count[list];
        }
    }

    // ONLY THIS THREAD WRITES INDIRECT COMMANDS!!!!!!!!!!!!
    if (group_thread_id == 0) {
        for (u32 list = 0; list < SPRITE_LIST_COUNT; list++) {
            let last = list * SCAN_SIZE + SCAN_SIZE - 1;
            let visible_total = scan_visible[last];
            let visible_underlay = scan_underlay[last];

            DrawIndirectCommand underlays;
            underlays.vertex_count = 4;
            underlays.instance_count = visible_underlay;
            underlays.first_vertex = 0;
            underlays.first_instance = 0;
            draw_commands[list * 2] = underlays;

            DrawIndirectCommand active;
            active.vertex_count = 4;
            active.instance_count = visible_total - visible_underlay;
            active.first_vertex = 0;
            active.first_instance = 0;
            draw_commands[list * 2 + 1] = active;
        }
    }
}

[numthreads(WORKGROUP_SIZE, 1, 1)]
[shader("compute")]
func cs_compact(
    u32 thread_id : SV_DispatchThreadID,
    u32 group_id : SV_GroupID,
    u32 group_thread_id : SV_GroupThreadID,
    uniform f32x2 center,
    uniform f32x2 viewport,
    uniform u32x2 focused_area_owner,
    uniform f32 zoom,
    uniform u32 first_sprite,
    uniform u32 sprite_count,
    uniform u32 chunk_count,
    uniform u32 active_first_relative,
    uniform u32 list_capacity
) {
    var visible = false;
    var list = SPRITE_LIST_SCENE;
    if (thread_id < sprite_count) {
        let sprite = sprites[first_sprite + thread_id];
        visible = sprite_is_visible(sprite, center, viewport, zoom, focused_area_owner);
        list = sprite_list(sprite.flags());
    }

    let underlay = thread_id < active_first_relative;
    let scan = scan_chunk(group_thread_id, visible, underlay, list);

    if (!visible || group_id >= chunk_count) {
        return;
    }

    let region = list * list_capacity;
    let chunk = chunks[group_id];
    let underlay_rank = list_count(scan.underlay, list);
    if (underlay) {
        visible_sprite_indices[region + chunk.underlay_offset[list] + underlay_rank - 1] = first_sprite + thread_id;
    } else {
        let rank = list_count(scan.visible, list) - underlay_rank;
        visible_sprite_indices[region + active_first_relative + chunk.offset[list] + rank - 1] =
            first_sprite + thread_id;
    }
}
