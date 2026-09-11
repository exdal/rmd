pub mod color;
pub mod device;
pub mod error;
pub mod imgui;
pub mod renderer;
pub mod texture;

use dmm::PrefabInstanceId;

pub use crate::{device::Device, error::GpuError, renderer::Renderer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VisibilityId(std::num::NonZeroU32);

impl VisibilityId {
    pub fn from_raw(raw: u32) -> Option<Self> { std::num::NonZeroU32::new(raw).map(Self) }

    pub fn from_sprite_index(index: usize) -> Option<Self> {
        let index = u32::try_from(index).ok()?;

        index.checked_add(1).and_then(Self::from_raw)
    }

    pub const fn get(self) -> u32 { self.0.get() }

    pub const fn sprite_index(self) -> usize { self.get() as usize - 1 }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacementFlash {
    pub owner: PrefabInstanceId,
    pub strength: f32,
}

// this is for our tile origin -> pixel_x/y override indicator
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SelectionGuide {
    pub origin: [f32; 2],
    pub target: [f32; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ViewportInteraction {
    pub cursor: Option<[u32; 2]>,
    pub hovered_area: Option<PrefabInstanceId>,
    pub selected: Option<PrefabInstanceId>,
    pub selection_guide: Option<SelectionGuide>,
    pub placement_flash: Option<PlacementFlash>,
    pub mode: InteractionMode,
}

// parallel to tool::Tool to avoid circular dep
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InteractionMode {
    #[default]
    Place,
    Select {
        pick: Option<PickRequest>,
    },
    Delete {
        pick: Option<PickRequest>,
    },
}

impl InteractionMode {
    pub const fn pick(self) -> Option<PickRequest> {
        match self {
            Self::Place => None,
            Self::Select { pick } | Self::Delete { pick } => pick,
        }
    }

    pub const fn is_delete(self) -> bool { matches!(self, Self::Delete { .. }) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickRequest {
    Cursor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PickResult {
    #[default]
    Miss,
    Hit(PrefabInstanceId),
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SpriteTexture {
    pub index: u32,
    /// Pixel-space top-left corner within the DMI sheet.
    pub source_position: [u32; 2],
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpriteInstance {
    pub owner: PrefabInstanceId,
    pub area_owner: Option<PrefabInstanceId>,
    pub texture: SpriteTexture,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub z: u32,
    pub is_area: bool,
    pub area_edges: u32,
    /// premultiplied RGBA
    pub color: [f32; 4],
    /// sort key, from plane and layer
    pub depth: f32,
}

pub const AREA_EDGE_NORTH: u32 = 1 << 0;
pub const AREA_EDGE_EAST: u32 = 1 << 1;
pub const AREA_EDGE_SOUTH: u32 = 1 << 2;
pub const AREA_EDGE_WEST: u32 = 1 << 3;
pub const AREA_EDGES_ALL: u32 = AREA_EDGE_NORTH | AREA_EDGE_EAST | AREA_EDGE_SOUTH | AREA_EDGE_WEST;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub x: f32,
    pub y: f32,
    pub zoom: f32,
    pub viewport_width: u32,
    pub viewport_height: u32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
            viewport_width: 1280,
            viewport_height: 720,
        }
    }
}

#[derive(Debug)]
pub struct Frame<'a> {
    pub sprite_instances: &'a [SpriteInstance],
    pub area_tiles: &'a [SpriteInstance],
    pub focused_area: Option<PrefabInstanceId>,
    pub active_z: u32,
    pub underlay_depth: u32,
    pub show_areas: bool,
    pub show_area_outlines: bool,
    pub camera: Camera,
    pub revision: u64,
    pub pending_update: Option<FrameUpdate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameUpdate {
    pub previous_revision: u64,
    pub sprites: Option<UpdateRange>,
    pub area_tiles: Option<UpdateRange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpdateRange {
    pub start: usize,
    pub end: usize,
}

pub(crate) fn extent3d(extent: ash::vk::Extent2D) -> ash::vk::Extent3D {
    ash::vk::Extent3D {
        width: extent.width,
        height: extent.height,
        depth: 1,
    }
}

pub(crate) fn read_spirv(bytes: &[u8]) -> Result<Vec<u32>, GpuError> {
    let mut cursor = std::io::Cursor::new(bytes);

    ash::util::read_spv(&mut cursor).map_err(|e| GpuError::Loader(e.to_string()))
}

#[cfg(test)]
mod tests {
    use crate::VisibilityId;

    #[test]
    fn visibility_ids_are_one_based_sprite_indices() {
        assert_eq!(VisibilityId::from_raw(0), None);
        assert_eq!(VisibilityId::from_sprite_index(0).unwrap().get(), 1);
        assert_eq!(VisibilityId::from_raw(17).unwrap().sprite_index(), 16);
        assert_eq!(VisibilityId::from_sprite_index(u32::MAX as usize), None);
    }
}
