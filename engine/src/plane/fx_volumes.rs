//! FX noise resources: Nubis-style tileable Perlin-Worley base/detail
//! volumes and the 2D curl warp texture, all generated on CPU at boot and
//! uploaded once.

use ash::vk;

/// Volume textures for the plume/trail passes.
pub(super) struct FxVolumes {
    pub(super) noise_base_image: vk::Image,
    pub(super) noise_base_memory: vk::DeviceMemory,
    pub(super) noise_base_view: vk::ImageView,
    pub(super) noise_base_sampler: vk::Sampler,
    pub(super) noise_detail_image: vk::Image,
    pub(super) noise_detail_memory: vk::DeviceMemory,
    pub(super) noise_detail_view: vk::ImageView,
    pub(super) noise_detail_sampler: vk::Sampler,
    pub(super) noise_curl_view: vk::ImageView,
    pub(super) noise_curl_sampler: vk::Sampler,
}

pub(super) unsafe fn upload_fx_volumes(
    device: &ash::Device,
    mem_props: &vk::PhysicalDeviceMemoryProperties,
    queue_family: u32,
    queue: vk::Queue,
) -> FxVolumes {
    // Mirrors the weave upload path: staging buffer plus one-time submit.
    let upload_volume = |device: &ash::Device,
                         queue: vk::Queue,
                         queue_family: u32,
                         mem_props: &vk::PhysicalDeviceMemoryProperties,
                         data: &[u8],
                         w: u32,
                         h: u32,
                         d: u32| {
        let tex_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_3D)
            .format(vk::Format::R8G8B8A8_UNORM)
            .extent(vk::Extent3D { width: w, height: h, depth: d })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        let image = device.create_image(&tex_info, None).expect("nimg");
        let req = device.get_image_memory_requirements(image);
        let idx = crate::find_memory_type(
            mem_props,
            req.memory_type_bits,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        );
        let alloc = vk::MemoryAllocateInfo::default()
            .allocation_size(req.size)
            .memory_type_index(idx);
        let memory = device.allocate_memory(&alloc, None).expect("nmem");
        device.bind_image_memory(image, memory, 0).expect("nbind");
        let stage_info = vk::BufferCreateInfo::default()
            .size(data.len() as u64)
            .usage(vk::BufferUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let stage = device.create_buffer(&stage_info, None).expect("nstage");
        let sreq = device.get_buffer_memory_requirements(stage);
        let sidx = crate::find_memory_type(
            mem_props,
            sreq.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        );
        let salloc = vk::MemoryAllocateInfo::default()
            .allocation_size(sreq.size)
            .memory_type_index(sidx);
        let smem = device.allocate_memory(&salloc, None).expect("nsmem");
        device.bind_buffer_memory(stage, smem, 0).expect("nsbind");
        let mapped = device
            .map_memory(smem, 0, data.len() as u64, vk::MemoryMapFlags::empty())
            .expect("nmap") as *mut u8;
        std::ptr::copy_nonoverlapping(data.as_ptr(), mapped, data.len());
        device.unmap_memory(smem);
        let pool_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(queue_family)
            .flags(vk::CommandPoolCreateFlags::TRANSIENT);
        let pool = device.create_command_pool(&pool_info, None).expect("npool");
        let alloc_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let cmd = device.allocate_command_buffers(&alloc_info).expect("ncmd")[0];
        let begin = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        device.begin_command_buffer(cmd, &begin).expect("nbegin");
        let range = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .level_count(1)
            .layer_count(1);
        let to_dst = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::empty())
            .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .image(image)
            .subresource_range(range);
        device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[to_dst],
        );
        let copy = [vk::BufferImageCopy::default()
            .image_subresource(
                vk::ImageSubresourceLayers::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .layer_count(1),
            )
            .image_extent(vk::Extent3D { width: w, height: h, depth: d })];
        device.cmd_copy_buffer_to_image(
            cmd,
            stage,
            image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            &copy,
        );
        let to_read = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ)
            .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .image(image)
            .subresource_range(range);
        device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[to_read],
        );
        device.end_command_buffer(cmd).expect("nend");
        let fence = device
            .create_fence(&vk::FenceCreateInfo::default(), None)
            .expect("nfence");
        device.queue_submit(queue, &[vk::SubmitInfo::default().command_buffers(&[cmd])], fence)
            .expect("nsubmit");
        device.wait_for_fences(&[fence], true, u64::MAX).expect("nwait");
        device.destroy_fence(fence, None);
        device.destroy_command_pool(pool, None);
        device.destroy_buffer(stage, None);
        device.free_memory(smem, None);
        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_3D)
            .format(vk::Format::R8G8B8A8_UNORM)
            .subresource_range(
                vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .level_count(1)
                    .layer_count(1),
            );
        let view = device.create_image_view(&view_info, None).expect("nview");
        let smp_info = vk::SamplerCreateInfo::default()
            .mag_filter(vk::Filter::LINEAR)
            .min_filter(vk::Filter::LINEAR)
            .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
            .address_mode_u(vk::SamplerAddressMode::REPEAT)
            .address_mode_v(vk::SamplerAddressMode::REPEAT)
            .address_mode_w(vk::SamplerAddressMode::REPEAT)
            .max_lod(vk::LOD_CLAMP_NONE);
        let sampler = device.create_sampler(&smp_info, None).expect("nsampler");
        (image, memory, view, sampler)
    };
    let base_data = sim::noise::generate_base();
    let detail_data = sim::noise::generate_detail();
    let bn = sim::noise::BASE_N as u32;
    let dn = sim::noise::DETAIL_N as u32;
    let (noise_base_image, noise_base_memory, noise_base_view, noise_base_sampler) =
        upload_volume(device, queue, queue_family, &mem_props, &base_data, bn, bn, bn);
    let (noise_detail_image, noise_detail_memory, noise_detail_view, noise_detail_sampler) =
        upload_volume(device, queue, queue_family, &mem_props, &detail_data, dn, dn, dn);
    println!(
        "fx noise: base {}^3 RGBA + detail {}^3 RGBA uploaded",
        bn, dn
    );
    // Curl warp texture (2D RG, Nubis 2015): distorts plume/trail sample
    // positions for swirl without a velocity grid.
    let (noise_curl_view, noise_curl_sampler) = {
        let data = sim::noise::generate_curl();
        let cn = sim::noise::CURL_N as u32;
        let tex_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_UNORM)
            .extent(vk::Extent3D { width: cn, height: cn, depth: 1 })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        let image = device.create_image(&tex_info, None).expect("cimg");
        let req = device.get_image_memory_requirements(image);
        let idx = crate::find_memory_type(
            &mem_props,
            req.memory_type_bits,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        );
        let alloc = vk::MemoryAllocateInfo::default()
            .allocation_size(req.size)
            .memory_type_index(idx);
        let memory = device.allocate_memory(&alloc, None).expect("cmem");
        device.bind_image_memory(image, memory, 0).expect("cbind");
        let stage_info = vk::BufferCreateInfo::default()
            .size(data.len() as u64)
            .usage(vk::BufferUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let stage = device.create_buffer(&stage_info, None).expect("cstage");
        let sreq = device.get_buffer_memory_requirements(stage);
        let sidx = crate::find_memory_type(
            &mem_props,
            sreq.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        );
        let salloc = vk::MemoryAllocateInfo::default()
            .allocation_size(sreq.size)
            .memory_type_index(sidx);
        let smem = device.allocate_memory(&salloc, None).expect("csmem");
        device.bind_buffer_memory(stage, smem, 0).expect("csbind");
        let mapped = device
            .map_memory(smem, 0, data.len() as u64, vk::MemoryMapFlags::empty())
            .expect("cmap") as *mut u8;
        std::ptr::copy_nonoverlapping(data.as_ptr(), mapped, data.len());
        device.unmap_memory(smem);
        let pool_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(queue_family)
            .flags(vk::CommandPoolCreateFlags::TRANSIENT);
        let pool = device.create_command_pool(&pool_info, None).expect("cpool");
        let alloc_info = vk::CommandBufferAllocateInfo::default()
            .command_pool(pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let cmd = device.allocate_command_buffers(&alloc_info).expect("ccmd")[0];
        let begin = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        device.begin_command_buffer(cmd, &begin).expect("cbegin");
        let range = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .level_count(1)
            .layer_count(1);
        let to_dst = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::empty())
            .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .image(image)
            .subresource_range(range);
        device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[to_dst],
        );
        let copy = [vk::BufferImageCopy::default()
            .image_subresource(
                vk::ImageSubresourceLayers::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .layer_count(1),
            )
            .image_extent(vk::Extent3D { width: cn, height: cn, depth: 1 })];
        device.cmd_copy_buffer_to_image(
            cmd,
            stage,
            image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            &copy,
        );
        let to_read = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ)
            .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .image(image)
            .subresource_range(range);
        device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[to_read],
        );
        device.end_command_buffer(cmd).expect("cend");
        let fence = device
            .create_fence(&vk::FenceCreateInfo::default(), None)
            .expect("cfence");
        device.queue_submit(queue, &[vk::SubmitInfo::default().command_buffers(&[cmd])], fence)
            .expect("csubmit");
        device.wait_for_fences(&[fence], true, u64::MAX).expect("cwait");
        device.destroy_fence(fence, None);
        device.destroy_command_pool(pool, None);
        device.destroy_buffer(stage, None);
        device.free_memory(smem, None);
        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(vk::Format::R8G8B8A8_UNORM)
            .subresource_range(
                vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .level_count(1)
                    .layer_count(1),
            );
        let view = device.create_image_view(&view_info, None).expect("cview");
        let smp_info = vk::SamplerCreateInfo::default()
            .mag_filter(vk::Filter::LINEAR)
            .min_filter(vk::Filter::LINEAR)
            .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
            .address_mode_u(vk::SamplerAddressMode::REPEAT)
            .address_mode_v(vk::SamplerAddressMode::REPEAT)
            .address_mode_w(vk::SamplerAddressMode::REPEAT)
            .max_lod(vk::LOD_CLAMP_NONE);
        let sampler = device.create_sampler(&smp_info, None).expect("csampler");
        println!("fx noise: curl {}^2 RG uploaded", cn);
        (view, sampler)
    };
    // FX descriptor layout (group 1): base/detail volumes, curl warp,
    // each with its sampler. Binding 0 stays the airframe UBO via the
    // shared group 0 layout; scene HDR + composite layout arrive with
    FxVolumes {
        noise_base_image,
        noise_base_memory,
        noise_base_view,
        noise_base_sampler,
        noise_detail_image,
        noise_detail_memory,
        noise_detail_view,
        noise_detail_sampler,
        noise_curl_view,
        noise_curl_sampler,
    }
}
