// Native boot: window, Vulkan device, swapchain, fixed loop.
// One canvas. One GPU. Fixed passes. No fallback.

mod airframe;
mod airframe_util;
mod camera;
mod flight;
mod forsyth;
mod plane;
mod vendor;

use ash::{vk, Entry};
use flight::{Controls, Pose};
use glam::Mat4;
use plane::Plane;
use std::ffi::CStr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::{Window, WindowId};

const NVIDIA_VENDOR: u32 = 0x10DE;
const SIM_STEP: f32 = 1.0 / 90.0;
const SHADER_MARKER: &str = include_str!("plane.wgsl");

#[derive(Default)]
struct StageStats {
    acquire_us: u64,
    submit_us: u64,
    present_us: u64,
    sim_us: u64,
    camera_us: u64,
    gpu_us: u64,
    frames: u64,
}

impl StageStats {
    fn add(&mut self, acquire: u64, submit: u64, present: u64) {
        self.acquire_us += acquire;
        self.submit_us += submit;
        self.present_us += present;
        self.frames += 1;
    }

    fn add_cpu(&mut self, sim: u64, camera: u64) {
        self.sim_us += sim;
        self.camera_us += camera;
    }

    fn add_gpu(&mut self, gpu_us: u64) {
        self.gpu_us += gpu_us;
    }

    fn report(&self) -> (u64, u64, u64, u64, u64, u64) {
        let n = self.frames.max(1);
        (
            self.acquire_us / n,
            self.submit_us / n,
            self.present_us / n,
            self.sim_us / n,
            self.camera_us / n,
            self.gpu_us / n,
        )
    }
}

pub(crate) fn wgsl_to_spirv(src: &str) -> Vec<u32> {
    let module = naga::front::wgsl::parse_str(src).expect("WGSL parse failed");
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .expect("WGSL validation failed");
    naga::back::spv::write_vec(
        &module,
        &info,
        &naga::back::spv::Options::default(),
        None,
    )
    .expect("SPIR-V emit failed")
}

fn pick_present(modes: &[vk::PresentModeKHR]) -> vk::PresentModeKHR {
    // Immediate tears but never blocks: highest throughput.
    // Mailbox second, FIFO last.
    if modes.contains(&vk::PresentModeKHR::IMMEDIATE) {
        vk::PresentModeKHR::IMMEDIATE
    } else if modes.contains(&vk::PresentModeKHR::MAILBOX) {
        vk::PresentModeKHR::MAILBOX
    } else {
        vk::PresentModeKHR::FIFO
    }
}

fn find_memory_type(
    mem_props: &vk::PhysicalDeviceMemoryProperties,
    bits: u32,
    flags: vk::MemoryPropertyFlags,
) -> u32 {
    for i in 0..mem_props.memory_type_count {
        if bits & (1 << i) != 0 && mem_props.memory_types[i as usize].property_flags.contains(flags)
        {
            return i;
        }
    }
    panic!("no fixed memory type found");
}

#[allow(dead_code)]
struct Frame {
    pool: vk::CommandPool,
    cmd: vk::CommandBuffer,
    frame_done: vk::Semaphore,
    fence: vk::Fence,
}


struct Gfx {
    _entry: Entry,
    instance: ash::Instance,
    surface_loader: ash::khr::surface::Instance,
    surface: vk::SurfaceKHR,
    device: ash::Device,
    queue: vk::Queue,
    queue_family: u32,
    swap_loader: ash::khr::swapchain::Device,
    swapchain: vk::SwapchainKHR,
    images: Vec<vk::Image>,
    views: Vec<vk::ImageView>,
    format: vk::Format,
    extent: vk::Extent2D,
    frames: Vec<Frame>,
    submitted: Vec<bool>,
    plane: Plane,
    depth_image: vk::Image,
    depth_memory: vk::DeviceMemory,
    depth_view: vk::ImageView,
    timestamp_period_ns: f32,
}

impl Gfx {
    unsafe fn new(window: &Window) -> Self {
        let entry = Entry::load().expect("no Vulkan loader");
        let display = window.display_handle().expect("no display").as_raw();
        let req = ash_window::enumerate_required_extensions(display).expect("no surface ext");
        let app_name = c"explora";
        let app_info = vk::ApplicationInfo::default()
            .application_name(app_name)
            .application_version(1)
            .engine_name(app_name)
            .engine_version(1)
            .api_version(vk::make_api_version(0, 1, 3, 0));
        let create_info = vk::InstanceCreateInfo::default()
            .application_info(&app_info)
            .enabled_extension_names(req);
        let instance = entry.create_instance(&create_info, None).expect("instance");

        let surface_loader = ash::khr::surface::Instance::new(&entry, &instance);
        let surface = ash_window::create_surface(
            &entry,
            &instance,
            display,
            window.window_handle().expect("no handle").as_raw(),
            None,
        )
        .expect("surface");

        // Fixed target: discrete NVIDIA GPU.
        let devices = instance.enumerate_physical_devices().expect("devices");
        let mut chosen = None;
        for device in devices {
            let props = instance.get_physical_device_properties(device);
            if props.device_type == vk::PhysicalDeviceType::DISCRETE_GPU
                && props.vendor_id == NVIDIA_VENDOR
            {
                chosen = Some(device);
            }
        }
        let physical = chosen.expect("fixed target missing: no NVIDIA discrete GPU");
        let timestamp_period_ns =
            instance.get_physical_device_properties(physical).limits.timestamp_period;
        let name = CStr::from_ptr(
            instance.get_physical_device_properties(physical).device_name.as_ptr(),
        )
        .to_string_lossy()
        .into_owned();
        println!("device locked: {}", name);

        let queue_props = instance.get_physical_device_queue_family_properties(physical);
        let mut queue_family = None;
        for (i, q) in queue_props.iter().enumerate() {
            if !q.queue_flags.contains(vk::QueueFlags::GRAPHICS) {
                continue;
            }
            let present = surface_loader
                .get_physical_device_surface_support(physical, i as u32, surface)
                .unwrap_or(false);
            if present {
                queue_family = Some(i as u32);
                break;
            }
        }
        let queue_family = queue_family.expect("no graphics+present queue");
        let priority = [1.0f32];
        let queue_info = vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family)
            .queue_priorities(&priority);
        let device_exts = [ash::khr::swapchain::NAME.as_ptr()];
        let mut dyn_feat = vk::PhysicalDeviceDynamicRenderingFeatures::default()
            .dynamic_rendering(true);
        let device_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(std::slice::from_ref(&queue_info))
            .enabled_extension_names(&device_exts)
            .push_next(&mut dyn_feat);
        let device = instance.create_device(physical, &device_info, None).expect("device");
        let queue = device.get_device_queue(queue_family, 0);
        let swap_loader = ash::khr::swapchain::Device::new(&instance, &device);

        let caps = surface_loader
            .get_physical_device_surface_capabilities(physical, surface)
            .expect("caps");
        let formats = surface_loader
            .get_physical_device_surface_formats(physical, surface)
            .expect("formats");
        let format = formats
            .iter()
            .find(|f| {
                f.format == vk::Format::B8G8R8A8_SRGB
                    && f.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
            })
            .copied()
            .unwrap_or(formats[0]);
        let modes = surface_loader
            .get_physical_device_surface_present_modes(physical, surface)
            .expect("modes");
        let present = pick_present(&modes);
        println!("present mode: {present:?} from {:?}", modes);
        let size = window.inner_size();
        let extent = vk::Extent2D {
            width: size.width.clamp(
                caps.min_image_extent.width,
                caps.max_image_extent.width,
            ),
            height: size.height.clamp(
                caps.min_image_extent.height,
                caps.max_image_extent.height,
            ),
        };
        // One spare image over the minimum: measured faster than
        // minimum count, which starves acquire behind the compositor.
        let image_count = (caps.min_image_count + 1).min(if caps.max_image_count == 0 {
            u32::MAX
        } else {
            caps.max_image_count
        });
        let swap_info = vk::SwapchainCreateInfoKHR::default()
            .surface(surface)
            .min_image_count(image_count)
            .image_format(format.format)
            .image_color_space(format.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(caps.current_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(present)
            .clipped(true);
        let swapchain = swap_loader.create_swapchain(&swap_info, None).expect("swapchain");

        let plane =
            Plane::build(&device, &instance, physical, queue_family, queue, format.format);
        let mut gfx = Self {
            _entry: entry,
            instance,
            surface_loader,
            surface,
            device,
            queue,
            queue_family,
            swap_loader,
            swapchain,
            images: Vec::new(),
            views: Vec::new(),
            format: format.format,
            extent,
            frames: Vec::new(),
            submitted: Vec::new(),
            plane,
            depth_image: vk::Image::null(),
            depth_memory: vk::DeviceMemory::null(),
            depth_view: vk::ImageView::null(),
            timestamp_period_ns,
        };
        gfx.build_swap_views();
        gfx.build_depth();
        gfx.build_frames();
        gfx
    }

    unsafe fn build_swap_views(&mut self) {
        self.images = self.swap_loader.get_swapchain_images(self.swapchain).expect("images");
        self.views = self
            .images
            .iter()
            .map(|image| {
                let view_info = vk::ImageViewCreateInfo::default()
                    .image(*image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(self.format)
                    .components(vk::ComponentMapping::default())
                    .subresource_range(
                        vk::ImageSubresourceRange::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .base_mip_level(0)
                            .level_count(1)
                            .base_array_layer(0)
                            .layer_count(1),
                    );
                self.device.create_image_view(&view_info, None).expect("view")
            })
            .collect();
    }

    unsafe fn physical(&self) -> vk::PhysicalDevice {
        // Fixed target is chosen once at boot; recover it from the surface.
        // Kept simple on purpose: exactly one discrete NVIDIA GPU exists here.
        let devices = self.instance.enumerate_physical_devices().expect("devices");
        for device in devices {
            let props = self.instance.get_physical_device_properties(device);
            if props.device_type == vk::PhysicalDeviceType::DISCRETE_GPU
                && props.vendor_id == NVIDIA_VENDOR
            {
                return device;
            }
        }
        panic!("fixed target missing");
    }

    unsafe fn build_depth(&mut self) {
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::D32_SFLOAT)
            .extent(vk::Extent3D { width: self.extent.width, height: self.extent.height, depth: 1 })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        self.depth_image = self.device.create_image(&image_info, None).expect("dimg");
        let req = self.device.get_image_memory_requirements(self.depth_image);
        let mem_props = self.instance.get_physical_device_memory_properties(self.physical());
        let index = find_memory_type(
            &mem_props,
            req.memory_type_bits,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        );
        let alloc = vk::MemoryAllocateInfo::default()
            .allocation_size(req.size)
            .memory_type_index(index);
        self.depth_memory = self.device.allocate_memory(&alloc, None).expect("dmem");
        self.device.bind_image_memory(self.depth_image, self.depth_memory, 0).expect("dbind");
        let view_info = vk::ImageViewCreateInfo::default()
            .image(self.depth_image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(vk::Format::D32_SFLOAT)
            .subresource_range(
                vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::DEPTH)
                    .base_mip_level(0)
                    .level_count(1)
                    .base_array_layer(0)
                    .layer_count(1),
            );
        self.depth_view = self.device.create_image_view(&view_info, None).expect("dview");
    }

    unsafe fn build_frames(&mut self) {
        // One frame slot per swapchain image. The plane records once
        // per image. The hot loop only waits, copies uniforms, submits.
        self.plane.build_frames(
            &self.device,
            &self.instance,
            self.physical(),
            self.images.len(),
        );
        for (index, image) in self.images.iter().enumerate() {
            let pool_info = vk::CommandPoolCreateInfo::default()
                .queue_family_index(self.queue_family)
                .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
            let pool = self.device.create_command_pool(&pool_info, None).expect("pool");
            let alloc = vk::CommandBufferAllocateInfo::default()
                .command_pool(pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1);
            let cmd = self.device.allocate_command_buffers(&alloc).expect("cmd")[0];
            let semaphore = vk::SemaphoreCreateInfo::default();
            let fence = vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED);
            self.frames.push(Frame {
                pool,
                cmd,
                frame_done: self.device.create_semaphore(&semaphore, None).expect("sem"),
                fence: self.device.create_fence(&fence, None).expect("fence"),
            });
            self.submitted.push(false);
            self.plane.record(
                &self.device,
                cmd,
                *image,
                self.views[index],
                self.depth_image,
                self.depth_view,
                self.extent,
                index,
            );
        }
    }

    unsafe fn destroy_swap_side(&mut self) {
        for frame in self.frames.drain(..) {
            self.device.destroy_semaphore(frame.frame_done, None);
            self.device.destroy_fence(frame.fence, None);
            self.device.destroy_command_pool(frame.pool, None);
        }
        self.submitted.clear();
        self.plane.destroy_frames(&self.device);
        self.device.destroy_image_view(self.depth_view, None);
        self.device.destroy_image(self.depth_image, None);
        self.device.free_memory(self.depth_memory, None);
        self.depth_view = vk::ImageView::null();
        self.depth_image = vk::Image::null();
        for view in self.views.drain(..) {
            self.device.destroy_image_view(view, None);
        }
        self.swap_loader.destroy_swapchain(self.swapchain, None);
    }

    unsafe fn recreate(&mut self, window: &Window) {
        self.device.device_wait_idle().expect("idle");
        self.destroy_swap_side();
        let caps = self
            .surface_loader
            .get_physical_device_surface_capabilities(self.physical(), self.surface)
            .expect("caps");
        let size = window.inner_size();
        self.extent = vk::Extent2D {
            width: size.width.clamp(caps.min_image_extent.width, caps.max_image_extent.width),
            height: size.height.clamp(caps.min_image_extent.height, caps.max_image_extent.height),
        };
        let formats = self
            .surface_loader
            .get_physical_device_surface_formats(self.physical(), self.surface)
            .expect("formats");
        let format = formats
            .iter()
            .find(|f| {
                f.format == vk::Format::B8G8R8A8_SRGB
                    && f.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
            })
            .copied()
            .unwrap_or(formats[0]);
        self.format = format.format;
        let modes = self
            .surface_loader
            .get_physical_device_surface_present_modes(self.physical(), self.surface)
            .expect("modes");
        let present = pick_present(&modes);
        // One spare image over the minimum: measured faster than
        // minimum count, which starves acquire behind the compositor.
        let image_count = (caps.min_image_count + 1).min(if caps.max_image_count == 0 {
            u32::MAX
        } else {
            caps.max_image_count
        });
        let swap_info = vk::SwapchainCreateInfoKHR::default()
            .surface(self.surface)
            .min_image_count(image_count)
            .image_format(self.format)
            .image_color_space(format.color_space)
            .image_extent(self.extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(caps.current_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(present)
            .clipped(true);
        self.swapchain = self.swap_loader.create_swapchain(&swap_info, None).expect("swap");
        self.build_swap_views();
        self.build_depth();
        self.build_frames();
    }

    unsafe fn screenshot(&mut self, path: &str) {
        // One-shot debug readback. Never runs in the hot loop.
        self.device.device_wait_idle().expect("shot idle");
        let w = self.extent.width as usize;
        let h = self.extent.height as usize;
        let size = (w * h * 4) as u64;
        let buffer_info = vk::BufferCreateInfo::default()
            .size(size)
            .usage(vk::BufferUsageFlags::TRANSFER_DST)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let buffer = self.device.create_buffer(&buffer_info, None).expect("shot buf");
        let req = self.device.get_buffer_memory_requirements(buffer);
        let mem_props = self.instance.get_physical_device_memory_properties(self.physical());
        let index = find_memory_type(
            &mem_props,
            req.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        );
        let alloc = vk::MemoryAllocateInfo::default()
            .allocation_size(req.size)
            .memory_type_index(index);
        let memory = self.device.allocate_memory(&alloc, None).expect("shot mem");
        self.device.bind_buffer_memory(buffer, memory, 0).expect("shot bind");
        let pool_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(self.queue_family)
            .flags(vk::CommandPoolCreateFlags::TRANSIENT);
        let pool = self.device.create_command_pool(&pool_info, None).expect("shot pool");
        let alloc = vk::CommandBufferAllocateInfo::default()
            .command_pool(pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let cmd = self.device.allocate_command_buffers(&alloc).expect("shot cmd")[0];
        // Copy the most recently presented image back.
        let image = self.images[0];
        let range = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .base_mip_level(0)
            .level_count(1)
            .base_array_layer(0)
            .layer_count(1);
        let to_transfer = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::MEMORY_READ)
            .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
            .old_layout(vk::ImageLayout::PRESENT_SRC_KHR)
            .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .image(image)
            .subresource_range(range);
        let to_present = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::TRANSFER_READ)
            .dst_access_mask(vk::AccessFlags::MEMORY_READ)
            .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
            .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
            .image(image)
            .subresource_range(range);
        let region = vk::BufferImageCopy::default()
            .image_subresource(
                vk::ImageSubresourceLayers::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .mip_level(0)
                    .base_array_layer(0)
                    .layer_count(1),
            )
            .image_extent(vk::Extent3D { width: self.extent.width, height: self.extent.height, depth: 1 });
        let begin = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        self.device.begin_command_buffer(cmd, &begin).expect("shot begin");
        self.device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[to_transfer],
        );
        self.device.cmd_copy_image_to_buffer(cmd, image, vk::ImageLayout::TRANSFER_SRC_OPTIMAL, buffer, &[region]);
        self.device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[to_present],
        );
        self.device.end_command_buffer(cmd).expect("shot end");
        let fence_info = vk::FenceCreateInfo::default();
        let fence = self.device.create_fence(&fence_info, None).expect("shot fence");
        let cmds = [cmd];
        let submit = vk::SubmitInfo::default().command_buffers(&cmds);
        self.device.queue_submit(self.queue, &[submit], fence).expect("shot submit");
        self.device.wait_for_fences(&[fence], true, u64::MAX).expect("shot wait");
        let mapped = self
            .device
            .map_memory(memory, 0, size, vk::MemoryMapFlags::empty())
            .expect("shot map") as *const u8;
        let pixels = std::slice::from_raw_parts(mapped, size as usize).to_vec();
        self.device.unmap_memory(memory);
        // PPM top row first. Image row 0 is the top as presented.
        let mut ppm = format!("P6\n{} {}\n255\n", w, h).into_bytes();
        for y in 0..h {
            for x in 0..w {
                let o = (y * w + x) * 4;
                ppm.push(pixels[o + 2]);
                ppm.push(pixels[o + 1]);
                ppm.push(pixels[o]);
            }
        }
        std::fs::write(path, &ppm).expect("shot write");
        println!("screenshot wrote {} ({}x{})", path, w, h);
        self.device.destroy_fence(fence, None);
        self.device.destroy_command_pool(pool, None);
        self.device.destroy_buffer(buffer, None);
        self.device.free_memory(memory, None);
    }

    unsafe fn draw(
        &mut self,
        pose: &Pose,
        u: &Controls,
        view_proj: &Mat4,
        time: f32,
        dt: f32,
        stats: &mut StageStats,
    ) -> DrawResult {
        // Hot loop: wait, update part uniforms, submit, present.
        // The wait is compositor backpressure. Skipping on timeout was
        // measured slower: released images beat skipped frames.
        let t0 = std::time::Instant::now();
        let next = self.swap_loader.acquire_next_image(
            self.swapchain,
            u64::MAX,
            vk::Semaphore::null(),
            vk::Fence::null(),
        );
        let image_index = match next {
            Ok((index, _)) => index as usize,
            Err(vk::Result::TIMEOUT) | Err(vk::Result::NOT_READY) => {
                return DrawResult::Skipped;
            }
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => return DrawResult::Rebuild,
            Err(error) => panic!("acquire failed: {error:?}"),
        };
        let frame = &self.frames[image_index];
        self.device.wait_for_fences(&[frame.fence], true, u64::MAX).expect("fence");
        self.device.reset_fences(&[frame.fence]).expect("reset fence");
        // Previous frame's GPU timestamps are ready behind this fence.
        // First use skips: pre-signaled fence, queries never written.
        if self.submitted[image_index] {
            let mut stamps = [0u64; 2];
            let query_ok = self
                .device
                .get_query_pool_results(
                    self.plane.query_pool(),
                    (image_index * 2).min(14) as u32,
                    &mut stamps,
                    vk::QueryResultFlags::TYPE_64 | vk::QueryResultFlags::WAIT,
                )
                .is_ok();
            if query_ok && stamps[1] >= stamps[0] {
                let ns = (stamps[1] - stamps[0]) as f64 * self.timestamp_period_ns as f64;
                stats.add_gpu((ns / 1000.0) as u64);
            }
        }
        self.plane.update(u, pose, view_proj, time, dt, image_index);
        let buffers = [frame.cmd];
        let signal = [frame.frame_done];
        let submit = vk::SubmitInfo::default()
            .command_buffers(&buffers)
            .signal_semaphores(&signal);
        let t1 = std::time::Instant::now();
        self.device.queue_submit(self.queue, &[submit], frame.fence).expect("submit");
        self.submitted[image_index] = true;
        let t2 = std::time::Instant::now();
        // Inline present. A present thread overlapped the round trip
        // but lost overall: driver lock contention plus an unbounded
        // present flood starved the loop with 15 ms stalls.
        let swapchains = [self.swapchain];
        let indices = [image_index as u32];
        let present_info = vk::PresentInfoKHR::default()
            .wait_semaphores(&signal)
            .swapchains(&swapchains)
            .image_indices(&indices);
        match self.swap_loader.queue_present(self.queue, &present_info) {
            Ok(suboptimal) => {
                let t3 = std::time::Instant::now();
                stats.add(
                    t1.duration_since(t0).as_micros() as u64,
                    t2.duration_since(t1).as_micros() as u64,
                    t3.duration_since(t2).as_micros() as u64,
                );
                if suboptimal {
                    return DrawResult::Rebuild;
                }
                return DrawResult::Presented;
            }
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => return DrawResult::Rebuild,
            Err(error) => panic!("present failed: {error:?}"),
        }
    }
}

#[derive(PartialEq, Eq)]
enum DrawResult {
    Presented,
    Skipped,
    Rebuild,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UserEvent {
    RenderDone,
}

struct Shared {
    exit: AtomicBool,
    keys: AtomicU32,
}

const KEY_W: u32 = 1;
const KEY_S: u32 = 1 << 1;
const KEY_A: u32 = 1 << 2;
const KEY_D: u32 = 1 << 3;
const KEY_Q: u32 = 1 << 4;
const KEY_E: u32 = 1 << 5;
const KEY_SHIFT: u32 = 1 << 6;

fn key_bit(code: KeyCode) -> u32 {
    match code {
        KeyCode::KeyW => KEY_W,
        KeyCode::KeyS => KEY_S,
        KeyCode::KeyA => KEY_A,
        KeyCode::KeyD => KEY_D,
        KeyCode::KeyQ => KEY_Q,
        KeyCode::KeyE => KEY_E,
        KeyCode::ShiftLeft | KeyCode::ShiftRight => KEY_SHIFT,
        _ => 0,
    }
}

fn controls_from(bits: u32) -> Controls {
    Controls {
        pitch: (if bits & KEY_S != 0 { 1.0 } else { 0.0 })
            - (if bits & KEY_W != 0 { 1.0 } else { 0.0 }),
        bank: (if bits & KEY_D != 0 { 1.0 } else { 0.0 })
            - (if bits & KEY_A != 0 { 1.0 } else { 0.0 }),
        yaw: (if bits & KEY_E != 0 { 1.0 } else { 0.0 })
            - (if bits & KEY_Q != 0 { 1.0 } else { 0.0 }),
        boost: bits & KEY_SHIFT != 0,
    }
}

struct App {
    window: Option<std::sync::Arc<Window>>,
    shared: std::sync::Arc<Shared>,
    proxy: winit::event_loop::EventLoopProxy<UserEvent>,
    render_thread: Option<std::thread::JoinHandle<()>>,
}
impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("explora")
            .with_inner_size(winit::dpi::LogicalSize::new(1600, 1000))
            .with_fullscreen(if std::env::args().any(|a| a == "--fullscreen") {
                Some(winit::window::Fullscreen::Borderless(None))
            } else {
                None
            });
        let window = std::sync::Arc::new(event_loop.create_window(attrs).expect("window"));
        let thread_window = window.clone();
        let shared = self.shared.clone();
        let proxy = self.proxy.clone();
        self.window = Some(window);
        self.render_thread = Some(std::thread::spawn(move || {
            render_main(thread_window, shared, proxy);
        }));
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                self.shared.exit.store(true, Ordering::Release);
            }
            WindowEvent::KeyboardInput { event: KeyEvent { physical_key, state, .. }, .. } => {
                if let PhysicalKey::Code(code) = physical_key {
                    let bit = key_bit(code);
                    if state.is_pressed() {
                        if code == KeyCode::F11 {
                            if let Some(window) = self.window.as_ref() {
                                let full = window.fullscreen().is_none();
                                window.set_fullscreen(full.then(|| {
                                    winit::window::Fullscreen::Borderless(None)
                                }));
                            }
                        }
                        self.shared.keys.fetch_or(bit, Ordering::Relaxed);
                    } else {
                        self.shared.keys.fetch_and(!bit, Ordering::Relaxed);
                    }
                }
            }
            _ => {}
        }
        let _ = event_loop;
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        if event == UserEvent::RenderDone {
            if let Some(handle) = self.render_thread.take() {
                let _ = handle.join();
            }
            event_loop.exit();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Event thread sleeps. The render thread never waits on it.
        event_loop.set_control_flow(ControlFlow::Wait);
    }
}

fn render_main(
    window: std::sync::Arc<Window>,
    shared: std::sync::Arc<Shared>,
    proxy: winit::event_loop::EventLoopProxy<UserEvent>,
) {
    vendor::pin_to_performance_cores();
    let mut gfx = unsafe { Gfx::new(&window) };
    {
        let mut hash = 0u64;
        for b in SHADER_MARKER.bytes() {
            hash = hash.wrapping_mul(1099511628211).wrapping_add(b as u64);
        }
        println!("plane shader hash: {:016x}", hash);
    }
    let mut vendor = unsafe { vendor::Vendor::open() };
    let mut pose = Pose::start();
    let mut accumulator = 0.0f32;
    let mut last = Instant::now();
    let boot = last;
    let mut stages = StageStats::default();
    let mut stat_timer = 0.0f32;
    let mut stat_frames = 0u32;
    let mut stat_skipped = 0u64;
    let shot_path = std::env::var("EXPLORA_SHOT").ok();
    let shot_frame: u64 = std::env::var("EXPLORA_SHOT_FRAME")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(120);
    let mut presented_total = 0u64;
    let mut shot_done = false;
    // Benchmark: skip 120 warmup frames, then average N presented
    // frames of engine cost and exit. Compositor pace excluded by
    // construction: only submit, update, and GPU pass count.
    let bench_target: Option<u64> = std::env::args()
        .position(|a| a == "--benchmark")
        .and_then(|i| std::env::args().nth(i + 1))
        .and_then(|v| v.parse().ok());
    let mut bench_seen = 0u64;
    loop {
        if shared.exit.load(Ordering::Acquire) {
            break;
        }
        let now = Instant::now();
        let dt = (now - last).as_secs_f32().clamp(0.0, 0.1);
        last = now;
        accumulator += dt;
        let cpu0 = Instant::now();
        let controls = controls_from(shared.keys.load(Ordering::Relaxed));
        let mut steps = 0;
        while accumulator >= SIM_STEP && steps < 5 {
            pose.step(&controls, SIM_STEP);
            accumulator -= SIM_STEP;
            steps += 1;
        }
        if steps == 5 {
            accumulator = 0.0;
        }
        let cpu1 = Instant::now();
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            continue;
        }
        let aspect = size.width as f32 / size.height as f32;
        let view_proj = camera::view_proj(&pose, aspect);
        let cpu2 = Instant::now();
        stages.add_cpu(
            cpu1.duration_since(cpu0).as_nanos() as u64,
            cpu2.duration_since(cpu1).as_nanos() as u64,
        );
        if gfx.extent.width != size.width || gfx.extent.height != size.height {
            unsafe { gfx.recreate(&window) };
            continue;
        }
        // Plane animation runs on the render thread with the same dt.
        let time = boot.elapsed().as_secs_f32();
        match unsafe { gfx.draw(&pose, &controls, &view_proj, time, dt, &mut stages) } {
            DrawResult::Rebuild => {
                unsafe { gfx.recreate(&window) };
                continue;
            }
            DrawResult::Skipped => {
                stat_skipped += 1;
            }
            DrawResult::Presented => {
                stat_frames += 1;
                presented_total += 1;
                if !shot_done {
                    if let (Some(path), true) = (shot_path.as_ref(), presented_total >= shot_frame) {
                        shot_done = true;
                        unsafe { gfx.screenshot(path) };
                    }
                }
                if let Some(target) = bench_target {
                    bench_seen += 1;
                    if bench_seen == 120 {
                        stages = StageStats::default();
                        stat_frames = 0;
                    }
                    if bench_seen >= 120 + target {
                        let (acq, sub, _pre, sim_ns, cam_ns, gpu_us) = stages.report();
                        let engine_us =
                            sub as f64 + sim_ns as f64 / 1000.0 + cam_ns as f64 / 1000.0 + gpu_us as f64;
                        println!(
                            "benchmark: {} frames | submit {sub} us update {:.1} us gpu {gpu_us} us | engine {:.1} us ({:.0} engine-fps) | acquire {acq} us",
                            stat_frames,
                            sim_ns as f64 / 1000.0 + cam_ns as f64 / 1000.0,
                            engine_us,
                            1_000_000.0 / engine_us.max(1.0),
                        );
                        break;
                    }
                }
            }
        }
        stat_timer += dt;
        if stat_timer >= 1.0 && bench_target.is_none() {
            let fps = stat_frames as f32 / stat_timer;
            stat_timer = 0.0;
            stat_frames = 0;
            let skipped = stat_skipped;
            stat_skipped = 0;
            let (acq, sub, pre, sim_ns, cam_ns, gpu_us) = stages.report();
            stages = StageStats::default();
            let stats = vendor.sample();
            println!(
                "fps {:.1} ({:.1} us) | acq {acq} sub {sub} pre {pre} us | sim {sim_ns} cam {cam_ns} ns gpu {gpu_us} us | skipped {} | speed {:.0} kt {} | GPU {}C {}MHz",
                fps,
                1_000_000.0 / fps.max(1.0),
                skipped,
                pose.speed * 1.944,
                if pose.boost > 0.5 { "BOOST" } else { "glide" },
                stats.temp_c,
                stats.clock_mhz,
            );
            window.set_title(&format!(
                "explora | {:.0} fps {:.0} kt {} | GPU {}C {}MHz",
                fps,
                pose.speed * 1.944,
                if pose.boost > 0.5 { "BOOST" } else { "glide" },
                stats.temp_c,
                stats.clock_mhz,
            ));
        }
    }
    let _ = proxy.send_event(UserEvent::RenderDone);
}

fn main() {
    let event_loop = EventLoop::<UserEvent>::with_user_event().build().expect("event loop");
    let proxy = event_loop.create_proxy();
    let mut app = App {
        window: None,
        shared: std::sync::Arc::new(Shared {
            exit: AtomicBool::new(false),
            keys: AtomicU32::new(0),
        }),
        proxy,
        render_thread: None,
    };
    event_loop.run_app(&mut app).expect("run");
}
