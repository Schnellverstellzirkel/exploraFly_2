//! One-time ray-traced shadow resource construction (VK_KHR_ray_query):
//! per-node airframe BLAS plus terrain and landmark-structure casters, and
//! the loaders/addresses the per-frame TLAS needs.

use ash::khr;
use ash::vk;
use crate::ubo::align_256;
use super::VERTEX_BYTES;
use super::geometry::GeometryBuffers;
use super::airframe_mesh::AirframeMesh;

/// Everything the renderer keeps for ray-traced sun shadows.
pub(super) struct RtResources {
    pub(super) loader: khr::acceleration_structure::Device,
    pub(super) instance_count: u32,
    pub(super) vertex_address: vk::DeviceAddress,
    pub(super) index_buffer: vk::Buffer,
    pub(super) index_memory: vk::DeviceMemory,
    pub(super) index_address: vk::DeviceAddress,
    pub(super) blas: Vec<vk::AccelerationStructureKHR>,
    pub(super) blas_addresses: Vec<vk::DeviceAddress>,
    pub(super) geom_nodes: Vec<u32>,
    pub(super) blas_buffer: vk::Buffer,
    pub(super) blas_memory: vk::DeviceMemory,
    pub(super) terrain_vertex_buffer: vk::Buffer,
    pub(super) terrain_vertex_memory: vk::DeviceMemory,
    pub(super) terrain_blas_address: vk::DeviceAddress,
    pub(super) structures_vertex_buffer: vk::Buffer,
    pub(super) structures_vertex_memory: vk::DeviceMemory,
    pub(super) structures_blas_address: vk::DeviceAddress,
}

pub(super) unsafe fn build_rt(
    device: &ash::Device,
    instance: &ash::Instance,
    _physical: vk::PhysicalDevice,
    mem_props: &vk::PhysicalDeviceMemoryProperties,
    queue_family: u32,
    queue: vk::Queue,
    rt_supported: bool,
    geometry: &GeometryBuffers,
    mesh: &AirframeMesh,
) -> RtResources {
    let GeometryBuffers {
        vertex_buffer: _,
        index_buffer: _,
        rt_vertex_address,
        rt_terrain_index_address,
        opaque_count: _,
        ..
    } = *geometry;
    let stream = mesh.vertex_data;
    let rt_idx = mesh.rt_indices;
    let rt_geom_nodes: Vec<u32> = mesh.rt_node_iter().collect();
    let rt_node_ranges: Vec<(u32, u32)> = mesh.rt_range_iter().collect();
    // Ray-traced soft shadows (VK_KHR_ray_query): one bottom-level
    // structure per animated node, built once here into a shared
    // device-local buffer. Each frame slot references them through a
    // top-level structure updated with per-node instance transforms.
    let rt_loader = khr::acceleration_structure::Device::new(instance, device);
    let mut rt_blas_buffer = vk::Buffer::null();
    let mut rt_blas_memory = vk::DeviceMemory::null();
    let mut rt_blas = Vec::new();
    let mut rt_blas_addresses = Vec::new();
    let mut rt_index_buffer = vk::Buffer::null();
    let mut rt_index_memory = vk::DeviceMemory::null();
    let mut rt_index_address = 0;
    let mut rt_terrain_vertex_buffer = vk::Buffer::null();
    let mut rt_terrain_vertex_memory = vk::DeviceMemory::null();
    let mut rt_terrain_blas_address = 0;
    let mut rt_structures_vertex_buffer = vk::Buffer::null();
    let mut rt_structures_vertex_memory = vk::DeviceMemory::null();
    let mut rt_structures_blas_address = 0;
    const TERRAIN_RT_TILES: u32 = 1;
    const STRUCTURES_RT_INSTANCES: u32 = 1;
    let rt_instance_count = if rt_supported && !rt_geom_nodes.is_empty() {
        rt_geom_nodes.len() as u32 + TERRAIN_RT_TILES + STRUCTURES_RT_INSTANCES
    } else {
        0
    };
    if rt_supported && rt_instance_count > 0 {
        let rt_index_bytes = rt_idx.len() as u64;
        let (ribuf, rimem) = super::geometry::upload_buffer(device, &mem_props, 
            rt_index_bytes,
            vk::BufferUsageFlags::INDEX_BUFFER
                | vk::BufferUsageFlags::TRANSFER_DST
                | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
                | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR,
        );
        let rt_stage_info = vk::BufferCreateInfo::default()
            .size(rt_index_bytes)
            .usage(vk::BufferUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let rt_stage = device.create_buffer(&rt_stage_info, None).expect("rtstage");
        let rt_stage_req = device.get_buffer_memory_requirements(rt_stage);
        let rt_stage_index = crate::find_memory_type(
            &mem_props,
            rt_stage_req.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        );
        let rt_stage_alloc = vk::MemoryAllocateInfo::default()
            .allocation_size(rt_stage_req.size)
            .memory_type_index(rt_stage_index);
        let rt_stage_mem = device.allocate_memory(&rt_stage_alloc, None).expect("rtsmem");
        device.bind_buffer_memory(rt_stage, rt_stage_mem, 0).expect("rtsbind");
        let rt_stage_map = device
            .map_memory(rt_stage_mem, 0, rt_index_bytes, vk::MemoryMapFlags::empty())
            .expect("rtsmap") as *mut u8;
        if !rt_idx.is_empty() {
            std::ptr::copy_nonoverlapping(
                rt_idx.as_ptr(),
                rt_stage_map,
                rt_idx.len(),
            );
        }
        device.unmap_memory(rt_stage_mem);

        // Generate immutable world terrain vertices for RT BLAS
        let samples = world::terrain_samples_static();
        let grid_cells = world::TERRAIN_GRID_CELLS as usize;
        let stride = grid_cells + 1;
        let mut terrain_verts: Vec<f32> = Vec::with_capacity(stride * stride * 3);
        for z in 0..stride {
            for x in 0..stride {
                let cx = x & (grid_cells - 1);
                let cz = z & (grid_cells - 1);
                let h = samples[cz * grid_cells + cx][0].max(world::WATER_LEVEL);
                terrain_verts.push(x as f32 * world::TERRAIN_CELL_METRES);
                terrain_verts.push(h);
                terrain_verts.push(z as f32 * world::TERRAIN_CELL_METRES);
            }
        }
        let terrain_vert_bytes = (terrain_verts.len() * 4) as u64;
        let (tvbuf, tvmem) = super::geometry::upload_buffer(device, &mem_props, 
            terrain_vert_bytes,
            vk::BufferUsageFlags::VERTEX_BUFFER
                | vk::BufferUsageFlags::TRANSFER_DST
                | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
                | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR,
        );
        let tv_stage_info = vk::BufferCreateInfo::default()
            .size(terrain_vert_bytes)
            .usage(vk::BufferUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let tv_stage = device.create_buffer(&tv_stage_info, None).expect("tvstage");
        let tv_stage_req = device.get_buffer_memory_requirements(tv_stage);
        let tv_stage_index = crate::find_memory_type(
            &mem_props,
            tv_stage_req.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        );
        let tv_stage_alloc = vk::MemoryAllocateInfo::default()
            .allocation_size(tv_stage_req.size)
            .memory_type_index(tv_stage_index);
        let tv_stage_mem = device.allocate_memory(&tv_stage_alloc, None).expect("tvsmem");
        device.bind_buffer_memory(tv_stage, tv_stage_mem, 0).expect("tvsbind");
        let tv_stage_map = device
            .map_memory(tv_stage_mem, 0, terrain_vert_bytes, vk::MemoryMapFlags::empty())
            .expect("tvsmap") as *mut u8;
        std::ptr::copy_nonoverlapping(
            terrain_verts.as_ptr() as *const u8,
            tv_stage_map,
            terrain_verts.len() * 4,
        );
        device.unmap_memory(tv_stage_mem);

        let structure_verts = world::landmark_structure_triangles();
        let structure_vert_bytes = (structure_verts.len() * 4) as u64;
        let (svbuf, svmem) = super::geometry::upload_buffer(device, &mem_props, 
            structure_vert_bytes,
            vk::BufferUsageFlags::VERTEX_BUFFER
                | vk::BufferUsageFlags::TRANSFER_DST
                | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
                | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR,
        );
        let sv_stage_info = vk::BufferCreateInfo::default()
            .size(structure_vert_bytes)
            .usage(vk::BufferUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let sv_stage = device.create_buffer(&sv_stage_info, None).expect("svstage");
        let sv_stage_req = device.get_buffer_memory_requirements(sv_stage);
        let sv_stage_index = crate::find_memory_type(
            &mem_props,
            sv_stage_req.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        );
        let sv_stage_alloc = vk::MemoryAllocateInfo::default()
            .allocation_size(sv_stage_req.size)
            .memory_type_index(sv_stage_index);
        let sv_stage_mem = device.allocate_memory(&sv_stage_alloc, None).expect("svsmem");
        device.bind_buffer_memory(sv_stage, sv_stage_mem, 0).expect("svsbind");
        let sv_stage_map = device
            .map_memory(sv_stage_mem, 0, structure_vert_bytes, vk::MemoryMapFlags::empty())
            .expect("svsmap") as *mut u8;
        std::ptr::copy_nonoverlapping(
            structure_verts.as_ptr() as *const u8,
            sv_stage_map,
            structure_verts.len() * 4,
        );
        device.unmap_memory(sv_stage_mem);

        let rt_pool_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(queue_family)
            .flags(vk::CommandPoolCreateFlags::TRANSIENT);
        let rt_pool = device.create_command_pool(&rt_pool_info, None).expect("rtpool");
        let rt_alloc = vk::CommandBufferAllocateInfo::default()
            .command_pool(rt_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let rt_cmd = device.allocate_command_buffers(&rt_alloc).expect("rtcmd")[0];
        let rt_begin = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        device.begin_command_buffer(rt_cmd, &rt_begin).expect("rtcbegin");
        let rt_copy = vk::BufferCopy::default().size(rt_index_bytes);
        device.cmd_copy_buffer(rt_cmd, rt_stage, ribuf, &[rt_copy]);
        let tv_copy = vk::BufferCopy::default().size(terrain_vert_bytes);
        device.cmd_copy_buffer(rt_cmd, tv_stage, tvbuf, &[tv_copy]);
        let sv_copy = vk::BufferCopy::default().size(structure_vert_bytes);
        device.cmd_copy_buffer(rt_cmd, sv_stage, svbuf, &[sv_copy]);
        device.end_command_buffer(rt_cmd).expect("rtcend");
        let rt_fence_info = vk::FenceCreateInfo::default();
        let rt_fence = device.create_fence(&rt_fence_info, None).expect("rtfence");
        let rt_cmd_ref = [rt_cmd];
        let rt_submit = vk::SubmitInfo::default().command_buffers(&rt_cmd_ref);
        device.queue_submit(queue, &[rt_submit], rt_fence).expect("rtsubmit");
        device.wait_for_fences(&[rt_fence], true, u64::MAX).expect("rtfwait");
        device.destroy_fence(rt_fence, None);
        device.destroy_command_pool(rt_pool, None);
        device.destroy_buffer(rt_stage, None);
        device.free_memory(rt_stage_mem, None);
        device.destroy_buffer(tv_stage, None);
        device.free_memory(tv_stage_mem, None);
        device.destroy_buffer(sv_stage, None);
        device.free_memory(sv_stage_mem, None);
        rt_index_buffer = ribuf;
        rt_index_memory = rimem;
        rt_index_address = device
            .get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(ribuf));
        rt_terrain_vertex_buffer = tvbuf;
        rt_terrain_vertex_memory = tvmem;
        let rt_terrain_vertex_address = device
            .get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(tvbuf));
        rt_structures_vertex_buffer = svbuf;
        rt_structures_vertex_memory = svmem;
        let rt_structures_vertex_address = device
            .get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(svbuf));

        // Triangle geometries for the casters: airframe node ranges + terrain mesh + landmark structures.
        let mut geoms: Vec<vk::AccelerationStructureGeometryKHR> =
            Vec::with_capacity(rt_geom_nodes.len() + 2);
        let mut ranges: Vec<vk::AccelerationStructureBuildRangeInfoKHR> =
            Vec::with_capacity(rt_geom_nodes.len() + 2);
        let max_vertex = (stream.len() / VERTEX_BYTES) as u32 - 1;
        for &(off, cnt) in rt_node_ranges.iter() {
            let triangles = vk::AccelerationStructureGeometryTrianglesDataKHR::default()
                .vertex_format(vk::Format::R32G32B32_SFLOAT)
                .vertex_data(vk::DeviceOrHostAddressConstKHR {
                    device_address: rt_vertex_address,
                })
                .vertex_stride(VERTEX_BYTES as u64)
                .max_vertex(max_vertex)
                .index_type(vk::IndexType::UINT16)
                    .index_data(vk::DeviceOrHostAddressConstKHR {
                        device_address: rt_index_address + (off as u64) * 2,
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
        // Add terrain mesh geometry.
        let terrain_triangles = vk::AccelerationStructureGeometryTrianglesDataKHR::default()
            .vertex_format(vk::Format::R32G32B32_SFLOAT)
            .vertex_data(vk::DeviceOrHostAddressConstKHR {
                device_address: rt_terrain_vertex_address,
            })
            .vertex_stride(12)
            .max_vertex(world::TERRAIN_VERTEX_COUNT - 1)
            .index_type(vk::IndexType::UINT32)
            .index_data(vk::DeviceOrHostAddressConstKHR {
                device_address: rt_terrain_index_address,
            });
        geoms.push(
            vk::AccelerationStructureGeometryKHR::default()
                .geometry_type(vk::GeometryTypeKHR::TRIANGLES)
                .geometry(vk::AccelerationStructureGeometryDataKHR { triangles: terrain_triangles })
                .flags(vk::GeometryFlagsKHR::OPAQUE),
        );
        ranges.push(
            vk::AccelerationStructureBuildRangeInfoKHR::default()
                .primitive_count(world::TERRAIN_INDEX_COUNT / 3)
                .primitive_offset(0)
                .first_vertex(0),
        );
        // Add landmark structures mesh as BLAS geometry.
        let structure_tri_count = (structure_verts.len() / 9) as u32;
        let structure_triangles = vk::AccelerationStructureGeometryTrianglesDataKHR::default()
            .vertex_format(vk::Format::R32G32B32_SFLOAT)
            .vertex_data(vk::DeviceOrHostAddressConstKHR {
                device_address: rt_structures_vertex_address,
            })
            .vertex_stride(12)
            .max_vertex((structure_verts.len() / 3) as u32 - 1)
            .index_type(vk::IndexType::NONE_KHR);
        geoms.push(
            vk::AccelerationStructureGeometryKHR::default()
                .geometry_type(vk::GeometryTypeKHR::TRIANGLES)
                .geometry(vk::AccelerationStructureGeometryDataKHR { triangles: structure_triangles })
                .flags(vk::GeometryFlagsKHR::OPAQUE),
        );
        ranges.push(
            vk::AccelerationStructureBuildRangeInfoKHR::default()
                .primitive_count(structure_tri_count)
                .primitive_offset(0)
                .first_vertex(0),
        );

        // Query the hardware sizes, then pack all BLAS into one buffer.
        let mut sizes: Vec<vk::AccelerationStructureBuildSizesInfoKHR> =
            (0..geoms.len()).map(|_| Default::default()).collect();
        for (i, g) in geoms.iter().enumerate() {
            let build_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
                .ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL)
                .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
                .flags(
                    vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE
                        | vk::BuildAccelerationStructureFlagsKHR::ALLOW_COMPACTION,
                )
                .geometries(std::slice::from_ref(g));
            rt_loader.get_acceleration_structure_build_sizes(
                vk::AccelerationStructureBuildTypeKHR::DEVICE,
                &build_info,
                &[ranges[i].primitive_count],
                &mut sizes[i],
            );
        }
        // Scratch regions must not overlap within one
        // cmd_build_acceleration_structures call (`VUID-...-scratchData-03704`),
        // so give each node its own 256-aligned slot in a summed buffer.
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
        let (abuf, amem) = super::geometry::upload_buffer(device, &mem_props, 
            total_as,
            vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR
                | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
        );
        let (sbuf, smem) = super::geometry::upload_buffer(device, &mem_props, 
            scratch_bytes,
            vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
        );
        let as_address = device
            .get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(abuf));
        let scratch_address = device
            .get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(sbuf));
        let mut as_offset = 0u64;
        for (i, s) in sizes.iter().enumerate() {
            let ci = vk::AccelerationStructureCreateInfoKHR::default()
                .create_flags(vk::AccelerationStructureCreateFlagsKHR::empty())
                .buffer(abuf)
                .offset(as_offset)
                .size(s.acceleration_structure_size)
                .ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL);
            rt_blas.push(rt_loader.create_acceleration_structure(&ci, None).expect("blas"));
            rt_blas_addresses.push(as_address + as_offset);
            as_offset = align_256(as_offset + s.acceleration_structure_size);
            let _ = i;
        }
        // Build every BLAS in one command.
        let rt_pool_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(queue_family)
            .flags(vk::CommandPoolCreateFlags::TRANSIENT);
        let rt_pool = device.create_command_pool(&rt_pool_info, None).expect("rtpool2");
        let rt_alloc = vk::CommandBufferAllocateInfo::default()
            .command_pool(rt_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let rt_cmd = device.allocate_command_buffers(&rt_alloc).expect("rtcmd2")[0];
        let rt_begin = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        device.begin_command_buffer(rt_cmd, &rt_begin).expect("rtcbegin2");
        let mut build_infos: Vec<vk::AccelerationStructureBuildGeometryInfoKHR> = Vec::with_capacity(geoms.len());
        let mut build_ranges: Vec<Vec<vk::AccelerationStructureBuildRangeInfoKHR>> = Vec::with_capacity(geoms.len());
        for (i, g) in geoms.iter().enumerate() {
            build_infos.push(
                vk::AccelerationStructureBuildGeometryInfoKHR::default()
                    .ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL)
                    .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
                    .flags(
                        vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE
                            | vk::BuildAccelerationStructureFlagsKHR::ALLOW_COMPACTION,
                    )
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
        rt_loader.cmd_build_acceleration_structures(rt_cmd, &build_infos, &build_range_refs);
        let build_barrier = vk::MemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_WRITE_KHR)
            .dst_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_READ_KHR);
        device.cmd_pipeline_barrier(
            rt_cmd,
            vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
            vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
            vk::DependencyFlags::empty(),
            &[build_barrier],
            &[],
            &[],
        );
        device.end_command_buffer(rt_cmd).expect("rtcend2");
        let rt_fence = device.create_fence(&rt_fence_info, None).expect("rtfence2");
        let rt_cmd_ref = [rt_cmd];
        let rt_submit = vk::SubmitInfo::default().command_buffers(&rt_cmd_ref);
        device.queue_submit(queue, &[rt_submit], rt_fence).expect("rtsubmit2");
        device.wait_for_fences(&[rt_fence], true, u64::MAX).expect("rtfwait2");
        device.destroy_fence(rt_fence, None);
        device.destroy_command_pool(rt_pool, None);

        // The build scratch buffer is dead the moment the build fence
        // signals; keeping it resident only displaces VRAM the TLAS
        // traversal and the frame targets want back.
        device.destroy_buffer(sbuf, None);
        device.free_memory(smem, None);

        // Compaction pass (VK_KHR_acceleration_structure): PREFER_FAST_TRACE
        // builds reserve rebuild slack the casters never use again. Query the
        // compacted sizes, repack into a second pool, and swap the handles and
        // device addresses the TLAS instances reference. Geometry is copied
        // verbatim, so every shadow ray hits the same triangles.
        let query_info = vk::QueryPoolCreateInfo::default()
            .query_type(vk::QueryType::ACCELERATION_STRUCTURE_COMPACTED_SIZE_KHR)
            .query_count(rt_blas.len() as u32);
        let size_query = device.create_query_pool(&query_info, None).expect("rtqpool");
        let cp_pool_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(queue_family)
            .flags(vk::CommandPoolCreateFlags::TRANSIENT);
        let cp_pool = device.create_command_pool(&cp_pool_info, None).expect("rtcpool");
        let cp_alloc = vk::CommandBufferAllocateInfo::default()
            .command_pool(cp_pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(2);
        let cp_cmds = device.allocate_command_buffers(&cp_alloc).expect("rtccmd");
        let as_build_read_barrier = vk::MemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_WRITE_KHR)
            .dst_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_READ_KHR);

        // Submit 1: write compacted sizes for every caster.
        device.begin_command_buffer(cp_cmds[0], &rt_begin).expect("rtcqbegin");
        device.cmd_reset_query_pool(cp_cmds[0], size_query, 0, rt_blas.len() as u32);
        device.cmd_pipeline_barrier(
            cp_cmds[0],
            vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
            vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
            vk::DependencyFlags::empty(),
            &[as_build_read_barrier],
            &[],
            &[],
        );
        rt_loader.cmd_write_acceleration_structures_properties(
            cp_cmds[0],
            &rt_blas,
            vk::QueryType::ACCELERATION_STRUCTURE_COMPACTED_SIZE_KHR,
            size_query,
            0,
        );
        device.end_command_buffer(cp_cmds[0]).expect("rtcqend");
        let q_fence = device.create_fence(&rt_fence_info, None).expect("rtqfence");
        let q_submit = vk::SubmitInfo::default().command_buffers(&cp_cmds[0..1]);
        device.queue_submit(queue, &[q_submit], q_fence).expect("rtqsubmit");
        device.wait_for_fences(&[q_fence], true, u64::MAX).expect("rtqwait");
        device.destroy_fence(q_fence, None);
        let mut compact_sizes = vec![0u64; rt_blas.len()];
        device.get_query_pool_results(
            size_query,
            0,
            &mut compact_sizes,
            vk::QueryResultFlags::TYPE_64 | vk::QueryResultFlags::WAIT,
        )
        .expect("rtqresults");
        device.destroy_query_pool(size_query, None);

        // Repack compacted casters contiguously.
        let mut compact_offset = 0u64;
        let mut compact_offsets = Vec::with_capacity(rt_blas.len());
        for s in &compact_sizes {
            compact_offsets.push(compact_offset);
            compact_offset = align_256(compact_offset + s);
        }
        let (cbuf, cmem) = super::geometry::upload_buffer(
            device,
            mem_props,
            compact_offset.max(256),
            vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR
                | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
        );
        let _c_address = device
            .get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(cbuf));
        let mut compacted = Vec::with_capacity(rt_blas.len());
        let mut compact_addresses = Vec::with_capacity(rt_blas.len());
        for (i, s) in compact_sizes.iter().enumerate() {
            let ci = vk::AccelerationStructureCreateInfoKHR::default()
                .create_flags(vk::AccelerationStructureCreateFlagsKHR::empty())
                .buffer(cbuf)
                .offset(compact_offsets[i])
                .size(*s)
                .ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL);
            let handle = rt_loader
                .create_acceleration_structure(&ci, None)
                .expect("compact blas");
            compact_addresses.push(
                rt_loader.get_acceleration_structure_device_address(
                    &vk::AccelerationStructureDeviceAddressInfoKHR::default()
                        .acceleration_structure(handle),
                ),
            );
            compacted.push(handle);
        }
        device.begin_command_buffer(cp_cmds[1], &rt_begin).expect("rtccbegin");
        let copies: Vec<vk::CopyAccelerationStructureInfoKHR> = rt_blas
            .iter()
            .zip(compacted.iter())
            .map(|(src, dst)| {
                vk::CopyAccelerationStructureInfoKHR::default()
                    .src(*src)
                    .dst(*dst)
                    .mode(vk::CopyAccelerationStructureModeKHR::COMPACT)
            })
            .collect();
        for copy in &copies {
            rt_loader.cmd_copy_acceleration_structure(cp_cmds[1], copy);
        }
        device.cmd_pipeline_barrier(
            cp_cmds[1],
            vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
            vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
            vk::DependencyFlags::empty(),
            &[as_build_read_barrier],
            &[],
            &[],
        );
        device.end_command_buffer(cp_cmds[1]).expect("rtccend");
        let c_fence = device.create_fence(&rt_fence_info, None).expect("rtcfence");
        let c_submit = vk::SubmitInfo::default().command_buffers(&cp_cmds[1..2]);
        device.queue_submit(queue, &[c_submit], c_fence).expect("rtcsubmit");
        device.wait_for_fences(&[c_fence], true, u64::MAX).expect("rtcwait");
        device.destroy_fence(c_fence, None);
        device.destroy_command_pool(cp_pool, None);

        for handle in &rt_blas {
            rt_loader.destroy_acceleration_structure(*handle, None);
        }
        device.destroy_buffer(abuf, None);
        device.free_memory(amem, None);
        rt_blas = compacted;
        rt_blas_addresses = compact_addresses;
        rt_terrain_blas_address = rt_blas_addresses[rt_geom_nodes.len()];
        rt_structures_blas_address = rt_blas_addresses[rt_geom_nodes.len() + 1];
        rt_blas_buffer = cbuf;
        rt_blas_memory = cmem;
        let compact_total: u64 = compact_sizes.iter().sum();
        println!(
            "RT: {} airframe BLAS + terrain (2.1M tris) + structures ({} tris), {:.1} KiB casters, structures compacted {:.1} -> {:.1} MiB",
            rt_geom_nodes.len(),
            structure_tri_count,
            (rt_index_bytes + structure_vert_bytes) as f32 / 1024.0,
            total_as as f32 / (1024.0 * 1024.0),
            compact_total as f32 / (1024.0 * 1024.0),
        );
    }

    RtResources {
        loader: rt_loader,
        instance_count: rt_instance_count,
        vertex_address: rt_vertex_address,
        index_buffer: rt_index_buffer,
        index_memory: rt_index_memory,
        index_address: rt_index_address,
        blas: rt_blas,
        blas_addresses: rt_blas_addresses,
        geom_nodes: rt_geom_nodes,
        blas_buffer: rt_blas_buffer,
        blas_memory: rt_blas_memory,
        terrain_vertex_buffer: rt_terrain_vertex_buffer,
        terrain_vertex_memory: rt_terrain_vertex_memory,
        terrain_blas_address: rt_terrain_blas_address,
        structures_vertex_buffer: rt_structures_vertex_buffer,
        structures_vertex_memory: rt_structures_vertex_memory,
        structures_blas_address: rt_structures_blas_address,
    }
}
