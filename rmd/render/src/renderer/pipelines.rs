use vir::{ComputePipelineInfo, GraphicsPipelineInfo, PipelineId, RenderGraph};

use super::{
    bindless::BindlessDescriptorSet,
    shaders::{
        AREA_COLOR_CS_SPV, BLUR_FS_SPV, BLUR_VS_SPV, CAPTURE_CS_SPV, GEOMETRY_VS_SPV, GUIDE_FS_SPV, GUIDE_VS_SPV,
        INTERACTION_FS_SPV, INTERACTION_VS_SPV, LIGHTING_FS_SPV, LIGHTING_VS_SPV, PICK_CS_SPV, SCENE_RESOLVE_FS_SPV,
        SPRITE_CULL_CLASSIFY_CS_SPV, SPRITE_CULL_COMPACT_CS_SPV, SPRITE_CULL_SCAN_CS_SPV, SPRITE_OVERLAY_LIGHT_FS_SPV,
        SPRITE_SCENE_FS_SPV, SPRITE_SHADE_FS_SPV, SPRITE_VIS_FS_SPV,
    },
};
use crate::{GpuError, read_spirv};

pub(super) struct Pipelines {
    pub(super) sprite: PipelineId,
    pub(super) scene: PipelineId,
    pub(super) visibility: PipelineId,
    pub(super) overlay_light: PipelineId,
    pub(super) cull: [PipelineId; 3],
    pub(super) interaction: PipelineId,
    pub(super) guide: PipelineId,
    pub(super) lighting: PipelineId,
    pub(super) resolve: PipelineId,
    pub(super) blur: PipelineId,
    pub(super) pick: PipelineId,
    pub(super) capture: PipelineId,
    pub(super) area_color: PipelineId,
}

impl Pipelines {
    pub(super) fn declare(graph: &mut RenderGraph, bindless: &BindlessDescriptorSet) -> Result<Self, GpuError> {
        let geometry_vs = read_spirv(GEOMETRY_VS_SPV)?;
        let sprite = graph.declare_pipeline(
            GraphicsPipelineInfo::new()
                .with_shader(&geometry_vs)
                .with_shader(&read_spirv(SPRITE_SHADE_FS_SPV)?)
                .with_bindless_set(1, bindless.layout, bindless.set),
        )?;
        let scene = graph.declare_pipeline(
            GraphicsPipelineInfo::new()
                .with_shader(&geometry_vs)
                .with_shader(&read_spirv(SPRITE_SCENE_FS_SPV)?)
                .with_bindless_set(1, bindless.layout, bindless.set),
        )?;
        let visibility = graph.declare_pipeline(
            GraphicsPipelineInfo::new()
                .with_shader(&geometry_vs)
                .with_shader(&read_spirv(SPRITE_VIS_FS_SPV)?)
                .with_bindless_set(1, bindless.layout, bindless.set),
        )?;
        let overlay_light = graph.declare_pipeline(
            GraphicsPipelineInfo::new()
                .with_shader(&geometry_vs)
                .with_shader(&read_spirv(SPRITE_OVERLAY_LIGHT_FS_SPV)?)
                .with_bindless_set(1, bindless.layout, bindless.set),
        )?;
        let cull = [
            graph.declare_compute_pipeline(ComputePipelineInfo::new(&read_spirv(SPRITE_CULL_CLASSIFY_CS_SPV)?))?,
            graph.declare_compute_pipeline(ComputePipelineInfo::new(&read_spirv(SPRITE_CULL_SCAN_CS_SPV)?))?,
            graph.declare_compute_pipeline(ComputePipelineInfo::new(&read_spirv(SPRITE_CULL_COMPACT_CS_SPV)?))?,
        ];
        let interaction = graph.declare_pipeline(
            GraphicsPipelineInfo::new()
                .with_shader(&read_spirv(INTERACTION_VS_SPV)?)
                .with_shader(&read_spirv(INTERACTION_FS_SPV)?),
        )?;
        let guide = graph.declare_pipeline(
            GraphicsPipelineInfo::new()
                .with_shader(&read_spirv(GUIDE_VS_SPV)?)
                .with_shader(&read_spirv(GUIDE_FS_SPV)?),
        )?;
        let lighting = graph.declare_pipeline(
            GraphicsPipelineInfo::new()
                .with_shader(&read_spirv(LIGHTING_VS_SPV)?)
                .with_shader(&read_spirv(LIGHTING_FS_SPV)?),
        )?;
        let resolve = graph.declare_pipeline(
            GraphicsPipelineInfo::new()
                .with_shader(&read_spirv(LIGHTING_VS_SPV)?)
                .with_shader(&read_spirv(SCENE_RESOLVE_FS_SPV)?),
        )?;
        let blur = graph.declare_pipeline(
            GraphicsPipelineInfo::new()
                .with_shader(&read_spirv(BLUR_VS_SPV)?)
                .with_shader(&read_spirv(BLUR_FS_SPV)?),
        )?;
        let pick = graph.declare_compute_pipeline(ComputePipelineInfo::new(&read_spirv(PICK_CS_SPV)?))?;
        let capture = graph.declare_compute_pipeline(ComputePipelineInfo::new(&read_spirv(CAPTURE_CS_SPV)?))?;
        let area_color = graph.declare_compute_pipeline(
            ComputePipelineInfo::new(&read_spirv(AREA_COLOR_CS_SPV)?).with_bindless_set(
                1,
                bindless.layout,
                bindless.set,
            ),
        )?;

        Ok(Self {
            sprite,
            scene,
            visibility,
            overlay_light,
            cull,
            interaction,
            guide,
            lighting,
            resolve,
            blur,
            pick,
            capture,
            area_color,
        })
    }
}
