use ash::vk;
use vir::{ValueId, allocator::Allocator};

use super::{
    Renderer,
    gpu::{GpuLightTile, gpu_light_offset, gpu_light_tile, payload_bytes},
    sprites::{merged_ranges, updates_since, valid_update_range},
    upload::{device_buffer_info, upload_buffer},
};
use crate::{Frame, GpuError, MapViewFrame, UpdateRange};

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

        if self.uploaded_lighting.len() == frame.map_views.len() && self.prepare_lighting_update(frame)? {
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
        let Some(buffer) = self.lights else {
            return Ok(false);
        };

        let mut plans = Vec::new();
        for (index, (uploaded, view)) in self.uploaded_lighting.iter().zip(frame.map_views).enumerate() {
            if uploaded.revision == view.lighting.map(|lighting| lighting.revision) {
                continue;
            }

            let Some(ranges) = plan_lighting_update(uploaded, view) else {
                return Ok(false);
            };
            if uploaded.base as usize + uploaded.count > self.light_capacity {
                return Ok(false);
            }
            plans.push((index, ranges));
        }

        let mut payloads = Vec::new();
        for (index, ranges) in &plans {
            let Some(lighting) = frame.map_views[*index].lighting else {
                return Ok(false);
            };
            let base = self.uploaded_lighting[*index].base as usize;

            for range in ranges {
                let payload = (range.start - base..range.end - base)
                    .map(|local| gpu_light_tile(local, &lighting.tiles[local]))
                    .collect::<Result<Vec<_>, _>>()?;
                payloads.push((*range, payload));
            }
        }

        if payloads.iter().any(|(_, payload)| !payload.is_empty()) {
            self.graph.wait()?;
        }

        let writes = payloads
            .iter()
            .map(|(range, payload)| Ok((gpu_light_offset(range.start)?, payload_bytes(payload))))
            .collect::<Result<Vec<_>, GpuError>>()?;
        upload_buffer(&mut self.device, &mut self.graph, buffer, &writes)?;

        for (index, _) in &plans {
            let revision = frame.map_views[*index].lighting.map(|lighting| lighting.revision);
            self.uploaded_lighting[*index].revision = revision;
        }

        Ok(true)
    }
}

fn plan_lighting_update(uploaded: &UploadedLighting, view: &MapViewFrame<'_>) -> Option<Vec<UpdateRange>> {
    let lighting = view.lighting?;
    let updates = updates_since(lighting.pending_updates, uploaded.revision?, |update| {
        update.previous_revision
    })?;
    let ranges = merged_ranges(updates.iter().map(|update| update.tiles));
    let is_valid = uploaded.size == lighting.size
        && uploaded.count == lighting.tiles.len()
        && ranges
            .iter()
            .all(|range| valid_update_range(Some(*range), lighting.tiles.len()));
    if !is_valid {
        return None;
    }

    let base = uploaded.base as usize;

    Some(
        ranges
            .into_iter()
            .map(|range| UpdateRange {
                start: base + range.start,
                end: base + range.end,
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LightTile, LightingFrame, LightingUpdate, MapViewRect, renderer::fixtures::map_view};

    const TILES: [LightTile; 4] = [LightTile { corners: [[0.0; 3]; 4] }; 4];

    fn lit_view(revision: u64, size: [u32; 3], updates: &'static [LightingUpdate]) -> MapViewFrame<'static> {
        MapViewFrame {
            lighting: Some(LightingFrame {
                size,
                tiles: &TILES,
                tile_size: 32,
                minimum_brightness: 0.0,
                revision,
                pending_updates: updates,
            }),
            ..map_view(MapViewRect::default())
        }
    }

    fn uploaded(base: u32, revision: Option<u64>, size: [u32; 3]) -> UploadedLighting {
        UploadedLighting {
            revision,
            base,
            count: 4,
            size,
        }
    }

    static UPDATES: [LightingUpdate; 1] = [LightingUpdate {
        previous_revision: 5,
        tiles: UpdateRange { start: 1, end: 3 },
    }];

    #[test]
    fn a_later_view_updates_at_its_own_base() {
        let view = lit_view(6, [2, 2, 1], &UPDATES);

        assert_eq!(
            plan_lighting_update(&uploaded(100, Some(5), [2, 2, 1]), &view),
            Some(vec![UpdateRange { start: 101, end: 103 }])
        );
    }

    #[test]
    fn lighting_updates_fall_back_when_the_layout_changes() {
        let view = lit_view(6, [2, 2, 1], &UPDATES);

        assert_eq!(plan_lighting_update(&uploaded(0, Some(5), [4, 1, 1]), &view), None);
        assert_eq!(plan_lighting_update(&uploaded(0, Some(4), [2, 2, 1]), &view), None);
        assert_eq!(plan_lighting_update(&uploaded(0, None, [2, 2, 1]), &view), None);
        assert_eq!(
            plan_lighting_update(&uploaded(0, Some(5), [2, 2, 1]), &map_view(MapViewRect::default())),
            None
        );
    }
}
