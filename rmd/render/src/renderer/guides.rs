use vir::{Buffer, FrameAllocator, ValueId};

use super::{
    Renderer,
    gpu::{owner_words, payload_bytes},
    upload::upload_frame_buffer,
};
use crate::{Frame, GpuError};

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct GpuGuideLine {
    origin: [f32; 2],
    target: [f32; 2],
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(super) struct GuidePush {
    pub(super) viewport: [f32; 2],
    pub(super) picked_owner: [u32; 2],
    pub(super) stripe_offset: f32,
}

pub(super) struct GuideSlots {
    pub(super) has_lines: ValueId,
    pub(super) lines: ValueId,
    pub(super) push: ValueId,
    pub(super) draw: ValueId,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PreparedGuides {
    pub(super) lines: Buffer,
    pub(super) connected_owners: Buffer,
    pub(super) line_count: u32,
    pub(super) connected_count: u32,
}

impl Renderer {
    pub(super) fn prepare_guides(
        allocator: &mut FrameAllocator, frame: &Frame<'_>,
    ) -> Result<Vec<PreparedGuides>, GpuError> {
        let finite = |origin: &[f32; 2], target: &[f32; 2]| origin.iter().chain(target).all(|value| value.is_finite());
        frame
            .map_views
            .iter()
            .filter(|view| !view.rect.is_empty())
            .map(|view| {
                let lines = view
                    .guide_lines
                    .iter()
                    .copied()
                    .filter(|guide| finite(&guide.origin, &guide.target))
                    .map(|guide| {
                        let (origin, target) = project_guide_line(guide, view.camera, view.rect);
                        GpuGuideLine { origin, target }
                    })
                    .filter(|guide| finite(&guide.origin, &guide.target))
                    .take(u32::MAX as usize)
                    .collect::<Vec<_>>();
                let owners = view
                    .connected
                    .iter()
                    .copied()
                    .map(owner_words)
                    .take(u32::MAX as usize)
                    .collect::<Vec<_>>();
                let lines_buffer = upload_frame_buffer(allocator, payload_bytes(&lines), "selection guide lines")?;
                let connected_owners = upload_frame_buffer(allocator, payload_bytes(&owners), "connected owners")?;

                Ok(PreparedGuides {
                    lines: lines_buffer,
                    connected_owners,
                    line_count: lines.len() as u32,
                    connected_count: owners.len() as u32,
                })
            })
            .collect()
    }
}

fn project_guide_line(
    guide: crate::GuideLine, camera: crate::Camera, rect: crate::MapViewRect,
) -> ([f32; 2], [f32; 2]) {
    let logical = [
        camera.viewport_width.max(1) as f32,
        camera.viewport_height.max(1) as f32,
    ];
    let scale = [rect.width as f32 / logical[0], rect.height as f32 / logical[1]];
    let project = |point: [f32; 2]| {
        [
            ((point[0] - camera.x) * camera.zoom + logical[0] * 0.5) * scale[0],
            ((camera.y - point[1]) * camera.zoom + logical[1] * 0.5) * scale[1],
        ]
    };

    (project(guide.origin), project(guide.target))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Camera, GuideLine};

    /// The shader computes `clip = (map - center) * zoom / (viewport * 0.5)`.
    /// `zoom` is logical pixels per map unit, matching the camera maths the UI
    /// uses for overlays, so the projection must divide by the camera's logical
    /// size — not by the physical rectangle it lands in.
    fn projected_tile_size(zoom: f32, pushed_viewport: f32, rect_width: f32) -> f32 {
        // One map unit, in NDC, scaled onto the physical rectangle.
        let ndc = zoom / (pushed_viewport * 0.5);

        ndc * rect_width * 0.5
    }

    #[test]
    fn a_map_unit_covers_the_same_screen_space_at_any_display_scale() {
        let zoom = 4.0;
        let logical = 800.0;

        // At 1x the logical and physical rectangles agree.
        assert_eq!(projected_tile_size(zoom, logical, 800.0), 4.0);

        // At 2x the rectangle doubles. Projecting against the logical size
        // keeps the map the same on-screen size, just at twice the resolution.
        assert_eq!(projected_tile_size(zoom, logical, 1600.0), 8.0);

        // Projecting against the physical size instead halves the map relative
        // to every ImGui overlay, which is the 2x mismatch this guards.
        assert_eq!(projected_tile_size(zoom, 1600.0, 1600.0), 4.0);
    }

    #[test]
    fn guide_lines_project_into_map_view_coordinates() {
        let guide = GuideLine {
            origin: [110.0, 40.0],
            target: [130.0, 60.0],
        };
        let camera = Camera {
            x: 100.0,
            y: 50.0,
            zoom: 2.0,
            viewport_width: 800,
            viewport_height: 600,
        };

        let full = crate::MapViewRect {
            x: 0,
            y: 0,
            width: 800,
            height: 600,
        };
        assert_eq!(
            project_guide_line(guide, camera, full),
            ([420.0, 320.0], [460.0, 280.0]),
        );

        // Moving the ImGui window does not move coordinates inside its private
        // render attachment.
        let right = crate::MapViewRect {
            x: 800,
            y: 100,
            width: 800,
            height: 600,
        };
        assert_eq!(
            project_guide_line(guide, camera, right),
            ([420.0, 320.0], [460.0, 280.0]),
        );

        let scaled = crate::MapViewRect {
            x: 0,
            y: 0,
            width: 1600,
            height: 1200,
        };
        assert_eq!(
            project_guide_line(guide, camera, scaled),
            ([840.0, 640.0], [920.0, 560.0]),
        );

        let panned = Camera {
            x: 110.0,
            y: 60.0,
            ..camera
        };
        assert_eq!(
            project_guide_line(guide, panned, scaled),
            ([800.0, 680.0], [880.0, 600.0]),
        );
    }
}
