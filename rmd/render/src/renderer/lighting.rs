use ash::vk;
use vir::{ValueId, allocator::Allocator};

use super::{
    Renderer,
    gpu::{GpuLightTile, gpu_light_offset, gpu_light_tile, payload_bytes},
    sprites::{merged_ranges, updates_since, valid_update_range},
    upload::{device_buffer_info, upload_buffer},
};
use crate::{Frame, GpuError};

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(super) struct LightingPush {
    pub(super) center: [f32; 2],
    pub(super) viewport: [f32; 2],
    pub(super) zoom: f32,
    pub(super) tile_size: f32,
    pub(super) map_size: [u32; 2],
    pub(super) level_count: u32,
    pub(super) base: u32,
    pub(super) minimum_brightness: f32,
}

pub(super) struct LightingSlots {
    pub(super) push: ValueId,
}

#[derive(Debug, Default)]
pub(super) struct UploadedLighting {
    pub(super) revision: Option<u64>,
    pub(super) base: u32,
    count: usize,
    pub(super) size: [u32; 3],
}

impl Renderer {
    pub(super) fn prepare_lighting(&mut self, frame: &Frame<'_>) -> Result<(), GpuError> {
        if self.uploaded_lighting.len() == frame.map_views.len()
            && self
                .uploaded_lighting
                .iter()
                .zip(frame.map_views)
                .all(|(uploaded, view)| uploaded.revision == view.lighting.map(|lighting| lighting.revision))
        {
            return Ok(());
        }

        if frame.map_views.len() == 1 && self.uploaded_lighting.len() == 1 && self.prepare_lighting_update(frame)? {
            return Ok(());
        }

        let total = frame
            .map_views
            .iter()
            .filter_map(|view| view.lighting)
            .try_fold(0usize, |total, lighting| total.checked_add(lighting.tiles.len()))
            .ok_or(GpuError::SpriteUploadTooLarge)?;

        u32::try_from(total).map_err(|_| GpuError::SpriteUploadTooLarge)?;

        let mut payload = Vec::with_capacity(total);
        let mut uploaded = Vec::with_capacity(frame.map_views.len());
        for view in frame.map_views {
            let base = u32::try_from(payload.len()).map_err(|_| GpuError::SpriteUploadTooLarge)?;
            let Some(lighting) = view.lighting else {
                uploaded.push(UploadedLighting {
                    base,
                    ..Default::default()
                });
                continue;
            };
            let expected = lighting
                .size
                .into_iter()
                .try_fold(1usize, |length, value| length.checked_mul(value as usize));
            if expected != Some(lighting.tiles.len()) {
                return Err(vk::Result::ERROR_INITIALIZATION_FAILED.into());
            }
            for (index, tile) in lighting.tiles.iter().enumerate() {
                payload.push(gpu_light_tile(index, tile)?);
            }
            uploaded.push(UploadedLighting {
                revision: Some(lighting.revision),
                base,
                count: lighting.tiles.len(),
                size: lighting.size,
            });
        }

        self.device.wait_idle()?;
        if total > 0 && (self.lights.is_none() || self.light_capacity < total) {
            let capacity = total
                .max(1024)
                .checked_next_power_of_two()
                .ok_or(GpuError::SpriteUploadTooLarge)?;
            let size = capacity
                .checked_mul(size_of::<GpuLightTile>())
                .and_then(|size| u64::try_from(size).ok())
                .ok_or(GpuError::SpriteUploadTooLarge)?;
            let buffer = self
                .device
                .allocator
                .allocate_buffer(&device_buffer_info(size, "static light tiles"))?;
            if let Err(error) = upload_buffer(
                &mut self.device,
                &mut self.graph,
                buffer,
                &[(0, payload_bytes(&payload))],
            ) {
                self.device.allocator.deallocate_buffer(buffer);
                return Err(error);
            }

            if let Some(old) = self.lights.replace(buffer) {
                self.device.allocator.deallocate_buffer(old);
            }

            self.light_capacity = capacity;
        } else if total > 0
            && let Some(buffer) = self.lights
        {
            upload_buffer(
                &mut self.device,
                &mut self.graph,
                buffer,
                &[(0, payload_bytes(&payload))],
            )?;
        }
        self.uploaded_lighting = uploaded;

        Ok(())
    }

    fn prepare_lighting_update(&mut self, frame: &Frame<'_>) -> Result<bool, GpuError> {
        let (Some(view), Some(uploaded)) = (frame.map_views.first(), self.uploaded_lighting.first()) else {
            return Ok(false);
        };
        let (Some(lighting), Some(revision), Some(buffer)) = (view.lighting, uploaded.revision, self.lights) else {
            return Ok(false);
        };
        let Some(updates) = updates_since(lighting.pending_updates, revision, |update| update.previous_revision) else {
            return Ok(false);
        };

        let ranges = merged_ranges(updates.iter().map(|update| update.tiles));
        if uploaded.base != 0
            || uploaded.size != lighting.size
            || uploaded.count != lighting.tiles.len()
            || lighting.tiles.len() > self.light_capacity
            || !ranges
                .iter()
                .all(|range| valid_update_range(Some(*range), lighting.tiles.len()))
        {
            return Ok(false);
        }

        let payloads = ranges
            .iter()
            .map(|range| {
                lighting.tiles[range.start..range.end]
                    .iter()
                    .enumerate()
                    .map(|(offset, tile)| gpu_light_tile(range.start + offset, tile))
                    .collect::<Result<Vec<_>, _>>()
            })
            .collect::<Result<Vec<_>, _>>()?;
        if payloads.iter().any(|payload| !payload.is_empty()) {
            self.graph.wait()?;
        }
        let writes = ranges
            .iter()
            .zip(&payloads)
            .map(|(range, payload)| Ok((gpu_light_offset(range.start)?, payload_bytes(payload))))
            .collect::<Result<Vec<_>, GpuError>>()?;
        upload_buffer(&mut self.device, &mut self.graph, buffer, &writes)?;

        let Some(uploaded) = self.uploaded_lighting.first_mut() else {
            return Ok(false);
        };

        uploaded.revision = Some(lighting.revision);

        Ok(true)
    }
}
