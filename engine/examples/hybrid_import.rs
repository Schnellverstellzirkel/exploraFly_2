//! Read-only display experiment: AMD scanout allocation -> NVIDIA Vulkan import.
//! Does not modeset, open a window, or change the desktop. Successful binding
//! verifies a GPU clear through AMD readback; it does not prove direct scanout.
//! Run: cargo run -p explora-engine --example hybrid_import --profile perf

use ash::{vk, Entry};
use std::{ffi::c_void, fs::OpenOptions, os::fd::AsRawFd};

fn main() {
    unsafe { probe() }
}

unsafe fn probe() {
    let node = std::fs::read_dir("/sys/class/drm")
        .unwrap()
        .flatten()
        .find(|e| {
            e.file_name().to_string_lossy().starts_with("renderD")
                && std::fs::read_to_string(e.path().join("device/vendor"))
                    .is_ok_and(|v| v.trim() == "0x1002")
        })
        .expect("AMD render node");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(std::path::Path::new("/dev/dri").join(node.file_name()))
        .unwrap();
    let gbm = libloading::Library::new("libgbm.so.1").unwrap();
    let create = gbm
        .get::<unsafe extern "C" fn(i32) -> *mut c_void>(b"gbm_create_device")
        .unwrap();
    let allocate = gbm.get::<unsafe extern "C" fn(*mut c_void, u32, u32, u32, *const u64, u32, u32) -> *mut c_void>(b"gbm_bo_create_with_modifiers2").unwrap();
    let export = gbm
        .get::<unsafe extern "C" fn(*mut c_void) -> i32>(b"gbm_bo_get_fd")
        .unwrap();
    let stride = gbm
        .get::<unsafe extern "C" fn(*mut c_void) -> u32>(b"gbm_bo_get_stride")
        .unwrap();
    let modifier = gbm
        .get::<unsafe extern "C" fn(*mut c_void) -> u64>(b"gbm_bo_get_modifier")
        .unwrap();
    let destroy_bo = gbm
        .get::<unsafe extern "C" fn(*mut c_void)>(b"gbm_bo_destroy")
        .unwrap();
    let destroy_gbm = gbm
        .get::<unsafe extern "C" fn(*mut c_void)>(b"gbm_device_destroy")
        .unwrap();
    let map = gbm
        .get::<unsafe extern "C" fn(
            *mut c_void,
            u32,
            u32,
            u32,
            u32,
            u32,
            *mut u32,
            *mut *mut c_void,
        ) -> *mut c_void>(b"gbm_bo_map")
        .unwrap();
    let unmap = gbm
        .get::<unsafe extern "C" fn(*mut c_void, *mut c_void)>(b"gbm_bo_unmap")
        .unwrap();
    let gbm_device = create(file.as_raw_fd());
    assert!(!gbm_device.is_null(), "GBM device creation failed");
    // Explicit DRM_FORMAT_MOD_LINEAR; legacy GBM allocation may return
    // DRM_FORMAT_MOD_INVALID, which cannot describe an explicit Vulkan layout.
    let modifiers = [0u64];
    let bo = allocate(
        gbm_device,
        2880,
        1800,
        u32::from_le_bytes(*b"XR24"),
        modifiers.as_ptr(),
        1,
        1 | 4,
    );
    assert!(!bo.is_null(), "AMD linear scanout allocation failed");
    let fd = export(bo);
    assert!(fd >= 0, "DMA-BUF export failed");
    let pitch = stride(bo);
    let drm_modifier = modifier(bo);
    assert_eq!(drm_modifier, 0, "expected explicit linear modifier");
    println!("AMD buffer: 2880x1800, stride {pitch}, modifier {drm_modifier:#x}");

    let entry = Entry::load().unwrap();
    let app = vk::ApplicationInfo::default().api_version(vk::API_VERSION_1_3);
    let instance = entry
        .create_instance(
            &vk::InstanceCreateInfo::default().application_info(&app),
            None,
        )
        .unwrap();
    let physical = instance
        .enumerate_physical_devices()
        .unwrap()
        .into_iter()
        .find(|p| instance.get_physical_device_properties(*p).vendor_id == 0x10de)
        .expect("NVIDIA GPU");
    let family = instance
        .get_physical_device_queue_family_properties(physical)
        .iter()
        .position(|p| p.queue_flags.contains(vk::QueueFlags::GRAPHICS))
        .unwrap() as u32;
    let priority = [1.0];
    let queues = [vk::DeviceQueueCreateInfo::default()
        .queue_family_index(family)
        .queue_priorities(&priority)];
    let extensions = [
        ash::khr::external_memory_fd::NAME.as_ptr(),
        ash::ext::external_memory_dma_buf::NAME.as_ptr(),
        ash::ext::image_drm_format_modifier::NAME.as_ptr(),
        ash::ext::queue_family_foreign::NAME.as_ptr(),
    ];
    let device = instance
        .create_device(
            physical,
            &vk::DeviceCreateInfo::default()
                .queue_create_infos(&queues)
                .enabled_extension_names(&extensions),
            None,
        )
        .unwrap();
    let handles = vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT;
    let mut external = vk::ExternalMemoryImageCreateInfo::default().handle_types(handles);
    let layouts = [vk::SubresourceLayout::default()
        .offset(0)
        .row_pitch(pitch as u64)];
    let mut layout = vk::ImageDrmFormatModifierExplicitCreateInfoEXT::default()
        .drm_format_modifier(drm_modifier)
        .plane_layouts(&layouts);
    let info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(vk::Format::B8G8R8A8_UNORM)
        .extent(vk::Extent3D {
            width: 2880,
            height: 1800,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
        .usage(vk::ImageUsageFlags::TRANSFER_DST)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .push_next(&mut external)
        .push_next(&mut layout);

    // All failure paths below close the exported FD unless Vulkan consumed it.
    let mut image = vk::Image::null();
    let mut memory = vk::DeviceMemory::null();
    let mut fd_consumed = false;
    let result = (|| -> Result<(), String> {
        image = device
            .create_image(&info, None)
            .map_err(|e| format!("create image: {e:?}"))?;
        let req = device.get_image_memory_requirements(image);
        let external_fd = ash::khr::external_memory_fd::Device::new(&instance, &device);
        let mut props = vk::MemoryFdPropertiesKHR::default();
        external_fd
            .get_memory_fd_properties(handles, fd, &mut props)
            .map_err(|e| format!("DMA-BUF memory properties: {e:?}"))?;
        let bits = req.memory_type_bits & props.memory_type_bits;
        if bits == 0 {
            return Err("no compatible NVIDIA memory type".into());
        }
        let mut import = vk::ImportMemoryFdInfoKHR::default()
            .handle_type(handles)
            .fd(fd);
        let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
        memory = device
            .allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(req.size)
                    .memory_type_index(bits.trailing_zeros())
                    .push_next(&mut import)
                    .push_next(&mut dedicated),
                None,
            )
            .map_err(|e| format!("import allocation: {e:?}"))?;
        fd_consumed = true;
        device
            .bind_image_memory(image, memory, 0)
            .map_err(|e| format!("bind image: {e:?}"))?;
        let pool = device
            .create_command_pool(
                &vk::CommandPoolCreateInfo::default().queue_family_index(family),
                None,
            )
            .unwrap();
        let cmd = device
            .allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )
            .unwrap()[0];
        device
            .begin_command_buffer(cmd, &vk::CommandBufferBeginInfo::default())
            .unwrap();
        let range = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .level_count(1)
            .layer_count(1);
        let acquire = vk::ImageMemoryBarrier::default()
            .image(image)
            .subresource_range(range)
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::GENERAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_FOREIGN_EXT)
            .dst_queue_family_index(family)
            .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE);
        device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[acquire],
        );
        device.cmd_clear_color_image(
            cmd,
            image,
            vk::ImageLayout::GENERAL,
            &vk::ClearColorValue {
                float32: [1.0, 0.0, 0.0, 1.0],
            },
            &[range],
        );
        let release = vk::ImageMemoryBarrier::default()
            .image(image)
            .subresource_range(range)
            .old_layout(vk::ImageLayout::GENERAL)
            .new_layout(vk::ImageLayout::GENERAL)
            .src_queue_family_index(family)
            .dst_queue_family_index(vk::QUEUE_FAMILY_FOREIGN_EXT)
            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE);
        device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[release],
        );
        device.end_command_buffer(cmd).unwrap();
        let fence = device
            .create_fence(&vk::FenceCreateInfo::default(), None)
            .unwrap();
        let commands = [cmd];
        device
            .queue_submit(
                device.get_device_queue(family, 0),
                &[vk::SubmitInfo::default().command_buffers(&commands)],
                fence,
            )
            .unwrap();
        device.wait_for_fences(&[fence], true, u64::MAX).unwrap();
        device.destroy_fence(fence, None);
        device.destroy_command_pool(pool, None);
        // Diagnostic CPU readback only. A real presenter would export a sync
        // fence and give the DMA-BUF to Wayland without mapping it on the CPU.
        let mut map_stride = 0;
        let mut map_data = std::ptr::null_mut();
        let pixels = map(bo, 0, 0, 2880, 1800, 1, &mut map_stride, &mut map_data);
        if pixels.is_null() {
            return Err("AMD readback mapping failed".into());
        }
        let mut correct = true;
        for y in 0..1800usize {
            for x in 0..2880usize {
                let pixel = (pixels as *const u8).add(y * map_stride as usize + x * 4);
                correct &= *pixel == 0 && *pixel.add(1) == 0 && *pixel.add(2) == 255;
            }
        }
        unmap(bo, map_data);
        if !correct {
            return Err("AMD readback did not match NVIDIA's red clear".into());
        }
        Ok(())
    })();
    if image != vk::Image::null() {
        device.destroy_image(image, None);
    }
    if memory != vk::DeviceMemory::null() {
        device.free_memory(memory, None);
    }
    if !fd_consumed {
        libc::close(fd);
    }
    device.destroy_device(None);
    instance.destroy_instance(None);
    destroy_bo(bo);
    destroy_gbm(gbm_device);
    match result {
        Ok(()) => println!("PASS: NVIDIA wrote all 5,184,000 pixels of an AMD scanout allocation; AMD readback verified. Wayland scanout and copy latency remain untested."),
        Err(error) => { eprintln!("UNSUPPORTED: {error}"); std::process::exit(1); }
    }
}
