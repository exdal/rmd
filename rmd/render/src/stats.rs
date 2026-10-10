use std::time::{Duration, Instant};

use ash::vk;
pub use vir::PassTiming;

use crate::device::{DeviceInfo, HeapUsage};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderSpan {
    PrepareUi,
    Record,
    UploadSprites,
    UploadLighting,
    CullBuffers,
    Previews,
    Bind,
    Execute,
    PickReadback,
    Stall,
}

impl RenderSpan {
    pub const ALL: [Self; 10] = [
        Self::PrepareUi,
        Self::Record,
        Self::UploadSprites,
        Self::UploadLighting,
        Self::CullBuffers,
        Self::Previews,
        Self::Bind,
        Self::Execute,
        Self::PickReadback,
        Self::Stall,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::PrepareUi => "imgui prepare",
            Self::Record => "graph record",
            Self::UploadSprites => "sprite upload",
            Self::UploadLighting => "lighting upload",
            Self::CullBuffers => "cull buffers",
            Self::Previews => "preview upload",
            Self::Bind => "bind frame values",
            Self::Execute => "graph execute",
            Self::PickReadback => "pick readback",
            Self::Stall => "gpu waits (inside the above)",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Upload {
    #[default]
    None,
    Incremental {
        bytes: u64,
    },
    Full {
        bytes: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ViewSprites {
    pub underlay: u32,
    pub active: u32,
}

#[derive(Debug, Clone, Default)]
pub struct RenderStats {
    cpu: [Duration; RenderSpan::ALL.len()],
    pub gpu: Vec<PassTiming>,
    pub sprite_upload: Upload,
    pub lighting_upload: Upload,
    pub is_recorded: bool,
    pub sprite_count: usize,
    pub views: Vec<ViewSprites>,
}

impl RenderStats {
    pub fn cpu(&self, span: RenderSpan) -> Duration { self.cpu[span as usize] }

    pub(crate) fn add(&mut self, span: RenderSpan, since: Instant) { self.cpu[span as usize] += since.elapsed(); }

    pub(crate) fn measure<T>(&mut self, span: RenderSpan, f: impl FnOnce() -> T) -> T {
        let clock = Instant::now();
        let result = f();
        self.add(span, clock);

        result
    }

    pub(crate) fn reset(&mut self) {
        self.cpu = Default::default();
        self.gpu.clear();
        self.sprite_upload = Upload::None;
        self.lighting_upload = Upload::None;
        self.is_recorded = false;
        self.sprite_count = 0;
        self.views.clear();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    Vram,
    HostVisibleVram,
    SystemRam,
}

impl Placement {
    pub fn from_flags(flags: vk::MemoryPropertyFlags) -> Self {
        let is_device_local = flags.contains(vk::MemoryPropertyFlags::DEVICE_LOCAL);
        let is_host_visible = flags.contains(vk::MemoryPropertyFlags::HOST_VISIBLE);
        match (is_device_local, is_host_visible) {
            (true, true) => Self::HostVisibleVram,
            (true, false) => Self::Vram,
            (false, _) => Self::SystemRam,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Vram => "VRAM",
            Self::HostVisibleVram => "VRAM, host-visible",
            Self::SystemRam => "system RAM",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BufferPlacement {
    pub bytes: u64,
    pub placement: Option<Placement>,
}

#[derive(Debug, Clone)]
pub struct RendererInfo {
    pub device: DeviceInfo,
    pub swapchain_images: usize,
    pub has_timestamps: bool,
    pub texture_count: usize,
    pub texture_bytes: u64,
    pub sprites: Option<BufferPlacement>,
    pub lights: Option<BufferPlacement>,
    pub heaps: Vec<HeapUsage>,
}
