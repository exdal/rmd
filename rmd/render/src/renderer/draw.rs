use ash::vk;
use dear_imgui_rs::render::{PendingFrame, ReconciledFrame};
use vir::{AllocatorKind, Buffer, FrameAllocator, PassCallback, Rect2D, SuperFrameAllocator};

use super::{
    PlatformWindows, Renderer,
    cull::bind_sprite_cull,
    gpu::owner_words,
    guides::GuidePush,
    lighting::LightingPush,
    preview::preview_plan,
    record::{BlurPush, CameraPush, FrameGraphState, InteractionPush, PickPush},
    sprites::{VisibleRange, visible_range},
};
use crate::{
    Frame, GpuError, PickRequest, PickResult, VisibilityId, extent3d,
    imgui::{ImGuiFrame, ImGuiPass, collect_sprite_sheets},
    viewport::{SecondaryViewport, ViewportTarget, sync_targets},
};

const HIGHLIGHT_STRIPE_PERIOD: f32 = 12.0;
const HIGHLIGHT_STRIPE_SPEED: f32 = 12.0;

#[derive(Debug, Clone)]
pub(super) struct MapViewPlan {
    pub(super) camera: CameraPush,
    pub(super) sprite_base: u32,
    pub(super) visible: VisibleRange,
}

impl Renderer {
    pub fn draw(&mut self, frame: &Frame<'_>) -> Result<(), GpuError> {
        self.draw_inner(frame, None, None)?;

        Ok(())
    }

    /// `platform_windows` runs once the frame's textures are reconciled, updates ImGui's platform
    /// windows and returns the secondary viewports to draw alongside the main one.
    pub fn draw_imgui(
        &mut self, frame: &Frame<'_>, pending: PendingFrame<'_>,
        mut platform_windows: impl FnMut(&mut ReconciledFrame<'_>) -> Vec<SecondaryViewport>,
    ) -> Result<Option<(usize, PickRequest, PickResult)>, GpuError> {
        if pending.draw_requirements().requires_raw_callback_support() {
            return Err(GpuError::RawDrawCallback);
        }

        if self.imgui.is_none() {
            let mut imgui = ImGuiPass::new(&mut self.device)?;
            if let Err(error) = imgui.declare_pipeline(&mut self.graph) {
                imgui.destroy(&mut self.device);

                return Err(error);
            }

            self.imgui = Some(imgui);
        }

        self.draw_inner(frame, None, Some((pending, &mut platform_windows)))
    }

    fn draw_inner(
        &mut self, frame: &Frame<'_>, viewport: Option<vk::Extent2D>,
        imgui: Option<(PendingFrame<'_>, PlatformWindows<'_>)>,
    ) -> Result<Option<(usize, PickRequest, PickResult)>, GpuError> {
        self.frame += 1;
        if self.stale {
            self.recreate_swapchain()?;
        }

        // taken out for the frame so `self` stays free to re-record around the borrowed allocator
        let mut frames = self.frames.take().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
        let result = self.draw_frame(&mut frames, frame, viewport, imgui);
        self.frames = Some(frames);

        result
    }

    fn prepare_ui(
        &mut self, next: &mut FrameAllocator, pending: PendingFrame<'_>, platform_windows: PlatformWindows<'_>,
    ) -> Result<(ImGuiFrame, Vec<ImGuiFrame>), GpuError> {
        let imgui = self.imgui.as_mut().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
        let feedback = imgui.poll_textures(&mut self.device, &mut self.graph, next, pending.texture_requests())?;
        let mut reconciled = pending
            .reconcile_texture_feedback(feedback)
            .map_err(|e| GpuError::ImGui(e.to_string()))?;

        let secondary = platform_windows(&mut reconciled);
        if sync_targets(
            &mut self.device,
            &mut self.viewports,
            &mut self.failed_viewports,
            &secondary,
        )? {
            self.recorded = None;
        }

        collect_sprite_sheets(reconciled.draw_data(), &mut self.requested_sheets);
        for viewport in &secondary {
            collect_sprite_sheets(viewport.draw_data(), &mut self.requested_sheets);
        }
        self.load_requested_sheets()?;

        let fallback = self.fallback.as_ref().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
        let imgui = self.imgui.as_ref().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
        let main = imgui.prepare(next, reconciled.draw_data(), self.extent, &self.sheets, fallback)?;
        let mut others = Vec::with_capacity(self.viewports.len());
        for target in &self.viewports {
            let viewport = secondary
                .iter()
                .find(|viewport| viewport.id() == target.id)
                .ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
            others.push(imgui.prepare(next, viewport.draw_data(), target.extent, &self.sheets, fallback)?);
        }

        Ok((main, others))
    }

    fn draw_frame(
        &mut self, frames: &mut SuperFrameAllocator, frame: &Frame<'_>, viewport: Option<vk::Extent2D>,
        imgui: Option<(PendingFrame<'_>, PlatformWindows<'_>)>,
    ) -> Result<Option<(usize, PickRequest, PickResult)>, GpuError> {
        let next = frames.get_next_frame()?;
        let with_imgui = imgui.is_some();
        let ui_frames = match imgui {
            Some((pending, platform_windows)) => Some(self.prepare_ui(next, pending, platform_windows)?),
            None => None,
        };

        let viewport = viewport.unwrap_or(self.extent);
        let mut graph_state = FrameGraphState::new(viewport, with_imgui, frame);
        graph_state.viewports = self.viewports.iter().map(|target| target.id).collect();
        if self
            .recorded
            .as_ref()
            .is_none_or(|recorded| recorded.state != graph_state)
        {
            self.record(graph_state)?;
        }

        self.prepare_sprites(frame)?;
        self.prepare_lighting(frame)?;
        self.prepare_cull_buffers(frame)?;
        self.prepare_previews(frame)?;
        self.load_requested_sheets()?;
        let stripe_offset =
            (self.highlight_started_at.elapsed().as_secs_f32() * HIGHLIGHT_STRIPE_SPEED) % HIGHLIGHT_STRIPE_PERIOD;

        let recorded = self.recorded.as_mut().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
        let prepared_guides = Self::prepare_guides(next, frame)?;
        let sprites = self.sprites.as_ref().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
        recorded.program.set(recorded.sprites, sprites);
        if let Some(slot) = recorded.lights {
            let lights = self.lights.as_ref().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
            recorded.program.set(slot, lights);
        }

        let plans: Vec<MapViewPlan> = frame
            .map_views
            .iter()
            .zip(&self.uploaded)
            .filter(|(view, _)| !view.rect.is_empty())
            .map(|(view, uploaded)| {
                let (placement_flash_owner, placement_flash_strength) = view
                    .interaction
                    .placement_flash
                    .filter(|flash| flash.strength.is_finite() && flash.strength > 0.0)
                    .map_or(([0; 2], 0.0), |flash| {
                        (owner_words(flash.owner), flash.strength.min(1.0))
                    });
                let visible = visible_range(&uploaded.ranges, view.active_z, frame.underlay_depth);
                let logical = [
                    view.camera.viewport_width.max(1) as f32,
                    view.camera.viewport_height.max(1) as f32,
                ];

                MapViewPlan {
                    camera: CameraPush {
                        center: [view.camera.x, view.camera.y],
                        viewport: logical,
                        zoom: view.camera.zoom,
                        base: uploaded.base,
                        focused_area_owner: view.focused_area.map(owner_words).unwrap_or([0; 2]),
                        placement_flash_owner,
                        placement_flash_strength,
                    },
                    sprite_base: uploaded.base,
                    visible,
                }
            })
            .collect();

        let visible: Vec<_> = frame
            .map_views
            .iter()
            .zip(&self.uploaded)
            .zip(&self.uploaded_lighting)
            .filter(|((view, _), _)| !view.rect.is_empty())
            .collect();
        if plans.len() != recorded.map_views.len() || visible.len() != recorded.map_views.len() {
            return Err(vk::Result::ERROR_INITIALIZATION_FAILED.into());
        }

        let picking = frame
            .picking
            .filter(|index| frame.map_views.get(*index).is_some_and(|view| !view.rect.is_empty()));
        let picking_map = picking.map(|index| {
            frame.map_views[..index]
                .iter()
                .filter(|view| !view.rect.is_empty())
                .count()
        });
        let cursor = picking.and_then(|index| {
            let view = &frame.map_views[index];
            view.interaction
                .cursor
                .filter(|cursor| cursor[0] < view.rect.width && cursor[1] < view.rect.height)
        });
        let pick = picking.and_then(|index| frame.map_views[index].interaction.mode.pick());
        if let Some(slot) = recorded.pick_result {
            let pick_readback = self
                .pick_readback
                .as_ref()
                .ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
            recorded.program.set(slot, pick_readback);
        }

        for (index, map_view) in recorded.map_views.iter().enumerate() {
            let (((view, _), uploaded_lighting), plan) = (visible[index], plans[index].clone());
            let map_extent = vk::Extent2D {
                width: view.rect.width.max(1),
                height: view.rect.height.max(1),
            };
            recorded.program.set(map_view.extent, extent3d(map_extent));
            recorded.program.set_bytes(map_view.camera, &plan.camera);
            recorded
                .program
                .set_bytes(map_view.underlay_camera, &CameraPush { base: 0, ..plan.camera });
            recorded.program.set_bytes(
                map_view.active_camera,
                &CameraPush {
                    base: plan.visible.underlay_count,
                    ..plan.camera
                },
            );
            recorded
                .program
                .set_bytes(map_view.blur, &BlurPush::for_camera(&plan.camera));

            let buffers = self.cull.get(index).ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
            recorded.program.set(map_view.cull.visible, buffers.visible);
            recorded.program.set(map_view.cull.chunks, buffers.chunks);
            recorded.program.set(map_view.cull.commands, buffers.commands);
            bind_sprite_cull(&mut recorded.program, &map_view.cull, &plan, buffers);

            if let Some(slots) = &map_view.lighting {
                let lighting = view.lighting.ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
                if uploaded_lighting.revision != Some(lighting.revision) || uploaded_lighting.size != lighting.size {
                    return Err(vk::Result::ERROR_INITIALIZATION_FAILED.into());
                }
                recorded.program.set_bytes(
                    slots.push,
                    &LightingPush {
                        center: plan.camera.center,
                        viewport: plan.camera.viewport,
                        zoom: plan.camera.zoom,
                        tile_size: lighting.tile_size.max(1) as f32,
                        map_size: [lighting.size[0], lighting.size[1]],
                        level_count: lighting.size[2],
                        base: uploaded_lighting.base,
                        minimum_brightness: lighting.minimum_brightness.clamp(0.0, 1.0),
                    },
                );
            }

            if let Some(slots) = &map_view.preview {
                let preview = view.preview.ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
                let uploaded = self
                    .previews
                    .get(index)
                    .and_then(Option::as_ref)
                    .ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
                let plan = preview_plan(view.camera, preview)?;
                recorded.program.set(slots.sprites, uploaded.sprites);
                recorded.program.set_bytes(slots.camera, &plan.camera);
                recorded.program.set(slots.cull.visible, uploaded.cull.visible);
                recorded.program.set(slots.cull.chunks, uploaded.cull.chunks);
                recorded.program.set(slots.cull.commands, uploaded.cull.commands);
                bind_sprite_cull(&mut recorded.program, &slots.cull, &plan, &uploaded.cull);
            }

            // a focused area shows its own outlines even with outlines turned off
            recorded.program.set(
                map_view.area_outlines,
                frame.show_area_outlines || view.focused_area.is_some(),
            );

            let Some(slots) = map_view.interaction.as_ref() else {
                continue;
            };
            let interaction = view.interaction;
            let local_cursor = interaction
                .cursor
                .filter(|cursor| cursor[0] < view.rect.width && cursor[1] < view.rect.height);
            let guides = prepared_guides
                .get(index)
                .ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
            let push = InteractionPush {
                cursor: local_cursor.unwrap_or([0; 2]),
                cursor_valid: u32::from(local_cursor.is_some()),
                picked_owner: interaction.picked.map(owner_words).unwrap_or([0; 2]),
                stripe_offset,
                focused_area_owner: view.focused_area.map(owner_words).unwrap_or([0; 2]),
                connected_count: guides.connected_count,
            };
            recorded.program.set_bytes(slots.push, &push);
            recorded.program.set(slots.connected_owners, guides.connected_owners);
            recorded.program.set_bytes(
                slots.pick_push,
                &PickPush {
                    cursor: local_cursor.unwrap_or([0; 2]),
                    cursor_valid: u32::from(local_cursor.is_some()),
                },
            );
            let should_pick = map_view.should_pick.ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
            let picks = pick.is_some() && picking_map == Some(index) && local_cursor.is_some();
            recorded.program.set(should_pick, picks);

            let has_highlight = interaction.picked.is_some() || local_cursor.is_some() || guides.connected_count > 0;
            let has_guide_lines = guides.line_count > 0;
            recorded.program.set(slots.has_highlight, has_highlight);
            recorded
                .program
                .set(slots.needs_visibility, has_highlight || has_guide_lines || picks);

            if let Some(slots) = &map_view.guides {
                recorded.program.set(slots.has_lines, has_guide_lines);
                let count = guides.line_count;
                recorded.program.set(slots.lines, guides.lines);
                recorded.program.set_bytes(
                    slots.push,
                    &GuidePush {
                        viewport: [map_extent.width as f32, map_extent.height as f32],
                        picked_owner: interaction.picked.map(owner_words).unwrap_or([0; 2]),
                        stripe_offset,
                    },
                );
                recorded.program.set(
                    slots.draw,
                    PassCallback::new(move |cmd| {
                        if count > 0 {
                            cmd.set_viewport(0, Rect2D::framebuffer())
                                .set_scissor(0, Rect2D::framebuffer())
                                .draw(4, count);
                        }
                    }),
                );
            }

            recorded.program.set(
                slots.highlight_draw,
                PassCallback::new(move |cmd| {
                    cmd.set_viewport(0, Rect2D::framebuffer())
                        .set_scissor(0, Rect2D::framebuffer())
                        .push_constants(&push)
                        .draw(4, 1);
                }),
            );

            let _ = (
                map_view.scene_attachment,
                map_view.visibility_attachment,
                map_view.output_attachment,
            );
        }

        if let (Some((main, others)), Some(imgui)) = (ui_frames.as_ref(), self.imgui.as_ref()) {
            if let Some(slots) = recorded.ui.as_ref() {
                imgui.bind(&mut recorded.program, slots, main);
            }
            for (slots, ui_frame) in recorded.viewport_ui.iter().zip(others) {
                if let Some(slots) = slots {
                    imgui.bind(&mut recorded.program, slots, ui_frame);
                }
            }
        }

        let executed =
            match self
                .graph
                .execute(&self.device.context, &recorded.program, &mut AllocatorKind::Frame(next))
            {
                Ok(()) => true,
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR | vk::Result::SUBOPTIMAL_KHR) => {
                    // one program presents every window and cannot tell which one failed
                    self.stale = true;
                    self.viewports.iter_mut().for_each(ViewportTarget::mark_stale);

                    false
                },
                Err(error) => return Err(error.into()),
            };

        let (Some(index), Some(request), true) = (picking, pick, cursor.is_some()) else {
            return Ok(None);
        };
        if !executed {
            return Ok(None);
        }

        self.graph.wait()?;
        let bytes = self
            .pick_readback
            .as_mut()
            .and_then(Buffer::mapped_slice_mut)
            .and_then(|bytes| bytes.get(..4))
            .ok_or(vk::Result::ERROR_MEMORY_MAP_FAILED)?;
        let raw = u32::from_ne_bytes(bytes.try_into().map_err(|_| vk::Result::ERROR_MEMORY_MAP_FAILED)?);
        let base = self.uploaded.get(index).map_or(0, |uploaded| uploaded.base) as usize;
        let picked = VisibilityId::from_raw(raw)
            .map(VisibilityId::sprite_index)
            .and_then(|sprite| sprite.checked_sub(base))
            .and_then(|sprite| frame.map_views.get(index)?.sprite_instances.get(sprite))
            .map_or(PickResult::Miss, |sprite| PickResult::Hit(sprite.owner));

        Ok(Some((index, request, picked)))
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        GuideLine, HighlightStyle, MapViewFrame, MapViewRect,
        renderer::fixtures::{map_view, owner},
    };

    #[test]
    fn marching_stripes_animate_the_frame_unless_the_highlight_is_tinted() {
        let rect = MapViewRect {
            x: 0,
            y: 0,
            width: 640,
            height: 480,
        };
        assert!(!map_view(rect).is_animated());

        let mut hovered = map_view(rect);
        hovered.interaction.cursor = Some([10, 10]);
        assert!(hovered.is_animated());

        let mut outside = map_view(rect);
        outside.interaction.cursor = Some([640, 10]);
        assert!(!outside.is_animated());

        let mut picked = map_view(rect);
        picked.interaction.picked = Some(owner());
        assert!(picked.is_animated());

        picked.interaction.highlight = HighlightStyle::Tint;
        assert!(!picked.is_animated());

        let guides = [GuideLine {
            origin: [0.0; 2],
            target: [32.0, 0.0],
        }];
        let guided = MapViewFrame {
            guide_lines: &guides,
            interaction: picked.interaction,
            ..map_view(rect)
        };
        assert!(guided.is_animated(), "guide lines march in either highlight style");
    }
}
