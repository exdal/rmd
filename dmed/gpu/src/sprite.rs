use ash::vk;
use render::{Frame, SpriteInstance, texture::TextureCatalog};
use vir::{
    Access,
    BlendPreset,
    Buffer,
    BufferInfo,
    GraphicsPipelineInfo,
    MemoryLocation,
    Module,
    PipelineId,
    Program,
    RasterizationState,
    Rect2D,
    RenderGraph,
    ValueId,
    allocator::Allocator,
};

use crate::{
    device::Device,
    error::GpuError,
    read_spirv,
    resources::BindlessDescriptorSet,
    upload::{self, GpuTextures},
};

const VERTEX_SPIRV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sprite.vert.spv"));
const FRAGMENT_SPIRV: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/sprite.frag.spv"));

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct GpuSprite {
    position_size: [f32; 4],
    color: [f32; 4],
    texture: u32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
struct CameraPush {
    center: [f32; 2],
    viewport: [f32; 2],
    zoom: f32,
}

pub struct SpriteSlots {
    camera: ValueId,
    count: ValueId,
    sprites: ValueId,
}

#[derive(Default)]
pub struct SpritePass {
    pipeline: Option<PipelineId>,
    textures: Option<GpuTextures>,
    bindless: Option<BindlessDescriptorSet>,
    sprites: Option<Buffer>,
    sprite_capacity: usize,
    uploaded_revision: Option<u64>,
}

impl SpritePass {
    pub fn new() -> Self { Self::default() }

    pub fn is_ready(&self) -> bool { self.pipeline.is_some() }

    pub fn upload_textures(&mut self, device: &mut Device, catalog: &TextureCatalog) -> Result<(), GpuError> {
        self.destroy_textures(device);

        let uploaded = upload::upload(device, catalog)?;
        let bindless = match BindlessDescriptorSet::create(device, &uploaded) {
            Ok(bindless) => bindless,
            Err(error) => {
                uploaded.destroy(device);
                return Err(error);
            },
        };

        self.textures = Some(uploaded);
        self.bindless = Some(bindless);

        Ok(())
    }

    pub fn declare_pipeline(&mut self, graph: &mut RenderGraph) -> Result<(), GpuError> {
        self.pipeline = None;

        let Some(bindless) = self.bindless.as_ref() else {
            return Ok(());
        };

        let vertex = read_spirv(VERTEX_SPIRV)?;
        let fragment = read_spirv(FRAGMENT_SPIRV)?;
        let pipeline = graph.declare_pipeline(
            GraphicsPipelineInfo::new()
                .with_shader(&vertex)
                .with_shader(&fragment)
                .with_bindless_set(1, bindless.layout, bindless.set),
        )?;

        self.pipeline = Some(pipeline);

        Ok(())
    }

    pub fn record(&mut self, module: &mut Module, target: ValueId) -> (ValueId, Option<SpriteSlots>) {
        let Some(pipeline) = self.pipeline else {
            return (target, None);
        };

        self.uploaded_revision = None;

        let camera = module.declare_bytes_var("camera", size_of::<CameraPush>() as u32);
        let count = module.declare_u32_var("sprite count", 0);
        let sprites = module.declare_buffer_var("sprites", Access::HostWrite);

        let drawn = module
            .begin_rendering([(target, Access::ColorRW)])
            .with_name("sprites")
            .bind_graphics_pipeline(pipeline)
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
            .draw(4u32, count)
            .end_rendering();

        (drawn, Some(SpriteSlots { camera, count, sprites }))
    }

    pub fn prepare(&mut self, device: &mut Device, frame: &Frame) -> Result<(), GpuError> {
        let reallocated = self.ensure_sprite_buffer(device, frame.sprites.len())?;

        if reallocated || self.uploaded_revision != Some(frame.revision) {
            self.write_sprites(&frame.sprites)?;
            self.uploaded_revision = Some(frame.revision);
        }

        Ok(())
    }

    pub fn bind(&self, program: &mut Program, slots: &SpriteSlots, frame: &Frame, target: vk::Extent2D) {
        let Some(buffer) = self.sprites.as_ref() else {
            return;
        };

        let push = CameraPush {
            center: [frame.camera.x, frame.camera.y],
            viewport: [target.width as f32, target.height as f32],
            zoom: frame.camera.zoom,
        };

        program.set_bytes(slots.camera, &push);
        program.set(slots.count, frame.sprites.len() as u32);
        program.set(slots.sprites, buffer);
    }

    pub fn destroy(&mut self, device: &mut Device) {
        self.pipeline = None;
        self.destroy_textures(device);

        if let Some(buffer) = self.sprites.take() {
            device.allocator.deallocate_buffer(buffer);
        }
        self.sprite_capacity = 0;
        self.uploaded_revision = None;
    }

    fn destroy_textures(&mut self, device: &mut Device) {
        self.pipeline = None;

        if let Some(bindless) = self.bindless.take() {
            bindless.destroy(device);
        }
        if let Some(textures) = self.textures.take() {
            textures.destroy(device);
        }
    }

    fn ensure_sprite_buffer(&mut self, device: &mut Device, count: usize) -> Result<bool, GpuError> {
        if self.sprites.is_some() && self.sprite_capacity >= count {
            return Ok(false);
        }

        if let Some(old) = self.sprites.take() {
            device.allocator.deallocate_buffer(old);
        }

        let capacity = count.max(1024).next_power_of_two();
        let size = (capacity.saturating_mul(size_of::<GpuSprite>())) as u64;

        self.sprites = Some(device.allocator.allocate_buffer(
            &BufferInfo::new(size, vk::BufferUsageFlags::STORAGE_BUFFER, MemoryLocation::CpuToGpu).with_name("sprites"),
        )?);
        self.sprite_capacity = capacity;

        Ok(true)
    }

    fn write_sprites(&mut self, sprites: &[SpriteInstance]) -> Result<(), GpuError> {
        let payload = sprites
            .iter()
            .map(|sprite| GpuSprite {
                position_size: [
                    sprite.x,
                    sprite.y,
                    sprite.texture.width as f32,
                    sprite.texture.height as f32,
                ],
                color: sprite.color,
                texture: sprite.texture.index,
            })
            .collect::<Vec<_>>();

        if let Some(buffer) = self.sprites.as_mut() {
            buffer.write(0, &payload)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use vir::{VertexAttribute, VertexLayout, resource::shader};

    use super::*;

    #[test]
    fn the_pushed_camera_block_matches_reflection() {
        let reflection = shader::reflect(&read_spirv(VERTEX_SPIRV).expect("valid spirv")).expect("shader reflects");

        assert_eq!(reflection.push_constant_offset, 0);
        assert_eq!(reflection.push_constant_size as usize, size_of::<CameraPush>());
    }

    /// The quad comes from `SV_VertexID` and the instance from `SV_InstanceID`, so reflection must
    /// derive no vertex bindings at all.
    #[test]
    fn sprites_are_drawn_without_a_vertex_buffer() {
        let reflections = [
            shader::reflect(&read_spirv(VERTEX_SPIRV).expect("valid spirv")).expect("shader reflects"),
            shader::reflect(&read_spirv(FRAGMENT_SPIRV).expect("valid spirv")).expect("shader reflects"),
        ];
        let layout = VertexLayout::interleaved(&reflections);

        assert_eq!(layout.stride, 0);
        assert_eq!(layout.attributes, Vec::<VertexAttribute>::new());
    }
}
