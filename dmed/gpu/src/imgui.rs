use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use ash::vk;
use dear_imgui_rs::{
    TextureId,
    render::{
        DrawCmd,
        DrawData,
        SnapshotTextureId,
        TextureFeedback,
        TextureOp,
        TextureRequest,
        TextureUploadIdentity,
        TextureUploadRect,
    },
    texture::{TextureFormat, get_format_bytes_per_pixel},
};
use vir::{
    Access,
    AllocatorKind,
    BlendPreset,
    Buffer,
    BufferImageCopy,
    BufferInfo,
    Context,
    DomainFlag,
    DynamicStateFlags,
    FrameAllocator,
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
    ValueId,
    allocator::Allocator,
};

use crate::{device::Device, error::GpuError, read_spirv};

const VERTEX_SPIRV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/imgui.vert.spv"));
const FRAGMENT_SPIRV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/imgui.frag.spv"));
const TEXTURE_FORMAT: vk::Format = vk::Format::R8G8B8A8_SRGB;
const TEXTURE_RESTING: Access = Access::FragmentSampled;
pub const VIEWPORT_TEXTURE: TextureId = TextureId::new(1);

const FIRST_MINTED_TEXTURE: u64 = 2;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct PushConstants {
    scale: [f32; 2],
    translate: [f32; 2],
}

struct Texture {
    image: Image,
    image_view: vk::ImageView,
    extent: vk::Extent2D,
    format: TextureFormat,
    uploaded: bool,
    upload: Option<TextureUploadIdentity>,
}

impl Texture {
    fn attachment(&self, layout: vk::ImageLayout) -> ImageAttachment {
        ImageAttachment::from_image(&self.image, layout).with_image_view(self.image_view)
    }
}

#[derive(Clone)]
enum Source {
    Owned(ImageAttachment),
    Viewport,
}

#[derive(Clone)]
struct MeshDraw {
    clip: vk::Rect2D,
    source: Source,
    sampler: vk::Sampler,
    index_offset: u32,
    index_count: u32,
    vertex_offset: i32,
}

struct Upload {
    texture: TextureId,
    staging: Buffer,
    region: BufferImageCopy,
}

pub struct ImGuiFrame {
    vertices: Buffer,
    indices: Buffer,
    draws: Arc<[MeshDraw]>,
    push: PushConstants,
}

impl ImGuiFrame {
    pub fn draw_count(&self) -> usize { self.draws.len() }
}

pub struct ImGuiSlots {
    has_draws: ValueId,
    vertices: ValueId,
    indices: ValueId,
    push: ValueId,
    body: ValueId,
    viewport: ValueId,
}

pub struct ImGuiPass {
    pipeline: Option<PipelineId>,
    textures: HashMap<TextureId, Texture>,
    minted: HashMap<SnapshotTextureId, TextureId>,
    retired: HashSet<SnapshotTextureId>,
    linear: vk::Sampler,
    nearest: vk::Sampler,
    next_texture: u64,
}

impl ImGuiPass {
    pub fn new(device: &mut Device) -> Result<Self, GpuError> {
        let linear = device
            .allocator
            .allocate_sampler(&SamplerInfo::linear().with_address_mode(vk::SamplerAddressMode::CLAMP_TO_EDGE))?;
        let nearest = match device
            .allocator
            .allocate_sampler(&SamplerInfo::nearest().with_address_mode(vk::SamplerAddressMode::CLAMP_TO_EDGE))
        {
            Ok(nearest) => nearest,
            Err(error) => {
                device.allocator.deallocate_sampler(linear);
                return Err(error.into());
            },
        };

        Ok(Self {
            pipeline: None,
            textures: HashMap::new(),
            minted: HashMap::new(),
            retired: HashSet::new(),
            linear,
            nearest,
            next_texture: FIRST_MINTED_TEXTURE,
        })
    }

    pub fn declare_pipeline(&mut self, graph: &mut RenderGraph) -> Result<(), GpuError> {
        let vertex = read_spirv(VERTEX_SPIRV)?;
        let fragment = read_spirv(FRAGMENT_SPIRV)?;

        self.pipeline =
            Some(graph.declare_pipeline(GraphicsPipelineInfo::new().with_shader(&vertex).with_shader(&fragment))?);

        Ok(())
    }

    pub fn record(&self, module: &mut Module, target: ValueId, viewport: ValueId) -> (ValueId, Option<ImGuiSlots>) {
        let Some(pipeline) = self.pipeline else {
            return (target, None);
        };

        let slots = ImGuiSlots {
            has_draws: module.declare_bool_var("has interface", false),
            vertices: module.declare_buffer_var("imgui vertices", Access::HostWrite),
            indices: module.declare_buffer_var("imgui indices", Access::HostWrite),
            push: module.declare_bytes_var("imgui push block", size_of::<PushConstants>() as u32),
            body: module.declare_callback_var("imgui draws"),
            viewport,
        };

        let drawn = module.set_condition(
            slots.has_draws,
            |m| {
                m.begin_rendering([(target, Access::ColorRW), (viewport, TEXTURE_RESTING)])
                    .with_name("imgui")
                    .bind_graphics_pipeline(pipeline)
                    .set_dynamic_state(DynamicStateFlags::Viewport | DynamicStateFlags::Scissor)
                    .set_viewport(0, Rect2D::framebuffer())
                    .set_scissor(0, Rect2D::framebuffer())
                    .broadcast_color_blend(BlendPreset::AlphaBlend)
                    .set_rasterization(RasterizationState {
                        cull_mode: vk::CullModeFlags::NONE,
                        ..Default::default()
                    })
                    .push_constants_from(slots.push)
                    .bind_vertex_buffer(0, slots.vertices)
                    .bind_index_buffer(slots.indices, vk::IndexType::UINT16)
                    .record_from(slots.body)
                    .end_rendering()
            },
            |_| target,
        );

        (drawn, Some(slots))
    }

    pub fn bind(&self, program: &mut Program, slots: &ImGuiSlots, frame: &ImGuiFrame) {
        program.set(slots.has_draws, !frame.draws.is_empty());
        program.set_bytes(slots.push, &frame.push);

        program.set(slots.vertices, frame.vertices);
        program.set(slots.indices, frame.indices);

        if frame.draws.is_empty() {
            program.set(slots.body, PassCallback::empty());

            return;
        }

        let draws = frame.draws.clone();
        let viewport = slots.viewport;
        program.set(
            slots.body,
            PassCallback::new(move |cmd| {
                for draw in draws.iter() {
                    let attachment = match &draw.source {
                        Source::Owned(attachment) => attachment.clone(),
                        Source::Viewport => cmd.image(viewport),
                    };

                    cmd.set_scissor(
                        0,
                        Rect2D::absolute(
                            draw.clip.offset.x,
                            draw.clip.offset.y,
                            draw.clip.extent.width,
                            draw.clip.extent.height,
                        ),
                    )
                    .bind_texture(0, 0, &attachment, draw.sampler)
                    .draw_indexed_range(
                        draw.index_count,
                        1,
                        draw.index_offset,
                        draw.vertex_offset,
                        0,
                    );
                }
            }),
        );
    }

    pub fn prepare(
        &self, allocator: &mut FrameAllocator, data: &DrawData, target: vk::Extent2D,
    ) -> Result<ImGuiFrame, GpuError> {
        if data.requirements().requires_raw_callback_support() {
            return Err(GpuError::RawDrawCallback);
        }

        let display_pos = data.display_pos();
        let display_size = data.display_size();
        let scale = data.framebuffer_scale();

        let mut vertices: Vec<DrawVertex> = Vec::with_capacity(data.total_vtx_count());
        let mut indices: Vec<u16> = Vec::with_capacity(data.total_idx_count());
        let mut draws = Vec::new();
        let mut sampler = self.linear;

        for list in data.draw_lists() {
            let list_vertices = list.vtx_buffer();
            let list_indices = list.idx_buffer();
            let vertex_offset = vertices.len();

            let mut appended = false;

            for command in list.commands() {
                match command {
                    DrawCmd::SetSamplerLinear => sampler = self.linear,
                    DrawCmd::SetSamplerNearest => sampler = self.nearest,
                    DrawCmd::ResetRenderState => {},
                    DrawCmd::RawCallback(_) => return Err(GpuError::RawDrawCallback),
                    DrawCmd::Elements { count, cmd_params } => {
                        if count == 0 {
                            continue;
                        }

                        let clip = scissor(cmd_params.clip_rect, display_pos, scale, target);
                        if clip.extent.width == 0 || clip.extent.height == 0 {
                            continue;
                        }

                        let source = if cmd_params.texture_id == VIEWPORT_TEXTURE {
                            Source::Viewport
                        } else {
                            let Some(texture) = self.textures.get(&cmd_params.texture_id) else {
                                return Err(GpuError::UnknownTexture(cmd_params.texture_id.id()));
                            };

                            Source::Owned(texture.attachment(TEXTURE_RESTING.into()))
                        };

                        if !appended {
                            vertices.extend_from_slice(cast_vertices(list_vertices));
                            appended = true;
                        }

                        let Some(range) = list_indices.get(cmd_params.idx_offset..) else {
                            continue;
                        };
                        let Some(range) = range.get(..count) else {
                            continue;
                        };

                        draws.push(MeshDraw {
                            clip,
                            source,
                            sampler,
                            index_offset: indices.len() as u32,
                            index_count: count as u32,
                            vertex_offset: (vertex_offset + cmd_params.vtx_offset) as i32,
                        });
                        indices.extend_from_slice(range);
                    },
                }
            }
        }

        let push = PushConstants {
            scale: [
                2.0 / display_size[0].max(f32::EPSILON),
                2.0 / display_size[1].max(f32::EPSILON),
            ],
            translate: [
                -1.0 - display_pos[0] * (2.0 / display_size[0].max(f32::EPSILON)),
                -1.0 - display_pos[1] * (2.0 / display_size[1].max(f32::EPSILON)),
            ],
        };

        if draws.is_empty() {
            return Ok(ImGuiFrame {
                vertices: Buffer::default(),
                indices: Buffer::default(),
                draws: Arc::from(draws),
                push,
            });
        }

        let mut vertex_buffer = allocator.allocate_buffer(
            &BufferInfo::vertex(size_of_val(vertices.as_slice()) as u64).with_name("imgui vertices"),
        )?;
        vertex_buffer.write(0, &vertices)?;

        let mut index_buffer = allocator.allocate_buffer(
            &BufferInfo::new(
                size_of_val(indices.as_slice()) as u64,
                vk::BufferUsageFlags::INDEX_BUFFER,
                MemoryLocation::CpuToGpu,
            )
            .with_name("imgui indices"),
        )?;
        index_buffer.write(0, &indices)?;

        Ok(ImGuiFrame {
            vertices: vertex_buffer,
            indices: index_buffer,
            draws: Arc::from(draws),
            push,
        })
    }

    pub fn poll_textures(
        &mut self, device: &mut Device, graph: &mut RenderGraph, allocator: &mut FrameAllocator,
        requests: &[TextureRequest],
    ) -> Result<Vec<TextureFeedback>, GpuError> {
        let mut feedback = Vec::with_capacity(requests.len());
        let mut uploads = Vec::new();
        let mut committed = Vec::new();
        let mut discarded = Vec::new();

        for request in requests {
            let snapshot = request.texture();

            match request.operation() {
                TextureOp::Create {
                    format,
                    width,
                    height,
                    row_pitch,
                    pixels,
                } => {
                    if self.retired.contains(&snapshot) {
                        feedback.push(request.superseded());
                        continue;
                    }

                    let extent = vk::Extent2D {
                        width: *width,
                        height: *height,
                    };
                    if extent.width == 0 || extent.height == 0 {
                        feedback.push(request.superseded());
                        continue;
                    }

                    let identity = request.upload_identity().ok_or_else(|| {
                        GpuError::ImGui("a texture create request had no upload identity".to_string())
                    })?;
                    let existing = self.minted.get(&snapshot).copied();
                    if let Some(id) = existing
                        && self
                            .textures
                            .get(&id)
                            .is_some_and(|texture| texture.upload == Some(identity))
                    {
                        feedback.push(request.uploaded(id).map_err(imgui_error)?);
                        continue;
                    }

                    let staging = stage(allocator, &rows(*format, extent, *row_pitch, pixels)?)?;
                    let id = self.create_texture(device, snapshot, extent, *format, &mut discarded)?;

                    uploads.push(Upload {
                        texture: id,
                        staging,
                        region: BufferImageCopy::region(vk::Offset2D::default(), extent),
                    });
                    committed.push((id, identity));
                    feedback.push(request.uploaded(id).map_err(imgui_error)?);
                },

                TextureOp::Update {
                    format,
                    width,
                    height,
                    rects,
                } => {
                    if self.retired.contains(&snapshot) {
                        feedback.push(request.superseded());
                        continue;
                    }

                    let Some(id) = self.minted.get(&snapshot).copied() else {
                        feedback.push(request.retry());
                        continue;
                    };
                    let Some(texture) = self.textures.get_mut(&id) else {
                        feedback.push(request.retry());
                        continue;
                    };

                    let identity = request.upload_identity().ok_or_else(|| {
                        GpuError::ImGui("a texture update request had no upload identity".to_string())
                    })?;
                    if texture.upload == Some(identity) {
                        feedback.push(request.uploaded(id).map_err(imgui_error)?);
                        continue;
                    }
                    if texture.format != *format || texture.extent.width != *width || texture.extent.height != *height {
                        return Err(GpuError::TextureUpdateMismatch);
                    }

                    for patch in rects {
                        uploads.push(patch_upload(allocator, id, *format, texture.extent, patch)?);
                    }

                    committed.push((id, identity));
                    feedback.push(request.uploaded(id).map_err(imgui_error)?);
                },

                TextureOp::Destroy => {
                    self.retired.insert(snapshot);

                    if let Some(id) = self.minted.remove(&snapshot)
                        && let Some(texture) = self.textures.remove(&id)
                    {
                        discarded.push(texture);
                    }

                    feedback.push(request.destroyed().map_err(imgui_error)?);
                },
            }
        }

        self.upload(&device.context, graph, allocator, &uploads)?;

        for (id, identity) in committed {
            if let Some(texture) = self.textures.get_mut(&id) {
                texture.upload = Some(identity);
            }
        }

        self.discard(device, discarded)?;

        Ok(feedback)
    }

    pub fn reset_textures(&mut self, device: &mut Device) -> Result<(), GpuError> {
        device.wait_idle()?;
        self.destroy_texture_map(device);

        Ok(())
    }

    pub fn destroy(&mut self, device: &mut Device) {
        self.pipeline = None;
        self.destroy_texture_map(device);

        device.allocator.deallocate_sampler(self.linear);
        device.allocator.deallocate_sampler(self.nearest);
    }

    fn create_texture(
        &mut self, device: &mut Device, snapshot: SnapshotTextureId, extent: vk::Extent2D, format: TextureFormat,
        discarded: &mut Vec<Texture>,
    ) -> Result<TextureId, GpuError> {
        let next = self.next_texture.checked_add(1).ok_or(GpuError::TextureIdExhausted)?;
        let id = TextureId::new(self.next_texture);
        self.next_texture = next;

        let info = ImageInfo::texture(extent, TEXTURE_FORMAT).with_name(format!("imgui texture {}", id.id()));
        let image = device.allocator.allocate_image(&info)?;
        let image_view = match device.allocator.allocate_image_view(
            image.handle(),
            TEXTURE_FORMAT,
            vk::ImageViewType::TYPE_2D,
            image.subresource_range(),
        ) {
            Ok(view) => view,
            Err(error) => {
                device.allocator.deallocate_image(image);
                return Err(error.into());
            },
        };

        self.textures.insert(
            id,
            Texture {
                image,
                image_view,
                extent,
                format,
                uploaded: false,
                upload: None,
            },
        );

        if let Some(previous) = self.minted.insert(snapshot, id)
            && let Some(texture) = self.textures.remove(&previous)
        {
            discarded.push(texture);
        }

        Ok(id)
    }

    fn destroy_texture_map(&mut self, device: &mut Device) {
        self.minted.clear();
        self.retired.clear();

        for (_, texture) in self.textures.drain() {
            device.allocator.deallocate_image_view(texture.image_view);
            device.allocator.deallocate_image(texture.image);
        }
    }

    fn upload(
        &mut self, ctx: &Context, graph: &mut RenderGraph, allocator: &mut FrameAllocator, uploads: &[Upload],
    ) -> Result<(), GpuError> {
        if uploads.is_empty() {
            return Ok(());
        }

        let mut values: Vec<(TextureId, ValueId)> = Vec::new();
        let mut module = Module::default();

        for upload in uploads {
            let Some(texture) = self.textures.get(&upload.texture) else {
                continue;
            };

            let index = match values.iter().position(|(other, _)| *other == upload.texture) {
                Some(index) => index,
                None => {
                    // a patch lands over what is already there, so only a texture this upload
                    // created has nothing to preserve
                    let layout = match texture.uploaded {
                        true => TEXTURE_RESTING.into(),
                        false => vk::ImageLayout::UNDEFINED,
                    };
                    let value = module.import_attachment(&texture.attachment(layout));
                    module.set_name(value, format!("imgui texture {}", upload.texture.id()));
                    values.push((upload.texture, value));
                    values.len().saturating_sub(1)
                },
            };

            let staging = module.import_buffer(&upload.staging, Access::HostWrite);
            let Some(slot) = values.get_mut(index) else {
                continue;
            };
            slot.1 = module.copy_buffer_to_image_region(staging, slot.1, upload.region);
        }

        let roots = values
            .iter()
            .map(|(_, value)| module.release(*value, TEXTURE_RESTING, DomainFlag::Graphics))
            .collect::<Vec<_>>();
        let program = module.compile_all(&*graph, &roots)?;
        graph.execute_blocking(ctx, &program, &mut AllocatorKind::Frame(allocator))?;

        for (id, _) in &values {
            if let Some(texture) = self.textures.get_mut(id) {
                texture.uploaded = true;
            }
        }

        Ok(())
    }

    /// Work submitted for earlier frames may still be sampling these, so they go behind a wait.
    fn discard(&mut self, device: &mut Device, discarded: Vec<Texture>) -> Result<(), GpuError> {
        if discarded.is_empty() {
            return Ok(());
        }

        device.wait_idle()?;

        for texture in discarded {
            device.allocator.deallocate_image_view(texture.image_view);
            device.allocator.deallocate_image(texture.image);
        }

        Ok(())
    }
}

fn imgui_error(error: impl std::fmt::Display) -> GpuError { GpuError::ImGui(error.to_string()) }

/// `DrawVert` as the vertex buffer holds it. The shader reads the color as a `uint`, so the
/// packed layout is what goes over the wire.
#[repr(C)]
#[derive(Clone, Copy)]
struct DrawVertex {
    position: [f32; 2],
    uv: [f32; 2],
    color: u32,
}

fn cast_vertices(vertices: &[dear_imgui_rs::render::DrawVert]) -> &[DrawVertex] {
    // `DrawVert` is `#[repr(C)]` over the same three fields, and the layout is asserted below
    unsafe { std::slice::from_raw_parts(vertices.as_ptr().cast::<DrawVertex>(), vertices.len()) }
}

/// Clips `rect`, which imgui gives in `display_pos` space, to the framebuffer it is drawn into.
fn scissor(rect: [f32; 4], display_pos: [f32; 2], scale: [f32; 2], target: vk::Extent2D) -> vk::Rect2D {
    let width = target.width as f32;
    let height = target.height as f32;

    let min_x = ((rect[0] - display_pos[0]) * scale[0]).round().clamp(0.0, width) as u32;
    let min_y = ((rect[1] - display_pos[1]) * scale[1]).round().clamp(0.0, height) as u32;
    let max_x = ((rect[2] - display_pos[0]) * scale[0]).round().clamp(0.0, width) as u32;
    let max_y = ((rect[3] - display_pos[1]) * scale[1]).round().clamp(0.0, height) as u32;

    vk::Rect2D {
        offset: vk::Offset2D {
            x: min_x as i32,
            y: min_y as i32,
        },
        extent: vk::Extent2D {
            width: max_x.saturating_sub(min_x),
            height: max_y.saturating_sub(min_y),
        },
    }
}

fn stage(allocator: &mut FrameAllocator, pixels: &[u8]) -> Result<Buffer, GpuError> {
    let mut staging =
        allocator.allocate_buffer(&BufferInfo::staging(pixels.len() as u64).with_name("imgui staging"))?;
    staging.write(0, pixels)?;

    Ok(staging)
}

fn patch_upload(
    allocator: &mut FrameAllocator, texture: TextureId, format: TextureFormat, target: vk::Extent2D,
    patch: &TextureUploadRect,
) -> Result<Upload, GpuError> {
    let extent = validate_patch(target, patch)?;
    let pixels = rows(format, extent, patch.row_pitch, &patch.data)?;
    let staging = stage(allocator, &pixels)?;

    Ok(Upload {
        texture,
        staging,
        region: BufferImageCopy::region(
            vk::Offset2D {
                x: i32::from(patch.rect.x),
                y: i32::from(patch.rect.y),
            },
            extent,
        ),
    })
}

fn validate_patch(target: vk::Extent2D, patch: &TextureUploadRect) -> Result<vk::Extent2D, GpuError> {
    let extent = vk::Extent2D {
        width: u32::from(patch.rect.w),
        height: u32::from(patch.rect.h),
    };
    let right = u32::from(patch.rect.x)
        .checked_add(extent.width)
        .ok_or(GpuError::TextureUpdateMismatch)?;
    let bottom = u32::from(patch.rect.y)
        .checked_add(extent.height)
        .ok_or(GpuError::TextureUpdateMismatch)?;
    if right > target.width || bottom > target.height {
        return Err(GpuError::TextureUpdateMismatch);
    }

    Ok(extent)
}

/// imgui's uploads are not necessarily tightly packed, and `Alpha8` is coverage over white, so
/// both come out of here as the packed RGBA the images are.
fn rows(format: TextureFormat, extent: vk::Extent2D, row_pitch: usize, source: &[u8]) -> Result<Vec<u8>, GpuError> {
    let bytes = get_format_bytes_per_pixel(format);
    let width = usize::try_from(extent.width).map_err(|_| GpuError::TextureUploadTooLarge)?;
    let height = usize::try_from(extent.height).map_err(|_| GpuError::TextureUploadTooLarge)?;
    let packed = width.checked_mul(bytes).ok_or(GpuError::TextureUploadTooLarge)?;

    if row_pitch < packed {
        return Err(GpuError::TextureRowPitchTooSmall);
    }

    let output_len = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(GpuError::TextureUploadTooLarge)?;
    let mut out = Vec::with_capacity(output_len);

    for row in 0..height {
        let start = row.checked_mul(row_pitch).ok_or(GpuError::TextureUploadTooLarge)?;
        let end = start.checked_add(packed).ok_or(GpuError::TextureUploadTooLarge)?;
        let line = source.get(start..end).ok_or(GpuError::TextureRowPitchTooSmall)?;

        match format {
            TextureFormat::RGBA32 => out.extend_from_slice(line),
            TextureFormat::Alpha8 => {
                for coverage in line {
                    out.extend_from_slice(&[0xff, 0xff, 0xff, *coverage]);
                }
            },
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use dear_imgui_rs::{render::DrawVert, texture::TextureRect};
    use vir::{
        DescriptorBinding,
        VertexAttribute,
        VertexLayout,
        resource::{pipeline::push_constant_ranges, shader},
    };

    use super::*;

    fn reflect(spirv: &[u8]) -> shader::Reflection {
        shader::reflect(&read_spirv(spirv).expect("valid spirv")).expect("shader reflects")
    }

    /// The whole backend rests on this: `DrawVert` is `[f32;2], [f32;2], u32`, and reflection only
    /// derives 32-bit attribute formats, so the shader declares the packed color as a `uint`. That
    /// has to land on the same 20 bytes imgui uploads. A dear-imgui bump that changes `DrawVert`
    /// should fail here rather than render garbage.
    #[test]
    fn the_reflected_vertex_layout_matches_imgui() {
        let layout = VertexLayout::interleaved(&[reflect(VERTEX_SPIRV), reflect(FRAGMENT_SPIRV)]);

        assert_eq!(size_of::<DrawVertex>(), size_of::<DrawVert>());
        assert_eq!(layout.stride as usize, size_of::<DrawVert>());
        assert_eq!(
            layout.attributes,
            vec![
                VertexAttribute {
                    location: 0,
                    format: vk::Format::R32G32_SFLOAT,
                    offset: 0,
                },
                VertexAttribute {
                    location: 1,
                    format: vk::Format::R32G32_SFLOAT,
                    offset: 8,
                },
                VertexAttribute {
                    location: 2,
                    format: vk::Format::R32_UINT,
                    offset: 16,
                },
            ]
        );
    }

    /// Only the vertex stage reads the projection block.
    #[test]
    fn the_push_constant_range_covers_the_vertex_stage() {
        let ranges = push_constant_ranges(&[reflect(VERTEX_SPIRV), reflect(FRAGMENT_SPIRV)]);

        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].stage_flags, vk::ShaderStageFlags::VERTEX);
        assert_eq!(ranges[0].offset, 0);
        assert_eq!(ranges[0].size as usize, size_of::<PushConstants>());
    }

    #[test]
    fn the_fragment_shader_declares_one_texture() {
        assert_eq!(
            reflect(FRAGMENT_SPIRV).bindings,
            vec![DescriptorBinding {
                set: 0,
                binding: 0,
                descriptor_type: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
                count: 1,
                variable_count: false,
                stages: vk::ShaderStageFlags::FRAGMENT,
                access: Access::FragmentSampled,
            }]
        );
        assert!(reflect(VERTEX_SPIRV).bindings.is_empty());
    }

    #[test]
    fn a_clip_rect_is_taken_out_of_display_space_and_clamped() {
        let target = vk::Extent2D {
            width: 200,
            height: 100,
        };
        let clip = scissor([-20.0, 10.0, 60.0, 400.0], [10.0, 0.0], [2.0, 2.0], target);

        assert_eq!(clip.offset, vk::Offset2D { x: 0, y: 20 });
        assert_eq!(clip.extent, vk::Extent2D { width: 100, height: 80 });
    }

    #[test]
    fn a_clip_rect_outside_the_target_has_no_area() {
        let target = vk::Extent2D { width: 64, height: 64 };
        let clip = scissor([100.0, 100.0, 200.0, 200.0], [0.0, 0.0], [1.0, 1.0], target);

        assert_eq!(clip.extent.width, 0);
        assert_eq!(clip.extent.height, 0);
    }

    #[test]
    fn a_loose_row_pitch_is_packed_down() {
        let extent = vk::Extent2D { width: 2, height: 2 };
        let source = [
            1, 2, 3, 4, 5, 6, 7, 8, 0xff, 0xff, 9, 10, 11, 12, 13, 14, 15, 16, 0xff, 0xff,
        ];
        let packed = rows(TextureFormat::RGBA32, extent, 10, &source).expect("packs");

        assert_eq!(packed, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);
    }

    #[test]
    fn coverage_is_widened_to_white() {
        let extent = vk::Extent2D { width: 2, height: 1 };
        let packed = rows(TextureFormat::Alpha8, extent, 2, &[0x00, 0x80]).expect("packs");

        assert_eq!(packed, vec![0xff, 0xff, 0xff, 0x00, 0xff, 0xff, 0xff, 0x80]);
    }

    #[test]
    fn a_row_pitch_narrower_than_a_row_is_refused() {
        let extent = vk::Extent2D { width: 4, height: 1 };

        assert_eq!(
            rows(TextureFormat::RGBA32, extent, 8, &[0; 16]),
            Err(GpuError::TextureRowPitchTooSmall)
        );
    }

    #[test]
    fn a_texture_patch_must_fit_its_target() {
        let target = vk::Extent2D { width: 16, height: 8 };
        let patch = TextureUploadRect {
            rect: TextureRect {
                x: 12,
                y: 2,
                w: 5,
                h: 4,
            },
            row_pitch: 20,
            data: vec![0; 80],
        };

        assert_eq!(validate_patch(target, &patch), Err(GpuError::TextureUpdateMismatch));
    }

    #[test]
    fn a_texture_patch_may_touch_the_target_edge() {
        let target = vk::Extent2D { width: 16, height: 8 };
        let patch = TextureUploadRect {
            rect: TextureRect {
                x: 12,
                y: 4,
                w: 4,
                h: 4,
            },
            row_pitch: 16,
            data: vec![0; 64],
        };

        assert_eq!(validate_patch(target, &patch), Ok(vk::Extent2D { width: 4, height: 4 }));
    }
}
