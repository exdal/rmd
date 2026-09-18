use std::{collections::HashMap, ops::Range, time::Instant};

use ash::vk;
use dear_imgui_rs::render::PendingFrame;
use dmi::IconFile;
use vir::{
    Access,
    AllocatorKind,
    BlendPreset,
    Buffer,
    BufferImageCopy,
    BufferInfo,
    ComputePipelineInfo,
    DRAW_INDIRECT_STRIDE,
    DomainFlag,
    DrawIndirectCommand,
    GraphicsPipelineInfo,
    Image,
    ImageAttachment,
    ImageInfo,
    MemoryLocation,
    Module,
    PassCallback,
    PipelineId,
    Program,
    RasterizationState,
    Rect2D,
    RenderGraph,
    SamplerInfo,
    SuperFrameAllocator,
    SwapChain,
    ValueId,
    allocator::Allocator,
};

use crate::{
    AREA_EDGES_ALL,
    Device,
    Frame,
    GpuError,
    PickResult,
    SpriteInstance,
    SpritePreview,
    VisibilityId,
    extent3d,
    imgui::{ImGuiPass, ImGuiSlots},
    read_spirv,
    spec,
    texture::TextureCatalog,
};

const GEOMETRY_VS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sprite.vert.spv"));
const SPRITE_SHADE_FS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sprite.frag.spv"));
const SPRITE_VIS_FS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sprite_visibility.frag.spv"));
const SPRITE_OVERLAY_LIGHT_FS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sprite_overlay_light.frag.spv"));
const SPRITE_CULL_CLASSIFY_CS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sprite_cull_classify.comp.spv"));
const SPRITE_CULL_SCAN_CS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sprite_cull_scan.comp.spv"));
const SPRITE_CULL_COMPACT_CS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sprite_cull_compact.comp.spv"));
const AREA_COLOR_CS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/area_color.comp.spv"));
const INTERACTION_VS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/interaction.vert.spv"));
const INTERACTION_FS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/interaction.frag.spv"));
const GUIDE_VS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/guide.vert.spv"));
const GUIDE_FS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/guide.frag.spv"));
const LIGHTING_VS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/lighting.vert.spv"));
const LIGHTING_FS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/lighting.frag.spv"));
const PICK_CS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/pick.comp.spv"));
const UPLOAD_BATCH_BYTES: usize = 64 * 1024 * 1024;
const HIGHLIGHT_STRIPE_PERIOD: f32 = 12.0;
const HIGHLIGHT_STRIPE_SPEED: f32 = 12.0;
const CULL_WORKGROUP_SIZE: u32 = 64;
const SPRITE_DRAW_COMMANDS: u64 = 2;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
struct GpuSprite {
    owner: [u32; 2],
    area_owner: [u32; 2],
    position: [f32; 2],
    z: u32,
    packed_size: u32,
    color: u32,
    source_position_size: [u32; 2],
    texture_flags: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GpuLightTile {
    corners: [u32; 4],
}

const SPRITE_FLAG_AREA: u32 = 1;
const SPRITE_AREA_EDGE_SHIFT: u32 = 1;
const SPRITE_FLAG_EMISSIVE_MASK: u32 = 1 << 5;
const SPRITE_FLAG_EMISSIVE_BLOCKER: u32 = 1 << 6;
const SPRITE_FLAG_OVERLAY_LIGHT: u32 = 1 << 7;
const SPRITE_FLAG_OVERLAY_LIGHT_SUBTRACT: u32 = 1 << 8;
const SPRITE_FLAG_FULLBRIGHT: u32 = 1 << 9;
const SPRITE_FLAGS_SHIFT: u32 = 22;
const SPRITE_TEXTURE_MASK: u32 = (1 << SPRITE_FLAGS_SHIFT) - 1;
const SPRITE_TEXTURE_CAPACITY: u32 = SPRITE_TEXTURE_MASK + 1;
const TEXTURE_RESERVE: usize = 8192;

#[repr(C)]
#[derive(Clone, Copy)]
struct AreaColorPush {
    first_sprite: u32,
    sprite_count: u32,
    group_count: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct CameraPush {
    center: [f32; 2],
    viewport: [f32; 2],
    zoom: f32,
    base: u32,
    focused_area_owner: [u32; 2],
    placement_flash_owner: [u32; 2],
    placement_flash_strength: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
struct CullPush {
    center: [f32; 2],
    viewport: [f32; 2],
    focused_area_owner: [u32; 2],
    zoom: f32,
    first_sprite: u32,
    sprite_count: u32,
    chunk_count: u32,
    active_first_relative: u32,
}

#[repr(C)]
struct CullChunk {
    count: u32,
    underlay_count: u32,
    offset: u32,
    underlay_offset: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct InteractionPush {
    cursor: [u32; 2],
    cursor_valid: u32,
    selected_owner: [u32; 2],
    stripe_offset: f32,
    focused_area_owner: [u32; 2],
    connected_count: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct GpuGuideLine {
    origin: [f32; 2],
    target: [f32; 2],
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct GuidePush {
    viewport: [f32; 2],
    selected_owner: [u32; 2],
    stripe_offset: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PickPush {
    cursor: [u32; 2],
    cursor_valid: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct LightingPush {
    center: [f32; 2],
    viewport: [f32; 2],
    zoom: f32,
    tile_size: f32,
    map_size: [u32; 2],
    level_count: u32,
    base: u32,
}

struct LightingSlots {
    push: ValueId,
}

struct InteractionSlots {
    push: ValueId,
    connected_owners: ValueId,
    pick_push: ValueId,
    area_tiles: ValueId,
    hover_draw: ValueId,
    highlight_draw: ValueId,
}

struct CullSlots {
    push: ValueId,
    sprite_count: ValueId,
    visible: ValueId,
    chunks: ValueId,
    commands: ValueId,
}

struct CullValues {
    visible: ValueId,
    chunks: ValueId,
    commands: ValueId,
}

fn record_sprite_cull(
    module: &mut Module, pipelines: [PipelineId; 3], sprites: ValueId, values: CullValues, state: &FrameGraphState,
    name: &str,
) -> (CullValues, CullSlots) {
    let CullValues {
        visible,
        chunks,
        commands,
    } = values;
    let push = module.declare_bytes_var(&format!("{name} push"), size_of::<CullPush>() as u32);
    let sprite_count = module.declare_u32_var(&format!("{name} sprite count"), 0);
    let slots = CullSlots {
        push,
        sprite_count,
        visible,
        chunks,
        commands,
    };

    let [_sprites, chunks] = module
        .begin_compute([(sprites, Access::ComputeRead), (chunks, Access::ComputeWrite)])
        .with_name(format!("{name} classify"))
        .bind_compute_pipeline(pipelines[0])
        .bind_buffer(0, 1, sprites)
        .bind_buffer(0, 4, chunks)
        .push_constants_from(push)
        .specialize_constant(spec::SHOW_AREAS, state.show_areas)
        .specialize_constant(spec::SHOW_AREA_OUTLINES, state.show_area_outlines)
        .dispatch_invocations(sprite_count, 1u32, 1u32)
        .end_compute();

    let [commands, chunks] = module
        .begin_compute([(commands, Access::ComputeWrite), (chunks, Access::ComputeRW)])
        .with_name(format!("{name} scan"))
        .bind_compute_pipeline(pipelines[1])
        .bind_buffer(0, 3, commands)
        .bind_buffer(0, 4, chunks)
        .push_constants_from(push)
        .dispatch(1u32, 1u32, 1u32)
        .end_compute();

    let [visible, _sprites, chunks] = module
        .begin_compute([
            (visible, Access::ComputeWrite),
            (sprites, Access::ComputeRead),
            (chunks, Access::ComputeRead),
        ])
        .with_name(format!("{name} compact"))
        .bind_compute_pipeline(pipelines[2])
        .bind_buffer(0, 1, sprites)
        .bind_buffer(0, 2, visible)
        .bind_buffer(0, 4, chunks)
        .push_constants_from(push)
        .specialize_constant(spec::SHOW_AREAS, state.show_areas)
        .specialize_constant(spec::SHOW_AREA_OUTLINES, state.show_area_outlines)
        .dispatch_invocations(sprite_count, 1u32, 1u32)
        .end_compute();

    (
        CullValues {
            visible,
            chunks,
            commands,
        },
        slots,
    )
}

struct CullBuffers {
    visible: Buffer,
    chunks: Buffer,
    commands: Buffer,
    sprite_capacity: usize,
}

impl CullBuffers {
    fn allocate(device: &mut Device, sprites: usize) -> Result<Self, GpuError> {
        let capacity = sprites
            .max(1024)
            .checked_next_power_of_two()
            .ok_or(GpuError::SpriteUploadTooLarge)?;
        let indices = capacity
            .checked_mul(size_of::<u32>())
            .and_then(|size| u64::try_from(size).ok())
            .ok_or(GpuError::SpriteUploadTooLarge)?;
        let chunk_count = u32::try_from(capacity)
            .map_err(|_| GpuError::SpriteUploadTooLarge)?
            .div_ceil(CULL_WORKGROUP_SIZE)
            .max(1);

        let visible = device.allocator.allocate_buffer(
            &BufferInfo::new(indices, vk::BufferUsageFlags::STORAGE_BUFFER, MemoryLocation::GpuOnly)
                .with_name("visible sprite indices"),
        )?;
        let chunks = match device.allocator.allocate_buffer(
            &BufferInfo::new(
                u64::from(chunk_count) * size_of::<CullChunk>() as u64,
                vk::BufferUsageFlags::STORAGE_BUFFER,
                MemoryLocation::GpuOnly,
            )
            .with_name("cull chunks"),
        ) {
            Ok(buffer) => buffer,
            Err(error) => {
                device.allocator.deallocate_buffer(visible);

                return Err(error.into());
            },
        };
        let commands = match device.allocator.allocate_buffer(
            &BufferInfo::new(
                SPRITE_DRAW_COMMANDS * size_of::<DrawIndirectCommand>() as u64,
                vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::INDIRECT_BUFFER,
                MemoryLocation::GpuOnly,
            )
            .with_name("sprite draw commands"),
        ) {
            Ok(buffer) => buffer,
            Err(error) => {
                device.allocator.deallocate_buffer(chunks);
                device.allocator.deallocate_buffer(visible);

                return Err(error.into());
            },
        };

        Ok(Self {
            visible,
            chunks,
            commands,
            sprite_capacity: capacity,
        })
    }

    fn destroy(self, device: &mut Device) {
        device.allocator.deallocate_buffer(self.visible);
        device.allocator.deallocate_buffer(self.chunks);
        device.allocator.deallocate_buffer(self.commands);
    }
}

/// The spec constants and layout a recording is built for, so a change to one re-records.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FrameGraphState {
    extent: [u32; 2],
    with_imgui: bool,
    show_areas: bool,
    show_area_outlines: bool,
    map_views: Vec<MapViewState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MapViewState {
    rect: crate::MapViewRect,
    highlight_tint: bool,
    delete_mode: bool,
    preview: bool,
    lighting: bool,
}

impl FrameGraphState {
    fn new(viewport: vk::Extent2D, with_imgui: bool, frame: &Frame<'_>) -> Self {
        Self {
            extent: [viewport.width, viewport.height],
            with_imgui,
            show_areas: frame.show_areas,
            show_area_outlines: frame.show_area_outlines,
            map_views: frame
                .map_views
                .iter()
                .filter(|view| !view.rect.is_empty())
                .map(|view| MapViewState {
                    rect: view.rect,
                    highlight_tint: view.interaction.highlight.is_tint(),
                    delete_mode: view.interaction.mode.is_delete(),
                    preview: view.preview.is_some_and(|preview| !preview.sprites.is_empty()),
                    lighting: view.lighting.is_some_and(|lighting| !lighting.tiles.is_empty()),
                })
                .collect(),
        }
    }
}

struct Recorded {
    program: Program,
    sprites: ValueId,
    lights: Option<ValueId>,
    map_views: Vec<MapViewPass>,
    pick_result: Option<ValueId>,
    state: FrameGraphState,
    ui: Option<ImGuiSlots>,
}

struct MapViewPass {
    scene_attachment: ValueId,
    visibility_attachment: ValueId,
    output_attachment: ValueId,
    extent: ValueId,
    camera: ValueId,
    underlay_camera: ValueId,
    active_camera: ValueId,
    cull: CullSlots,
    lighting: Option<LightingSlots>,
    preview: Option<PreviewSlots>,
    should_pick: Option<ValueId>,
    interaction: Option<InteractionSlots>,
    guides: Option<GuideSlots>,
}

struct GuideSlots {
    lines: ValueId,
    push: ValueId,
    draw: ValueId,
}

struct AreaColorPass {
    program: Program,
    sprites: ValueId,
    push: ValueId,
    groups: ValueId,
}

struct PreviewSlots {
    sprites: ValueId,
    camera: ValueId,
    cull: CullSlots,
}

impl AreaColorPass {
    fn record(graph: &RenderGraph, pipeline: PipelineId) -> Result<Self, GpuError> {
        let mut module = Module::default();
        let sprites = module.declare_buffer_var("area color sprites", Access::HostWrite);
        let push = module.declare_bytes_var("area color push", size_of::<AreaColorPush>() as u32);
        let groups = module.declare_u32_var("area color groups", 0);
        let [colored] = module
            .begin_compute([(sprites, Access::ComputeRW)])
            .with_name("area colors")
            .bind_compute_pipeline(pipeline)
            .bind_buffer(0, 1, sprites)
            .push_constants_from(push)
            .dispatch(groups, 1, 1)
            .end_compute();
        let program = module.compile(graph, colored)?;

        Ok(Self {
            program,
            sprites,
            push,
            groups,
        })
    }
}

pub(crate) struct TextureImage {
    image: Image,
    view: vk::ImageView,
}

impl TextureImage {
    pub(crate) fn attachment(&self, layout: vk::ImageLayout) -> ImageAttachment {
        ImageAttachment::from_image(&self.image, layout).with_image_view(self.view)
    }
}

struct TextureSource<'a> {
    width: u32,
    height: u32,
    pixels: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LevelRange {
    z: u32,
    base: u32,
    count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct VisibleRange {
    base: u32,
    count: u32,
    underlay_count: u32,
}

#[derive(Debug, Clone)]
struct MapViewPlan {
    camera: CameraPush,
    sprite_base: u32,
    visible: VisibleRange,
}

impl MapViewPlan {
    fn bind_scene(&self, cmd: &mut vir::Recorder<'_>) {
        cmd.set_viewport(0, Rect2D::framebuffer())
            .set_scissor(0, Rect2D::framebuffer())
            .push_constants(&self.camera);
    }
}

fn bind_sprite_cull(program: &mut Program, slots: &CullSlots, plan: &MapViewPlan) {
    let sprite_count = plan.visible.count;
    program.set_bytes(
        slots.push,
        &CullPush {
            center: plan.camera.center,
            viewport: plan.camera.viewport,
            focused_area_owner: plan.camera.focused_area_owner,
            zoom: plan.camera.zoom,
            first_sprite: plan.sprite_base + plan.visible.base,
            sprite_count,
            chunk_count: sprite_count.div_ceil(CULL_WORKGROUP_SIZE),
            active_first_relative: plan.visible.underlay_count,
        },
    );
    program.set(slots.sprite_count, sprite_count);
}

#[derive(Debug, Default)]
struct UploadedMapView {
    revision: u64,
    base: u32,
    count: usize,
    area_base: u32,
    area_count: usize,
    area_owners: Vec<dmm::PrefabInstanceId>,
    area_tile_indices: HashMap<dmm::PrefabInstanceId, u32>,
    ranges: Vec<LevelRange>,
}

#[derive(Debug, Default)]
struct UploadedLighting {
    revision: Option<u64>,
    base: u32,
    count: usize,
    size: [u32; 3],
}

struct UploadedPreview {
    revision: u64,
    sprites: Buffer,
    cull: CullBuffers,
}

struct HostBuffer {
    buffer: Buffer,
    capacity: u64,
}

impl HostBuffer {
    fn allocate(device: &mut Device, capacity: u64, name: &str) -> Result<Self, GpuError> {
        let capacity = capacity.max(1);
        let buffer = device.allocator.allocate_buffer(
            &BufferInfo::new(capacity, vk::BufferUsageFlags::STORAGE_BUFFER, MemoryLocation::CpuToGpu).with_name(name),
        )?;

        Ok(Self { buffer, capacity })
    }

    fn destroy(self, device: &mut Device) { device.allocator.deallocate_buffer(self.buffer); }
}

/// Resizes `slots` to one buffer per visible view and writes each payload, keeping a byte of
/// storage for an empty one so the pass can always bind it.
fn upload_per_view(
    device: &mut Device, slots: &mut Vec<HostBuffer>, payloads: &[&[u8]], name: &str,
) -> Result<(), GpuError> {
    let wanted = |payload: &[u8]| (payload.len() as u64).max(1).next_power_of_two();
    if slots.len() != payloads.len()
        || payloads
            .iter()
            .enumerate()
            .any(|(index, payload)| slots[index].capacity < wanted(payload))
    {
        device.wait_idle()?;
        while slots.len() > payloads.len() {
            if let Some(slot) = slots.pop() {
                slot.destroy(device);
            }
        }

        for (index, payload) in payloads.iter().enumerate() {
            let wanted = wanted(payload);
            match slots.get(index) {
                Some(slot) if slot.capacity >= wanted => {},
                Some(_) => {
                    let fresh = HostBuffer::allocate(device, wanted, name)?;
                    std::mem::replace(&mut slots[index], fresh).destroy(device);
                },
                None => slots.push(HostBuffer::allocate(device, wanted, name)?),
            }
        }
    }

    for (slot, payload) in slots.iter_mut().zip(payloads) {
        if !payload.is_empty() {
            slot.buffer.write(0, payload)?;
        }
    }

    Ok(())
}

impl UploadedPreview {
    fn allocate(device: &mut Device, preview: SpritePreview<'_>) -> Result<Self, GpuError> {
        let payload = gpu_sprite_range(
            preview.sprites,
            crate::UpdateRange {
                start: 0,
                end: preview.sprites.len(),
            },
        )?;
        let cull = CullBuffers::allocate(device, payload.len())?;
        let size = cull
            .sprite_capacity
            .checked_mul(size_of::<GpuSprite>())
            .and_then(|size| u64::try_from(size).ok());
        let Some(size) = size else {
            cull.destroy(device);
            return Err(GpuError::SpriteUploadTooLarge);
        };
        let mut sprites = match device.allocator.allocate_buffer(
            &BufferInfo::new(size, vk::BufferUsageFlags::STORAGE_BUFFER, MemoryLocation::CpuToGpu)
                .with_name("block preview sprites"),
        ) {
            Ok(buffer) => buffer,
            Err(error) => {
                cull.destroy(device);
                return Err(error.into());
            },
        };
        if let Err(error) = sprites.write(0, &payload) {
            device.allocator.deallocate_buffer(sprites);
            cull.destroy(device);
            return Err(error.into());
        }
        Ok(Self {
            revision: preview.revision,
            sprites,
            cull,
        })
    }

    fn destroy(self, device: &mut Device) {
        device.allocator.deallocate_buffer(self.sprites);
        self.cull.destroy(device);
    }
}

fn preview_plan(camera: crate::Camera, preview: SpritePreview<'_>) -> Result<MapViewPlan, GpuError> {
    Ok(MapViewPlan {
        camera: CameraPush {
            center: [camera.x - preview.offset[0], camera.y - preview.offset[1]],
            viewport: [
                camera.viewport_width.max(1) as f32,
                camera.viewport_height.max(1) as f32,
            ],
            zoom: camera.zoom,
            base: 0,
            focused_area_owner: [0; 2],
            placement_flash_owner: [0; 2],
            placement_flash_strength: 0.0,
        },
        sprite_base: 0,
        visible: VisibleRange {
            base: 0,
            count: u32::try_from(preview.sprites.len()).map_err(|_| GpuError::SpriteUploadTooLarge)?,
            underlay_count: 0,
        },
    })
}

pub struct Renderer {
    graph: RenderGraph,
    recorded: Option<Recorded>,
    frames: Option<SuperFrameAllocator>,
    swapchain: Option<SwapChain>,
    sprite_pipeline: PipelineId,
    visibility_pipeline: PipelineId,
    overlay_light_pipeline: PipelineId,
    cull_pipelines: [PipelineId; 3],
    lighting_pipeline: PipelineId,
    interaction_pipeline: PipelineId,
    guide_pipeline: PipelineId,
    pick_pipeline: PipelineId,
    area_colors: AreaColorPass,
    bindless: BindlessDescriptorSet,
    textures: Vec<TextureImage>,
    fallback: Option<TextureImage>,
    sampler: vk::Sampler,
    sprites: Option<Buffer>,
    sprite_capacity: usize,
    area_tiles: Option<Buffer>,
    area_tile_capacity: usize,
    lights: Option<Buffer>,
    light_capacity: usize,
    cull: Vec<CullBuffers>,
    guides: Vec<HostBuffer>,
    connected: Vec<HostBuffer>,
    pick_readback: Option<Buffer>,
    uploaded: Vec<UploadedMapView>,
    uploaded_lighting: Vec<UploadedLighting>,
    previews: Vec<Option<UploadedPreview>>,
    imgui: Option<ImGuiPass>,
    highlight_started_at: Instant,
    extent: vk::Extent2D,
    stale: bool,
    device: Device,
}

impl Renderer {
    pub const fn map_view_texture(index: usize) -> dear_imgui_rs::TextureId { crate::imgui::map_view_texture(index) }

    pub const fn sprite_texture(index: u32) -> dear_imgui_rs::TextureId { crate::imgui::sprite_texture(index) }

    pub fn new(device: Device, width: u32, height: u32, texture_capacity: usize) -> Result<Self, GpuError> {
        let texture_limit = device.max_bindless_textures.min(SPRITE_TEXTURE_CAPACITY);
        let reserve = TEXTURE_RESERVE.min(texture_limit as usize);
        let texture_capacity = descriptor_capacity(texture_capacity.max(reserve), texture_limit)?;
        let bindless = BindlessDescriptorSet::create(&device, texture_capacity)?;
        let mut graph = RenderGraph::new(&device.context);

        let geometry_vs = read_spirv(GEOMETRY_VS_SPV)?;
        let sprite_pipeline = match graph.declare_pipeline(
            GraphicsPipelineInfo::new()
                .with_shader(&geometry_vs)
                .with_shader(&read_spirv(SPRITE_SHADE_FS_SPV)?)
                .with_bindless_set(1, bindless.layout, bindless.set),
        ) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                drop(graph);
                bindless.destroy(&device);

                return Err(error.into());
            },
        };
        let visibility_pipeline = match graph.declare_pipeline(
            GraphicsPipelineInfo::new()
                .with_shader(&geometry_vs)
                .with_shader(&read_spirv(SPRITE_VIS_FS_SPV)?)
                .with_bindless_set(1, bindless.layout, bindless.set),
        ) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                drop(graph);
                bindless.destroy(&device);

                return Err(error.into());
            },
        };
        let overlay_light_pipeline = match graph.declare_pipeline(
            GraphicsPipelineInfo::new()
                .with_shader(&geometry_vs)
                .with_shader(&read_spirv(SPRITE_OVERLAY_LIGHT_FS_SPV)?)
                .with_bindless_set(1, bindless.layout, bindless.set),
        ) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                drop(graph);
                bindless.destroy(&device);

                return Err(error.into());
            },
        };
        let cull_pipelines = match (|| -> Result<[PipelineId; 3], GpuError> {
            Ok([
                graph.declare_compute_pipeline(ComputePipelineInfo::new(&read_spirv(SPRITE_CULL_CLASSIFY_CS_SPV)?))?,
                graph.declare_compute_pipeline(ComputePipelineInfo::new(&read_spirv(SPRITE_CULL_SCAN_CS_SPV)?))?,
                graph.declare_compute_pipeline(ComputePipelineInfo::new(&read_spirv(SPRITE_CULL_COMPACT_CS_SPV)?))?,
            ])
        })() {
            Ok(pipelines) => pipelines,
            Err(error) => {
                drop(graph);
                bindless.destroy(&device);

                return Err(error);
            },
        };
        let interaction_pipeline = match graph.declare_pipeline(
            GraphicsPipelineInfo::new()
                .with_shader(&read_spirv(INTERACTION_VS_SPV)?)
                .with_shader(&read_spirv(INTERACTION_FS_SPV)?),
        ) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                drop(graph);
                bindless.destroy(&device);

                return Err(error.into());
            },
        };
        let guide_pipeline = match graph.declare_pipeline(
            GraphicsPipelineInfo::new()
                .with_shader(&read_spirv(GUIDE_VS_SPV)?)
                .with_shader(&read_spirv(GUIDE_FS_SPV)?),
        ) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                drop(graph);
                bindless.destroy(&device);

                return Err(error.into());
            },
        };
        let lighting_pipeline = match graph.declare_pipeline(
            GraphicsPipelineInfo::new()
                .with_shader(&read_spirv(LIGHTING_VS_SPV)?)
                .with_shader(&read_spirv(LIGHTING_FS_SPV)?),
        ) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                drop(graph);
                bindless.destroy(&device);

                return Err(error.into());
            },
        };
        let pick_pipeline = match graph.declare_compute_pipeline(ComputePipelineInfo::new(&read_spirv(PICK_CS_SPV)?)) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                drop(graph);
                bindless.destroy(&device);

                return Err(error.into());
            },
        };
        let area_color_pipeline = match graph.declare_compute_pipeline(
            ComputePipelineInfo::new(&read_spirv(AREA_COLOR_CS_SPV)?).with_bindless_set(
                1,
                bindless.layout,
                bindless.set,
            ),
        ) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                drop(graph);
                bindless.destroy(&device);

                return Err(error.into());
            },
        };
        let area_colors = match AreaColorPass::record(&graph, area_color_pipeline) {
            Ok(pass) => pass,
            Err(error) => {
                drop(graph);
                bindless.destroy(&device);

                return Err(error);
            },
        };

        let mut renderer = Self {
            graph,
            recorded: None,
            frames: None,
            swapchain: None,
            sprite_pipeline,
            visibility_pipeline,
            overlay_light_pipeline,
            cull_pipelines,
            lighting_pipeline,
            interaction_pipeline,
            guide_pipeline,
            pick_pipeline,
            area_colors,
            bindless,
            textures: Vec::new(),
            fallback: None,
            sampler: vk::Sampler::null(),
            sprites: None,
            sprite_capacity: 0,
            area_tiles: None,
            area_tile_capacity: 0,
            lights: None,
            light_capacity: 0,
            cull: Vec::new(),
            guides: Vec::new(),
            connected: Vec::new(),
            pick_readback: None,
            uploaded: Vec::new(),
            uploaded_lighting: Vec::new(),
            previews: Vec::new(),
            imgui: None,
            highlight_started_at: Instant::now(),
            extent: vk::Extent2D {
                width: width.max(1),
                height: height.max(1),
            },
            stale: true,
            device,
        };

        renderer.sampler = renderer
            .device
            .allocator
            .allocate_sampler(&SamplerInfo::nearest().with_address_mode(vk::SamplerAddressMode::CLAMP_TO_EDGE))?;
        let pick_readback = renderer.device.allocator.allocate_buffer(
            &BufferInfo::new(4, vk::BufferUsageFlags::STORAGE_BUFFER, MemoryLocation::GpuToCpu)
                .with_name("pick readback"),
        )?;
        renderer.pick_readback = Some(pick_readback);
        renderer
            .pick_readback
            .as_mut()
            .ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?
            .write(0, &[0u32])?;
        renderer.fallback = renderer
            .upload_images(&[TextureSource {
                width: 1,
                height: 1,
                pixels: &[0; 4],
            }])?
            .pop();
        renderer.write_descriptors(&[])?;
        renderer.recreate_swapchain()?;

        Ok(renderer)
    }

    pub fn extent(&self) -> vk::Extent2D { self.extent }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }

        self.extent = vk::Extent2D { width, height };
        self.stale = true;
    }

    pub fn upload_textures(&mut self, catalog: &TextureCatalog) -> Result<(), GpuError> {
        let requested = u32::try_from(catalog.len()).map_err(|_| GpuError::TextureUploadTooLarge)?;
        if requested > self.bindless.count {
            return Err(GpuError::TextureCapacityExceeded {
                requested,
                capacity: self.bindless.count,
            });
        }

        self.device.wait_idle()?;

        let textures = self.upload_catalog(catalog)?;
        if let Err(error) = self.write_descriptors(&textures) {
            destroy_images(&mut self.device, textures);

            return Err(error);
        }

        self.recorded = None;
        self.uploaded.clear();
        let old = std::mem::replace(&mut self.textures, textures);
        destroy_images(&mut self.device, old);

        Ok(())
    }

    pub fn draw(&mut self, frame: &Frame<'_>) -> Result<(), GpuError> {
        self.draw_inner(frame, None, None)?;

        Ok(())
    }

    pub fn draw_imgui(
        &mut self, frame: &Frame<'_>, pending: PendingFrame<'_>,
    ) -> Result<Option<(usize, PickResult)>, GpuError> {
        if pending.draw_requirements().requires_raw_callback_support() {
            return Err(GpuError::RawDrawCallback);
        }

        if self.imgui.is_none() {
            let mut imgui = ImGuiPass::new(&mut self.device)?;
            if let Err(error) = imgui.declare_pipeline(&mut self.graph) {
                imgui.destroy(&mut self.device);

                return Err(error);
            }

            self.imgui = Some(imgui);
        }

        self.draw_inner(frame, None, Some(pending))
    }

    pub fn reset_imgui_textures(&mut self) -> Result<(), GpuError> {
        if let Some(imgui) = self.imgui.as_mut() {
            imgui.reset_textures(&mut self.device)?;
            self.recorded = None;
        }

        Ok(())
    }

    fn recreate_swapchain(&mut self) -> Result<(), GpuError> {
        self.device.wait_idle()?;
        self.recorded = None;

        let (swapchain, extent, _) =
            self.device
                .create_swapchain(self.extent.width, self.extent.height, self.swapchain.as_ref())?;
        self.frames = Some(
            self.device
                .context
                .create_super_frame_allocator(swapchain.attachments.len()),
        );
        self.swapchain = Some(swapchain);
        self.extent = extent;
        self.stale = false;

        Ok(())
    }

    fn record(&mut self, state: FrameGraphState) -> Result<(), GpuError> {
        let swapchain = self.swapchain.as_ref().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
        let with_imgui = state.with_imgui;
        let mut module = Module::default();
        let swapchain_attachment = module.acquire_next_image(swapchain);
        let sprites = module.declare_buffer_var("sprites", Access::ComputeWrite);
        let lights = state
            .map_views
            .iter()
            .any(|view| view.lighting)
            .then(|| module.declare_buffer_var("light tiles", Access::FragmentRead));
        let pick_result = (with_imgui && !state.map_views.is_empty())
            .then(|| module.declare_buffer_var("pick result", Access::HostRead));
        let mut pick_after = pick_result;
        let mut map_views = Vec::with_capacity(state.map_views.len());

        for (index, view) in state.map_views.iter().copied().enumerate() {
            let rect = view.rect;
            let viewport = vk::Extent2D {
                width: rect.width.max(1),
                height: rect.height.max(1),
            };
            let extent = module.declare_extent_3d_var(&format!("map view {index} extent"), extent3d(viewport));
            let mut scene_attachment = module.transient_image_sized(
                &ImageInfo::color_target(viewport, vk::Format::R8G8B8A8_SRGB)
                    .with_usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_SRC)
                    .with_name(format!("map view {index} scene attachment")),
                extent,
            );
            scene_attachment = module.clear(scene_attachment, vir::clear::f32::BLACK);
            let mut emissive_attachment = module.transient_image_sized(
                &ImageInfo::color_target(viewport, vk::Format::R8_UNORM)
                    .with_usage(vk::ImageUsageFlags::SAMPLED)
                    .with_name(format!("map view {index} emissive attachment")),
                extent,
            );
            emissive_attachment = module.clear(emissive_attachment, vir::clear::f32::BLACK);
            let mut visibility_attachment = module.transient_image_sized(
                &ImageInfo::color_target(viewport, vk::Format::R32_UINT)
                    .with_usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_SRC)
                    .with_name(format!("map view {index} visibility attachment")),
                extent,
            );
            visibility_attachment = module.clear(visibility_attachment, vir::clear::u32::TRANSPARENT);
            let mut level_attachment = module.transient_image_sized(
                &ImageInfo::color_target(viewport, vk::Format::R32_UINT)
                    .with_usage(vk::ImageUsageFlags::SAMPLED)
                    .with_name(format!("map view {index} level attachment")),
                extent,
            );
            level_attachment = module.clear(level_attachment, vir::clear::u32::TRANSPARENT);

            let visible_indices =
                module.declare_buffer_var(&format!("map view {index} visible sprite indices"), Access::VertexRead);
            let chunks = module.declare_buffer_var(&format!("map view {index} cull chunks"), Access::ComputeRead);
            let draw_commands =
                module.declare_buffer_var(&format!("map view {index} sprite draw commands"), Access::IndirectRead);

            let camera = module.declare_bytes_var(&format!("map view {index} camera"), size_of::<CameraPush>() as u32);
            let underlay_camera = module.declare_bytes_var(
                &format!("map view {index} underlay camera"),
                size_of::<CameraPush>() as u32,
            );
            let active_camera = module.declare_bytes_var(
                &format!("map view {index} active camera"),
                size_of::<CameraPush>() as u32,
            );

            let (cull_values, cull) = record_sprite_cull(
                &mut module,
                self.cull_pipelines,
                sprites,
                CullValues {
                    visible: visible_indices,
                    chunks,
                    commands: draw_commands,
                },
                &state,
                &format!("map view {index} cull"),
            );
            let draw_commands = cull_values.commands;
            let visible_indices = cull_values.visible;

            [
                scene_attachment,
                emissive_attachment,
                visibility_attachment,
                level_attachment,
                _,
                _,
            ] = module
                .begin_rendering([
                    (scene_attachment, Access::ColorRW),
                    (emissive_attachment, Access::ColorRW),
                    (visibility_attachment, Access::ColorRW),
                    (level_attachment, Access::ColorRW),
                    (draw_commands, Access::IndirectRead),
                    (visible_indices, Access::VertexRead),
                ])
                .with_name(format!("map view {index} sprites"))
                .bind_graphics_pipeline(self.visibility_pipeline)
                .set_primitive_topology(vk::PrimitiveTopology::TRIANGLE_STRIP)
                .set_viewport(0, Rect2D::framebuffer())
                .set_scissor(0, Rect2D::framebuffer())
                .set_color_blend(0, BlendPreset::PremultipliedAlphaBlend)
                .set_color_blend(1, BlendPreset::PremultipliedAlphaBlend)
                .set_color_blend(2, BlendPreset::Off)
                .set_color_blend(3, BlendPreset::Off)
                .set_rasterization(RasterizationState {
                    cull_mode: vk::CullModeFlags::NONE,
                    ..Default::default()
                })
                .bind_buffer(0, 1, sprites)
                .bind_buffer(0, 2, visible_indices)
                .specialize_constant(spec::SPRITE_INDIRECT, true)
                .specialize_constant(spec::SHOW_AREAS, state.show_areas)
                .specialize_constant(spec::SHOW_AREA_OUTLINES, state.show_area_outlines)
                .specialize_constant(spec::SPRITE_WRITES_ID, false)
                .push_constants_from(underlay_camera)
                .draw_indirect_at(draw_commands, 0, 1u32, DRAW_INDIRECT_STRIDE)
                .specialize_constant(spec::SPRITE_WRITES_ID, true)
                .push_constants_from(active_camera)
                .draw_indirect_at(
                    draw_commands,
                    u64::from(DRAW_INDIRECT_STRIDE),
                    1u32,
                    DRAW_INDIRECT_STRIDE,
                )
                .end_rendering();

            let lighting = if view.lighting {
                let light_tiles = lights.ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
                let mut overlay_lightmap = module.transient_image_sized(
                    &ImageInfo::color_target(viewport, vk::Format::R16G16B16A16_SFLOAT)
                        .with_usage(vk::ImageUsageFlags::SAMPLED)
                        .with_name(format!("map view {index} overlay lightmap")),
                    extent,
                );
                overlay_lightmap = module.clear(overlay_lightmap, vir::clear::f32::BLACK);
                [overlay_lightmap, _, _, _] = module
                    .begin_rendering([
                        (overlay_lightmap, Access::ColorRW),
                        (level_attachment, Access::FragmentSampled),
                        (draw_commands, Access::IndirectRead),
                        (visible_indices, Access::VertexRead),
                    ])
                    .with_name(format!("map view {index} overlay lights"))
                    .bind_graphics_pipeline(self.overlay_light_pipeline)
                    .set_primitive_topology(vk::PrimitiveTopology::TRIANGLE_STRIP)
                    .set_viewport(0, Rect2D::framebuffer())
                    .set_scissor(0, Rect2D::framebuffer())
                    .broadcast_color_blend(BlendPreset::Additive)
                    .set_rasterization(RasterizationState {
                        cull_mode: vk::CullModeFlags::NONE,
                        ..Default::default()
                    })
                    .bind_buffer(0, 1, sprites)
                    .bind_buffer(0, 2, visible_indices)
                    .bind_image(0, 3, level_attachment)
                    .specialize_constant(spec::SPRITE_INDIRECT, true)
                    .specialize_constant(spec::SHOW_AREAS, state.show_areas)
                    .specialize_constant(spec::SHOW_AREA_OUTLINES, state.show_area_outlines)
                    .push_constants_from(underlay_camera)
                    .draw_indirect_at(draw_commands, 0, 1u32, DRAW_INDIRECT_STRIDE)
                    .push_constants_from(active_camera)
                    .draw_indirect_at(
                        draw_commands,
                        u64::from(DRAW_INDIRECT_STRIDE),
                        1u32,
                        DRAW_INDIRECT_STRIDE,
                    )
                    .end_rendering();
                let push =
                    module.declare_bytes_var(&format!("map view {index} lighting"), size_of::<LightingPush>() as u32);
                let mut lit_attachment = module.transient_image_sized(
                    &ImageInfo::color_target(viewport, vk::Format::R8G8B8A8_SRGB)
                        .with_usage(vk::ImageUsageFlags::SAMPLED)
                        .with_name(format!("map view {index} lit scene attachment")),
                    extent,
                );
                lit_attachment = module.clear(lit_attachment, vir::clear::f32::BLACK);
                [lit_attachment] = module
                    .begin_rendering([(lit_attachment, Access::ColorRW)])
                    .with_name(format!("map view {index} static lighting"))
                    .bind_graphics_pipeline(self.lighting_pipeline)
                    .set_primitive_topology(vk::PrimitiveTopology::TRIANGLE_LIST)
                    .set_viewport(0, Rect2D::framebuffer())
                    .set_scissor(0, Rect2D::framebuffer())
                    .bind_texture(0, 0, scene_attachment, self.sampler)
                    .bind_buffer(0, 1, light_tiles)
                    .bind_texture(0, 2, emissive_attachment, self.sampler)
                    .bind_texture(0, 3, overlay_lightmap, self.sampler)
                    .bind_image(0, 4, level_attachment)
                    .push_constants_from(push)
                    .draw(3, 1)
                    .end_rendering();
                scene_attachment = lit_attachment;

                Some(LightingSlots { push })
            } else {
                None
            };

            [scene_attachment, _, _] = module
                .begin_rendering([
                    (scene_attachment, Access::ColorRW),
                    (draw_commands, Access::IndirectRead),
                    (visible_indices, Access::VertexRead),
                ])
                .with_name(format!("map view {index} area outlines"))
                .bind_graphics_pipeline(self.sprite_pipeline)
                .set_primitive_topology(vk::PrimitiveTopology::TRIANGLE_STRIP)
                .set_viewport(0, Rect2D::framebuffer())
                .set_scissor(0, Rect2D::framebuffer())
                .broadcast_color_blend(BlendPreset::PremultipliedAlphaBlend)
                .set_rasterization(RasterizationState {
                    cull_mode: vk::CullModeFlags::NONE,
                    ..Default::default()
                })
                .bind_buffer(0, 1, sprites)
                .bind_buffer(0, 2, visible_indices)
                .specialize_constant(spec::SPRITE_INDIRECT, true)
                .specialize_constant(spec::SHOW_AREAS, state.show_areas)
                .specialize_constant(spec::SHOW_AREA_OUTLINES, state.show_area_outlines)
                .specialize_constant(spec::SPRITE_EDGE_ONLY, true)
                .push_constants_from(active_camera)
                .draw_indirect_at(
                    draw_commands,
                    u64::from(DRAW_INDIRECT_STRIDE),
                    1u32,
                    DRAW_INDIRECT_STRIDE,
                )
                .end_rendering();

            let (mut output_attachment, interaction, guides, should_pick) = if with_imgui {
                // TODO: vir doesnt support explicit barriers, fix this shit
                [visibility_attachment] = module
                    .begin_compute([(visibility_attachment, Access::FragmentSampled | Access::ComputeSampled)])
                    .with_name(format!("map view {index} shared visibility access"))
                    .end_compute();

                let push = module.declare_bytes_var(
                    &format!("map view {index} interaction"),
                    size_of::<InteractionPush>() as u32,
                );
                let mut highlighted_scene_attachment = module.transient_image_sized(
                    &ImageInfo::color_target(viewport, vk::Format::R8G8B8A8_SRGB)
                        .with_usage(vk::ImageUsageFlags::SAMPLED)
                        .with_name(format!("map view {index} highlighted scene attachment")),
                    extent,
                );
                highlighted_scene_attachment = module.clear(highlighted_scene_attachment, vir::clear::f32::BLACK);
                let area_tiles =
                    module.declare_buffer_var(&format!("map view {index} hover area tiles"), Access::ComputeWrite);
                let highlight_draw = module.declare_callback_var(&format!("map view {index} highlight draw"));
                let hover_draw = module.declare_callback_var(&format!("map view {index} hover area draw"));
                let connected_owners =
                    module.declare_buffer_var(&format!("map view {index} connected owners"), Access::HostWrite);

                [highlighted_scene_attachment, _] = module
                    .begin_rendering([
                        (highlighted_scene_attachment, Access::ColorRW),
                        (connected_owners, Access::FragmentRead),
                    ])
                    .with_name(format!("map view {index} highlights"))
                    .bind_graphics_pipeline(self.interaction_pipeline)
                    .set_primitive_topology(vk::PrimitiveTopology::TRIANGLE_STRIP)
                    .set_viewport(0, Rect2D::framebuffer())
                    .set_scissor(0, Rect2D::framebuffer())
                    .bind_texture(0, 0, scene_attachment, self.sampler)
                    .bind_image(0, 1, visibility_attachment)
                    .bind_buffer(0, 2, sprites)
                    .bind_buffer(0, 4, connected_owners)
                    .specialize_constant(spec::HIGHLIGHT_TINT, view.highlight_tint)
                    .specialize_constant(spec::DELETE_MODE, view.delete_mode)
                    .push_constants_from(push)
                    .record_from(highlight_draw)
                    .end_rendering();

                [highlighted_scene_attachment, _] = module
                    .begin_rendering([
                        (highlighted_scene_attachment, Access::ColorRW),
                        (visible_indices, Access::VertexRead),
                    ])
                    .with_name(format!("map view {index} hover area outline"))
                    .bind_graphics_pipeline(self.sprite_pipeline)
                    .set_primitive_topology(vk::PrimitiveTopology::TRIANGLE_STRIP)
                    .set_viewport(0, Rect2D::framebuffer())
                    .set_scissor(0, Rect2D::framebuffer())
                    .broadcast_color_blend(BlendPreset::PremultipliedAlphaBlend)
                    .set_rasterization(RasterizationState {
                        cull_mode: vk::CullModeFlags::NONE,
                        ..Default::default()
                    })
                    .bind_buffer(0, 1, area_tiles)
                    .bind_buffer(0, 2, visible_indices)
                    .specialize_constant(spec::SPRITE_INDIRECT, false)
                    .specialize_constant(spec::SHOW_AREAS, state.show_areas)
                    .specialize_constant(spec::SHOW_AREA_OUTLINES, true)
                    .specialize_constant(spec::SPRITE_EDGE_ONLY, false)
                    .push_constants_from(camera)
                    .record_from(hover_draw)
                    .end_rendering();

                let guide_lines =
                    module.declare_buffer_var(&format!("map view {index} guide lines"), Access::HostWrite);
                let guide_push =
                    module.declare_bytes_var(&format!("map view {index} guide push"), size_of::<GuidePush>() as u32);
                let guide_draw = module.declare_callback_var(&format!("map view {index} guide draw"));
                [highlighted_scene_attachment, _] = module
                    .begin_rendering([
                        (highlighted_scene_attachment, Access::ColorRW),
                        (guide_lines, Access::VertexRead | Access::FragmentRead),
                    ])
                    .with_name(format!("map view {index} guides"))
                    .bind_graphics_pipeline(self.guide_pipeline)
                    .set_primitive_topology(vk::PrimitiveTopology::TRIANGLE_STRIP)
                    .set_viewport(0, Rect2D::framebuffer())
                    .set_scissor(0, Rect2D::framebuffer())
                    .set_rasterization(RasterizationState {
                        cull_mode: vk::CullModeFlags::NONE,
                        ..Default::default()
                    })
                    .bind_buffer(0, 0, guide_lines)
                    .bind_image(0, 1, visibility_attachment)
                    .bind_buffer(0, 2, sprites)
                    .push_constants_from(guide_push)
                    .record_from(guide_draw)
                    .end_rendering();

                let pick_push =
                    module.declare_bytes_var(&format!("map view {index} pick cursor"), size_of::<PickPush>() as u32);
                let should_pick = module.declare_bool_var(&format!("map view {index} should pick"), false);
                let pick_buffer = pick_after.ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
                let next_pick = module.set_condition(
                    should_pick,
                    |m| {
                        let [_visibility, next_pick] = m
                            .begin_compute([
                                (visibility_attachment, Access::ComputeSampled),
                                (pick_buffer, Access::ComputeWrite),
                            ])
                            .with_name(format!("map view {index} mouse pick"))
                            .bind_compute_pipeline(self.pick_pipeline)
                            .bind_image(0, 1, visibility_attachment)
                            .bind_buffer(0, 3, pick_buffer)
                            .push_constants_from(pick_push)
                            .dispatch(1u32, 1u32, 1u32)
                            .end_compute();
                        next_pick
                    },
                    |_| pick_buffer,
                );
                pick_after = Some(next_pick);

                (
                    highlighted_scene_attachment,
                    Some(InteractionSlots {
                        push,
                        connected_owners,
                        pick_push,
                        area_tiles,
                        hover_draw,
                        highlight_draw,
                    }),
                    Some(GuideSlots {
                        lines: guide_lines,
                        push: guide_push,
                        draw: guide_draw,
                    }),
                    Some(should_pick),
                )
            } else {
                (scene_attachment, None, None, None)
            };

            let preview = if view.preview {
                let name = format!("map view {index} block preview");
                let sprites = module.declare_buffer_var(&format!("{name} sprites"), Access::HostWrite);
                let camera = module.declare_bytes_var(&format!("{name} camera"), size_of::<CameraPush>() as u32);
                let visible = module.declare_buffer_var(&format!("{name} visible indices"), Access::VertexRead);
                let chunks = module.declare_buffer_var(&format!("{name} chunks"), Access::ComputeRead);
                let commands = module.declare_buffer_var(&format!("{name} draw commands"), Access::IndirectRead);
                let (values, cull) = record_sprite_cull(
                    &mut module,
                    self.cull_pipelines,
                    sprites,
                    CullValues {
                        visible,
                        chunks,
                        commands,
                    },
                    &state,
                    &name,
                );
                [output_attachment, _, _] = module
                    .begin_rendering([
                        (output_attachment, Access::ColorRW),
                        (values.commands, Access::IndirectRead),
                        (values.visible, Access::VertexRead),
                    ])
                    .with_name(name)
                    .bind_graphics_pipeline(self.sprite_pipeline)
                    .set_primitive_topology(vk::PrimitiveTopology::TRIANGLE_STRIP)
                    .set_viewport(0, Rect2D::framebuffer())
                    .set_scissor(0, Rect2D::framebuffer())
                    .broadcast_color_blend(BlendPreset::PremultipliedAlphaBlend)
                    .set_rasterization(RasterizationState {
                        cull_mode: vk::CullModeFlags::NONE,
                        ..Default::default()
                    })
                    .bind_buffer(0, 1, sprites)
                    .bind_buffer(0, 2, values.visible)
                    .specialize_constant(spec::SPRITE_INDIRECT, true)
                    .specialize_constant(spec::SHOW_AREAS, state.show_areas)
                    .specialize_constant(spec::SHOW_AREA_OUTLINES, state.show_area_outlines)
                    .specialize_constant(spec::SPRITE_EDGE_ONLY, false)
                    .push_constants_from(camera)
                    .draw_indirect_at(
                        values.commands,
                        u64::from(DRAW_INDIRECT_STRIDE),
                        1u32,
                        DRAW_INDIRECT_STRIDE,
                    )
                    .end_rendering();
                Some(PreviewSlots { sprites, camera, cull })
            } else {
                None
            };

            map_views.push(MapViewPass {
                scene_attachment,
                visibility_attachment,
                output_attachment,
                extent,
                camera,
                underlay_camera,
                active_camera,
                cull,
                lighting,
                preview,
                should_pick,
                interaction,
                guides,
            });
        }

        let pick_host = pick_after.map(|pick| module.release(pick, Access::HostRead, DomainFlag::Host));
        let mut sampled_map_view_roots = Vec::new();
        let (presented_attachment, ui) = if with_imgui {
            for map_view in &map_views {
                let sampled = module.release(
                    map_view.output_attachment,
                    Access::FragmentSampled,
                    DomainFlag::Graphics,
                );
                sampled_map_view_roots.push(sampled);
            }

            let imgui = self.imgui.as_ref().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
            let target_attachment = module.clear(swapchain_attachment, vir::clear::f32::BLACK);
            let map_view_outputs = map_views.iter().map(|view| view.output_attachment).collect::<Vec<_>>();

            imgui.record(&mut module, target_attachment, &map_view_outputs)
        } else if let Some(map_view) = map_views.first() {
            (
                module.blit_filtered(map_view.output_attachment, swapchain_attachment, vk::Filter::NEAREST),
                None,
            )
        } else {
            (module.clear(swapchain_attachment, vir::clear::f32::BLACK), None)
        };
        let present = module.present(presented_attachment);
        let mut roots = sampled_map_view_roots;
        if let Some(pick_host) = pick_host {
            roots.push(pick_host);
        }
        roots.push(present);
        let program = module.compile_all(&self.graph, &roots)?;

        self.recorded = Some(Recorded {
            program,
            sprites,
            lights,
            map_views,
            pick_result,
            state,
            ui,
        });

        Ok(())
    }

    fn draw_inner(
        &mut self, frame: &Frame<'_>, viewport: Option<vk::Extent2D>, pending: Option<PendingFrame<'_>>,
    ) -> Result<Option<(usize, PickResult)>, GpuError> {
        if self.stale {
            self.recreate_swapchain()?;
        }

        let viewport = viewport.unwrap_or(self.extent);
        let with_imgui = pending.is_some();
        let graph_state = FrameGraphState::new(viewport, with_imgui, frame);
        if self
            .recorded
            .as_ref()
            .is_none_or(|recorded| recorded.state != graph_state)
        {
            self.record(graph_state)?;
        }

        self.prepare_sprites(frame)?;
        self.prepare_lighting(frame)?;
        self.prepare_cull_buffers(frame)?;
        self.prepare_previews(frame)?;
        let stripe_offset =
            (self.highlight_started_at.elapsed().as_secs_f32() * HIGHLIGHT_STRIPE_SPEED) % HIGHLIGHT_STRIPE_PERIOD;
        let guide_counts = self.prepare_guides(frame)?;

        let recorded = self.recorded.as_mut().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
        let frames = self.frames.as_mut().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
        let sprites = self.sprites.as_ref().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
        recorded.program.set(recorded.sprites, sprites);
        if let Some(slot) = recorded.lights {
            let lights = self.lights.as_ref().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
            recorded.program.set(slot, lights);
        }

        let plans: Vec<MapViewPlan> = frame
            .map_views
            .iter()
            .zip(&self.uploaded)
            .filter(|(view, _)| !view.rect.is_empty())
            .map(|(view, uploaded)| {
                let (placement_flash_owner, placement_flash_strength) = view
                    .interaction
                    .placement_flash
                    .filter(|flash| flash.strength.is_finite() && flash.strength > 0.0)
                    .map_or(([0; 2], 0.0), |flash| {
                        (owner_words(flash.owner), flash.strength.min(1.0))
                    });
                let visible = visible_range(&uploaded.ranges, view.active_z, frame.underlay_depth);
                let logical = [
                    view.camera.viewport_width.max(1) as f32,
                    view.camera.viewport_height.max(1) as f32,
                ];

                MapViewPlan {
                    camera: CameraPush {
                        center: [view.camera.x, view.camera.y],
                        viewport: logical,
                        zoom: view.camera.zoom,
                        base: uploaded.base,
                        focused_area_owner: view.focused_area.map(owner_words).unwrap_or([0; 2]),
                        placement_flash_owner,
                        placement_flash_strength,
                    },
                    sprite_base: uploaded.base,
                    visible,
                }
            })
            .collect();

        let visible: Vec<_> = frame
            .map_views
            .iter()
            .zip(&self.uploaded)
            .zip(&self.uploaded_lighting)
            .filter(|((view, _), _)| !view.rect.is_empty())
            .collect();
        if plans.len() != recorded.map_views.len() || visible.len() != recorded.map_views.len() {
            return Err(vk::Result::ERROR_INITIALIZATION_FAILED.into());
        }

        let picking = frame
            .picking
            .filter(|index| frame.map_views.get(*index).is_some_and(|view| !view.rect.is_empty()));
        let picking_map = picking.map(|index| {
            frame.map_views[..index]
                .iter()
                .filter(|view| !view.rect.is_empty())
                .count()
        });
        let cursor = picking.and_then(|index| {
            let view = &frame.map_views[index];
            view.interaction
                .cursor
                .filter(|cursor| cursor[0] < view.rect.width && cursor[1] < view.rect.height)
        });
        let pick = picking.and_then(|index| frame.map_views[index].interaction.mode.pick());
        let area_tiles = self.area_tiles.as_ref().unwrap_or(sprites);

        if let Some(slot) = recorded.pick_result {
            let pick_readback = self
                .pick_readback
                .as_ref()
                .ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
            recorded.program.set(slot, pick_readback);
        }

        for (index, map_view) in recorded.map_views.iter().enumerate() {
            let (((view, uploaded), uploaded_lighting), plan) = (visible[index], plans[index].clone());
            let map_extent = vk::Extent2D {
                width: view.rect.width.max(1),
                height: view.rect.height.max(1),
            };
            recorded.program.set(map_view.extent, extent3d(map_extent));
            recorded.program.set_bytes(map_view.camera, &plan.camera);
            recorded
                .program
                .set_bytes(map_view.underlay_camera, &CameraPush { base: 0, ..plan.camera });
            recorded.program.set_bytes(
                map_view.active_camera,
                &CameraPush {
                    base: plan.visible.underlay_count,
                    ..plan.camera
                },
            );

            let buffers = self.cull.get(index).ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
            recorded.program.set(map_view.cull.visible, buffers.visible);
            recorded.program.set(map_view.cull.chunks, buffers.chunks);
            recorded.program.set(map_view.cull.commands, buffers.commands);
            bind_sprite_cull(&mut recorded.program, &map_view.cull, &plan);

            if let Some(slots) = &map_view.lighting {
                let lighting = view.lighting.ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
                if uploaded_lighting.revision != Some(lighting.revision) || uploaded_lighting.size != lighting.size {
                    return Err(vk::Result::ERROR_INITIALIZATION_FAILED.into());
                }
                recorded.program.set_bytes(
                    slots.push,
                    &LightingPush {
                        center: plan.camera.center,
                        viewport: plan.camera.viewport,
                        zoom: plan.camera.zoom,
                        tile_size: lighting.tile_size.max(1) as f32,
                        map_size: [lighting.size[0], lighting.size[1]],
                        level_count: lighting.size[2],
                        base: uploaded_lighting.base,
                    },
                );
            }

            if let Some(slots) = &map_view.preview {
                let preview = view.preview.ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
                let uploaded = self
                    .previews
                    .get(index)
                    .and_then(Option::as_ref)
                    .ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
                let plan = preview_plan(view.camera, preview)?;
                recorded.program.set(slots.sprites, uploaded.sprites);
                recorded.program.set_bytes(slots.camera, &plan.camera);
                recorded.program.set(slots.cull.visible, uploaded.cull.visible);
                recorded.program.set(slots.cull.chunks, uploaded.cull.chunks);
                recorded.program.set(slots.cull.commands, uploaded.cull.commands);
                bind_sprite_cull(&mut recorded.program, &slots.cull, &plan);
            }

            let Some(slots) = map_view.interaction.as_ref() else {
                continue;
            };
            let interaction = view.interaction;
            let local_cursor = interaction
                .cursor
                .filter(|cursor| cursor[0] < view.rect.width && cursor[1] < view.rect.height);
            let (guide_count, connected_count) = guide_counts.get(index).copied().unwrap_or((0, 0));
            let connected = self
                .connected
                .get(index)
                .ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
            let push = InteractionPush {
                cursor: local_cursor.unwrap_or([0; 2]),
                cursor_valid: u32::from(local_cursor.is_some()),
                selected_owner: interaction.selected.map(owner_words).unwrap_or([0; 2]),
                stripe_offset,
                focused_area_owner: view.focused_area.map(owner_words).unwrap_or([0; 2]),
                connected_count,
            };
            recorded.program.set_bytes(slots.push, &push);
            recorded.program.set(slots.connected_owners, connected.buffer);
            recorded.program.set_bytes(
                slots.pick_push,
                &PickPush {
                    cursor: local_cursor.unwrap_or([0; 2]),
                    cursor_valid: u32::from(local_cursor.is_some()),
                },
            );
            let should_pick = map_view.should_pick.ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
            recorded.program.set(
                should_pick,
                pick.is_some() && picking_map == Some(index) && local_cursor.is_some(),
            );
            recorded.program.set(slots.area_tiles, area_tiles);

            if let Some(slots) = &map_view.guides {
                let guides = self.guides.get(index).ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
                let count = guide_count;
                recorded.program.set(slots.lines, guides.buffer);
                recorded.program.set_bytes(
                    slots.push,
                    &GuidePush {
                        viewport: [map_extent.width as f32, map_extent.height as f32],
                        selected_owner: interaction.selected.map(owner_words).unwrap_or([0; 2]),
                        stripe_offset,
                    },
                );
                recorded.program.set(
                    slots.draw,
                    PassCallback::new(move |cmd| {
                        if count > 0 {
                            cmd.set_viewport(0, Rect2D::framebuffer())
                                .set_scissor(0, Rect2D::framebuffer())
                                .draw(4, count);
                        }
                    }),
                );
            }

            recorded.program.set(
                slots.highlight_draw,
                PassCallback::new(move |cmd| {
                    cmd.set_viewport(0, Rect2D::framebuffer())
                        .set_scissor(0, Rect2D::framebuffer())
                        .push_constants(&push)
                        .draw(4, 1);
                }),
            );

            let hover = view
                .interaction
                .hovered_area
                .and_then(|owner| uploaded.area_tile_indices.get(&owner).copied())
                .map(|base| (plan, uploaded.area_base + base));
            recorded.program.set(
                slots.hover_draw,
                PassCallback::new(move |cmd| {
                    let Some((plan, base)) = hover.as_ref() else {
                        return;
                    };

                    plan.bind_scene(cmd);
                    cmd.push_constants_at(std::mem::offset_of!(CameraPush, base) as u32, base);
                    cmd.draw(4, 1);
                }),
            );

            let _ = (
                map_view.scene_attachment,
                map_view.visibility_attachment,
                map_view.output_attachment,
            );
        }

        let next = frames.get_next_frame()?;
        if let Some(pending) = pending {
            let imgui = self.imgui.as_mut().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
            let feedback = imgui.poll_textures(&mut self.device, &mut self.graph, next, pending.texture_requests())?;
            let reconciled = pending
                .reconcile_texture_feedback(feedback)
                .map_err(|e| GpuError::ImGui(e.to_string()))?;
            let ui_frame = imgui.prepare(next, reconciled.draw_data(), self.extent, &self.textures)?;

            if let Some(slots) = recorded.ui.as_ref() {
                imgui.bind(&mut recorded.program, slots, &ui_frame);
            }
        }

        let executed =
            match self
                .graph
                .execute(&self.device.context, &recorded.program, &mut AllocatorKind::Frame(next))
            {
                Ok(()) => true,
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR | vk::Result::SUBOPTIMAL_KHR) => {
                    self.stale = true;

                    false
                },
                Err(error) => return Err(error.into()),
            };

        let (Some(index), true, true) = (picking, pick.is_some(), cursor.is_some()) else {
            return Ok(None);
        };
        if !executed {
            return Ok(None);
        }

        self.graph.wait()?;
        let bytes = self
            .pick_readback
            .as_mut()
            .and_then(Buffer::mapped_slice_mut)
            .and_then(|bytes| bytes.get(..4))
            .ok_or(vk::Result::ERROR_MEMORY_MAP_FAILED)?;
        let raw = u32::from_ne_bytes(bytes.try_into().map_err(|_| vk::Result::ERROR_MEMORY_MAP_FAILED)?);
        let base = self.uploaded.get(index).map_or(0, |uploaded| uploaded.base) as usize;
        let picked = VisibilityId::from_raw(raw)
            .map(VisibilityId::sprite_index)
            .and_then(|sprite| sprite.checked_sub(base))
            .and_then(|sprite| frame.map_views.get(index)?.sprite_instances.get(sprite))
            .map_or(PickResult::Miss, |sprite| PickResult::Hit(sprite.owner));

        Ok(Some((index, picked)))
    }

    fn prepare_sprites(&mut self, frame: &Frame<'_>) -> Result<(), GpuError> {
        if self.sprites.is_some()
            && self.uploaded.len() == frame.map_views.len()
            && self
                .uploaded
                .iter()
                .zip(frame.map_views)
                .all(|(uploaded, view)| uploaded.revision == view.revision)
        {
            return Ok(());
        }

        if frame.map_views.len() == 1 && self.uploaded.len() == 1 && self.prepare_frame_update(frame)? {
            return Ok(());
        }

        let total: usize = frame.map_views.iter().map(|view| view.sprite_instances.len()).sum();
        u32::try_from(total).map_err(|_| GpuError::SpriteUploadTooLarge)?;
        let mut payload = Vec::with_capacity(total);
        let area_total: usize = frame.map_views.iter().map(|view| view.area_tiles.len()).sum();
        u32::try_from(area_total).map_err(|_| GpuError::SpriteUploadTooLarge)?;
        let mut area_payload = Vec::with_capacity(area_total);

        let mut uploaded = Vec::with_capacity(frame.map_views.len());
        for view in frame.map_views {
            let base = u32::try_from(payload.len()).map_err(|_| GpuError::SpriteUploadTooLarge)?;
            let area_base = u32::try_from(area_payload.len()).map_err(|_| GpuError::SpriteUploadTooLarge)?;

            for sprite in view.sprite_instances {
                let index = payload.len();
                payload.push(gpu_sprite(index, sprite)?);
            }

            let mut area_tile_indices = HashMap::with_capacity(view.area_tiles.len());
            for area in view.area_tiles {
                let index = area_payload.len();
                let local = u32::try_from(index).map_err(|_| GpuError::SpriteUploadTooLarge)? - area_base;
                area_tile_indices.insert(area.owner, local);
                area_payload.push(gpu_sprite(index, area)?);
            }

            uploaded.push(UploadedMapView {
                revision: view.revision,
                base,
                count: view.sprite_instances.len(),
                area_base,
                area_count: view.area_tiles.len(),
                area_owners: view.area_tiles.iter().map(|area| area.owner).collect(),
                area_tile_indices,
                ranges: level_ranges(view.sprite_instances),
            });
        }

        self.device.wait_idle()?;
        self.uploaded.clear();

        if self.sprites.is_none() || self.sprite_capacity < total {
            let capacity = total
                .max(1024)
                .checked_next_power_of_two()
                .ok_or(GpuError::SpriteUploadTooLarge)?;
            let size = capacity
                .checked_mul(size_of::<GpuSprite>())
                .ok_or(GpuError::SpriteUploadTooLarge)?;
            let size = u64::try_from(size).map_err(|_| GpuError::SpriteUploadTooLarge)?;
            let mut buffer = self.device.allocator.allocate_buffer(
                &BufferInfo::new(size, vk::BufferUsageFlags::STORAGE_BUFFER, MemoryLocation::CpuToGpu)
                    .with_name("sprites"),
            )?;
            if let Err(error) = buffer.write(0, &payload) {
                self.device.allocator.deallocate_buffer(buffer);

                return Err(error.into());
            }

            if let Some(old) = self.sprites.replace(buffer) {
                self.device.allocator.deallocate_buffer(old);
            }

            self.sprite_capacity = capacity;
        } else if let Some(buffer) = self.sprites.as_mut() {
            buffer.write(0, &payload)?;
        }

        if area_total > 0 && (self.area_tiles.is_none() || self.area_tile_capacity < area_total) {
            let capacity = area_total
                .max(1024)
                .checked_next_power_of_two()
                .ok_or(GpuError::SpriteUploadTooLarge)?;
            let size = capacity
                .checked_mul(size_of::<GpuSprite>())
                .ok_or(GpuError::SpriteUploadTooLarge)?;
            let size = u64::try_from(size).map_err(|_| GpuError::SpriteUploadTooLarge)?;
            let mut buffer = self.device.allocator.allocate_buffer(
                &BufferInfo::new(size, vk::BufferUsageFlags::STORAGE_BUFFER, MemoryLocation::CpuToGpu)
                    .with_name("area tile outlines"),
            )?;
            if let Err(error) = buffer.write(0, &area_payload) {
                self.device.allocator.deallocate_buffer(buffer);

                return Err(error.into());
            }

            if let Some(old) = self.area_tiles.replace(buffer) {
                self.device.allocator.deallocate_buffer(old);
            }

            self.area_tile_capacity = capacity;
        } else if let Some(buffer) = self.area_tiles.as_mut() {
            buffer.write(0, &area_payload)?;
        }

        let sprites = *self.sprites.as_ref().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
        self.color_area_outlines(sprites, crate::UpdateRange { start: 0, end: total })?;
        if let Some(area_tiles) = self.area_tiles {
            self.color_area_outlines(
                area_tiles,
                crate::UpdateRange {
                    start: 0,
                    end: area_total,
                },
            )?;
        }

        self.uploaded = uploaded;

        Ok(())
    }

    fn prepare_lighting(&mut self, frame: &Frame<'_>) -> Result<(), GpuError> {
        if self.uploaded_lighting.len() == frame.map_views.len()
            && self
                .uploaded_lighting
                .iter()
                .zip(frame.map_views)
                .all(|(uploaded, view)| uploaded.revision == view.lighting.map(|lighting| lighting.revision))
        {
            return Ok(());
        }

        if frame.map_views.len() == 1 && self.uploaded_lighting.len() == 1 && self.prepare_lighting_update(frame)? {
            return Ok(());
        }

        let total = frame
            .map_views
            .iter()
            .filter_map(|view| view.lighting)
            .try_fold(0usize, |total, lighting| total.checked_add(lighting.tiles.len()))
            .ok_or(GpuError::SpriteUploadTooLarge)?;

        u32::try_from(total).map_err(|_| GpuError::SpriteUploadTooLarge)?;

        let mut payload = Vec::with_capacity(total);
        let mut uploaded = Vec::with_capacity(frame.map_views.len());
        for view in frame.map_views {
            let base = u32::try_from(payload.len()).map_err(|_| GpuError::SpriteUploadTooLarge)?;
            let Some(lighting) = view.lighting else {
                uploaded.push(UploadedLighting {
                    base,
                    ..Default::default()
                });
                continue;
            };
            let expected = lighting
                .size
                .into_iter()
                .try_fold(1usize, |length, value| length.checked_mul(value as usize));
            if expected != Some(lighting.tiles.len()) {
                return Err(vk::Result::ERROR_INITIALIZATION_FAILED.into());
            }
            for (index, tile) in lighting.tiles.iter().enumerate() {
                payload.push(gpu_light_tile(index, tile)?);
            }
            uploaded.push(UploadedLighting {
                revision: Some(lighting.revision),
                base,
                count: lighting.tiles.len(),
                size: lighting.size,
            });
        }

        self.device.wait_idle()?;
        if total > 0 && (self.lights.is_none() || self.light_capacity < total) {
            let capacity = total
                .max(1024)
                .checked_next_power_of_two()
                .ok_or(GpuError::SpriteUploadTooLarge)?;
            let size = capacity
                .checked_mul(size_of::<GpuLightTile>())
                .and_then(|size| u64::try_from(size).ok())
                .ok_or(GpuError::SpriteUploadTooLarge)?;
            let mut buffer = self.device.allocator.allocate_buffer(
                &BufferInfo::new(size, vk::BufferUsageFlags::STORAGE_BUFFER, MemoryLocation::CpuToGpu)
                    .with_name("static light tiles"),
            )?;

            if let Err(error) = buffer.write(0, &payload) {
                self.device.allocator.deallocate_buffer(buffer);
                return Err(error.into());
            }

            if let Some(old) = self.lights.replace(buffer) {
                self.device.allocator.deallocate_buffer(old);
            }

            self.light_capacity = capacity;
        } else if total > 0
            && let Some(buffer) = self.lights.as_mut()
        {
            buffer.write(0, &payload)?;
        }
        self.uploaded_lighting = uploaded;

        Ok(())
    }

    fn prepare_lighting_update(&mut self, frame: &Frame<'_>) -> Result<bool, GpuError> {
        let (Some(view), Some(uploaded)) = (frame.map_views.first(), self.uploaded_lighting.first()) else {
            return Ok(false);
        };
        let (Some(lighting), Some(update), Some(buffer)) = (
            view.lighting,
            view.lighting.and_then(|lighting| lighting.pending_update),
            self.lights.as_mut(),
        ) else {
            return Ok(false);
        };

        let range = update.tiles;
        if uploaded.base != 0
            || uploaded.revision != Some(update.previous_revision)
            || uploaded.size != lighting.size
            || uploaded.count != lighting.tiles.len()
            || lighting.tiles.len() > self.light_capacity
            || range.start > range.end
            || range.end > lighting.tiles.len()
        {
            return Ok(false);
        }

        let payload = lighting.tiles[range.start..range.end]
            .iter()
            .enumerate()
            .map(|(offset, tile)| gpu_light_tile(range.start + offset, tile))
            .collect::<Result<Vec<_>, _>>()?;
        if !payload.is_empty() {
            self.graph.wait()?;
            buffer.write(gpu_light_offset(range.start)?, &payload)?;
        }

        let Some(uploaded) = self.uploaded_lighting.first_mut() else {
            return Ok(false);
        };

        uploaded.revision = Some(lighting.revision);

        Ok(true)
    }

    /// Projects each visible view's guide lines and uploads them, returning the instance count per view.
    /// Projects each visible view's guide lines and collects the owners linked to its selection,
    /// uploading both and returning the guide and connected counts per view.
    fn prepare_guides(&mut self, frame: &Frame<'_>) -> Result<Vec<(u32, u32)>, GpuError> {
        let finite = |origin: &[f32; 2], target: &[f32; 2]| origin.iter().chain(target).all(|value| value.is_finite());
        let visible = || frame.map_views.iter().filter(|view| !view.rect.is_empty());
        let lines = visible()
            .map(|view| {
                view.guide_lines
                    .iter()
                    .copied()
                    .filter(|guide| finite(&guide.origin, &guide.target))
                    .map(|guide| {
                        let (origin, target) = project_guide_line(guide, view.camera, view.rect);
                        GpuGuideLine { origin, target }
                    })
                    .filter(|guide| finite(&guide.origin, &guide.target))
                    .take(u32::MAX as usize)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let owners = visible()
            .map(|view| {
                view.connected
                    .iter()
                    .copied()
                    .map(owner_words)
                    .take(u32::MAX as usize)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        let line_bytes = lines.iter().map(|lines| payload_bytes(lines)).collect::<Vec<_>>();
        let owner_bytes = owners.iter().map(|owners| payload_bytes(owners)).collect::<Vec<_>>();
        upload_per_view(&mut self.device, &mut self.guides, &line_bytes, "selection guide lines")?;
        upload_per_view(&mut self.device, &mut self.connected, &owner_bytes, "connected owners")?;

        Ok(lines
            .iter()
            .zip(&owners)
            .map(|(lines, owners)| (lines.len() as u32, owners.len() as u32))
            .collect())
    }

    fn prepare_previews(&mut self, frame: &Frame<'_>) -> Result<(), GpuError> {
        let wanted: Vec<_> = frame
            .map_views
            .iter()
            .filter(|view| !view.rect.is_empty())
            .map(|view| view.preview.filter(|preview| !preview.sprites.is_empty()))
            .collect();
        if wanted.len() == self.previews.len()
            && wanted.iter().zip(&self.previews).all(|(preview, uploaded)| {
                preview.is_none_or(|preview| {
                    uploaded
                        .as_ref()
                        .is_some_and(|uploaded| uploaded.revision == preview.revision)
                })
            })
        {
            return Ok(());
        }

        self.graph.wait()?;
        for preview in self.previews.drain(wanted.len().min(self.previews.len())..).flatten() {
            preview.destroy(&mut self.device);
        }
        self.previews.resize_with(wanted.len(), || None);
        for (slot, preview) in self.previews.iter_mut().zip(wanted) {
            let Some(preview) = preview else { continue };
            if slot
                .as_ref()
                .is_some_and(|uploaded| uploaded.revision == preview.revision)
            {
                continue;
            }
            if let Some(uploaded) = slot
                .as_mut()
                .filter(|uploaded| uploaded.cull.sprite_capacity >= preview.sprites.len())
            {
                let payload = gpu_sprite_range(
                    preview.sprites,
                    crate::UpdateRange {
                        start: 0,
                        end: preview.sprites.len(),
                    },
                )?;
                uploaded.sprites.write(0, &payload)?;
                uploaded.revision = preview.revision;
            } else {
                let uploaded = UploadedPreview::allocate(&mut self.device, preview)?;
                if let Some(previous) = slot.replace(uploaded) {
                    previous.destroy(&mut self.device);
                }
            }
        }
        Ok(())
    }

    fn prepare_cull_buffers(&mut self, frame: &Frame<'_>) -> Result<(), GpuError> {
        let wanted = frame
            .map_views
            .iter()
            .filter(|view| !view.rect.is_empty())
            .map(|view| view.sprite_instances.len())
            .collect::<Vec<_>>();
        if wanted.len() == self.cull.len()
            && wanted
                .iter()
                .zip(&self.cull)
                .all(|(sprites, buffers)| buffers.sprite_capacity >= *sprites)
        {
            return Ok(());
        }

        self.device.wait_idle()?;

        while self.cull.len() > wanted.len() {
            if let Some(buffers) = self.cull.pop() {
                buffers.destroy(&mut self.device);
            }
        }

        for (index, sprites) in wanted.into_iter().enumerate() {
            if self
                .cull
                .get(index)
                .is_some_and(|buffers| buffers.sprite_capacity >= sprites)
            {
                continue;
            }

            let buffers = CullBuffers::allocate(&mut self.device, sprites)?;
            match self.cull.get_mut(index) {
                Some(slot) => {
                    let previous = std::mem::replace(slot, buffers);
                    previous.destroy(&mut self.device);
                },
                None => self.cull.push(buffers),
            }
        }

        Ok(())
    }

    fn prepare_frame_update(&mut self, frame: &Frame<'_>) -> Result<bool, GpuError> {
        let (Some(view), Some(uploaded)) = (frame.map_views.first(), self.uploaded.first()) else {
            return Ok(false);
        };
        let Some(update) = view.pending_update else {
            return Ok(false);
        };
        if uploaded.base != 0
            || uploaded.area_base != 0
            || uploaded.revision != update.previous_revision
            || view.sprite_instances.len() > self.sprite_capacity
            || view.area_tiles.len() > self.area_tile_capacity
            || !valid_update_range(update.sprites, view.sprite_instances.len())
            || !valid_update_range(update.area_tiles, view.area_tiles.len())
            || (uploaded.count != view.sprite_instances.len() && update.sprites.is_none())
            || (uploaded.area_count != view.area_tiles.len() && update.area_tiles.is_none())
        {
            return Ok(false);
        }
        let Some(sprites) = self.sprites else {
            return Ok(false);
        };
        if !view.area_tiles.is_empty() && self.area_tiles.is_none() {
            return Ok(false);
        }

        let sprite_payload = update
            .sprites
            .map(|range| gpu_sprite_range(view.sprite_instances, range))
            .transpose()?;
        let area_payload = update
            .area_tiles
            .map(|range| gpu_sprite_range(view.area_tiles, range))
            .transpose()?;
        if sprite_payload.as_ref().is_some_and(|payload| !payload.is_empty())
            || area_payload.as_ref().is_some_and(|payload| !payload.is_empty())
        {
            self.graph.wait()?;
        }
        if let (Some(range), Some(payload), Some(buffer)) =
            (update.sprites, sprite_payload.as_ref(), self.sprites.as_mut())
            && !payload.is_empty()
        {
            buffer.write(gpu_sprite_offset(range.start)?, payload)?;
        }
        if let (Some(range), Some(payload), Some(buffer)) =
            (update.area_tiles, area_payload.as_ref(), self.area_tiles.as_mut())
            && !payload.is_empty()
        {
            buffer.write(gpu_sprite_offset(range.start)?, payload)?;
        }
        if let Some(range) = update.sprites {
            self.color_area_outlines(sprites, range)?;
        }
        if let (Some(buffer), Some(range)) = (self.area_tiles, update.area_tiles) {
            self.color_area_outlines(buffer, range)?;
            let Some(uploaded) = self.uploaded.first_mut() else {
                return Ok(false);
            };
            patch_area_tile_indices(
                &mut uploaded.area_tile_indices,
                &mut uploaded.area_owners,
                view.area_tiles,
                range,
            )?;
        }

        let Some(uploaded) = self.uploaded.first_mut() else {
            return Ok(false);
        };
        if uploaded.count != view.sprite_instances.len() {
            uploaded.ranges = level_ranges(view.sprite_instances);
        }
        uploaded.revision = view.revision;
        uploaded.count = view.sprite_instances.len();
        uploaded.area_count = view.area_tiles.len();

        Ok(true)
    }

    fn color_area_outlines(&mut self, buffer: Buffer, range: crate::UpdateRange) -> Result<(), GpuError> {
        let first_sprite = u32::try_from(range.start).map_err(|_| GpuError::SpriteUploadTooLarge)?;
        let sprite_count =
            u32::try_from(range.end.saturating_sub(range.start)).map_err(|_| GpuError::SpriteUploadTooLarge)?;
        if sprite_count == 0 {
            return Ok(());
        }

        let group_count = area_color_group_count(sprite_count, self.device.max_compute_work_group_count_x);
        self.area_colors.program.set(self.area_colors.sprites, buffer);
        self.area_colors.program.set_bytes(
            self.area_colors.push,
            &AreaColorPush {
                first_sprite,
                sprite_count,
                group_count,
            },
        );
        self.area_colors.program.set(self.area_colors.groups, group_count);
        self.graph.execute_blocking(
            &self.device.context,
            &self.area_colors.program,
            &mut AllocatorKind::Persistent(&mut self.device.allocator),
        )?;

        Ok(())
    }

    fn write_descriptors(&self, textures: &[TextureImage]) -> Result<(), GpuError> {
        let fallback = self.fallback.as_ref().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
        let info = vk::DescriptorImageInfo::default()
            .sampler(self.sampler)
            .image_view(fallback.view)
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        let mut infos = vec![info; self.bindless.count as usize];

        for (info, texture) in infos.iter_mut().zip(textures) {
            info.image_view = texture.view;
        }

        self.bindless.write(&infos, &self.device);

        Ok(())
    }

    fn upload_images(&mut self, sources: &[TextureSource<'_>]) -> Result<Vec<TextureImage>, GpuError> {
        let mut textures = Vec::with_capacity(sources.len());
        let result = (|| {
            for (index, source) in sources.iter().enumerate() {
                let extent = vk::Extent2D {
                    width: source.width,
                    height: source.height,
                };
                let image = self.device.allocator.allocate_image(
                    &ImageInfo::texture(extent, vk::Format::R8G8B8A8_SRGB).with_name(format!("sprite texture {index}")),
                )?;
                let view = match self.device.allocator.allocate_image_view(
                    image.handle(),
                    vk::Format::R8G8B8A8_SRGB,
                    vk::ImageViewType::TYPE_2D,
                    image.subresource_range(),
                ) {
                    Ok(view) => view,
                    Err(error) => {
                        self.device.allocator.deallocate_image(image);

                        return Err(error.into());
                    },
                };

                textures.push(TextureImage { image, view });
            }

            let sizes = sources.iter().map(|source| source.pixels.len()).collect::<Vec<_>>();
            let ranges = batch_ranges(&sizes, UPLOAD_BATCH_BYTES).ok_or(GpuError::TextureUploadTooLarge)?;

            for range in ranges {
                let batch_sources = sources.get(range.clone()).ok_or(GpuError::TextureUploadTooLarge)?;
                let batch_textures = textures.get(range).ok_or(GpuError::TextureUploadTooLarge)?;
                upload_batch(&mut self.device, &mut self.graph, batch_sources, batch_textures)?;
            }

            Ok(())
        })();
        if let Err(error) = result {
            destroy_images(&mut self.device, textures);

            return Err(error);
        }

        Ok(textures)
    }

    fn upload_catalog(&mut self, catalog: &TextureCatalog) -> Result<Vec<TextureImage>, GpuError> {
        let dimension_limit = self.device.max_image_dimension_2d.min(u16::MAX.into());
        for texture in catalog.textures() {
            if texture.width() > dimension_limit || texture.height() > dimension_limit {
                return Err(GpuError::TextureSheetTooLarge {
                    path: texture.path().display().to_string(),
                    width: texture.width(),
                    height: texture.height(),
                    limit: dimension_limit,
                });
            }
        }

        let sizes = catalog
            .textures()
            .iter()
            .map(|texture| texture.decoded_bytes())
            .collect::<Vec<_>>();
        let ranges = batch_ranges(&sizes, UPLOAD_BATCH_BYTES).ok_or(GpuError::TextureUploadTooLarge)?;
        let mut textures = Vec::with_capacity(catalog.len());

        for range in ranges {
            let batch = catalog.textures().get(range).ok_or(GpuError::TextureUploadTooLarge)?;
            let files = match batch
                .iter()
                .map(|texture| {
                    let path = texture.path();
                    let file = IconFile::load(path).map_err(|error| GpuError::TextureLoad {
                        path: path.display().to_string(),
                        message: error.to_string(),
                    })?;
                    if file.sheet_width != texture.width() || file.sheet_height != texture.height() {
                        return Err(GpuError::TextureSourceChanged {
                            path: path.display().to_string(),
                            expected_width: texture.width(),
                            expected_height: texture.height(),
                            actual_width: file.sheet_width,
                            actual_height: file.sheet_height,
                        });
                    }

                    Ok(file)
                })
                .collect::<Result<Vec<_>, GpuError>>()
            {
                Ok(files) => files,
                Err(error) => {
                    destroy_images(&mut self.device, textures);
                    return Err(error);
                },
            };
            let sources = files
                .iter()
                .map(|file| TextureSource {
                    width: file.sheet_width,
                    height: file.sheet_height,
                    pixels: &file.pixels,
                })
                .collect::<Vec<_>>();
            match self.upload_images(&sources) {
                Ok(batch_textures) => textures.extend(batch_textures),
                Err(error) => {
                    destroy_images(&mut self.device, textures);
                    return Err(error);
                },
            }
        }

        Ok(textures)
    }
}

fn valid_update_range(range: Option<crate::UpdateRange>, len: usize) -> bool {
    range.is_none_or(|range| range.start <= range.end && range.end <= len)
}

fn patch_area_tile_indices(
    indices: &mut HashMap<dmm::PrefabInstanceId, u32>, uploaded_owners: &mut Vec<dmm::PrefabInstanceId>,
    area_tiles: &[SpriteInstance], range: crate::UpdateRange,
) -> Result<(), GpuError> {
    let old_len = uploaded_owners.len();
    for owner in uploaded_owners.iter().take(range.end.min(old_len)).skip(range.start) {
        indices.remove(owner);
    }
    for owner in uploaded_owners.iter().take(old_len).skip(area_tiles.len()) {
        indices.remove(owner);
    }

    uploaded_owners.truncate(area_tiles.len());
    if uploaded_owners.len() < area_tiles.len() {
        uploaded_owners.extend(area_tiles[uploaded_owners.len()..].iter().map(|area| area.owner));
    }
    for (index, (uploaded_owner, area)) in uploaded_owners
        .iter_mut()
        .zip(area_tiles)
        .enumerate()
        .take(range.end)
        .skip(range.start)
    {
        *uploaded_owner = area.owner;
        let index = u32::try_from(index).map_err(|_| GpuError::SpriteUploadTooLarge)?;
        indices.insert(area.owner, index);
    }

    Ok(())
}

fn gpu_sprite_offset(start: usize) -> Result<u64, GpuError> {
    start
        .checked_mul(size_of::<GpuSprite>())
        .and_then(|offset| u64::try_from(offset).ok())
        .ok_or(GpuError::SpriteUploadTooLarge)
}

fn gpu_sprite_range(sprites: &[SpriteInstance], range: crate::UpdateRange) -> Result<Vec<GpuSprite>, GpuError> {
    sprites[range.start..range.end]
        .iter()
        .enumerate()
        .map(|(offset, sprite)| gpu_sprite(range.start + offset, sprite))
        .collect()
}

fn gpu_light_tile(index: usize, tile: &crate::LightTile) -> Result<GpuLightTile, GpuError> {
    let mut corners = [0; 4];
    for (packed, corner) in corners.iter_mut().zip(tile.corners) {
        *packed = pack_unorm4x8([corner[0], corner[1], corner[2], 1.0]).ok_or(GpuError::SpritePackingOutOfRange {
            sprite: index,
            field: "light color",
        })?;
    }

    Ok(GpuLightTile { corners })
}

fn gpu_light_offset(start: usize) -> Result<u64, GpuError> {
    start
        .checked_mul(size_of::<GpuLightTile>())
        .and_then(|offset| u64::try_from(offset).ok())
        .ok_or(GpuError::SpriteUploadTooLarge)
}

fn gpu_sprite(index: usize, sprite: &SpriteInstance) -> Result<GpuSprite, GpuError> {
    let mut flags = if sprite.is_area {
        SPRITE_FLAG_AREA | ((sprite.area_edges & AREA_EDGES_ALL) << SPRITE_AREA_EDGE_SHIFT)
    } else {
        0
    };
    if sprite.is_area && sprite.area_edges == 0 {
        flags |= SPRITE_FLAG_FULLBRIGHT;
    }
    flags |= match sprite.lighting {
        crate::SpriteLighting::Normal => 0,
        crate::SpriteLighting::Emissive => SPRITE_FLAG_EMISSIVE_MASK,
        crate::SpriteLighting::Blocker => SPRITE_FLAG_EMISSIVE_BLOCKER,
        crate::SpriteLighting::OverlayLight => SPRITE_FLAG_OVERLAY_LIGHT,
        crate::SpriteLighting::OverlayLightSubtract => SPRITE_FLAG_OVERLAY_LIGHT_SUBTRACT,
    };

    let out_of_range = |field| GpuError::SpritePackingOutOfRange { sprite: index, field };
    if !sprite.x.is_finite() || !sprite.y.is_finite() {
        return Err(out_of_range("position"));
    }
    let position = [sprite.x, sprite.y];
    let size = pack_half2(sprite.width, sprite.height).ok_or_else(|| out_of_range("size"))?;
    let color = pack_unorm4x8(sprite.color).ok_or_else(|| out_of_range("color"))?;
    let source_position = pack_u16x2(sprite.texture.source_position).ok_or_else(|| out_of_range("source position"))?;
    let source_size =
        pack_u16x2([sprite.texture.width, sprite.texture.height]).ok_or_else(|| out_of_range("source size"))?;
    if sprite.texture.index > SPRITE_TEXTURE_MASK {
        return Err(out_of_range("texture index"));
    }

    Ok(GpuSprite {
        owner: owner_words(sprite.owner),
        area_owner: sprite.area_owner.map(owner_words).unwrap_or([0; 2]),
        position,
        z: sprite.z,
        packed_size: size,
        color,
        source_position_size: [source_position, source_size],
        texture_flags: sprite.texture.index | (flags << SPRITE_FLAGS_SHIFT),
    })
}

fn owner_words(owner: dmm::PrefabInstanceId) -> [u32; 2] { split_owner(owner.get()) }

fn split_owner(owner: u64) -> [u32; 2] { [owner as u32, (owner >> 32) as u32] }

fn project_guide_line(
    guide: crate::GuideLine, camera: crate::Camera, rect: crate::MapViewRect,
) -> ([f32; 2], [f32; 2]) {
    let logical = [
        camera.viewport_width.max(1) as f32,
        camera.viewport_height.max(1) as f32,
    ];
    let scale = [rect.width as f32 / logical[0], rect.height as f32 / logical[1]];
    let project = |point: [f32; 2]| {
        [
            ((point[0] - camera.x) * camera.zoom + logical[0] * 0.5) * scale[0],
            ((camera.y - point[1]) * camera.zoom + logical[1] * 0.5) * scale[1],
        ]
    };

    (project(guide.origin), project(guide.target))
}

/// Reinterprets a slice of tightly packed `repr(C)` shader records as the bytes to upload.
fn payload_bytes<T: Copy>(values: &[T]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), std::mem::size_of_val(values)) }
}

fn pack_half2(x: f32, y: f32) -> Option<u32> {
    let x = meshopt_quantize_half(x);
    let y = meshopt_quantize_half(y);

    ((x & 0x7c00) != 0x7c00 && (y & 0x7c00) != 0x7c00).then_some(u32::from(x) | (u32::from(y) << 16))
}

fn meshopt_quantize_half(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = (bits >> 16) & 0x8000;
    let exponent_mantissa = (bits & 0x7fff_ffff) as i32;

    let mut half = (exponent_mantissa - (112 << 23) + (1 << 12)) >> 13;
    half = if exponent_mantissa < (113 << 23) { 0 } else { half };
    half = if exponent_mantissa >= (143 << 23) { 0x7c00 } else { half };
    half = if exponent_mantissa > (255 << 23) { 0x7e00 } else { half };

    (sign | half as u32) as u16
}

#[cfg(test)]
fn meshopt_dequantize_half(half: u16) -> f32 {
    let sign = u32::from(half & 0x8000) << 16;
    let exponent_mantissa = u32::from(half & 0x7fff);

    let mut result = (exponent_mantissa + (112 << 10)) << 13;
    result = if exponent_mantissa < (1 << 10) { 0 } else { result };
    result += if exponent_mantissa >= (31 << 10) { 112 << 23 } else { 0 };

    f32::from_bits(sign | result)
}

fn pack_unorm4x8(values: [f32; 4]) -> Option<u32> {
    if !values
        .iter()
        .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
    {
        return None;
    }

    Some(
        meshopt_quantize_unorm(values[0], 8)
            | (meshopt_quantize_unorm(values[1], 8) << 8)
            | (meshopt_quantize_unorm(values[2], 8) << 16)
            | (meshopt_quantize_unorm(values[3], 8) << 24),
    )
}

fn meshopt_quantize_unorm(value: f32, bits: u32) -> u32 {
    debug_assert!((1..=16).contains(&bits));

    let scale = ((1 << bits) - 1) as f32;
    (value * scale + 0.5) as u32
}

#[cfg(test)]
fn meshopt_dequantize_unorm(value: u32, bits: u32) -> f32 {
    debug_assert!((1..=16).contains(&bits));

    value as f32 / ((1 << bits) - 1) as f32
}

fn pack_u16x2(values: [u32; 2]) -> Option<u32> {
    (values[0] <= u16::MAX.into() && values[1] <= u16::MAX.into()).then_some(values[0] | (values[1] << 16))
}

fn area_color_group_count(sprite_count: u32, limit: u32) -> u32 {
    if sprite_count == 0 {
        0
    } else {
        sprite_count.min(limit.max(1))
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        let _ = self.device.wait_idle();
        self.recorded = None;

        if let Some(imgui) = self.imgui.as_mut() {
            imgui.destroy(&mut self.device);
        }

        self.bindless.destroy(&self.device);
        destroy_images(&mut self.device, std::mem::take(&mut self.textures));
        destroy_images(&mut self.device, self.fallback.take());
        self.device.allocator.deallocate_sampler(self.sampler);

        for buffers in std::mem::take(&mut self.cull) {
            buffers.destroy(&mut self.device);
        }
        for preview in std::mem::take(&mut self.previews).into_iter().flatten() {
            preview.destroy(&mut self.device);
        }
        for guides in std::mem::take(&mut self.guides) {
            guides.destroy(&mut self.device);
        }
        for connected in std::mem::take(&mut self.connected) {
            connected.destroy(&mut self.device);
        }
        if let Some(buffer) = self.sprites.take() {
            self.device.allocator.deallocate_buffer(buffer);
        }
        if let Some(buffer) = self.area_tiles.take() {
            self.device.allocator.deallocate_buffer(buffer);
        }
        if let Some(buffer) = self.lights.take() {
            self.device.allocator.deallocate_buffer(buffer);
        }
        if let Some(buffer) = self.pick_readback.take() {
            self.device.allocator.deallocate_buffer(buffer);
        }
    }
}

fn level_ranges(sprite_instances: &[SpriteInstance]) -> Vec<LevelRange> {
    let mut ranges: Vec<LevelRange> = Vec::new();

    for (index, sprite) in sprite_instances.iter().enumerate() {
        if let Some(range) = ranges.last_mut()
            && range.z == sprite.z
        {
            range.count += 1;
        } else {
            ranges.push(LevelRange {
                z: sprite.z,
                base: index as u32,
                count: 1,
            });
        }
    }

    ranges
}

fn visible_range(ranges: &[LevelRange], active_z: u32, underlay_depth: u32) -> VisibleRange {
    let minimum_z = active_z.saturating_sub(underlay_depth).max(1);
    let mut underlays = (0, 0);
    let mut active = (0, 0);

    for range in ranges {
        if (minimum_z..active_z).contains(&range.z) {
            if underlays.1 == 0 {
                underlays.0 = range.base;
            }
            underlays.1 += range.count;
        } else if range.z == active_z {
            active = (range.base, range.count);
        }
    }

    VisibleRange {
        base: if underlays.1 == 0 { active.0 } else { underlays.0 },
        count: underlays.1 + active.1,
        underlay_count: underlays.1,
    }
}

fn destroy_images(device: &mut Device, textures: impl IntoIterator<Item = TextureImage>) {
    for texture in textures {
        device.allocator.deallocate_image_view(texture.view);
        device.allocator.deallocate_image(texture.image);
    }
}

fn descriptor_capacity(requested: usize, limit: u32) -> Result<u32, GpuError> {
    let requested = u32::try_from(requested).map_err(|_| GpuError::TextureUploadTooLarge)?;
    if requested > limit {
        return Err(GpuError::TooManyTextures { requested, limit });
    }

    Ok(requested.max(1))
}

struct BindlessDescriptorSet {
    layout: vk::DescriptorSetLayout,
    set: vk::DescriptorSet,
    pool: vk::DescriptorPool,
    count: u32,
}

impl BindlessDescriptorSet {
    fn create(device: &Device, count: u32) -> Result<Self, GpuError> {
        let vk_device = device.context.device();

        let binding = vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .descriptor_count(count)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT | vk::ShaderStageFlags::COMPUTE);
        let bindings = [binding];
        let layout_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
        let layout = unsafe { vk_device.create_descriptor_set_layout(&layout_info, None) }?;

        let pool_size = vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .descriptor_count(count);
        let pool_sizes = [pool_size];
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .max_sets(1)
            .pool_sizes(&pool_sizes);
        let pool = match unsafe { vk_device.create_descriptor_pool(&pool_info, None) } {
            Ok(pool) => pool,
            Err(error) => {
                unsafe { vk_device.destroy_descriptor_set_layout(layout, None) };
                return Err(error.into());
            },
        };

        let layouts = [layout];
        let allocate_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(pool)
            .set_layouts(&layouts);
        let set = match unsafe { vk_device.allocate_descriptor_sets(&allocate_info) } {
            Ok(sets) => match sets.first().copied() {
                Some(set) => set,
                None => {
                    unsafe {
                        vk_device.destroy_descriptor_pool(pool, None);
                        vk_device.destroy_descriptor_set_layout(layout, None);
                    }
                    return Err(vk::Result::ERROR_INITIALIZATION_FAILED.into());
                },
            },
            Err(error) => {
                unsafe {
                    vk_device.destroy_descriptor_pool(pool, None);
                    vk_device.destroy_descriptor_set_layout(layout, None);
                }
                return Err(error.into());
            },
        };

        Ok(Self {
            layout,
            set,
            pool,
            count,
        })
    }

    fn destroy(&self, device: &Device) {
        let vk_device = device.context.device();
        unsafe {
            vk_device.destroy_descriptor_pool(self.pool, None);
            vk_device.destroy_descriptor_set_layout(self.layout, None);
        }
    }

    fn write(&self, infos: &[vk::DescriptorImageInfo], device: &Device) {
        let vk_device = device.context.device();

        let write = vk::WriteDescriptorSet::default()
            .dst_set(self.set)
            .dst_binding(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .image_info(infos);
        unsafe { vk_device.update_descriptor_sets(std::slice::from_ref(&write), &[]) }
    }
}

fn upload_batch(
    device: &mut Device, graph: &mut RenderGraph, sources: &[TextureSource<'_>], textures: &[TextureImage],
) -> Result<(), GpuError> {
    let total = sources
        .iter()
        .try_fold(0usize, |total, source| total.checked_add(source.pixels.len()))
        .ok_or(GpuError::TextureUploadTooLarge)?;
    let total = u64::try_from(total).map_err(|_| GpuError::TextureUploadTooLarge)?;
    let mut staging = device
        .allocator
        .allocate_buffer(&BufferInfo::staging(total).with_name("sprite texture staging"))?;

    let result = (|| {
        let mut offset = 0u64;
        for source in sources {
            staging.write(offset, source.pixels)?;
            offset = offset
                .checked_add(u64::try_from(source.pixels.len()).map_err(|_| GpuError::TextureUploadTooLarge)?)
                .ok_or(GpuError::TextureUploadTooLarge)?;
        }

        let mut module = Module::default();
        let source_buffer = module.import_buffer(&staging, Access::HostWrite);
        let mut roots = Vec::with_capacity(textures.len());
        let mut offset = 0u64;

        for (source, texture) in sources.iter().zip(textures) {
            let destination_attachment =
                module.import_attachment(&ImageAttachment::from_image(&texture.image, vk::ImageLayout::UNDEFINED));
            let copied_attachment = module.copy_buffer_to_image_region(
                source_buffer,
                destination_attachment,
                copy_region(offset, source.width, source.height),
            );
            roots.push(module.release(
                copied_attachment,
                Access::FragmentSampled | Access::ComputeSampled,
                DomainFlag::Graphics,
            ));
            offset = offset
                .checked_add(u64::try_from(source.pixels.len()).map_err(|_| GpuError::TextureUploadTooLarge)?)
                .ok_or(GpuError::TextureUploadTooLarge)?;
        }

        let program = module.compile_all(&*graph, &roots)?;
        graph.execute_blocking(
            &device.context,
            &program,
            &mut AllocatorKind::Persistent(&mut device.allocator),
        )?;

        Ok(())
    })();

    if result.is_err() {
        let _ = device.wait_idle();
    }

    device.allocator.deallocate_buffer(staging);

    result
}

fn copy_region(buffer_offset: u64, width: u32, height: u32) -> BufferImageCopy {
    BufferImageCopy {
        buffer_offset,
        image_offset: vk::Offset3D { x: 0, y: 0, z: 0 },
        image_extent: vk::Extent3D {
            width,
            height,
            depth: 1,
        },
        mip_level: 0,
    }
}

fn batch_ranges(sizes: &[usize], budget: usize) -> Option<Vec<Range<usize>>> {
    if budget == 0 && !sizes.is_empty() {
        return None;
    }

    let mut ranges = Vec::new();
    let mut start = 0usize;
    let mut bytes = 0usize;

    for (index, size) in sizes.iter().copied().enumerate() {
        let next = bytes.checked_add(size)?;
        if index > start && next > budget {
            ranges.push(start..index);
            start = index;
            bytes = size;
        } else {
            bytes = next;
        }
    }

    if start < sizes.len() {
        ranges.push(start..sizes.len());
    }

    Some(ranges)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use ash::vk;
    use dmm::PrefabInstanceId;
    use vir::resource::shader;

    use super::{
        AREA_COLOR_CS_SPV,
        AreaColorPush,
        CULL_WORKGROUP_SIZE,
        CameraPush,
        CullPush,
        Frame,
        FrameGraphState,
        GEOMETRY_VS_SPV,
        GUIDE_FS_SPV,
        GpuGuideLine,
        GpuSprite,
        GuidePush,
        INTERACTION_FS_SPV,
        InteractionPush,
        LIGHTING_FS_SPV,
        LevelRange,
        LightingPush,
        PICK_CS_SPV,
        PickPush,
        SPRITE_CULL_CLASSIFY_CS_SPV,
        SPRITE_CULL_COMPACT_CS_SPV,
        SPRITE_CULL_SCAN_CS_SPV,
        SPRITE_FLAG_AREA,
        SPRITE_FLAG_EMISSIVE_BLOCKER,
        SPRITE_FLAG_EMISSIVE_MASK,
        SPRITE_FLAG_FULLBRIGHT,
        SPRITE_FLAG_OVERLAY_LIGHT,
        SPRITE_FLAG_OVERLAY_LIGHT_SUBTRACT,
        SPRITE_FLAGS_SHIFT,
        SPRITE_OVERLAY_LIGHT_FS_SPV,
        SPRITE_SHADE_FS_SPV,
        SPRITE_TEXTURE_MASK,
        SPRITE_VIS_FS_SPV,
        UPLOAD_BATCH_BYTES,
        VisibleRange,
        area_color_group_count,
        batch_ranges,
        copy_region,
        descriptor_capacity,
        gpu_light_tile,
        gpu_sprite,
        level_ranges,
        meshopt_dequantize_half,
        meshopt_dequantize_unorm,
        meshopt_quantize_half,
        pack_unorm4x8,
        patch_area_tile_indices,
        project_guide_line,
        spec,
        split_owner,
        valid_update_range,
        visible_range,
    };
    use crate::{
        AREA_EDGE_EAST,
        AREA_EDGE_NORTH,
        Camera,
        GpuError,
        GuideLine,
        HighlightStyle,
        InteractionMode,
        LightTile,
        LightingFrame,
        MapViewFrame,
        MapViewRect,
        SpriteInstance,
        SpriteLighting,
        SpriteTexture,
        UpdateRange,
        read_spirv,
    };

    fn owner() -> PrefabInstanceId { PrefabInstanceId::from_raw(1).expect("nonzero prefab instance ID") }

    fn sprite(z: u32) -> SpriteInstance {
        SpriteInstance {
            owner: owner(),
            area_owner: None,
            texture: SpriteTexture::default(),
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            z,
            is_area: false,
            area_edges: 0,
            lighting: SpriteLighting::Normal,
            color: [1.0; 4],
            depth: 0.0,
        }
    }

    fn owners(count: usize) -> Vec<PrefabInstanceId> {
        (1..=count)
            .map(|raw| PrefabInstanceId::from_raw(raw as u64).expect("nonzero prefab instance ID"))
            .collect()
    }

    fn map_view(rect: MapViewRect) -> MapViewFrame<'static> {
        MapViewFrame {
            rect,
            camera: Camera::default(),
            sprite_instances: &[],
            area_tiles: &[],
            focused_area: None,
            active_z: 1,
            level_count: 1,
            revision: 1,
            pending_update: None,
            lighting: None,
            guide_lines: &[],
            connected: &[],
            interaction: Default::default(),
            preview: None,
        }
    }

    fn map_view_with_sprites<'a>(rect: MapViewRect, sprites: &'a [SpriteInstance]) -> MapViewFrame<'a> {
        MapViewFrame {
            sprite_instances: sprites,
            ..map_view(rect)
        }
    }

    fn graph_state(target: vk::Extent2D, with_imgui: bool, map_views: &[MapViewFrame<'_>]) -> FrameGraphState {
        FrameGraphState::new(
            target,
            with_imgui,
            &Frame {
                map_views,
                underlay_depth: 0,
                show_areas: false,
                show_area_outlines: true,
                picking: None,
            },
        )
    }

    #[test]
    fn frame_graph_state_ignores_dynamic_frame_data() {
        let target = vk::Extent2D {
            width: 1280,
            height: 720,
        };
        let rect = MapViewRect {
            x: 100,
            y: 50,
            width: 640,
            height: 480,
        };
        let unchanged = [map_view(rect)];
        let mut dynamic = map_view(rect);
        dynamic.camera.x = 32.0;
        dynamic.camera.zoom = 2.0;
        dynamic.interaction.cursor = Some([120, 80]);
        dynamic.interaction.selected = Some(owner());
        let dynamic = [dynamic];

        assert_eq!(
            graph_state(target, true, &unchanged),
            graph_state(target, true, &dynamic),
        );
    }

    #[test]
    fn frame_graph_state_changes_with_the_layout() {
        let target = vk::Extent2D {
            width: 1280,
            height: 720,
        };
        let left = MapViewRect {
            x: 0,
            y: 0,
            width: 640,
            height: 720,
        };
        let right = MapViewRect {
            x: 640,
            y: 0,
            width: 640,
            height: 720,
        };
        let base = graph_state(target, true, &[map_view(left), map_view(right)]);

        let moved = MapViewRect { x: 32, ..left };
        assert_ne!(base, graph_state(target, true, &[map_view(moved), map_view(right)]));

        let resized = MapViewRect { width: 600, ..left };
        assert_ne!(base, graph_state(target, true, &[map_view(resized), map_view(right)]));
        assert_ne!(base, graph_state(target, true, &[map_view(left)]));
        assert_ne!(base, graph_state(target, true, &[map_view(right), map_view(left)]));
        assert_ne!(
            base,
            graph_state(
                vk::Extent2D {
                    width: 1920,
                    height: 1080,
                },
                true,
                &[map_view(left), map_view(right)],
            )
        );
        assert_ne!(base, graph_state(target, false, &[map_view(left), map_view(right)]));
    }

    /// Every toggle the shaders specialize on has to be part of the key, or a recording outlives
    /// the constants it was built with.
    #[test]
    fn frame_graph_state_changes_with_the_specialized_toggles() {
        let target = vk::Extent2D {
            width: 1280,
            height: 720,
        };
        let rect = MapViewRect {
            x: 0,
            y: 0,
            width: 1280,
            height: 720,
        };
        let views = [map_view(rect)];
        let frame = |show_areas, show_area_outlines| Frame {
            map_views: &views,
            underlay_depth: 0,
            show_areas,
            show_area_outlines,
            picking: None,
        };
        let base = FrameGraphState::new(target, true, &frame(false, true));

        assert_ne!(base, FrameGraphState::new(target, true, &frame(true, true)));
        assert_ne!(base, FrameGraphState::new(target, true, &frame(false, false)));

        let mut tinted = map_view(rect);
        tinted.interaction.highlight = HighlightStyle::Tint;
        assert_ne!(base, graph_state(target, true, &[tinted]));

        let mut deleting = map_view(rect);
        deleting.interaction.mode = InteractionMode::Delete { pick: None };
        assert_ne!(base, graph_state(target, true, &[deleting]));

        let tiles = [LightTile { corners: [[1.0; 3]; 4] }];
        let mut lit = map_view(rect);
        lit.lighting = Some(LightingFrame {
            size: [1, 1, 1],
            tiles: &tiles,
            tile_size: 32,
            revision: 1,
            pending_update: None,
        });
        assert_ne!(base, graph_state(target, true, &[lit]));
    }

    /// The cull buffers are renderer-owned and grown in place, so neither the sprite count nor the
    /// number of levels a view shows is allowed to force a recompile.
    #[test]
    fn frame_graph_state_ignores_sprite_counts_and_level_counts() {
        let target = vk::Extent2D {
            width: 1280,
            height: 720,
        };
        let rect = MapViewRect {
            width: 640,
            height: 480,
            ..Default::default()
        };
        let one_sprite = [sprite(1)];
        let two_sprites = [sprite(1), sprite(1)];
        let mut layered = map_view_with_sprites(rect, &two_sprites);
        layered.level_count = 2;

        let one = graph_state(target, true, &[map_view_with_sprites(rect, &one_sprite)]);
        let two = graph_state(target, true, &[layered]);

        assert_eq!(one, two);
    }

    #[test]
    fn empty_map_views_do_not_create_graph_passes() {
        let target = vk::Extent2D {
            width: 1280,
            height: 720,
        };
        let rect = MapViewRect {
            x: 0,
            y: 0,
            width: 640,
            height: 720,
        };
        let visible = graph_state(target, true, &[map_view(rect)]);
        let visible_with_empty = graph_state(target, true, &[map_view(rect), map_view(MapViewRect::default())]);
        let hidden = graph_state(target, true, &[map_view(MapViewRect::default())]);

        assert_eq!(visible, visible_with_empty);
        assert_ne!(visible, hidden);
        assert!(
            hidden.map_views.is_empty(),
            "an empty editor declares no map-view attachments"
        );
    }

    #[test]
    fn update_ranges_must_fit_their_current_buffer() {
        assert!(valid_update_range(None, 2));
        assert!(valid_update_range(Some(UpdateRange { start: 1, end: 2 }), 2));
        assert!(valid_update_range(Some(UpdateRange { start: 2, end: 2 }), 2));
        assert!(!valid_update_range(Some(UpdateRange { start: 2, end: 1 }), 2));
        assert!(!valid_update_range(Some(UpdateRange { start: 0, end: 3 }), 2));
    }

    #[test]
    fn area_owner_indices_follow_swapped_removals_and_appends() {
        let owners = owners(4);
        let mut tiles = owners[..3]
            .iter()
            .map(|owner| {
                let mut sprite = sprite(1);
                sprite.owner = *owner;
                sprite
            })
            .collect::<Vec<_>>();
        let mut uploaded = owners[..3].to_vec();
        let mut indices = uploaded
            .iter()
            .enumerate()
            .map(|(index, owner)| (*owner, index as u32))
            .collect::<HashMap<_, _>>();

        tiles.swap_remove(1);
        patch_area_tile_indices(&mut indices, &mut uploaded, &tiles, UpdateRange { start: 1, end: 2 }).unwrap();
        assert_eq!(uploaded, vec![owners[0], owners[2]]);
        assert_eq!(indices.get(&owners[0]), Some(&0));
        assert_eq!(indices.get(&owners[1]), None);
        assert_eq!(indices.get(&owners[2]), Some(&1));

        let mut appended = sprite(1);
        appended.owner = owners[3];
        tiles.push(appended);
        patch_area_tile_indices(&mut indices, &mut uploaded, &tiles, UpdateRange { start: 2, end: 3 }).unwrap();
        assert_eq!(uploaded, vec![owners[0], owners[2], owners[3]]);
        assert_eq!(indices.get(&owners[3]), Some(&2));

        tiles.pop();
        patch_area_tile_indices(&mut indices, &mut uploaded, &tiles, UpdateRange { start: 2, end: 2 }).unwrap();
        assert_eq!(uploaded, vec![owners[0], owners[2]]);
        assert_eq!(indices.get(&owners[3]), None);
    }

    #[test]
    fn area_edges_and_tile_geometry_reach_the_gpu() {
        let mut area = sprite(37);
        area.area_owner = PrefabInstanceId::from_raw(0x1234_5678_9abc_def0);
        area.x = 32.0;
        area.y = 64.0;
        area.width = 32.0;
        area.height = 32.0;
        area.is_area = true;
        area.area_edges = AREA_EDGE_NORTH | AREA_EDGE_EAST;
        area.texture = SpriteTexture {
            index: 3,
            source_position: [4, 8],
            width: 16,
            height: 24,
        };
        area.color = [0.0; 4];

        let gpu = gpu_sprite(0, &area).expect("pack");

        assert_eq!(gpu.owner, [area.owner.get() as u32, 0]);
        assert_eq!(gpu.area_owner, [0x9abc_def0, 0x1234_5678]);
        assert_eq!(gpu.position, [32.0, 64.0]);
        assert_eq!(gpu.z, 37);
        assert_eq!(gpu.packed_size, 0x5000_5000);
        assert_eq!(gpu.color, 0);
        assert_eq!(gpu.source_position_size, [0x0008_0004, 0x0018_0010]);
        assert_eq!(gpu.texture_flags & SPRITE_TEXTURE_MASK, 3);
        assert_eq!(
            gpu.texture_flags >> SPRITE_FLAGS_SHIFT,
            1 | ((AREA_EDGE_NORTH | AREA_EDGE_EAST) << 1)
        );
    }

    #[test]
    fn filled_area_sprites_are_fullbright_without_outline_flags() {
        let mut area = sprite(1);
        area.is_area = true;

        let gpu = gpu_sprite(0, &area).expect("pack");

        assert_eq!(
            gpu.texture_flags >> SPRITE_FLAGS_SHIFT,
            SPRITE_FLAG_AREA | SPRITE_FLAG_FULLBRIGHT
        );
    }

    #[test]
    fn lighting_roles_reach_the_gpu_flags() {
        let mut emissive = sprite(1);
        emissive.lighting = SpriteLighting::Emissive;
        let mut blocker = sprite(1);
        blocker.lighting = SpriteLighting::Blocker;
        let mut overlay = sprite(1);
        overlay.lighting = SpriteLighting::OverlayLight;
        let mut subtract = sprite(1);
        subtract.lighting = SpriteLighting::OverlayLightSubtract;

        let emissive = gpu_sprite(0, &emissive).expect("pack emissive");
        let blocker = gpu_sprite(1, &blocker).expect("pack blocker");
        let overlay = gpu_sprite(2, &overlay).expect("pack overlay light");
        let subtract = gpu_sprite(3, &subtract).expect("pack subtractive overlay light");

        assert_eq!(emissive.texture_flags >> SPRITE_FLAGS_SHIFT, SPRITE_FLAG_EMISSIVE_MASK);
        assert_ne!(SPRITE_FLAG_EMISSIVE_MASK, SPRITE_FLAG_FULLBRIGHT);
        assert_eq!(
            blocker.texture_flags >> SPRITE_FLAGS_SHIFT,
            SPRITE_FLAG_EMISSIVE_BLOCKER
        );
        assert_eq!(overlay.texture_flags >> SPRITE_FLAGS_SHIFT, SPRITE_FLAG_OVERLAY_LIGHT);
        assert_eq!(
            subtract.texture_flags >> SPRITE_FLAGS_SHIFT,
            SPRITE_FLAG_OVERLAY_LIGHT_SUBTRACT
        );
    }

    #[test]
    fn meshopt_half_conversion_matches_reference_values() {
        assert_eq!(meshopt_quantize_half(0.0), 0x0000);
        assert_eq!(meshopt_quantize_half(-0.0), 0x8000);
        assert_eq!(meshopt_quantize_half(1.0), 0x3c00);
        assert_eq!(meshopt_quantize_half(-2.0), 0xc000);
        assert_eq!(meshopt_quantize_half(65_504.0), 0x7bff);
        assert_eq!(meshopt_quantize_half(f32::INFINITY), 0x7c00);
        assert_eq!(meshopt_quantize_half(f32::NAN), 0x7e00);
        assert_eq!(meshopt_quantize_half(f32::MIN_POSITIVE), 0x0000);

        assert_eq!(meshopt_dequantize_half(0x0000).to_bits(), 0.0f32.to_bits());
        assert_eq!(meshopt_dequantize_half(0x8000).to_bits(), (-0.0f32).to_bits());
        assert_eq!(meshopt_dequantize_half(0x3c00), 1.0);
        assert_eq!(meshopt_dequantize_half(0xc000), -2.0);
        assert_eq!(meshopt_dequantize_half(0x7bff), 65_504.0);
        assert!(meshopt_dequantize_half(0x7c00).is_infinite());
        assert!(meshopt_dequantize_half(0x7e00).is_nan());
    }

    #[test]
    fn color_uses_rgba_unorm8_order() {
        let packed = pack_unorm4x8([0.0, 0.5, 1.0, 128.0 / 255.0]).expect("pack");

        assert_eq!(packed, 0x80ff_8000);
        assert_eq!(meshopt_dequantize_unorm((packed >> 8) & 0xff, 8), 128.0 / 255.0);
        assert_eq!(meshopt_dequantize_unorm((packed >> 16) & 0xff, 8), 1.0);
    }

    #[test]
    fn light_tiles_use_four_rgb_unorm8_corners() {
        let tile = LightTile {
            corners: [[1.0, 0.0, 0.0], [0.0, 0.5, 0.0], [0.0, 0.0, 1.0], [0.25; 3]],
        };
        let packed = gpu_light_tile(0, &tile).expect("pack light tile");

        assert_eq!(size_of_val(&packed), 16);
        assert_eq!(packed.corners[0], 0xff00_00ff);
        assert_eq!(packed.corners[1], 0xff00_8000);
        assert_eq!(packed.corners[2], 0xffff_0000);
    }

    #[test]
    fn sprite_packing_rejects_values_outside_the_wire_format() {
        let mut instance = sprite(1);
        instance.x = 70_000.0;
        assert_eq!(
            gpu_sprite(7, &instance).expect("f32 position").position,
            [70_000.0, 0.0]
        );

        instance.x = f32::NAN;
        assert_eq!(
            gpu_sprite(7, &instance),
            Err(GpuError::SpritePackingOutOfRange {
                sprite: 7,
                field: "position"
            })
        );

        instance.x = 0.0;
        instance.width = 70_000.0;
        assert_eq!(
            gpu_sprite(7, &instance),
            Err(GpuError::SpritePackingOutOfRange {
                sprite: 7,
                field: "size"
            })
        );

        instance.width = 0.0;
        instance.color[0] = f32::NAN;
        assert_eq!(
            gpu_sprite(7, &instance),
            Err(GpuError::SpritePackingOutOfRange {
                sprite: 7,
                field: "color"
            })
        );

        instance.color = [1.0; 4];
        instance.texture.source_position = [u16::MAX as u32 + 1, 0];
        assert_eq!(
            gpu_sprite(7, &instance),
            Err(GpuError::SpritePackingOutOfRange {
                sprite: 7,
                field: "source position"
            })
        );

        instance.texture.source_position = [0, 0];
        instance.texture.width = u16::MAX as u32 + 1;
        assert_eq!(
            gpu_sprite(7, &instance),
            Err(GpuError::SpritePackingOutOfRange {
                sprite: 7,
                field: "source size"
            })
        );

        instance.texture.width = 0;
        instance.texture.index = SPRITE_TEXTURE_MASK + 1;
        assert_eq!(
            gpu_sprite(7, &instance),
            Err(GpuError::SpritePackingOutOfRange {
                sprite: 7,
                field: "texture index"
            })
        );
    }

    #[test]
    fn sprite_positions_keep_single_pixel_precision_at_large_coordinates() {
        let mut instance = sprite(1);
        instance.x = 2_080.0;
        instance.y = -2_080.0;
        let original = gpu_sprite(0, &instance).expect("pack");

        instance.x -= 1.0;
        instance.y += 1.0;
        let shifted = gpu_sprite(0, &instance).expect("pack");

        assert_eq!(original.position, [2_080.0, -2_080.0]);
        assert_eq!(shifted.position, [2_079.0, -2_079.0]);
        assert_ne!(original.position, shifted.position);
    }

    #[test]
    fn adjacent_instances_are_grouped_into_level_ranges() {
        let sprites = [sprite(1), sprite(1), sprite(2), sprite(4), sprite(4), sprite(4)];

        assert_eq!(
            level_ranges(&sprites),
            [
                LevelRange {
                    z: 1,
                    base: 0,
                    count: 2
                },
                LevelRange {
                    z: 2,
                    base: 2,
                    count: 1
                },
                LevelRange {
                    z: 4,
                    base: 3,
                    count: 3
                },
            ]
        );
    }

    #[test]
    fn visible_ranges_select_only_the_active_level_and_configured_underlays() {
        let ranges = [
            LevelRange {
                z: 1,
                base: 0,
                count: 2,
            },
            LevelRange {
                z: 2,
                base: 2,
                count: 3,
            },
            LevelRange {
                z: 3,
                base: 5,
                count: 4,
            },
            LevelRange {
                z: 4,
                base: 9,
                count: 5,
            },
        ];

        assert_eq!(
            visible_range(&ranges, 3, 2),
            VisibleRange {
                base: 0,
                count: 9,
                underlay_count: 5,
            }
        );
    }

    /// The active level leads the merged range whenever nothing underneath it is shown, so the
    /// cull's `active_first_relative` is zero rather than the active level's own base.
    #[test]
    fn a_view_without_underlays_starts_at_the_active_level() {
        let ranges = [
            LevelRange {
                z: 1,
                base: 0,
                count: 2,
            },
            LevelRange {
                z: 2,
                base: 2,
                count: 3,
            },
        ];

        assert_eq!(
            visible_range(&ranges, 2, 0),
            VisibleRange {
                base: 2,
                count: 3,
                underlay_count: 0,
            }
        );
        assert_eq!(visible_range(&ranges, 9, 2), VisibleRange::default());
    }

    #[test]
    fn underlay_depth_clamps_at_the_bottom_and_can_be_disabled() {
        let ranges = [
            LevelRange {
                z: 1,
                base: 0,
                count: 2,
            },
            LevelRange {
                z: 2,
                base: 2,
                count: 3,
            },
        ];

        assert_eq!(
            visible_range(&ranges, 2, 9),
            VisibleRange {
                base: 0,
                count: 5,
                underlay_count: 2,
            }
        );
        assert_eq!(
            visible_range(&ranges, 2, 0),
            VisibleRange {
                base: 2,
                count: 3,
                underlay_count: 0,
            }
        );
    }

    #[test]
    fn sprite_and_camera_layouts_match_the_shader_scalar_layout() {
        let reflection = shader::reflect(&read_spirv(GEOMETRY_VS_SPV).expect("valid SPIR-V")).expect("shader reflects");

        assert_eq!(size_of::<GpuSprite>(), 48);
        assert_eq!(reflection.push_constant_offset, 0);
        assert_eq!(reflection.push_constant_size as usize, size_of::<CameraPush>());
    }

    #[test]
    fn lighting_shader_layout_matches_the_renderer() {
        let reflection = shader::reflect(&read_spirv(LIGHTING_FS_SPV).expect("valid SPIR-V")).expect("shader reflects");
        let mut bindings = reflection
            .bindings
            .iter()
            .map(|binding| (binding.set, binding.binding))
            .collect::<Vec<_>>();
        bindings.sort_unstable();

        assert_eq!(bindings, [(0, 0), (0, 1), (0, 2), (0, 3), (0, 4)]);
        assert_eq!(reflection.push_constant_offset, 0);
        assert_eq!(reflection.push_constant_size as usize, size_of::<LightingPush>());
    }

    #[test]
    fn sprite_vertex_shader_reads_sprites_and_the_compacted_indices() {
        let reflection = shader::reflect(&read_spirv(GEOMETRY_VS_SPV).expect("valid SPIR-V")).expect("shader reflects");
        let mut bindings = reflection
            .bindings
            .iter()
            .map(|binding| (binding.set, binding.binding, binding.access))
            .collect::<Vec<_>>();
        bindings.sort_unstable_by_key(|binding| (binding.0, binding.1));

        assert_eq!(reflection.push_constant_offset, 0);
        assert_eq!(reflection.push_constant_size as usize, size_of::<CameraPush>());
        assert_eq!(bindings.len(), 2);
        assert_eq!((bindings[0].0, bindings[0].1), (0, 1));
        assert_eq!((bindings[1].0, bindings[1].1), (0, 2));
        assert!(bindings[0].2.contains(vir::Access::VertexRead));
        assert!(bindings[1].2.contains(vir::Access::VertexRead));
    }

    #[test]
    fn sprite_cull_compute_layouts_match_the_host() {
        let stages = [
            (
                SPRITE_CULL_CLASSIFY_CS_SPV,
                [CULL_WORKGROUP_SIZE, 1, 1],
                vec![(1, vir::Access::ComputeRead), (4, vir::Access::ComputeWrite)],
            ),
            (
                SPRITE_CULL_SCAN_CS_SPV,
                [256, 1, 1],
                vec![(3, vir::Access::ComputeWrite), (4, vir::Access::ComputeRW)],
            ),
            (
                SPRITE_CULL_COMPACT_CS_SPV,
                [CULL_WORKGROUP_SIZE, 1, 1],
                vec![
                    (1, vir::Access::ComputeRead),
                    (2, vir::Access::ComputeWrite),
                    (4, vir::Access::ComputeRead),
                ],
            ),
        ];

        for (spirv, local_size, expected_bindings) in stages {
            let reflection = shader::reflect(&read_spirv(spirv).expect("valid SPIR-V")).expect("shader reflects");
            let mut bindings = reflection
                .bindings
                .iter()
                .map(|binding| (binding.binding, binding.access))
                .collect::<Vec<_>>();
            bindings.sort_unstable_by_key(|binding| binding.0);

            assert_eq!(reflection.local_size, local_size);
            assert_eq!(reflection.push_constant_offset, 0);
            assert_eq!(reflection.push_constant_size as usize, size_of::<CullPush>());
            assert_eq!(bindings, expected_bindings);
        }
    }

    #[test]
    fn owner_ids_keep_both_words() {
        assert_eq!(split_owner(0x1234_5678_9abc_def0), [0x9abc_def0, 0x1234_5678]);
    }

    /// The shader computes `clip = (map - center) * zoom / (viewport * 0.5)`.
    /// `zoom` is logical pixels per map unit, matching the camera maths the UI
    /// uses for overlays, so the projection must divide by the camera's logical
    /// size — not by the physical rectangle it lands in.
    fn projected_tile_size(zoom: f32, pushed_viewport: f32, rect_width: f32) -> f32 {
        // One map unit, in NDC, scaled onto the physical rectangle.
        let ndc = zoom / (pushed_viewport * 0.5);

        ndc * rect_width * 0.5
    }

    #[test]
    fn a_map_unit_covers_the_same_screen_space_at_any_display_scale() {
        let zoom = 4.0;
        let logical = 800.0;

        // At 1x the logical and physical rectangles agree.
        assert_eq!(projected_tile_size(zoom, logical, 800.0), 4.0);

        // At 2x the rectangle doubles. Projecting against the logical size
        // keeps the map the same on-screen size, just at twice the resolution.
        assert_eq!(projected_tile_size(zoom, logical, 1600.0), 8.0);

        // Projecting against the physical size instead halves the map relative
        // to every ImGui overlay, which is the 2x mismatch this guards.
        assert_eq!(projected_tile_size(zoom, 1600.0, 1600.0), 4.0);
    }

    #[test]
    fn guide_lines_project_into_map_view_coordinates() {
        let guide = GuideLine {
            origin: [110.0, 40.0],
            target: [130.0, 60.0],
        };
        let camera = Camera {
            x: 100.0,
            y: 50.0,
            zoom: 2.0,
            viewport_width: 800,
            viewport_height: 600,
        };

        let full = crate::MapViewRect {
            x: 0,
            y: 0,
            width: 800,
            height: 600,
        };
        assert_eq!(
            project_guide_line(guide, camera, full),
            ([420.0, 320.0], [460.0, 280.0]),
        );

        // Moving the ImGui window does not move coordinates inside its private
        // render attachment.
        let right = crate::MapViewRect {
            x: 800,
            y: 100,
            width: 800,
            height: 600,
        };
        assert_eq!(
            project_guide_line(guide, camera, right),
            ([420.0, 320.0], [460.0, 280.0]),
        );

        let scaled = crate::MapViewRect {
            x: 0,
            y: 0,
            width: 1600,
            height: 1200,
        };
        assert_eq!(
            project_guide_line(guide, camera, scaled),
            ([840.0, 640.0], [920.0, 560.0]),
        );

        let panned = Camera {
            x: 110.0,
            y: 60.0,
            ..camera
        };
        assert_eq!(
            project_guide_line(guide, panned, scaled),
            ([800.0, 680.0], [880.0, 600.0]),
        );
    }

    #[test]
    fn interaction_fragment_layout_matches_the_host() {
        let reflection =
            shader::reflect(&read_spirv(INTERACTION_FS_SPV).expect("valid SPIR-V")).expect("shader reflects");
        let mut bindings = reflection
            .bindings
            .iter()
            .map(|binding| (binding.set, binding.binding))
            .collect::<Vec<_>>();
        bindings.sort_unstable();

        assert_eq!(bindings, [(0, 0), (0, 1), (0, 2), (0, 4)]);
        assert_eq!(reflection.push_constant_offset, 0);
        assert_eq!(reflection.push_constant_size as usize, size_of::<InteractionPush>());
    }

    #[test]
    fn guide_fragment_layout_matches_the_host() {
        let reflection = shader::reflect(&read_spirv(GUIDE_FS_SPV).expect("valid SPIR-V")).expect("shader reflects");
        let mut bindings = reflection
            .bindings
            .iter()
            .map(|binding| (binding.set, binding.binding))
            .collect::<Vec<_>>();
        bindings.sort_unstable();

        assert_eq!(bindings, [(0, 0), (0, 1), (0, 2)]);
        assert_eq!(reflection.push_constant_offset, 0);
        assert_eq!(reflection.push_constant_size as usize, size_of::<GuidePush>());
        assert_eq!(size_of::<GpuGuideLine>(), 16);
    }

    #[test]
    fn pick_compute_layout_matches_the_host() {
        let reflection = shader::reflect(&read_spirv(PICK_CS_SPV).expect("valid SPIR-V")).expect("shader reflects");
        let mut bindings = reflection
            .bindings
            .iter()
            .map(|binding| (binding.set, binding.binding, binding.access))
            .collect::<Vec<_>>();
        bindings.sort_unstable_by_key(|binding| (binding.0, binding.1));

        assert_eq!(reflection.local_size, [1, 1, 1]);
        assert_eq!(reflection.push_constant_offset, 0);
        assert_eq!(reflection.push_constant_size as usize, size_of::<PickPush>());
        assert_eq!(bindings.len(), 2);
        assert_eq!((bindings[0].0, bindings[0].1), (0, 1));
        assert_eq!((bindings[1].0, bindings[1].1), (0, 3));
        assert!(bindings[0].2.contains(vir::Access::ComputeSampled));
        assert!(bindings[1].2.contains(vir::Access::ComputeWrite));
    }

    #[test]
    fn area_color_compute_layout_matches_the_host() {
        let reflection =
            shader::reflect(&read_spirv(AREA_COLOR_CS_SPV).expect("valid SPIR-V")).expect("shader reflects");
        let mut bindings = reflection
            .bindings
            .iter()
            .map(|binding| (binding.set, binding.binding, binding.access))
            .collect::<Vec<_>>();
        bindings.sort_unstable_by_key(|binding| (binding.0, binding.1));

        assert_eq!(reflection.local_size, [64, 1, 1]);
        assert_eq!(reflection.push_constant_offset, 0);
        assert_eq!(reflection.push_constant_size as usize, size_of::<AreaColorPush>());
        assert_eq!(bindings.len(), 2);
        assert_eq!((bindings[0].0, bindings[0].1), (0, 1));
        assert_eq!((bindings[1].0, bindings[1].1), (1, 0));
        assert!(bindings[0].2.contains(vir::Access::ComputeRW));
        assert!(bindings[1].2.contains(vir::Access::ComputeSampled));
    }

    #[test]
    fn area_color_dispatch_stays_within_the_device_limit() {
        assert_eq!(area_color_group_count(0, 64), 0);
        assert_eq!(area_color_group_count(32, 64), 32);
        assert_eq!(area_color_group_count(65, 64), 64);
        assert_eq!(area_color_group_count(1, 0), 1);
    }

    #[test]
    fn sprite_fragment_passes_read_sprites_and_textures() {
        for (spirv, expected) in [
            (SPRITE_VIS_FS_SPV, vec![(0, 1), (1, 0)]),
            (SPRITE_SHADE_FS_SPV, vec![(0, 1), (1, 0)]),
            (SPRITE_OVERLAY_LIGHT_FS_SPV, vec![(0, 1), (0, 3), (1, 0)]),
        ] {
            let reflection = shader::reflect(&read_spirv(spirv).expect("valid SPIR-V")).expect("shader reflects");
            let mut bindings = reflection
                .bindings
                .iter()
                .map(|binding| (binding.set, binding.binding))
                .collect::<Vec<_>>();
            bindings.sort_unstable();

            assert_eq!(bindings, expected);
        }
    }

    #[test]
    fn shader_stages_declare_the_constants_the_host_specializes() {
        let stages = [
            (GEOMETRY_VS_SPV, spec::SHOW_AREAS),
            (GEOMETRY_VS_SPV, spec::SHOW_AREA_OUTLINES),
            (GEOMETRY_VS_SPV, spec::SPRITE_INDIRECT),
            (SPRITE_SHADE_FS_SPV, spec::SPRITE_EDGE_ONLY),
            (SPRITE_VIS_FS_SPV, spec::SPRITE_WRITES_ID),
            (SPRITE_CULL_CLASSIFY_CS_SPV, spec::SHOW_AREAS),
            (SPRITE_CULL_CLASSIFY_CS_SPV, spec::SHOW_AREA_OUTLINES),
            (SPRITE_CULL_COMPACT_CS_SPV, spec::SHOW_AREAS),
            (SPRITE_CULL_COMPACT_CS_SPV, spec::SHOW_AREA_OUTLINES),
            (INTERACTION_FS_SPV, spec::HIGHLIGHT_TINT),
            (INTERACTION_FS_SPV, spec::DELETE_MODE),
        ];

        for (spirv, id) in stages {
            let reflection = shader::reflect(&read_spirv(spirv).expect("valid SPIR-V")).expect("shader reflects");
            let declared = reflection
                .spec_constants
                .iter()
                .map(|constant| constant.id)
                .collect::<Vec<_>>();

            assert!(
                declared.contains(&id),
                "stage does not declare specialization constant {id}"
            );
        }
    }

    #[test]
    fn copies_are_always_two_dimensional() {
        let region = copy_region(128, 32, 48);

        assert_eq!(region.buffer_offset, 128);
        assert_eq!(
            (region.image_offset.x, region.image_offset.y, region.image_offset.z),
            (0, 0, 0)
        );
        assert_eq!(
            (
                region.image_extent.width,
                region.image_extent.height,
                region.image_extent.depth
            ),
            (32, 48, 1)
        );
    }

    #[test]
    fn large_uploads_are_split_at_texture_boundaries() {
        let half = UPLOAD_BATCH_BYTES / 2;
        let ranges = batch_ranges(&[half, half, 4], UPLOAD_BATCH_BYTES).expect("valid ranges");

        assert_eq!(ranges, vec![0..2, 2..3]);
    }

    #[test]
    fn one_oversized_texture_gets_its_own_batch() {
        let ranges = batch_ranges(&[UPLOAD_BATCH_BYTES + 4, 4], UPLOAD_BATCH_BYTES).expect("valid ranges");

        assert_eq!(ranges, vec![0..1, 1..2]);
    }

    #[test]
    fn descriptor_capacity_matches_the_catalog_and_keeps_one_fallback_slot() {
        assert_eq!(descriptor_capacity(8, 10), Ok(8));
        assert_eq!(descriptor_capacity(0, 10), Ok(1));
        assert_eq!(
            descriptor_capacity(11, 10),
            Err(crate::GpuError::TooManyTextures {
                requested: 11,
                limit: 10,
            })
        );
    }
}
