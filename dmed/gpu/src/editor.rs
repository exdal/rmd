use ash::vk;
use dear_imgui_rs::render::PendingFrame;
use render::{Frame, texture::TextureCatalog};
use vir::{
    AllocatorKind,
    ClearValue,
    ImageInfo,
    Module,
    Program,
    RenderGraph,
    SuperFrameAllocator,
    SwapChain,
    ValueId,
};

use crate::{
    device::Device,
    error::GpuError,
    imgui::{ImGuiPass, ImGuiSlots},
    sprite::{SpritePass, SpriteSlots},
};

const MAP_BACKGROUND: ClearValue = ClearValue::rgba_f32(0.0, 0.0, 0.0, 1.0);
const UI_BACKGROUND: ClearValue = ClearValue::rgba_f32(0.01, 0.01, 0.01, 1.0);
const VIEWPORT_FORMAT: vk::Format = vk::Format::R8G8B8A8_SRGB;

struct Recorded {
    program: Program,
    viewport_extent: ValueId,
    sprites: Option<SpriteSlots>,
    ui: Option<ImGuiSlots>,
}

pub struct EditorRenderer {
    device: Device,
    graph: Option<RenderGraph>,
    swapchain: Option<SwapChain>,
    frames: Option<SuperFrameAllocator>,
    recorded: Option<Recorded>,
    sprite: SpritePass,
    imgui: ImGuiPass,
    extent: vk::Extent2D,
    viewport: vk::Extent2D,
    stale: bool,
}

impl EditorRenderer {
    pub const VIEWPORT_TEXTURE: dear_imgui_rs::TextureId = crate::imgui::VIEWPORT_TEXTURE;

    pub fn new(device: Device, width: u32, height: u32) -> Result<Self, GpuError> {
        let mut device = device;
        let imgui = ImGuiPass::new(&mut device)?;

        let mut renderer = Self {
            device,
            graph: None,
            swapchain: None,
            frames: None,
            recorded: None,
            sprite: SpritePass::new(),
            imgui,
            extent: vk::Extent2D { width, height },
            viewport: vk::Extent2D { width, height },
            stale: true,
        };

        renderer.recreate_swapchain()?;
        renderer.recreate_graph()?;

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
        self.device.wait_idle()?;
        self.recorded = None;
        self.graph = None;

        self.sprite.upload_textures(&mut self.device, catalog)?;

        self.recreate_graph()
    }

    pub fn reset_imgui_textures(&mut self) -> Result<(), GpuError> { self.imgui.reset_textures(&mut self.device) }

    pub fn draw(&mut self, frame: &Frame, viewport: (u32, u32), pending: PendingFrame<'_>) -> Result<(), GpuError> {
        if pending.draw_requirements().requires_raw_callback_support() {
            return Err(GpuError::RawDrawCallback);
        }

        let viewport = vk::Extent2D {
            width: viewport.0.max(1),
            height: viewport.1.max(1),
        };

        if self.stale {
            self.recreate_swapchain()?;
        }

        self.viewport = viewport;

        if self.recorded.is_none() {
            self.record()?;
        }

        let Self {
            device,
            graph,
            frames,
            recorded,
            sprite,
            imgui,
            extent,
            stale,
            ..
        } = self;
        let (Some(graph), Some(frames), Some(recorded)) = (graph.as_mut(), frames.as_mut(), recorded.as_mut()) else {
            return Ok(());
        };

        let next = frames.get_next_frame()?;

        let feedback = imgui.poll_textures(device, graph, next, pending.texture_requests())?;
        let reconciled = pending
            .reconcile_texture_feedback(feedback)
            .map_err(|e| GpuError::ImGui(e.to_string()))?;
        let ui_frame = imgui.prepare(next, reconciled.draw_data(), *extent)?;

        sprite.prepare(device, frame)?;

        recorded.program.set(recorded.viewport_extent, extent3d(viewport));

        if let Some(slots) = recorded.sprites.as_ref() {
            sprite.bind(&mut recorded.program, slots, frame, viewport);
        }
        if let Some(slots) = recorded.ui.as_ref() {
            imgui.bind(&mut recorded.program, slots, &ui_frame);
        }

        match graph.execute(&device.context, &recorded.program, &mut AllocatorKind::Frame(next)) {
            Ok(()) => Ok(()),
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR | vk::Result::SUBOPTIMAL_KHR) => {
                *stale = true;

                Ok(())
            },
            Err(e) => Err(GpuError::Vulkan(e)),
        }
    }

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

    fn recreate_graph(&mut self) -> Result<(), GpuError> {
        self.recorded = None;
        self.graph = None;

        let mut graph = RenderGraph::new(&self.device.context);
        self.sprite.declare_pipeline(&mut graph)?;
        self.imgui.declare_pipeline(&mut graph)?;
        self.graph = Some(graph);

        Ok(())
    }

    fn record(&mut self) -> Result<(), GpuError> {
        let (Some(swapchain), Some(graph)) = (self.swapchain.as_ref(), self.graph.as_ref()) else {
            return Ok(());
        };

        let mut module = Module::default();
        let swapchain_image = module.acquire_next_image(swapchain);

        let viewport_extent = module.declare_extent_3d_var("viewport extent", extent3d(self.viewport));
        let info = ImageInfo::color_target(self.viewport, VIEWPORT_FORMAT)
            .with_usage(vk::ImageUsageFlags::SAMPLED)
            .with_name("map viewport");
        let viewport = module.transient_image_sized(&info, viewport_extent);
        let viewport = module.clear(viewport, MAP_BACKGROUND);
        let (viewport, sprites) = self.sprite.record(&mut module, viewport);

        let target = module.clear(swapchain_image, UI_BACKGROUND);
        let (drawn, ui) = self.imgui.record(&mut module, target, viewport);

        let present = module.present(drawn);
        let program = module.compile(graph, present)?;

        self.recorded = Some(Recorded {
            program,
            viewport_extent,
            sprites,
            ui,
        });

        Ok(())
    }
}

impl Drop for EditorRenderer {
    fn drop(&mut self) {
        let _ = self.device.wait_idle();
        self.recorded = None;
        self.graph = None;
        self.imgui.destroy(&mut self.device);
        self.sprite.destroy(&mut self.device);
    }
}

fn extent3d(extent: vk::Extent2D) -> vk::Extent3D {
    vk::Extent3D {
        width: extent.width,
        height: extent.height,
        depth: 1,
    }
}
