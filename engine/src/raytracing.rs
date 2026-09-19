//! Hardware Ray Tracing Acceleration Structures (VK_KHR_ray_query).
//!
//! Manages Bottom-Level Acceleration Structures (BLAS) for articulated airframe
//! kinematic nodes and per-frame Top-Level Acceleration Structures (TLAS) for
//! physically accurate soft shadow casting against the ground and environment.

use ash::khr;
use ash::vk;
use glam::Mat4;

use crate::ubo::align_256;

/// Allocate a device-local GPU buffer, setting `DEVICE_ADDRESS` memory flags if required.
pub unsafe fn alloc_device_buffer(
    device: &ash::Device,
    mem_props: &vk::PhysicalDeviceMemoryProperties,
    size: u64,
    usage: vk::BufferUsageFlags,
) -> (vk::Buffer, vk::DeviceMemory) {
    let info = vk::BufferCreateInfo::default()
        .size(size)
        .usage(usage)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let buffer = device.create_buffer(&info, None).expect("device buffer");
    let req = device.get_buffer_memory_requirements(buffer);
    let index = crate::find_memory_type(
        mem_props,
        req.memory_type_bits,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
    );
    let alloc = vk::MemoryAllocateInfo::default()
        .allocation_size(req.size)
        .memory_type_index(index);
    let memory = if usage.contains(vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS) {
        let mut addr_flags =
            vk::MemoryAllocateFlagsInfo::default().flags(vk::MemoryAllocateFlags::DEVICE_ADDRESS);
        device
            .allocate_memory(&alloc.push_next(&mut addr_flags), None)
            .expect("device memory (addressable)")
    } else {
        device
            .allocate_memory(&alloc, None)
            .expect("device memory")
    };
    device.bind_buffer_memory(buffer, memory, 0).expect("bind device buffer");
    (buffer, memory)
}

/// Allocate a host-visible, host-coherent staging or instance buffer and map it.
pub unsafe fn alloc_host_buffer(
    device: &ash::Device,
    mem_props: &vk::PhysicalDeviceMemoryProperties,
    size: u64,
    usage: vk::BufferUsageFlags,
) -> (vk::Buffer, vk::DeviceMemory, *mut u8) {
    let info = vk::BufferCreateInfo::default()
        .size(size)
        .usage(usage)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let buffer = device.create_buffer(&info, None).expect("host buffer");
    let req = device.get_buffer_memory_requirements(buffer);
    let index = crate::find_memory_type(
        mem_props,
        req.memory_type_bits,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
    );
    let alloc = vk::MemoryAllocateInfo::default()
        .allocation_size(req.size)
        .memory_type_index(index);
    let memory = if usage.contains(vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS) {
        let mut addr_flags =
            vk::MemoryAllocateFlagsInfo::default().flags(vk::MemoryAllocateFlags::DEVICE_ADDRESS);
        device
            .allocate_memory(&alloc.push_next(&mut addr_flags), None)
            .expect("host memory (addressable)")
    } else {
        device
            .allocate_memory(&alloc, None)
            .expect("host memory")
    };
    device.bind_buffer_memory(buffer, memory, 0).expect("bind host buffer");
    let mapped = device
        .map_memory(memory, 0, size, vk::MemoryMapFlags::empty())
        .expect("map host buffer") as *mut u8;
    (buffer, memory, mapped)
}

/// Bottom-Level Acceleration Structure (BLAS) context for all airframe components.
///
/// Contains one BLAS per kinematic node (wings, flaps, rudder, petals, fuselage)
/// compacted into a single device-local memory allocation.
pub struct BlasContext {
    pub blas: Vec<vk::AccelerationStructureKHR>,
    pub blas_addresses: Vec<vk::DeviceAddress>,
    pub geom_nodes: Vec<u32>,
    pub buffer: vk::Buffer,
    pub memory: vk::DeviceMemory,
    pub scratch_buffer: vk::Buffer,
    pub scratch_memory: vk::DeviceMemory,
    pub index_buffer: vk::Buffer,
    pub index_memory: vk::DeviceMemory,
    pub index_address: vk::DeviceAddress,
}

impl BlasContext {
    /// Build all bottom-level acceleration structures in a single command buffer pass.
    pub unsafe fn build(
        device: &ash::Device,
        rt_loader: &khr::acceleration_structure::Device,
        mem_props: &vk::PhysicalDeviceMemoryProperties,
        queue: vk::Queue,
        queue_family: u32,
        vertex_address: vk::DeviceAddress,
        vertex_count: usize,
        vertex_bytes: usize,
        rt_idx: &[u16],
        rt_node_ranges: &[(u32, u32)],
        rt_geom_nodes: &[u32],
    ) -> Self {
        let rt_index_bytes = (rt_idx.len() * 2) as u64;
        let (index_buffer, index_memory) = alloc_device_buffer(
            device,
            mem_props,
            rt_index_bytes,
            vk::BufferUsageFlags::INDEX_BUFFER
                | vk::BufferUsageFlags::TRANSFER_DST
                | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
                | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR,
        );

        // Upload RT index buffer via staging
        {
            let (stage_buf, stage_mem, stage_map) = alloc_host_buffer(
                device,
                mem_props,
                rt_index_bytes,
                vk::BufferUsageFlags::TRANSFER_SRC,
            );
            if !rt_idx.is_empty() {
                std::ptr::copy_nonoverlapping(
                    rt_idx.as_ptr() as *const u8,
                    stage_map,
                    rt_idx.len() * 2,
                );
            }
            device.unmap_memory(stage_mem);

            let pool_info = vk::CommandPoolCreateInfo::default()
                .queue_family_index(queue_family)
                .flags(vk::CommandPoolCreateFlags::TRANSIENT);
            let pool = device.create_command_pool(&pool_info, None).expect("pool");
            let alloc = vk::CommandBufferAllocateInfo::default()
                .command_pool(pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1);
            let cmd = device.allocate_command_buffers(&alloc).expect("cmd")[0];
            let begin = vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            device.begin_command_buffer(cmd, &begin).expect("begin");
            let copy = vk::BufferCopy::default().size(rt_index_bytes);
            device.cmd_copy_buffer(cmd, stage_buf, index_buffer, &[copy]);
            device.end_command_buffer(cmd).expect("end");

            let fence = device.create_fence(&vk::FenceCreateInfo::default(), None).expect("fence");
            let cmd_ref = [cmd];
            let submit = vk::SubmitInfo::default().command_buffers(&cmd_ref);
            device.queue_submit(queue, &[submit], fence).expect("submit");
            device.wait_for_fences(&[fence], true, u64::MAX).expect("wait");
            device.destroy_fence(fence, None);
            device.destroy_command_pool(pool, None);
            device.destroy_buffer(stage_buf, None);
            device.free_memory(stage_mem, None);
        }

        let index_address = device.get_buffer_device_address(
            &vk::BufferDeviceAddressInfo::default().buffer(index_buffer),
        );

        // Triangle geometries for casters, one per node range
        let mut geoms: Vec<vk::AccelerationStructureGeometryKHR> =
            Vec::with_capacity(rt_geom_nodes.len());
        let mut ranges: Vec<vk::AccelerationStructureBuildRangeInfoKHR> =
            Vec::with_capacity(rt_geom_nodes.len());
        let max_vertex = vertex_count as u32 - 1;

        for (off, cnt) in rt_node_ranges {
            let triangles = vk::AccelerationStructureGeometryTrianglesDataKHR::default()
                .vertex_format(vk::Format::R32G32B32_SFLOAT)
                .vertex_data(vk::DeviceOrHostAddressConstKHR {
                    device_address: vertex_address,
                })
                .vertex_stride(vertex_bytes as u64)
                .max_vertex(max_vertex)
                .index_type(vk::IndexType::UINT16)
                .index_data(vk::DeviceOrHostAddressConstKHR {
                    device_address: index_address + (*off as u64) * 2,
                });
            geoms.push(
                vk::AccelerationStructureGeometryKHR::default()
                    .geometry_type(vk::GeometryTypeKHR::TRIANGLES)
                    .geometry(vk::AccelerationStructureGeometryDataKHR { triangles })
                    .flags(vk::GeometryFlagsKHR::OPAQUE),
            );
            ranges.push(
                vk::AccelerationStructureBuildRangeInfoKHR::default()
                    .primitive_count(cnt / 3)
                    .primitive_offset(0)
                    .first_vertex(0),
            );
        }

        // Hardware size query
        let mut sizes: Vec<vk::AccelerationStructureBuildSizesInfoKHR> =
            (0..geoms.len()).map(|_| Default::default()).collect();
        for (i, g) in geoms.iter().enumerate() {
            let build_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
                .ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL)
                .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
                .flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
                .geometries(std::slice::from_ref(g));
            rt_loader.get_acceleration_structure_build_sizes(
                vk::AccelerationStructureBuildTypeKHR::DEVICE,
                &build_info,
                &[ranges[i].primitive_count],
                &mut sizes[i],
            );
        }

        let scratch_bytes = sizes
            .iter()
            .fold(0u64, |off, s| align_256(off + s.build_scratch_size));
        let mut scratch_offsets: Vec<u64> = Vec::with_capacity(sizes.len());
        let mut scratch_off = 0u64;
        for s in &sizes {
            scratch_offsets.push(scratch_off);
            scratch_off = align_256(scratch_off + s.build_scratch_size);
        }
        let mut as_offset = 0u64;
        for s in &sizes {
            as_offset = align_256(as_offset + s.acceleration_structure_size);
        }
        let total_as = as_offset;

        let (abuf, amem) = alloc_device_buffer(
            device,
            mem_props,
            total_as,
            vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR
                | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
        );
        let (sbuf, smem) = alloc_device_buffer(
            device,
            mem_props,
            scratch_bytes,
            vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
        );

        let as_address = device
            .get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(abuf));
        let scratch_address = device
            .get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(sbuf));

        let mut rt_blas = Vec::with_capacity(sizes.len());
        let mut rt_blas_addresses = Vec::with_capacity(sizes.len());
        let mut as_offset = 0u64;
        for s in &sizes {
            let ci = vk::AccelerationStructureCreateInfoKHR::default()
                .buffer(abuf)
                .offset(as_offset)
                .size(s.acceleration_structure_size)
                .ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL);
            rt_blas.push(rt_loader.create_acceleration_structure(&ci, None).expect("blas"));
            rt_blas_addresses.push(as_address + as_offset);
            as_offset = align_256(as_offset + s.acceleration_structure_size);
        }

        // Build every BLAS in one command
        let pool_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(queue_family)
            .flags(vk::CommandPoolCreateFlags::TRANSIENT);
        let pool = device.create_command_pool(&pool_info, None).expect("rt pool");
        let alloc = vk::CommandBufferAllocateInfo::default()
            .command_pool(pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let cmd = device.allocate_command_buffers(&alloc).expect("rt cmd")[0];
        let begin = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        device.begin_command_buffer(cmd, &begin).expect("rt begin");

        let mut build_infos: Vec<vk::AccelerationStructureBuildGeometryInfoKHR> =
            Vec::with_capacity(geoms.len());
        let mut build_ranges: Vec<Vec<vk::AccelerationStructureBuildRangeInfoKHR>> =
            Vec::with_capacity(geoms.len());
        for (i, g) in geoms.iter().enumerate() {
            build_infos.push(
                vk::AccelerationStructureBuildGeometryInfoKHR::default()
                    .ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL)
                    .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
                    .flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
                    .geometries(std::slice::from_ref(g))
                    .dst_acceleration_structure(rt_blas[i])
                    .scratch_data(vk::DeviceOrHostAddressKHR {
                        device_address: scratch_address + scratch_offsets[i],
                    }),
            );
            build_ranges.push(vec![ranges[i]]);
        }
        let build_range_refs: Vec<&[vk::AccelerationStructureBuildRangeInfoKHR]> =
            build_ranges.iter().map(|v| v.as_slice()).collect();
        rt_loader.cmd_build_acceleration_structures(cmd, &build_infos, &build_range_refs);

        let build_barrier = vk::MemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_WRITE_KHR)
            .dst_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_READ_KHR);
        device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
            vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
            vk::DependencyFlags::empty(),
            &[build_barrier],
            &[],
            &[],
        );
        device.end_command_buffer(cmd).expect("rt end");

        let fence = device.create_fence(&vk::FenceCreateInfo::default(), None).expect("rt fence");
        let cmd_ref = [cmd];
        let submit = vk::SubmitInfo::default().command_buffers(&cmd_ref);
        device.queue_submit(queue, &[submit], fence).expect("rt submit");
        device.wait_for_fences(&[fence], true, u64::MAX).expect("rt wait");
        device.destroy_fence(fence, None);
        device.destroy_command_pool(pool, None);

        println!(
            "RT: {} node BLAS, {:.1} KiB casters, {:.1} KiB structures",
            rt_blas.len(),
            rt_index_bytes as f32 / 1024.0,
            total_as as f32 / 1024.0,
        );

        Self {
            blas: rt_blas,
            blas_addresses: rt_blas_addresses,
            geom_nodes: rt_geom_nodes.to_vec(),
            buffer: abuf,
            memory: amem,
            scratch_buffer: sbuf,
            scratch_memory: smem,
            index_buffer,
            index_memory,
            index_address,
        }
    }

    /// Destroy BLAS acceleration structures and buffers.
    pub unsafe fn destroy(
        &mut self,
        device: &ash::Device,
        rt_loader: &khr::acceleration_structure::Device,
    ) {
        for b in self.blas.drain(..) {
            rt_loader.destroy_acceleration_structure(b, None);
        }
        device.destroy_buffer(self.buffer, None);
        device.free_memory(self.memory, None);
        device.destroy_buffer(self.scratch_buffer, None);
        device.free_memory(self.scratch_memory, None);
        device.destroy_buffer(self.index_buffer, None);
        device.free_memory(self.index_memory, None);
    }
}

/// Per-swapchain frame Top-Level Acceleration Structure (TLAS) slot.
pub struct TlasSlot {
    pub instance_buffer: vk::Buffer,
    pub instance_memory: vk::DeviceMemory,
    pub instance_mapped: *mut u8,
    pub tlas: vk::AccelerationStructureKHR,
    pub tlas_buffer: vk::Buffer,
    pub tlas_memory: vk::DeviceMemory,
    pub scratch_buffer: vk::Buffer,
    pub scratch_memory: vk::DeviceMemory,
}

impl TlasSlot {
    /// Create a new TLAS and host-visible instance buffer for one swapchain frame.
    pub unsafe fn create(
        device: &ash::Device,
        rt_loader: &khr::acceleration_structure::Device,
        mem_props: &vk::PhysicalDeviceMemoryProperties,
        instance_count: u32,
        descriptor_set: vk::DescriptorSet,
    ) -> Self {
        let inst_bytes = (instance_count as usize)
            * std::mem::size_of::<vk::AccelerationStructureInstanceKHR>();
        let (inst_buffer, inst_memory, inst_mapped) = alloc_host_buffer(
            device,
            mem_props,
            inst_bytes as u64,
            vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
                | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR,
        );

        let inst_address = device.get_buffer_device_address(
            &vk::BufferDeviceAddressInfo::default().buffer(inst_buffer),
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

        let mut tlas_size = vk::AccelerationStructureBuildSizesInfoKHR::default();
        let tlas_build_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
            .ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL)
            .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
            .flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
            .geometries(std::slice::from_ref(&tlas_geometry));
        rt_loader.get_acceleration_structure_build_sizes(
            vk::AccelerationStructureBuildTypeKHR::DEVICE,
            &tlas_build_info,
            &[instance_count],
            &mut tlas_size,
        );

        let (tlas_buffer, tlas_memory) = alloc_device_buffer(
            device,
            mem_props,
            tlas_size.acceleration_structure_size,
            vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR
                | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
        );
        let (scratch_buffer, scratch_memory) = alloc_device_buffer(
            device,
            mem_props,
            tlas_size.build_scratch_size,
            vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
        );

        let tlas_create = vk::AccelerationStructureCreateInfoKHR::default()
            .buffer(tlas_buffer)
            .offset(0)
            .size(tlas_size.acceleration_structure_size)
            .ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL);
        let tlas = rt_loader
            .create_acceleration_structure(&tlas_create, None)
            .expect("tlas");

        let tlas_ref = [tlas];
        let mut rt_as_write = vk::WriteDescriptorSetAccelerationStructureKHR::default()
            .acceleration_structures(&tlas_ref);
        let write_rt_as = vk::WriteDescriptorSet::default()
            .dst_set(descriptor_set)
            .dst_binding(5)
            .descriptor_count(1)
            .descriptor_type(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR)
            .push_next(&mut rt_as_write);
        device.update_descriptor_sets(&[write_rt_as], &[]);

        Self {
            instance_buffer,
            instance_memory,
            instance_mapped,
            tlas,
            tlas_buffer,
            tlas_memory,
            scratch_buffer,
            scratch_memory,
        }
    }

    /// Write updated articulated kinematic node instance matrices into the mapped instance buffer.
    pub unsafe fn write_instances(
        &mut self,
        geom_nodes: &[u32],
        blas_addresses: &[vk::DeviceAddress],
        model: Mat4,
        node_transform: impl Fn(usize) -> Mat4,
    ) {
        let instances =
            self.instance_mapped as *mut vk::AccelerationStructureInstanceKHR;
        for (i, node) in geom_nodes.iter().enumerate() {
            let m = model * node_transform(*node as usize);
            let c = m.to_cols_array();
            let mask: u8 = if *node == 2 || (*node >= 4 && *node <= 6) {
                0x02
            } else if *node == 3 || (*node >= 7 && *node <= 9) {
                0x04
            } else {
                0x01
            };
            let inst = vk::AccelerationStructureInstanceKHR {
                transform: vk::TransformMatrixKHR {
                    matrix: [
                        c[0], c[4], c[8], c[12],
                        c[1], c[5], c[9], c[13],
                        c[2], c[6], c[10], c[14],
                    ],
                },
                instance_custom_index_and_mask: vk::Packed24_8::new(*node as u32, mask),
                instance_shader_binding_table_record_offset_and_flags: vk::Packed24_8::new(
                    0,
                    vk::GeometryInstanceFlagsKHR::TRIANGLE_FACING_CULL_DISABLE.as_raw() as u8,
                ),
                acceleration_structure_reference: vk::AccelerationStructureReferenceKHR {
                    device_handle: blas_addresses[i],
                },
            };
            std::ptr::write(instances.add(i), inst);
        }
    }

    /// Record top-level acceleration structure build commands into the active frame command buffer.
    pub unsafe fn record_build(
        &self,
        device: &ash::Device,
        rt_loader: &khr::acceleration_structure::Device,
        cmd: vk::CommandBuffer,
        instance_count: u32,
    ) {
        let inst_address = device.get_buffer_device_address(
            &vk::BufferDeviceAddressInfo::default().buffer(self.instance_buffer),
        );
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
            &vk::BufferDeviceAddressInfo::default().buffer(self.scratch_buffer),
        );
        let tlas_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
            .ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL)
            .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
            .flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
            .geometries(std::slice::from_ref(&tlas_geometry))
            .dst_acceleration_structure(self.tlas)
            .scratch_data(vk::DeviceOrHostAddressKHR {
                device_address: scratch_address,
            });
        let range = vk::AccelerationStructureBuildRangeInfoKHR::default()
            .primitive_count(instance_count)
            .primitive_offset(0);
        rt_loader.cmd_build_acceleration_structures(cmd, &[tlas_info], &[&[range]]);

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

    /// Destroy TLAS and its associated instance and scratch buffers.
    pub unsafe fn destroy(
        &mut self,
        device: &ash::Device,
        rt_loader: &khr::acceleration_structure::Device,
    ) {
        if self.tlas != vk::AccelerationStructureKHR::null() {
            rt_loader.destroy_acceleration_structure(self.tlas, None);
            self.tlas = vk::AccelerationStructureKHR::null();
        }
        if self.instance_buffer != vk::Buffer::null() {
            device.unmap_memory(self.instance_memory);
            device.destroy_buffer(self.instance_buffer, None);
            device.free_memory(self.instance_memory, None);
            self.instance_buffer = vk::Buffer::null();
            self.instance_memory = vk::DeviceMemory::null();
            self.instance_mapped = std::ptr::null_mut();
        }
        if self.tlas_buffer != vk::Buffer::null() {
            device.destroy_buffer(self.tlas_buffer, None);
            device.free_memory(self.tlas_memory, None);
            self.tlas_buffer = vk::Buffer::null();
            self.tlas_memory = vk::DeviceMemory::null();
        }
        if self.scratch_buffer != vk::Buffer::null() {
            device.destroy_buffer(self.scratch_buffer, None);
            device.free_memory(self.scratch_memory, None);
            self.scratch_buffer = vk::Buffer::null();
            self.scratch_memory = vk::DeviceMemory::null();
        }
    }
}
