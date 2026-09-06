use std::ops::Range;

use ash::vk;
use dear_imgui_rs::render::PendingFrame;
use vir::{
    Access,
    AllocatorKind,
    BlendPreset,
    Buffer,
    BufferImageCopy,
    BufferInfo,
    DomainFlag,
    GraphicsPipelineInfo,
    Image,
    ImageAttachment,
    ImageInfo,
    MemoryLocation,
    Module,
    PassCallback,
    PipelineId,
    Program,
    RasterizationState,
    Rect2D,
    RenderGraph,
    SamplerInfo,
    SuperFrameAllocator,
    SwapChain,
    ValueId,
    allocator::Allocator,
};

use crate::{
    Device,
    Frame,
    GpuError,
    SpriteInstance,
    extent3d,
    imgui::{ImGuiPass, ImGuiSlots},
    read_spirv,
    texture::TextureCatalog,
};

const VERTEX_SPIRV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sprite.vert.spv"));
const FRAGMENT_SPIRV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sprite.frag.spv"));
const BLUR_VERTEX_SPIRV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/blur.vert.spv"));
const BLUR_FRAGMENT_SPIRV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/blur.frag.spv"));
const UPLOAD_BATCH_BYTES: usize = 64 * 1024 * 1024;

#[repr(C)]
#[derive(Clone, Copy)]
struct GpuSprite {
    position_size: [f32; 4],
    color: [f32; 4],
    texture: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CameraPush {
    center: [f32; 2],
    viewport: [f32; 2],
    zoom: f32,
    base: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct BlurPush {
    sample_step: [f32; 2],
}

struct Recorded {
    program: Program,
    scene_extent: ValueId,
    underlay_extent: ValueId,
    sprites: ValueId,
    camera: ValueId,
    blur_push: ValueId,
    underlay_draws: ValueId,
    active_draws: ValueId,
    ui: Option<ImGuiSlots>,
}

struct TextureImage {
    image: Image,
    view: vk::ImageView,
}

struct TextureSource<'a> {
    width: u32,
    height: u32,
    pixels: &'a [u8],
}

pub struct Renderer {
    graph: RenderGraph,
    recorded: Option<Recorded>,
    frames: Option<SuperFrameAllocator>,
    swapchain: Option<SwapChain>,
    pipeline: PipelineId,
    blur_pipeline: PipelineId,
    bindless: BindlessDescriptorSet,
    textures: Vec<TextureImage>,
    fallback: Option<TextureImage>,
    sampler: vk::Sampler,
    blur_sampler: vk::Sampler,
    sprites: Option<Buffer>,
    sprite_capacity: usize,
    uploaded_revision: Option<u64>,
    ranges: Vec<(u32, u32)>,
    imgui: Option<ImGuiPass>,
    extent: vk::Extent2D,
    stale: bool,
    device: Device,
}

impl Renderer {
    pub const VIEWPORT_TEXTURE: dear_imgui_rs::TextureId = crate::imgui::VIEWPORT_TEXTURE;

    pub fn new(device: Device, width: u32, height: u32) -> Result<Self, GpuError> {
        let vertex = read_spirv(VERTEX_SPIRV)?;
        let fragment = read_spirv(FRAGMENT_SPIRV)?;
        let blur_vertex = read_spirv(BLUR_VERTEX_SPIRV)?;
        let blur_fragment = read_spirv(BLUR_FRAGMENT_SPIRV)?;
        let bindless = BindlessDescriptorSet::create(&device)?;
        let mut graph = RenderGraph::new(&device.context);
        let pipeline = match graph.declare_pipeline(
            GraphicsPipelineInfo::new()
                .with_shader(&vertex)
                .with_shader(&fragment)
                .with_bindless_set(1, bindless.layout, bindless.set),
        ) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                drop(graph);
                bindless.destroy(&device);

                return Err(error.into());
            },
        };
        let blur_pipeline = match graph.declare_pipeline(
            GraphicsPipelineInfo::new()
                .with_shader(&blur_vertex)
                .with_shader(&blur_fragment),
        ) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                drop(graph);
                bindless.destroy(&device);

                return Err(error.into());
            },
        };

        let mut renderer = Self {
            graph,
            recorded: None,
            frames: None,
            swapchain: None,
            pipeline,
            blur_pipeline,
            bindless,
            textures: Vec::new(),
            fallback: None,
            sampler: vk::Sampler::null(),
            blur_sampler: vk::Sampler::null(),
            sprites: None,
            sprite_capacity: 0,
            uploaded_revision: None,
            ranges: Vec::new(),
            imgui: None,
            extent: vk::Extent2D {
                width: width.max(1),
                height: height.max(1),
            },
            stale: true,
            device,
        };

        renderer.sampler = renderer
            .device
            .allocator
            .allocate_sampler(&SamplerInfo::nearest().with_address_mode(vk::SamplerAddressMode::CLAMP_TO_EDGE))?;
        renderer.blur_sampler = renderer.device.allocator.allocate_sampler(&SamplerInfo::linear())?;
        renderer.fallback = renderer
            .upload_images(&[TextureSource {
                width: 1,
                height: 1,
                pixels: &[0; 4],
            }])?
            .pop();
        renderer.write_descriptors(&[])?;
        renderer.recreate_swapchain()?;

        Ok(renderer)
    }

    pub fn extent(&self) -> vk::Extent2D { self.extent }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }

        self.extent = vk::Extent2D { width, height };
        self.stale = true;
    }

    pub fn upload_textures(&mut self, catalog: &TextureCatalog) -> Result<(), GpuError> {
        let requested = u32::try_from(catalog.len()).map_err(|_| GpuError::TextureUploadTooLarge)?;
        if requested > self.device.max_bindless_textures {
            return Err(GpuError::TooManyTextures {
                requested,
                limit: self.device.max_bindless_textures,
            });
        }

        self.device.wait_idle()?;

        let sources = catalog
            .textures()
            .iter()
            .map(|texture| TextureSource {
                width: texture.width(),
                height: texture.height(),
                pixels: texture.pixels(),
            })
            .collect::<Vec<_>>();
        let textures = self.upload_images(&sources)?;
        if let Err(error) = self.write_descriptors(&textures) {
            destroy_images(&mut self.device, textures);

            return Err(error);
        }

        self.recorded = None;
        self.uploaded_revision = None;
        let old = std::mem::replace(&mut self.textures, textures);
        destroy_images(&mut self.device, old);

        Ok(())
    }

    pub fn draw(&mut self, frame: &Frame) -> Result<(), GpuError> { self.draw_inner(frame, None, None) }

    pub fn draw_imgui(
        &mut self, frame: &Frame, viewport: (u32, u32), pending: PendingFrame<'_>,
    ) -> Result<(), GpuError> {
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

        let viewport = vk::Extent2D {
            width: viewport.0.max(1),
            height: viewport.1.max(1),
        };

        self.draw_inner(frame, Some(viewport), Some(pending))
    }

    pub fn reset_imgui_textures(&mut self) -> Result<(), GpuError> {
        if let Some(imgui) = self.imgui.as_mut() {
            imgui.reset_textures(&mut self.device)?;
            self.recorded = None;
        }

        Ok(())
    }

    fn recreate_swapchain(&mut self) -> Result<(), GpuError> {
        self.device.wait_idle()?;
        self.recorded = None;

        let (swapchain, extent, _) =
            self.device
                .create_swapchain(self.extent.width, self.extent.height, self.swapchain.as_ref())?;
        self.frames = Some(
            self.device
                .context
                .create_super_frame_allocator(swapchain.attachments.len()),
        );
        self.swapchain = Some(swapchain);
        self.extent = extent;
        self.stale = false;

        Ok(())
    }

    fn record(&mut self, viewport: vk::Extent2D, with_imgui: bool) -> Result<(), GpuError> {
        let swapchain = self.swapchain.as_ref().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
        let mut module = Module::default();
        let swapchain_image = module.acquire_next_image(swapchain);

        let scene_extent = module.declare_extent_3d_var("scene extent", extent3d(viewport));
        let scene = module.transient_image_sized(
            &ImageInfo::color_target(viewport, vk::Format::R8G8B8A8_SRGB)
                .with_usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_SRC)
                .with_name("scene"),
            scene_extent,
        );
        let scene = module.clear(scene, vir::clear::f32::BLACK);

        let underlay_extent = module.declare_extent_3d_var("underlay extent", extent3d(underlay_viewport(viewport)));
        let underlays = module.transient_image_sized(
            &ImageInfo::color_target(underlay_viewport(viewport), vk::Format::R8G8B8A8_UNORM)
                .with_usage(vk::ImageUsageFlags::SAMPLED)
                .with_name("underlays"),
            underlay_extent,
        );
        let underlays = module.clear(underlays, vir::clear::f32::BLACK);

        let sprites = module.declare_buffer_var("sprites", Access::HostWrite);
        let camera = module.declare_bytes_var("camera", size_of::<CameraPush>() as u32);
        let blur_push = module.declare_bytes_var("blur", size_of::<BlurPush>() as u32);
        let underlay_draws = module.declare_callback_var("underlay draws");
        let active_draws = module.declare_callback_var("active draws");

        let underlays_drawn = module
            .begin_rendering([(underlays, Access::ColorRW)])
            .with_name("underlays")
            .bind_graphics_pipeline(self.pipeline)
            .set_primitive_topology(vk::PrimitiveTopology::TRIANGLE_STRIP)
            .set_viewport(0, Rect2D::framebuffer())
            .set_scissor(0, Rect2D::framebuffer())
            .broadcast_color_blend(BlendPreset::PremultipliedAlphaBlend)
            .set_rasterization(RasterizationState {
                cull_mode: vk::CullModeFlags::NONE,
                ..Default::default()
            })
            .bind_buffer(0, 1, sprites)
            .push_constants_from(camera)
            .record_from(underlay_draws)
            .end_rendering();

        let blurred = module
            .begin_rendering([(scene, Access::ColorRW)])
            .with_name("underlay blur")
            .bind_graphics_pipeline(self.blur_pipeline)
            .set_primitive_topology(vk::PrimitiveTopology::TRIANGLE_STRIP)
            .set_viewport(0, Rect2D::framebuffer())
            .set_scissor(0, Rect2D::framebuffer())
            .bind_texture(0, 0, underlays_drawn, self.blur_sampler)
            .push_constants_from(blur_push)
            .draw(4u32, 1u32)
            .end_rendering();

        let drawn = module
            .begin_rendering([(blurred, Access::ColorRW)])
            .with_name("sprites")
            .bind_graphics_pipeline(self.pipeline)
            .set_primitive_topology(vk::PrimitiveTopology::TRIANGLE_STRIP)
            .set_viewport(0, Rect2D::framebuffer())
            .set_scissor(0, Rect2D::framebuffer())
            .broadcast_color_blend(BlendPreset::PremultipliedAlphaBlend)
            .set_rasterization(RasterizationState {
                cull_mode: vk::CullModeFlags::NONE,
                ..Default::default()
            })
            .bind_buffer(0, 1, sprites)
            .push_constants_from(camera)
            .record_from(active_draws)
            .end_rendering();

        let (presented, ui) = if with_imgui {
            let imgui = self.imgui.as_ref().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
            let target = module.clear(swapchain_image, vir::clear::f32::BLACK);

            imgui.record(&mut module, target, drawn)
        } else {
            (module.blit_filtered(drawn, swapchain_image, vk::Filter::NEAREST), None)
        };
        let present = module.present(presented);
        let program = module.compile(&self.graph, present)?;

        self.recorded = Some(Recorded {
            program,
            scene_extent,
            underlay_extent,
            sprites,
            camera,
            blur_push,
            underlay_draws,
            active_draws,
            ui,
        });

        Ok(())
    }

    fn draw_inner(
        &mut self, frame: &Frame, viewport: Option<vk::Extent2D>, pending: Option<PendingFrame<'_>>,
    ) -> Result<(), GpuError> {
        if self.stale {
            self.recreate_swapchain()?;
        }

        let viewport = viewport.unwrap_or(self.extent);
        let with_imgui = pending.is_some();
        if self.recorded.as_ref().is_none_or(|r| r.ui.is_some() != with_imgui) {
            self.record(viewport, with_imgui)?;
        }

        self.prepare_sprites(frame)?;

        let recorded = self.recorded.as_mut().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
        let frames = self.frames.as_mut().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
        let sprites = self.sprites.as_ref().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
        recorded.program.set(recorded.scene_extent, extent3d(viewport));
        let underlay_extent = underlay_viewport(viewport);
        recorded
            .program
            .set(recorded.underlay_extent, extent3d(underlay_extent));
        recorded.program.set(recorded.sprites, sprites);

        let push = CameraPush {
            center: [frame.camera.x, frame.camera.y],
            viewport: [viewport.width as f32, viewport.height as f32],
            zoom: frame.camera.zoom,
            base: 0,
        };
        recorded.program.set_bytes(recorded.camera, &push);
        recorded.program.set_bytes(
            recorded.blur_push,
            &BlurPush {
                sample_step: [
                    frame.camera.zoom / underlay_extent.width as f32,
                    frame.camera.zoom / underlay_extent.height as f32,
                ],
            },
        );

        let mut ranges = self.ranges.clone();
        let active_range = ranges.pop().unwrap_or((0, 0));
        let underlay_ranges = ranges;
        recorded.program.set(
            recorded.underlay_draws,
            PassCallback::new(move |cmd| {
                for &(base, count) in &underlay_ranges {
                    cmd.push_constants_at(std::mem::offset_of!(CameraPush, base) as u32, &base)
                        .draw(4, count);
                }
            }),
        );
        recorded.program.set(
            recorded.active_draws,
            PassCallback::new(move |cmd| {
                let (base, count) = active_range;
                cmd.push_constants_at(std::mem::offset_of!(CameraPush, base) as u32, &base)
                    .draw(4, count);
            }),
        );

        let next = frames.get_next_frame()?;
        if let Some(pending) = pending {
            let imgui = self.imgui.as_mut().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
            let feedback = imgui.poll_textures(&mut self.device, &mut self.graph, next, pending.texture_requests())?;
            let reconciled = pending
                .reconcile_texture_feedback(feedback)
                .map_err(|e| GpuError::ImGui(e.to_string()))?;
            let ui_frame = imgui.prepare(next, reconciled.draw_data(), self.extent)?;

            if let Some(slots) = recorded.ui.as_ref() {
                imgui.bind(&mut recorded.program, slots, &ui_frame);
            }
        }

        match self
            .graph
            .execute(&self.device.context, &recorded.program, &mut AllocatorKind::Frame(next))
        {
            Ok(()) => Ok(()),
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR | vk::Result::SUBOPTIMAL_KHR) => {
                self.stale = true;

                Ok(())
            },
            Err(error) => Err(error.into()),
        }
    }

    fn prepare_sprites(&mut self, frame: &Frame) -> Result<(), GpuError> {
        let level_count = frame
            .underlays
            .len()
            .checked_add(1)
            .ok_or(GpuError::SpriteUploadTooLarge)?;
        if self.uploaded_revision == Some(frame.revision) && self.ranges.len() == level_count {
            return Ok(());
        }

        let total = levels(frame)
            .try_fold(0usize, |total, level| total.checked_add(level.len()))
            .ok_or(GpuError::SpriteUploadTooLarge)?;
        u32::try_from(total).map_err(|_| GpuError::SpriteUploadTooLarge)?;
        let mut payload = Vec::with_capacity(total);
        let mut ranges = Vec::with_capacity(level_count);

        for level in levels(frame) {
            ranges.push((payload.len() as u32, level.len() as u32));
            payload.extend(level.iter().map(|sprite| GpuSprite {
                position_size: [
                    sprite.x,
                    sprite.y,
                    sprite.texture.width as f32,
                    sprite.texture.height as f32,
                ],
                color: sprite.color,
                texture: sprite.texture.index,
            }));
        }

        self.device.wait_idle()?;
        self.uploaded_revision = None;

        if self.sprites.is_none() || self.sprite_capacity < total {
            let capacity = total
                .max(1024)
                .checked_next_power_of_two()
                .ok_or(GpuError::SpriteUploadTooLarge)?;
            let size = capacity
                .checked_mul(size_of::<GpuSprite>())
                .ok_or(GpuError::SpriteUploadTooLarge)?;
            let size = u64::try_from(size).map_err(|_| GpuError::SpriteUploadTooLarge)?;
            let mut buffer = self.device.allocator.allocate_buffer(
                &BufferInfo::new(size, vk::BufferUsageFlags::STORAGE_BUFFER, MemoryLocation::CpuToGpu)
                    .with_name("sprites"),
            )?;
            if let Err(error) = buffer.write(0, &payload) {
                self.device.allocator.deallocate_buffer(buffer);

                return Err(error.into());
            }

            if let Some(old) = self.sprites.replace(buffer) {
                self.device.allocator.deallocate_buffer(old);
            }

            self.sprite_capacity = capacity;
        } else if let Some(buffer) = self.sprites.as_mut() {
            buffer.write(0, &payload)?;
        }

        self.ranges = ranges;
        self.uploaded_revision = Some(frame.revision);

        Ok(())
    }

    fn write_descriptors(&self, textures: &[TextureImage]) -> Result<(), GpuError> {
        let fallback = self.fallback.as_ref().ok_or(vk::Result::ERROR_INITIALIZATION_FAILED)?;
        let info = vk::DescriptorImageInfo::default()
            .sampler(self.sampler)
            .image_view(fallback.view)
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        let mut infos = vec![info; self.device.max_bindless_textures as usize];

        for (info, texture) in infos.iter_mut().zip(textures) {
            info.image_view = texture.view;
        }

        self.bindless.write(&infos, &self.device);

        Ok(())
    }

    fn upload_images(&mut self, sources: &[TextureSource<'_>]) -> Result<Vec<TextureImage>, GpuError> {
        let mut textures = Vec::with_capacity(sources.len());
        let result = (|| {
            for (index, source) in sources.iter().enumerate() {
                let extent = vk::Extent2D {
                    width: source.width,
                    height: source.height,
                };
                let image = self.device.allocator.allocate_image(
                    &ImageInfo::texture(extent, vk::Format::R8G8B8A8_SRGB).with_name(format!("sprite texture {index}")),
                )?;
                let view = match self.device.allocator.allocate_image_view(
                    image.handle(),
                    vk::Format::R8G8B8A8_SRGB,
                    vk::ImageViewType::TYPE_2D,
                    image.subresource_range(),
                ) {
                    Ok(view) => view,
                    Err(error) => {
                        self.device.allocator.deallocate_image(image);

                        return Err(error.into());
                    },
                };

                textures.push(TextureImage { image, view });
            }

            let sizes = sources.iter().map(|source| source.pixels.len()).collect::<Vec<_>>();
            let ranges = batch_ranges(&sizes, UPLOAD_BATCH_BYTES).ok_or(GpuError::TextureUploadTooLarge)?;

            for range in ranges {
                let batch_sources = sources.get(range.clone()).ok_or(GpuError::TextureUploadTooLarge)?;
                let batch_textures = textures.get(range).ok_or(GpuError::TextureUploadTooLarge)?;
                upload_batch(&mut self.device, &mut self.graph, batch_sources, batch_textures)?;
            }

            Ok(())
        })();
        if let Err(error) = result {
            destroy_images(&mut self.device, textures);

            return Err(error);
        }

        Ok(textures)
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        let _ = self.device.wait_idle();
        self.recorded = None;

        if let Some(imgui) = self.imgui.as_mut() {
            imgui.destroy(&mut self.device);
        }

        self.bindless.destroy(&self.device);
        destroy_images(&mut self.device, std::mem::take(&mut self.textures));
        destroy_images(&mut self.device, self.fallback.take());
        self.device.allocator.deallocate_sampler(self.sampler);
        self.device.allocator.deallocate_sampler(self.blur_sampler);

        if let Some(buffer) = self.sprites.take() {
            self.device.allocator.deallocate_buffer(buffer);
        }
    }
}

fn levels(frame: &Frame) -> impl Iterator<Item = &[SpriteInstance]> {
    frame
        .underlays
        .iter()
        .map(Vec::as_slice)
        .chain(std::iter::once(frame.sprites.as_slice()))
}

/// Underlays render at this resolution and the blur pass upsamples them back to `viewport` while it blurs.
fn underlay_viewport(viewport: vk::Extent2D) -> vk::Extent2D {
    vk::Extent2D {
        width: viewport.width,
        height: viewport.height,
    }
}

fn destroy_images(device: &mut Device, textures: impl IntoIterator<Item = TextureImage>) {
    for texture in textures {
        device.allocator.deallocate_image_view(texture.view);
        device.allocator.deallocate_image(texture.image);
    }
}

struct BindlessDescriptorSet {
    layout: vk::DescriptorSetLayout,
    set: vk::DescriptorSet,
    pool: vk::DescriptorPool,
}

impl BindlessDescriptorSet {
    fn create(device: &Device) -> Result<Self, GpuError> {
        let count = device.max_bindless_textures;
        let vk_device = device.context.device();

        let binding = vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .descriptor_count(count)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT);
        let bindings = [binding];
        let layout_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
        let layout = unsafe { vk_device.create_descriptor_set_layout(&layout_info, None) }?;

        let pool_size = vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .descriptor_count(count);
        let pool_sizes = [pool_size];
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .max_sets(1)
            .pool_sizes(&pool_sizes);
        let pool = match unsafe { vk_device.create_descriptor_pool(&pool_info, None) } {
            Ok(pool) => pool,
            Err(error) => {
                unsafe { vk_device.destroy_descriptor_set_layout(layout, None) };
                return Err(error.into());
            },
        };

        let layouts = [layout];
        let allocate_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(pool)
            .set_layouts(&layouts);
        let set = match unsafe { vk_device.allocate_descriptor_sets(&allocate_info) } {
            Ok(sets) => match sets.first().copied() {
                Some(set) => set,
                None => {
                    unsafe {
                        vk_device.destroy_descriptor_pool(pool, None);
                        vk_device.destroy_descriptor_set_layout(layout, None);
                    }
                    return Err(vk::Result::ERROR_INITIALIZATION_FAILED.into());
                },
            },
            Err(error) => {
                unsafe {
                    vk_device.destroy_descriptor_pool(pool, None);
                    vk_device.destroy_descriptor_set_layout(layout, None);
                }
                return Err(error.into());
            },
        };

        Ok(Self { layout, set, pool })
    }

    fn destroy(&self, device: &Device) {
        let vk_device = device.context.device();
        unsafe {
            vk_device.destroy_descriptor_pool(self.pool, None);
            vk_device.destroy_descriptor_set_layout(self.layout, None);
        }
    }

    fn write(&self, infos: &[vk::DescriptorImageInfo], device: &Device) {
        let vk_device = device.context.device();

        let write = vk::WriteDescriptorSet::default()
            .dst_set(self.set)
            .dst_binding(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .image_info(infos);
        unsafe { vk_device.update_descriptor_sets(std::slice::from_ref(&write), &[]) }
    }
}

fn upload_batch(
    device: &mut Device, graph: &mut RenderGraph, sources: &[TextureSource<'_>], textures: &[TextureImage],
) -> Result<(), GpuError> {
    let total = sources
        .iter()
        .try_fold(0usize, |total, source| total.checked_add(source.pixels.len()))
        .ok_or(GpuError::TextureUploadTooLarge)?;
    let total = u64::try_from(total).map_err(|_| GpuError::TextureUploadTooLarge)?;
    let mut staging = device
        .allocator
        .allocate_buffer(&BufferInfo::staging(total).with_name("sprite texture staging"))?;

    let result = (|| {
        let mut offset = 0u64;
        for source in sources {
            staging.write(offset, source.pixels)?;
            offset = offset
                .checked_add(u64::try_from(source.pixels.len()).map_err(|_| GpuError::TextureUploadTooLarge)?)
                .ok_or(GpuError::TextureUploadTooLarge)?;
        }

        let mut module = Module::default();
        let source_buffer = module.import_buffer(&staging, Access::HostWrite);
        let mut roots = Vec::with_capacity(textures.len());
        let mut offset = 0u64;

        for (source, texture) in sources.iter().zip(textures) {
            let destination =
                module.import_attachment(&ImageAttachment::from_image(&texture.image, vk::ImageLayout::UNDEFINED));
            let copied = module.copy_buffer_to_image_region(
                source_buffer,
                destination,
                copy_region(offset, source.width, source.height),
            );
            roots.push(module.release(copied, Access::FragmentSampled, DomainFlag::Graphics));
            offset = offset
                .checked_add(u64::try_from(source.pixels.len()).map_err(|_| GpuError::TextureUploadTooLarge)?)
                .ok_or(GpuError::TextureUploadTooLarge)?;
        }

        let program = module.compile_all(&*graph, &roots)?;
        graph.execute_blocking(
            &device.context,
            &program,
            &mut AllocatorKind::Persistent(&mut device.allocator),
        )?;

        Ok(())
    })();

    if result.is_err() {
        let _ = device.wait_idle();
    }

    device.allocator.deallocate_buffer(staging);

    result
}

fn copy_region(buffer_offset: u64, width: u32, height: u32) -> BufferImageCopy {
    BufferImageCopy {
        buffer_offset,
        image_offset: vk::Offset3D { x: 0, y: 0, z: 0 },
        image_extent: vk::Extent3D {
            width,
            height,
            depth: 1,
        },
        mip_level: 0,
    }
}

fn batch_ranges(sizes: &[usize], budget: usize) -> Option<Vec<Range<usize>>> {
    if budget == 0 && !sizes.is_empty() {
        return None;
    }

    let mut ranges = Vec::new();
    let mut start = 0usize;
    let mut bytes = 0usize;

    for (index, size) in sizes.iter().copied().enumerate() {
        let next = bytes.checked_add(size)?;
        if index > start && next > budget {
            ranges.push(start..index);
            start = index;
            bytes = size;
        } else {
            bytes = next;
        }
    }

    if start < sizes.len() {
        ranges.push(start..sizes.len());
    }

    Some(ranges)
}

#[cfg(test)]
mod tests {
    use super::{UPLOAD_BATCH_BYTES, batch_ranges, copy_region};

    #[test]
    fn copies_are_always_two_dimensional() {
        let region = copy_region(128, 32, 48);

        assert_eq!(region.buffer_offset, 128);
        assert_eq!(
            (region.image_offset.x, region.image_offset.y, region.image_offset.z),
            (0, 0, 0)
        );
        assert_eq!(
            (
                region.image_extent.width,
                region.image_extent.height,
                region.image_extent.depth
            ),
            (32, 48, 1)
        );
    }

    #[test]
    fn large_uploads_are_split_at_texture_boundaries() {
        let half = UPLOAD_BATCH_BYTES / 2;
        let ranges = batch_ranges(&[half, half, 4], UPLOAD_BATCH_BYTES).expect("valid ranges");

        assert_eq!(ranges, vec![0..2, 2..3]);
    }

    #[test]
    fn one_oversized_texture_gets_its_own_batch() {
        let ranges = batch_ranges(&[UPLOAD_BATCH_BYTES + 4, 4], UPLOAD_BATCH_BYTES).expect("valid ranges");

        assert_eq!(ranges, vec![0..1, 1..2]);
    }
}
