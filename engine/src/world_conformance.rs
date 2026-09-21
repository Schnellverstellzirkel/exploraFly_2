//! Headless GPU conformance for the Rust/GLSL world recipe.
//!
//! This is a diagnostic executable path rather than a frame-loop test: it
//! dispatches 8,192 coordinates through the actual Vulkan shader compiler and
//! compares the result with the public Rust world functions. It catches drift
//! in equations that cannot be represented by generated constants alone.

use ash::{vk, Entry};
use std::ffi::CString;

const SAMPLE_COUNT: usize = 8_192;
const WORKGROUP_SIZE: u32 = 64;

fn vk_error(context: &str, error: vk::Result) -> String {
    format!("{context}: {error:?}")
}

unsafe fn host_memory_type(
    properties: &vk::PhysicalDeviceMemoryProperties,
    type_bits: u32,
) -> Result<u32, String> {
    let required = vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
    for index in 0..properties.memory_type_count {
        if (type_bits & (1 << index)) != 0
            && properties.memory_types[index as usize]
                .property_flags
                .contains(required)
        {
            return Ok(index);
        }
    }
    Err("no host-visible coherent memory type for conformance buffers".into())
}

unsafe fn make_buffer(
    device: &ash::Device,
    properties: &vk::PhysicalDeviceMemoryProperties,
    size: u64,
) -> Result<(vk::Buffer, vk::DeviceMemory), String> {
    let info = vk::BufferCreateInfo::default()
        .size(size)
        .usage(vk::BufferUsageFlags::STORAGE_BUFFER)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let buffer = device
        .create_buffer(&info, None)
        .map_err(|e| vk_error("create conformance buffer", e))?;
    let requirements = device.get_buffer_memory_requirements(buffer);
    let memory_type = host_memory_type(properties, requirements.memory_type_bits)?;
    let allocation = vk::MemoryAllocateInfo::default()
        .allocation_size(requirements.size)
        .memory_type_index(memory_type);
    let memory = device
        .allocate_memory(&allocation, None)
        .map_err(|e| vk_error("allocate conformance buffer", e))?;
    device
        .bind_buffer_memory(buffer, memory, 0)
        .map_err(|e| vk_error("bind conformance buffer", e))?;
    Ok((buffer, memory))
}

type ConformanceInputs = (
    Vec<[f32; 4]>,
    Vec<[f32; 4]>,
    Vec<[f32; 4]>,
    Vec<[f32; 4]>,
);

fn inputs() -> ConformanceInputs {
    let mut coordinates = Vec::with_capacity(SAMPLE_COUNT);
    let mut moisture = Vec::with_capacity(SAMPLE_COUNT);
    let mut expected = Vec::with_capacity(SAMPLE_COUNT);
    let mut ocean_expected = Vec::with_capacity(SAMPLE_COUNT);
    for i in 0..SAMPLE_COUNT {
        // Keep coordinates inside one exact f32 world period while covering
        // negative-equivalent-looking phase boundaries and fractional cells.
        let x = ((i as u32).wrapping_mul(7919) % 65_536) as f32 + (i % 17) as f32 * 0.125;
        let z = ((i as u32).wrapping_mul(1543).wrapping_add(31_337) % 65_536) as f32
            + (i % 13) as f32 * 0.1875;
        let x = x.rem_euclid(world::WORLD_PERIOD as f32);
        let z = z.rem_euclid(world::WORLD_PERIOD as f32);
        let height = world::height_at(x as f64, z as f64);
        let slope = ((i * 37 % 1000) as f32 / 1000.0) * 0.95;
        let wetness = ((i * 61 % 1000) as f32 / 1000.0).clamp(0.0, 1.0);
        coordinates.push([x, z, height, slope]);
        moisture.push([wetness, 0.0, 0.0, 0.0]);
        expected.push([
            height,
            height.max(world::WATER_LEVEL),
            world::vegetation::tree_presence_probability_at(height, slope, wetness, x, z),
            world::vegetation::forest_cover(height, slope, wetness),
        ]);
        ocean_expected.push([world::ocean_mask_at(x as f64, z as f64), 0.0, 0.0, 0.0]);
    }
    (coordinates, moisture, expected, ocean_expected)
}

/// Run the Vulkan world-equation comparison and return a human-readable error
/// instead of silently accepting a stale shader recipe.
pub unsafe fn run() -> Result<(), String> {
    let entry = Entry::load().map_err(|e| format!("load Vulkan loader: {e}"))?;
    let app_name = CString::new("explora-world-conformance").unwrap();
    let app_info = vk::ApplicationInfo::default()
        .application_name(&app_name)
        .application_version(1)
        .engine_name(&app_name)
        .engine_version(1)
        .api_version(vk::API_VERSION_1_3);
    let instance = entry
        .create_instance(
            &vk::InstanceCreateInfo::default().application_info(&app_info),
            None,
        )
        .map_err(|e| vk_error("create conformance instance", e))?;

    let physical = instance
        .enumerate_physical_devices()
        .map_err(|e| vk_error("enumerate conformance devices", e))?
        .into_iter()
        .find_map(|candidate| {
            instance
                .get_physical_device_queue_family_properties(candidate)
                .iter()
                .enumerate()
                .find(|(_, family)| family.queue_flags.contains(vk::QueueFlags::COMPUTE))
                .map(|(index, _)| (candidate, index as u32))
        })
        .ok_or_else(|| "no Vulkan compute queue for world conformance".to_string())?;
    let (physical, queue_family) = physical;
    let memory_properties = instance.get_physical_device_memory_properties(physical);
    let priority = [1.0f32];
    let queue_info = vk::DeviceQueueCreateInfo::default()
        .queue_family_index(queue_family)
        .queue_priorities(&priority);
    let device = instance
        .create_device(
            physical,
            &vk::DeviceCreateInfo::default().queue_create_infos(std::slice::from_ref(&queue_info)),
            None,
        )
        .map_err(|e| vk_error("create conformance device", e))?;
    let queue = device.get_device_queue(queue_family, 0);

    let bytes = (SAMPLE_COUNT * std::mem::size_of::<[f32; 4]>()) as u64;
    let (coordinates_buffer, coordinates_memory) = make_buffer(&device, &memory_properties, bytes)?;
    let (moisture_buffer, moisture_memory) = make_buffer(&device, &memory_properties, bytes)?;
    let (results_buffer, results_memory) = make_buffer(&device, &memory_properties, bytes)?;
    let (ocean_buffer, ocean_memory) = make_buffer(&device, &memory_properties, bytes)?;
    let (coordinates, moisture, expected, ocean_expected) = inputs();

    let write_buffer = |memory: vk::DeviceMemory, data: &[[f32; 4]]| -> Result<(), String> {
        let mapped = device
            .map_memory(memory, 0, bytes, vk::MemoryMapFlags::empty())
            .map_err(|e| vk_error("map conformance input", e))?;
        std::ptr::copy_nonoverlapping(
            data.as_ptr().cast::<u8>(),
            mapped.cast::<u8>(),
            bytes as usize,
        );
        device.unmap_memory(memory);
        Ok(())
    };
    write_buffer(coordinates_memory, &coordinates)?;
    write_buffer(moisture_memory, &moisture)?;

    let bindings = [
        vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::COMPUTE),
        vk::DescriptorSetLayoutBinding::default()
            .binding(1)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::COMPUTE),
        vk::DescriptorSetLayoutBinding::default()
            .binding(2)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::COMPUTE),
        vk::DescriptorSetLayoutBinding::default()
            .binding(3)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::COMPUTE),
    ];
    let set_layout = device
        .create_descriptor_set_layout(
            &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
            None,
        )
        .map_err(|e| vk_error("create conformance descriptor layout", e))?;
    let pool_size = [vk::DescriptorPoolSize::default()
        .ty(vk::DescriptorType::STORAGE_BUFFER)
        .descriptor_count(4)];
    let pool = device
        .create_descriptor_pool(
            &vk::DescriptorPoolCreateInfo::default()
                .max_sets(1)
                .pool_sizes(&pool_size),
            None,
        )
        .map_err(|e| vk_error("create conformance descriptor pool", e))?;
    let set = device
        .allocate_descriptor_sets(
            &vk::DescriptorSetAllocateInfo::default()
                .descriptor_pool(pool)
                .set_layouts(std::slice::from_ref(&set_layout)),
        )
        .map_err(|e| vk_error("allocate conformance descriptor set", e))?[0];
    let coordinate_info = [vk::DescriptorBufferInfo::default()
        .buffer(coordinates_buffer)
        .range(bytes)];
    let moisture_info = [vk::DescriptorBufferInfo::default()
        .buffer(moisture_buffer)
        .range(bytes)];
    let result_info = [vk::DescriptorBufferInfo::default()
        .buffer(results_buffer)
        .range(bytes)];
    let ocean_info = [vk::DescriptorBufferInfo::default()
        .buffer(ocean_buffer)
        .range(bytes)];
    let writes = [
        vk::WriteDescriptorSet::default()
            .dst_set(set)
            .dst_binding(0)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .buffer_info(&coordinate_info),
        vk::WriteDescriptorSet::default()
            .dst_set(set)
            .dst_binding(1)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .buffer_info(&moisture_info),
        vk::WriteDescriptorSet::default()
            .dst_set(set)
            .dst_binding(2)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .buffer_info(&result_info),
        vk::WriteDescriptorSet::default()
            .dst_set(set)
            .dst_binding(3)
            .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
            .buffer_info(&ocean_info),
    ];
    device.update_descriptor_sets(&writes, &[]);

    let layout = device
        .create_pipeline_layout(
            &vk::PipelineLayoutCreateInfo::default().set_layouts(std::slice::from_ref(&set_layout)),
            None,
        )
        .map_err(|e| vk_error("create conformance pipeline layout", e))?;
    let words = crate::spv_words(include_bytes!(concat!(
        env!("OUT_DIR"),
        "/world_conformance.comp.spv"
    )));
    let shader = device
        .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&words), None)
        .map_err(|e| vk_error("create conformance shader module", e))?;
    let stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::COMPUTE)
        .module(shader)
        .name(c"main");
    let pipeline = device
        .create_compute_pipelines(
            vk::PipelineCache::null(),
            &[vk::ComputePipelineCreateInfo::default()
                .stage(stage)
                .layout(layout)],
            None,
        )
        .map_err(|(_, e)| vk_error("create conformance pipeline", e))?[0];

    let command_pool = device
        .create_command_pool(
            &vk::CommandPoolCreateInfo::default()
                .queue_family_index(queue_family)
                .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
            None,
        )
        .map_err(|e| vk_error("create conformance command pool", e))?;
    let command = device
        .allocate_command_buffers(
            &vk::CommandBufferAllocateInfo::default()
                .command_pool(command_pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1),
        )
        .map_err(|e| vk_error("allocate conformance command buffer", e))?[0];
    device
        .begin_command_buffer(command, &vk::CommandBufferBeginInfo::default())
        .map_err(|e| vk_error("begin conformance command buffer", e))?;
    device.cmd_bind_pipeline(command, vk::PipelineBindPoint::COMPUTE, pipeline);
    device.cmd_bind_descriptor_sets(
        command,
        vk::PipelineBindPoint::COMPUTE,
        layout,
        0,
        &[set],
        &[],
    );
    device.cmd_dispatch(
        command,
        (SAMPLE_COUNT as u32).div_ceil(WORKGROUP_SIZE),
        1,
        1,
    );
    let host_barrier = [vk::MemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::SHADER_WRITE)
        .dst_access_mask(vk::AccessFlags::HOST_READ)];
    device.cmd_pipeline_barrier(
        command,
        vk::PipelineStageFlags::COMPUTE_SHADER,
        vk::PipelineStageFlags::HOST,
        vk::DependencyFlags::empty(),
        &host_barrier,
        &[],
        &[],
    );
    device
        .end_command_buffer(command)
        .map_err(|e| vk_error("end conformance command buffer", e))?;
    let fence = device
        .create_fence(&vk::FenceCreateInfo::default(), None)
        .map_err(|e| vk_error("create conformance fence", e))?;
    let submit = vk::SubmitInfo::default().command_buffers(std::slice::from_ref(&command));
    device
        .queue_submit(queue, &[submit], fence)
        .map_err(|e| vk_error("submit conformance dispatch", e))?;
    device
        .wait_for_fences(&[fence], true, u64::MAX)
        .map_err(|e| vk_error("wait for conformance dispatch", e))?;
    let mapped = device
        .map_memory(results_memory, 0, bytes, vk::MemoryMapFlags::empty())
        .map_err(|e| vk_error("map conformance results", e))?;
    let gpu = std::slice::from_raw_parts(mapped.cast::<[f32; 4]>(), SAMPLE_COUNT).to_vec();
    device.unmap_memory(results_memory);
    let mapped = device
        .map_memory(ocean_memory, 0, bytes, vk::MemoryMapFlags::empty())
        .map_err(|e| vk_error("map ocean conformance results", e))?;
    let gpu_ocean = std::slice::from_raw_parts(mapped.cast::<[f32; 4]>(), SAMPLE_COUNT).to_vec();
    device.unmap_memory(ocean_memory);

    let tolerances = [0.02f32, 0.02, 0.00002, 0.00002];
    let mut maximum = [0.0f32; 5];
    let mut failure = None;
    for (index, (actual, reference)) in gpu.iter().zip(&expected).enumerate() {
        for channel in 0..4 {
            let error = (actual[channel] - reference[channel]).abs();
            maximum[channel] = maximum[channel].max(error);
            if error > tolerances[channel] && failure.is_none() {
                failure = Some(format!(
                    "sample {index}, channel {channel}: GPU {:.8}, Rust {:.8}, error {:.8} > {:.8}",
                    actual[channel], reference[channel], error, tolerances[channel]
                ));
            }
        }
    }
    for (index, (actual, reference)) in gpu_ocean.iter().zip(&ocean_expected).enumerate() {
        let error = (actual[0] - reference[0]).abs();
        maximum[4] = maximum[4].max(error);
        if error > 0.00002 && failure.is_none() {
            failure = Some(format!(
                "sample {index}, ocean mask: GPU {:.8}, Rust {:.8}, error {:.8} > {:.8}",
                actual[0], reference[0], error, 0.00002
            ));
        }
    }

    device.destroy_fence(fence, None);
    device.destroy_command_pool(command_pool, None);
    device.destroy_pipeline(pipeline, None);
    device.destroy_shader_module(shader, None);
    device.destroy_pipeline_layout(layout, None);
    device.destroy_descriptor_pool(pool, None);
    device.destroy_descriptor_set_layout(set_layout, None);
    device.destroy_buffer(coordinates_buffer, None);
    device.free_memory(coordinates_memory, None);
    device.destroy_buffer(moisture_buffer, None);
    device.free_memory(moisture_memory, None);
    device.destroy_buffer(results_buffer, None);
    device.free_memory(results_memory, None);
    device.destroy_buffer(ocean_buffer, None);
    device.free_memory(ocean_memory, None);
    device.destroy_device(None);
    instance.destroy_instance(None);

    if let Some(failure) = failure {
        return Err(failure);
    }
    println!(
        "world conformance: PASS ({SAMPLE_COUNT} GPU samples; max abs error h={:.6}, surface={:.6}, presence={:.8}, forest={:.8}, ocean={:.8})",
        maximum[0], maximum[1], maximum[2], maximum[3], maximum[4]
    );
    Ok(())
}
