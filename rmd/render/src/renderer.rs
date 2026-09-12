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
    DomainFlag,
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
    VisibilityId,
    extent3d,
    imgui::{ImGuiPass, ImGuiSlots},
    read_spirv,
    texture::TextureCatalog,
};

const GEOMETRY_VS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sprite.vert.spv"));
const SPRITE_SHADE_FS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sprite.frag.spv"));
const SPRITE_VIS_FS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sprite_visibility.frag.spv"));
const SPRITE_OUTLINE_FS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sprite_outline.frag.spv"));
const AREA_COLOR_CS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/area_color.comp.spv"));
const BLUR_VS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/blur.vert.spv"));
const BLUR_FS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/blur.frag.spv"));
const INTERACTION_VS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/interaction.vert.spv"));
const INTERACTION_FS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/interaction.frag.spv"));
const PICK_CS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/pick.comp.spv"));
const UPLOAD_BATCH_BYTES: usize = 64 * 1024 * 1024;
const HIGHLIGHT_STRIPE_PERIOD: f32 = 12.0;
const HIGHLIGHT_STRIPE_SPEED: f32 = 12.0;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
struct GpuSprite {
    owner: [u32; 2],
    area_owner: [u32; 2],
    position: [f32; 2],
    packed_size: u32,
    color: u32,
    source_position_size: [u32; 2],
    texture_flags: u32,
}

const SPRITE_FLAG_AREA: u32 = 1;
const SPRITE_AREA_EDGE_SHIFT: u32 = 1;
const SPRITE_FLAGS_SHIFT: u32 = 27;
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
    show_areas: u32,
    show_area_outlines: u32,
    focused_area_owner: [u32; 2],
    placement_flash_owner: [u32; 2],
    placement_flash_strength: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct BlurPush {
    sample_step: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct InteractionPush {
    cursor: [u32; 2],
    cursor_valid: u32,
    selected_owner: [u32; 2],
    stripe_offset: f32,
    guide_origin: [f32; 2],
    guide_target: [f32; 2],
    guide_valid: u32,
    interaction_mode: u32,
    focused_area_owner: [u32; 2],
    highlight_tint: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PickPush {
    cursor: [u32; 2],
    cursor_valid: u32,
}

struct InteractionSlots {
    push: ValueId,
    pick_push: ValueId,
    area_tiles: ValueId,
    hover_draw: ValueId,
    highlight_draw: ValueId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FrameGraphState {
    extent: [u32; 2],
    with_imgui: bool,
    map_views: Vec<MapViewGraphState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MapViewGraphState {
    rect: crate::MapViewRect,
    has_underlays: bool,
}

impl FrameGraphState {
    fn new(viewport: vk::Extent2D, with_imgui: bool, map_views: &[crate::MapViewFrame<'_>]) -> Self {
        Self {
            extent: [viewport.width, viewport.height],
            with_imgui,
            map_views: map_views
                .iter()
                .filter(|view| !view.rect.is_empty())
                .map(|view| MapViewGraphState {
                    rect: view.rect,
                    has_underlays: view.level_count > 1,
                })
                .collect(),
        }
    }
}

struct Recorded {
    program: Program,
    sprites: ValueId,
    map_views: Vec<MapViewPass>,
    pick_result: Option<ValueId>,
    state: FrameGraphState,
    ui: Option<ImGuiSlots>,
}

struct MapViewPass {
    scene_attachment: ValueId,
    visibility_attachment: ValueId,
    underlays_attachment: ValueId,
    output_attachment: ValueId,
    extent: ValueId,
    underlay_extent: ValueId,
    camera: ValueId,
    blur_push: ValueId,
    blur_draw: ValueId,
    underlay_draw: Option<ValueId>,
    active_draw: ValueId,
    outline_draw: ValueId,
    should_pick: Option<ValueId>,
    interaction: Option<InteractionSlots>,
}

struct AreaColorPass {
    program: Program,
    sprites: ValueId,
    push: ValueId,
    groups: ValueId,
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

#[derive(Debug, Clone)]
struct MapViewPlan {
    camera: CameraPush,
    blur: BlurPush,
    sprite_base: u32,
    underlay_ranges: Vec<(u32, u32)>,
    active_range: (u32, u32),
}

impl MapViewPlan {
    fn bind_scene(&self, cmd: &mut vir::Recorder<'_>) {
        cmd.set_viewport(0, Rect2D::framebuffer())
            .set_scissor(0, Rect2D::framebuffer())
            .push_constants(&self.camera);
    }
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

pub struct Renderer {
    graph: RenderGraph,
    recorded: Option<Recorded>,
    frames: Option<SuperFrameAllocator>,
    swapchain: Option<SwapChain>,
    sprite_pipeline: PipelineId,
    visibility_pipeline: PipelineId,
    outline_pipeline: PipelineId,
    blur_pipeline: PipelineId,
    interaction_pipeline: PipelineId,
    pick_pipeline: PipelineId,
    area_colors: AreaColorPass,
    bindless: BindlessDescriptorSet,
    textures: Vec<TextureImage>,
    fallback: Option<TextureImage>,
    sampler: vk::Sampler,
    blur_sampler: vk::Sampler,
    sprites: Option<Buffer>,
    sprite_capacity: usize,
    area_tiles: Option<Buffer>,
    area_tile_capacity: usize,
    pick_readback: Option<Buffer>,
    uploaded: Vec<UploadedMapView>,
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
        let outline_pipeline = match graph.declare_pipeline(
            GraphicsPipelineInfo::new()
                .with_shader(&geometry_vs)
                .with_shader(&read_spirv(SPRITE_OUTLINE_FS_SPV)?)
                .with_bindless_set(1, bindless.layout, bindless.set),
        ) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                drop(graph);
                bindless.destroy(&device);

                return Err(error.into());
            },
        };
        let blur_pipeline = match graph.declare_pipeline(
            GraphicsPipelineInfo::new()
                .with_shader(&read_spirv(BLUR_VS_SPV)?)
                .with_shader(&read_spirv(BLUR_FS_SPV)?),
        ) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                drop(graph);
                bindless.destroy(&device);

                return Err(error.into());
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
            outline_pipeline,
            blur_pipeline,
            interaction_pipeline,
            pick_pipeline,
            area_colors,
            bindless,
            textures: Vec::new(),
            fallback: None,
            sampler: vk::Sampler::null(),
            blur_sampler: vk::Sampler::null(),
            sprites: None,
            sprite_capacity: 0,
            area_tiles: None,
            area_tile_capacity: 0,
            pick_readback: None,
            uploaded: Vec::new(),
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
        renderer.blur_sampler = renderer.device.allocator.allocate_sampler(&SamplerInfo::linear())?;
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
        let swapchain_image = module.acquire_next_image(swapchain);
        let sprites = module.declare_buffer_var("sprites", Access::ComputeWrite);
        let pick_result = (with_imgui && !state.map_views.is_empty())
            .then(|| module.declare_buffer_var("pick result", Access::HostRead));
        let mut pick_after = pick_result;
        let mut map_views = Vec::with_capacity(state.map_views.len());

        for (index, view_state) in state.map_views.iter().copied().enumerate() {
            let rect = view_state.rect;
            let viewport = vk::Extent2D {
                width: rect.width.max(1),
                height: rect.height.max(1),
            };
            let underlay_viewport = if view_state.has_underlays {
                viewport
            } else {
                vk::Extent2D { width: 1, height: 1 }
            };
            let extent = module.declare_extent_3d_var(&format!("map view {index} extent"), extent3d(viewport));
            let underlay_extent = module.declare_extent_3d_var(
                &format!("map view {index} underlay extent"),
                extent3d(underlay_viewport),
            );
            let scene = module.transient_image_sized(
                &ImageInfo::color_target(viewport, vk::Format::R8G8B8A8_SRGB)
                    .with_usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_SRC)
                    .with_name(format!("map view {index} scene")),
                extent,
            );
            let mut scene_attachment = module.clear(scene, vir::clear::f32::BLACK);
            let visibility = module.transient_image_sized(
                &ImageInfo::color_target(viewport, vk::Format::R32_UINT)
                    .with_usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_SRC)
                    .with_name(format!("map view {index} visibility")),
                extent,
            );
            let mut visibility_attachment = module.clear(visibility, vir::clear::u32::TRANSPARENT);
            let underlays = module.transient_image_sized(
                &ImageInfo::color_target(viewport, vk::Format::R8G8B8A8_SRGB)
                    .with_usage(vk::ImageUsageFlags::SAMPLED)
                    .with_name(format!("map view {index} underlays")),
                underlay_extent,
            );
            let mut underlays_attachment = module.clear(
                underlays,
                if view_state.has_underlays {
                    vir::clear::f32::BLACK
                } else {
                    vir::clear::f32::TRANSPARENT
                },
            );

            let camera = module.declare_bytes_var(&format!("map view {index} camera"), size_of::<CameraPush>() as u32);
            let blur_push = module.declare_bytes_var(&format!("map view {index} blur"), size_of::<BlurPush>() as u32);
            let underlay_draw = view_state
                .has_underlays
                .then(|| module.declare_callback_var(&format!("map view {index} underlay draws")));
            let blur_draw = module.declare_callback_var(&format!("map view {index} blur draw"));
            let active_draw = module.declare_callback_var(&format!("map view {index} active draws"));
            let outline_draw = module.declare_callback_var(&format!("map view {index} area outline draws"));

            if let Some(underlay_draw) = underlay_draw {
                [underlays_attachment] = module
                    .begin_rendering([(underlays_attachment, Access::ColorRW)])
                    .with_name(format!("map view {index} underlays"))
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
                    .push_constants_from(camera)
                    .record_from(underlay_draw)
                    .end_rendering();

                [scene_attachment] = module
                    .begin_rendering([(scene_attachment, Access::ColorRW)])
                    .with_name(format!("map view {index} underlay blur"))
                    .bind_graphics_pipeline(self.blur_pipeline)
                    .set_primitive_topology(vk::PrimitiveTopology::TRIANGLE_STRIP)
                    .set_viewport(0, Rect2D::framebuffer())
                    .set_scissor(0, Rect2D::framebuffer())
                    .bind_texture(0, 0, underlays_attachment, self.blur_sampler)
                    .push_constants_from(blur_push)
                    .record_from(blur_draw)
                    .end_rendering();
            }

            [scene_attachment, visibility_attachment] = module
                .begin_rendering([
                    (scene_attachment, Access::ColorRW),
                    (visibility_attachment, Access::ColorRW),
                ])
                .with_name(format!("map view {index} sprites"))
                .bind_graphics_pipeline(self.visibility_pipeline)
                .set_primitive_topology(vk::PrimitiveTopology::TRIANGLE_STRIP)
                .set_viewport(0, Rect2D::framebuffer())
                .set_scissor(0, Rect2D::framebuffer())
                .set_color_blend(0, BlendPreset::PremultipliedAlphaBlend)
                .set_color_blend(1, BlendPreset::Off)
                .set_rasterization(RasterizationState {
                    cull_mode: vk::CullModeFlags::NONE,
                    ..Default::default()
                })
                .bind_buffer(0, 1, sprites)
                .push_constants_from(camera)
                .record_from(active_draw)
                .end_rendering();

            [scene_attachment] = module
                .begin_rendering([(scene_attachment, Access::ColorRW)])
                .with_name(format!("map view {index} area outlines"))
                .bind_graphics_pipeline(self.outline_pipeline)
                .set_primitive_topology(vk::PrimitiveTopology::TRIANGLE_STRIP)
                .set_viewport(0, Rect2D::framebuffer())
                .set_scissor(0, Rect2D::framebuffer())
                .broadcast_color_blend(BlendPreset::PremultipliedAlphaBlend)
                .set_rasterization(RasterizationState {
                    cull_mode: vk::CullModeFlags::NONE,
                    ..Default::default()
                })
                .bind_buffer(0, 1, sprites)
                .push_constants_from(camera)
                .record_from(outline_draw)
                .end_rendering();

            let (output_attachment, interaction, should_pick) = if with_imgui {
                let push = module.declare_bytes_var(
                    &format!("map view {index} interaction"),
                    size_of::<InteractionPush>() as u32,
                );
                let highlighted = module.transient_image_sized(
                    &ImageInfo::color_target(viewport, vk::Format::R8G8B8A8_SRGB)
                        .with_usage(vk::ImageUsageFlags::SAMPLED)
                        .with_name(format!("map view {index} highlighted scene")),
                    extent,
                );
                let mut highlighted = module.clear(highlighted, vir::clear::f32::BLACK);
                let area_tiles =
                    module.declare_buffer_var(&format!("map view {index} hover area tiles"), Access::ComputeWrite);
                let highlight_draw = module.declare_callback_var(&format!("map view {index} highlight draw"));
                let hover_draw = module.declare_callback_var(&format!("map view {index} hover area draw"));

                [highlighted] = module
                    .begin_rendering([(highlighted, Access::ColorRW)])
                    .with_name(format!("map view {index} highlights"))
                    .bind_graphics_pipeline(self.interaction_pipeline)
                    .set_primitive_topology(vk::PrimitiveTopology::TRIANGLE_STRIP)
                    .set_viewport(0, Rect2D::framebuffer())
                    .set_scissor(0, Rect2D::framebuffer())
                    .bind_texture(0, 0, scene_attachment, self.sampler)
                    .bind_image(0, 1, visibility_attachment)
                    .bind_buffer(0, 2, sprites)
                    .push_constants_from(push)
                    .record_from(highlight_draw)
                    .end_rendering();

                [highlighted] = module
                    .begin_rendering([(highlighted, Access::ColorRW)])
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
                    .push_constants_from(camera)
                    .record_from(hover_draw)
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
                    highlighted,
                    Some(InteractionSlots {
                        push,
                        pick_push,
                        area_tiles,
                        hover_draw,
                        highlight_draw,
                    }),
                    Some(should_pick),
                )
            } else {
                (scene_attachment, None, None)
            };

            map_views.push(MapViewPass {
                scene_attachment,
                visibility_attachment,
                underlays_attachment,
                output_attachment,
                extent,
                underlay_extent,
                camera,
                blur_push,
                blur_draw,
                underlay_draw,
                active_draw,
                outline_draw,
                should_pick,
                interaction,
            });
        }

        let pick_host = pick_after.map(|pick| module.release(pick, Access::HostRead, DomainFlag::Host));
        let mut sampled_map_view_roots = Vec::new();
        let (presented, ui) = if with_imgui {
            for map_view in &map_views {
                let sampled = module.release(
                    map_view.output_attachment,
                    Access::FragmentSampled,
                    DomainFlag::Graphics,
                );
                sampled_map_view_roots.push(sampled);
            }

            let imgui = self.imgui.as_ref().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
            let target = module.clear(swapchain_image, vir::clear::f32::BLACK);
            let map_view_outputs = map_views.iter().map(|view| view.output_attachment).collect::<Vec<_>>();

            imgui.record(&mut module, target, &map_view_outputs)
        } else if let Some(map_view) = map_views.first() {
            (
                module.blit_filtered(map_view.output_attachment, swapchain_image, vk::Filter::NEAREST),
                None,
            )
        } else {
            (module.clear(swapchain_image, vir::clear::f32::BLACK), None)
        };
        let present = module.present(presented);
        let mut roots = sampled_map_view_roots;
        if let Some(pick_host) = pick_host {
            roots.push(pick_host);
        }
        roots.push(present);
        let program = module.compile_all(&self.graph, &roots)?;

        self.recorded = Some(Recorded {
            program,
            sprites,
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
        let graph_state = FrameGraphState::new(viewport, with_imgui, frame.map_views);
        if self
            .recorded
            .as_ref()
            .is_none_or(|recorded| recorded.state != graph_state)
        {
            self.record(graph_state)?;
        }

        self.prepare_sprites(frame)?;
        let stripe_offset =
            (self.highlight_started_at.elapsed().as_secs_f32() * HIGHLIGHT_STRIPE_SPEED) % HIGHLIGHT_STRIPE_PERIOD;

        let recorded = self.recorded.as_mut().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
        let frames = self.frames.as_mut().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
        let sprites = self.sprites.as_ref().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
        recorded.program.set(recorded.sprites, sprites);

        let plans: Vec<MapViewPlan> = frame
            .map_views
            .iter()
            .zip(&self.uploaded)
            .filter(|(view, _)| !view.rect.is_empty())
            .map(|(view, uploaded)| {
                let rect = view.rect;
                let (placement_flash_owner, placement_flash_strength) = view
                    .interaction
                    .placement_flash
                    .filter(|flash| flash.strength.is_finite() && flash.strength > 0.0)
                    .map_or(([0; 2], 0.0), |flash| {
                        (owner_words(flash.owner), flash.strength.min(1.0))
                    });
                let (underlay_ranges, active_range) =
                    visible_ranges(&uploaded.ranges, view.active_z, frame.underlay_depth);

                let logical = [
                    view.camera.viewport_width.max(1) as f32,
                    view.camera.viewport_height.max(1) as f32,
                ];
                let scale = [rect.width as f32 / logical[0], rect.height as f32 / logical[1]];

                MapViewPlan {
                    camera: CameraPush {
                        center: [view.camera.x, view.camera.y],
                        viewport: logical,
                        zoom: view.camera.zoom,
                        base: uploaded.base,
                        show_areas: u32::from(frame.show_areas),
                        show_area_outlines: u32::from(frame.show_area_outlines),
                        focused_area_owner: view.focused_area.map(owner_words).unwrap_or([0; 2]),
                        placement_flash_owner,
                        placement_flash_strength,
                    },
                    blur: BlurPush {
                        sample_step: [
                            view.camera.zoom * scale[0] / rect.width.max(1) as f32,
                            view.camera.zoom * scale[1] / rect.height.max(1) as f32,
                        ],
                    },
                    sprite_base: uploaded.base,
                    underlay_ranges,
                    active_range,
                }
            })
            .collect();

        let visible: Vec<_> = frame
            .map_views
            .iter()
            .zip(&self.uploaded)
            .filter(|(view, _)| !view.rect.is_empty())
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
            let ((view, uploaded), plan) = (visible[index], plans[index].clone());
            let map_extent = vk::Extent2D {
                width: view.rect.width.max(1),
                height: view.rect.height.max(1),
            };
            recorded.program.set(map_view.extent, extent3d(map_extent));
            let underlay_extent = if map_view.underlay_draw.is_some() {
                map_extent
            } else {
                vk::Extent2D { width: 1, height: 1 }
            };
            recorded
                .program
                .set(map_view.underlay_extent, extent3d(underlay_extent));
            recorded.program.set_bytes(map_view.camera, &plan.camera);
            recorded.program.set_bytes(map_view.blur_push, &plan.blur);

            if let Some(underlay_draw) = map_view.underlay_draw {
                let underlay_plan = plan.clone();
                recorded.program.set(
                    underlay_draw,
                    PassCallback::new(move |cmd| {
                        underlay_plan.bind_scene(cmd);
                        for &(base, count) in &underlay_plan.underlay_ranges {
                            let base = underlay_plan.sprite_base + base;
                            cmd.push_constants_at(std::mem::offset_of!(CameraPush, base) as u32, &base)
                                .draw(4, count);
                        }
                    }),
                );
            }

            let blur = plan.blur;
            recorded.program.set(
                map_view.blur_draw,
                PassCallback::new(move |cmd| {
                    cmd.set_viewport(0, Rect2D::framebuffer())
                        .set_scissor(0, Rect2D::framebuffer())
                        .push_constants(&blur)
                        .draw(4, 1);
                }),
            );

            let active_plan = plan.clone();
            recorded.program.set(
                map_view.active_draw,
                PassCallback::new(move |cmd| {
                    active_plan.bind_scene(cmd);
                    let (base, count) = active_plan.active_range;
                    let base = active_plan.sprite_base + base;
                    cmd.push_constants_at(std::mem::offset_of!(CameraPush, base) as u32, &base)
                        .draw(4, count);
                }),
            );

            let outline_plan = plan.clone();
            recorded.program.set(
                map_view.outline_draw,
                PassCallback::new(move |cmd| {
                    outline_plan.bind_scene(cmd);
                    let (base, count) = outline_plan.active_range;
                    let base = outline_plan.sprite_base + base;
                    cmd.push_constants_at(std::mem::offset_of!(CameraPush, base) as u32, &base)
                        .draw(4, count);
                }),
            );

            let Some(slots) = map_view.interaction.as_ref() else {
                continue;
            };
            let interaction = view.interaction;
            let local_cursor = interaction
                .cursor
                .filter(|cursor| cursor[0] < view.rect.width && cursor[1] < view.rect.height);
            let guide = interaction
                .selection_guide
                .filter(|guide| {
                    interaction.selected.is_some()
                        && guide.origin.iter().chain(&guide.target).all(|value| value.is_finite())
                })
                .map(|guide| project_selection_guide(guide, view.camera, view.rect))
                .filter(|(origin, target)| origin.iter().chain(target).all(|value| value.is_finite()));
            let (guide_origin, guide_target, guide_valid) =
                guide.map_or(([0.0; 2], [0.0; 2], 0), |(origin, target)| (origin, target, 1));
            let push = InteractionPush {
                cursor: local_cursor.unwrap_or([0; 2]),
                cursor_valid: u32::from(local_cursor.is_some()),
                selected_owner: interaction.selected.map(owner_words).unwrap_or([0; 2]),
                stripe_offset,
                guide_origin,
                guide_target,
                guide_valid,
                interaction_mode: u32::from(interaction.mode.is_delete()),
                focused_area_owner: view.focused_area.map(owner_words).unwrap_or([0; 2]),
                highlight_tint: u32::from(interaction.highlight.is_tint()),
            };
            recorded.program.set_bytes(slots.push, &push);
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
                    let show = 1u32;

                    plan.bind_scene(cmd);
                    cmd.push_constants_at(std::mem::offset_of!(CameraPush, base) as u32, base);
                    cmd.push_constants_at(std::mem::offset_of!(CameraPush, show_area_outlines) as u32, &show);
                    cmd.draw(4, 1);
                }),
            );

            let _ = (
                map_view.scene_attachment,
                map_view.visibility_attachment,
                map_view.underlays_attachment,
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

fn gpu_sprite(index: usize, sprite: &SpriteInstance) -> Result<GpuSprite, GpuError> {
    let flags = if sprite.is_area {
        SPRITE_FLAG_AREA | ((sprite.area_edges & AREA_EDGES_ALL) << SPRITE_AREA_EDGE_SHIFT)
    } else {
        0
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
        packed_size: size,
        color,
        source_position_size: [source_position, source_size],
        texture_flags: sprite.texture.index | (flags << SPRITE_FLAGS_SHIFT),
    })
}

fn owner_words(owner: dmm::PrefabInstanceId) -> [u32; 2] { split_owner(owner.get()) }

fn split_owner(owner: u64) -> [u32; 2] { [owner as u32, (owner >> 32) as u32] }

fn project_selection_guide(
    guide: crate::SelectionGuide, camera: crate::Camera, rect: crate::MapViewRect,
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
        self.device.allocator.deallocate_sampler(self.blur_sampler);

        if let Some(buffer) = self.sprites.take() {
            self.device.allocator.deallocate_buffer(buffer);
        }
        if let Some(buffer) = self.area_tiles.take() {
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

fn visible_ranges(ranges: &[LevelRange], active_z: u32, underlay_depth: u32) -> (Vec<(u32, u32)>, (u32, u32)) {
    let minimum_z = active_z.saturating_sub(underlay_depth).max(1);
    let mut underlays = Vec::new();
    let mut active = (0, 0);

    for range in ranges {
        if (minimum_z..active_z).contains(&range.z) {
            underlays.push((range.base, range.count));
        } else if range.z == active_z {
            active = (range.base, range.count);
        }
    }

    (underlays, active)
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
            let destination =
                module.import_attachment(&ImageAttachment::from_image(&texture.image, vk::ImageLayout::UNDEFINED));
            let copied = module.copy_buffer_to_image_region(
                source_buffer,
                destination,
                copy_region(offset, source.width, source.height),
            );
            roots.push(module.release(
                copied,
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
        CameraPush,
        FrameGraphState,
        GEOMETRY_VS_SPV,
        GpuSprite,
        INTERACTION_FS_SPV,
        InteractionPush,
        LevelRange,
        PICK_CS_SPV,
        PickPush,
        SPRITE_FLAG_AREA,
        SPRITE_FLAGS_SHIFT,
        SPRITE_OUTLINE_FS_SPV,
        SPRITE_TEXTURE_MASK,
        SPRITE_VIS_FS_SPV,
        UPLOAD_BATCH_BYTES,
        area_color_group_count,
        batch_ranges,
        copy_region,
        descriptor_capacity,
        gpu_sprite,
        level_ranges,
        meshopt_dequantize_half,
        meshopt_dequantize_unorm,
        meshopt_quantize_half,
        pack_unorm4x8,
        patch_area_tile_indices,
        project_selection_guide,
        split_owner,
        valid_update_range,
        visible_ranges,
    };
    use crate::{
        AREA_EDGE_EAST,
        AREA_EDGE_NORTH,
        Camera,
        GpuError,
        MapViewFrame,
        MapViewRect,
        SelectionGuide,
        SpriteInstance,
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
            interaction: Default::default(),
        }
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
            FrameGraphState::new(target, true, &unchanged),
            FrameGraphState::new(target, true, &dynamic),
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
        let base = FrameGraphState::new(target, true, &[map_view(left), map_view(right)]);

        let moved = MapViewRect { x: 32, ..left };
        assert_ne!(
            base,
            FrameGraphState::new(target, true, &[map_view(moved), map_view(right)])
        );

        let resized = MapViewRect { width: 600, ..left };
        assert_ne!(
            base,
            FrameGraphState::new(target, true, &[map_view(resized), map_view(right)])
        );
        assert_ne!(base, FrameGraphState::new(target, true, &[map_view(left)]));
        assert_ne!(
            base,
            FrameGraphState::new(target, true, &[map_view(right), map_view(left)])
        );
        assert_ne!(
            base,
            FrameGraphState::new(
                vk::Extent2D {
                    width: 1920,
                    height: 1080,
                },
                true,
                &[map_view(left), map_view(right)],
            )
        );
        assert_ne!(
            base,
            FrameGraphState::new(target, false, &[map_view(left), map_view(right)])
        );
    }

    #[test]
    fn frame_graph_state_tracks_whether_underlay_passes_are_needed() {
        let target = vk::Extent2D {
            width: 1280,
            height: 720,
        };
        let rect = MapViewRect {
            width: 640,
            height: 480,
            ..Default::default()
        };
        let single_level = map_view(rect);
        let mut layered = map_view(rect);
        layered.level_count = 2;

        let single_level = FrameGraphState::new(target, true, &[single_level]);
        let layered = FrameGraphState::new(target, true, &[layered]);

        assert!(!single_level.map_views[0].has_underlays);
        assert!(layered.map_views[0].has_underlays);
        assert_ne!(single_level, layered);
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
        let visible = FrameGraphState::new(target, true, &[map_view(rect)]);
        let visible_with_empty =
            FrameGraphState::new(target, true, &[map_view(rect), map_view(MapViewRect::default())]);
        let hidden = FrameGraphState::new(target, true, &[map_view(MapViewRect::default())]);

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
        let mut area = sprite(1);
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
    fn normal_area_sprites_have_no_outline_flags() {
        let mut area = sprite(1);
        area.is_area = true;

        let gpu = gpu_sprite(0, &area).expect("pack");

        assert_eq!(gpu.texture_flags >> SPRITE_FLAGS_SHIFT, SPRITE_FLAG_AREA);
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

        let (underlays, active) = visible_ranges(&ranges, 3, 2);

        assert_eq!(underlays, [(0, 2), (2, 3)]);
        assert_eq!(active, (5, 4));
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

        assert_eq!(visible_ranges(&ranges, 2, 9), (vec![(0, 2)], (2, 3)));
        assert_eq!(visible_ranges(&ranges, 2, 0), (Vec::new(), (2, 3)));
    }

    #[test]
    fn sprite_and_camera_layouts_match_the_shader_scalar_layout() {
        let reflection = shader::reflect(&read_spirv(GEOMETRY_VS_SPV).expect("valid SPIR-V")).expect("shader reflects");

        assert_eq!(size_of::<GpuSprite>(), 44);
        assert_eq!(reflection.push_constant_offset, 0);
        assert_eq!(reflection.push_constant_size as usize, size_of::<CameraPush>());
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
    fn selection_guides_project_into_map_view_coordinates() {
        let guide = SelectionGuide {
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
            project_selection_guide(guide, camera, full),
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
            project_selection_guide(guide, camera, right),
            ([420.0, 320.0], [460.0, 280.0]),
        );

        let scaled = crate::MapViewRect {
            x: 0,
            y: 0,
            width: 1600,
            height: 1200,
        };
        assert_eq!(
            project_selection_guide(guide, camera, scaled),
            ([840.0, 640.0], [920.0, 560.0]),
        );

        let panned = Camera {
            x: 110.0,
            y: 60.0,
            ..camera
        };
        assert_eq!(
            project_selection_guide(guide, panned, scaled),
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

        assert_eq!(bindings, [(0, 0), (0, 1), (0, 2)]);
        assert_eq!(reflection.push_constant_offset, 0);
        assert_eq!(reflection.push_constant_size as usize, size_of::<InteractionPush>());
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
        for spirv in [SPRITE_VIS_FS_SPV, SPRITE_OUTLINE_FS_SPV] {
            let reflection = shader::reflect(&read_spirv(spirv).expect("valid SPIR-V")).expect("shader reflects");
            let mut bindings = reflection
                .bindings
                .iter()
                .map(|binding| (binding.set, binding.binding))
                .collect::<Vec<_>>();
            bindings.sort_unstable();

            assert_eq!(bindings, [(0, 1), (1, 0)]);
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
