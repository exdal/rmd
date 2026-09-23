pub mod color;
pub mod device;
pub mod error;
pub mod imgui;
pub mod renderer;
mod spec;
pub mod texture;

use dmm::{Coord, PrefabInstanceId};

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

/// A map-space guide segment rendered over a map view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GuideLine {
    pub origin: [f32; 2],
    pub target: [f32; 2],
}

// parallel to settings::SelectionHighlight to avoid circular dep
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HighlightStyle {
    /// marching stripes traced around the highlighted sprite
    #[default]
    Outline,
    /// flat accent tint over the highlighted sprite
    Tint,
}

impl HighlightStyle {
    pub const fn is_tint(self) -> bool { matches!(self, Self::Tint) }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MapViewInteraction {
    pub cursor: Option<[u32; 2]>,
    pub hovered_area: Option<PrefabInstanceId>,
    pub selected: Option<PrefabInstanceId>,
    pub placement_flash: Option<PlacementFlash>,
    pub highlight: HighlightStyle,
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
    Select,
    Delete,
    NodeSeed(Coord),
    NodeDelete(Coord),
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

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum SpriteLighting {
    #[default]
    Normal,
    Emissive,
    Blocker,
    OverlayLight,
    OverlayLightSubtract,
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
    pub lighting: SpriteLighting,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MapViewRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl MapViewRect {
    pub const fn is_empty(self) -> bool { self.width == 0 || self.height == 0 }
}

#[derive(Debug)]
pub struct MapViewFrame<'a> {
    pub rect: MapViewRect,
    pub camera: Camera,
    pub sprite_instances: &'a [SpriteInstance],
    pub area_tiles: &'a [SpriteInstance],
    pub focused_area: Option<PrefabInstanceId>,
    pub active_z: u32,
    pub level_count: u32,
    pub revision: u64,
    pub pending_update: Option<FrameUpdate>,
    pub lighting: Option<LightingFrame<'a>>,
    pub guide_lines: &'a [GuideLine],
    pub connected: &'a [PrefabInstanceId],
    pub interaction: MapViewInteraction,
    pub preview: Option<SpritePreview<'a>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightTile {
    pub corners: [[f32; 3]; 4],
}

#[derive(Debug, Clone, Copy)]
pub struct LightingFrame<'a> {
    pub size: [u32; 3],
    pub tiles: &'a [LightTile],
    pub tile_size: u32,
    /// Minimum displayed light luminance, normalized to 0..=1.
    pub minimum_brightness: f32,
    pub revision: u64,
    pub pending_update: Option<LightingUpdate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LightingUpdate {
    pub previous_revision: u64,
    pub tiles: UpdateRange,
}

#[derive(Debug, Clone, Copy)]
pub struct SpritePreview<'a> {
    pub sprites: &'a [SpriteInstance],
    pub revision: u64,
    pub offset: [f32; 2],
}

#[derive(Debug)]
pub struct Frame<'a> {
    pub map_views: &'a [MapViewFrame<'a>],
    pub underlay_depth: u32,
    pub show_areas: bool,
    pub show_area_outlines: bool,
    /// Index into `map_views` of the map under the mouse, if any. Only that one
    /// runs the pick pass.
    pub picking: Option<usize>,
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
