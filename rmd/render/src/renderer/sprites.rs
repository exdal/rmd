use ash::vk;
use vir::allocator::Allocator;

use super::{
    Renderer,
    gpu::{GpuSprite, gpu_sprite_offset, gpu_sprite_range, payload_bytes},
    sheets::{merge_sheets, note_sheets, sheet_indices},
    upload::{device_buffer_info, upload_buffer},
};
use crate::{Frame, GpuError, MapViewFrame, SpriteInstance, UpdateRange};

const MIN_VIEW_CAPACITY: usize = 1024;

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
    capacity: usize,
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

        if self.sprites.is_some() && self.uploaded.len() == frame.map_views.len() && self.prepare_frame_update(frame)? {
            return Ok(());
        }

        let regions = view_regions(frame.map_views.iter().map(|view| view.sprite_instances.len()))?;
        let total = regions.last().map_or(0, |&(base, capacity)| base as usize + capacity);

        let mut payloads = Vec::with_capacity(frame.map_views.len());
        let mut uploaded = Vec::with_capacity(frame.map_views.len());
        for (view, &(base, capacity)) in frame.map_views.iter().zip(&regions) {
            let count = view.sprite_instances.len();
            payloads.push(gpu_sprite_range(
                view.sprite_instances,
                UpdateRange { start: 0, end: count },
            )?);

            note_sheets(&mut self.requested_sheets, view.sprite_instances);

            uploaded.push(UploadedMapView {
                revision: view.revision,
                base,
                count,
                capacity,
                ranges: level_ranges(view.sprite_instances),
                sheets: sheet_indices(view.sprite_instances),
            });
        }

        let writes = uploaded
            .iter()
            .zip(&payloads)
            .map(|(view, payload)| Ok((gpu_sprite_offset(view.base as usize)?, payload_bytes(payload))))
            .collect::<Result<Vec<_>, GpuError>>()?;

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
            if let Err(error) = upload_buffer(&mut self.device, &mut self.graph, buffer, &writes) {
                self.device.allocator.deallocate_buffer(buffer);

                return Err(error);
            }

            if let Some(old) = self.sprites.replace(buffer) {
                self.device.allocator.deallocate_buffer(old);
            }

            self.sprite_capacity = capacity;
        } else if let Some(buffer) = self.sprites {
            upload_buffer(&mut self.device, &mut self.graph, buffer, &writes)?;
        }

        // area colors sample the sprites' sheets, so those have to be loaded first
        self.load_requested_sheets()?;

        let sprites = *self.sprites.as_ref().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
        for view in &uploaded {
            let start = view.base as usize;
            self.color_area_outlines(
                sprites,
                UpdateRange {
                    start,
                    end: start + view.count,
                },
            )?;
        }

        self.uploaded = uploaded;

        Ok(())
    }

    fn prepare_frame_update(&mut self, frame: &Frame<'_>) -> Result<bool, GpuError> {
        let Some(sprites) = self.sprites else {
            return Ok(false);
        };

        let mut plans = Vec::new();
        for (index, (uploaded, view)) in self.uploaded.iter().zip(frame.map_views).enumerate() {
            if uploaded.revision == view.revision {
                continue;
            }

            let Some(ranges) = plan_sprite_update(uploaded, view) else {
                return Ok(false);
            };
            plans.push((index, ranges));
        }

        let mut payloads = Vec::new();
        let mut packed_views = Vec::with_capacity(plans.len());
        for (index, ranges) in &plans {
            let view = &frame.map_views[*index];
            let base = self.uploaded[*index].base as usize;

            let mut packed = Vec::new();
            for range in ranges {
                let local = UpdateRange {
                    start: range.start - base,
                    end: range.end - base,
                };
                payloads.push((*range, gpu_sprite_range(view.sprite_instances, local)?));
                note_sheets(&mut packed, &view.sprite_instances[local.start..local.end]);
            }
            packed_views.push(packed);
        }

        let writes = payloads
            .iter()
            .map(|(range, payload)| Ok((gpu_sprite_offset(range.start)?, payload_bytes(payload))))
            .collect::<Result<Vec<_>, GpuError>>()?;

        if payloads.iter().any(|(_, payload)| !payload.is_empty()) {
            self.graph.wait()?;
        }

        upload_buffer(&mut self.device, &mut self.graph, sprites, &writes)?;
        for packed in &packed_views {
            self.requested_sheets.extend_from_slice(packed);
        }
        self.load_requested_sheets()?;

        for (range, _) in &payloads {
            self.color_area_outlines(sprites, *range)?;
        }

        for ((index, _), packed) in plans.iter().zip(&packed_views) {
            let view = &frame.map_views[*index];
            let uploaded = &mut self.uploaded[*index];
            if uploaded.count != view.sprite_instances.len() {
                uploaded.ranges = level_ranges(view.sprite_instances);
            }
            uploaded.revision = view.revision;
            uploaded.count = view.sprite_instances.len();
            merge_sheets(&mut uploaded.sheets, packed);
        }

        Ok(true)
    }
}

fn view_regions(counts: impl Iterator<Item = usize>) -> Result<Vec<(u32, usize)>, GpuError> {
    let mut regions = Vec::new();
    let mut next = 0usize;
    for count in counts {
        let capacity = count
            .checked_add(count / 4)
            .ok_or(GpuError::SpriteUploadTooLarge)?
            .max(MIN_VIEW_CAPACITY);
        let base = u32::try_from(next).map_err(|_| GpuError::SpriteUploadTooLarge)?;
        next = next.checked_add(capacity).ok_or(GpuError::SpriteUploadTooLarge)?;
        u32::try_from(next).map_err(|_| GpuError::SpriteUploadTooLarge)?;

        regions.push((base, capacity));
    }

    Ok(regions)
}

fn plan_sprite_update(uploaded: &UploadedMapView, view: &MapViewFrame<'_>) -> Option<Vec<UpdateRange>> {
    let updates = updates_since(view.pending_updates, uploaded.revision, |update| {
        update.previous_revision
    })?;
    let len = view.sprite_instances.len();
    let ranges = merged_ranges(updates.iter().flat_map(|update| update.sprites.iter().copied()));
    let is_valid = len <= uploaded.capacity
        && ranges.iter().all(|range| valid_update_range(Some(*range), len))
        && (uploaded.count == len || !ranges.is_empty());
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

pub(super) fn updates_since<T>(updates: &[T], uploaded: u64, previous_revision: impl Fn(&T) -> u64) -> Option<&[T]> {
    let start = updates
        .iter()
        .position(|update| previous_revision(update) == uploaded)?;

    Some(&updates[start..])
}

pub(super) fn merged_ranges(ranges: impl Iterator<Item = UpdateRange>) -> Vec<UpdateRange> {
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

pub(super) fn valid_update_range(range: Option<UpdateRange>, len: usize) -> bool {
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
    use crate::{
        FrameUpdate, MapViewRect,
        renderer::fixtures::{map_view_with_sprites, sprite},
    };

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

    fn update(previous_revision: u64, start: usize, end: usize) -> FrameUpdate {
        FrameUpdate {
            previous_revision,
            sprites: vec![UpdateRange { start, end }],
        }
    }

    fn no_ranges(previous_revision: u64) -> FrameUpdate {
        FrameUpdate {
            previous_revision,
            sprites: Vec::new(),
        }
    }

    fn uploaded_view(base: u32, count: usize, capacity: usize) -> UploadedMapView {
        UploadedMapView {
            revision: 5,
            base,
            count,
            capacity,
            ..Default::default()
        }
    }

    fn changed_view<'a>(sprites: &'a [SpriteInstance], updates: &'a [FrameUpdate]) -> MapViewFrame<'a> {
        MapViewFrame {
            revision: 6,
            pending_updates: updates,
            ..map_view_with_sprites(MapViewRect::default(), sprites)
        }
    }

    #[test]
    fn view_regions_are_ascending_and_leave_slack() {
        let regions = view_regions([0, 10, 4000, 1024].into_iter()).unwrap();

        assert_eq!(regions, [(0, 1024), (1024, 1024), (2048, 5000), (7048, 1280)]);
        for (pair, count) in regions.windows(2).zip([0, 10, 4000]) {
            assert_eq!(pair[0].0 as usize + pair[0].1, pair[1].0 as usize);
            assert!(pair[0].1 >= count);
        }
    }

    #[test]
    fn view_regions_reject_u32_overflow() {
        assert!(view_regions([u32::MAX as usize].into_iter()).is_err());
    }

    #[test]
    fn a_later_view_updates_at_its_own_base() {
        let sprites = [sprite(1), sprite(1), sprite(1)];
        let updates = [update(5, 1, 3)];

        assert_eq!(
            plan_sprite_update(&uploaded_view(1024, 3, 1024), &changed_view(&sprites, &updates)),
            Some(vec![UpdateRange { start: 1025, end: 1027 }])
        );
    }

    #[test]
    fn growth_inside_the_region_is_accepted_and_past_it_is_not() {
        let sprites = vec![sprite(1); 5];
        let updates = [update(5, 3, 5)];

        assert!(plan_sprite_update(&uploaded_view(0, 3, 5), &changed_view(&sprites, &updates)).is_some());
        assert_eq!(
            plan_sprite_update(&uploaded_view(0, 3, 4), &changed_view(&sprites, &updates)),
            None
        );
    }

    #[test]
    fn sprite_updates_fall_back_when_they_cannot_be_applied() {
        let sprites = [sprite(1), sprite(1)];
        let uploaded = uploaded_view(0, 2, 1024);

        assert_eq!(
            plan_sprite_update(&uploaded, &changed_view(&sprites, &[update(4, 0, 1)])),
            None
        );
        assert_eq!(
            plan_sprite_update(&uploaded_view(0, 1, 1024), &changed_view(&sprites, &[no_ranges(5)])),
            None
        );
        assert_eq!(
            plan_sprite_update(&uploaded_view(0, 1, 1024), &changed_view(&sprites, &[])),
            None
        );
        assert_eq!(
            plan_sprite_update(&uploaded, &changed_view(&sprites, &[update(5, 1, 3)])),
            None
        );
    }
}
