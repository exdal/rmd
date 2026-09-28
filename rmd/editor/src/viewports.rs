use std::{cell::RefCell, ffi::c_void, ptr::NonNull, sync::Arc};

use dear_imgui_rs::{BackendFlags, Context, Style, render::ReconciledFrame, sys};
use dear_imgui_winit::WinitPlatform;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use render::SecondaryViewport;
use winit::{event_loop::ActiveEventLoop, window::Window};

struct Captured {
    id: u32,
    draw_data: NonNull<sys::ImDrawData>,
    window: *const Window,
}

thread_local! {
    static CAPTURED: RefCell<Vec<Captured>> = const { RefCell::new(Vec::new()) };
}

unsafe extern "C" fn capture_window(viewport: *mut sys::ImGuiViewport, _: *mut c_void) {
    let Some(viewport) = (unsafe { viewport.as_ref() }) else {
        return;
    };
    let (Some(draw_data), false) = (NonNull::new(viewport.DrawData), viewport.PlatformHandle.is_null()) else {
        return;
    };

    CAPTURED.with_borrow_mut(|captured| {
        captured.push(Captured {
            id: viewport.ID,
            draw_data,
            window: viewport.PlatformHandle.cast_const().cast(),
        })
    });
}

pub fn install(context: &mut Context) {
    unsafe {
        context
            .platform_io_mut()
            .set_renderer_render_window_raw(Some(capture_window))
    };
    let flags = context.io().backend_flags() | BackendFlags::RENDERER_HAS_VIEWPORTS;
    context.io_mut().set_backend_flags(flags);
}

pub fn uninstall(context: &mut Context) {
    unsafe { context.platform_io_mut().set_renderer_render_window_raw(None) };
    let flags = context.io().backend_flags() - BackendFlags::RENDERER_HAS_VIEWPORTS;
    context.io_mut().set_backend_flags(flags);
}

pub struct ScaledStyle {
    base: Style,
    physical_pixels: bool,
    scale: f32,
    font_scale: f32,
}

impl ScaledStyle {
    /// `physical_pixels` is whether ImGui coordinates are framebuffer pixels, as with multiple
    /// viewports outside macOS. Otherwise the framebuffer scale already applies the OS scale.
    pub fn new(context: &mut Context, physical_pixels: bool) -> Self {
        if physical_pixels {
            context.io_mut().set_config_dpi_scale_fonts(true);
            context.io_mut().set_config_dpi_scale_viewports(true);
        }

        Self {
            base: context.style().clone(),
            physical_pixels,
            scale: 1.0,
            font_scale: 1.0,
        }
    }

    pub fn apply(&mut self, context: &mut Context, dpi_scale: f32, override_percent: Option<u32>) {
        if !dpi_scale.is_finite() || dpi_scale <= 0.0 {
            return;
        }
        let absolute_scale = override_percent.map_or(dpi_scale, |percent| percent as f32 / 100.0);
        let font_scale = absolute_scale / dpi_scale;
        let scale = if self.physical_pixels {
            absolute_scale
        } else {
            font_scale
        };
        if (scale == self.scale && font_scale == self.font_scale) || !scale.is_finite() || scale <= 0.0 {
            return;
        }

        *context.style_mut() = self.base.clone();
        context.style_mut().scale_all_sizes(scale);
        keep_thin_lines(context.style_mut(), &self.base);
        context
            .style_mut()
            .set_font_scale_main(self.base.font_scale_main() * font_scale);
        self.scale = scale;
        self.font_scale = font_scale;
    }
}

// `scale_all_sizes` truncates, so a 1px line scaled below 1.0 disappears
fn keep_thin_lines(style: &mut Style, base: &Style) {
    let keep = |scaled: f32, base: f32| scaled.max(base.min(1.0));
    style.set_window_border_size(keep(style.window_border_size(), base.window_border_size()));
    style.set_child_border_size(keep(style.child_border_size(), base.child_border_size()));
    style.set_popup_border_size(keep(style.popup_border_size(), base.popup_border_size()));
    style.set_frame_border_size(keep(style.frame_border_size(), base.frame_border_size()));
    style.set_image_border_size(keep(style.image_border_size(), base.image_border_size()));
    style.set_tab_border_size(keep(style.tab_border_size(), base.tab_border_size()));
    style.set_tab_bar_border_size(keep(style.tab_bar_border_size(), base.tab_bar_border_size()));
    style.set_drag_drop_target_border_size(keep(
        style.drag_drop_target_border_size(),
        base.drag_drop_target_border_size(),
    ));
    style.set_separator_text_border_size(keep(
        style.separator_text_border_size(),
        base.separator_text_border_size(),
    ));
    style.set_separator_size(keep(style.separator_size(), base.separator_size()));
    style.set_input_text_cursor_size(keep(style.input_text_cursor_size(), base.input_text_cursor_size()));
    style.set_tree_lines_size(keep(style.tree_lines_size(), base.tree_lines_size()));
    style.set_tab_bar_overline_size(keep(style.tab_bar_overline_size(), base.tab_bar_overline_size()));
    style.set_mouse_cursor_scale(keep(style.mouse_cursor_scale(), base.mouse_cursor_scale()));
}

/// Must run inside the platform's event loop scope, which lets it create and destroy windows.
pub fn collect(frame: &mut ReconciledFrame<'_>) -> Vec<SecondaryViewport> {
    CAPTURED.with_borrow_mut(Vec::clear);
    frame.update_and_render_platform_windows_default();

    CAPTURED
        .with_borrow_mut(std::mem::take)
        .into_iter()
        .filter_map(|captured| {
            // dear-imgui-winit publishes `Arc::as_ptr` of the window it owns
            let window = unsafe {
                Arc::increment_strong_count(captured.window);
                Arc::from_raw(captured.window)
            };
            let handle = window.window_handle().ok()?.as_raw();
            let display = window.display_handle().ok()?.as_raw();
            let size = window.inner_size();

            // the draw data lives until the next ImGui frame, past the draw that consumes it
            Some(unsafe {
                SecondaryViewport::new(
                    captured.id,
                    handle,
                    display,
                    [size.width, size.height],
                    captured.draw_data.cast(),
                    window,
                )
            })
        })
        .collect()
}

// the next frame panics when a redraw bails out before `collect` updated the platform windows
pub fn finish_frame(platform: &WinitPlatform, context: &mut Context, event_loop: &ActiveEventLoop) {
    let raw = unsafe { &*context.as_raw() };
    let pending =
        raw.FrameCount > 0 && raw.FrameCountEnded == raw.FrameCount && raw.FrameCountPlatformEnded != raw.FrameCount;
    if !platform.viewports_enabled() || !pending {
        return;
    }

    let (updated, faults) = platform
        .with_event_loop(event_loop, |_| context.update_platform_windows())
        .into_parts();
    for fault in faults {
        log::error!("{fault}");
    }
    if updated.is_none() {
        log::error!("imgui platform windows were not updated this frame");
    }
}

#[cfg(test)]
mod tests {
    use dear_imgui_rs::Context;

    use super::ScaledStyle;

    #[test]
    fn absolute_ui_scale_replaces_os_scale_without_compounding() {
        let _context = crate::ui::IMGUI_CONTEXT.lock().unwrap();
        let mut context = Context::create();
        let base_padding = context.style().window_padding();
        let base_font_scale = context.style().font_scale_main();
        let mut scaled = ScaledStyle::new(&mut context, true);

        scaled.apply(&mut context, 1.5, None);
        assert_eq!(context.style().font_scale_main(), base_font_scale);
        assert_eq!(context.style().window_padding()[0], (base_padding[0] * 1.5).floor());

        scaled.apply(&mut context, 1.5, Some(100));
        assert_eq!(context.style().font_scale_main(), base_font_scale / 1.5);
        assert_eq!(context.style().window_padding()[0], base_padding[0]);

        scaled.apply(&mut context, 1.5, Some(200));
        assert_eq!(context.style().font_scale_main(), base_font_scale * (2.0 / 1.5));
        assert_eq!(context.style().window_padding()[0], (base_padding[0] * 2.0).floor());
    }

    #[test]
    fn single_window_mode_keeps_os_scale_as_the_default() {
        let _context = crate::ui::IMGUI_CONTEXT.lock().unwrap();
        let mut context = Context::create();
        let base_padding = context.style().window_padding();
        let mut scaled = ScaledStyle::new(&mut context, false);

        scaled.apply(&mut context, 1.5, None);
        assert_eq!(context.style().window_padding()[0], base_padding[0]);
        assert_eq!(context.style().font_scale_main(), 1.0);

        scaled.apply(&mut context, 1.5, Some(200));
        assert_eq!(
            context.style().window_padding()[0],
            (base_padding[0] * (2.0 / 1.5)).floor()
        );
        assert_eq!(context.style().font_scale_main(), 2.0 / 1.5);
    }

    #[test]
    fn thin_lines_survive_a_scale_below_one() {
        let _context = crate::ui::IMGUI_CONTEXT.lock().unwrap();
        let mut context = Context::create();
        let base = context.style().clone();
        let mut scaled = ScaledStyle::new(&mut context, false);

        scaled.apply(&mut context, 1.5, Some(50));
        let style = context.style();
        assert_eq!(style.child_border_size(), base.child_border_size().min(1.0));
        assert_eq!(style.frame_border_size(), base.frame_border_size().min(1.0));
        assert_eq!(style.separator_size(), base.separator_size().min(1.0));
        assert_eq!(style.input_text_cursor_size(), base.input_text_cursor_size().min(1.0));
        assert_eq!(style.tree_lines_size(), base.tree_lines_size().min(1.0));
    }
}
