pub(super) const GEOMETRY_VS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sprite.vert.spv"));
pub(super) const SPRITE_SHADE_FS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sprite.frag.spv"));
pub(super) const SPRITE_SCENE_FS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sprite_scene.frag.spv"));
pub(super) const SPRITE_VIS_FS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sprite_visibility.frag.spv"));
pub(super) const SPRITE_OVERLAY_LIGHT_FS_SPV: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/sprite_overlay_light.frag.spv"));
pub(super) const SPRITE_CULL_CLASSIFY_CS_SPV: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/sprite_cull_classify.comp.spv"));
pub(super) const SPRITE_CULL_SCAN_CS_SPV: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/sprite_cull_scan.comp.spv"));
pub(super) const SPRITE_CULL_COMPACT_CS_SPV: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/sprite_cull_compact.comp.spv"));
pub(super) const AREA_COLOR_CS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/area_color.comp.spv"));
pub(super) const INTERACTION_VS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/interaction.vert.spv"));
pub(super) const INTERACTION_FS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/interaction.frag.spv"));
pub(super) const GUIDE_VS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/guide.vert.spv"));
pub(super) const GUIDE_FS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/guide.frag.spv"));
pub(super) const LIGHTING_VS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/lighting.vert.spv"));
pub(super) const LIGHTING_FS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/lighting.frag.spv"));
pub(super) const SCENE_RESOLVE_FS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/scene_resolve.frag.spv"));
pub(super) const BLUR_VS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/blur.vert.spv"));
pub(super) const BLUR_FS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/blur.frag.spv"));
pub(super) const PICK_CS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/pick.comp.spv"));
pub(super) const CAPTURE_CS_SPV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/capture.comp.spv"));

#[cfg(test)]
mod tests {
    use vir::resource::shader;

    use super::*;
    use crate::{
        read_spirv,
        renderer::{
            area_colors::AreaColorPush,
            capture::CAPTURE_WORKGROUP_SIZE,
            cull::{CULL_WORKGROUP_SIZE, CullPush},
            gpu::GpuSprite,
            guides::{GpuGuideLine, GuidePush},
            lighting::LightingPush,
            record::{BlurPush, CameraPush, InteractionPush, PickPush},
        },
        spec,
    };

    #[test]
    fn sprite_and_camera_layouts_match_the_shader_scalar_layout() {
        let reflection = shader::reflect(&read_spirv(GEOMETRY_VS_SPV).expect("valid SPIR-V")).expect("shader reflects");

        assert_eq!(size_of::<GpuSprite>(), 60);
        assert_eq!(std::mem::offset_of!(GpuSprite, node_pickable), 56);
        assert_eq!(reflection.push_constant_offset, 0);
        assert_eq!(reflection.push_constant_size as usize, size_of::<CameraPush>());
    }

    #[test]
    fn lighting_shader_layout_matches_the_renderer() {
        let reflection = shader::reflect(&read_spirv(LIGHTING_FS_SPV).expect("valid SPIR-V")).expect("shader reflects");
        let mut bindings = reflection
            .bindings
            .iter()
            .map(|binding| (binding.set, binding.binding))
            .collect::<Vec<_>>();
        bindings.sort_unstable();

        assert_eq!(bindings, [(0, 0), (0, 1), (0, 2), (0, 3), (0, 4)]);
        assert_eq!(reflection.push_constant_offset, 0);
        assert_eq!(reflection.push_constant_size as usize, size_of::<LightingPush>());
    }

    #[test]
    fn scene_resolve_shader_reads_only_the_scene() {
        let reflection =
            shader::reflect(&read_spirv(SCENE_RESOLVE_FS_SPV).expect("valid SPIR-V")).expect("shader reflects");
        let bindings = reflection
            .bindings
            .iter()
            .map(|binding| (binding.set, binding.binding))
            .collect::<Vec<_>>();

        assert_eq!(bindings, [(0, 0)]);
        assert_eq!(reflection.push_constant_size, 0);
    }

    #[test]
    fn blur_shader_layout_matches_the_renderer() {
        let reflection = shader::reflect(&read_spirv(BLUR_FS_SPV).expect("valid SPIR-V")).expect("shader reflects");
        let bindings = reflection
            .bindings
            .iter()
            .map(|binding| (binding.set, binding.binding))
            .collect::<Vec<_>>();

        assert_eq!(bindings, [(0, 0)]);
        assert_eq!(reflection.push_constant_offset, 0);
        assert_eq!(reflection.push_constant_size as usize, size_of::<BlurPush>());
    }

    #[test]
    fn sprite_vertex_shader_reads_sprites_and_the_compacted_indices() {
        let reflection = shader::reflect(&read_spirv(GEOMETRY_VS_SPV).expect("valid SPIR-V")).expect("shader reflects");
        let mut bindings = reflection
            .bindings
            .iter()
            .map(|binding| (binding.set, binding.binding, binding.access))
            .collect::<Vec<_>>();
        bindings.sort_unstable_by_key(|binding| (binding.0, binding.1));

        assert_eq!(reflection.push_constant_offset, 0);
        assert_eq!(reflection.push_constant_size as usize, size_of::<CameraPush>());
        assert_eq!(bindings.len(), 2);
        assert_eq!((bindings[0].0, bindings[0].1), (0, 1));
        assert_eq!((bindings[1].0, bindings[1].1), (0, 2));
        assert!(bindings[0].2.contains(vir::Access::VertexRead));
        assert!(bindings[1].2.contains(vir::Access::VertexRead));
    }

    #[test]
    fn sprite_cull_compute_layouts_match_the_host() {
        let stages = [
            (
                SPRITE_CULL_CLASSIFY_CS_SPV,
                [CULL_WORKGROUP_SIZE, 1, 1],
                vec![(1, vir::Access::ComputeRead), (4, vir::Access::ComputeWrite)],
            ),
            (
                SPRITE_CULL_SCAN_CS_SPV,
                [256, 1, 1],
                vec![(3, vir::Access::ComputeWrite), (4, vir::Access::ComputeRW)],
            ),
            (
                SPRITE_CULL_COMPACT_CS_SPV,
                [CULL_WORKGROUP_SIZE, 1, 1],
                vec![
                    (1, vir::Access::ComputeRead),
                    (2, vir::Access::ComputeWrite),
                    (4, vir::Access::ComputeRead),
                ],
            ),
        ];

        for (spirv, local_size, expected_bindings) in stages {
            let reflection = shader::reflect(&read_spirv(spirv).expect("valid SPIR-V")).expect("shader reflects");
            let mut bindings = reflection
                .bindings
                .iter()
                .map(|binding| (binding.binding, binding.access))
                .collect::<Vec<_>>();
            bindings.sort_unstable_by_key(|binding| binding.0);

            assert_eq!(reflection.local_size, local_size);
            assert_eq!(reflection.push_constant_offset, 0);
            assert_eq!(reflection.push_constant_size as usize, size_of::<CullPush>());
            assert_eq!(bindings, expected_bindings);
        }
    }

    #[test]
    fn interaction_fragment_layout_matches_the_host() {
        let reflection =
            shader::reflect(&read_spirv(INTERACTION_FS_SPV).expect("valid SPIR-V")).expect("shader reflects");
        let mut bindings = reflection
            .bindings
            .iter()
            .map(|binding| (binding.set, binding.binding))
            .collect::<Vec<_>>();
        bindings.sort_unstable();

        assert_eq!(bindings, [(0, 1), (0, 2), (0, 4), (0, 5)]);
        assert_eq!(reflection.push_constant_offset, 0);
        assert_eq!(reflection.push_constant_size as usize, size_of::<InteractionPush>());
    }

    #[test]
    fn guide_fragment_layout_matches_the_host() {
        let reflection = shader::reflect(&read_spirv(GUIDE_FS_SPV).expect("valid SPIR-V")).expect("shader reflects");
        let mut bindings = reflection
            .bindings
            .iter()
            .map(|binding| (binding.set, binding.binding))
            .collect::<Vec<_>>();
        bindings.sort_unstable();

        assert_eq!(bindings, [(0, 0), (0, 1), (0, 2), (0, 3)]);
        assert_eq!(reflection.push_constant_offset, 0);
        assert_eq!(reflection.push_constant_size as usize, size_of::<GuidePush>());
        assert_eq!(size_of::<GpuGuideLine>(), 16);
    }

    #[test]
    fn pick_compute_layout_matches_the_host() {
        let reflection = shader::reflect(&read_spirv(PICK_CS_SPV).expect("valid SPIR-V")).expect("shader reflects");
        let mut bindings = reflection
            .bindings
            .iter()
            .map(|binding| (binding.set, binding.binding, binding.access))
            .collect::<Vec<_>>();
        bindings.sort_unstable_by_key(|binding| (binding.0, binding.1));

        assert_eq!(reflection.local_size, [1, 1, 1]);
        assert_eq!(reflection.push_constant_offset, 0);
        assert_eq!(reflection.push_constant_size as usize, size_of::<PickPush>());
        assert_eq!(bindings.len(), 2);
        assert_eq!((bindings[0].0, bindings[0].1), (0, 1));
        assert_eq!((bindings[1].0, bindings[1].1), (0, 3));
        assert!(bindings[0].2.contains(vir::Access::ComputeSampled));
        assert!(bindings[1].2.contains(vir::Access::ComputeWrite));
    }

    #[test]
    fn capture_compute_layout_matches_the_host() {
        let reflection = shader::reflect(&read_spirv(CAPTURE_CS_SPV).expect("valid SPIR-V")).expect("shader reflects");
        let mut bindings = reflection
            .bindings
            .iter()
            .map(|binding| (binding.set, binding.binding, binding.access))
            .collect::<Vec<_>>();
        bindings.sort_unstable_by_key(|binding| (binding.0, binding.1));

        assert_eq!(
            reflection.local_size,
            [CAPTURE_WORKGROUP_SIZE, CAPTURE_WORKGROUP_SIZE, 1]
        );
        assert_eq!(reflection.push_constant_offset, 0);
        assert_eq!(reflection.push_constant_size as usize, size_of::<[u32; 2]>());
        assert_eq!(bindings.len(), 2);
        assert_eq!((bindings[0].0, bindings[0].1), (0, 0));
        assert_eq!((bindings[1].0, bindings[1].1), (0, 1));
        assert!(bindings[0].2.contains(vir::Access::ComputeSampled));
        assert!(bindings[1].2.contains(vir::Access::ComputeWrite));
    }

    #[test]
    fn area_color_compute_layout_matches_the_host() {
        let reflection =
            shader::reflect(&read_spirv(AREA_COLOR_CS_SPV).expect("valid SPIR-V")).expect("shader reflects");
        let mut bindings = reflection
            .bindings
            .iter()
            .map(|binding| (binding.set, binding.binding, binding.access))
            .collect::<Vec<_>>();
        bindings.sort_unstable_by_key(|binding| (binding.0, binding.1));

        assert_eq!(reflection.local_size, [64, 1, 1]);
        assert_eq!(reflection.push_constant_offset, 0);
        assert_eq!(reflection.push_constant_size as usize, size_of::<AreaColorPush>());
        assert_eq!(bindings.len(), 2);
        assert_eq!((bindings[0].0, bindings[0].1), (0, 1));
        assert_eq!((bindings[1].0, bindings[1].1), (1, 0));
        assert!(bindings[0].2.contains(vir::Access::ComputeRW));
        assert!(bindings[1].2.contains(vir::Access::ComputeSampled));
    }

    #[test]
    fn sprite_fragment_passes_read_sprites_and_textures() {
        for (spirv, expected) in [
            (SPRITE_SCENE_FS_SPV, vec![(0, 1), (1, 0)]),
            (SPRITE_VIS_FS_SPV, vec![(0, 1), (1, 0)]),
            (SPRITE_SHADE_FS_SPV, vec![(0, 1), (1, 0)]),
            (SPRITE_OVERLAY_LIGHT_FS_SPV, vec![(0, 1), (1, 0)]),
        ] {
            let reflection = shader::reflect(&read_spirv(spirv).expect("valid SPIR-V")).expect("shader reflects");
            let mut bindings = reflection
                .bindings
                .iter()
                .map(|binding| (binding.set, binding.binding))
                .collect::<Vec<_>>();
            bindings.sort_unstable();

            assert_eq!(bindings, expected);
        }
    }

    #[test]
    fn shader_stages_declare_the_constants_the_host_specializes() {
        let stages = [
            (GEOMETRY_VS_SPV, spec::SHOW_AREAS),
            (GEOMETRY_VS_SPV, spec::SHOW_AREA_OUTLINES),
            (GEOMETRY_VS_SPV, spec::SPRITE_INDIRECT),
            (GEOMETRY_VS_SPV, spec::SPRITE_LIST),
            (SPRITE_SHADE_FS_SPV, spec::SPRITE_EDGE_ONLY),
            (SPRITE_SHADE_FS_SPV, spec::SHOW_AREAS),
            (SPRITE_SCENE_FS_SPV, spec::SHOW_AREAS),
            (SPRITE_VIS_FS_SPV, spec::NODE_MODE),
            (SPRITE_CULL_CLASSIFY_CS_SPV, spec::SHOW_AREAS),
            (SPRITE_CULL_CLASSIFY_CS_SPV, spec::SHOW_AREA_OUTLINES),
            (SPRITE_CULL_COMPACT_CS_SPV, spec::SHOW_AREAS),
            (SPRITE_CULL_COMPACT_CS_SPV, spec::SHOW_AREA_OUTLINES),
            (INTERACTION_FS_SPV, spec::HIGHLIGHT_TINT),
            (INTERACTION_FS_SPV, spec::DELETE_MODE),
        ];

        for (spirv, id) in stages {
            let reflection = shader::reflect(&read_spirv(spirv).expect("valid SPIR-V")).expect("shader reflects");
            let declared = reflection
                .spec_constants
                .iter()
                .map(|constant| constant.id)
                .collect::<Vec<_>>();

            assert!(
                declared.contains(&id),
                "stage does not declare specialization constant {id}"
            );
        }
    }
}
