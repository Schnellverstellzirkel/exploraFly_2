//! Boot-time texture uploads: procedural sail cloth with CPU-built mips,
//! the Turquin specular LUT, the Hillaire atmosphere transport LUTs, and
//! the photographic ground detail sets.

use ash::vk;
use crate::atmo_lut;
use crate::detail;
use crate::lut::energy_lut;
use super::spirv::weave_mips;

/// Every texture handle the descriptor sets and struct need.
pub(super) struct TextureResources {
    pub(super) terrain_image: vk::Image,
    pub(super) terrain_memory: vk::DeviceMemory,
    pub(super) terrain_view: vk::ImageView,
    pub(super) weave_image: vk::Image,
    pub(super) weave_memory: vk::DeviceMemory,
    pub(super) weave_view: vk::ImageView,
    pub(super) weave_sampler: vk::Sampler,
    pub(super) lut_view: vk::ImageView,
    pub(super) lut_sampler: vk::Sampler,
    pub(super) atmo_transmittance_image: vk::Image,
    pub(super) atmo_transmittance_memory: vk::DeviceMemory,
    pub(super) atmo_transmittance_view: vk::ImageView,
    pub(super) atmo_multiscattering_image: vk::Image,
    pub(super) atmo_multiscattering_memory: vk::DeviceMemory,
    pub(super) atmo_multiscattering_view: vk::ImageView,
    pub(super) atmo_sampler: vk::Sampler,
    pub(super) detail_textures: detail::DetailTextures,
}

pub(super) unsafe fn upload_textures(
    device: &ash::Device,
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    mem_props: &vk::PhysicalDeviceMemoryProperties,
    queue_family: u32,
    queue: vk::Queue,
    max_aniso: f32,
) -> TextureResources {
    // Weave cloth texture with CPU-built mips. Upload once,
    // sample with anisotropy. Minification reads small mips
    // instead of aliasing the fine grid into static.
    let mips = weave_mips();
    let mip_count = mips.len() as u32;
    let total: usize = mips.iter().map(|(_, _, d)| d.len()).sum();
    let tex_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(vk::Format::R8G8B8A8_SRGB)
        .extent(vk::Extent3D {
            width: 64,
            height: 64,
            depth: 1,
        })
        .mip_levels(mip_count)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED);
    let weave_image = device.create_image(&tex_info, None).expect("timg");
    let tex_req = device.get_image_memory_requirements(weave_image);
    let tex_index = crate::find_memory_type(
        mem_props,
        tex_req.memory_type_bits,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
    );
    let tex_alloc = vk::MemoryAllocateInfo::default()
        .allocation_size(tex_req.size)
        .memory_type_index(tex_index);
    let weave_memory = device.allocate_memory(&tex_alloc, None).expect("tmem");
    device
        .bind_image_memory(weave_image, weave_memory, 0)
        .expect("tbind");
    let stage2_info = vk::BufferCreateInfo::default()
        .size(total as u64)
        .usage(vk::BufferUsageFlags::TRANSFER_SRC)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let stage2 = device.create_buffer(&stage2_info, None).expect("tstage");
    let stage2_req = device.get_buffer_memory_requirements(stage2);
    let stage2_index = crate::find_memory_type(
        mem_props,
        stage2_req.memory_type_bits,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
    );
    let stage2_alloc = vk::MemoryAllocateInfo::default()
        .allocation_size(stage2_req.size)
        .memory_type_index(stage2_index);
    let stage2_mem = device.allocate_memory(&stage2_alloc, None).expect("tsmem");
    device
        .bind_buffer_memory(stage2, stage2_mem, 0)
        .expect("tsbind");
    let tmap = device
        .map_memory(stage2_mem, 0, total as u64, vk::MemoryMapFlags::empty())
        .expect("tmap") as *mut u8;
    let mut offset = 0usize;
    let mut copies = Vec::with_capacity(mips.len());
    for (w, h, data) in &mips {
        std::ptr::copy_nonoverlapping(data.as_ptr(), tmap.add(offset), data.len());
        copies.push(
            vk::BufferImageCopy::default()
                .buffer_offset(offset as u64)
                .image_subresource(
                    vk::ImageSubresourceLayers::default()
                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                        .mip_level(copies.len() as u32)
                        .base_array_layer(0)
                        .layer_count(1),
                )
                .image_extent(vk::Extent3D {
                    width: *w,
                    height: *h,
                    depth: 1,
                }),
        );
        offset += data.len();
    }
    device.unmap_memory(stage2_mem);
    let tpool_info = vk::CommandPoolCreateInfo::default()
        .queue_family_index(queue_family)
        .flags(vk::CommandPoolCreateFlags::TRANSIENT);
    let tpool = device
        .create_command_pool(&tpool_info, None)
        .expect("tpool");
    let talloc = vk::CommandBufferAllocateInfo::default()
        .command_pool(tpool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(1);
    let tcmd = device.allocate_command_buffers(&talloc).expect("tcmd")[0];
    let tbegin = vk::CommandBufferBeginInfo::default()
        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
    device.begin_command_buffer(tcmd, &tbegin).expect("tbegin");
    let full_range = vk::ImageSubresourceRange::default()
        .aspect_mask(vk::ImageAspectFlags::COLOR)
        .base_mip_level(0)
        .level_count(mip_count)
        .base_array_layer(0)
        .layer_count(1);
    let to_dst = vk::ImageMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::empty())
        .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .old_layout(vk::ImageLayout::UNDEFINED)
        .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .image(weave_image)
        .subresource_range(full_range);
    device.cmd_pipeline_barrier(
        tcmd,
        vk::PipelineStageFlags::TOP_OF_PIPE,
        vk::PipelineStageFlags::TRANSFER,
        vk::DependencyFlags::empty(),
        &[],
        &[],
        &[to_dst],
    );
    device.cmd_copy_buffer_to_image(
        tcmd,
        stage2,
        weave_image,
        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
        &copies,
    );
    let to_read = vk::ImageMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .dst_access_mask(vk::AccessFlags::SHADER_READ)
        .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
        .image(weave_image)
        .subresource_range(full_range);
    device.cmd_pipeline_barrier(
        tcmd,
        vk::PipelineStageFlags::TRANSFER,
        vk::PipelineStageFlags::FRAGMENT_SHADER,
        vk::DependencyFlags::empty(),
        &[],
        &[],
        &[to_read],
    );
    device.end_command_buffer(tcmd).expect("tend");
    let tfence = device
        .create_fence(&vk::FenceCreateInfo::default(), None)
        .expect("tfence");
    let tcmd_ref = [tcmd];
    let tsubmit = vk::SubmitInfo::default().command_buffers(&tcmd_ref);
    device
        .queue_submit(queue, &[tsubmit], tfence)
        .expect("tsubmit");
    device
        .wait_for_fences(&[tfence], true, u64::MAX)
        .expect("twait");
    device.destroy_fence(tfence, None);
    device.destroy_command_pool(tpool, None);
    device.destroy_buffer(stage2, None);
    device.free_memory(stage2_mem, None);
    let view_info = vk::ImageViewCreateInfo::default()
        .image(weave_image)
        .view_type(vk::ImageViewType::TYPE_2D)
        .format(vk::Format::R8G8B8A8_SRGB)
        .subresource_range(
            vk::ImageSubresourceRange::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .base_mip_level(0)
                .level_count(mip_count)
                .base_array_layer(0)
                .layer_count(1),
        );
    let weave_view = device.create_image_view(&view_info, None).expect("tview");
    let sampler_info = vk::SamplerCreateInfo::default()
        .mag_filter(vk::Filter::LINEAR)
        .min_filter(vk::Filter::LINEAR)
        .mipmap_mode(vk::SamplerMipmapMode::LINEAR)
        .address_mode_u(vk::SamplerAddressMode::REPEAT)
        .address_mode_v(vk::SamplerAddressMode::REPEAT)
        .address_mode_w(vk::SamplerAddressMode::REPEAT)
        .anisotropy_enable(true)
        .max_anisotropy(max_aniso)
        .max_lod(mip_count as f32);
    let weave_sampler = device
        .create_sampler(&sampler_info, None)
        .expect("tsampler");

    // Turquin multi-scatter compensation LUT: 32x32 R32_SFLOAT of
    // Ess(n dot v, alpha), Monte Carlo precomputed on CPU. CLAMP addressing
    // (alpha = 1.0 must not wrap to row 0), bilinear, single mip.
    let lut = energy_lut();
    let lut_bytes = lut.len() * 4;
    let lut_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(vk::Format::R32_SFLOAT)
        .extent(vk::Extent3D {
            width: 32,
            height: 32,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED);
    let lut_image = device.create_image(&lut_info, None).expect("limg");
    let lut_req = device.get_image_memory_requirements(lut_image);
    let lut_index = crate::find_memory_type(
        mem_props,
        lut_req.memory_type_bits,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
    );
    let lut_alloc = vk::MemoryAllocateInfo::default()
        .allocation_size(lut_req.size)
        .memory_type_index(lut_index);
    let lut_memory = device.allocate_memory(&lut_alloc, None).expect("lmem");
    device
        .bind_image_memory(lut_image, lut_memory, 0)
        .expect("lbind");
    let lut_stage_info = vk::BufferCreateInfo::default()
        .size(lut_bytes as u64)
        .usage(vk::BufferUsageFlags::TRANSFER_SRC)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let lut_stage = device.create_buffer(&lut_stage_info, None).expect("lstage");
    let lut_stage_req = device.get_buffer_memory_requirements(lut_stage);
    let lut_stage_index = crate::find_memory_type(
        mem_props,
        lut_stage_req.memory_type_bits,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
    );
    let lut_stage_alloc = vk::MemoryAllocateInfo::default()
        .allocation_size(lut_stage_req.size)
        .memory_type_index(lut_stage_index);
    let lut_stage_mem = device.allocate_memory(&lut_stage_alloc, None).expect("lsmem");
    device
        .bind_buffer_memory(lut_stage, lut_stage_mem, 0)
        .expect("lsbind");
    let lut_map = device
        .map_memory(lut_stage_mem, 0, lut_bytes as u64, vk::MemoryMapFlags::empty())
        .expect("lmap") as *mut u8;
    std::ptr::copy_nonoverlapping(
        lut.as_ptr() as *const u8,
        lut_map,
        lut_bytes,
    );
    device.unmap_memory(lut_stage_mem);
    let lut_sampler_info = vk::SamplerCreateInfo::default()
        .mag_filter(vk::Filter::LINEAR)
        .min_filter(vk::Filter::LINEAR)
        .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
        .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .max_lod(vk::LOD_CLAMP_NONE);
    let lut_sampler = device
        .create_sampler(&lut_sampler_info, None)
        .expect("lsampler");
    let lut_view_info = vk::ImageViewCreateInfo::default()
        .image(lut_image)
        .view_type(vk::ImageViewType::TYPE_2D)
        .format(vk::Format::R32_SFLOAT)
        .subresource_range(
            vk::ImageSubresourceRange::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .level_count(1)
                .layer_count(1),
        );
    let lut_view = device.create_image_view(&lut_view_info, None).expect("lview");

    // Upload the LUT through its own one-time submit (the weave transfer
    // command buffer has already been submitted above).
    let lpool_info = vk::CommandPoolCreateInfo::default()
        .queue_family_index(queue_family)
        .flags(vk::CommandPoolCreateFlags::TRANSIENT);
    let lpool = device.create_command_pool(&lpool_info, None).expect("lpool");
    let lalloc = vk::CommandBufferAllocateInfo::default()
        .command_pool(lpool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(1);
    let lcmd = device.allocate_command_buffers(&lalloc).expect("lcmd")[0];
    device.begin_command_buffer(lcmd, &vk::CommandBufferBeginInfo::default())
        .expect("lbegin");
    let lut_range = vk::ImageSubresourceRange::default()
        .aspect_mask(vk::ImageAspectFlags::COLOR)
        .level_count(1)
        .layer_count(1);
    let lut_to_dst = vk::ImageMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::empty())
        .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .old_layout(vk::ImageLayout::UNDEFINED)
        .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .image(lut_image)
        .subresource_range(lut_range);
    device.cmd_pipeline_barrier(
        lcmd,
        vk::PipelineStageFlags::TOP_OF_PIPE,
        vk::PipelineStageFlags::TRANSFER,
        vk::DependencyFlags::empty(),
        &[],
        &[],
        &[lut_to_dst],
    );
    let lut_copy = [vk::BufferImageCopy::default()
        .image_subresource(
            vk::ImageSubresourceLayers::default()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .layer_count(1),
        )
        .image_extent(vk::Extent3D {
            width: 32,
            height: 32,
            depth: 1,
        })];
    device.cmd_copy_buffer_to_image(
        lcmd,
        lut_stage,
        lut_image,
        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
        &lut_copy,
    );
    let lut_to_read = vk::ImageMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .dst_access_mask(vk::AccessFlags::SHADER_READ)
        .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
        .image(lut_image)
        .subresource_range(lut_range);
    device.cmd_pipeline_barrier(
        lcmd,
        vk::PipelineStageFlags::TRANSFER,
        vk::PipelineStageFlags::FRAGMENT_SHADER,
        vk::DependencyFlags::empty(),
        &[],
        &[],
        &[lut_to_read],
    );
    device.end_command_buffer(lcmd).expect("lend");
    let lfence = device
        .create_fence(&vk::FenceCreateInfo::default(), None)
        .expect("lfence");
    let lcmd_ref = [lcmd];
    let lsubmit = vk::SubmitInfo::default().command_buffers(&lcmd_ref);
    device.queue_submit(queue, &[lsubmit], lfence).expect("lsubmit");
    device.wait_for_fences(&[lfence], true, u64::MAX).expect("lwait");
    device.destroy_fence(lfence, None);
    device.destroy_command_pool(lpool, None);
    device.destroy_buffer(lut_stage, None);
    device.free_memory(lut_stage_mem, None);

    // Hillaire atmosphere transport LUTs.  They are generated once from
    // the reference density profiles, then sampled by the per-pixel sky
    // march so every view sample does not need a second light march.
    let atmo_luts = atmo_lut::generate();
    println!(
        "atmosphere LUTs: transmittance {}x{}, multiple scattering {}x{}",
        atmo_lut::TRANSMITTANCE_WIDTH,
        atmo_lut::TRANSMITTANCE_HEIGHT,
        atmo_lut::MULTISCATTERING_WIDTH,
        atmo_lut::MULTISCATTERING_HEIGHT,
    );
    let upload_atmo_lut = |width: u32,
                           height: u32,
                           data: &[[f32; 4]],
                           label: &str|
     -> (vk::Image, vk::DeviceMemory, vk::ImageView) {
        let bytes = std::mem::size_of_val(data);
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::R32G32B32A32_SFLOAT)
            .extent(vk::Extent3D { width, height, depth: 1 })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        let image = device
            .create_image(&image_info, None)
            .unwrap_or_else(|_| panic!("{label} image"));
        let image_req = device.get_image_memory_requirements(image);
        let image_index = crate::find_memory_type(
            mem_props,
            image_req.memory_type_bits,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        );
        let image_alloc = vk::MemoryAllocateInfo::default()
            .allocation_size(image_req.size)
            .memory_type_index(image_index);
        let memory = device
            .allocate_memory(&image_alloc, None)
            .unwrap_or_else(|_| panic!("{label} memory"));
        device
            .bind_image_memory(image, memory, 0)
            .unwrap_or_else(|_| panic!("{label} bind"));

        let stage_info = vk::BufferCreateInfo::default()
            .size(bytes as u64)
            .usage(vk::BufferUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let stage = device
            .create_buffer(&stage_info, None)
            .unwrap_or_else(|_| panic!("{label} staging buffer"));
        let stage_req = device.get_buffer_memory_requirements(stage);
        let stage_index = crate::find_memory_type(
            mem_props,
            stage_req.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        );
        let stage_alloc = vk::MemoryAllocateInfo::default()
            .allocation_size(stage_req.size)
            .memory_type_index(stage_index);
        let stage_memory = device
            .allocate_memory(&stage_alloc, None)
            .unwrap_or_else(|_| panic!("{label} staging memory"));
        device
            .bind_buffer_memory(stage, stage_memory, 0)
            .unwrap_or_else(|_| panic!("{label} staging bind"));
        let mapped = device
            .map_memory(stage_memory, 0, bytes as u64, vk::MemoryMapFlags::empty())
            .unwrap_or_else(|_| panic!("{label} map")) as *mut u8;
        std::ptr::copy_nonoverlapping(data.as_ptr() as *const u8, mapped, bytes);
        device.unmap_memory(stage_memory);

        let pool_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(queue_family)
            .flags(vk::CommandPoolCreateFlags::TRANSIENT);
        let pool = device
            .create_command_pool(&pool_info, None)
            .unwrap_or_else(|_| panic!("{label} command pool"));
        let alloc = vk::CommandBufferAllocateInfo::default()
            .command_pool(pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let cmd = device
            .allocate_command_buffers(&alloc)
            .unwrap_or_else(|_| panic!("{label} command buffer"))[0];
        let begin = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        device
            .begin_command_buffer(cmd, &begin)
            .unwrap_or_else(|_| panic!("{label} begin"));
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
            .image_extent(vk::Extent3D { width, height, depth: 1 })];
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
            vk::PipelineStageFlags::VERTEX_SHADER | vk::PipelineStageFlags::FRAGMENT_SHADER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[to_read],
        );
        device
            .end_command_buffer(cmd)
            .unwrap_or_else(|_| panic!("{label} end"));
        let fence = device
            .create_fence(&vk::FenceCreateInfo::default(), None)
            .unwrap_or_else(|_| panic!("{label} fence"));
        let cmd_ref = [cmd];
        let submit = vk::SubmitInfo::default().command_buffers(&cmd_ref);
        device
            .queue_submit(queue, &[submit], fence)
            .unwrap_or_else(|_| panic!("{label} submit"));
        device
            .wait_for_fences(&[fence], true, u64::MAX)
            .unwrap_or_else(|_| panic!("{label} wait"));
        device.destroy_fence(fence, None);
        device.destroy_command_pool(pool, None);
        device.destroy_buffer(stage, None);
        device.free_memory(stage_memory, None);

        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(vk::Format::R32G32B32A32_SFLOAT)
            .subresource_range(range);
        let view = device
            .create_image_view(&view_info, None)
            .unwrap_or_else(|_| panic!("{label} view"));
        (image, memory, view)
    };
    let (atmo_transmittance_image, atmo_transmittance_memory, atmo_transmittance_view) =
        upload_atmo_lut(
            atmo_lut::TRANSMITTANCE_WIDTH as u32,
            atmo_lut::TRANSMITTANCE_HEIGHT as u32,
            &atmo_luts.transmittance,
            "transmittance",
        );
    let (atmo_multiscattering_image, atmo_multiscattering_memory, atmo_multiscattering_view) =
        upload_atmo_lut(
            atmo_lut::MULTISCATTERING_WIDTH as u32,
            atmo_lut::MULTISCATTERING_HEIGHT as u32,
            &atmo_luts.multiscattering,
            "multiple scattering",
        );
    let (terrain_image, terrain_memory, terrain_view) = upload_atmo_lut(
        world::TERRAIN_GRID_CELLS,
        world::TERRAIN_GRID_CELLS,
        world::terrain_samples_static(),
        "terrain heights and normals",
    );
    let detail_textures = crate::detail::upload_detail(
        instance,
        physical,
        device,
        mem_props,
        queue_family,
        queue,
    );
    let atmo_sampler_info = vk::SamplerCreateInfo::default()
        .mag_filter(vk::Filter::LINEAR)
        .min_filter(vk::Filter::LINEAR)
        .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
        .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
        .max_lod(vk::LOD_CLAMP_NONE);
    let atmo_sampler = device
        .create_sampler(&atmo_sampler_info, None)
        .expect("atmosphere sampler");
    TextureResources {
        terrain_image,
        terrain_memory,
        terrain_view,
        weave_image,
        weave_memory,
        weave_view,
        weave_sampler,
        lut_view,
        lut_sampler,
        atmo_transmittance_image,
        atmo_transmittance_memory,
        atmo_transmittance_view,
        atmo_multiscattering_image,
        atmo_multiscattering_memory,
        atmo_multiscattering_view,
        atmo_sampler,
        detail_textures,
    }
}
