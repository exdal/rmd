use ash::vk;

use crate::{device::Device, error::GpuError, upload::GpuTextures};

pub struct BindlessDescriptorSet {
    pub layout: vk::DescriptorSetLayout,
    pub set: vk::DescriptorSet,
    pool: vk::DescriptorPool,
}

impl BindlessDescriptorSet {
    pub fn create(device: &Device, textures: &GpuTextures) -> Result<Self, GpuError> {
        let count = u32::try_from(textures.images.len()).map_err(|_| GpuError::TooManyTextures {
            requested: u32::MAX,
            limit: device.max_bindless_textures,
        })?;
        if count > device.max_bindless_textures {
            return Err(GpuError::TooManyTextures {
                requested: count,
                limit: device.max_bindless_textures,
            });
        }

        let binding = vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .descriptor_count(count)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT);
        let bindings = [binding];
        let layout_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
        let vk_device = device.context.device();
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

        let image_infos = textures
            .images
            .iter()
            .map(|texture| {
                vk::DescriptorImageInfo::default()
                    .sampler(textures.sampler)
                    .image_view(texture.view)
                    .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            })
            .collect::<Vec<_>>();
        let write = vk::WriteDescriptorSet::default()
            .dst_set(set)
            .dst_binding(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .image_info(&image_infos);
        unsafe { vk_device.update_descriptor_sets(std::slice::from_ref(&write), &[]) };

        Ok(Self { layout, set, pool })
    }

    pub fn destroy(&self, device: &Device) {
        let vk_device = device.context.device();
        unsafe {
            vk_device.destroy_descriptor_pool(self.pool, None);
            vk_device.destroy_descriptor_set_layout(self.layout, None);
        }
    }
}
