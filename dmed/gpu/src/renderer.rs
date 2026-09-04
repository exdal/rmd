use ash::vk;
use render::{Frame, RenderError, Renderer, texture::TextureCatalog};
use vir::{AllocatorKind, ClearValue, Module, Program, RenderGraph, SuperFrameAllocator, SwapChain};

use crate::{
    device::Device,
    error::GpuError,
    sprite::{SpritePass, SpriteSlots},
};

const BACKGROUND: ClearValue = ClearValue::rgba_f32(0.0, 0.0, 0.0, 1.0);

struct Recorded {
    program: Program,
    sprites: Option<SpriteSlots>,
}

pub struct VirRenderer {
    device: Device,
    graph: Option<RenderGraph>,
    swapchain: Option<SwapChain>,
    frames: Option<SuperFrameAllocator>,
    recorded: Option<Recorded>,
    sprite: SpritePass,
    extent: vk::Extent2D,
    stale: bool,
}

impl VirRenderer {
    pub fn new(device: Device, width: u32, height: u32) -> Result<Self, GpuError> {
        let mut renderer = Self {
            device,
            graph: None,
            swapchain: None,
            frames: None,
            recorded: None,
            sprite: SpritePass::new(),
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
        let (Some(swapchain), Some(graph)) = (self.swapchain.as_ref(), self.graph.as_ref()) else {
            return Ok(());
        };

        let mut module = Module::default();
        let swapchain_image = module.acquire_next_image(swapchain);
        let target = module.clear(swapchain_image, BACKGROUND);

        let (drawn, sprites) = self.sprite.record(&mut module, target);
        let present = module.present(drawn);
        let program = module.compile(graph, present)?;

        self.recorded = Some(Recorded { program, sprites });

        Ok(())
    }

    fn draw_inner(&mut self, frame: &Frame) -> Result<(), GpuError> {
        if self.stale {
            self.recreate_swapchain()?;
        }

        if !self.sprite.is_ready() {
            return Ok(());
        }

        if self.recorded.is_none() {
            self.record()?;
        }

        self.sprite.prepare(&mut self.device, frame)?;

        let (Some(recorded), Some(frames), Some(graph)) =
            (self.recorded.as_mut(), self.frames.as_mut(), self.graph.as_mut())
        else {
            return Ok(());
        };
        let Some(slots) = recorded.sprites.as_ref() else {
            return Ok(());
        };

        self.sprite.bind(&mut recorded.program, slots, frame, self.extent);

        let next = frames.get_next_frame()?;

        match graph.execute(&self.device.context, &recorded.program, &mut AllocatorKind::Frame(next)) {
            Ok(()) => Ok(()),
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR | vk::Result::SUBOPTIMAL_KHR) => {
                self.stale = true;

                Ok(())
            },
            Err(e) => Err(GpuError::Vulkan(e)),
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

        self.recorded = None;
        self.graph = None;

        self.sprite
            .upload_textures(&mut self.device, textures)
            .map_err(RenderError::from)?;

        let mut graph = RenderGraph::new(&self.device.context);
        self.sprite.declare_pipeline(&mut graph).map_err(RenderError::from)?;
        self.graph = Some(graph);

        Ok(())
    }

    fn draw(&mut self, frame: &Frame) -> Result<(), RenderError> { self.draw_inner(frame).map_err(RenderError::from) }
}

impl Drop for VirRenderer {
    fn drop(&mut self) {
        let _ = self.device.wait_idle();
        self.recorded = None;
        self.graph = None;
        self.sprite.destroy(&mut self.device);
    }
}
