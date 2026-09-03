use ash::vk;
use render::{Frame, RenderError, Renderer, SpriteInstance, texture::TextureCatalog};
use vir::{
    Access,
    AllocatorKind,
    BlendPreset,
    Buffer,
    BufferInfo,
    ClearValue,
    GraphicsPipelineInfo,
    MemoryLocation,
    Module,
    PipelineId,
    Program,
    RasterizationState,
    Rect2D,
    RenderGraph,
    SuperFrameAllocator,
    SwapChain,
    ValueId,
    allocator::Allocator,
};

use crate::{
    device::Device,
    error::GpuError,
    resources::BindlessDescriptorSet,
    upload::{self, GpuTextures},
};

const VERTEX_SPIRV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sprite.vert.spv"));
const FRAGMENT_SPIRV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sprite.frag.spv"));

const BACKGROUND: ClearValue = ClearValue::rgba_f32(0.0, 0.0, 0.0, 1.0);

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct GpuSprite {
    position_size: [f32; 4],
    color: [f32; 4],
    texture: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct CameraPush {
    center: [f32; 2],
    viewport: [f32; 2],
    zoom: f32,
}

struct Recorded {
    program: Program,
    camera: ValueId,
    count: ValueId,
    sprites: ValueId,
}

pub struct VirRenderer {
    device: Device,
    graph: Option<RenderGraph>,
    pipeline: Option<PipelineId>,
    swapchain: Option<SwapChain>,
    frames: Option<SuperFrameAllocator>,
    recorded: Option<Recorded>,
    textures: Option<GpuTextures>,
    bindless: Option<BindlessDescriptorSet>,
    sprites: Option<Buffer>,
    sprite_capacity: usize,
    uploaded_revision: Option<u64>,
    extent: vk::Extent2D,
    stale: bool,
}

impl VirRenderer {
    pub fn new(device: Device, width: u32, height: u32) -> Result<Self, GpuError> {
        let mut renderer = Self {
            device,
            graph: None,
            pipeline: None,
            swapchain: None,
            frames: None,
            recorded: None,
            textures: None,
            bindless: None,
            sprites: None,
            sprite_capacity: 0,
            uploaded_revision: None,
            extent: vk::Extent2D { width, height },
            stale: true,
        };

        renderer.recreate_swapchain()?;

        Ok(renderer)
    }

    pub fn extent(&self) -> vk::Extent2D { self.extent }

    fn recreate_swapchain(&mut self) -> Result<(), GpuError> {
        self.device.wait_idle()?;
        self.recorded = None;

        let old = self.swapchain.take();
        let (swapchain, extent, _) =
            self.device
                .create_swapchain(self.extent.width, self.extent.height, old.as_ref())?;

        let image_count = swapchain.attachments.len();
        self.swapchain = Some(swapchain);
        self.frames = Some(self.device.context.create_super_frame_allocator(image_count));
        self.extent = extent;
        self.stale = false;

        Ok(())
    }

    fn record(&mut self) -> Result<(), GpuError> {
        let (Some(swapchain), Some(pipeline), Some(graph), Some(_)) = (
            self.swapchain.as_ref(),
            self.pipeline,
            self.graph.as_ref(),
            self.textures.as_ref(),
        ) else {
            return Ok(());
        };

        let mut module = Module::default();
        let camera = module.declare_bytes_var("camera", size_of::<CameraPush>() as u32);
        let count = module.declare_u32_var("sprite count", 0);
        let sprites = module.declare_buffer_var("sprites", Access::HostWrite);

        let swapchain_image = module.acquire_next_image(swapchain);
        let target = module.clear(swapchain_image, BACKGROUND);

        let drawn = module
            .begin_rendering([(target, Access::ColorRW)])
            .with_name("sprites")
            .bind_graphics_pipeline(pipeline)
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
            .draw(4u32, count)
            .end_rendering();

        let present = module.present(drawn);
        let program = module.compile(graph, present)?;

        self.recorded = Some(Recorded {
            program,
            camera,
            count,
            sprites,
        });
        self.uploaded_revision = None;

        Ok(())
    }

    fn ensure_sprite_buffer(&mut self, count: usize) -> Result<bool, GpuError> {
        if self.sprites.is_some() && self.sprite_capacity >= count {
            return Ok(false);
        }

        if let Some(old) = self.sprites.take() {
            self.device.allocator.deallocate_buffer(old);
        }

        let capacity = count.max(1024).next_power_of_two();
        let size = (capacity.saturating_mul(size_of::<GpuSprite>())) as u64;

        self.sprites = Some(self.device.allocator.allocate_buffer(
            &BufferInfo::new(size, vk::BufferUsageFlags::STORAGE_BUFFER, MemoryLocation::CpuToGpu).with_name("sprites"),
        )?);
        self.sprite_capacity = capacity;

        Ok(true)
    }

    fn write_sprites(&mut self, sprites: &[SpriteInstance]) -> Result<(), GpuError> {
        let payload = sprites
            .iter()
            .map(|sprite| GpuSprite {
                position_size: [
                    sprite.x,
                    sprite.y,
                    sprite.texture.width as f32,
                    sprite.texture.height as f32,
                ],
                color: sprite.color,
                texture: sprite.texture.index,
            })
            .collect::<Vec<_>>();

        if let Some(buffer) = self.sprites.as_mut() {
            buffer.write(0, &payload)?;
        }

        Ok(())
    }

    fn draw_inner(&mut self, frame: &Frame) -> Result<(), GpuError> {
        if self.stale {
            self.recreate_swapchain()?;
        }

        if self.recorded.is_none() {
            self.record()?;
        }

        if self.textures.is_none() {
            return Ok(());
        }

        let reallocated = self.ensure_sprite_buffer(frame.sprites.len())?;
        if reallocated || self.uploaded_revision != Some(frame.revision) {
            self.write_sprites(&frame.sprites)?;
            self.uploaded_revision = Some(frame.revision);
        }

        let push = CameraPush {
            center: [frame.camera.x, frame.camera.y],
            viewport: [self.extent.width as f32, self.extent.height as f32],
            zoom: frame.camera.zoom,
        };

        let (Some(recorded), Some(buffer)) = (self.recorded.as_mut(), self.sprites.as_ref()) else {
            return Ok(());
        };

        recorded.program.set_bytes(recorded.camera, &push);
        recorded.program.set(recorded.count, frame.sprites.len() as u32);
        recorded.program.set(recorded.sprites, buffer);

        let Some(frames) = self.frames.as_mut() else {
            return Ok(());
        };
        let next = frames.get_next_frame()?;

        let Some(graph) = self.graph.as_mut() else {
            return Ok(());
        };

        match graph.execute(&self.device.context, &recorded.program, &mut AllocatorKind::Frame(next)) {
            Ok(()) => Ok(()),
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR | vk::Result::SUBOPTIMAL_KHR) => {
                self.stale = true;

                Ok(())
            },
            Err(e) => Err(GpuError::Vulkan(e)),
        }
    }

    fn destroy_texture_state(&mut self) {
        self.recorded = None;
        self.pipeline = None;

        if let Some(graph) = self.graph.take() {
            drop(graph);
        }
        if let Some(bindless) = self.bindless.take() {
            bindless.destroy(&self.device);
        }
        if let Some(textures) = self.textures.take() {
            textures.destroy(&mut self.device);
        }
    }
}

impl Renderer for VirRenderer {
    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }

        self.extent = vk::Extent2D { width, height };
        self.stale = true;
    }

    fn upload_textures(&mut self, textures: &TextureCatalog) -> Result<(), RenderError> {
        self.device.wait_idle().map_err(RenderError::from)?;
        self.destroy_texture_state();

        let vertex = read_spirv(VERTEX_SPIRV).map_err(RenderError::from)?;
        let fragment = read_spirv(FRAGMENT_SPIRV).map_err(RenderError::from)?;
        let uploaded = upload::upload(&mut self.device, textures).map_err(RenderError::from)?;
        let bindless = match BindlessDescriptorSet::create(&self.device, &uploaded) {
            Ok(bindless) => bindless,
            Err(error) => {
                uploaded.destroy(&mut self.device);
                return Err(error.into());
            },
        };
        let mut graph = RenderGraph::new(&self.device.context);
        let pipeline = match graph.declare_pipeline(
            GraphicsPipelineInfo::new()
                .with_shader(&vertex)
                .with_shader(&fragment)
                .with_bindless_set(1, bindless.layout, bindless.set),
        ) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                drop(graph);
                bindless.destroy(&self.device);
                uploaded.destroy(&mut self.device);
                return Err(GpuError::from(error).into());
            },
        };

        self.graph = Some(graph);
        self.pipeline = Some(pipeline);
        self.textures = Some(uploaded);
        self.bindless = Some(bindless);

        Ok(())
    }

    fn draw(&mut self, frame: &Frame) -> Result<(), RenderError> { self.draw_inner(frame).map_err(RenderError::from) }
}

impl Drop for VirRenderer {
    fn drop(&mut self) {
        let _ = self.device.wait_idle();
        self.destroy_texture_state();

        if let Some(buffer) = self.sprites.take() {
            self.device.allocator.deallocate_buffer(buffer);
        }
    }
}

fn read_spirv(bytes: &[u8]) -> Result<Vec<u32>, GpuError> {
    let mut cursor = std::io::Cursor::new(bytes);

    ash::util::read_spv(&mut cursor).map_err(|e| GpuError::Loader(e.to_string()))
}
