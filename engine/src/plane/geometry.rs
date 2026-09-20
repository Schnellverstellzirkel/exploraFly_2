//! Device geometry upload: one device-local vertex buffer, one index
//! buffer holding airframe + terrain + cloud indices, staged through a
//! single one-time submit.

use ash::vk;
use world;
use super::airframe_mesh::AirframeMesh;

/// Everything later stages and the struct need from the geometry upload.
pub(super) struct GeometryBuffers {
    pub(super) vertex_buffer: vk::Buffer,
    pub(super) vertex_memory: vk::DeviceMemory,
    pub(super) index_buffer: vk::Buffer,
    pub(super) index_memory: vk::DeviceMemory,
    pub(super) rt_vertex_address: vk::DeviceAddress,
    pub(super) rt_terrain_index_address: vk::DeviceAddress,
    pub(super) glass_first: u32,
    pub(super) glass_count: u32,
    pub(super) opaque_count: u32,
    pub(super) terrain_index_offset: u64,
    pub(super) cloud_index_offset: u64,
}

/// Allocate and bind one device-local buffer (DEVICE_ADDRESS capable).
/// Allocation size at or above which memory is dedicated to the resource:
/// the driver skips suballocation and places large pools (BLAS storage,
/// mesh buffers) directly, avoiding heap fragmentation.
pub(super) const DEDICATE_ABOVE: u64 = 16 * 1024 * 1024;

pub(super) unsafe fn upload_buffer(
device: &ash::Device,
mem_props: &vk::PhysicalDeviceMemoryProperties,
size: u64,
usage: vk::BufferUsageFlags,
) -> (vk::Buffer, vk::DeviceMemory) {
        let info = vk::BufferCreateInfo::default()
            .size(size)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let buffer = device.create_buffer(&info, None).expect("buffer");
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
            // Memory backing device-addressable buffers must be allocated
            // with the DEVICE_ADDRESS flag (VUID-vkBindBufferMemory-bufferDeviceAddress-03339).
            let mut addr_flags = vk::MemoryAllocateFlagsInfo::default()
                .flags(vk::MemoryAllocateFlags::DEVICE_ADDRESS);
            if req.size >= DEDICATE_ABOVE {
                let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().buffer(buffer);
                device
                    .allocate_memory(&alloc.push_next(&mut addr_flags).push_next(&mut dedicated), None)
                    .expect("mem")
            } else {
                device
                    .allocate_memory(&alloc.push_next(&mut addr_flags), None)
                    .expect("mem")
            }
        } else {
            if req.size >= DEDICATE_ABOVE {
                let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().buffer(buffer);
                device.allocate_memory(&alloc.push_next(&mut dedicated), None).expect("mem")
            } else {
                device.allocate_memory(&alloc, None).expect("mem")
            }
        };
        device.bind_buffer_memory(buffer, memory, 0).expect("bind");
        (buffer, memory)
}

pub(super) unsafe fn upload_geometry(
    device: &ash::Device,
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    queue_family: u32,
    queue: vk::Queue,
    rt_supported: bool,
    mesh: AirframeMesh,
) -> GeometryBuffers {
    let AirframeMesh { stream, opaque, glass, rt_idx: _, rt_geom_nodes: _, rt_node_ranges: _ } = mesh;
    let mem_props = instance.get_physical_device_memory_properties(physical);
        let mut vertex_usage = vk::BufferUsageFlags::VERTEX_BUFFER | vk::BufferUsageFlags::TRANSFER_DST;
        if rt_supported {
            vertex_usage |= vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
                | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR;
        }
        let (vertex_buffer, vertex_memory) = upload_buffer(device, &mem_props, stream.len() as u64, vertex_usage);
        let rt_vertex_address = if rt_supported {
            device
                .get_buffer_device_address(
                    &vk::BufferDeviceAddressInfo::default().buffer(vertex_buffer),
                )
        } else {
            0
        };
        // Opaque then glass in one index buffer.
        let mut indices = opaque;
        let glass_first = indices.len() as u32;
        let glass_count = glass.len() as u32;
        let opaque_count = glass_first;
        indices.extend_from_slice(&glass);
        let terrain_index_offset = ((indices.len() * 2 + 3) & !3) as u64;
        let terrain_indices = world::terrain_indices();
        let cloud_index_offset = terrain_index_offset + (terrain_indices.len() * 4) as u64;
        let cloud_indices = crate::clouds::indices();
        let index_bytes = cloud_index_offset as usize + cloud_indices.len() * 2;
        let mut index_usage = vk::BufferUsageFlags::INDEX_BUFFER | vk::BufferUsageFlags::TRANSFER_DST;
        if rt_supported {
            index_usage |= vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
                | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR;
        }
        let (index_buffer, index_memory) = upload_buffer(device, &mem_props, index_bytes as u64, index_usage);
        let rt_terrain_index_address = if rt_supported {
            device.get_buffer_device_address(
                &vk::BufferDeviceAddressInfo::default().buffer(index_buffer),
            ) + terrain_index_offset
        } else {
            0
        };
        let pool_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(queue_family)
            .flags(vk::CommandPoolCreateFlags::TRANSIENT);
        let pool = device.create_command_pool(&pool_info, None).expect("spool");
        let alloc = vk::CommandBufferAllocateInfo::default()
            .command_pool(pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let cmd = device.allocate_command_buffers(&alloc).expect("scmd")[0];
        let stage_info = vk::BufferCreateInfo::default()
            .size((stream.len() + index_bytes) as u64)
            .usage(vk::BufferUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let stage = device.create_buffer(&stage_info, None).expect("stage");
        let stage_req = device.get_buffer_memory_requirements(stage);
        let stage_index = crate::find_memory_type(
            &mem_props,
            stage_req.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        );
        let stage_alloc = vk::MemoryAllocateInfo::default()
            .allocation_size(stage_req.size)
            .memory_type_index(stage_index);
        let stage_mem = device.allocate_memory(&stage_alloc, None).expect("smem");
        device
            .bind_buffer_memory(stage, stage_mem, 0)
            .expect("sbind");
        let mapped = device
            .map_memory(stage_mem, 0, stage_req.size, vk::MemoryMapFlags::empty())
            .expect("smap") as *mut u8;
        std::ptr::copy_nonoverlapping(stream.as_ptr(), mapped, stream.len());
        std::ptr::copy_nonoverlapping(
            indices.as_ptr() as *const u8,
            mapped.add(stream.len()),
            indices.len() * 2,
        );
        std::ptr::copy_nonoverlapping(
            terrain_indices.as_ptr() as *const u8,
            mapped.add(stream.len() + terrain_index_offset as usize),
            terrain_indices.len() * 4,
        );
        std::ptr::copy_nonoverlapping(
            cloud_indices.as_ptr() as *const u8,
            mapped.add(stream.len() + cloud_index_offset as usize),
            cloud_indices.len() * 2,
        );
        device.unmap_memory(stage_mem);
        let begin = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        device.begin_command_buffer(cmd, &begin).expect("sbegin");
        let copy_v = vk::BufferCopy::default().size(stream.len() as u64);
        device.cmd_copy_buffer(cmd, stage, vertex_buffer, &[copy_v]);
        let copy_i = vk::BufferCopy::default()
            .src_offset(stream.len() as u64)
            .size(index_bytes as u64);
        device.cmd_copy_buffer(cmd, stage, index_buffer, &[copy_i]);
        device.end_command_buffer(cmd).expect("send");
        let fence_info = vk::FenceCreateInfo::default();
        let fence = device.create_fence(&fence_info, None).expect("sfence");
        let cmd_ref = [cmd];
        let submit = vk::SubmitInfo::default().command_buffers(&cmd_ref);
        device
            .queue_submit(queue, &[submit], fence)
            .expect("ssubmit");
        device
            .wait_for_fences(&[fence], true, u64::MAX)
            .expect("swait");
        device.destroy_fence(fence, None);
        device.destroy_command_pool(pool, None);
        device.destroy_buffer(stage, None);
        device.free_memory(stage_mem, None);
    GeometryBuffers {
        vertex_buffer,
        vertex_memory,
        index_buffer,
        index_memory,
        rt_vertex_address,
        rt_terrain_index_address,
        glass_first,
        glass_count,
        opaque_count,
        terrain_index_offset,
        cloud_index_offset,
    }
}
