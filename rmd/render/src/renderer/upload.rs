use std::ops::Range;

use ash::vk;
use vir::{
    Access, AllocatorKind, Buffer, BufferCopy, BufferImageCopy, BufferInfo, DomainFlag, FrameAllocator,
    ImageAttachment, ImageInfo, MemoryLocation, Module, RenderGraph, allocator::Allocator,
};

use super::{
    Renderer,
    sheets::{Sheet, SheetState, TextureImage},
};
use crate::{Device, GpuError};

pub(super) const UPLOAD_BATCH_BYTES: usize = 64 * 1024 * 1024;

pub(super) struct TextureSource<'a> {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) pixels: &'a [u8],
}

pub(super) fn upload_frame_buffer(
    allocator: &mut FrameAllocator, payload: &[u8], name: &str,
) -> Result<Buffer, GpuError> {
    let mut buffer = allocator.allocate_buffer(
        &BufferInfo::new(
            (payload.len() as u64).max(1),
            vk::BufferUsageFlags::STORAGE_BUFFER,
            MemoryLocation::CpuToGpu,
        )
        .with_name(name),
    )?;
    if !payload.is_empty() {
        buffer.write(0, payload)?;
    }

    Ok(buffer)
}

impl Renderer {
    pub(super) fn upload_images(&mut self, sources: &[TextureSource<'_>]) -> Result<Vec<TextureImage>, GpuError> {
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

pub(super) fn loaded_images(sheets: Vec<Sheet>) -> impl Iterator<Item = TextureImage> {
    sheets.into_iter().filter_map(|sheet| match sheet.state {
        SheetState::Loaded { image, .. } => Some(image),
        SheetState::Unloaded | SheetState::Failed => None,
    })
}

pub(super) fn destroy_images(device: &mut Device, textures: impl IntoIterator<Item = TextureImage>) {
    for texture in textures {
        device.allocator.deallocate_image_view(texture.view);
        device.allocator.deallocate_image(texture.image);
    }
}

pub(super) fn device_buffer_info(size: u64, name: &str) -> BufferInfo {
    BufferInfo::new(
        size,
        vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
        MemoryLocation::GpuOnly,
    )
    .with_name(name)
}

pub(super) fn upload_buffer(
    device: &mut Device, graph: &mut RenderGraph, target: Buffer, writes: &[(u64, &[u8])],
) -> Result<(), GpuError> {
    let total = writes.iter().map(|(_, bytes)| bytes.len() as u64).sum::<u64>();
    if total == 0 {
        return Ok(());
    }

    let mut staging = device
        .allocator
        .allocate_buffer(&BufferInfo::staging(total).with_name("buffer upload staging"))?;
    let result = (|| {
        let mut regions = Vec::with_capacity(writes.len());
        let mut src_offset = 0;
        for (dst_offset, bytes) in writes {
            staging.write(src_offset, bytes)?;
            regions.push(BufferCopy {
                src_offset,
                dst_offset: *dst_offset,
                size: bytes.len() as u64,
            });
            src_offset += bytes.len() as u64;
        }

        let mut module = Module::default();
        let source = module.import_buffer(&staging, Access::HostWrite);
        let destination = module.import_buffer(&target, Access::None);
        let copied = module.copy_buffer(source, destination, &regions);
        let exported = module.export(
            copied,
            Access::ComputeRead | Access::VertexRead | Access::FragmentRead,
            DomainFlag::Graphics,
        );
        let program = module.compile(&*graph, exported)?;
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
            let destination_attachment = module.import_attachment(
                &ImageAttachment::from_image(&texture.image, vk::ImageLayout::UNDEFINED),
                Access::None,
            );
            let copied_attachment = module.copy_buffer_to_image_region(
                source_buffer,
                destination_attachment,
                copy_region(offset, source.width, source.height),
            );
            roots.push(module.export(
                copied_attachment,
                Access::FragmentSampled | Access::ComputeSampled,
                DomainFlag::Graphics,
            ));
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

pub(super) fn batch_ranges(sizes: &[usize], budget: usize) -> Option<Vec<Range<usize>>> {
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
    use super::*;

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
