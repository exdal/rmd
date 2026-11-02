use ash::vk;
use vir::{Access, AllocatorKind, BufferInfo, DomainFlag, MemoryLocation, Module, allocator::Allocator};

use super::{
    Renderer,
    cull::bind_sprite_cull,
    draw::MapViewPlan,
    lighting::LightingPush,
    record::{BlurPush, CameraPush},
    sprites::visible_range,
};
use crate::{CapturedImage, Frame, GpuError, extent3d};

const CAPTURE_TILE_SIZE: u32 = 2048;
pub(super) const CAPTURE_WORKGROUP_SIZE: u32 = 8;

impl Renderer {
    /// `origin` is the bottom-left corner of the captured region, in map pixels
    pub fn capture(
        &mut self, frame: &Frame<'_>, view: usize, origin: [u32; 2], size: [u32; 2],
    ) -> Result<CapturedImage, GpuError> {
        let [width, height] = size;
        let rgba_len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            .filter(|len| *len > 0)
            .ok_or(GpuError::CaptureTooLarge)?;
        let map_view = frame
            .map_views
            .get(view)
            .filter(|map_view| !map_view.rect.is_empty())
            .ok_or(GpuError::CaptureUnavailable)?;
        let uploaded = self
            .uploaded
            .get(view)
            .filter(|uploaded| uploaded.revision == map_view.revision)
            .ok_or(GpuError::CaptureUnavailable)?;
        let sprite_base = uploaded.base;
        let visible = visible_range(&uploaded.ranges, map_view.active_z, frame.underlay_depth);
        let cull_index = frame.map_views[..view]
            .iter()
            .filter(|map_view| !map_view.rect.is_empty())
            .count();
        let cull = self.cull.get(cull_index).ok_or(GpuError::CaptureUnavailable)?;
        let sprites = *self.sprites.as_ref().ok_or(GpuError::CaptureUnavailable)?;
        let lighting = match map_view.lighting.filter(|lighting| !lighting.tiles.is_empty()) {
            Some(lighting) => {
                let uploaded = self
                    .uploaded_lighting
                    .get(view)
                    .filter(|uploaded| uploaded.revision == Some(lighting.revision) && uploaded.size == lighting.size)
                    .ok_or(GpuError::CaptureUnavailable)?;
                let lights = *self.lights.as_ref().ok_or(GpuError::CaptureUnavailable)?;

                Some((lighting, uploaded.base, lights))
            },
            None => None,
        };

        let tile = CAPTURE_TILE_SIZE
            .min(self.device.max_image_dimension_2d)
            .min(width.max(height));
        let tile_extent = vk::Extent2D {
            width: tile,
            height: tile,
        };

        let mut module = Module::default();
        let sprites_slot = module.declare_buffer_var("capture sprites", Access::ComputeWrite);
        let lights_slot = lighting.map(|_| module.declare_buffer_var("capture light tiles", Access::FragmentRead));
        let scene = self.record_map_scene(
            &mut module,
            sprites_slot,
            lights_slot,
            tile_extent,
            frame.show_areas,
            frame.show_area_outlines,
            frame.underlay_depth > 0,
            "capture",
        );
        let pixels_slot = module.declare_buffer_var("capture pixels", Access::HostRead);
        let groups = tile.div_ceil(CAPTURE_WORKGROUP_SIZE);
        let [_, pixels] = module
            .begin_compute([
                (scene.scene, Access::ComputeSampled),
                (pixels_slot, Access::ComputeWrite),
            ])
            .with_name("capture readback")
            .bind_compute_pipeline(self.capture_pipeline)
            .bind_image(0, 0, scene.scene)
            .bind_buffer(0, 1, pixels_slot)
            .push_constants(&[tile, tile])
            .dispatch(groups, groups, 1u32)
            .end_compute();
        let pixels = module.export(pixels, Access::HostRead, DomainFlag::Host);
        let mut program = module.compile(&self.graph, pixels)?;

        program.set(scene.extent, extent3d(tile_extent));
        program.set(scene.area_outlines, frame.show_area_outlines);
        program.set(sprites_slot, sprites);
        program.set(scene.cull.visible, cull.visible);
        program.set(scene.cull.chunks, cull.chunks);
        program.set(scene.cull.commands, cull.commands);
        if let (Some(slot), Some((_, _, lights))) = (lights_slot, lighting) {
            program.set(slot, lights);
        }

        self.graph.wait()?;

        let mut readback = self.device.allocator.allocate_buffer(
            &BufferInfo::new(
                u64::from(tile) * u64::from(tile) * 4,
                vk::BufferUsageFlags::STORAGE_BUFFER,
                MemoryLocation::GpuToCpu,
            )
            .with_name("capture readback"),
        )?;
        program.set(pixels_slot, readback);

        let mut frames = self.device.context.create_super_frame_allocator(1);
        let mut rgba = vec![0; rgba_len];
        let result = (|| -> Result<(), GpuError> {
            for region in capture_tiles(origin, size, tile) {
                let camera = CameraPush {
                    center: region.center,
                    viewport: [tile as f32; 2],
                    zoom: 1.0,
                    base: sprite_base,
                    focused_area_owner: [0; 2],
                    placement_flash_owner: [0; 2],
                    placement_flash_strength: 0.0,
                };
                let plan = MapViewPlan {
                    camera,
                    sprite_base,
                    visible,
                };
                program.set_bytes(scene.camera, &camera);
                program.set_bytes(scene.underlay_camera, &CameraPush { base: 0, ..camera });
                program.set_bytes(
                    scene.active_camera,
                    &CameraPush {
                        base: visible.underlay_count,
                        ..camera
                    },
                );
                program.set_bytes(scene.blur, &BlurPush::for_camera(&camera));
                bind_sprite_cull(&mut program, &scene.cull, &plan, cull);
                if let (Some(slots), Some((lighting, base, _))) = (&scene.lighting, lighting) {
                    program.set_bytes(
                        slots.push,
                        &LightingPush {
                            center: camera.center,
                            viewport: camera.viewport,
                            zoom: camera.zoom,
                            tile_size: lighting.tile_size.max(1) as f32,
                            map_size: [lighting.size[0], lighting.size[1]],
                            level_count: lighting.size[2],
                            base,
                            minimum_brightness: lighting.minimum_brightness.clamp(0.0, 1.0),
                        },
                    );
                }

                let next = frames.get_next_frame()?;
                self.graph
                    .execute_blocking(&self.device.context, &program, &mut AllocatorKind::Frame(next))?;

                let pixels = readback.mapped_slice_mut().ok_or(vk::Result::ERROR_MEMORY_MAP_FAILED)?;
                let row_len = region.width as usize * 4;
                for row in 0..region.height as usize {
                    let source = row * tile as usize * 4;
                    let target = ((region.y as usize + row) * width as usize + region.x as usize) * 4;
                    rgba[target..target + row_len].copy_from_slice(&pixels[source..source + row_len]);
                }
            }

            Ok(())
        })();

        drop(frames);
        self.device.allocator.deallocate_buffer(readback);
        result?;

        Ok(CapturedImage { width, height, rgba })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct CaptureTile {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    center: [f32; 2],
}

fn capture_tiles(origin: [u32; 2], size: [u32; 2], tile: u32) -> Vec<CaptureTile> {
    let [width, height] = size;
    let [left, bottom] = origin.map(|value| value as f32);
    let half = tile as f32 / 2.0;
    let mut tiles = Vec::new();

    for y in (0..height).step_by(tile.max(1) as usize) {
        for x in (0..width).step_by(tile.max(1) as usize) {
            tiles.push(CaptureTile {
                x,
                y,
                width: tile.min(width - x),
                height: tile.min(height - y),
                // we go up
                center: [left + x as f32 + half, bottom + (height - y) as f32 - half],
            });
        }
    }

    tiles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_tiles_cover_the_map_from_the_top_row_down() {
        let tiles = capture_tiles([0, 0], [8160, 8160], 2048);

        assert_eq!(tiles.len(), 16);
        assert_eq!(
            tiles[0],
            CaptureTile {
                x: 0,
                y: 0,
                width: 2048,
                height: 2048,
                center: [1024.0, 7136.0],
            }
        );
        assert_eq!(
            tiles[15],
            CaptureTile {
                x: 6144,
                y: 6144,
                width: 2016,
                height: 2016,
                center: [7168.0, 992.0],
            }
        );
    }

    #[test]
    fn capture_tiles_offset_their_centers_by_the_region_origin() {
        assert_eq!(
            capture_tiles([64, 32], [96, 64], 64),
            [
                CaptureTile {
                    x: 0,
                    y: 0,
                    width: 64,
                    height: 64,
                    center: [96.0, 64.0],
                },
                CaptureTile {
                    x: 64,
                    y: 0,
                    width: 32,
                    height: 64,
                    center: [160.0, 64.0],
                },
            ]
        );
    }

    #[test]
    fn capture_tiles_crop_a_map_smaller_than_one_tile() {
        assert_eq!(
            capture_tiles([0, 0], [100, 70], 100),
            [CaptureTile {
                x: 0,
                y: 0,
                width: 100,
                height: 70,
                center: [50.0, 20.0],
            }]
        );
    }
}
