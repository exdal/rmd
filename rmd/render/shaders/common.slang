module common;

__include common.types;
__include common.color;
__include common.encoding;

public static const f32 LEVEL_DEPTH_SCALE = 65535.0;

public struct DrawIndirectCommand {
    public u32 vertex_count;
    public u32 instance_count;
    public u32 first_vertex;
    public u32 first_instance;
};
