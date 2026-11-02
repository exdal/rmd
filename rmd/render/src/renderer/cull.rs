use ash::vk;
use vir::{
    Access, Buffer, BufferInfo, DRAW_INDIRECT_STRIDE, DrawIndirectCommand, MemoryLocation, Module, PipelineId, Program,
    ValueId, allocator::Allocator,
};

use super::{Renderer, draw::MapViewPlan, gpu::SPRITE_LISTS};
use crate::{Device, Frame, GpuError, spec};

pub(super) const CULL_WORKGROUP_SIZE: u32 = 64;
// an underlay and an active command per list
const SPRITE_DRAW_COMMANDS: u64 = 2 * SPRITE_LISTS as u64;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct CullPush {
    center: [f32; 2],
    viewport: [f32; 2],
    focused_area_owner: [u32; 2],
    zoom: f32,
    first_sprite: u32,
    sprite_count: u32,
    chunk_count: u32,
    active_first_relative: u32,
    list_capacity: u32,
}

#[repr(C)]
struct CullChunk {
    count: [u32; SPRITE_LISTS as usize],
    underlay_count: [u32; SPRITE_LISTS as usize],
    offset: [u32; SPRITE_LISTS as usize],
    underlay_offset: [u32; SPRITE_LISTS as usize],
}

pub(super) struct CullSlots {
    push: ValueId,
    sprite_count: ValueId,
    pub(super) visible: ValueId,
    pub(super) chunks: ValueId,
    pub(super) commands: ValueId,
}

pub(super) struct CullValues {
    pub(super) visible: ValueId,
    pub(super) chunks: ValueId,
    pub(super) commands: ValueId,
}

pub(super) fn sprite_draw_offset(list: u32, active: bool) -> u64 {
    u64::from(DRAW_INDIRECT_STRIDE) * u64::from(list * 2 + u32::from(active))
}

pub(super) fn record_sprite_cull(
    module: &mut Module, pipelines: [PipelineId; 3], sprites: ValueId, values: CullValues, show_areas: bool,
    show_area_outlines: bool, name: &str,
) -> (CullValues, CullSlots) {
    let CullValues {
        visible,
        chunks,
        commands,
    } = values;
    let push = module.declare_bytes_var(&format!("{name} push"), size_of::<CullPush>() as u32);
    let sprite_count = module.declare_u32_var(&format!("{name} sprite count"), 0);
    let slots = CullSlots {
        push,
        sprite_count,
        visible,
        chunks,
        commands,
    };

    let [_sprites, chunks] = module
        .begin_compute([(sprites, Access::ComputeRead), (chunks, Access::ComputeWrite)])
        .with_name(format!("{name} classify"))
        .bind_compute_pipeline(pipelines[0])
        .bind_buffer(0, 1, sprites)
        .bind_buffer(0, 4, chunks)
        .push_constants_from(push)
        .specialize_constant(spec::SHOW_AREAS, show_areas)
        .specialize_constant(spec::SHOW_AREA_OUTLINES, show_area_outlines)
        .dispatch_invocations(sprite_count, 1u32, 1u32)
        .end_compute();

    let [commands, chunks] = module
        .begin_compute([(commands, Access::ComputeWrite), (chunks, Access::ComputeRW)])
        .with_name(format!("{name} scan"))
        .bind_compute_pipeline(pipelines[1])
        .bind_buffer(0, 3, commands)
        .bind_buffer(0, 4, chunks)
        .push_constants_from(push)
        .dispatch(1u32, 1u32, 1u32)
        .end_compute();

    let [visible, _sprites, chunks] = module
        .begin_compute([
            (visible, Access::ComputeWrite),
            (sprites, Access::ComputeRead),
            (chunks, Access::ComputeRead),
        ])
        .with_name(format!("{name} compact"))
        .bind_compute_pipeline(pipelines[2])
        .bind_buffer(0, 1, sprites)
        .bind_buffer(0, 2, visible)
        .bind_buffer(0, 4, chunks)
        .push_constants_from(push)
        .specialize_constant(spec::SHOW_AREAS, show_areas)
        .specialize_constant(spec::SHOW_AREA_OUTLINES, show_area_outlines)
        .dispatch_invocations(sprite_count, 1u32, 1u32)
        .end_compute();

    (
        CullValues {
            visible,
            chunks,
            commands,
        },
        slots,
    )
}

pub(super) struct CullBuffers {
    pub(super) visible: Buffer,
    pub(super) chunks: Buffer,
    pub(super) commands: Buffer,
    pub(super) sprite_capacity: usize,
}

impl CullBuffers {
    pub(super) fn allocate(device: &mut Device, sprites: usize) -> Result<Self, GpuError> {
        let capacity = sprites
            .max(1024)
            .checked_next_power_of_two()
            .ok_or(GpuError::SpriteUploadTooLarge)?;
        // every list gets a region of its own, as long as the whole range
        let indices = capacity
            .checked_mul(SPRITE_LISTS as usize * size_of::<u32>())
            .and_then(|size| u64::try_from(size).ok())
            .ok_or(GpuError::SpriteUploadTooLarge)?;
        let chunk_count = u32::try_from(capacity)
            .map_err(|_| GpuError::SpriteUploadTooLarge)?
            .div_ceil(CULL_WORKGROUP_SIZE)
            .max(1);

        let visible = device.allocator.allocate_buffer(
            &BufferInfo::new(indices, vk::BufferUsageFlags::STORAGE_BUFFER, MemoryLocation::GpuOnly)
                .with_name("visible sprite indices"),
        )?;
        let chunks = match device.allocator.allocate_buffer(
            &BufferInfo::new(
                u64::from(chunk_count) * size_of::<CullChunk>() as u64,
                vk::BufferUsageFlags::STORAGE_BUFFER,
                MemoryLocation::GpuOnly,
            )
            .with_name("cull chunks"),
        ) {
            Ok(buffer) => buffer,
            Err(error) => {
                device.allocator.deallocate_buffer(visible);

                return Err(error.into());
            },
        };
        let commands = match device.allocator.allocate_buffer(
            &BufferInfo::new(
                SPRITE_DRAW_COMMANDS * size_of::<DrawIndirectCommand>() as u64,
                vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::INDIRECT_BUFFER,
                MemoryLocation::GpuOnly,
            )
            .with_name("sprite draw commands"),
        ) {
            Ok(buffer) => buffer,
            Err(error) => {
                device.allocator.deallocate_buffer(chunks);
                device.allocator.deallocate_buffer(visible);

                return Err(error.into());
            },
        };

        Ok(Self {
            visible,
            chunks,
            commands,
            sprite_capacity: capacity,
        })
    }

    pub(super) fn destroy(self, device: &mut Device) {
        device.allocator.deallocate_buffer(self.visible);
        device.allocator.deallocate_buffer(self.chunks);
        device.allocator.deallocate_buffer(self.commands);
    }
}

pub(super) fn bind_sprite_cull(program: &mut Program, slots: &CullSlots, plan: &MapViewPlan, buffers: &CullBuffers) {
    let sprite_count = plan.visible.count;
    program.set_bytes(
        slots.push,
        &CullPush {
            center: plan.camera.center,
            viewport: plan.camera.viewport,
            focused_area_owner: plan.camera.focused_area_owner,
            zoom: plan.camera.zoom,
            first_sprite: plan.sprite_base + plan.visible.base,
            sprite_count,
            chunk_count: sprite_count.div_ceil(CULL_WORKGROUP_SIZE),
            active_first_relative: plan.visible.underlay_count,
            list_capacity: buffers.sprite_capacity as u32,
        },
    );
    program.set(slots.sprite_count, sprite_count);
}

impl Renderer {
    pub(super) fn prepare_cull_buffers(&mut self, frame: &Frame<'_>) -> Result<(), GpuError> {
        let wanted = frame
            .map_views
            .iter()
            .filter(|view| !view.rect.is_empty())
            .map(|view| view.sprite_instances.len())
            .collect::<Vec<_>>();
        if wanted.len() == self.cull.len()
            && wanted
                .iter()
                .zip(&self.cull)
                .all(|(sprites, buffers)| buffers.sprite_capacity >= *sprites)
        {
            return Ok(());
        }

        self.device.wait_idle()?;

        while self.cull.len() > wanted.len() {
            if let Some(buffers) = self.cull.pop() {
                buffers.destroy(&mut self.device);
            }
        }

        for (index, sprites) in wanted.into_iter().enumerate() {
            if self
                .cull
                .get(index)
                .is_some_and(|buffers| buffers.sprite_capacity >= sprites)
            {
                continue;
            }

            let buffers = CullBuffers::allocate(&mut self.device, sprites)?;
            match self.cull.get_mut(index) {
                Some(slot) => {
                    let previous = std::mem::replace(slot, buffers);
                    previous.destroy(&mut self.device);
                },
                None => self.cull.push(buffers),
            }
        }

        Ok(())
    }
}
