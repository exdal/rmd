use std::ops::Range;

use ash::vk;
use render::texture::TextureCatalog;
use vir::{
    Access,
    AllocatorKind,
    BufferImageCopy,
    BufferInfo,
    DomainFlag,
    Image,
    ImageAttachment,
    ImageInfo,
    Module,
    RenderGraph,
    SamplerInfo,
    allocator::Allocator,
};

use crate::{device::Device, error::GpuError};

const UPLOAD_BATCH_BYTES: usize = 64 * 1024 * 1024;
const FORMAT: vk::Format = vk::Format::R8G8B8A8_SRGB;

pub struct TextureImage {
    pub image: Image,
    pub view: vk::ImageView,
}

pub struct GpuTextures {
    pub images: Vec<TextureImage>,
    pub sampler: vk::Sampler,
}

impl GpuTextures {
    pub fn destroy(&self, device: &mut Device) {
        for texture in &self.images {
            device.allocator.deallocate_image_view(texture.view);
            device.allocator.deallocate_image(texture.image);
        }
        device.allocator.deallocate_sampler(self.sampler);
    }
}

#[derive(Clone, Copy)]
struct TextureSource<'a> {
    width: u32,
    height: u32,
    pixels: &'a [u8],
}

pub fn upload(device: &mut Device, catalog: &TextureCatalog) -> Result<GpuTextures, GpuError> {
    let fallback = [0u8; 4];
    let sources = if catalog.is_empty() {
        vec![TextureSource {
            width: 1,
            height: 1,
            pixels: &fallback,
        }]
    } else {
        catalog
            .textures()
            .iter()
            .map(|texture| TextureSource {
                width: texture.width(),
                height: texture.height(),
                pixels: texture.pixels(),
            })
            .collect()
    };

    let sampler = device
        .allocator
        .allocate_sampler(&SamplerInfo::nearest().with_address_mode(vk::SamplerAddressMode::CLAMP_TO_EDGE))?;
    let mut textures = GpuTextures {
        images: Vec::with_capacity(sources.len()),
        sampler,
    };

    let result = allocate_images(device, &sources, &mut textures).and_then(|()| {
        let mut upload_graph = RenderGraph::new(&device.context);

        upload_images(device, &mut upload_graph, &sources, &textures)
    });
    if let Err(error) = result {
        textures.destroy(device);
        return Err(error);
    }

    Ok(textures)
}

fn allocate_images(
    device: &mut Device, sources: &[TextureSource<'_>], textures: &mut GpuTextures,
) -> Result<(), GpuError> {
    for (index, source) in sources.iter().enumerate() {
        let extent = vk::Extent2D {
            width: source.width,
            height: source.height,
        };
        let image = device
            .allocator
            .allocate_image(&ImageInfo::texture(extent, FORMAT).with_name(format!("sprite texture {index}")))?;

        let view = match device.allocator.allocate_image_view(
            image.handle(),
            FORMAT,
            vk::ImageViewType::TYPE_2D,
            image.subresource_range(),
        ) {
            Ok(view) => view,
            Err(error) => {
                device.allocator.deallocate_image(image);
                return Err(error.into());
            },
        };

        textures.images.push(TextureImage { image, view });
    }

    Ok(())
}

fn upload_images(
    device: &mut Device, graph: &mut RenderGraph, sources: &[TextureSource<'_>], textures: &GpuTextures,
) -> Result<(), GpuError> {
    let sizes = sources.iter().map(|source| source.pixels.len()).collect::<Vec<_>>();
    let ranges = batch_ranges(&sizes, UPLOAD_BATCH_BYTES).ok_or(GpuError::TextureUploadTooLarge)?;

    for range in ranges {
        let Some(batch_sources) = sources.get(range.clone()) else {
            return Err(GpuError::TextureUploadTooLarge);
        };
        let Some(batch_textures) = textures.images.get(range) else {
            return Err(GpuError::TextureUploadTooLarge);
        };
        upload_batch(device, graph, batch_sources, batch_textures)?;
    }

    Ok(())
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
