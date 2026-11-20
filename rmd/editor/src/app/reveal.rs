use winit::{
    error::OsError,
    event_loop::ActiveEventLoop,
    window::{Window, WindowAttributes},
};

#[cfg(target_os = "windows")]
pub(super) fn create_window(
    event_loop: &ActiveEventLoop, attributes: WindowAttributes, maximized: bool,
) -> Result<Window, OsError> {
    let window = event_loop.create_window(attributes.with_visible(false))?;
    set_cloaked(&window, true);
    window.set_visible(true);
    if maximized {
        window.set_maximized(true);
    }

    Ok(window)
}

#[cfg(not(target_os = "windows"))]
pub(super) fn create_window(
    event_loop: &ActiveEventLoop, attributes: WindowAttributes, maximized: bool,
) -> Result<Window, OsError> {
    event_loop.create_window(attributes.with_maximized(maximized))
}

#[cfg(target_os = "windows")]
pub(super) fn reveal(window: &Window) {
    set_cloaked(window, false);
}

#[cfg(not(target_os = "windows"))]
pub(super) fn reveal(_: &Window) {}

#[cfg(target_os = "windows")]
fn set_cloaked(window: &Window, cloaked: bool) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_CLOAK, DwmSetWindowAttribute};

    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };

    let value = i32::from(cloaked);
    let result = unsafe {
        DwmSetWindowAttribute(
            handle.hwnd.get() as _,
            DWMWA_CLOAK as u32,
            (&raw const value).cast(),
            size_of::<i32>() as u32,
        )
    };
    if result < 0 {
        log::warn!(
            "could not {} the window: {result:#x}",
            if cloaked { "cloak" } else { "uncloak" }
        );
    }
}
