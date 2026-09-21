//! Per-swapchain-slot resources and the pre-recorded command buffers.
//! Frames are recorded once per slot; the hot loop only copies uniforms
//! and submits. All per-image allocations (descriptor pools, UBO/FX
//! buffers, cone and trail rings, TLAS instance memory) live here.

use ash::vk;
use glam::Vec3;

use crate::ubo::*;
use super::Plane;
use super::{
    FRAME_BYTES, GPU_STAMPS_PER_FRAME, RT_INSTANCE_BYTES, TERRAIN_COMMAND_BYTES,
    CANOPY_COMMAND_BYTES, CANOPY_COMMAND_COUNT, CANOPY_COMMAND_OFFSET,
    VEGETATION_COMMAND_BYTES, VEGETATION_COMMAND_COUNT, VEGETATION_COMMAND_OFFSET,
    VEGETATION_OUTPUT_BYTES, VEGETATION_OUTPUT_COMMAND_BYTES,
};
use super::descriptors::material_descriptor_pool_sizes;

impl Plane {
    pub unsafe fn build_frames(
        &mut self,
        device: &ash::Device,
        instance: &ash::Instance,
        physical: vk::PhysicalDevice,
        queue_family: u32,
        queue: vk::Queue,
        images: usize,
    ) {
        let pool_sizes = material_descriptor_pool_sizes(self.rt_supported, images as u32);
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .pool_sizes(&pool_sizes)
            .max_sets(images as u32);
        self.descriptor_pool = device
            .create_descriptor_pool(&pool_info, None)
            .expect("ppool");
        let layouts = vec![self.set_layout; images];
        let alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(self.descriptor_pool)
            .set_layouts(&layouts);
        let sets = device.allocate_descriptor_sets(&alloc_info).expect("psets");
        let mem_props = instance.get_physical_device_memory_properties(physical);
        let upload = |size: u64, usage: vk::BufferUsageFlags| {
            let info = vk::BufferCreateInfo::default()
                .size(size)
                .usage(usage)
                .sharing_mode(vk::SharingMode::EXCLUSIVE);
            let buffer = device.create_buffer(&info, None).expect("fbuf");
            let req = device.get_buffer_memory_requirements(buffer);
            let index = crate::find_memory_type(
                &mem_props,
                req.memory_type_bits,
                vk::MemoryPropertyFlags::DEVICE_LOCAL,
            );
            let alloc = vk::MemoryAllocateInfo::default()
                .allocation_size(req.size)
                .memory_type_index(index);
            let memory = if usage.contains(vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS) {
                let mut addr_flags = vk::MemoryAllocateFlagsInfo::default()
                    .flags(vk::MemoryAllocateFlags::DEVICE_ADDRESS);
                device
                    .allocate_memory(&alloc.push_next(&mut addr_flags), None)
                    .expect("fmem")
            } else {
                device.allocate_memory(&alloc, None).expect("fmem")
            };
            device.bind_buffer_memory(buffer, memory, 0).expect("fbind");
            (buffer, memory)
        };
        for set in sets {
            let buffer_info = vk::BufferCreateInfo::default()
                .size(FRAME_BYTES as u64)
                .usage(vk::BufferUsageFlags::UNIFORM_BUFFER | vk::BufferUsageFlags::INDIRECT_BUFFER)
                .sharing_mode(vk::SharingMode::EXCLUSIVE);
            let buffer = device.create_buffer(&buffer_info, None).expect("pubo");
            let req = device.get_buffer_memory_requirements(buffer);
            let index = crate::find_memory_type(
                &mem_props,
                req.memory_type_bits,
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            );
            let alloc = vk::MemoryAllocateInfo::default()
                .allocation_size(req.size)
                .memory_type_index(index);
            let memory = device.allocate_memory(&alloc, None).expect("pmem");
            device.bind_buffer_memory(buffer, memory, 0).expect("pbind");
            let mapped = device
                .map_memory(memory, 0, FRAME_BYTES as u64, vk::MemoryMapFlags::empty())
                .expect("pmap") as *mut u8;
            let _ = libc::mlock(mapped as *const libc::c_void, FRAME_BYTES);
            let buffer_ref = [vk::DescriptorBufferInfo::default()
                .buffer(buffer)
                .offset(0)
                .range(UBO_BYTES as u64)];
            let write_ubo = [vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .buffer_info(&buffer_ref)];
            let image_ref = [vk::DescriptorImageInfo::default()
                .image_view(self.weave_view)
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
            let write_tex = [vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(1)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(&image_ref)];
            let sampler_ref = [vk::DescriptorImageInfo::default().sampler(self.weave_sampler)];
            let write_smp = [vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(2)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .image_info(&sampler_ref)];
            let lut_ref = [vk::DescriptorImageInfo::default()
                .image_view(self.lut_view)
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
            let write_lut = [vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(3)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(&lut_ref)];
            let lut_sampler_ref =
                [vk::DescriptorImageInfo::default().sampler(self.lut_sampler)];
            let write_lut_smp = [vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(4)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .image_info(&lut_sampler_ref)];
            let atmo_transmittance_ref = [vk::DescriptorImageInfo::default()
                .image_view(self.atmo_transmittance_view)
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
            let write_atmo_transmittance = [vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(6)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(&atmo_transmittance_ref)];
            let atmo_sampler_ref = [vk::DescriptorImageInfo::default().sampler(self.atmo_sampler)];
            let write_atmo_transmittance_smp = [vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(7)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .image_info(&atmo_sampler_ref)];
            let atmo_multiscattering_ref = [vk::DescriptorImageInfo::default()
                .image_view(self.atmo_multiscattering_view)
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
            let write_atmo_multiscattering = [vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(8)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(&atmo_multiscattering_ref)];
            let write_atmo_multiscattering_smp = [vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(9)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .image_info(&atmo_sampler_ref)];
            device.update_descriptor_sets(&write_ubo, &[]);
            device.update_descriptor_sets(&write_tex, &[]);
            device.update_descriptor_sets(&write_smp, &[]);
            device.update_descriptor_sets(&write_lut, &[]);
            device.update_descriptor_sets(&write_lut_smp, &[]);
            device.update_descriptor_sets(&write_atmo_transmittance, &[]);
            device.update_descriptor_sets(&write_atmo_transmittance_smp, &[]);
            device.update_descriptor_sets(&write_atmo_multiscattering, &[]);
            device.update_descriptor_sets(&write_atmo_multiscattering_smp, &[]);
            let terrain_ref = [vk::DescriptorImageInfo::default()
                .image_view(self.terrain_view)
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
            device.update_descriptor_sets(&[
                vk::WriteDescriptorSet::default()
                    .dst_set(set).dst_binding(10)
                    .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                    .image_info(&terrain_ref),
                vk::WriteDescriptorSet::default()
                    .dst_set(set).dst_binding(11)
                    .descriptor_type(vk::DescriptorType::SAMPLER)
                    .image_info(&atmo_sampler_ref),
            ], &[]);
            let vegetation_ref = [vk::DescriptorBufferInfo::default()
                .buffer(self.vegetation_buffer)
                .offset(0)
                .range(vk::WHOLE_SIZE)];
            let canopy_ref = [vk::DescriptorBufferInfo::default()
                .buffer(self.canopy_buffer)
                .offset(0)
                .range(vk::WHOLE_SIZE)];
            let vegetation_cells_ref = [vk::DescriptorBufferInfo::default()
                .buffer(self.vegetation_cells_buffer)
                .offset(0)
                .range(vk::WHOLE_SIZE)];
            let (vegetation_output_buffer, vegetation_output_memory) = upload(
                VEGETATION_OUTPUT_BYTES as u64,
                vk::BufferUsageFlags::STORAGE_BUFFER
                    | vk::BufferUsageFlags::INDIRECT_BUFFER
                    | vk::BufferUsageFlags::TRANSFER_DST,
            );
            let vegetation_output_ref = [vk::DescriptorBufferInfo::default()
                .buffer(vegetation_output_buffer)
                .offset(0)
                .range(vk::WHOLE_SIZE)];
            device.update_descriptor_sets(&[
                vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(21)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .buffer_info(&vegetation_ref),
                vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(22)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .buffer_info(&vegetation_cells_ref),
                vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(23)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .buffer_info(&vegetation_output_ref),
                vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(24)
                    .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                    .buffer_info(&canopy_ref),
            ], &[]);
            // Photo detail maps share one repeat/anisotropic sampler.
            let detail_sampler_ref =
                [vk::DescriptorImageInfo::default().sampler(self.detail_textures.sampler)];
            let mut detail_writes = Vec::with_capacity(9);
            for (slot, view) in self.detail_textures.views.iter().enumerate() {
                let binding = if slot < crate::detail::DETAIL_COUNT {
                    crate::detail::DETAIL_DIFFUSE_BINDINGS[slot]
                } else {
                    crate::detail::DETAIL_NORMAL_BINDINGS[slot - crate::detail::DETAIL_COUNT]
                };
                let info = vk::DescriptorImageInfo::default()
                    .image_view(*view)
                    .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
                detail_writes.push((info, binding));
            }
            for (info, binding) in &detail_writes {
                device.update_descriptor_sets(&[
                    vk::WriteDescriptorSet::default()
                        .dst_set(set).dst_binding(*binding)
                        .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                        .image_info(std::slice::from_ref(info)),
                ], &[]);
            }
            device.update_descriptor_sets(&[
                vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(crate::detail::DETAIL_SAMPLER_BINDING)
                    .descriptor_type(vk::DescriptorType::SAMPLER)
                    .image_info(&detail_sampler_ref),
            ], &[]);
            // Ray-traced shadows: per-slot top-level structure + host-written
            // instance transforms. Size it for updates and build the initial
            // identity pose before the reusable hot command buffer is recorded.
            if self.rt_supported && self.rt_instance_count > 0 {
                let inst_bytes = (self.rt_instance_count as usize) * RT_INSTANCE_BYTES;
                let inst_info = vk::BufferCreateInfo::default()
                    .size(inst_bytes as u64)
                    .usage(
                        vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
                            | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR,
                    )
                    .sharing_mode(vk::SharingMode::EXCLUSIVE);
                let inst_buffer = device.create_buffer(&inst_info, None).expect("rtinstbuf");
                let inst_req = device.get_buffer_memory_requirements(inst_buffer);
                let inst_index = crate::find_memory_type(
                    &mem_props,
                    inst_req.memory_type_bits,
                    vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
                );
                let inst_alloc = vk::MemoryAllocateInfo::default()
                    .allocation_size(inst_req.size)
                    .memory_type_index(inst_index);
                let inst_memory = {
                    let mut addr_flags = vk::MemoryAllocateFlagsInfo::default()
                        .flags(vk::MemoryAllocateFlags::DEVICE_ADDRESS);
                    device
                        .allocate_memory(&inst_alloc.push_next(&mut addr_flags), None)
                        .expect("rtinstmem")
                };
                device.bind_buffer_memory(inst_buffer, inst_memory, 0).expect("rtinstbind");
                let inst_map = device
                    .map_memory(inst_memory, 0, inst_bytes as u64, vk::MemoryMapFlags::empty())
                    .expect("rtinstmap") as *mut u8;
                let inst_address = device
                    .get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(inst_buffer));
                let instances_geom = vk::AccelerationStructureGeometryInstancesDataKHR::default()
                    .array_of_pointers(false)
                    .data(vk::DeviceOrHostAddressConstKHR {
                        device_address: inst_address,
                    });
                let tlas_geometry = vk::AccelerationStructureGeometryKHR::default()
                    .geometry_type(vk::GeometryTypeKHR::INSTANCES)
                    .geometry(vk::AccelerationStructureGeometryDataKHR {
                        instances: instances_geom,
                    });
                let mut tlas_size = vk::AccelerationStructureBuildSizesInfoKHR::default();
                let tlas_build_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
                    .ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL)
                    .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
                    .flags(
                        vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE
                            | vk::BuildAccelerationStructureFlagsKHR::ALLOW_UPDATE,
                    )
                    .geometries(std::slice::from_ref(&tlas_geometry));
                self.rt_loader.get_acceleration_structure_build_sizes(
                    vk::AccelerationStructureBuildTypeKHR::DEVICE,
                    &tlas_build_info,
                    &[self.rt_instance_count],
                    &mut tlas_size,
                );
                let (tlas_buffer, tlas_memory) = upload(
                    tlas_size.acceleration_structure_size,
                    vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR
                        | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
                );
                let (tlas_scratch, tlas_scratch_memory) = upload(
                    tlas_size.build_scratch_size,
                    vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
                );
                let tlas_create = vk::AccelerationStructureCreateInfoKHR::default()
                    .create_flags(vk::AccelerationStructureCreateFlagsKHR::empty())
                    .buffer(tlas_buffer)
                    .offset(0)
                    .size(tlas_size.acceleration_structure_size)
                    .ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL);
                let tlas = self
                    .rt_loader
                    .create_acceleration_structure(&tlas_create, None)
                    .expect("tlas");
                let tlas_ref = [tlas];
                let mut rt_as_write = vk::WriteDescriptorSetAccelerationStructureKHR::default()
                    .acceleration_structures(&tlas_ref);
                let write_rt_as = vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(5)
                    .descriptor_count(1)
                    .descriptor_type(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR)
                    .push_next(&mut rt_as_write);
                device.update_descriptor_sets(&[write_rt_as], &[]);
                self.rt_instance_buffers.push(inst_buffer);
                self.rt_instance_memories.push(inst_memory);
                self.rt_instance_mapped.push(inst_map);
                self.rt_tlas.push(tlas);
                self.rt_tlas_buffers.push(tlas_buffer);
                self.rt_tlas_memories.push(tlas_memory);
                self.rt_scratch.push(tlas_scratch);
                self.rt_scratch_memories.push(tlas_scratch_memory);
            } else {
                self.rt_instance_buffers.push(vk::Buffer::null());
                self.rt_instance_memories.push(vk::DeviceMemory::null());
                self.rt_instance_mapped.push(std::ptr::null_mut());
                self.rt_tlas.push(vk::AccelerationStructureKHR::null());
                self.rt_tlas_buffers.push(vk::Buffer::null());
                self.rt_tlas_memories.push(vk::DeviceMemory::null());
                self.rt_scratch.push(vk::Buffer::null());
                self.rt_scratch_memories.push(vk::DeviceMemory::null());
            }
            if self.rt_supported && self.rt_instance_count > 0 {
                let rt_slot = self.rt_tlas.len() - 1;
                self.write_initial_rt_instances(rt_slot);
                self.build_initial_rt_tlas(device, queue_family, queue, rt_slot);
            }
            self.ubo_buffers.push(buffer);
            self.ubo_memories.push(memory);
            self.ubo_mapped.push(mapped);
            self.ubo_sets.push(set);
            self.vegetation_output_buffers.push(vegetation_output_buffer);
            self.vegetation_output_memories.push(vegetation_output_memory);
        }
        // FX sets (group 1): noise volumes + curl warp shared across frames.
        let fx_pool_sizes = [
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(images as u32 * 3),
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::SAMPLER)
                .descriptor_count(images as u32 * 3),
        ];
        self.fx_pool = device
            .create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .pool_sizes(&fx_pool_sizes)
                    .max_sets(images as u32),
                None,
            )
            .expect("fxpool");
        let fx_layouts = vec![self.fx_layout; images];
        self.fx_sets = device
            .allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(self.fx_pool)
                    .set_layouts(&fx_layouts),
            )
            .expect("fxsets");
        for set in &self.fx_sets {
            let base_ref = [vk::DescriptorImageInfo::default()
                .image_view(self.noise_base_view)
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
            let base_smp = [vk::DescriptorImageInfo::default().sampler(self.noise_base_sampler)];
            let det_ref = [vk::DescriptorImageInfo::default()
                .image_view(self.noise_detail_view)
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
            let det_smp =
                [vk::DescriptorImageInfo::default().sampler(self.noise_detail_sampler)];
            let curl_ref = [vk::DescriptorImageInfo::default()
                .image_view(self.noise_curl_view)
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
            let curl_smp =
                [vk::DescriptorImageInfo::default().sampler(self.noise_curl_sampler)];
            device.update_descriptor_sets(
                &[vk::WriteDescriptorSet::default()
                    .dst_set(*set)
                    .dst_binding(0)
                    .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                    .image_info(&base_ref)],
                &[],
            );
            device.update_descriptor_sets(
                &[vk::WriteDescriptorSet::default()
                    .dst_set(*set)
                    .dst_binding(1)
                    .descriptor_type(vk::DescriptorType::SAMPLER)
                    .image_info(&base_smp)],
                &[],
            );
            device.update_descriptor_sets(
                &[vk::WriteDescriptorSet::default()
                    .dst_set(*set)
                    .dst_binding(2)
                    .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                    .image_info(&det_ref)],
                &[],
            );
            device.update_descriptor_sets(
                &[vk::WriteDescriptorSet::default()
                    .dst_set(*set)
                    .dst_binding(3)
                    .descriptor_type(vk::DescriptorType::SAMPLER)
                    .image_info(&det_smp)],
                &[],
            );
            device.update_descriptor_sets(
                &[vk::WriteDescriptorSet::default()
                    .dst_set(*set)
                    .dst_binding(4)
                    .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                    .image_info(&curl_ref)],
                &[],
            );
            device.update_descriptor_sets(
                &[vk::WriteDescriptorSet::default()
                    .dst_set(*set)
                    .dst_binding(5)
                    .descriptor_type(vk::DescriptorType::SAMPLER)
                    .image_info(&curl_smp)],
                &[],
            );
        }
        // Per-frame host-visible cone (4 KiB) and ribbon (256 KiB) buffers.
        // Written in update(), read by plume/trail draws in the same frame's
        // final pass only, so one buffer per swapchain image is race-free.
        for _ in 0..images {
            let mk_host = |size: u64| {
                let info = vk::BufferCreateInfo::default()
                    .size(size)
                    .usage(vk::BufferUsageFlags::VERTEX_BUFFER)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE);
                let buffer = device.create_buffer(&info, None).expect("fxvbo");
                let req = device.get_buffer_memory_requirements(buffer);
                let index = crate::find_memory_type(
                    &mem_props,
                    req.memory_type_bits,
                    vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
                );
                let alloc = vk::MemoryAllocateInfo::default()
                    .allocation_size(req.size)
                    .memory_type_index(index);
                let memory = device.allocate_memory(&alloc, None).expect("fxvmem");
                device.bind_buffer_memory(buffer, memory, 0).expect("fxvbind");
                let mapped = device
                    .map_memory(memory, 0, size, vk::MemoryMapFlags::empty())
                    .expect("fxvmap") as *mut u8;
                (buffer, memory, mapped)
            };
            let (cb, cm, cmapped) = mk_host(4096);
            std::ptr::write_bytes(cmapped, 0, 4096);
            self.cone_buffers.push(cb);
            self.cone_memories.push(cm);
            self.cone_mapped.push(cmapped);
            let (tb, tm, tmapped) = mk_host(262144);
            std::ptr::write_bytes(tmapped, 0, 262144);
            self.trail_buffers.push(tb);
            self.trail_memories.push(tm);
            self.trail_mapped.push(tmapped);
            self.trail_origin.push(Vec3::ZERO);
            self.trail_filled.push(false);
            // Static index data, identical per slot. Host-visible for direct fill.
            let mk_index = |data: &[u16]| {
                let size = (data.len() * 2) as u64;
                let info = vk::BufferCreateInfo::default()
                    .size(size)
                    .usage(vk::BufferUsageFlags::INDEX_BUFFER)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE);
                let buffer = device.create_buffer(&info, None).expect("fxibo");
                let req = device.get_buffer_memory_requirements(buffer);
                let index = crate::find_memory_type(
                    &mem_props,
                    req.memory_type_bits,
                    vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
                );
                let alloc = vk::MemoryAllocateInfo::default()
                    .allocation_size(req.size)
                    .memory_type_index(index);
                let memory = device.allocate_memory(&alloc, None).expect("fximem");
                device.bind_buffer_memory(buffer, memory, 0).expect("fxibind");
                let mapped = device
                    .map_memory(memory, 0, size, vk::MemoryMapFlags::empty())
                    .expect("fximap") as *mut u16;
                std::ptr::copy_nonoverlapping(data.as_ptr(), mapped, data.len());
                device.unmap_memory(memory);
                (buffer, memory)
            };
            let (_, cone_idx) = crate::fx_gpu::build_plume_cone(1.0, 1.0);
            debug_assert_eq!(cone_idx.len() as u32, crate::fx_gpu::CONE_INDEX_COUNT);
            let (cb_ibo, cm_ibo) = mk_index(&cone_idx);
            self.cone_ibos.push(cb_ibo);
            self.cone_ibo_mems.push(cm_ibo);
            let trail_idx = crate::fx_gpu::build_trail_indices();
            let (tb_ibo, tm_ibo) = mk_index(&trail_idx);
            self.trail_ibos.push(tb_ibo);
            self.trail_ibo_mems.push(tm_ibo);
            self.trail_index_count = trail_idx.len() as u32;
        }
        self.image_count = images;
    }

    /// Seed one TLAS with the default pose. The per-slot command buffer can
    /// then use UPDATE for every rendered frame; BUILD is never repeated in
    /// the hot path.
    unsafe fn build_initial_rt_tlas(
        &self,
        device: &ash::Device,
        queue_family: u32,
        queue: vk::Queue,
        image_index: usize,
    ) {
        let inst_address = device.get_buffer_device_address(
            &vk::BufferDeviceAddressInfo::default().buffer(self.rt_instance_buffers[image_index]),
        );
        let instances_geom = vk::AccelerationStructureGeometryInstancesDataKHR::default()
            .array_of_pointers(false)
            .data(vk::DeviceOrHostAddressConstKHR {
                device_address: inst_address,
            });
        let tlas_geometry = vk::AccelerationStructureGeometryKHR::default()
            .geometry_type(vk::GeometryTypeKHR::INSTANCES)
            .geometry(vk::AccelerationStructureGeometryDataKHR {
                instances: instances_geom,
            });
        let scratch_address = device.get_buffer_device_address(
            &vk::BufferDeviceAddressInfo::default().buffer(self.rt_scratch[image_index]),
        );
        let info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
            .ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL)
            .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
            .flags(
                vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE
                    | vk::BuildAccelerationStructureFlagsKHR::ALLOW_UPDATE,
            )
            .geometries(std::slice::from_ref(&tlas_geometry))
            .dst_acceleration_structure(self.rt_tlas[image_index])
            .scratch_data(vk::DeviceOrHostAddressKHR {
                device_address: scratch_address,
            });
        let range = vk::AccelerationStructureBuildRangeInfoKHR::default()
            .primitive_count(self.rt_instance_count)
            .primitive_offset(0);
        let pool = device
            .create_command_pool(
                &vk::CommandPoolCreateInfo::default().queue_family_index(queue_family),
                None,
            )
            .expect("initial rt pool");
        let command = device
            .allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )
            .expect("initial rt command")[0];
        device
            .begin_command_buffer(
                command,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )
            .expect("initial rt begin");
        let host_write = [vk::MemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::HOST_WRITE)
            .dst_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_READ_KHR)];
        device.cmd_pipeline_barrier(
            command,
            vk::PipelineStageFlags::HOST,
            vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
            vk::DependencyFlags::empty(),
            &host_write,
            &[],
            &[],
        );
        self.rt_loader
            .cmd_build_acceleration_structures(command, &[info], &[&[range]]);
        device
            .end_command_buffer(command)
            .expect("initial rt end");
        let fence = device
            .create_fence(&vk::FenceCreateInfo::default(), None)
            .expect("initial rt fence");
        let commands = [command];
        device
            .queue_submit(
                queue,
                &[vk::SubmitInfo::default().command_buffers(&commands)],
                fence,
            )
            .expect("initial rt submit");
        device
            .wait_for_fences(&[fence], true, u64::MAX)
            .expect("initial rt wait");
        device.destroy_fence(fence, None);
        device.destroy_command_pool(pool, None);
    }

    /// Write a valid identity transform for every BLAS before the first
    /// initial BUILD. The first real presented update replaces these transforms
    /// with the aircraft/origin pose.
    unsafe fn write_initial_rt_instances(&self, image_index: usize) {
        let instances = self.rt_instance_mapped[image_index]
            as *mut vk::AccelerationStructureInstanceKHR;
        std::ptr::write_bytes(instances, 0, self.rt_instance_count as usize);
        let identity = [
            1.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0, 0.0,
            0.0, 0.0, 1.0, 0.0,
        ];
        let flags = vk::GeometryInstanceFlagsKHR::TRIANGLE_FACING_CULL_DISABLE.as_raw() as u8;
        let write = |address: vk::DeviceAddress, index: u32, mask: u8| {
            vk::AccelerationStructureInstanceKHR {
                transform: vk::TransformMatrixKHR { matrix: identity },
                instance_custom_index_and_mask: vk::Packed24_8::new(index, mask),
                instance_shader_binding_table_record_offset_and_flags:
                    vk::Packed24_8::new(0, flags),
                acceleration_structure_reference: vk::AccelerationStructureReferenceKHR {
                    device_handle: address,
                },
            }
        };
        for (i, node) in self.rt_geom_nodes.iter().enumerate() {
            let mask = if *node == 2 || (*node >= 4 && *node <= 6) {
                0x02
            } else if *node == 3 || (*node >= 7 && *node <= 9) {
                0x04
            } else {
                0x01
            };
            *instances.add(i) = write(self.rt_blas_addresses[i], *node, mask);
        }
        let base = self.rt_geom_nodes.len();
        if self.rt_terrain_blas_address != 0 {
            *instances.add(base) = write(self.rt_terrain_blas_address, 100, 0x10);
        }
        if self.rt_structures_blas_address != 0 {
            *instances.add(base + 1) = write(self.rt_structures_blas_address, 101, 0x08);
        }
    }

    pub unsafe fn destroy_frames(&mut self, device: &ash::Device) {
        for memory in self.cone_ibo_mems.drain(..) {
            device.free_memory(memory, None);
        }
        for buffer in self.cone_ibos.drain(..) {
            device.destroy_buffer(buffer, None);
        }
        for memory in self.trail_ibo_mems.drain(..) {
            device.free_memory(memory, None);
        }
        for buffer in self.trail_ibos.drain(..) {
            device.destroy_buffer(buffer, None);
        }
        self.trail_index_count = 0;
        for memory in self.cone_memories.drain(..) {
            device.unmap_memory(memory);
            device.free_memory(memory, None);
        }
        for buffer in self.cone_buffers.drain(..) {
            device.destroy_buffer(buffer, None);
        }
        self.cone_mapped.clear();
        for memory in self.trail_memories.drain(..) {
            device.unmap_memory(memory);
            device.free_memory(memory, None);
        }
        for buffer in self.trail_buffers.drain(..) {
            device.destroy_buffer(buffer, None);
        }
        self.trail_mapped.clear();
        self.trail_origin.clear();
        self.trail_filled.clear();
        self.fx_sets.clear();
        if self.fx_pool != vk::DescriptorPool::null() {
            device.destroy_descriptor_pool(self.fx_pool, None);
            self.fx_pool = vk::DescriptorPool::null();
        }
        for memory in self.ubo_memories.drain(..) {
            device.unmap_memory(memory);
            device.free_memory(memory, None);
        }
        for buffer in self.ubo_buffers.drain(..) {
            device.destroy_buffer(buffer, None);
        }
        self.ubo_mapped.clear();
        self.ubo_sets.clear();
        for memory in self.vegetation_output_memories.drain(..) {
            device.free_memory(memory, None);
        }
        for buffer in self.vegetation_output_buffers.drain(..) {
            device.destroy_buffer(buffer, None);
        }
        device.destroy_descriptor_pool(self.descriptor_pool, None);
        self.descriptor_pool = vk::DescriptorPool::null();
        // Ray-traced shadow per-slot resources.
        for buffer in self.rt_scratch.drain(..) {
            if buffer != vk::Buffer::null() {
                device.destroy_buffer(buffer, None);
            }
        }
        for memory in self.rt_scratch_memories.drain(..) {
            if memory != vk::DeviceMemory::null() {
                device.free_memory(memory, None);
            }
        }
        for accel in self.rt_tlas.drain(..) {
            if accel != vk::AccelerationStructureKHR::null() {
                self.rt_loader.destroy_acceleration_structure(accel, None);
            }
        }
        for buffer in self.rt_tlas_buffers.drain(..) {
            if buffer != vk::Buffer::null() {
                device.destroy_buffer(buffer, None);
            }
        }
        for memory in self.rt_tlas_memories.drain(..) {
            if memory != vk::DeviceMemory::null() {
                device.free_memory(memory, None);
            }
        }
        for memory in self.rt_instance_memories.drain(..) {
            if memory != vk::DeviceMemory::null() {
                device.unmap_memory(memory);
                device.free_memory(memory, None);
            }
        }
        for buffer in self.rt_instance_buffers.drain(..) {
            if buffer != vk::Buffer::null() {
                device.destroy_buffer(buffer, None);
            }
        }
        self.rt_instance_mapped.clear();
        self.image_count = 0;
    }

    pub unsafe fn prepare_query_pool(&mut self, device: &ash::Device, image_count: usize) {
        device.destroy_query_pool(self.query_pool, None);
        let query_info = vk::QueryPoolCreateInfo::default()
            .query_type(vk::QueryType::TIMESTAMP)
            .query_count((image_count * GPU_STAMPS_PER_FRAME as usize) as u32);
        self.query_pool = device
            .create_query_pool(&query_info, None)
            .expect("query pool");
    }

    #[inline]

    pub unsafe fn record(
        &self,
        device: &ash::Device,
        cmd: vk::CommandBuffer,
        image: vk::Image,
        view: vk::ImageView,
        msaa_image: vk::Image,
        msaa_view: vk::ImageView,
        depth_image: vk::Image,
        depth_view: vk::ImageView,
        hdr_image: vk::Image,
        hdr_view: vk::ImageView,
        comp_set: vk::DescriptorSet,
        scene_extent: vk::Extent2D,
        output_extent: vk::Extent2D,
        image_index: usize,
        query_base: u32,
        measure_gpu: bool,
    ) {
        let begin = vk::CommandBufferBeginInfo::default();
        device.begin_command_buffer(cmd, &begin).expect("pbegin");
        if measure_gpu {
            device.cmd_reset_query_pool(cmd, self.query_pool, query_base, GPU_STAMPS_PER_FRAME);
            // Include acceleration-structure updates and barriers in whole-frame timing.
            device.cmd_write_timestamp(cmd, vk::PipelineStageFlags::TOP_OF_PIPE, self.query_pool, query_base);
        }
        let scene_viewport = vk::Viewport::default()
            .x(0.0)
            .y(0.0)
            .width(scene_extent.width as f32)
            .height(scene_extent.height as f32)
            .min_depth(0.0)
            .max_depth(1.0);
        let scene_scissor = vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: scene_extent,
        };
        let output_viewport = vk::Viewport::default()
            .x(0.0)
            .y(0.0)
            .width(output_extent.width as f32)
            .height(output_extent.height as f32)
            .min_depth(0.0)
            .max_depth(1.0);
        let output_scissor = vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: output_extent,
        };
        let set = self.ubo_sets[image_index];
        // Ray-traced shadows: update this slot's already-built top-level
        // structure from the host-written instance transforms. The initial
        // BUILD happens once in build_frames; UPDATE keeps the hot path
        // proportional to animated instance transforms rather than rebuilding
        // the whole hierarchy.
        if self.rt_supported && self.rt_instance_count > 0 && measure_gpu {
            let inst_address = device.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default()
                .buffer(self.rt_instance_buffers[image_index]));
            let host_write = [vk::MemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::HOST_WRITE)
                .dst_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_READ_KHR)];
            device.cmd_pipeline_barrier(
                cmd,
                vk::PipelineStageFlags::HOST,
                vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
                vk::DependencyFlags::empty(),
                &host_write,
                &[],
                &[],
            );
            let instances_geom =
                vk::AccelerationStructureGeometryInstancesDataKHR::default()
                    .array_of_pointers(false)
                    .data(vk::DeviceOrHostAddressConstKHR {
                        device_address: inst_address,
                    });
            let tlas_geometry = vk::AccelerationStructureGeometryKHR::default()
                .geometry_type(vk::GeometryTypeKHR::INSTANCES)
                .geometry(vk::AccelerationStructureGeometryDataKHR {
                    instances: instances_geom,
                });
            let scratch_address = device.get_buffer_device_address(
                &vk::BufferDeviceAddressInfo::default().buffer(self.rt_scratch[image_index]),
            );
            let tlas_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
                .ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL)
                .mode(vk::BuildAccelerationStructureModeKHR::UPDATE)
                .flags(
                    vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE
                        | vk::BuildAccelerationStructureFlagsKHR::ALLOW_UPDATE,
                )
                .geometries(std::slice::from_ref(&tlas_geometry))
                .src_acceleration_structure(self.rt_tlas[image_index])
                .dst_acceleration_structure(self.rt_tlas[image_index])
                .scratch_data(vk::DeviceOrHostAddressKHR {
                    device_address: scratch_address,
                });
            let range = vk::AccelerationStructureBuildRangeInfoKHR::default()
                .primitive_count(self.rt_instance_count)
                .primitive_offset(0);
            self.rt_loader
                .cmd_build_acceleration_structures(cmd, &[tlas_info], &[&[range]]);
            let tlas_ready = [vk::MemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_WRITE_KHR)
                .dst_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_READ_KHR)];
            device.cmd_pipeline_barrier(
                cmd,
                vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::DependencyFlags::empty(),
                &tlas_ready,
                &[],
                &[],
            );
        }
        if !measure_gpu {
            // Intermediate pass: zero-attachment null rendering. The vertex
            // stage evaluates the animated airframe and the rasterizer
            // traverses every triangle, but nothing is stored and no
            // attachment layouts are touched, so passes need no barriers and
            // can overlap freely on the GPU timeline.
            let void_rendering = vk::RenderingInfo::default()
                .render_area(vk::Rect2D {
                    offset: vk::Offset2D { x: 0, y: 0 },
                    extent: scene_extent,
                })
                .layer_count(1);
            device.cmd_begin_rendering(cmd, &void_rendering);
            device.cmd_set_viewport(cmd, 0, &[scene_viewport]);
            device.cmd_set_scissor(cmd, 0, &[scene_scissor]);
            device.cmd_bind_vertex_buffers(cmd, 0, &[self.vertex_buffer], &[0]);
            device.cmd_bind_index_buffer(cmd, self.index_buffer, 0, vk::IndexType::UINT16);
            device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.void_pipeline);
            device.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.layout,
                0,
                &[set],
                &[],
            );
            device.cmd_draw_indexed(cmd, self.opaque_count, 1, 0, 0, 0);
            device.cmd_end_rendering(cmd);
            device.end_command_buffer(cmd).expect("pend");
            return;
        }
        let clear_color = vk::ClearValue {
            color: vk::ClearColorValue {
                float32: [0.0, 0.0, 0.0, 1.0],
            },
        };
        let clear_depth = vk::ClearValue {
            depth_stencil: vk::ClearDepthStencilValue {
                depth: 1.0,
                stencil: 0,
            },
        };
        // HDR scene pass: opaque + atmospheric background + terrain + FX all
        // compose in linear HDR. The swapchain only sees the final composite.
        let mut color_info = vk::RenderingAttachmentInfo::default()
            .image_view(if self.samples == vk::SampleCountFlags::TYPE_1 {
                hdr_view
            } else {
                msaa_view
            })
            .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::DONT_CARE)
            .store_op(if self.samples == vk::SampleCountFlags::TYPE_1 {
                vk::AttachmentStoreOp::STORE
            } else {
                vk::AttachmentStoreOp::DONT_CARE
            })
            .clear_value(clear_color);
        if self.samples != vk::SampleCountFlags::TYPE_1 {
            color_info = color_info
                .resolve_mode(vk::ResolveModeFlags::AVERAGE)
                .resolve_image_view(hdr_view)
                .resolve_image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
        }
        let depth_info = vk::RenderingAttachmentInfo::default()
            .image_view(depth_view)
            .image_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::DONT_CARE)
            .clear_value(clear_depth);
        let colors = [color_info];
        let rendering = vk::RenderingInfo::default()
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: scene_extent,
            })
            .layer_count(1)
            .color_attachments(&colors)
            .depth_attachment(&depth_info);
        let color_range = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .base_mip_level(0)
            .level_count(1)
            .base_array_layer(0)
            .layer_count(1);
        let depth_range = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::DEPTH)
            .base_mip_level(0)
            .level_count(1)
            .base_array_layer(0)
            .layer_count(1);
        // This is the only attachment-writing pass of the burst: the void
        // passes never touch the images, so the layouts are established here
        // with discard transitions (depth clear is the presented pass's own).
        // HDR target carries the scene; the swapchain is touched only by the
        // trailing composite triangle.
        let mut to_draw = Vec::with_capacity(3);
        if self.samples != vk::SampleCountFlags::TYPE_1 {
            to_draw.push(
                vk::ImageMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::empty())
                    .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                    .old_layout(vk::ImageLayout::UNDEFINED)
                    .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                    .image(msaa_image)
                    .subresource_range(color_range),
            );
        }
        to_draw.push(
            vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::empty())
                .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .image(hdr_image)
                .subresource_range(color_range),
        );
        to_draw.push(
            vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::empty())
                .dst_access_mask(vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)
                .image(depth_image)
                .subresource_range(depth_range),
        );
        let attachment_dependency = [vk::MemoryBarrier::default()
            .src_access_mask(
                vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                    | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
            )
            .dst_access_mask(
                vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                    | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
            )];
        let attachment_stages = vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
            | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS
            | vk::PipelineStageFlags::LATE_FRAGMENT_TESTS;
        device.cmd_pipeline_barrier(
            cmd,
            attachment_stages,
            attachment_stages,
            vk::DependencyFlags::empty(),
            &attachment_dependency,
            &[],
            &to_draw,
        );
        device.cmd_begin_rendering(cmd, &rendering);
        device.cmd_set_viewport(cmd, 0, &[scene_viewport]);
        device.cmd_set_scissor(cmd, 0, &[scene_scissor]);
        device.cmd_bind_vertex_buffers(cmd, 0, &[self.vertex_buffer], &[0]);
        device.cmd_bind_index_buffer(cmd, self.index_buffer, 0, vk::IndexType::UINT16);
        device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.opaque_pipeline);
        device.cmd_bind_descriptor_sets(
            cmd,
            vk::PipelineBindPoint::GRAPHICS,
            self.layout,
            0,
            &[set],
            &[],
        );
        device.cmd_draw_indexed(cmd, self.opaque_count, 1, 0, 0, 0);
        if measure_gpu {
            // Per-pass GPU breakdown: q0 start, then one stamp after each
            // scene stage. Terrain and vegetation are split because their
            // command streams now have independent budgets.
            let stamp = |device: &ash::Device, q: u32| {
                device.cmd_write_timestamp(
                    cmd,
                    vk::PipelineStageFlags::TOP_OF_PIPE,
                    self.query_pool,
                    query_base + q,
                );
            };
            stamp(device, 1);
            device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.terrain_pipeline);
            device.cmd_bind_index_buffer(cmd, self.index_buffer, self.terrain_index_offset, vk::IndexType::UINT32);
            for first in (0..self.terrain_chunk_count).step_by(self.terrain_draw_batch as usize) {
                device.cmd_draw_indexed_indirect(cmd, self.ubo_buffers[image_index],
                    (UBO_BYTES + first as usize * TERRAIN_COMMAND_BYTES) as u64,
                    self.terrain_draw_batch.min(self.terrain_chunk_count - first),
                    TERRAIN_COMMAND_BYTES as u32);
            }
            // Landmark vertices use the general material fragment shader; keep
            // them out of the terrain-only module so terrain waves, detail,
            // and atmospheric pixels have a smaller live shader footprint.
            device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.ground_pipeline);
            device.cmd_draw_indexed(cmd, world::GROUND_FEATURE_INDEX_COUNT, 1, world::TERRAIN_INDEX_COUNT, 0, 0);
            stamp(device, 2);
            if self.gpu_vegetation_cull {
                // Reset only the atomic survivor counter. The cull shader
                // rewrites the remaining DrawIndirectCommand words and the
                // compact records; a 4-byte fill keeps the reset bandwidth
                // negligible compared with clearing the whole output list.
                let output = self.vegetation_output_buffers[image_index];
                device.cmd_fill_buffer(cmd, output, 4, 4, 0);
                let reset_barrier = [vk::BufferMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(
                        vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE,
                    )
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .buffer(output)
                    .offset(4)
                    .size(4)];
                device.cmd_pipeline_barrier(
                    cmd,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::COMPUTE_SHADER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &reset_barrier,
                    &[],
                );
                device.cmd_bind_pipeline(
                    cmd,
                    vk::PipelineBindPoint::COMPUTE,
                    self.vegetation_cull_pipeline,
                );
                device.cmd_bind_descriptor_sets(
                    cmd,
                    vk::PipelineBindPoint::COMPUTE,
                    self.layout,
                    0,
                    &[set],
                    &[],
                );
                let cull_groups = (world::vegetation::VEGETATION_CULL_CELL_COUNT + 63) / 64;
                device.cmd_dispatch(cmd, cull_groups, 1, 1);
                let cull_barrier = [vk::BufferMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::SHADER_WRITE)
                    .dst_access_mask(
                        vk::AccessFlags::SHADER_READ | vk::AccessFlags::SHADER_WRITE,
                    )
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .buffer(output)
                    .offset(0)
                    .size(VEGETATION_OUTPUT_BYTES as u64)];
                device.cmd_pipeline_barrier(
                    cmd,
                    vk::PipelineStageFlags::COMPUTE_SHADER,
                    vk::PipelineStageFlags::COMPUTE_SHADER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &cull_barrier,
                    &[],
                );
                device.cmd_bind_pipeline(
                    cmd,
                    vk::PipelineBindPoint::COMPUTE,
                    self.vegetation_finalize_pipeline,
                );
                device.cmd_dispatch(cmd, 1, 1, 1);
                let draw_barrier = [vk::BufferMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::SHADER_WRITE)
                    .dst_access_mask(
                        vk::AccessFlags::INDIRECT_COMMAND_READ | vk::AccessFlags::SHADER_READ,
                    )
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .buffer(output)
                    .offset(0)
                    .size(VEGETATION_OUTPUT_BYTES as u64)];
                device.cmd_pipeline_barrier(
                    cmd,
                    vk::PipelineStageFlags::COMPUTE_SHADER,
                    vk::PipelineStageFlags::DRAW_INDIRECT | vk::PipelineStageFlags::VERTEX_SHADER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &draw_barrier,
                    &[],
                );
                device.cmd_bind_pipeline(
                    cmd,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.vegetation_compact_pipeline,
                );
                device.cmd_draw_indirect(
                    cmd,
                    output,
                    0,
                    1,
                    VEGETATION_OUTPUT_COMMAND_BYTES as u32,
                );
            } else {
                device.cmd_bind_pipeline(
                    cmd,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.vegetation_pipeline,
                );
                for first in (0..VEGETATION_COMMAND_COUNT)
                    .step_by(self.vegetation_draw_batch as usize)
                {
                    device.cmd_draw_indirect(
                        cmd,
                        self.ubo_buffers[image_index],
                        (VEGETATION_COMMAND_OFFSET
                            + first as usize * VEGETATION_COMMAND_BYTES) as u64,
                        self.vegetation_draw_batch.min(VEGETATION_COMMAND_COUNT - first),
                        VEGETATION_COMMAND_BYTES as u32,
                    );
                }
            }
            stamp(device, 3);
            device.cmd_bind_pipeline(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.canopy_pipeline,
            );
            for first in (0..CANOPY_COMMAND_COUNT).step_by(self.canopy_draw_batch as usize) {
                device.cmd_draw_indirect(
                    cmd,
                    self.ubo_buffers[image_index],
                    (CANOPY_COMMAND_OFFSET + first as usize * CANOPY_COMMAND_BYTES) as u64,
                    self.canopy_draw_batch.min(CANOPY_COMMAND_COUNT - first),
                    CANOPY_COMMAND_BYTES as u32,
                );
            }
            stamp(device, 4);
            // Mesh clouds draw after terrain; Performance uses a smaller
            // atmospheric LOD, while occluded puffs are Early-Z culled by
            // mountain depth.
            device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.cloud_pipeline);
            device.cmd_bind_index_buffer(cmd, self.index_buffer, self.cloud_index_offset, vk::IndexType::UINT16);
            device.cmd_draw_indexed(
                cmd,
                self.cloud_puffs * crate::clouds::CLOUD_CORNERS,
                self.cloud_cells,
                0,
                0,
                0,
            );
            stamp(device, 5);
            // Sky quad draws last at depth 0.999999; all terrain and cloud fragments are Early-Z culled.
            device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.sky_pipeline);
            device.cmd_draw(cmd, 6, 1, 0, 0);
            stamp(device, 6);
            // Plume cone raymarch into HDR (forward alpha blend, no depth write).
            let fx_set = self.fx_sets[image_index];
            device.cmd_bind_pipeline(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.plume_pipeline,
            );
            device.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.fx_pipeline_layout,
                0,
                &[set, fx_set],
                &[],
            );
            device.cmd_bind_vertex_buffers(cmd, 0, &[self.cone_buffers[image_index]], &[0]);
            device.cmd_bind_index_buffer(
                cmd,
                self.cone_ibos[image_index],
                0,
                vk::IndexType::UINT16,
            );
            device.cmd_draw_indexed(cmd, crate::fx_gpu::CONE_INDEX_COUNT, 1, 0, 0, 0);
            stamp(device, 7);
            // Persistent ribbons into HDR. Fixed index range; unused verts are
            // zero density and discard in the fragment shader.
            device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.trail_pipeline);
            device.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.fx_pipeline_layout,
                0,
                &[set, fx_set],
                &[],
            );
            device.cmd_bind_vertex_buffers(cmd, 0, &[self.trail_buffers[image_index]], &[0]);
            device.cmd_bind_index_buffer(
                cmd,
                self.trail_ibos[image_index],
                0,
                vk::IndexType::UINT16,
            );
            device.cmd_draw_indexed(cmd, self.trail_index_count, 1, 0, 0, 0);
            stamp(device, 8);
            device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.glass_pipeline);
            device.cmd_bind_vertex_buffers(cmd, 0, &[self.vertex_buffer], &[0]);
            device.cmd_bind_index_buffer(cmd, self.index_buffer, 0, vk::IndexType::UINT16);
            device.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.layout,
                0,
                &[set],
                &[],
            );
            device.cmd_draw_indexed(cmd, self.glass_count, 1, self.glass_first, 0, 0);
            stamp(device, 9);
        }
        device.cmd_end_rendering(cmd);
        if measure_gpu {
            // HDR scene to shader-readable for the composite triangle.
            let hdr_to_read = vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                .dst_access_mask(vk::AccessFlags::SHADER_READ)
                .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .image(hdr_image)
                .subresource_range(color_range);
            // Swapchain discard transition; composite overwrites every pixel.
            let swap_to_draw = vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::empty())
                .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .image(image)
                .subresource_range(color_range);
            device.cmd_pipeline_barrier(
                cmd,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                    | vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[hdr_to_read, swap_to_draw],
            );
            let swap_clear = vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: [0.0, 0.0, 0.0, 1.0],
                },
            };
            let swap_color = vk::RenderingAttachmentInfo::default()
                .image_view(view)
                .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .load_op(vk::AttachmentLoadOp::DONT_CARE)
                .store_op(vk::AttachmentStoreOp::STORE)
                .clear_value(swap_clear);
            let swap_colors = [swap_color];
            let swap_rendering = vk::RenderingInfo::default()
                .render_area(vk::Rect2D {
                    offset: vk::Offset2D { x: 0, y: 0 },
                    extent: output_extent,
                })
                .layer_count(1)
                .color_attachments(&swap_colors);
            device.cmd_begin_rendering(cmd, &swap_rendering);
            device.cmd_set_viewport(cmd, 0, &[output_viewport]);
            device.cmd_set_scissor(cmd, 0, &[output_scissor]);
            device.cmd_bind_pipeline(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.composite_pipeline,
            );
            device.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.composite_layout,
                0,
                &[set, comp_set],
                &[],
            );
            device.cmd_draw(cmd, 6, 1, 0, 0);
            device.cmd_end_rendering(cmd);
            // The Performance composite uses coarse shading, but the HUD is
            // a second full-rate alpha pass. Make the composite color writes
            // visible to the attachment load before beginning that overlay.
            let hud_dependency = vk::MemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                .dst_access_mask(
                    vk::AccessFlags::COLOR_ATTACHMENT_READ
                        | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
                );
            device.cmd_pipeline_barrier(
                cmd,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::DependencyFlags::empty(),
                &[hud_dependency],
                &[],
                &[],
            );
            let hud_color = vk::RenderingAttachmentInfo::default()
                .image_view(view)
                .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .load_op(vk::AttachmentLoadOp::LOAD)
                .store_op(vk::AttachmentStoreOp::STORE);
            let hud_colors = [hud_color];
            let hud_rendering = vk::RenderingInfo::default()
                .render_area(vk::Rect2D {
                    offset: vk::Offset2D { x: 0, y: 0 },
                    extent: output_extent,
                })
                .layer_count(1)
                .color_attachments(&hud_colors);
            device.cmd_begin_rendering(cmd, &hud_rendering);
            device.cmd_set_viewport(cmd, 0, &[output_viewport]);
            device.cmd_set_scissor(cmd, 0, &[output_scissor]);
            device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.hud_pipeline);
            device.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.layout,
                0,
                &[set],
                &[],
            );
            device.cmd_draw(cmd, 6, 1, 0, 0);
            device.cmd_end_rendering(cmd);
            let to_present = vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                .dst_access_mask(vk::AccessFlags::empty())
                .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                .image(image)
                .subresource_range(color_range);
            device.cmd_pipeline_barrier(
                cmd,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[to_present],
            );
            device.cmd_write_timestamp(
                cmd,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                self.query_pool,
                query_base + 10,
            );
        }
        device.end_command_buffer(cmd).expect("pend");
    }

}
