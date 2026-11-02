use ash::vk;
use vir::allocator::Allocator;

use super::{
    Renderer,
    gpu::{GpuSprite, gpu_sprite, gpu_sprite_offset, gpu_sprite_range, payload_bytes},
    sheets::{merge_sheets, note_sheets, sheet_indices},
    upload::{device_buffer_info, upload_buffer},
};
use crate::{Frame, GpuError, SpriteInstance};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LevelRange {
    z: u32,
    base: u32,
    count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct VisibleRange {
    pub(super) base: u32,
    pub(super) count: u32,
    pub(super) underlay_count: u32,
}

#[derive(Debug, Default)]
pub(super) struct UploadedMapView {
    pub(super) revision: u64,
    pub(super) base: u32,
    count: usize,
    pub(super) ranges: Vec<LevelRange>,
    pub(super) sheets: Vec<u32>,
}

impl Renderer {
    pub(super) fn prepare_sprites(&mut self, frame: &Frame<'_>) -> Result<(), GpuError> {
        if self.sprites.is_some()
            && self.uploaded.len() == frame.map_views.len()
            && self
                .uploaded
                .iter()
                .zip(frame.map_views)
                .all(|(uploaded, view)| uploaded.revision == view.revision)
        {
            return Ok(());
        }

        if frame.map_views.len() == 1 && self.uploaded.len() == 1 && self.prepare_frame_update(frame)? {
            return Ok(());
        }

        let total: usize = frame.map_views.iter().map(|view| view.sprite_instances.len()).sum();
        u32::try_from(total).map_err(|_| GpuError::SpriteUploadTooLarge)?;
        let mut payload = Vec::with_capacity(total);

        let mut uploaded = Vec::with_capacity(frame.map_views.len());
        for view in frame.map_views {
            let base = u32::try_from(payload.len()).map_err(|_| GpuError::SpriteUploadTooLarge)?;

            for sprite in view.sprite_instances {
                let index = payload.len();
                payload.push(gpu_sprite(index, sprite)?);
            }

            note_sheets(&mut self.requested_sheets, view.sprite_instances);

            uploaded.push(UploadedMapView {
                revision: view.revision,
                base,
                count: view.sprite_instances.len(),
                ranges: level_ranges(view.sprite_instances),
                sheets: sheet_indices(view.sprite_instances),
            });
        }

        self.device.wait_idle()?;
        self.uploaded.clear();

        if self.sprites.is_none() || self.sprite_capacity < total {
            let capacity = total
                .max(1024)
                .checked_next_power_of_two()
                .ok_or(GpuError::SpriteUploadTooLarge)?;
            let size = capacity
                .checked_mul(size_of::<GpuSprite>())
                .ok_or(GpuError::SpriteUploadTooLarge)?;
            let size = u64::try_from(size).map_err(|_| GpuError::SpriteUploadTooLarge)?;
            let buffer = self
                .device
                .allocator
                .allocate_buffer(&device_buffer_info(size, "sprites"))?;
            if let Err(error) = upload_buffer(
                &mut self.device,
                &mut self.graph,
                buffer,
                &[(0, payload_bytes(&payload))],
            ) {
                self.device.allocator.deallocate_buffer(buffer);

                return Err(error);
            }

            if let Some(old) = self.sprites.replace(buffer) {
                self.device.allocator.deallocate_buffer(old);
            }

            self.sprite_capacity = capacity;
        } else if let Some(buffer) = self.sprites {
            upload_buffer(
                &mut self.device,
                &mut self.graph,
                buffer,
                &[(0, payload_bytes(&payload))],
            )?;
        }

        // area colors sample the sprites' sheets, so those have to be loaded first
        self.load_requested_sheets()?;

        let sprites = *self.sprites.as_ref().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
        self.color_area_outlines(sprites, crate::UpdateRange { start: 0, end: total })?;

        self.uploaded = uploaded;

        Ok(())
    }

    fn prepare_frame_update(&mut self, frame: &Frame<'_>) -> Result<bool, GpuError> {
        let (Some(view), Some(uploaded)) = (frame.map_views.first(), self.uploaded.first()) else {
            return Ok(false);
        };
        let Some(updates) = updates_since(view.pending_updates, uploaded.revision, |update| {
            update.previous_revision
        }) else {
            return Ok(false);
        };
        let ranges = merged_ranges(updates.iter().flat_map(|update| update.sprites.iter().copied()));
        if uploaded.base != 0
            || view.sprite_instances.len() > self.sprite_capacity
            || !ranges
                .iter()
                .all(|range| valid_update_range(Some(*range), view.sprite_instances.len()))
            || (uploaded.count != view.sprite_instances.len() && ranges.is_empty())
        {
            return Ok(false);
        }
        let Some(sprites) = self.sprites else {
            return Ok(false);
        };

        let sprite_payloads = ranges
            .iter()
            .map(|range| gpu_sprite_range(view.sprite_instances, *range))
            .collect::<Result<Vec<_>, _>>()?;
        let mut packed = Vec::new();
        for range in &ranges {
            note_sheets(&mut packed, &view.sprite_instances[range.start..range.end]);
        }
        self.requested_sheets.extend_from_slice(&packed);
        if sprite_payloads.iter().any(|payload| !payload.is_empty()) {
            self.graph.wait()?;
        }

        let writes = ranges
            .iter()
            .zip(&sprite_payloads)
            .map(|(range, payload)| Ok((gpu_sprite_offset(range.start)?, payload_bytes(payload))))
            .collect::<Result<Vec<_>, GpuError>>()?;

        upload_buffer(&mut self.device, &mut self.graph, sprites, &writes)?;
        self.load_requested_sheets()?;

        for range in &ranges {
            self.color_area_outlines(sprites, *range)?;
        }

        let Some(uploaded) = self.uploaded.first_mut() else {
            return Ok(false);
        };
        if uploaded.count != view.sprite_instances.len() {
            uploaded.ranges = level_ranges(view.sprite_instances);
        }
        uploaded.revision = view.revision;
        uploaded.count = view.sprite_instances.len();
        merge_sheets(&mut uploaded.sheets, &packed);

        Ok(true)
    }
}

pub(super) fn updates_since<T>(updates: &[T], uploaded: u64, previous_revision: impl Fn(&T) -> u64) -> Option<&[T]> {
    let start = updates
        .iter()
        .position(|update| previous_revision(update) == uploaded)?;

    Some(&updates[start..])
}

pub(super) fn merged_ranges(ranges: impl Iterator<Item = crate::UpdateRange>) -> Vec<crate::UpdateRange> {
    let mut ranges = ranges.collect::<Vec<_>>();
    ranges.sort_unstable_by_key(|range| range.start);
    ranges.dedup_by(|next, kept| {
        let is_joined = next.start <= kept.end;
        if is_joined {
            kept.end = kept.end.max(next.end);
        }

        is_joined
    });

    ranges
}

pub(super) fn valid_update_range(range: Option<crate::UpdateRange>, len: usize) -> bool {
    range.is_none_or(|range| range.start <= range.end && range.end <= len)
}

fn level_ranges(sprite_instances: &[SpriteInstance]) -> Vec<LevelRange> {
    let mut ranges: Vec<LevelRange> = Vec::new();

    for (index, sprite) in sprite_instances.iter().enumerate() {
        if let Some(range) = ranges.last_mut()
            && range.z == sprite.z
        {
            range.count += 1;
        } else {
            ranges.push(LevelRange {
                z: sprite.z,
                base: index as u32,
                count: 1,
            });
        }
    }

    ranges
}

pub(super) fn visible_range(ranges: &[LevelRange], active_z: u32, underlay_depth: u32) -> VisibleRange {
    let minimum_z = active_z.saturating_sub(underlay_depth).max(1);
    let mut underlays = (0, 0);
    let mut active = (0, 0);

    for range in ranges {
        if (minimum_z..active_z).contains(&range.z) {
            if underlays.1 == 0 {
                underlays.0 = range.base;
            }
            underlays.1 += range.count;
        } else if range.z == active_z {
            active = (range.base, range.count);
        }
    }

    VisibleRange {
        base: if underlays.1 == 0 { active.0 } else { underlays.0 },
        count: underlays.1 + active.1,
        underlay_count: underlays.1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{UpdateRange, renderer::fixtures::sprite};

    #[test]
    fn update_ranges_must_fit_their_current_buffer() {
        assert!(valid_update_range(None, 2));
        assert!(valid_update_range(Some(UpdateRange { start: 1, end: 2 }), 2));
        assert!(valid_update_range(Some(UpdateRange { start: 2, end: 2 }), 2));
        assert!(!valid_update_range(Some(UpdateRange { start: 2, end: 1 }), 2));
        assert!(!valid_update_range(Some(UpdateRange { start: 0, end: 3 }), 2));
    }

    #[test]
    fn adjacent_instances_are_grouped_into_level_ranges() {
        let sprites = [sprite(1), sprite(1), sprite(2), sprite(4), sprite(4), sprite(4)];

        assert_eq!(
            level_ranges(&sprites),
            [
                LevelRange {
                    z: 1,
                    base: 0,
                    count: 2
                },
                LevelRange {
                    z: 2,
                    base: 2,
                    count: 1
                },
                LevelRange {
                    z: 4,
                    base: 3,
                    count: 3
                },
            ]
        );
    }

    #[test]
    fn pending_updates_resume_from_the_uploaded_revision() {
        let chain = [(10, 'a'), (11, 'b'), (12, 'c')];
        let previous_revision = |update: &(u64, char)| update.0;

        assert_eq!(updates_since(&chain, 11, previous_revision), Some(&chain[1..]));
        assert_eq!(updates_since(&chain, 10, previous_revision), Some(&chain[..]));
        assert_eq!(
            updates_since(&chain, 9, previous_revision),
            None,
            "the chain no longer reaches back to what the renderer has"
        );
        assert_eq!(updates_since(&[], 10, previous_revision), None);
    }

    #[test]
    fn update_ranges_merge_where_they_overlap_or_touch() {
        let range = |start, end| UpdateRange { start, end };

        assert_eq!(
            merged_ranges(
                [
                    range(512, 768),
                    range(0, 256),
                    range(256, 512),
                    range(1024, 1280),
                    range(600, 700)
                ]
                .into_iter()
            ),
            [range(0, 768), range(1024, 1280)]
        );
        assert!(merged_ranges(std::iter::empty()).is_empty());
    }

    #[test]
    fn visible_ranges_select_only_the_active_level_and_configured_underlays() {
        let ranges = [
            LevelRange {
                z: 1,
                base: 0,
                count: 2,
            },
            LevelRange {
                z: 2,
                base: 2,
                count: 3,
            },
            LevelRange {
                z: 3,
                base: 5,
                count: 4,
            },
            LevelRange {
                z: 4,
                base: 9,
                count: 5,
            },
        ];

        assert_eq!(
            visible_range(&ranges, 3, 2),
            VisibleRange {
                base: 0,
                count: 9,
                underlay_count: 5,
            }
        );
    }

    /// The active level leads the merged range whenever nothing underneath it is shown, so the
    /// cull's `active_first_relative` is zero rather than the active level's own base.
    #[test]
    fn a_view_without_underlays_starts_at_the_active_level() {
        let ranges = [
            LevelRange {
                z: 1,
                base: 0,
                count: 2,
            },
            LevelRange {
                z: 2,
                base: 2,
                count: 3,
            },
        ];

        assert_eq!(
            visible_range(&ranges, 2, 0),
            VisibleRange {
                base: 2,
                count: 3,
                underlay_count: 0,
            }
        );
        assert_eq!(visible_range(&ranges, 9, 2), VisibleRange::default());
    }

    #[test]
    fn underlay_depth_clamps_at_the_bottom_and_can_be_disabled() {
        let ranges = [
            LevelRange {
                z: 1,
                base: 0,
                count: 2,
            },
            LevelRange {
                z: 2,
                base: 2,
                count: 3,
            },
        ];

        assert_eq!(
            visible_range(&ranges, 2, 9),
            VisibleRange {
                base: 0,
                count: 5,
                underlay_count: 2,
            }
        );
        assert_eq!(
            visible_range(&ranges, 2, 0),
            VisibleRange {
                base: 2,
                count: 3,
                underlay_count: 0,
            }
        );
    }
}
