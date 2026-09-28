use std::{any::Any, ptr::NonNull, sync::Arc};

use ash::vk;
use dear_imgui_rs::render::DrawData;
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};
use vir::SwapChain;

use crate::{Device, GpuError};

pub struct SecondaryViewport {
    id: u32,
    window: RawWindowHandle,
    display: RawDisplayHandle,
    size: [u32; 2],
    draw_data: NonNull<DrawData>,
    keepalive: Arc<dyn Any>,
}

impl SecondaryViewport {
    /// # Safety
    ///
    /// `draw_data` must stay valid until the `Renderer::draw_imgui` call that receives this
    /// viewport returns. `keepalive` must own the native window behind `window`, so the surface
    /// built on it never outlives it.
    pub unsafe fn new(
        id: u32, window: RawWindowHandle, display: RawDisplayHandle, size: [u32; 2], draw_data: NonNull<DrawData>,
        keepalive: Arc<dyn Any>,
    ) -> Self {
        Self {
            id,
            window,
            display,
            size,
            draw_data,
            keepalive,
        }
    }

    pub const fn id(&self) -> u32 { self.id }

    pub(crate) fn draw_data(&self) -> &DrawData { unsafe { self.draw_data.as_ref() } }
}

pub(crate) struct ViewportTarget {
    pub(crate) id: u32,
    pub(crate) swapchain: SwapChain,
    pub(crate) extent: vk::Extent2D,
    surface: vk::SurfaceKHR,
    requested: [u32; 2],
    stale: bool,
    // dropped after the surface, see `destroy`
    keepalive: Arc<dyn Any>,
}

impl ViewportTarget {
    fn create(device: &mut Device, viewport: &SecondaryViewport) -> Result<Self, GpuError> {
        let surface = device.create_surface(viewport.window, viewport.display)?;
        let [width, height] = viewport.size;
        let (swapchain, extent, _) = match device.create_swapchain(surface, width, height, None) {
            Ok(created) => created,
            Err(error) => {
                device.destroy_surface(surface);

                return Err(error);
            },
        };

        Ok(Self {
            id: viewport.id,
            swapchain,
            extent,
            surface,
            requested: viewport.size,
            stale: false,
            keepalive: Arc::clone(&viewport.keepalive),
        })
    }

    fn recreate(&mut self, device: &mut Device, size: [u32; 2]) -> Result<(), GpuError> {
        let (swapchain, extent, _) = device.create_swapchain(self.surface, size[0], size[1], Some(&self.swapchain))?;
        let old = std::mem::replace(&mut self.swapchain, swapchain);
        device.destroy_swapchain(old);
        self.extent = extent;
        self.requested = size;
        self.stale = false;

        Ok(())
    }

    pub(crate) fn mark_stale(&mut self) { self.stale = true; }

    /// The caller waits for the device to be idle first.
    pub(crate) fn destroy(self, device: &mut Device) {
        device.destroy_swapchain(self.swapchain);
        device.destroy_surface(self.surface);
        drop(self.keepalive);
    }
}

/// Brings `targets` in line with this frame's viewports, in the same order. Returns whether any
/// swapchain changed, which invalidates a recording that acquires from them.
///
/// A window whose target cannot be built goes into `failed` and is left undrawn, without a retry,
/// until its size changes.
pub(crate) fn sync_targets(
    device: &mut Device, targets: &mut Vec<ViewportTarget>, failed: &mut Vec<(u32, [u32; 2])>,
    viewports: &[SecondaryViewport],
) -> Result<bool, GpuError> {
    failed.retain(|&(id, size)| {
        viewports
            .iter()
            .any(|viewport| viewport.id == id && viewport.size == size)
    });
    let live = viewports
        .iter()
        .filter(|viewport| viewport.size[0] > 0 && viewport.size[1] > 0)
        .filter(|viewport| !failed.contains(&(viewport.id, viewport.size)))
        .collect::<Vec<_>>();
    let changed = targets.len() != live.len()
        || targets
            .iter()
            .zip(&live)
            .any(|(target, viewport)| target.id != viewport.id || target.stale || target.requested != viewport.size);
    if !changed {
        return Ok(false);
    }

    device.wait_idle()?;

    let mut previous = std::mem::take(targets);
    for viewport in live {
        let built = match previous.iter().position(|target| target.id == viewport.id) {
            Some(index) => {
                let mut target = previous.swap_remove(index);
                let recreated = match target.stale || target.requested != viewport.size {
                    true => target.recreate(device, viewport.size),
                    false => Ok(()),
                };
                match recreated {
                    Ok(()) => Ok(target),
                    Err(error) => {
                        target.destroy(device);
                        Err(error)
                    },
                }
            },
            None => ViewportTarget::create(device, viewport),
        };
        match built {
            Ok(target) => targets.push(target),
            Err(error) => {
                log::error!("imgui window {:#x} is not drawn: {error}", viewport.id);
                failed.push((viewport.id, viewport.size));
            },
        }
    }

    for target in previous {
        target.destroy(device);
    }

    Ok(true)
}
