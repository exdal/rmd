use std::{
    ffi::CStr,
    ops::{Deref, DerefMut},
    path::PathBuf,
};

use dear_imgui_rs::{BackendFlags, Context, sys};

pub static CONTEXT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn rectangle_context() -> dear_imgui_rs::Context {
    let mut context = dear_imgui_rs::Context::create();
    context.set_ini_filename(None::<PathBuf>).unwrap();
    context.font_atlas().try_claim_legacy_renderer().unwrap().build();
    context.io_mut().set_display_size([800.0, 600.0]);
    context.io_mut().set_delta_time(1.0 / 60.0);
    context.io_mut().set_config_input_trickle_event_queue(false);
    context.io_mut().set_config_macosx_behaviors(false);
    context
}

pub struct PopupContext(Context);

impl Deref for PopupContext {
    type Target = Context;

    fn deref(&self) -> &Context {
        &self.0
    }
}

impl DerefMut for PopupContext {
    fn deref_mut(&mut self) -> &mut Context {
        &mut self.0
    }
}

impl Drop for PopupContext {
    fn drop(&mut self) {
        self.0.destroy_platform_windows().unwrap();
    }
}

/// Optionally installs a headless backend for ImGui's desktop-coordinate viewport behavior.
pub fn popup_context(desktop: bool) -> PopupContext {
    let mut context = rectangle_context();
    if !desktop {
        return PopupContext(context);
    }

    unsafe extern "C" fn create(viewport: *mut sys::ImGuiViewport) {
        unsafe { (*viewport).PlatformHandle = std::ptr::dangling_mut::<u8>().cast() };
    }
    unsafe extern "C" fn destroy(viewport: *mut sys::ImGuiViewport) {
        unsafe { (*viewport).PlatformHandle = std::ptr::null_mut() };
    }
    unsafe extern "C" fn noop(_: *mut sys::ImGuiViewport) {}
    unsafe extern "C" fn set_vec2(_: *mut sys::ImGuiViewport, _: *const sys::ImVec2) {}
    unsafe extern "C" fn set_title(_: *mut sys::ImGuiViewport, _: *const std::ffi::c_char) {}
    unsafe extern "C" fn get_pos(viewport: *mut sys::ImGuiViewport, output: *mut sys::ImVec2) {
        unsafe { *output = (*viewport).Pos };
    }
    unsafe extern "C" fn get_size(viewport: *mut sys::ImGuiViewport, output: *mut sys::ImVec2) {
        unsafe { *output = (*viewport).Size };
    }

    let flags =
        context.io().backend_flags() | BackendFlags::PLATFORM_HAS_VIEWPORTS | BackendFlags::RENDERER_HAS_VIEWPORTS;
    context.io_mut().set_backend_flags(flags);
    // SAFETY: the callbacks only access live ImGui viewports. The test owns the context and
    // uses inert handles without allocating native windows or renderer resources.
    unsafe {
        let platform = context.platform_io_mut();
        platform.set_platform_create_window_raw(Some(create));
        platform.set_platform_destroy_window_raw(Some(destroy));
        platform.set_platform_show_window_raw(Some(noop));
        platform.set_platform_set_window_pos_raw(Some(set_vec2));
        platform.set_platform_get_window_pos_raw(Some(get_pos));
        platform.set_platform_set_window_size_raw(Some(set_vec2));
        platform.set_platform_get_window_size_raw(Some(get_size));
        platform.set_platform_set_window_title_raw(Some(set_title));
        platform.set_monitors(&[sys::ImGuiPlatformMonitor {
            MainPos: sys::ImVec2 { x: 0.0, y: 0.0 },
            MainSize: sys::ImVec2 { x: 3840.0, y: 2160.0 },
            WorkPos: sys::ImVec2 { x: 0.0, y: 0.0 },
            WorkSize: sys::ImVec2 { x: 3840.0, y: 2160.0 },
            DpiScale: 1.0,
            PlatformHandle: std::ptr::null_mut(),
        }]);
        create(context.main_viewport().as_raw().cast_mut());
    }
    context.enable_multi_viewport();
    PopupContext(context)
}

pub fn set_desktop_geometry(context: &mut Context, position: [f32; 2], size: [f32; 2]) {
    context.io_mut().set_display_size(size);
    // SAFETY: the test owns the context; its headless backend reads this simulated native position.
    unsafe {
        (*context.main_viewport().as_raw().cast_mut()).Pos = sys::ImVec2 {
            x: position[0],
            y: position[1],
        };
    }
}

pub fn finish_frame(context: &mut Context) {
    assert!(context.render_legacy().valid());
    context.update_platform_windows();
}

pub fn assert_window_centered(context: &mut Context, name: &CStr) {
    let center = context.main_viewport().work_center();
    // SAFETY: the test owns the context and inspects its live, rendered window.
    unsafe {
        let window = sys::igFindWindowByName(name.as_ptr());
        assert!(!window.is_null());
        let window = &*window;
        assert!(window.Active && !window.Hidden, "the popup should be visible");
        assert_eq!(
            window.Viewport.cast_const().cast::<sys::ImGuiViewport>(),
            context.main_viewport().as_raw()
        );
        for (axis, actual) in [window.Pos.x + window.Size.x / 2.0, window.Pos.y + window.Size.y / 2.0]
            .into_iter()
            .enumerate()
        {
            assert!(
                (actual - center[axis]).abs() <= 1.0,
                "popup center {actual}, viewport center {}",
                center[axis]
            );
        }
    }
}
