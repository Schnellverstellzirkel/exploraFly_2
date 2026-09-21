//! Group-0 material descriptor layout and pool sizing (scene textures,
//! atmosphere LUTs, terrain cache, optional ray-query TLAS, optional mesh
//! hierarchy SSBOs).

use ash::vk;

pub(super) const MESH_MESHLET_BINDING: u32 = 25;
pub(super) const MESH_PART_BINDING: u32 = 26;
pub(super) const MESH_LOD_BINDING: u32 = 27;
pub(super) const MESH_VERT_INDEX_BINDING: u32 = 28;
pub(super) const MESH_TRI_BINDING: u32 = 29;
pub(super) const MESH_VERTEX_STREAM_BINDING: u32 = 30;

pub(super) fn material_descriptor_bindings(
    rt_supported: bool,
    mesh_shaders: bool,
) -> Vec<vk::DescriptorSetLayoutBinding<'static>> {
    let ubo_stages = vk::ShaderStageFlags::VERTEX
        | vk::ShaderStageFlags::FRAGMENT
        | vk::ShaderStageFlags::COMPUTE
        | if mesh_shaders {
            vk::ShaderStageFlags::TASK_EXT | vk::ShaderStageFlags::MESH_EXT
        } else {
            vk::ShaderStageFlags::empty()
        };
    let mut bindings = vec![
        vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(1)
            .stage_flags(ubo_stages),
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
        .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::COMPUTE));
    bindings.push(vk::DescriptorSetLayoutBinding::default()
        .binding(22)
        .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
        .descriptor_count(1)
        .stage_flags(vk::ShaderStageFlags::COMPUTE));
    bindings.push(vk::DescriptorSetLayoutBinding::default()
        .binding(23)
        .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
        .descriptor_count(1)
        .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::COMPUTE));
    bindings.push(vk::DescriptorSetLayoutBinding::default()
        .binding(24)
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
    if mesh_shaders {
        let task_mesh = vk::ShaderStageFlags::TASK_EXT | vk::ShaderStageFlags::MESH_EXT;
        for (binding, stages) in [
            (MESH_MESHLET_BINDING, task_mesh),
            (MESH_PART_BINDING, task_mesh),
            (MESH_LOD_BINDING, vk::ShaderStageFlags::TASK_EXT),
            (MESH_VERT_INDEX_BINDING, vk::ShaderStageFlags::MESH_EXT),
            (MESH_TRI_BINDING, vk::ShaderStageFlags::MESH_EXT),
            (MESH_VERTEX_STREAM_BINDING, vk::ShaderStageFlags::MESH_EXT),
        ] {
            bindings.push(
                vk::DescriptorSetLayoutBinding::default()
                    .binding(binding)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .descriptor_count(1)
                    .stage_flags(stages),
            );
        }
    }
    bindings
}

pub(super) fn material_descriptor_pool_sizes(
    rt_supported: bool,
    mesh_shaders: bool,
    sets: u32,
) -> Vec<vk::DescriptorPoolSize> {
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
            .ty(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count((4 + if mesh_shaders { 6 } else { 0 }) * sets),
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
    fn material_descriptors_follow_rt_and_mesh_capability() {
        for rt_supported in [false, true] {
            for mesh_shaders in [false, true] {
                let bindings = material_descriptor_bindings(rt_supported, mesh_shaders);
                let pool = material_descriptor_pool_sizes(rt_supported, mesh_shaders, 5);
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
                let mesh_bindings: Vec<_> = bindings.iter()
                    .filter(|b| (MESH_MESHLET_BINDING..=MESH_VERTEX_STREAM_BINDING).contains(&b.binding))
                    .collect();
                assert_eq!(mesh_bindings.len(), if mesh_shaders { 6 } else { 0 });
                if mesh_shaders {
                    let ubo = bindings.iter().find(|b| b.binding == 0).unwrap();
                    assert!(ubo.stage_flags.contains(vk::ShaderStageFlags::TASK_EXT));
                    assert!(ubo.stage_flags.contains(vk::ShaderStageFlags::MESH_EXT));
                }
            }
        }
    }

    #[test]
    fn material_descriptor_pool_covers_every_frame_set() {
        for rt_supported in [false, true] {
            for mesh_shaders in [false, true] {
                let bindings = material_descriptor_bindings(rt_supported, mesh_shaders);
                let pool = material_descriptor_pool_sizes(rt_supported, mesh_shaders, 5);
                let storage = if mesh_shaders { 50 } else { 20 };
                for (ty, expected) in [
                    (vk::DescriptorType::UNIFORM_BUFFER, 5),
                    (vk::DescriptorType::SAMPLED_IMAGE, 65),
                    (vk::DescriptorType::SAMPLER, 30),
                    (vk::DescriptorType::STORAGE_BUFFER, storage),
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
    }

    #[test]
    fn material_quality_variants_compile() {
        for samples in [4, 8, 16, 32, 128] {
            assert!(!plane_frag_spv(samples).is_empty());
        }
    }

}
