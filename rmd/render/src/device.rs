// copy pasted from vir-examples device builder

use std::ffi::CStr;

use ash::{ext, khr, vk};
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};
use vir::{Context, DomainFlag, Image, ImageAttachment, PersistentAllocator, SwapChain, allocator::Allocator};

use crate::error::GpuError;

pub struct Device {
    pub allocator: PersistentAllocator,
    pub context: Context,
    pub surface: vk::SurfaceKHR,
    pub physical_device: vk::PhysicalDevice,
    pub max_bindless_textures: u32,
    pub max_compute_work_group_count_x: u32,
    pub max_image_dimension_2d: u32,
    graphics_family: u32,
    entry: ash::Entry,
}

impl Device {
    pub fn new(window: RawWindowHandle, display: RawDisplayHandle) -> Result<Self, GpuError> {
        let entry = load_entry()?;
        let instance = create_instance(&entry, window)?;
        let (
            physical_device,
            queue_families,
            max_bindless_textures,
            max_compute_work_group_count_x,
            max_image_dimension_2d,
        ) = select_physical_device(&instance)?;
        let device = create_device(&instance, physical_device)?;

        let mut context = Context::new(device, physical_device, instance, &entry)?;
        let graphics = first_queue(&queue_families, vk::QueueFlags::GRAPHICS | vk::QueueFlags::COMPUTE)
            .ok_or(GpuError::NoGraphicsQueue)?;

        context.create_command_queue(graphics, DomainFlag::Graphics);

        let allocator = context.create_persistent_allocator();
        let surface = create_surface(&entry, context.instance(), window, display)?;

        Ok(Self {
            entry,
            context,
            surface,
            physical_device,
            allocator,
            max_bindless_textures,
            max_compute_work_group_count_x,
            max_image_dimension_2d,
            graphics_family: graphics,
        })
    }

    pub fn create_surface(
        &self, window: RawWindowHandle, display: RawDisplayHandle,
    ) -> Result<vk::SurfaceKHR, GpuError> {
        let surface = create_surface(&self.entry, self.context.instance(), window, display)?;
        let supported = unsafe {
            self.context.surface_loader().get_physical_device_surface_support(
                self.physical_device,
                self.graphics_family,
                surface,
            )
        };
        if !matches!(supported, Ok(true)) {
            self.destroy_surface(surface);

            return Err(GpuError::UnsupportedWindow);
        }

        Ok(surface)
    }

    pub fn destroy_surface(&self, surface: vk::SurfaceKHR) {
        unsafe { self.context.surface_loader().destroy_surface(surface, None) };
    }

    /// The caller waits for the device to be idle first.
    pub fn destroy_swapchain(&mut self, swapchain: SwapChain) {
        for attachment in &swapchain.attachments {
            self.allocator.deallocate_image_view(attachment.image_view());
        }
        for semaphore in &swapchain.semaphores {
            self.allocator.deallocate_semaphore(*semaphore);
        }

        unsafe {
            self.context
                .swapchain_loader()
                .destroy_swapchain(swapchain.handle, None)
        };
    }

    pub fn create_swapchain(
        &mut self, surface: vk::SurfaceKHR, width: u32, height: u32, old: Option<&SwapChain>,
    ) -> Result<(SwapChain, vk::Extent2D, vk::Format), GpuError> {
        let old_handle = old.map_or(vk::SwapchainKHR::null(), |s| s.handle);
        let (handle, format, extent) = build_swapchain(
            self.context.surface_loader(),
            self.context.swapchain_loader(),
            self.physical_device,
            surface,
            width,
            height,
            old_handle,
        )?;

        let images = unsafe { self.context.swapchain_loader().get_swapchain_images(handle) }?;
        let attachments = images
            .into_iter()
            .map(|image| {
                let extent = vk::Extent3D::default()
                    .width(extent.width)
                    .height(extent.height)
                    .depth(1);
                let subresource_range = vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .layer_count(1)
                    .level_count(1);

                ImageAttachment::new(
                    Image::imported(image, format, extent, vk::SampleCountFlags::TYPE_1),
                    format,
                    extent,
                    vk::SampleCountFlags::TYPE_1,
                    vk::ImageLayout::UNDEFINED,
                )
                .with_subresource_range(subresource_range)
            })
            .collect::<Vec<_>>();

        let swapchain = SwapChain::new(&mut self.allocator, handle, surface, attachments)?;

        Ok((swapchain, extent, format))
    }

    pub fn wait_idle(&self) -> Result<(), GpuError> {
        unsafe { self.context.device().device_wait_idle() }?;

        Ok(())
    }
}

fn load_entry() -> Result<ash::Entry, GpuError> { unsafe { ash::Entry::load() }.or_else(load_fallback) }

#[cfg(not(target_os = "macos"))]
fn load_fallback(error: ash::LoadingError) -> Result<ash::Entry, GpuError> { Err(GpuError::Loader(error.to_string())) }

#[cfg(target_os = "macos")]
fn load_fallback(error: ash::LoadingError) -> Result<ash::Entry, GpuError> {
    use std::{
        env,
        path::{Path, PathBuf},
    };

    // dyld only searches DYLD_LIBRARY_PATH and /usr/lib for a bare library name.
    let sdk = env::var_os("VULKAN_SDK").map(|sdk| Path::new(&sdk).join("lib/libvulkan.dylib"));
    let paths = sdk
        .into_iter()
        .chain(["/usr/local/lib/libvulkan.dylib", "/opt/homebrew/lib/libvulkan.dylib"].map(PathBuf::from));
    for path in paths {
        if let Ok(entry) = unsafe { ash::Entry::load_from(&path) } {
            return Ok(entry);
        }
    }

    Err(GpuError::Loader(format!(
        "{error}; install the Vulkan SDK with KosmicKrisp and source its setup-env.sh"
    )))
}

fn create_instance(entry: &ash::Entry, window: RawWindowHandle) -> Result<ash::Instance, GpuError> {
    let mut extensions = vec![
        khr::surface::NAME,
        khr::get_surface_capabilities2::NAME,
        khr::get_physical_device_properties2::NAME,
        surface_extension(window)?,
    ];
    let available = unsafe { entry.enumerate_instance_extension_properties(None) }?;
    if has_extension(&available, ext::debug_utils::NAME) {
        extensions.push(ext::debug_utils::NAME);
    }
    let pointers = extensions.iter().map(|name| name.as_ptr()).collect::<Vec<_>>();

    let app_info = vk::ApplicationInfo::default()
        .application_name(c"rmd")
        .application_version(vk::make_api_version(0, 0, 0, 0))
        .engine_name(c"vir")
        .engine_version(vk::make_api_version(0, 0, 0, 0))
        .api_version(vk::make_api_version(0, 1, 3, 0));

    let create_info = vk::InstanceCreateInfo::default()
        .application_info(&app_info)
        .enabled_extension_names(&pointers);

    Ok(unsafe { entry.create_instance(&create_info, None) }?)
}

fn select_physical_device(
    instance: &ash::Instance,
) -> Result<(vk::PhysicalDevice, Vec<vk::QueueFamilyProperties>, u32, u32, u32), GpuError> {
    let minimum = vk::make_api_version(0, 1, 3, 0);
    let devices = unsafe { instance.enumerate_physical_devices() }?;

    let mut candidates = devices
        .into_iter()
        .filter_map(|handle| {
            let properties = unsafe { instance.get_physical_device_properties(handle) };
            if properties.api_version < minimum {
                return None;
            }

            let extensions = unsafe { instance.enumerate_device_extension_properties(handle) }.ok()?;
            if !has_extension(&extensions, khr::swapchain::NAME) {
                return None;
            }

            let mut vk12 = vk::PhysicalDeviceVulkan12Features::default();
            let mut features = vk::PhysicalDeviceFeatures2::default().push_next(&mut vk12);
            unsafe { instance.get_physical_device_features2(handle, &mut features) };
            if features.features.independent_blend == vk::FALSE
                || vk12.runtime_descriptor_array == vk::FALSE
                || vk12.shader_sampled_image_array_non_uniform_indexing == vk::FALSE
            {
                return None;
            }

            let mut subgroup = vk::PhysicalDeviceSubgroupProperties::default();
            let mut properties2 = vk::PhysicalDeviceProperties2::default().push_next(&mut subgroup);
            unsafe { instance.get_physical_device_properties2(handle, &mut properties2) };
            if !subgroup.supported_stages.contains(vk::ShaderStageFlags::COMPUTE)
                || !subgroup
                    .supported_operations
                    .contains(vk::SubgroupFeatureFlags::BASIC | vk::SubgroupFeatureFlags::BALLOT)
            {
                return None;
            }

            let families = unsafe { instance.get_physical_device_queue_family_properties(handle) };
            first_queue(&families, vk::QueueFlags::GRAPHICS | vk::QueueFlags::COMPUTE)?;

            let limits = properties.limits;
            let max_bindless_textures = limits
                .max_per_stage_descriptor_sampled_images
                .min(limits.max_descriptor_set_sampled_images)
                .min(limits.max_per_stage_descriptor_samplers)
                .min(limits.max_descriptor_set_samplers)
                .min(limits.max_per_stage_resources.saturating_sub(1));
            if max_bindless_textures == 0 {
                return None;
            }

            Some((
                handle,
                properties.device_type,
                families,
                max_bindless_textures,
                limits.max_compute_work_group_count[0],
                limits.max_image_dimension2_d,
            ))
        })
        .collect::<Vec<_>>();

    candidates.sort_by_key(|(_, device_type, ..)| {
        (
            *device_type != vk::PhysicalDeviceType::DISCRETE_GPU,
            device_type.as_raw(),
        )
    });

    let (handle, _, families, max_bindless_textures, max_compute_work_group_count_x, max_image_dimension_2d) =
        candidates.into_iter().next().ok_or(GpuError::NoSuitableDevice)?;

    Ok((
        handle,
        families,
        max_bindless_textures,
        max_compute_work_group_count_x,
        max_image_dimension_2d,
    ))
}

fn create_device(instance: &ash::Instance, physical_device: vk::PhysicalDevice) -> Result<ash::Device, GpuError> {
    let families = unsafe { instance.get_physical_device_queue_family_properties(physical_device) };
    let priorities = [1.0_f32];
    let queues = (0..families.len() as u32)
        .map(|index| {
            vk::DeviceQueueCreateInfo::default()
                .queue_family_index(index)
                .queue_priorities(&priorities)
        })
        .collect::<Vec<_>>();

    let mut extensions = vec![khr::swapchain::NAME.as_ptr()];
    let available = unsafe { instance.enumerate_device_extension_properties(physical_device) }?;
    if has_extension(&available, khr::push_descriptor::NAME) {
        extensions.push(khr::push_descriptor::NAME.as_ptr());
    }

    let mut vk13 = vk::PhysicalDeviceVulkan13Features::default()
        .synchronization2(true)
        .dynamic_rendering(true)
        .shader_demote_to_helper_invocation(true);
    let mut vk12 = vk::PhysicalDeviceVulkan12Features::default()
        .timeline_semaphore(true)
        .buffer_device_address(true)
        .scalar_block_layout(true)
        .runtime_descriptor_array(true)
        .shader_sampled_image_array_non_uniform_indexing(true);
    let mut vk11 = vk::PhysicalDeviceVulkan11Features::default()
        .variable_pointers(true)
        .variable_pointers_storage_buffer(true)
        .shader_draw_parameters(true);
    let mut features = vk::PhysicalDeviceFeatures2::default()
        .features(
            vk::PhysicalDeviceFeatures::default()
                .fill_mode_non_solid(true)
                .independent_blend(true),
        )
        .push_next(&mut vk11)
        .push_next(&mut vk12)
        .push_next(&mut vk13);

    let create_info = vk::DeviceCreateInfo::default()
        .queue_create_infos(&queues)
        .enabled_extension_names(&extensions)
        .push_next(&mut features);

    Ok(unsafe { instance.create_device(physical_device, &create_info, None) }?)
}

fn has_extension(extensions: &[vk::ExtensionProperties], name: &CStr) -> bool {
    extensions
        .iter()
        .filter_map(|extension| extension.extension_name_as_c_str().ok())
        .any(|extension| extension == name)
}

fn first_queue(families: &[vk::QueueFamilyProperties], flags: vk::QueueFlags) -> Option<u32> {
    families
        .iter()
        .position(|family| family.queue_flags.contains(flags))
        .map(|index| index as u32)
}

fn surface_extension(handle: RawWindowHandle) -> Result<&'static CStr, GpuError> {
    match handle {
        RawWindowHandle::Win32(_) => Ok(khr::win32_surface::NAME),
        RawWindowHandle::Wayland(_) => Ok(khr::wayland_surface::NAME),
        RawWindowHandle::Xlib(_) => Ok(khr::xlib_surface::NAME),
        RawWindowHandle::Xcb(_) => Ok(khr::xcb_surface::NAME),
        RawWindowHandle::AppKit(_) => Ok(ext::metal_surface::NAME),
        _ => Err(GpuError::UnsupportedWindow),
    }
}

fn create_surface(
    entry: &ash::Entry, instance: &ash::Instance, window: RawWindowHandle, display: RawDisplayHandle,
) -> Result<vk::SurfaceKHR, GpuError> {
    let surface = unsafe {
        match (window, display) {
            (RawWindowHandle::Win32(window), _) => khr::win32_surface::Instance::new(entry, instance)
                .create_win32_surface(
                    &vk::Win32SurfaceCreateInfoKHR::default()
                        .hinstance(window.hinstance.map_or(0, |h| h.get()))
                        .hwnd(window.hwnd.get()),
                    None,
                )?,

            (RawWindowHandle::Wayland(window), RawDisplayHandle::Wayland(display)) => {
                khr::wayland_surface::Instance::new(entry, instance).create_wayland_surface(
                    &vk::WaylandSurfaceCreateInfoKHR::default()
                        .display(display.display.as_ptr())
                        .surface(window.surface.as_ptr()),
                    None,
                )?
            },

            (RawWindowHandle::Xlib(window), RawDisplayHandle::Xlib(display)) => {
                khr::xlib_surface::Instance::new(entry, instance).create_xlib_surface(
                    &vk::XlibSurfaceCreateInfoKHR::default()
                        .dpy(display.display.map_or(std::ptr::null_mut(), |d| d.as_ptr()))
                        .window(window.window),
                    None,
                )?
            },

            (RawWindowHandle::Xcb(window), RawDisplayHandle::Xcb(display)) => {
                khr::xcb_surface::Instance::new(entry, instance).create_xcb_surface(
                    &vk::XcbSurfaceCreateInfoKHR::default()
                        .connection(display.connection.map_or(std::ptr::null_mut(), |c| c.as_ptr()))
                        .window(window.window.get()),
                    None,
                )?
            },

            #[cfg(target_os = "macos")]
            (RawWindowHandle::AppKit(window), _) => {
                let layer = raw_window_metal::Layer::from_ns_view(window.ns_view);
                ext::metal_surface::Instance::new(entry, instance).create_metal_surface(
                    &vk::MetalSurfaceCreateInfoEXT::default().layer(layer.as_ptr().as_ptr()),
                    None,
                )?
            },

            _ => return Err(GpuError::UnsupportedWindow),
        }
    };

    Ok(surface)
}

#[allow(clippy::too_many_arguments)]
fn build_swapchain(
    surface_loader: &khr::surface::Instance, swapchain_loader: &khr::swapchain::Device,
    physical_device: vk::PhysicalDevice, surface: vk::SurfaceKHR, width: u32, height: u32, old: vk::SwapchainKHR,
) -> Result<(vk::SwapchainKHR, vk::Format, vk::Extent2D), GpuError> {
    let capabilities = unsafe { surface_loader.get_physical_device_surface_capabilities(physical_device, surface) }?;
    let formats = unsafe { surface_loader.get_physical_device_surface_formats(physical_device, surface) }?;
    let mut image_count = capabilities.min_image_count.saturating_add(1);
    if capabilities.max_image_count > 0 {
        image_count = image_count.min(capabilities.max_image_count);
    }

    let extent = if capabilities.current_extent.width != u32::MAX {
        capabilities.current_extent
    } else {
        vk::Extent2D {
            width: width.clamp(capabilities.min_image_extent.width, capabilities.max_image_extent.width),
            height: height.clamp(
                capabilities.min_image_extent.height,
                capabilities.max_image_extent.height,
            ),
        }
    };

    let desired = [vk::Format::R8G8B8A8_SRGB, vk::Format::B8G8R8A8_SRGB];
    let surface_format = desired
        .iter()
        .find_map(|format| {
            formats
                .iter()
                .find(|available| {
                    available.format == *format && available.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
                })
                .copied()
        })
        .or_else(|| formats.first().copied())
        .ok_or(GpuError::Vulkan(vk::Result::ERROR_FORMAT_NOT_SUPPORTED))?;

    let create_info = vk::SwapchainCreateInfoKHR::default()
        .surface(surface)
        .min_image_count(image_count)
        .image_format(surface_format.format)
        .image_color_space(surface_format.color_space)
        .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_DST)
        .image_extent(extent)
        .image_array_layers(1)
        .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
        .present_mode(vk::PresentModeKHR::FIFO)
        .pre_transform(capabilities.current_transform)
        .clipped(true)
        .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
        .old_swapchain(old);

    let handle = unsafe { swapchain_loader.create_swapchain(&create_info, None) }?;

    Ok((handle, surface_format.format, extent))
}
