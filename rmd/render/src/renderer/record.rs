use ash::vk;
use vir::{
    Access, BlendPreset, ClearValue, ColorBlendAttachmentState, DRAW_INDIRECT_STRIDE, DepthState, DomainFlag,
    ImageInfo, Module, Program, RasterizationState, Rect2D, ValueId,
};

use super::{
    Renderer,
    cull::{CullSlots, CullValues, record_sprite_cull, sprite_draw_offset},
    gpu::{SPRITE_LIST_AREA_OUTLINE, SPRITE_LIST_OVERLAY_LIGHT, SPRITE_LIST_SCENE},
    guides::{GuidePush, GuideSlots},
    lighting::{LightingPush, LightingSlots},
    preview::PreviewSlots,
};
use crate::{Frame, GpuError, extent3d, imgui::ImGuiSlots, spec};

const LEVEL_DEPTH_FORMAT: vk::Format = vk::Format::D16_UNORM;
const LEVEL_DEPTH_WRITE: DepthState = DepthState::testing(vk::CompareOp::ALWAYS);
const HIGHLIGHT_BLEND: ColorBlendAttachmentState = ColorBlendAttachmentState {
    blend_enable: true,
    src_color_blend_factor: vk::BlendFactor::DST_ALPHA,
    dst_color_blend_factor: vk::BlendFactor::ONE_MINUS_SRC_ALPHA,
    color_blend_op: vk::BlendOp::ADD,
    src_alpha_blend_factor: vk::BlendFactor::ZERO,
    dst_alpha_blend_factor: vk::BlendFactor::ONE,
    alpha_blend_op: vk::BlendOp::ADD,
    color_write_mask: vk::ColorComponentFlags::RGBA,
};

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(super) struct CameraPush {
    pub(super) center: [f32; 2],
    pub(super) viewport: [f32; 2],
    pub(super) zoom: f32,
    pub(super) base: u32,
    pub(super) focused_area_owner: [u32; 2],
    pub(super) placement_flash_owner: [u32; 2],
    pub(super) placement_flash_strength: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct InteractionPush {
    pub(super) cursor: [u32; 2],
    pub(super) cursor_valid: u32,
    pub(super) picked_owner: [u32; 2],
    pub(super) stripe_offset: f32,
    pub(super) focused_area_owner: [u32; 2],
    pub(super) connected_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct PickPush {
    pub(super) cursor: [u32; 2],
    pub(super) cursor_valid: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(super) struct BlurPush {
    sample_step: [f32; 2],
}

impl BlurPush {
    pub(super) fn for_camera(camera: &CameraPush) -> Self {
        Self {
            sample_step: [camera.zoom / camera.viewport[0], camera.zoom / camera.viewport[1]],
        }
    }
}

pub(super) struct InteractionSlots {
    pub(super) push: ValueId,
    pub(super) has_highlight: ValueId,
    pub(super) needs_visibility: ValueId,
    pub(super) connected_owners: ValueId,
    pub(super) pick_push: ValueId,
    pub(super) highlight_draw: ValueId,
}

fn scene_attachments(
    color: ValueId, emissive: ValueId, level_depth: Option<ValueId>, commands: ValueId, visible: ValueId,
) -> Vec<(ValueId, Access)> {
    let mut attachments = vec![
        (color, Access::ColorRW),
        (emissive, Access::ColorRW),
        (commands, Access::IndirectRead),
        (visible, Access::VertexRead),
    ];
    if let Some(level_depth) = level_depth {
        attachments.push((level_depth, Access::DepthStencilRW));
    }
    attachments
}

fn end_scene_pass(module: &mut Module, level_depth: Option<ValueId>) -> (ValueId, ValueId, Option<ValueId>) {
    match level_depth {
        Some(_) => {
            let [color, emissive, _, _, level_depth] = module.end_rendering();
            (color, emissive, Some(level_depth))
        },
        None => {
            let [color, emissive, _, _] = module.end_rendering();
            (color, emissive, None)
        },
    }
}

/// The spec constants and layout a recording is built for, so a change to one re-records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FrameGraphState {
    extent: [u32; 2],
    with_imgui: bool,
    show_areas: bool,
    show_area_outlines: bool,
    blur_underlays: bool,
    map_views: Vec<MapViewState>,
    pub(super) viewports: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MapViewState {
    rect: crate::MapViewRect,
    highlight_tint: bool,
    delete_mode: bool,
    node_mode: bool,
    preview: bool,
    lighting: bool,
}

impl FrameGraphState {
    pub(super) fn new(viewport: vk::Extent2D, with_imgui: bool, frame: &Frame<'_>) -> Self {
        Self {
            viewports: Vec::new(),
            extent: [viewport.width, viewport.height],
            with_imgui,
            show_areas: frame.show_areas,
            show_area_outlines: frame.show_area_outlines,
            blur_underlays: frame.underlay_depth > 0,
            map_views: frame
                .map_views
                .iter()
                .filter(|view| !view.rect.is_empty())
                .map(|view| MapViewState {
                    rect: view.rect,
                    highlight_tint: view.interaction.highlight.is_tint(),
                    delete_mode: view.interaction.mode.is_delete(),
                    node_mode: view.interaction.mode.is_node(),
                    preview: view.preview.is_some_and(|preview| !preview.sprites.is_empty()),
                    lighting: view.lighting.is_some_and(|lighting| !lighting.tiles.is_empty()),
                })
                .collect(),
        }
    }
}

pub(super) struct Recorded {
    pub(super) program: Program,
    pub(super) sprites: ValueId,
    pub(super) lights: Option<ValueId>,
    pub(super) map_views: Vec<MapViewPass>,
    pub(super) pick_result: Option<ValueId>,
    pub(super) state: FrameGraphState,
    pub(super) ui: Option<ImGuiSlots>,
    pub(super) viewport_ui: Vec<Option<ImGuiSlots>>,
}

pub(super) struct MapViewPass {
    pub(super) scene_attachment: ValueId,
    pub(super) visibility_attachment: ValueId,
    pub(super) output_attachment: ValueId,
    pub(super) extent: ValueId,
    pub(super) area_outlines: ValueId,
    pub(super) camera: ValueId,
    pub(super) underlay_camera: ValueId,
    pub(super) active_camera: ValueId,
    pub(super) blur: ValueId,
    pub(super) cull: CullSlots,
    pub(super) lighting: Option<LightingSlots>,
    pub(super) preview: Option<PreviewSlots>,
    pub(super) should_pick: Option<ValueId>,
    pub(super) interaction: Option<InteractionSlots>,
    pub(super) guides: Option<GuideSlots>,
}

pub(super) struct MapScene {
    pub(super) extent: ValueId,
    pub(super) area_outlines: ValueId,
    pub(super) scene: ValueId,
    pub(super) camera: ValueId,
    pub(super) underlay_camera: ValueId,
    pub(super) active_camera: ValueId,
    pub(super) blur: ValueId,
    draw_commands: ValueId,
    visible_indices: ValueId,
    pub(super) cull: CullSlots,
    pub(super) lighting: Option<LightingSlots>,
}

impl Renderer {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn record_map_scene(
        &self, module: &mut Module, sprites: ValueId, lights: Option<ValueId>, viewport: vk::Extent2D,
        show_areas: bool, show_area_outlines: bool, blur_underlays: bool, name: &str,
    ) -> MapScene {
        let extent = module.declare_extent_3d_var(&format!("{name} extent"), extent3d(viewport));
        // sprites blend in gamma space like byond, lighting or resolve converts to srgb
        let mut scene_attachment = module.transient_image_sized(
            &ImageInfo::color_target(viewport, vk::Format::R8G8B8A8_UNORM)
                .with_usage(vk::ImageUsageFlags::SAMPLED)
                .with_name(format!("{name} scene attachment")),
            extent,
        );
        scene_attachment = module.clear(scene_attachment, vir::clear::f32::BLACK);
        let mut emissive_attachment = module.transient_image_sized(
            &ImageInfo::color_target(viewport, vk::Format::R8_UNORM)
                .with_usage(vk::ImageUsageFlags::SAMPLED)
                .with_name(format!("{name} emissive attachment")),
            extent,
        );
        emissive_attachment = module.clear(emissive_attachment, vir::clear::f32::BLACK);
        // only lighting reads the level back
        let mut level_depth = lights.map(|_| {
            let depth = module.transient_image_sized(
                &ImageInfo::depth_target(viewport, LEVEL_DEPTH_FORMAT)
                    .with_usage(vk::ImageUsageFlags::SAMPLED)
                    .with_name(format!("{name} level depth")),
                extent,
            );
            module.clear(depth, ClearValue::depth(0.0))
        });

        let visible_indices = module.declare_buffer_var(&format!("{name} visible sprite indices"), Access::VertexRead);
        let chunks = module.declare_buffer_var(&format!("{name} cull chunks"), Access::ComputeRead);
        let draw_commands = module.declare_buffer_var(&format!("{name} sprite draw commands"), Access::IndirectRead);

        let camera = module.declare_bytes_var(&format!("{name} camera"), size_of::<CameraPush>() as u32);
        let underlay_camera =
            module.declare_bytes_var(&format!("{name} underlay camera"), size_of::<CameraPush>() as u32);
        let active_camera = module.declare_bytes_var(&format!("{name} active camera"), size_of::<CameraPush>() as u32);
        let blur = module.declare_bytes_var(&format!("{name} blur"), size_of::<BlurPush>() as u32);

        let (cull_values, cull) = record_sprite_cull(
            module,
            self.pipelines.cull,
            sprites,
            CullValues {
                visible: visible_indices,
                chunks,
                commands: draw_commands,
            },
            show_areas,
            show_area_outlines,
            &format!("{name} cull"),
        );
        let draw_commands = cull_values.commands;
        let visible_indices = cull_values.visible;

        if blur_underlays {
            let mut underlay_attachment = module.transient_image_sized(
                &ImageInfo::color_target(viewport, vk::Format::R8G8B8A8_UNORM)
                    .with_usage(vk::ImageUsageFlags::SAMPLED)
                    .with_name(format!("{name} underlay attachment")),
                extent,
            );
            underlay_attachment = module.clear(underlay_attachment, vir::clear::f32::TRANSPARENT);
            // emissive and level still take the underlays so lighting picks their own level
            module
                .begin_rendering(scene_attachments(
                    underlay_attachment,
                    emissive_attachment,
                    level_depth,
                    draw_commands,
                    visible_indices,
                ))
                .with_name(format!("{name} underlays"))
                .bind_graphics_pipeline(self.pipelines.scene)
                .set_primitive_topology(vk::PrimitiveTopology::TRIANGLE_STRIP)
                .set_viewport(0, Rect2D::framebuffer())
                .set_scissor(0, Rect2D::framebuffer())
                .set_color_blend(0, BlendPreset::PremultipliedAlphaBlend)
                .set_color_blend(1, BlendPreset::PremultipliedAlphaBlend)
                .set_depth(LEVEL_DEPTH_WRITE)
                .set_rasterization(RasterizationState {
                    cull_mode: vk::CullModeFlags::NONE,
                    ..Default::default()
                })
                .bind_buffer(0, 1, sprites)
                .bind_buffer(0, 2, visible_indices)
                .specialize_constant(spec::SPRITE_INDIRECT, true)
                .specialize_constant(spec::SHOW_AREAS, show_areas)
                .specialize_constant(spec::SHOW_AREA_OUTLINES, show_area_outlines)
                .push_constants_from(underlay_camera)
                .draw_indirect_at(
                    draw_commands,
                    sprite_draw_offset(SPRITE_LIST_SCENE, false),
                    1u32,
                    DRAW_INDIRECT_STRIDE,
                );
            (underlay_attachment, emissive_attachment, level_depth) = end_scene_pass(module, level_depth);

            [scene_attachment] = module
                .begin_rendering([(scene_attachment, Access::ColorRW)])
                .with_name(format!("{name} underlay blur"))
                .bind_graphics_pipeline(self.pipelines.blur)
                .set_primitive_topology(vk::PrimitiveTopology::TRIANGLE_LIST)
                .set_viewport(0, Rect2D::framebuffer())
                .set_scissor(0, Rect2D::framebuffer())
                .broadcast_color_blend(BlendPreset::PremultipliedAlphaBlend)
                .bind_texture(0, 0, underlay_attachment, self.blur_sampler)
                .push_constants_from(blur)
                .draw(3, 1)
                .end_rendering();
        }

        module
            .begin_rendering(scene_attachments(
                scene_attachment,
                emissive_attachment,
                level_depth,
                draw_commands,
                visible_indices,
            ))
            .with_name(format!("{name} sprites"))
            .bind_graphics_pipeline(self.pipelines.scene)
            .set_primitive_topology(vk::PrimitiveTopology::TRIANGLE_STRIP)
            .set_viewport(0, Rect2D::framebuffer())
            .set_scissor(0, Rect2D::framebuffer())
            .set_color_blend(0, BlendPreset::PremultipliedAlphaBlend)
            .set_color_blend(1, BlendPreset::PremultipliedAlphaBlend)
            .set_depth(LEVEL_DEPTH_WRITE)
            .set_rasterization(RasterizationState {
                cull_mode: vk::CullModeFlags::NONE,
                ..Default::default()
            })
            .bind_buffer(0, 1, sprites)
            .bind_buffer(0, 2, visible_indices)
            .specialize_constant(spec::SPRITE_INDIRECT, true)
            .specialize_constant(spec::SHOW_AREAS, show_areas)
            .specialize_constant(spec::SHOW_AREA_OUTLINES, show_area_outlines)
            .push_constants_from(active_camera)
            .draw_indirect_at(
                draw_commands,
                sprite_draw_offset(SPRITE_LIST_SCENE, true),
                1u32,
                DRAW_INDIRECT_STRIDE,
            );
        (scene_attachment, emissive_attachment, level_depth) = end_scene_pass(module, level_depth);

        let lighting = if let (Some(light_tiles), Some(level_depth)) = (lights, level_depth) {
            let mut overlay_lightmap = module.transient_image_sized(
                &ImageInfo::color_target(viewport, vk::Format::R16G16B16A16_SFLOAT)
                    .with_usage(vk::ImageUsageFlags::SAMPLED)
                    .with_name(format!("{name} overlay lightmap")),
                extent,
            );
            overlay_lightmap = module.clear(overlay_lightmap, vir::clear::f32::BLACK);
            let lit_level;
            [overlay_lightmap, lit_level, _, _] = module
                .begin_rendering([
                    (overlay_lightmap, Access::ColorRW),
                    (level_depth, Access::DepthStencilRW),
                    (draw_commands, Access::IndirectRead),
                    (visible_indices, Access::VertexRead),
                ])
                .with_name(format!("{name} overlay lights"))
                .bind_graphics_pipeline(self.pipelines.overlay_light)
                .set_primitive_topology(vk::PrimitiveTopology::TRIANGLE_STRIP)
                .set_viewport(0, Rect2D::framebuffer())
                .set_scissor(0, Rect2D::framebuffer())
                .broadcast_color_blend(BlendPreset::Additive)
                // a light only lands where its own level is the one drawn
                .set_depth(DepthState::read_only(vk::CompareOp::EQUAL))
                .set_rasterization(RasterizationState {
                    cull_mode: vk::CullModeFlags::NONE,
                    ..Default::default()
                })
                .bind_buffer(0, 1, sprites)
                .bind_buffer(0, 2, visible_indices)
                .specialize_constant(spec::SPRITE_INDIRECT, true)
                .specialize_constant(spec::SHOW_AREAS, show_areas)
                .specialize_constant(spec::SHOW_AREA_OUTLINES, show_area_outlines)
                .specialize_constant(spec::SPRITE_LIST, SPRITE_LIST_OVERLAY_LIGHT)
                .push_constants_from(underlay_camera)
                .draw_indirect_at(
                    draw_commands,
                    sprite_draw_offset(SPRITE_LIST_OVERLAY_LIGHT, false),
                    1u32,
                    DRAW_INDIRECT_STRIDE,
                )
                .push_constants_from(active_camera)
                .draw_indirect_at(
                    draw_commands,
                    sprite_draw_offset(SPRITE_LIST_OVERLAY_LIGHT, true),
                    1u32,
                    DRAW_INDIRECT_STRIDE,
                )
                .end_rendering();
            let push = module.declare_bytes_var(&format!("{name} lighting"), size_of::<LightingPush>() as u32);
            let mut lit_attachment = module.transient_image_sized(
                &ImageInfo::color_target(viewport, vk::Format::R8G8B8A8_SRGB)
                    .with_usage(vk::ImageUsageFlags::SAMPLED)
                    .with_name(format!("{name} lit scene attachment")),
                extent,
            );
            lit_attachment = module.clear(lit_attachment, vir::clear::f32::BLACK);
            [lit_attachment] = module
                .begin_rendering([(lit_attachment, Access::ColorRW)])
                .with_name(format!("{name} static lighting"))
                .bind_graphics_pipeline(self.pipelines.lighting)
                .set_primitive_topology(vk::PrimitiveTopology::TRIANGLE_LIST)
                .set_viewport(0, Rect2D::framebuffer())
                .set_scissor(0, Rect2D::framebuffer())
                .bind_image(0, 0, scene_attachment)
                .bind_buffer(0, 1, light_tiles)
                .bind_image(0, 2, emissive_attachment)
                .bind_image(0, 3, overlay_lightmap)
                .bind_image(0, 4, lit_level)
                .push_constants_from(push)
                .draw(3, 1)
                .end_rendering();
            scene_attachment = lit_attachment;

            Some(LightingSlots { push })
        } else {
            let mut resolved_attachment = module.transient_image_sized(
                &ImageInfo::color_target(viewport, vk::Format::R8G8B8A8_SRGB)
                    .with_usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_SRC)
                    .with_name(format!("{name} resolved scene attachment")),
                extent,
            );
            resolved_attachment = module.clear(resolved_attachment, vir::clear::f32::BLACK);
            [resolved_attachment] = module
                .begin_rendering([(resolved_attachment, Access::ColorRW)])
                .with_name(format!("{name} scene resolve"))
                .bind_graphics_pipeline(self.pipelines.resolve)
                .set_primitive_topology(vk::PrimitiveTopology::TRIANGLE_LIST)
                .set_viewport(0, Rect2D::framebuffer())
                .set_scissor(0, Rect2D::framebuffer())
                .broadcast_color_blend(BlendPreset::Off)
                .bind_image(0, 0, scene_attachment)
                .draw(3, 1)
                .end_rendering();
            scene_attachment = resolved_attachment;

            None
        };

        let area_outlines = module.declare_bool_var(&format!("{name} area outlines"), show_area_outlines);
        let scene_attachment = module.set_condition(
            area_outlines,
            |m| {
                let [outlined, _, _] = m
                    .begin_rendering([
                        (scene_attachment, Access::ColorRW),
                        (draw_commands, Access::IndirectRead),
                        (visible_indices, Access::VertexRead),
                    ])
                    .with_name(format!("{name} area outlines"))
                    .bind_graphics_pipeline(self.pipelines.sprite)
                    .set_primitive_topology(vk::PrimitiveTopology::TRIANGLE_STRIP)
                    .set_viewport(0, Rect2D::framebuffer())
                    .set_scissor(0, Rect2D::framebuffer())
                    .broadcast_color_blend(BlendPreset::PremultipliedAlphaBlend)
                    .set_rasterization(RasterizationState {
                        cull_mode: vk::CullModeFlags::NONE,
                        ..Default::default()
                    })
                    .bind_buffer(0, 1, sprites)
                    .bind_buffer(0, 2, visible_indices)
                    .specialize_constant(spec::SPRITE_INDIRECT, true)
                    .specialize_constant(spec::SHOW_AREAS, show_areas)
                    .specialize_constant(spec::SHOW_AREA_OUTLINES, show_area_outlines)
                    .specialize_constant(spec::SPRITE_EDGE_ONLY, true)
                    .specialize_constant(spec::SPRITE_LIST, SPRITE_LIST_AREA_OUTLINE)
                    .push_constants_from(active_camera)
                    .draw_indirect_at(
                        draw_commands,
                        sprite_draw_offset(SPRITE_LIST_AREA_OUTLINE, true),
                        1u32,
                        DRAW_INDIRECT_STRIDE,
                    )
                    .end_rendering();
                outlined
            },
            |_| scene_attachment,
        );

        MapScene {
            extent,
            area_outlines,
            scene: scene_attachment,
            camera,
            underlay_camera,
            active_camera,
            blur,
            draw_commands,
            visible_indices,
            cull,
            lighting,
        }
    }

    pub(super) fn record(&mut self, state: FrameGraphState) -> Result<(), GpuError> {
        let swapchain = self.swapchain.as_ref().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
        let with_imgui = state.with_imgui;
        let mut module = Module::default();
        let swapchain_attachment = module.acquire_next_image(swapchain);
        let sprites = module.declare_buffer_var("sprites", Access::ComputeWrite);
        let lights = state
            .map_views
            .iter()
            .any(|view| view.lighting)
            .then(|| module.declare_buffer_var("light tiles", Access::FragmentRead));
        let pick_result = (with_imgui && !state.map_views.is_empty())
            .then(|| module.declare_buffer_var("pick result", Access::HostRead));
        let mut pick_after = pick_result;
        let mut map_views = Vec::with_capacity(state.map_views.len());

        for (index, view) in state.map_views.iter().copied().enumerate() {
            let rect = view.rect;
            let viewport = vk::Extent2D {
                width: rect.width.max(1),
                height: rect.height.max(1),
            };
            let MapScene {
                extent,
                area_outlines,
                scene: scene_attachment,
                camera,
                underlay_camera,
                active_camera,
                blur,
                draw_commands,
                visible_indices,
                cull,
                lighting,
            } = self.record_map_scene(
                &mut module,
                sprites,
                lights.filter(|_| view.lighting),
                viewport,
                state.show_areas,
                state.show_area_outlines,
                state.blur_underlays,
                &format!("map view {index}"),
            );
            let visibility_attachment = module.transient_image_sized(
                &ImageInfo::color_target(viewport, vk::Format::R32_UINT)
                    .with_usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_SRC)
                    .with_name(format!("map view {index} visibility attachment")),
                extent,
            );

            let (mut output_attachment, visibility_attachment, interaction, guides, should_pick) = if with_imgui {
                // every pass that reads visibility runs only when this one does
                let needs_visibility = module.declare_bool_var(&format!("map view {index} needs visibility"), false);
                let visibility_attachment = module.set_condition(
                    needs_visibility,
                    |m| {
                        let cleared = m.clear(visibility_attachment, vir::clear::u32::TRANSPARENT);
                        let [drawn, _, _] = m
                            .begin_rendering([
                                (cleared, Access::ColorRW),
                                (draw_commands, Access::IndirectRead),
                                (visible_indices, Access::VertexRead),
                            ])
                            .with_name(format!("map view {index} visibility"))
                            .bind_graphics_pipeline(self.pipelines.visibility)
                            .specialize_constant(spec::NODE_MODE, view.node_mode)
                            .set_primitive_topology(vk::PrimitiveTopology::TRIANGLE_STRIP)
                            .set_viewport(0, Rect2D::framebuffer())
                            .set_scissor(0, Rect2D::framebuffer())
                            .broadcast_color_blend(BlendPreset::Off)
                            .set_rasterization(RasterizationState {
                                cull_mode: vk::CullModeFlags::NONE,
                                ..Default::default()
                            })
                            .bind_buffer(0, 1, sprites)
                            .bind_buffer(0, 2, visible_indices)
                            .specialize_constant(spec::SPRITE_INDIRECT, true)
                            .specialize_constant(spec::SHOW_AREAS, state.show_areas)
                            .specialize_constant(spec::SHOW_AREA_OUTLINES, state.show_area_outlines)
                            .push_constants_from(active_camera)
                            .draw_indirect_at(
                                draw_commands,
                                sprite_draw_offset(SPRITE_LIST_SCENE, true),
                                1u32,
                                DRAW_INDIRECT_STRIDE,
                            )
                            .end_rendering();
                        drawn
                    },
                    |_| visibility_attachment,
                );

                let push = module.declare_bytes_var(
                    &format!("map view {index} interaction"),
                    size_of::<InteractionPush>() as u32,
                );
                let has_highlight = module.declare_bool_var(&format!("map view {index} has highlight"), false);
                let highlight_draw = module.declare_callback_var(&format!("map view {index} highlight draw"));
                let connected_owners =
                    module.declare_buffer_var(&format!("map view {index} connected owners"), Access::HostWrite);
                let highlighted_scene_attachment = module.set_condition(
                    has_highlight,
                    |m| {
                        let [highlighted, _] = m
                            .begin_rendering([
                                (scene_attachment, Access::ColorRW),
                                (connected_owners, Access::FragmentRead),
                            ])
                            .with_name(format!("map view {index} highlights"))
                            .bind_graphics_pipeline(self.pipelines.interaction)
                            .set_primitive_topology(vk::PrimitiveTopology::TRIANGLE_STRIP)
                            .set_viewport(0, Rect2D::framebuffer())
                            .set_scissor(0, Rect2D::framebuffer())
                            .broadcast_color_blend(HIGHLIGHT_BLEND)
                            .bind_image(0, 1, visibility_attachment)
                            .bind_buffer(0, 2, sprites)
                            .bind_buffer(0, 4, connected_owners)
                            .specialize_constant(spec::HIGHLIGHT_TINT, view.highlight_tint)
                            .specialize_constant(spec::DELETE_MODE, view.delete_mode)
                            .push_constants_from(push)
                            .record_from(highlight_draw)
                            .end_rendering();
                        highlighted
                    },
                    |_| scene_attachment,
                );

                let guide_lines =
                    module.declare_buffer_var(&format!("map view {index} guide lines"), Access::HostWrite);
                let guide_push =
                    module.declare_bytes_var(&format!("map view {index} guide push"), size_of::<GuidePush>() as u32);
                let guide_draw = module.declare_callback_var(&format!("map view {index} guide draw"));
                let has_guide_lines = module.declare_bool_var(&format!("map view {index} has guides"), false);
                let guided_scene_attachment = module.set_condition(
                    has_guide_lines,
                    |m| {
                        let [guided, _] = m
                            .begin_rendering([
                                (highlighted_scene_attachment, Access::ColorRW),
                                (guide_lines, Access::VertexRead | Access::FragmentRead),
                            ])
                            .with_name(format!("map view {index} guides"))
                            .bind_graphics_pipeline(self.pipelines.guide)
                            .set_primitive_topology(vk::PrimitiveTopology::TRIANGLE_STRIP)
                            .set_viewport(0, Rect2D::framebuffer())
                            .set_scissor(0, Rect2D::framebuffer())
                            .set_rasterization(RasterizationState {
                                cull_mode: vk::CullModeFlags::NONE,
                                ..Default::default()
                            })
                            .bind_buffer(0, 0, guide_lines)
                            .bind_image(0, 1, visibility_attachment)
                            .bind_buffer(0, 2, sprites)
                            .push_constants_from(guide_push)
                            .record_from(guide_draw)
                            .end_rendering();
                        guided
                    },
                    |_| highlighted_scene_attachment,
                );

                let pick_push =
                    module.declare_bytes_var(&format!("map view {index} pick cursor"), size_of::<PickPush>() as u32);
                let should_pick = module.declare_bool_var(&format!("map view {index} should pick"), false);
                let pick_buffer = pick_after.ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
                let next_pick = module.set_condition(
                    should_pick,
                    |m| {
                        let [_visibility, next_pick] = m
                            .begin_compute([
                                (visibility_attachment, Access::ComputeSampled),
                                (pick_buffer, Access::ComputeWrite),
                            ])
                            .with_name(format!("map view {index} mouse pick"))
                            .bind_compute_pipeline(self.pipelines.pick)
                            .bind_image(0, 1, visibility_attachment)
                            .bind_buffer(0, 3, pick_buffer)
                            .push_constants_from(pick_push)
                            .dispatch(1u32, 1u32, 1u32)
                            .end_compute();
                        next_pick
                    },
                    |_| pick_buffer,
                );
                pick_after = Some(next_pick);

                (
                    guided_scene_attachment,
                    visibility_attachment,
                    Some(InteractionSlots {
                        push,
                        has_highlight,
                        needs_visibility,
                        connected_owners,
                        pick_push,
                        highlight_draw,
                    }),
                    Some(GuideSlots {
                        has_lines: has_guide_lines,
                        lines: guide_lines,
                        push: guide_push,
                        draw: guide_draw,
                    }),
                    Some(should_pick),
                )
            } else {
                (scene_attachment, visibility_attachment, None, None, None)
            };

            let preview = if view.preview {
                let name = format!("map view {index} region preview");
                let sprites = module.declare_buffer_var(&format!("{name} sprites"), Access::HostWrite);
                let camera = module.declare_bytes_var(&format!("{name} camera"), size_of::<CameraPush>() as u32);
                let visible = module.declare_buffer_var(&format!("{name} visible indices"), Access::VertexRead);
                let chunks = module.declare_buffer_var(&format!("{name} chunks"), Access::ComputeRead);
                let commands = module.declare_buffer_var(&format!("{name} draw commands"), Access::IndirectRead);
                let (values, cull) = record_sprite_cull(
                    &mut module,
                    self.pipelines.cull,
                    sprites,
                    CullValues {
                        visible,
                        chunks,
                        commands,
                    },
                    state.show_areas,
                    state.show_area_outlines,
                    &name,
                );
                [output_attachment, _, _] = module
                    .begin_rendering([
                        (output_attachment, Access::ColorRW),
                        (values.commands, Access::IndirectRead),
                        (values.visible, Access::VertexRead),
                    ])
                    .with_name(name)
                    .bind_graphics_pipeline(self.pipelines.sprite)
                    .set_primitive_topology(vk::PrimitiveTopology::TRIANGLE_STRIP)
                    .set_viewport(0, Rect2D::framebuffer())
                    .set_scissor(0, Rect2D::framebuffer())
                    .broadcast_color_blend(BlendPreset::PremultipliedAlphaBlend)
                    .set_rasterization(RasterizationState {
                        cull_mode: vk::CullModeFlags::NONE,
                        ..Default::default()
                    })
                    .bind_buffer(0, 1, sprites)
                    .bind_buffer(0, 2, values.visible)
                    .specialize_constant(spec::SPRITE_INDIRECT, true)
                    .specialize_constant(spec::SHOW_AREAS, state.show_areas)
                    .specialize_constant(spec::SHOW_AREA_OUTLINES, state.show_area_outlines)
                    .specialize_constant(spec::SPRITE_EDGE_ONLY, false)
                    .push_constants_from(camera)
                    .draw_indirect_at(
                        values.commands,
                        sprite_draw_offset(SPRITE_LIST_SCENE, true),
                        1u32,
                        DRAW_INDIRECT_STRIDE,
                    )
                    .specialize_constant(spec::SPRITE_LIST, SPRITE_LIST_AREA_OUTLINE)
                    .draw_indirect_at(
                        values.commands,
                        sprite_draw_offset(SPRITE_LIST_AREA_OUTLINE, true),
                        1u32,
                        DRAW_INDIRECT_STRIDE,
                    )
                    .end_rendering();
                Some(PreviewSlots { sprites, camera, cull })
            } else {
                None
            };

            map_views.push(MapViewPass {
                scene_attachment,
                visibility_attachment,
                output_attachment,
                extent,
                area_outlines,
                camera,
                underlay_camera,
                active_camera,
                blur,
                cull,
                lighting,
                preview,
                should_pick,
                interaction,
                guides,
            });
        }

        let pick_host = pick_after.map(|pick| module.export(pick, Access::HostRead, DomainFlag::Host));
        let mut sampled_map_view_roots = Vec::new();
        let mut viewport_presents = Vec::new();
        let mut viewport_ui = Vec::new();
        let (presented_attachment, ui) = if with_imgui {
            for map_view in &map_views {
                let sampled = module.export(
                    map_view.output_attachment,
                    Access::FragmentSampled,
                    DomainFlag::Graphics,
                );
                sampled_map_view_roots.push(sampled);
            }

            let imgui = self.imgui.as_ref().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
            let target_attachment = module.clear(swapchain_attachment, vir::clear::f32::BLACK);
            let map_view_outputs = map_views.iter().map(|view| view.output_attachment).collect::<Vec<_>>();

            for target in &self.viewports {
                let acquired = module.acquire_next_image(&target.swapchain);
                let cleared = module.clear(acquired, vir::clear::f32::BLACK);
                let (drawn, slots) = imgui.record(&mut module, cleared, &map_view_outputs);
                viewport_presents.push(module.present(drawn));
                viewport_ui.push(slots);
            }

            imgui.record(&mut module, target_attachment, &map_view_outputs)
        } else if let Some(map_view) = map_views.first() {
            (
                module.blit_filtered(map_view.output_attachment, swapchain_attachment, vk::Filter::NEAREST),
                None,
            )
        } else {
            (module.clear(swapchain_attachment, vir::clear::f32::BLACK), None)
        };
        let present = module.present(presented_attachment);
        let mut roots = sampled_map_view_roots;
        if let Some(pick_host) = pick_host {
            roots.push(pick_host);
        }
        roots.extend(viewport_presents);
        roots.push(present);
        let program = module.compile_all(&self.graph, &roots)?;

        self.recorded = Some(Recorded {
            program,
            sprites,
            lights,
            map_views,
            pick_result,
            state,
            ui,
            viewport_ui,
        });

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use ash::vk;

    use super::*;
    use crate::{
        HighlightStyle, InteractionMode, LightTile, LightingFrame, MapViewFrame, MapViewRect,
        renderer::fixtures::{map_view, map_view_with_sprites, owner, sprite},
    };

    fn graph_state(target: vk::Extent2D, with_imgui: bool, map_views: &[MapViewFrame<'_>]) -> FrameGraphState {
        FrameGraphState::new(
            target,
            with_imgui,
            &Frame {
                map_views,
                underlay_depth: 0,
                show_areas: false,
                show_area_outlines: true,
                picking: None,
            },
        )
    }

    #[test]
    fn frame_graph_state_ignores_dynamic_frame_data() {
        let target = vk::Extent2D {
            width: 1280,
            height: 720,
        };
        let rect = MapViewRect {
            x: 100,
            y: 50,
            width: 640,
            height: 480,
        };
        let unchanged = [map_view(rect)];
        let mut dynamic = map_view(rect);
        dynamic.camera.x = 32.0;
        dynamic.camera.zoom = 2.0;
        dynamic.interaction.cursor = Some([120, 80]);
        dynamic.interaction.picked = Some(owner());
        let dynamic = [dynamic];

        assert_eq!(
            graph_state(target, true, &unchanged),
            graph_state(target, true, &dynamic),
        );
    }

    #[test]
    fn frame_graph_state_changes_with_the_layout() {
        let target = vk::Extent2D {
            width: 1280,
            height: 720,
        };
        let left = MapViewRect {
            x: 0,
            y: 0,
            width: 640,
            height: 720,
        };
        let right = MapViewRect {
            x: 640,
            y: 0,
            width: 640,
            height: 720,
        };
        let base = graph_state(target, true, &[map_view(left), map_view(right)]);

        let moved = MapViewRect { x: 32, ..left };
        assert_ne!(base, graph_state(target, true, &[map_view(moved), map_view(right)]));

        let resized = MapViewRect { width: 600, ..left };
        assert_ne!(base, graph_state(target, true, &[map_view(resized), map_view(right)]));
        assert_ne!(base, graph_state(target, true, &[map_view(left)]));
        assert_ne!(base, graph_state(target, true, &[map_view(right), map_view(left)]));
        assert_ne!(
            base,
            graph_state(
                vk::Extent2D {
                    width: 1920,
                    height: 1080,
                },
                true,
                &[map_view(left), map_view(right)],
            )
        );
        assert_ne!(base, graph_state(target, false, &[map_view(left), map_view(right)]));
    }

    /// Every toggle the shaders specialize on has to be part of the key, or a recording outlives
    /// the constants it was built with.
    #[test]
    fn frame_graph_state_changes_with_the_specialized_toggles() {
        let target = vk::Extent2D {
            width: 1280,
            height: 720,
        };
        let rect = MapViewRect {
            x: 0,
            y: 0,
            width: 1280,
            height: 720,
        };
        let views = [map_view(rect)];
        let frame = |show_areas, show_area_outlines| Frame {
            map_views: &views,
            underlay_depth: 0,
            show_areas,
            show_area_outlines,
            picking: None,
        };
        let base = FrameGraphState::new(target, true, &frame(false, true));

        assert_ne!(base, FrameGraphState::new(target, true, &frame(true, true)));
        assert_ne!(base, FrameGraphState::new(target, true, &frame(false, false)));

        let mut tinted = map_view(rect);
        tinted.interaction.highlight = HighlightStyle::Tint;
        assert_ne!(base, graph_state(target, true, &[tinted]));

        let mut deleting = map_view(rect);
        deleting.interaction.mode = InteractionMode::Delete { pick: None };
        assert_ne!(base, graph_state(target, true, &[deleting]));

        let mut nodes = map_view(rect);
        nodes.interaction.mode = InteractionMode::Node { pick: None };
        assert_ne!(base, graph_state(target, true, &[nodes]));

        let tiles = [LightTile { corners: [[1.0; 3]; 4] }];
        let mut lit = map_view(rect);
        lit.lighting = Some(LightingFrame {
            size: [1, 1, 1],
            tiles: &tiles,
            tile_size: 32,
            minimum_brightness: 0.0,
            revision: 1,
            pending_updates: &[],
        });
        assert_ne!(base, graph_state(target, true, &[lit]));
    }

    /// The cull buffers are renderer-owned and grown in place, so neither the sprite count nor the
    /// number of levels a view shows is allowed to force a recompile.
    #[test]
    fn frame_graph_state_ignores_sprite_counts_and_level_counts() {
        let target = vk::Extent2D {
            width: 1280,
            height: 720,
        };
        let rect = MapViewRect {
            width: 640,
            height: 480,
            ..Default::default()
        };
        let one_sprite = [sprite(1)];
        let two_sprites = [sprite(1), sprite(1)];
        let mut layered = map_view_with_sprites(rect, &two_sprites);
        layered.level_count = 2;

        let one = graph_state(target, true, &[map_view_with_sprites(rect, &one_sprite)]);
        let two = graph_state(target, true, &[layered]);

        assert_eq!(one, two);
    }

    #[test]
    fn empty_map_views_do_not_create_graph_passes() {
        let target = vk::Extent2D {
            width: 1280,
            height: 720,
        };
        let rect = MapViewRect {
            x: 0,
            y: 0,
            width: 640,
            height: 720,
        };
        let visible = graph_state(target, true, &[map_view(rect)]);
        let visible_with_empty = graph_state(target, true, &[map_view(rect), map_view(MapViewRect::default())]);
        let hidden = graph_state(target, true, &[map_view(MapViewRect::default())]);

        assert_eq!(visible, visible_with_empty);
        assert_ne!(visible, hidden);
        assert!(
            hidden.map_views.is_empty(),
            "an empty editor declares no map-view attachments"
        );
    }
}
