use std::time::Instant;

use ash::vk;
use dear_imgui_rs::render::ReconciledFrame;
use vir::{
    Buffer, BufferInfo, ComputePipelineInfo, GraphicsPipelineInfo, MemoryLocation, PipelineId, RenderGraph,
    SamplerInfo, SuperFrameAllocator, SwapChain, allocator::Allocator,
};

use self::{
    area_colors::AreaColorPass,
    bindless::{BindlessDescriptorSet, descriptor_capacity},
    cull::CullBuffers,
    gpu::SPRITE_TEXTURE_CAPACITY,
    lighting::UploadedLighting,
    preview::UploadedPreview,
    record::Recorded,
    shaders::{
        AREA_COLOR_CS_SPV, BLUR_FS_SPV, BLUR_VS_SPV, CAPTURE_CS_SPV, GEOMETRY_VS_SPV, GUIDE_FS_SPV, GUIDE_VS_SPV,
        INTERACTION_FS_SPV, INTERACTION_VS_SPV, LIGHTING_FS_SPV, LIGHTING_VS_SPV, PICK_CS_SPV, SCENE_RESOLVE_FS_SPV,
        SPRITE_CULL_CLASSIFY_CS_SPV, SPRITE_CULL_COMPACT_CS_SPV, SPRITE_CULL_SCAN_CS_SPV, SPRITE_OVERLAY_LIGHT_FS_SPV,
        SPRITE_SCENE_FS_SPV, SPRITE_SHADE_FS_SPV, SPRITE_VIS_FS_SPV,
    },
    sprites::UploadedMapView,
    upload::{TextureSource, destroy_images, loaded_images},
};
use crate::{
    Device, GpuError,
    imgui::ImGuiPass,
    read_spirv,
    viewport::{SecondaryViewport, ViewportTarget},
};

mod area_colors;
mod bindless;
mod capture;
mod cull;
mod draw;
#[cfg(test)]
mod fixtures;
mod gpu;
mod guides;
mod lighting;
mod preview;
mod record;
mod shaders;
mod sheets;
mod sprites;
mod upload;

pub(crate) use self::sheets::{Sheet, TextureImage};

type PlatformWindows<'a> = &'a mut dyn FnMut(&mut ReconciledFrame<'_>) -> Vec<SecondaryViewport>;

const FRAMES_IN_FLIGHT: usize = 2;

const TEXTURE_RESERVE: usize = 8192;

pub struct Renderer {
    graph: RenderGraph,
    recorded: Option<Recorded>,
    frames: Option<SuperFrameAllocator>,
    swapchain: Option<SwapChain>,
    viewports: Vec<ViewportTarget>,
    failed_viewports: Vec<(u32, [u32; 2])>,
    sprite_pipeline: PipelineId,
    scene_pipeline: PipelineId,
    visibility_pipeline: PipelineId,
    overlay_light_pipeline: PipelineId,
    cull_pipelines: [PipelineId; 3],
    lighting_pipeline: PipelineId,
    resolve_pipeline: PipelineId,
    blur_pipeline: PipelineId,
    interaction_pipeline: PipelineId,
    guide_pipeline: PipelineId,
    pick_pipeline: PipelineId,
    capture_pipeline: PipelineId,
    area_colors: AreaColorPass,
    bindless: BindlessDescriptorSet,
    sheets: Vec<Sheet>,
    frame: u64,
    requested_sheets: Vec<u32>,
    fallback: Option<TextureImage>,
    sampler: vk::Sampler,
    blur_sampler: vk::Sampler,
    sprites: Option<Buffer>,
    sprite_capacity: usize,
    lights: Option<Buffer>,
    light_capacity: usize,
    cull: Vec<CullBuffers>,
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
    pub const fn map_view_texture(index: usize) -> dear_imgui_rs::TextureId {
        crate::imgui::map_view_texture(index)
    }

    pub const fn sprite_texture(index: u32) -> dear_imgui_rs::TextureId {
        crate::imgui::sprite_texture(index)
    }

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
        let scene_pipeline = match graph.declare_pipeline(
            GraphicsPipelineInfo::new()
                .with_shader(&geometry_vs)
                .with_shader(&read_spirv(SPRITE_SCENE_FS_SPV)?)
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
        let resolve_pipeline = match graph.declare_pipeline(
            GraphicsPipelineInfo::new()
                .with_shader(&read_spirv(LIGHTING_VS_SPV)?)
                .with_shader(&read_spirv(SCENE_RESOLVE_FS_SPV)?),
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
        let pick_pipeline = match graph.declare_compute_pipeline(ComputePipelineInfo::new(&read_spirv(PICK_CS_SPV)?)) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                drop(graph);
                bindless.destroy(&device);

                return Err(error.into());
            },
        };
        let capture_pipeline =
            match graph.declare_compute_pipeline(ComputePipelineInfo::new(&read_spirv(CAPTURE_CS_SPV)?)) {
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
            viewports: Vec::new(),
            failed_viewports: Vec::new(),
            sprite_pipeline,
            scene_pipeline,
            visibility_pipeline,
            overlay_light_pipeline,
            cull_pipelines,
            lighting_pipeline,
            resolve_pipeline,
            blur_pipeline,
            interaction_pipeline,
            guide_pipeline,
            pick_pipeline,
            capture_pipeline,
            area_colors,
            bindless,
            sheets: Vec::new(),
            frame: 0,
            requested_sheets: Vec::new(),
            fallback: None,
            sampler: vk::Sampler::null(),
            blur_sampler: vk::Sampler::null(),
            sprites: None,
            sprite_capacity: 0,
            lights: None,
            light_capacity: 0,
            cull: Vec::new(),
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
        renderer.blur_sampler = renderer
            .device
            .allocator
            .allocate_sampler(&SamplerInfo::linear().with_address_mode(vk::SamplerAddressMode::CLAMP_TO_EDGE))?;
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
        renderer.write_descriptors()?;
        renderer.recreate_swapchain()?;

        Ok(renderer)
    }

    pub fn extent(&self) -> vk::Extent2D {
        self.extent
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }

        self.extent = vk::Extent2D { width, height };
        self.stale = true;
    }

    pub fn release_viewports(&mut self) -> Result<(), GpuError> {
        if self.viewports.is_empty() {
            return Ok(());
        }

        self.device.wait_idle()?;
        self.recorded = None;
        for target in std::mem::take(&mut self.viewports) {
            target.destroy(&mut self.device);
        }

        Ok(())
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

        let (swapchain, extent, _) = self.device.create_swapchain(
            self.device.surface,
            self.extent.width,
            self.extent.height,
            self.swapchain.as_ref(),
        )?;
        self.frames = Some(
            self.device
                .context
                .create_super_frame_allocator(swapchain.attachments.len().min(FRAMES_IN_FLIGHT)),
        );

        if let Some(old) = self.swapchain.replace(swapchain) {
            self.device.destroy_swapchain(old);
        }
        self.extent = extent;
        self.stale = false;

        Ok(())
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        let _ = self.device.wait_idle();
        self.recorded = None;

        for target in std::mem::take(&mut self.viewports) {
            target.destroy(&mut self.device);
        }

        if let Some(swapchain) = self.swapchain.take() {
            self.device.destroy_swapchain(swapchain);
        }

        if let Some(imgui) = self.imgui.as_mut() {
            imgui.destroy(&mut self.device);
        }

        self.bindless.destroy(&self.device);
        destroy_images(&mut self.device, loaded_images(std::mem::take(&mut self.sheets)));
        destroy_images(&mut self.device, self.fallback.take());
        self.device.allocator.deallocate_sampler(self.sampler);
        self.device.allocator.deallocate_sampler(self.blur_sampler);

        for buffers in std::mem::take(&mut self.cull) {
            buffers.destroy(&mut self.device);
        }
        for preview in std::mem::take(&mut self.previews).into_iter().flatten() {
            preview.destroy(&mut self.device);
        }
        if let Some(buffer) = self.sprites.take() {
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
