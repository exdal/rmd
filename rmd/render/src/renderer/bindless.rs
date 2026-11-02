use ash::vk;

use crate::{Device, GpuError};

pub(super) fn descriptor_capacity(requested: usize, limit: u32) -> Result<u32, GpuError> {
    let requested = u32::try_from(requested).map_err(|_| GpuError::TextureUploadTooLarge)?;
    if requested > limit {
        return Err(GpuError::TooManyTextures { requested, limit });
    }

    Ok(requested.max(1))
}

pub(super) struct BindlessDescriptorSet {
    pub(super) layout: vk::DescriptorSetLayout,
    pub(super) set: vk::DescriptorSet,
    pool: vk::DescriptorPool,
    pub(super) count: u32,
}

impl BindlessDescriptorSet {
    pub(super) fn create(device: &Device, count: u32) -> Result<Self, GpuError> {
        let vk_device = device.context.device();

        let binding = vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .descriptor_count(count)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT | vk::ShaderStageFlags::COMPUTE);
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

        Ok(Self {
            layout,
            set,
            pool,
            count,
        })
    }

    pub(super) fn destroy(&self, device: &Device) {
        let vk_device = device.context.device();
        unsafe {
            vk_device.destroy_descriptor_pool(self.pool, None);
            vk_device.destroy_descriptor_set_layout(self.layout, None);
        }
    }

    pub(super) fn write(&self, infos: &[vk::DescriptorImageInfo], device: &Device) {
        let vk_device = device.context.device();

        let write = vk::WriteDescriptorSet::default()
            .dst_set(self.set)
            .dst_binding(0)
            .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
            .image_info(infos);
        unsafe { vk_device.update_descriptor_sets(std::slice::from_ref(&write), &[]) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_capacity_matches_the_catalog_and_keeps_one_fallback_slot() {
        assert_eq!(descriptor_capacity(8, 10), Ok(8));
        assert_eq!(descriptor_capacity(0, 10), Ok(1));
        assert_eq!(
            descriptor_capacity(11, 10),
            Err(crate::GpuError::TooManyTextures {
                requested: 11,
                limit: 10,
            })
        );
    }
}
