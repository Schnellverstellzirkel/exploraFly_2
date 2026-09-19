//! CC0 photo detail textures for terrain materials.
//!
//! Four tilable 2K sets (diffuse sRGB + OpenGL normal) baked into the binary,
//! uploaded once with full mip chains and sampled world-space triplanar in
//! `ground.frag`. Distribution masks stay landform-driven; these textures
//! carry surface truth only. Sources and license in
//! `engine/assets/detail/ATTRIBUTION.md`.

use ash::vk;

pub const DETAIL_COUNT: usize = 4;
/// Diffuse binding per material: meadow, rock, snow, scree.
pub const DETAIL_DIFFUSE_BINDINGS: [u32; DETAIL_COUNT] = [12, 13, 14, 15];
/// Normal binding per material, same order.
pub const DETAIL_NORMAL_BINDINGS: [u32; DETAIL_COUNT] = [16, 17, 18, 19];
/// Shared repeat/anisotropic sampler binding.
pub const DETAIL_SAMPLER_BINDING: u32 = 20;

const DETAIL_DIFFUSE_BYTES: [&[u8]; DETAIL_COUNT] = [
    include_bytes!("../assets/detail/meadow/diff.jpg"),
    include_bytes!("../assets/detail/rock/diff.jpg"),
    include_bytes!("../assets/detail/snow/diff.jpg"),
    include_bytes!("../assets/detail/scree/diff.jpg"),
];
const DETAIL_NORMAL_BYTES: [&[u8]; DETAIL_COUNT] = [
    include_bytes!("../assets/detail/meadow/nor.jpg"),
    include_bytes!("../assets/detail/rock/nor.jpg"),
    include_bytes!("../assets/detail/snow/nor.jpg"),
    include_bytes!("../assets/detail/scree/nor.jpg"),
];

#[allow(dead_code)]
pub struct DetailTextures {
    pub images: [vk::Image; DETAIL_COUNT * 2],
    pub memories: [vk::DeviceMemory; DETAIL_COUNT * 2],
    pub views: [vk::ImageView; DETAIL_COUNT * 2],
    pub sampler: vk::Sampler,
    pub max_anisotropy: f32,
}

fn decode_jpeg(bytes: &[u8], label: &str) -> (Vec<u8>, u32, u32) {
    let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Jpeg)
        .unwrap_or_else(|e| panic!("{label} decode: {e}"));
    let rgba = img.to_rgba8();
    (rgba.clone().into_raw(), rgba.width(), rgba.height())
}

#[allow(clippy::too_many_arguments)]
pub unsafe fn upload_detail(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    device: &ash::Device,
    mem_props: &vk::PhysicalDeviceMemoryProperties,
    queue_family: u32,
    queue: vk::Queue,
) -> DetailTextures {
    for format in [vk::Format::R8G8B8A8_SRGB, vk::Format::R8G8B8A8_UNORM] {
        let props = instance.get_physical_device_format_properties(physical, format);
        assert!(
            props.optimal_tiling_features.contains(
                vk::FormatFeatureFlags::SAMPLED_IMAGE
                    | vk::FormatFeatureFlags::BLIT_SRC
                    | vk::FormatFeatureFlags::BLIT_DST
            ),
            "detail format {format:?} lacks blit/sample support"
        );
    }
    let max_aniso = instance
        .get_physical_device_properties(physical)
        .limits
        .max_sampler_anisotropy
        .min(16.0);

    let pool_info = vk::CommandPoolCreateInfo::default()
        .queue_family_index(queue_family)
        .flags(vk::CommandPoolCreateFlags::TRANSIENT);
    let pool = device
        .create_command_pool(&pool_info, None)
        .expect("detail command pool");
    let cmd = device
        .allocate_command_buffers(
            &vk::CommandBufferAllocateInfo::default()
                .command_pool(pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1),
        )
        .expect("detail command buffer")[0];
    device
        .begin_command_buffer(
            cmd,
            &vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
        )
        .expect("detail begin");

    let mut images = [vk::Image::null(); DETAIL_COUNT * 2];
    let mut memories = [vk::DeviceMemory::null(); DETAIL_COUNT * 2];
    let mut views = [vk::ImageView::null(); DETAIL_COUNT * 2];
    let mut staging = Vec::with_capacity(DETAIL_COUNT * 2);
    for (i, bytes) in DETAIL_DIFFUSE_BYTES
        .iter()
        .chain(DETAIL_NORMAL_BYTES.iter())
        .enumerate()
    {
        let label = if i < DETAIL_COUNT { "detail diffuse" } else { "detail normal" };
        let (pixels, width, height) = decode_jpeg(bytes, label);
        assert!(width == 2048 && height == 2048, "{label} size {width}x{height}");
        let format = if i < DETAIL_COUNT {
            vk::Format::R8G8B8A8_SRGB
        } else {
            vk::Format::R8G8B8A8_UNORM
        };
        let mips = (width.max(height) as f32).log2().floor() as u32 + 1;
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(vk::Extent3D { width, height, depth: 1 })
            .mip_levels(mips)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(
                vk::ImageUsageFlags::TRANSFER_SRC
                    | vk::ImageUsageFlags::TRANSFER_DST
                    | vk::ImageUsageFlags::SAMPLED,
            )
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        let image = device
            .create_image(&image_info, None)
            .unwrap_or_else(|_| panic!("{label} image"));
        let req = device.get_image_memory_requirements(image);
        let index = super::find_memory_type(
            mem_props,
            req.memory_type_bits,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        );
        let memory = device
            .allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(req.size)
                    .memory_type_index(index),
                None,
            )
            .unwrap_or_else(|_| panic!("{label} memory"));
        device
            .bind_image_memory(image, memory, 0)
            .unwrap_or_else(|_| panic!("{label} bind"));

        let bytes_len = pixels.len();
        let stage = device
            .create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(bytes_len as u64)
                    .usage(vk::BufferUsageFlags::TRANSFER_SRC)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )
            .unwrap_or_else(|_| panic!("{label} staging buffer"));
        let stage_req = device.get_buffer_memory_requirements(stage);
        let stage_index = super::find_memory_type(
            mem_props,
            stage_req.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        );
        let stage_memory = device
            .allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(stage_req.size)
                    .memory_type_index(stage_index),
                None,
            )
            .unwrap_or_else(|_| panic!("{label} staging memory"));
        device
            .bind_buffer_memory(stage, stage_memory, 0)
            .unwrap_or_else(|_| panic!("{label} staging bind"));
        let mapped = device
            .map_memory(stage_memory, 0, bytes_len as u64, vk::MemoryMapFlags::empty())
            .unwrap_or_else(|_| panic!("{label} map")) as *mut u8;
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), mapped, bytes_len);
        device.unmap_memory(stage_memory);

        let barrier = |old: vk::ImageLayout, new: vk::ImageLayout,
                       src_access: vk::AccessFlags, dst_access: vk::AccessFlags,
                       src_stage: vk::PipelineStageFlags, dst_stage: vk::PipelineStageFlags,
                       base_mip: u32, level_count: u32| {
            let b = vk::ImageMemoryBarrier::default()
                .src_access_mask(src_access)
                .dst_access_mask(dst_access)
                .old_layout(old)
                .new_layout(new)
                .image(image)
                .subresource_range(
                    vk::ImageSubresourceRange::default()
                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                        .base_mip_level(base_mip)
                        .level_count(level_count)
                        .layer_count(1),
                );
            device.cmd_pipeline_barrier(cmd, src_stage, dst_stage, vk::DependencyFlags::empty(), &[], &[], &[b]);
        };
        barrier(
            vk::ImageLayout::UNDEFINED,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            vk::AccessFlags::empty(),
            vk::AccessFlags::TRANSFER_WRITE,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
            0,
            mips,
        );
        let copy = [vk::BufferImageCopy::default()
            .image_subresource(
                vk::ImageSubresourceLayers::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .layer_count(1),
            )
            .image_extent(vk::Extent3D { width, height, depth: 1 })];
        device.cmd_copy_buffer_to_image(cmd, stage, image, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &copy);
        // Mip chain by blit; level 0 stays TRANSFER_DST until its children land.
        let mut w = width;
        let mut h = height;
        for mip in 1..mips {
            barrier(
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                vk::AccessFlags::TRANSFER_WRITE,
                vk::AccessFlags::TRANSFER_READ,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::TRANSFER,
                mip - 1,
                1,
            );
            let nw = (w / 2).max(1);
            let nh = (h / 2).max(1);
            let blit = [vk::ImageBlit::default()
                .src_subresource(
                    vk::ImageSubresourceLayers::default()
                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                        .mip_level(mip - 1)
                        .layer_count(1),
                )
                .src_offsets([
                    vk::Offset3D::default(),
                    vk::Offset3D { x: w as i32, y: h as i32, z: 1 },
                ])
                .dst_subresource(
                    vk::ImageSubresourceLayers::default()
                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                        .mip_level(mip)
                        .layer_count(1),
                )
                .dst_offsets([
                    vk::Offset3D::default(),
                    vk::Offset3D { x: nw as i32, y: nh as i32, z: 1 },
                ])];
            device.cmd_blit_image(
                cmd,
                image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &blit,
                vk::Filter::LINEAR,
            );
            barrier(
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                vk::AccessFlags::TRANSFER_READ,
                vk::AccessFlags::SHADER_READ,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::VERTEX_SHADER | vk::PipelineStageFlags::FRAGMENT_SHADER,
                mip - 1,
                1,
            );
            w = nw;
            h = nh;
        }
        barrier(
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            vk::AccessFlags::TRANSFER_WRITE,
            vk::AccessFlags::SHADER_READ,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::VERTEX_SHADER | vk::PipelineStageFlags::FRAGMENT_SHADER,
            mips - 1,
            1,
        );
        staging.push((stage, stage_memory));

        let view = device
            .create_image_view(
                &vk::ImageViewCreateInfo::default()
                    .image(image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(format)
                    .subresource_range(
                        vk::ImageSubresourceRange::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .level_count(mips)
                            .layer_count(1),
                    ),
                None,
            )
            .unwrap_or_else(|_| panic!("{label} view"));
        images[i] = image;
        memories[i] = memory;
        views[i] = view;
    }
    device
        .end_command_buffer(cmd)
        .expect("detail end");
    let fence = device
        .create_fence(&vk::FenceCreateInfo::default(), None)
        .expect("detail fence");
    device
        .queue_submit(queue, &[vk::SubmitInfo::default().command_buffers(&[cmd])], fence)
        .expect("detail submit");
    device
        .wait_for_fences(&[fence], true, u64::MAX)
        .expect("detail wait");
    device.destroy_fence(fence, None);
    device.destroy_command_pool(pool, None);
    for (stage_buf, stage_mem) in staging {
        device.destroy_buffer(stage_buf, None);
        device.free_memory(stage_mem, None);
    }

    let sampler = device
        .create_sampler(
            &vk::SamplerCreateInfo::default()
                .mag_filter(vk::Filter::LINEAR)
                .min_filter(vk::Filter::LINEAR)
                .mipmap_mode(vk::SamplerMipmapMode::LINEAR)
                .address_mode_u(vk::SamplerAddressMode::REPEAT)
                .address_mode_v(vk::SamplerAddressMode::REPEAT)
                .address_mode_w(vk::SamplerAddressMode::REPEAT)
                .mip_lod_bias(0.0)
                .anisotropy_enable(true)
                .max_anisotropy(max_aniso)
                .min_lod(0.0)
                .max_lod(vk::LOD_CLAMP_NONE),
            None,
        )
        .expect("detail sampler");
    DetailTextures { images, memories, views, sampler, max_anisotropy: max_aniso }
}

#[allow(dead_code)]
pub unsafe fn destroy_detail(device: &ash::Device, detail: &DetailTextures) {
    device.destroy_sampler(detail.sampler, None);
    for i in 0..DETAIL_COUNT * 2 {
        device.destroy_image_view(detail.views[i], None);
        device.destroy_image(detail.images[i], None);
        device.free_memory(detail.memories[i], None);
    }
}
