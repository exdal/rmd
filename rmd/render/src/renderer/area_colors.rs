use vir::{Access, AllocatorKind, Buffer, Module, PipelineId, Program, RenderGraph, ValueId};

use super::Renderer;
use crate::GpuError;

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct AreaColorPush {
    first_sprite: u32,
    sprite_count: u32,
    group_count: u32,
}

pub(super) struct AreaColorPass {
    program: Program,
    sprites: ValueId,
    push: ValueId,
    groups: ValueId,
}

impl AreaColorPass {
    pub(super) fn record(graph: &RenderGraph, pipeline: PipelineId) -> Result<Self, GpuError> {
        let mut module = Module::default();
        let sprites = module.declare_buffer_var("area color sprites", Access::HostWrite);
        let push = module.declare_bytes_var("area color push", size_of::<AreaColorPush>() as u32);
        let groups = module.declare_u32_var("area color groups", 0);
        let [colored] = module
            .begin_compute([(sprites, Access::ComputeRW)])
            .with_name("area colors")
            .bind_compute_pipeline(pipeline)
            .bind_buffer(0, 1, sprites)
            .push_constants_from(push)
            .dispatch(groups, 1, 1)
            .end_compute();
        let program = module.compile(graph, colored)?;

        Ok(Self {
            program,
            sprites,
            push,
            groups,
        })
    }
}

impl Renderer {
    pub(super) fn color_area_outlines(&mut self, buffer: Buffer, range: crate::UpdateRange) -> Result<(), GpuError> {
        let first_sprite = u32::try_from(range.start).map_err(|_| GpuError::SpriteUploadTooLarge)?;
        let sprite_count =
            u32::try_from(range.end.saturating_sub(range.start)).map_err(|_| GpuError::SpriteUploadTooLarge)?;
        if sprite_count == 0 {
            return Ok(());
        }

        let group_count = area_color_group_count(sprite_count, self.device.max_compute_work_group_count_x);
        self.area_colors.program.set(self.area_colors.sprites, buffer);
        self.area_colors.program.set_bytes(
            self.area_colors.push,
            &AreaColorPush {
                first_sprite,
                sprite_count,
                group_count,
            },
        );
        self.area_colors.program.set(self.area_colors.groups, group_count);
        self.graph.execute_blocking(
            &self.device.context,
            &self.area_colors.program,
            &mut AllocatorKind::Persistent(&mut self.device.allocator),
        )?;

        Ok(())
    }
}

fn area_color_group_count(sprite_count: u32, limit: u32) -> u32 {
    if sprite_count == 0 {
        0
    } else {
        sprite_count.min(limit.max(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn area_color_dispatch_stays_within_the_device_limit() {
        assert_eq!(area_color_group_count(0, 64), 0);
        assert_eq!(area_color_group_count(32, 64), 32);
        assert_eq!(area_color_group_count(65, 64), 64);
        assert_eq!(area_color_group_count(1, 0), 1);
    }
}
