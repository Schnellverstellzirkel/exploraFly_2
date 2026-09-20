//! Group-0 material descriptor layout and pool sizing (scene textures,
//! atmosphere LUTs, terrain cache, optional ray-query TLAS).

use ash::vk;

pub(super) fn material_descriptor_bindings(rt_supported: bool) -> Vec<vk::DescriptorSetLayoutBinding<'static>> {
    let mut bindings = vec![
        vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT),
        vk::DescriptorSetLayoutBinding::default()
            .binding(1)
            .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT),
        vk::DescriptorSetLayoutBinding::default()
            .binding(2)
            .descriptor_type(vk::DescriptorType::SAMPLER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT),
        vk::DescriptorSetLayoutBinding::default()
            .binding(3)
            .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT),
        vk::DescriptorSetLayoutBinding::default()
            .binding(4)
            .descriptor_type(vk::DescriptorType::SAMPLER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT),
    ];
    for (binding, descriptor_type) in [
        (6, vk::DescriptorType::SAMPLED_IMAGE),
        (7, vk::DescriptorType::SAMPLER),
        (8, vk::DescriptorType::SAMPLED_IMAGE),
        (9, vk::DescriptorType::SAMPLER),
    ] {
        bindings.push(vk::DescriptorSetLayoutBinding::default()
            .binding(binding)
            .descriptor_type(descriptor_type)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT));
    }
    for (binding, descriptor_type) in [
        (10, vk::DescriptorType::SAMPLED_IMAGE),
        (11, vk::DescriptorType::SAMPLER),
    ] {
        bindings.push(vk::DescriptorSetLayoutBinding::default()
            .binding(binding)
            .descriptor_type(descriptor_type)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::VERTEX));
    }
    bindings.push(vk::DescriptorSetLayoutBinding::default()
        .binding(21)
        .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
        .descriptor_count(1)
        .stage_flags(vk::ShaderStageFlags::VERTEX));
    bindings.push(vk::DescriptorSetLayoutBinding::default()
        .binding(22)
        .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
        .descriptor_count(1)
        .stage_flags(vk::ShaderStageFlags::VERTEX));
    // Photo detail textures: 4 diffuse + 4 normals, one shared sampler.
    for binding in crate::detail::DETAIL_DIFFUSE_BINDINGS
        .iter()
        .chain(crate::detail::DETAIL_NORMAL_BINDINGS.iter())
    {
        bindings.push(vk::DescriptorSetLayoutBinding::default()
            .binding(*binding)
            .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT));
    }
    bindings.push(vk::DescriptorSetLayoutBinding::default()
        .binding(crate::detail::DETAIL_SAMPLER_BINDING)
        .descriptor_type(vk::DescriptorType::SAMPLER)
        .descriptor_count(1)
        .stage_flags(vk::ShaderStageFlags::FRAGMENT));
    // The analytic shadow shaders do not declare binding 5, and the disabled
    // acceleration-structure extension cannot supply its descriptor type.
    if rt_supported {
        bindings.push(
            vk::DescriptorSetLayoutBinding::default()
                .binding(5)
                .descriptor_type(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
        );
    }
    bindings
}

pub(super) fn material_descriptor_pool_sizes(rt_supported: bool, sets: u32) -> Vec<vk::DescriptorPoolSize> {
    let mut sizes = vec![
        vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::UNIFORM_BUFFER).descriptor_count(sets),
        // Weave, energy LUT, two atmosphere LUTs, terrain, and 8 detail maps
        // each have a sampler; detail maps share one repeat sampler.
        vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::SAMPLED_IMAGE).descriptor_count(13 * sets),
        vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::SAMPLER).descriptor_count(6 * sets),
        vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::STORAGE_BUFFER).descriptor_count(2 * sets),
    ];
    if rt_supported {
        sizes.push(vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR).descriptor_count(sets));
    }
    sizes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plane::spirv::plane_frag_spv;

    #[test]
    fn material_descriptors_follow_rt_capability() {
        for rt_supported in [false, true] {
            let bindings = material_descriptor_bindings(rt_supported);
            let pool = material_descriptor_pool_sizes(rt_supported, 5);
            let acceleration_bindings: Vec<_> = bindings.iter()
                .filter(|b| b.descriptor_type == vk::DescriptorType::ACCELERATION_STRUCTURE_KHR)
                .collect();
            assert_eq!(acceleration_bindings.len(), usize::from(rt_supported));
            assert_eq!(pool.iter().filter(|p| p.ty == vk::DescriptorType::ACCELERATION_STRUCTURE_KHR).count(),
                usize::from(rt_supported));
            if rt_supported {
                assert_eq!(acceleration_bindings[0].binding, 5);
                assert_eq!(acceleration_bindings[0].descriptor_count, 1);
                assert_eq!(acceleration_bindings[0].stage_flags, vk::ShaderStageFlags::FRAGMENT);
            }
        }
    }

    #[test]
    fn material_descriptor_pool_covers_every_frame_set() {
        for rt_supported in [false, true] {
            let bindings = material_descriptor_bindings(rt_supported);
            let pool = material_descriptor_pool_sizes(rt_supported, 5);
            for (ty, expected) in [
                (vk::DescriptorType::UNIFORM_BUFFER, 5),
                (vk::DescriptorType::SAMPLED_IMAGE, 65),
                (vk::DescriptorType::SAMPLER, 30),
                (vk::DescriptorType::STORAGE_BUFFER, 10),
                (vk::DescriptorType::ACCELERATION_STRUCTURE_KHR, if rt_supported { 5 } else { 0 }),
            ] {
                let allocated: u32 = pool.iter().filter(|p| p.ty == ty).map(|p| p.descriptor_count).sum();
                assert_eq!(allocated, expected, "pool capacity for {ty:?}");
                let required: u32 = bindings.iter().filter(|b| b.descriptor_type == ty)
                    .map(|b| b.descriptor_count * 5).sum();
                assert!(allocated >= required, "insufficient capacity for {ty:?}");
            }
        }
    }

    #[test]
    fn material_quality_variants_compile() {
        for samples in [4, 8, 16, 32, 128] {
            assert!(!plane_frag_spv(samples).is_empty());
        }
    }

}
