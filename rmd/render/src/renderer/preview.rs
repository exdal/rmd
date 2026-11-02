use vir::{Buffer, RenderGraph, ValueId, allocator::Allocator};

use super::{
    Renderer,
    cull::{CullBuffers, CullSlots},
    draw::MapViewPlan,
    gpu::{GpuSprite, gpu_sprite_range, payload_bytes},
    record::CameraPush,
    sheets::{note_sheets, sheet_indices},
    sprites::VisibleRange,
    upload::{device_buffer_info, upload_buffer},
};
use crate::{Device, Frame, GpuError, SpritePreview};

pub(super) struct PreviewSlots {
    pub(super) sprites: ValueId,
    pub(super) camera: ValueId,
    pub(super) cull: CullSlots,
}

pub(super) struct UploadedPreview {
    revision: u64,
    pub(super) sprites: Buffer,
    pub(super) cull: CullBuffers,
    pub(super) sheets: Vec<u32>,
}

impl UploadedPreview {
    fn allocate(device: &mut Device, graph: &mut RenderGraph, preview: SpritePreview<'_>) -> Result<Self, GpuError> {
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
        let sprites = match device
            .allocator
            .allocate_buffer(&device_buffer_info(size, "region preview sprites"))
        {
            Ok(buffer) => buffer,
            Err(error) => {
                cull.destroy(device);
                return Err(error.into());
            },
        };

        if let Err(error) = upload_buffer(device, graph, sprites, &[(0, payload_bytes(&payload))]) {
            device.allocator.deallocate_buffer(sprites);
            cull.destroy(device);
            return Err(error);
        }

        Ok(Self {
            revision: preview.revision,
            sprites,
            cull,
            sheets: sheet_indices(preview.sprites),
        })
    }

    pub(super) fn destroy(self, device: &mut Device) {
        device.allocator.deallocate_buffer(self.sprites);
        self.cull.destroy(device);
    }
}

pub(super) fn preview_plan(camera: crate::Camera, preview: SpritePreview<'_>) -> Result<MapViewPlan, GpuError> {
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

impl Renderer {
    pub(super) fn prepare_previews(&mut self, frame: &Frame<'_>) -> Result<(), GpuError> {
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
            note_sheets(&mut self.requested_sheets, preview.sprites);
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
                upload_buffer(
                    &mut self.device,
                    &mut self.graph,
                    uploaded.sprites,
                    &[(0, payload_bytes(&payload))],
                )?;
                uploaded.revision = preview.revision;
                uploaded.sheets = sheet_indices(preview.sprites);
            } else {
                let uploaded = UploadedPreview::allocate(&mut self.device, &mut self.graph, preview)?;
                if let Some(previous) = slot.replace(uploaded) {
                    previous.destroy(&mut self.device);
                }
            }
        }
        Ok(())
    }
}
