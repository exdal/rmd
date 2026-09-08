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
    ViewportInteraction,
    VisibilityId,
    extent3d,
    imgui::{ImGuiPass, ImGuiSlots},
    read_spirv,
    texture::TextureCatalog,
};

const GEOMETRY_VS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sprite.vert.spv"));
const SPRITE_SHADE_FS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sprite.frag.spv"));
const SPRITE_VIS_FS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sprite_visibility.frag.spv"));
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

#[repr(C)]
#[derive(Clone, Copy)]
struct AreaColorPush {
    first_sprite: u32,
    sprite_count: u32,
    group_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CameraPush {
    center: [f32; 2],
    viewport: [f32; 2],
    zoom: f32,
    base: u32,
    show_areas: u32,
    show_area_outlines: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
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
    pick_result: ValueId,
    area_tiles: ValueId,
    hover_draws: ValueId,
}

struct Recorded {
    program: Program,
    scene_extent: ValueId,
    underlay_extent: ValueId,
    sprites: ValueId,
    camera: ValueId,
    blur_push: ValueId,
    underlay_draws: ValueId,
    active_draws: ValueId,
    interaction: Option<InteractionSlots>,
    ui: Option<ImGuiSlots>,
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

struct TextureImage {
    image: Image,
    view: vk::ImageView,
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

pub struct Renderer {
    graph: RenderGraph,
    recorded: Option<Recorded>,
    frames: Option<SuperFrameAllocator>,
    swapchain: Option<SwapChain>,
    sprite_pipeline: PipelineId,
    visibility_pipeline: PipelineId,
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
    area_tile_indices: HashMap<editor::document::PrefabInstanceId, u32>,
    pick_readback: Option<Buffer>,
    uploaded_revision: Option<u64>,
    uploaded_sprite_count: usize,
    uploaded_area_tile_count: usize,
    uploaded_area_owners: Vec<editor::document::PrefabInstanceId>,
    ranges: Vec<LevelRange>,
    imgui: Option<ImGuiPass>,
    highlight_started_at: Instant,
    extent: vk::Extent2D,
    stale: bool,
    device: Device,
}

impl Renderer {
    pub const VIEWPORT_TEXTURE: dear_imgui_rs::TextureId = crate::imgui::VIEWPORT_TEXTURE;

    pub fn new(device: Device, width: u32, height: u32, texture_capacity: usize) -> Result<Self, GpuError> {
        let texture_limit = device.max_bindless_textures.min(SPRITE_TEXTURE_CAPACITY);
        let texture_capacity = descriptor_capacity(texture_capacity, texture_limit)?;
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
            area_tile_indices: HashMap::new(),
            pick_readback: None,
            uploaded_revision: None,
            uploaded_sprite_count: 0,
            uploaded_area_tile_count: 0,
            uploaded_area_owners: Vec::new(),
            ranges: Vec::new(),
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
        self.uploaded_revision = None;
        let old = std::mem::replace(&mut self.textures, textures);
        destroy_images(&mut self.device, old);

        Ok(())
    }

    pub fn draw(&mut self, frame: &Frame<'_>) -> Result<(), GpuError> {
        self.draw_inner(frame, None, None, ViewportInteraction::default())?;

        Ok(())
    }

    pub fn draw_imgui(
        &mut self, frame: &Frame<'_>, viewport: (u32, u32), pending: PendingFrame<'_>, interaction: ViewportInteraction,
    ) -> Result<Option<PickResult>, GpuError> {
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

        let viewport = vk::Extent2D {
            width: viewport.0.max(1),
            height: viewport.1.max(1),
        };

        self.draw_inner(frame, Some(viewport), Some(pending), interaction)
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

    fn record(&mut self, viewport: vk::Extent2D, with_imgui: bool) -> Result<(), GpuError> {
        let swapchain = self.swapchain.as_ref().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
        let mut module = Module::default();
        let swapchain_image = module.acquire_next_image(swapchain);

        let scene_extent = module.declare_extent_3d_var("scene extent", extent3d(viewport));
        let scene_attachment = module.transient_image_sized(
            &ImageInfo::color_target(viewport, vk::Format::R8G8B8A8_SRGB)
                .with_usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_SRC)
                .with_name("scene"),
            scene_extent,
        );
        let scene_attachment = module.clear(scene_attachment, vir::clear::f32::BLACK);

        let visibility_attachment = module.transient_image_sized(
            &ImageInfo::color_target(viewport, vk::Format::R32_UINT)
                .with_usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_SRC)
                .with_name("visibility"),
            scene_extent,
        );
        let visibility_attachment = module.clear(visibility_attachment, vir::clear::u32::TRANSPARENT);

        let underlay_extent = module.declare_extent_3d_var("underlay extent", extent3d(viewport));
        let underlays = module.transient_image_sized(
            &ImageInfo::color_target(viewport, vk::Format::R8G8B8A8_SRGB)
                .with_usage(vk::ImageUsageFlags::SAMPLED)
                .with_name("underlays"),
            underlay_extent,
        );
        let underlays_attachment = module.clear(underlays, vir::clear::f32::BLACK);

        let sprites = module.declare_buffer_var("sprites", Access::ComputeWrite);
        let camera = module.declare_bytes_var("camera", size_of::<CameraPush>() as u32);
        let blur_push = module.declare_bytes_var("blur", size_of::<BlurPush>() as u32);
        let underlay_draws = module.declare_callback_var("underlay draws");
        let active_draws = module.declare_callback_var("active draws");

        let [underlays_attachment] = module
            .begin_rendering([(underlays_attachment, Access::ColorRW)])
            .with_name("underlays")
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
            .record_from(underlay_draws)
            .end_rendering();

        let [scene_attachment] = module
            .begin_rendering([(scene_attachment, Access::ColorRW)])
            .with_name("underlay blur")
            .bind_graphics_pipeline(self.blur_pipeline)
            .set_primitive_topology(vk::PrimitiveTopology::TRIANGLE_STRIP)
            .set_viewport(0, Rect2D::framebuffer())
            .set_scissor(0, Rect2D::framebuffer())
            .bind_texture(0, 0, underlays_attachment, self.blur_sampler)
            .push_constants_from(blur_push)
            .draw(4u32, 1u32)
            .end_rendering();

        let [scene_attachment, visibility_attachment] = module
            .begin_rendering([
                (scene_attachment, Access::ColorRW),
                (visibility_attachment, Access::ColorRW),
            ])
            .with_name("sprites")
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
            .record_from(active_draws)
            .end_rendering();

        let (scene_attachment, interaction, pick_host) = if with_imgui {
            let interaction_push =
                module.declare_bytes_var("viewport interaction", size_of::<InteractionPush>() as u32);
            let highlighted = module.transient_image_sized(
                &ImageInfo::color_target(viewport, vk::Format::R8G8B8A8_SRGB)
                    .with_usage(vk::ImageUsageFlags::SAMPLED)
                    .with_name("highlighted scene"),
                scene_extent,
            );
            let highlighted = module.clear(highlighted, vir::clear::f32::BLACK);
            let [highlighted] = module
                .begin_rendering([(highlighted, Access::ColorRW)])
                .with_name("viewport highlights")
                .bind_graphics_pipeline(self.interaction_pipeline)
                .set_primitive_topology(vk::PrimitiveTopology::TRIANGLE_STRIP)
                .set_viewport(0, Rect2D::framebuffer())
                .set_scissor(0, Rect2D::framebuffer())
                .bind_texture(0, 0, scene_attachment, self.sampler)
                .bind_image(0, 1, visibility_attachment)
                .bind_buffer(0, 2, sprites)
                .push_constants_from(interaction_push)
                .draw(4u32, 1u32)
                .end_rendering();

            let area_tiles = module.declare_buffer_var("hover area tiles", Access::ComputeWrite);
            let hover_draws = module.declare_callback_var("hover tile outline");
            let [highlighted] = module
                .begin_rendering([(highlighted, Access::ColorRW)])
                .with_name("hover tile outline")
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
                .record_from(hover_draws)
                .end_rendering();

            let pick_push = module.declare_bytes_var("pick cursor", size_of::<PickPush>() as u32);
            let pick_result = module.declare_buffer_var("pick result", Access::HostRead);
            let [_, pick_result_after] = module
                .begin_compute([
                    (visibility_attachment, Access::ComputeSampled),
                    (pick_result, Access::ComputeWrite),
                ])
                .with_name("mouse pick")
                .bind_compute_pipeline(self.pick_pipeline)
                .bind_image(0, 1, visibility_attachment)
                .bind_buffer(0, 3, pick_result)
                .push_constants_from(pick_push)
                .dispatch(1u32, 1u32, 1u32)
                .end_compute();
            let pick_host = module.release(pick_result_after, Access::HostRead, DomainFlag::Host);

            (
                highlighted,
                Some(InteractionSlots {
                    push: interaction_push,
                    pick_push,
                    pick_result,
                    area_tiles,
                    hover_draws,
                }),
                Some(pick_host),
            )
        } else {
            (scene_attachment, None, None)
        };

        let (presented, ui) = if with_imgui {
            let imgui = self.imgui.as_ref().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
            let target = module.clear(swapchain_image, vir::clear::f32::BLACK);

            imgui.record(&mut module, target, scene_attachment)
        } else {
            (
                module.blit_filtered(scene_attachment, swapchain_image, vk::Filter::NEAREST),
                None,
            )
        };
        let present = module.present(presented);
        let program = match pick_host {
            Some(pick_host) => module.compile_all(&self.graph, &[present, pick_host])?,
            None => module.compile(&self.graph, present)?,
        };

        self.recorded = Some(Recorded {
            program,
            scene_extent,
            underlay_extent,
            sprites,
            camera,
            blur_push,
            underlay_draws,
            active_draws,
            interaction,
            ui,
        });

        Ok(())
    }

    fn draw_inner(
        &mut self, frame: &Frame<'_>, viewport: Option<vk::Extent2D>, pending: Option<PendingFrame<'_>>,
        interaction: ViewportInteraction,
    ) -> Result<Option<PickResult>, GpuError> {
        if self.stale {
            self.recreate_swapchain()?;
        }

        let viewport = viewport.unwrap_or(self.extent);
        let with_imgui = pending.is_some();
        if self.recorded.as_ref().is_none_or(|r| r.ui.is_some() != with_imgui) {
            self.record(viewport, with_imgui)?;
        }

        self.prepare_sprites(frame)?;
        let stripe_offset =
            (self.highlight_started_at.elapsed().as_secs_f32() * HIGHLIGHT_STRIPE_SPEED) % HIGHLIGHT_STRIPE_PERIOD;

        let recorded = self.recorded.as_mut().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
        let frames = self.frames.as_mut().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
        let sprites = self.sprites.as_ref().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
        recorded.program.set(recorded.scene_extent, extent3d(viewport));
        recorded.program.set(recorded.underlay_extent, extent3d(viewport));
        recorded.program.set(recorded.sprites, sprites);

        let push = CameraPush {
            center: [frame.camera.x, frame.camera.y],
            viewport: [viewport.width as f32, viewport.height as f32],
            zoom: frame.camera.zoom,
            base: 0,
            show_areas: u32::from(frame.show_areas),
            show_area_outlines: u32::from(frame.show_area_outlines),
        };
        recorded.program.set_bytes(recorded.camera, &push);
        recorded.program.set_bytes(
            recorded.blur_push,
            &BlurPush {
                sample_step: [
                    frame.camera.zoom / viewport.width as f32,
                    frame.camera.zoom / viewport.height as f32,
                ],
            },
        );

        let (underlay_ranges, active_range) = visible_ranges(&self.ranges, frame.active_z, frame.underlay_depth);
        recorded.program.set(
            recorded.underlay_draws,
            PassCallback::new(move |cmd| {
                for &(base, count) in &underlay_ranges {
                    cmd.push_constants_at(std::mem::offset_of!(CameraPush, base) as u32, &base)
                        .draw(4, count);
                }
            }),
        );
        recorded.program.set(
            recorded.active_draws,
            PassCallback::new(move |cmd| {
                let (base, count) = active_range;
                cmd.push_constants_at(std::mem::offset_of!(CameraPush, base) as u32, &base)
                    .draw(4, count);
            }),
        );

        let cursor = interaction
            .cursor
            .filter(|cursor| cursor[0] < viewport.width && cursor[1] < viewport.height);
        if let Some(slots) = recorded.interaction.as_ref() {
            let selected_owner = interaction.selected.map(owner_words).unwrap_or([0; 2]);
            let cursor_value = cursor.unwrap_or([0; 2]);
            recorded.program.set_bytes(
                slots.push,
                &InteractionPush {
                    cursor: cursor_value,
                    cursor_valid: u32::from(cursor.is_some()),
                    selected_owner,
                    stripe_offset,
                },
            );
            recorded.program.set_bytes(
                slots.pick_push,
                &PickPush {
                    cursor: cursor_value,
                    cursor_valid: u32::from(cursor.is_some()),
                },
            );

            let area_tiles = self.area_tiles.as_ref().unwrap_or(sprites);
            let pick_readback = self
                .pick_readback
                .as_ref()
                .ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
            recorded.program.set(slots.area_tiles, area_tiles);
            recorded.program.set(slots.pick_result, pick_readback);

            let hovered_area = interaction
                .hovered_area
                .and_then(|owner| self.area_tile_indices.get(&owner).copied());
            recorded.program.set(
                slots.hover_draws,
                PassCallback::new(move |cmd| {
                    if let Some(base) = hovered_area {
                        let show = 1u32;
                        cmd.push_constants_at(std::mem::offset_of!(CameraPush, base) as u32, &base);
                        cmd.push_constants_at(std::mem::offset_of!(CameraPush, show_area_outlines) as u32, &show);
                        cmd.draw(4, 1);
                    }
                }),
            );
        }

        let next = frames.get_next_frame()?;
        if let Some(pending) = pending {
            let imgui = self.imgui.as_mut().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
            let feedback = imgui.poll_textures(&mut self.device, &mut self.graph, next, pending.texture_requests())?;
            let reconciled = pending
                .reconcile_texture_feedback(feedback)
                .map_err(|e| GpuError::ImGui(e.to_string()))?;
            let ui_frame = imgui.prepare(next, reconciled.draw_data(), self.extent)?;

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

        if !executed || !interaction.pick || cursor.is_none() {
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
        let picked = VisibilityId::from_raw(raw)
            .and_then(|id| frame.sprite_instances.get(id.sprite_index()))
            .map_or(PickResult::Miss, |sprite| PickResult::Hit(sprite.owner));

        Ok(Some(picked))
    }

    fn prepare_sprites(&mut self, frame: &Frame<'_>) -> Result<(), GpuError> {
        if self.uploaded_revision == Some(frame.revision) {
            return Ok(());
        }

        if self.prepare_frame_update(frame)? {
            return Ok(());
        }

        let total = frame.sprite_instances.len();
        u32::try_from(total).map_err(|_| GpuError::SpriteUploadTooLarge)?;
        let mut payload = Vec::with_capacity(total);
        let area_total = frame.area_tiles.len();
        u32::try_from(area_total).map_err(|_| GpuError::SpriteUploadTooLarge)?;
        let mut area_payload = Vec::with_capacity(area_total);
        let ranges = level_ranges(frame.sprite_instances);

        for (index, sprite) in frame.sprite_instances.iter().enumerate() {
            payload.push(gpu_sprite(index, sprite)?);
        }
        for (index, area) in frame.area_tiles.iter().enumerate() {
            area_payload.push(gpu_sprite(index, area)?);
        }

        self.device.wait_idle()?;
        self.uploaded_revision = None;

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

        self.ranges = ranges;
        self.area_tile_indices.clear();
        for (index, area) in frame.area_tiles.iter().enumerate() {
            let index = u32::try_from(index).map_err(|_| GpuError::SpriteUploadTooLarge)?;
            self.area_tile_indices.insert(area.owner, index);
        }
        self.uploaded_area_owners = frame.area_tiles.iter().map(|area| area.owner).collect();
        self.uploaded_revision = Some(frame.revision);
        self.uploaded_sprite_count = total;
        self.uploaded_area_tile_count = area_total;

        Ok(())
    }

    fn prepare_frame_update(&mut self, frame: &Frame<'_>) -> Result<bool, GpuError> {
        let Some(update) = frame.pending_update else {
            return Ok(false);
        };
        if self.uploaded_revision != Some(update.previous_revision)
            || frame.sprite_instances.len() > self.sprite_capacity
            || frame.area_tiles.len() > self.area_tile_capacity
            || !valid_update_range(update.sprites, frame.sprite_instances.len())
            || !valid_update_range(update.area_tiles, frame.area_tiles.len())
            || (self.uploaded_sprite_count != frame.sprite_instances.len() && update.sprites.is_none())
            || (self.uploaded_area_tile_count != frame.area_tiles.len() && update.area_tiles.is_none())
        {
            return Ok(false);
        }
        let Some(sprites) = self.sprites else {
            return Ok(false);
        };
        if !frame.area_tiles.is_empty() && self.area_tiles.is_none() {
            return Ok(false);
        }

        let sprite_payload = update
            .sprites
            .map(|range| gpu_sprite_range(frame.sprite_instances, range))
            .transpose()?;
        let area_payload = update
            .area_tiles
            .map(|range| gpu_sprite_range(frame.area_tiles, range))
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
            patch_area_tile_indices(
                &mut self.area_tile_indices,
                &mut self.uploaded_area_owners,
                frame.area_tiles,
                range,
            )?;
        }

        if self.uploaded_sprite_count != frame.sprite_instances.len() {
            self.ranges = level_ranges(frame.sprite_instances);
        }
        self.uploaded_revision = Some(frame.revision);
        self.uploaded_sprite_count = frame.sprite_instances.len();
        self.uploaded_area_tile_count = frame.area_tiles.len();

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
    indices: &mut HashMap<editor::document::PrefabInstanceId, u32>,
    uploaded_owners: &mut Vec<editor::document::PrefabInstanceId>, area_tiles: &[SpriteInstance],
    range: crate::UpdateRange,
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
        position,
        packed_size: size,
        color,
        source_position_size: [source_position, source_size],
        texture_flags: sprite.texture.index | (flags << SPRITE_FLAGS_SHIFT),
    })
}

fn owner_words(owner: editor::document::PrefabInstanceId) -> [u32; 2] { split_owner(owner.get()) }

fn split_owner(owner: u64) -> [u32; 2] { [owner as u32, (owner >> 32) as u32] }

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
    use core::path::TreePath;
    use std::collections::HashMap;

    use dmm::{Coord, Map, Prefab, Size};
    use editor::document::{MapDocument, PrefabInstanceId};
    use vir::resource::shader;

    use super::{
        AREA_COLOR_CS_SPV,
        AreaColorPush,
        CameraPush,
        GEOMETRY_VS_SPV,
        GpuSprite,
        INTERACTION_FS_SPV,
        InteractionPush,
        LevelRange,
        PICK_CS_SPV,
        PickPush,
        SPRITE_FLAG_AREA,
        SPRITE_FLAGS_SHIFT,
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
        split_owner,
        valid_update_range,
        visible_ranges,
    };
    use crate::{AREA_EDGE_EAST, AREA_EDGE_NORTH, GpuError, SpriteInstance, SpriteTexture, UpdateRange, read_spirv};

    fn owner() -> PrefabInstanceId {
        let mut map = Map::new(Size { x: 1, y: 1, z: 1 });
        let key = map.intern_tile(vec![Prefab::new(TreePath::parse("/obj/test"))]);
        map.grid[0][0][0] = key;
        let document = MapDocument::new(map, 1);

        document.instance_ids_at(Coord::new(1, 1, 1))[0]
    }

    fn sprite(z: u32) -> SpriteInstance {
        SpriteInstance {
            owner: owner(),
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
        let mut map = Map::new(Size { x: 1, y: 1, z: 1 });
        let tile = (0..count).map(|_| Prefab::new(TreePath::parse("/area/test"))).collect();
        let key = map.intern_tile(tile);
        map.grid[0][0][0] = key;
        let document = MapDocument::new(map, 1);

        document.instance_ids_at(Coord::new(1, 1, 1)).to_vec()
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

        assert_eq!(size_of::<GpuSprite>(), 36);
        assert_eq!(reflection.push_constant_offset, 0);
        assert_eq!(reflection.push_constant_size as usize, size_of::<CameraPush>());
    }

    #[test]
    fn owner_ids_keep_both_words() {
        assert_eq!(split_owner(0x1234_5678_9abc_def0), [0x9abc_def0, 0x1234_5678]);
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
    fn visibility_fragment_reads_sprites_and_textures() {
        let reflection =
            shader::reflect(&read_spirv(SPRITE_VIS_FS_SPV).expect("valid SPIR-V")).expect("shader reflects");
        let mut bindings = reflection
            .bindings
            .iter()
            .map(|binding| (binding.set, binding.binding))
            .collect::<Vec<_>>();
        bindings.sort_unstable();

        assert_eq!(bindings, [(0, 1), (1, 0)]);
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
