// Native boot: window, Vulkan device, swapchain, fixed loop.
// One canvas. One GPU. Fixed passes. No fallback.

mod audio;
mod hud;
mod fx_gpu;
mod plane;
mod quality;
mod vendor;

use ash::{vk, Entry};
use glam::Mat4;
use plane::Plane;
use quality::Quality;
use sim::camera::ChaseCamera;
use sim::effects::{self, Effects};
use sim::flight::{Controls, Pose, SIM_STEP};
use sim::wind::Wind;
use std::ffi::CStr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::{Window, WindowId};

const NVIDIA_VENDOR: u32 = 0x10DE;
const RENDER_BURST_DEFAULT: u32 = 1;
const RENDER_SAMPLES: vk::SampleCountFlags = vk::SampleCountFlags::TYPE_1;
const SHADER_MARKER: &str = include_str!("../shaders/plane.frag");

fn scaled_scene_extent(extent: vk::Extent2D, quality: Quality) -> vk::Extent2D {
    let [width, height] = quality.scene_size(extent.width, extent.height);
    vk::Extent2D {
        width,
        height,
    }
}

/// Render once per present for maximum presentation cadence. Extra passes only
/// exercise geometry without writing attachments and reduce real FPS.
/// Overridable for throughput experiments with EXPLORA_BURST (clamped 1..=256).
fn render_burst() -> u32 {
    std::env::var("EXPLORA_BURST")
        .ok()
        .and_then(|v| v.parse().ok())
        .map_or(RENDER_BURST_DEFAULT, |v: u32| v.clamp(1, 256))
}

/// Fine-grained microsecond timing breakdown across CPU stages, GPU timestamps, and presentation.
#[derive(Default)]
struct StageStats {
    acquire_us: u64,
    fence_wait_us: u64,
    submit_us: u64,
    present_us: u64,
    sim_us: u64,
    camera_us: u64,
    fx_us: u64,
    gpu_us: u64,
    gpu_pass_us: [u64; 7],
    gpu_samples: u64,
    frames: u64,
}

impl StageStats {
    fn add_batch(&mut self, acquire: u64, fence_wait: u64, submit: u64, present: u64, renders: u64) {
        self.acquire_us += acquire;
        self.fence_wait_us += fence_wait;
        self.submit_us += submit;
        self.present_us += present;
        self.frames += renders;
    }

    fn add_cpu(&mut self, sim: u64, camera: u64) {
        self.sim_us += sim;
        self.camera_us += camera;
    }

    fn add_fx(&mut self, fx: u64) {
        self.fx_us += fx;
    }

    fn add_gpu(&mut self, gpu_us: u64) {
        self.gpu_us += gpu_us;
        self.gpu_samples += 1;
    }

    fn add_gpu_pass(&mut self, pass: usize, us: u64) {
        if pass < 7 {
            self.gpu_pass_us[pass] += us;
        }
    }

    fn gpu_pass_avg(&self) -> [u64; 7] {
        let n = self.gpu_samples.max(1);
        let mut out = [0u64; 7];
        for (i, v) in self.gpu_pass_us.iter().enumerate() {
            out[i] = v / n;
        }
        out
    }

    fn report(&self) -> (u64, u64, u64, u64, u64, u64, u64, u64) {
        let n = self.frames.max(1);
        (
            self.acquire_us / n,
            self.fence_wait_us / n,
            self.submit_us / n,
            self.present_us / n,
            self.sim_us / n,
            self.camera_us / n,
            self.fx_us / n,
            self.gpu_us / self.gpu_samples.max(1),
        )
    }
}

/// Copy offline SPIR-V bytes (from build.rs shaderc output) into aligned
/// words for vkCreateShaderModule. Boot-time cost only.
pub(crate) fn spv_words(bytes: &[u8]) -> Vec<u32> {
    assert!(!bytes.is_empty() && bytes.len() % 4 == 0, "bad SPIR-V blob");
    let mut words = vec![0u32; bytes.len() / 4];
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), words.as_mut_ptr() as *mut u8, bytes.len());
    }
    // SPIR-V magic, catches truncated or non-SPIR-V blobs at boot.
    assert_eq!(words[0], 0x07230203, "bad SPIR-V magic");
    words
}

fn pick_present(modes: &[vk::PresentModeKHR]) -> vk::PresentModeKHR {
    if let Ok(value) = std::env::var("EXPLORA_PRESENT") {
        let mode = match value.as_str() {
            "immediate" => vk::PresentModeKHR::IMMEDIATE,
            "mailbox" => vk::PresentModeKHR::MAILBOX,
            "fifo" => vk::PresentModeKHR::FIFO,
            _ => panic!("EXPLORA_PRESENT must be immediate, mailbox, or fifo"),
        };
        assert!(
            modes.contains(&mode),
            "requested present mode {mode:?} unavailable: {modes:?}"
        );
        return mode;
    }
    // Prefer immediate, then mailbox. Neither guarantees nonblocking
    // acquire/present or bypasses the compositor.
    if modes.contains(&vk::PresentModeKHR::IMMEDIATE) {
        vk::PresentModeKHR::IMMEDIATE
    } else if modes.contains(&vk::PresentModeKHR::MAILBOX) {
        vk::PresentModeKHR::MAILBOX
    } else {
        vk::PresentModeKHR::FIFO
    }
}


pub(crate) unsafe fn find_memory_type(
    mem_props: &vk::PhysicalDeviceMemoryProperties,
    type_bits: u32,
    flags: vk::MemoryPropertyFlags,
) -> u32 {
    for i in 0..mem_props.memory_type_count {
        if (type_bits & (1 << i)) != 0
            && mem_props.memory_types[i as usize]
                .property_flags
                .contains(flags)
        {
            return i;
        }
    }
    panic!("no fixed memory type found");
}

/// Synchronization and command submission state for a single in-flight swapchain image.
#[allow(dead_code)]
struct Frame {
    pool: vk::CommandPool,
    cmds: Vec<vk::CommandBuffer>,
    frame_done: vk::Semaphore,
    fence: vk::Fence,
}

/// Per-swapchain-image depth stencil and optional MSAA color resolve attachments.
struct RenderTargets {
    depth_image: vk::Image,
    depth_memory: vk::DeviceMemory,
    depth_view: vk::ImageView,
    msaa_image: vk::Image,
    msaa_memory: vk::DeviceMemory,
    msaa_view: vk::ImageView,
}

/// Main Vulkan graphics device context, swapchain manager, and renderer state.
struct Gfx {
    _entry: Entry,
    instance: ash::Instance,
    surface_loader: ash::khr::surface::Instance,
    surface: vk::SurfaceKHR,
    device: ash::Device,
    queue: vk::Queue,
    present_queue: vk::Queue,
    queue_family: u32,
    burst: u32,
    swap_loader: ash::khr::swapchain::Device,
    swapchain: vk::SwapchainKHR,
    images: Vec<vk::Image>,
    views: Vec<vk::ImageView>,
    format: vk::Format,
    extent: vk::Extent2D,
    scene_extent: vk::Extent2D,
    quality: Quality,
    frames: Vec<Frame>,
    submitted: Vec<bool>,
    acquire_semaphores: Vec<vk::Semaphore>,
    acquire_index: usize,
    presentation_feedback: bool,
    present_id: u64,
    plane: Plane,
    targets: Vec<RenderTargets>,
    // Linear HDR scene targets (one per swapchain image) plus composite sets
    // sampling them for the final ACES pass to the swapchain.
    hdr_images: Vec<vk::Image>,
    hdr_memories: Vec<vk::DeviceMemory>,
    hdr_views: Vec<vk::ImageView>,
    comp_pool: vk::DescriptorPool,
    comp_sets: Vec<vk::DescriptorSet>,
    comp_sampler: vk::Sampler,
    last_presented: u32,
    timestamp_period_ns: f32,
}

impl Gfx {
    /// Initialize Vulkan instance, select target GPU, create swapchain, and allocate frame rendering resources.
    ///
    /// # Safety
    /// Must only be called once during engine initialization with a valid native window handle.
    unsafe fn new(window: &Window) -> Self {
        let quality = Quality::from_env();
        println!("render quality: {quality:?}");
        let entry = Entry::load().expect("no Vulkan loader");
        let display = window.display_handle().expect("no display").as_raw();
        println!(
            "window backend: {}",
            match display {
                winit::raw_window_handle::RawDisplayHandle::Wayland(_) => "Wayland",
                winit::raw_window_handle::RawDisplayHandle::Xlib(_)
                | winit::raw_window_handle::RawDisplayHandle::Xcb(_) => "X11",
                _ => "other",
            }
        );
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
        let timestamp_period_ns = instance
            .get_physical_device_properties(physical)
            .limits
            .timestamp_period;
        let max_aniso = instance
            .get_physical_device_properties(physical)
            .limits
            .max_sampler_anisotropy
            .min(16.0);
        let name = CStr::from_ptr(
            instance
                .get_physical_device_properties(physical)
                .device_name
                .as_ptr(),
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
        let priorities = [1.0f32, 1.0f32];
        let queue_info = vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family)
            .queue_priorities(&priorities);
        let fsr_supported = instance
            .enumerate_device_extension_properties(physical)
            .expect("device extensions")
            .iter()
            .any(|ext| {
                CStr::from_ptr(ext.extension_name.as_ptr())
                    == ash::khr::fragment_shading_rate::NAME
            });
        let mut fsr_features = vk::PhysicalDeviceFragmentShadingRateFeaturesKHR::default();
        let mut fsr_query = vk::PhysicalDeviceFeatures2::default().push_next(&mut fsr_features);
        instance.get_physical_device_features2(physical, &mut fsr_query);
        let ground_fsr = fsr_supported
            && fsr_features.pipeline_fragment_shading_rate == vk::TRUE;
        // Ray-traced soft shadows (VK_KHR_ray_query on the aircraft TLAS).
        // Feature support is probed up front; engines keep their analytic
        // fallback when any one of the three device features is missing.
        let mut accel_features = vk::PhysicalDeviceAccelerationStructureFeaturesKHR::default();
        let mut ray_query_features = vk::PhysicalDeviceRayQueryFeaturesKHR::default();
        let mut bda_features = vk::PhysicalDeviceBufferDeviceAddressFeatures::default();
        let mut rt_query = vk::PhysicalDeviceFeatures2::default()
            .push_next(&mut accel_features)
            .push_next(&mut ray_query_features)
            .push_next(&mut bda_features);
        instance.get_physical_device_features2(physical, &mut rt_query);
        let dev_ext_names: Vec<String> = instance
            .enumerate_device_extension_properties(physical)
            .expect("device extensions")
            .iter()
            .map(|e| {
                CStr::from_ptr(e.extension_name.as_ptr())
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        let accel_ext = dev_ext_names.iter().any(|n| n == ash::khr::acceleration_structure::NAME.to_str().unwrap());
        let rayq_ext = dev_ext_names.iter().any(|n| n == ash::khr::ray_query::NAME.to_str().unwrap());
        let rt_supported = accel_ext
            && rayq_ext
            && accel_features.acceleration_structure == vk::TRUE
            && ray_query_features.ray_query == vk::TRUE
            && bda_features.buffer_device_address == vk::TRUE;
        println!(
            "ray-traced shadows: {}",
            if rt_supported { "on" } else { "off (analytic fallback)" }
        );
        // Diagnostic only: NVIDIA WSI requests wp_presentation feedback for
        // present IDs. WAYLAND_DEBUG=1 then exposes actual display/zero-copy flags.
        let presentation_feedback = std::env::var_os("EXPLORA_PRESENT_FEEDBACK").is_some();
        println!("render schedule: {} passes/present, 1x MSAA", render_burst());
        let mut device_exts = vec![ash::khr::swapchain::NAME.as_ptr()];
        if ground_fsr {
            device_exts.push(ash::khr::fragment_shading_rate::NAME.as_ptr());
            fsr_features =
                vk::PhysicalDeviceFragmentShadingRateFeaturesKHR::default()
                    .pipeline_fragment_shading_rate(true);
        }
        if rt_supported {
            device_exts.push(ash::khr::acceleration_structure::NAME.as_ptr());
            device_exts.push(ash::khr::ray_query::NAME.as_ptr());
            // VK_KHR_acceleration_structure depends on this extension.
            device_exts.push(ash::khr::deferred_host_operations::NAME.as_ptr());
        }
        if presentation_feedback {
            device_exts.extend([
                ash::khr::present_id::NAME.as_ptr(),
                ash::khr::present_wait::NAME.as_ptr(),
            ]);
        }
        let mut present_id_features =
            vk::PhysicalDevicePresentIdFeaturesKHR::default().present_id(true);
        let mut present_wait_features =
            vk::PhysicalDevicePresentWaitFeaturesKHR::default().present_wait(true);
        let mut dyn_feat = vk::PhysicalDeviceVulkan13Features::default()
            .dynamic_rendering(true)
            .shader_demote_to_helper_invocation(true);
        let mut device_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(std::slice::from_ref(&queue_info))
            .enabled_extension_names(&device_exts)
            .push_next(&mut dyn_feat);
        if presentation_feedback {
            device_info = device_info
                .push_next(&mut present_id_features)
                .push_next(&mut present_wait_features);
        }
        if ground_fsr {
            device_info = device_info.push_next(&mut fsr_features);
        }
        if rt_supported {
            accel_features = vk::PhysicalDeviceAccelerationStructureFeaturesKHR::default()
                .acceleration_structure(true);
            ray_query_features =
                vk::PhysicalDeviceRayQueryFeaturesKHR::default().ray_query(true);
            bda_features =
                vk::PhysicalDeviceBufferDeviceAddressFeatures::default().buffer_device_address(true);
            device_info = device_info
                .push_next(&mut accel_features)
                .push_next(&mut ray_query_features)
                .push_next(&mut bda_features);
        }
        println!(
            "ground shading rate: {}",
            if ground_fsr { "2x2" } else { "native" }
        );
        let device = instance
            .create_device(physical, &device_info, None)
            .expect("device");
        let queue = device.get_device_queue(queue_family, 0);
        let present_queue = device.get_device_queue(queue_family, 1);
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
        println!(
            "swapchain caps: min_image_count {}, max_image_count {}",
            caps.min_image_count, caps.max_image_count
        );
        let size = window.inner_size();
        let extent = vk::Extent2D {
            width: size
                .width
                .clamp(caps.min_image_extent.width, caps.max_image_extent.width),
            height: size
                .height
                .clamp(caps.min_image_extent.height, caps.max_image_extent.height),
        };
        let scene_extent = scaled_scene_extent(extent, quality);
        // Request 8 swapchain images (max supported): eliminates acquire
        // starvation behind the Wayland compositor mailbox lifecycle.
        let image_count = 8u32.clamp(
            caps.min_image_count,
            if caps.max_image_count == 0 {
                u32::MAX
            } else {
                caps.max_image_count
            },
        );
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
        let swapchain = swap_loader
            .create_swapchain(&swap_info, None)
            .expect("swapchain");

        let plane = Plane::build(
            &device,
            &instance,
            physical,
            queue_family,
            queue,
            format.format,
            max_aniso,
            RENDER_SAMPLES,
            ground_fsr,
            rt_supported,
            quality,
        );
        let mut gfx = Self {
            _entry: entry,
            instance,
            surface_loader,
            surface,
            device,
            queue,
            present_queue,
            queue_family,
            burst: render_burst(),
            swap_loader,
            swapchain,
            images: Vec::new(),
            views: Vec::new(),
            format: format.format,
            extent,
            scene_extent,
            quality,
            frames: Vec::new(),
            submitted: Vec::new(),
            acquire_semaphores: Vec::new(),
            acquire_index: 0,
            presentation_feedback,
            present_id: 0,
            plane,
            targets: Vec::new(),
            hdr_images: Vec::new(),
            hdr_memories: Vec::new(),
            hdr_views: Vec::new(),
            comp_pool: vk::DescriptorPool::null(),
            comp_sets: Vec::new(),
            comp_sampler: vk::Sampler::null(),
            last_presented: 0,
            timestamp_period_ns,
        };
        gfx.build_swap_views();
        gfx.build_depth();
        gfx.build_frames();
        gfx
    }

    /// Create 2D color image views for each swapchain image.
    unsafe fn build_swap_views(&mut self) {
        self.images = self
            .swap_loader
            .get_swapchain_images(self.swapchain)
            .expect("images");
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
                self.device
                    .create_image_view(&view_info, None)
                    .expect("view")
            })
            .collect();
    }

    /// Retrieve the selected discrete NVIDIA physical device.
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

    /// Allocate dedicated device-local depth buffers and optional MSAA transient color buffers
    /// for every swapchain image.
    unsafe fn build_depth(&mut self) {
        // One color/depth target set per swapchain image lets independent
        // batches stay in flight without writing shared attachments.
        let device = &self.device;
        let instance = &self.instance;
        let physical = self.physical();
        let extent = self.scene_extent;
        let format = self.format;
        let samples = RENDER_SAMPLES;
        let make_image = |format: vk::Format, usage: vk::ImageUsageFlags| {
            let image_info = vk::ImageCreateInfo::default()
                .image_type(vk::ImageType::TYPE_2D)
                .format(format)
                .extent(vk::Extent3D {
                    width: extent.width,
                    height: extent.height,
                    depth: 1,
                })
                .mip_levels(1)
                .array_layers(1)
                .samples(samples)
                .tiling(vk::ImageTiling::OPTIMAL)
                .usage(usage)
                .sharing_mode(vk::SharingMode::EXCLUSIVE)
                .initial_layout(vk::ImageLayout::UNDEFINED);
            let image = device.create_image(&image_info, None).expect("target img");
            let req = device.get_image_memory_requirements(image);
            let mem_props = instance.get_physical_device_memory_properties(physical);
            let index = find_memory_type(
                &mem_props,
                req.memory_type_bits,
                vk::MemoryPropertyFlags::DEVICE_LOCAL,
            );
            let alloc = vk::MemoryAllocateInfo::default()
                .allocation_size(req.size)
                .memory_type_index(index);
            let memory = device.allocate_memory(&alloc, None).expect("target mem");
            device
                .bind_image_memory(image, memory, 0)
                .expect("target bind");
            (image, memory)
        };
        let mut targets = Vec::with_capacity(self.images.len());
        for _ in &self.images {
            let (msaa_image, msaa_memory, msaa_view) = if samples == vk::SampleCountFlags::TYPE_1 {
                (
                    vk::Image::null(),
                    vk::DeviceMemory::null(),
                    vk::ImageView::null(),
                )
            } else {
                let (image, memory) = make_image(
                    format,
                    vk::ImageUsageFlags::COLOR_ATTACHMENT
                        | vk::ImageUsageFlags::TRANSIENT_ATTACHMENT,
                );
                let view_info = vk::ImageViewCreateInfo::default()
                    .image(image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(format)
                    .subresource_range(
                        vk::ImageSubresourceRange::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .base_mip_level(0)
                            .level_count(1)
                            .base_array_layer(0)
                            .layer_count(1),
                    );
                let view = device
                    .create_image_view(&view_info, None)
                    .expect("msaa view");
                (image, memory, view)
            };
            let (depth_image, depth_memory) = make_image(
                vk::Format::D32_SFLOAT,
                vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT,
            );
            let depth_view_info = vk::ImageViewCreateInfo::default()
                .image(depth_image)
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
            let depth_view = device
                .create_image_view(&depth_view_info, None)
                .expect("depth view");
            targets.push(RenderTargets {
                depth_image,
                depth_memory,
                depth_view,
                msaa_image,
                msaa_memory,
                msaa_view,
            });
        }
        self.targets = targets;
        // Linear HDR scene targets, one per swapchain image. The airframe,
        // plume, trail, and glass passes compose here; composite reads them.
        // Packed 32-bit HDR: same 1.0-exceeding range as RGBA16F at half the
        // framebuffer bandwidth. No framebuffer alpha needed: blending uses
        // shader-output alpha (SRC_ALPHA), and composite reads RGB only.
        let hdr_format = vk::Format::B10G11R11_UFLOAT_PACK32;
        for _ in &self.images {
            let hdr_info = vk::ImageCreateInfo::default()
                .image_type(vk::ImageType::TYPE_2D)
                .format(hdr_format)
                .extent(vk::Extent3D {
                    width: extent.width,
                    height: extent.height,
                    depth: 1,
                })
                .mip_levels(1)
                .array_layers(1)
                .samples(vk::SampleCountFlags::TYPE_1)
                .tiling(vk::ImageTiling::OPTIMAL)
                .usage(
                    vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED,
                )
                .sharing_mode(vk::SharingMode::EXCLUSIVE)
                .initial_layout(vk::ImageLayout::UNDEFINED);
            let hdr_image = device.create_image(&hdr_info, None).expect("hdr img");
            let req = device.get_image_memory_requirements(hdr_image);
            let mem_props = instance.get_physical_device_memory_properties(physical);
            let index = find_memory_type(
                &mem_props,
                req.memory_type_bits,
                vk::MemoryPropertyFlags::DEVICE_LOCAL,
            );
            let alloc = vk::MemoryAllocateInfo::default()
                .allocation_size(req.size)
                .memory_type_index(index);
            let hdr_memory = device.allocate_memory(&alloc, None).expect("hdr mem");
            device
                .bind_image_memory(hdr_image, hdr_memory, 0)
                .expect("hdr bind");
            let view_info = vk::ImageViewCreateInfo::default()
                .image(hdr_image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(hdr_format)
                .subresource_range(
                    vk::ImageSubresourceRange::default()
                        .aspect_mask(vk::ImageAspectFlags::COLOR)
                        .base_mip_level(0)
                        .level_count(1)
                        .base_array_layer(0)
                        .layer_count(1),
                );
            let hdr_view = device.create_image_view(&view_info, None).expect("hdr view");
            self.hdr_images.push(hdr_image);
            self.hdr_memories.push(hdr_memory);
            self.hdr_views.push(hdr_view);
        }
        let smp_info = vk::SamplerCreateInfo::default()
            .mag_filter(vk::Filter::LINEAR)
            .min_filter(vk::Filter::LINEAR)
            .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
            .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .max_lod(vk::LOD_CLAMP_NONE);
        self.comp_sampler = device.create_sampler(&smp_info, None).expect("compsmp");
        println!(
            "hdr targets: {}x{} ({}x{} output) B10G11R11 x{} + composite sampler",
            self.scene_extent.width,
            self.scene_extent.height,
            self.extent.width,
            self.extent.height,
            self.images.len()
        );
    }

    /// Allocate per-frame synchronization objects, command buffers, and pre-record render passes.
    unsafe fn build_frames(&mut self) {
        let sem_info = vk::SemaphoreCreateInfo::default();
        self.acquire_semaphores = (0..self.images.len())
            .map(|_| {
                self.device
                    .create_semaphore(&sem_info, None)
                    .expect("acquire sem")
            })
            .collect();
        self.acquire_index = 0;
        self.plane
            .prepare_query_pool(&self.device, self.images.len());
        println!(
            "swapchain: {}x{}, {} images",
            self.extent.width,
            self.extent.height,
            self.images.len()
        );
        // One frame slot per swapchain image. The plane records once
        // per image. The hot loop only waits, copies uniforms, submits.
        self.plane.build_frames(
            &self.device,
            &self.instance,
            self.physical(),
            self.images.len() * self.burst as usize,
        );
        // Composite sets sample the HDR target of the matching frame slot.
        // With burst > 1 every render shares image's HDR view; only the final
        // (measured) pass runs composite.
        let comp_pool_sizes = [
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(self.images.len() as u32 * self.burst),
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::SAMPLER)
                .descriptor_count(self.images.len() as u32 * self.burst),
        ];
        self.comp_pool = self
            .device
            .create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .pool_sizes(&comp_pool_sizes)
                    .max_sets(self.images.len() as u32 * self.burst),
                None,
            )
            .expect("comppool");
        let comp_layouts =
            vec![self.plane.composite_set_layout(); self.images.len() * self.burst as usize];
        self.comp_sets = self
            .device
            .allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(self.comp_pool)
                    .set_layouts(&comp_layouts),
            )
            .expect("compsets");
        for (i, set) in self.comp_sets.iter().enumerate() {
            let img = i / self.burst as usize;
            let tex_ref = [vk::DescriptorImageInfo::default()
                .image_view(self.hdr_views[img])
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
            let smp_ref = [vk::DescriptorImageInfo::default().sampler(self.comp_sampler)];
            self.device.update_descriptor_sets(
                &[vk::WriteDescriptorSet::default()
                    .dst_set(*set)
                    .dst_binding(0)
                    .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                    .image_info(&tex_ref)],
                &[],
            );
            self.device.update_descriptor_sets(
                &[vk::WriteDescriptorSet::default()
                    .dst_set(*set)
                    .dst_binding(1)
                    .descriptor_type(vk::DescriptorType::SAMPLER)
                    .image_info(&smp_ref)],
                &[],
            );
        }
        for (index, image) in self.images.iter().enumerate() {
            let target = &self.targets[index];
            let pool_info = vk::CommandPoolCreateInfo::default()
                .queue_family_index(self.queue_family)
                .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
            let pool = self
                .device
                .create_command_pool(&pool_info, None)
                .expect("pool");
            let alloc = vk::CommandBufferAllocateInfo::default()
                .command_pool(pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(self.burst);
            let cmds = self.device.allocate_command_buffers(&alloc).expect("cmd");
            let semaphore = vk::SemaphoreCreateInfo::default();
            let fence = vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED);
            self.frames.push(Frame {
                pool,
                cmds: cmds.clone(),
                frame_done: self.device.create_semaphore(&semaphore, None).expect("sem"),
                fence: self.device.create_fence(&fence, None).expect("fence"),
            });
            self.submitted.push(false);
            for (render, cmd) in cmds.into_iter().enumerate() {
                let slot = index * self.burst as usize + render;
                self.plane.record(
                    &self.device,
                    cmd,
                    *image,
                    self.views[index],
                    target.msaa_image,
                    target.msaa_view,
                    target.depth_image,
                    target.depth_view,
                    self.hdr_images[index],
                    self.hdr_views[index],
                    self.comp_sets[slot],
                    self.scene_extent,
                    self.extent,
                    slot,
                    (index * 8) as u32,
                    render + 1 == self.burst as usize,
                );
            }
        }
    }

    /// Tear down all swapchain-dependent GPU resources prior to resize or exit.
    unsafe fn destroy_swap_side(&mut self) {
        for sem in self.acquire_semaphores.drain(..) {
            self.device.destroy_semaphore(sem, None);
        }
        for frame in self.frames.drain(..) {
            self.device.destroy_semaphore(frame.frame_done, None);
            self.device.destroy_fence(frame.fence, None);
            self.device.destroy_command_pool(frame.pool, None);
        }
        self.submitted.clear();
        self.plane.destroy_frames(&self.device);
        self.comp_sets.clear();
        if self.comp_pool != vk::DescriptorPool::null() {
            self.device.destroy_descriptor_pool(self.comp_pool, None);
            self.comp_pool = vk::DescriptorPool::null();
        }
        if self.comp_sampler != vk::Sampler::null() {
            self.device.destroy_sampler(self.comp_sampler, None);
            self.comp_sampler = vk::Sampler::null();
        }
        for ((img, mem), view) in self
            .hdr_images
            .drain(..)
            .zip(self.hdr_memories.drain(..))
            .zip(self.hdr_views.drain(..))
        {
            self.device.destroy_image_view(view, None);
            self.device.destroy_image(img, None);
            self.device.free_memory(mem, None);
        }
        for target in self.targets.drain(..) {
            self.device.destroy_image_view(target.depth_view, None);
            self.device.destroy_image(target.depth_image, None);
            self.device.free_memory(target.depth_memory, None);
            if target.msaa_image != vk::Image::null() {
                self.device.destroy_image_view(target.msaa_view, None);
                self.device.destroy_image(target.msaa_image, None);
                self.device.free_memory(target.msaa_memory, None);
            }
        }
        for view in self.views.drain(..) {
            self.device.destroy_image_view(view, None);
        }
        self.swap_loader.destroy_swapchain(self.swapchain, None);
    }

    /// Recreate the Vulkan swapchain and dependent attachments on window resize.
    unsafe fn recreate(&mut self, window: &Window) {
        self.device.device_wait_idle().expect("idle");
        self.destroy_swap_side();
        let caps = self
            .surface_loader
            .get_physical_device_surface_capabilities(self.physical(), self.surface)
            .expect("caps");
        let size = window.inner_size();
        self.extent = vk::Extent2D {
            width: size
                .width
                .clamp(caps.min_image_extent.width, caps.max_image_extent.width),
            height: size
                .height
                .clamp(caps.min_image_extent.height, caps.max_image_extent.height),
        };
        self.scene_extent = scaled_scene_extent(self.extent, self.quality);
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
        // Request 8 swapchain images (max supported): eliminates acquire
        // starvation behind the Wayland compositor mailbox lifecycle.
        let image_count = 8u32.clamp(
            caps.min_image_count,
            if caps.max_image_count == 0 {
                u32::MAX
            } else {
                caps.max_image_count
            },
        );
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
        self.swapchain = self
            .swap_loader
            .create_swapchain(&swap_info, None)
            .expect("swap");
        self.build_swap_views();
        self.build_depth();
        self.build_frames();
    }

    /// Capture a synchronous screenshot of the most recently presented swapchain image and save it as a PPM file.
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
        let buffer = self
            .device
            .create_buffer(&buffer_info, None)
            .expect("shot buf");
        let req = self.device.get_buffer_memory_requirements(buffer);
        let mem_props = self
            .instance
            .get_physical_device_memory_properties(self.physical());
        let index = find_memory_type(
            &mem_props,
            req.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        );
        let alloc = vk::MemoryAllocateInfo::default()
            .allocation_size(req.size)
            .memory_type_index(index);
        let memory = self.device.allocate_memory(&alloc, None).expect("shot mem");
        self.device
            .bind_buffer_memory(buffer, memory, 0)
            .expect("shot bind");
        let pool_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(self.queue_family)
            .flags(vk::CommandPoolCreateFlags::TRANSIENT);
        let pool = self
            .device
            .create_command_pool(&pool_info, None)
            .expect("shot pool");
        let alloc = vk::CommandBufferAllocateInfo::default()
            .command_pool(pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let cmd = self
            .device
            .allocate_command_buffers(&alloc)
            .expect("shot cmd")[0];
        // Copy the most recently presented image back, never a
        // stale sibling. Copying image zero measured staleness
        // instead of shimmer.
        let image = self.images[self.last_presented as usize];
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
            .image_extent(vk::Extent3D {
                width: self.extent.width,
                height: self.extent.height,
                depth: 1,
            });
        let begin = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        self.device
            .begin_command_buffer(cmd, &begin)
            .expect("shot begin");
        self.device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[to_transfer],
        );
        self.device.cmd_copy_image_to_buffer(
            cmd,
            image,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            buffer,
            &[region],
        );
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
        let fence = self
            .device
            .create_fence(&fence_info, None)
            .expect("shot fence");
        let cmds = [cmd];
        let submit = vk::SubmitInfo::default().command_buffers(&cmds);
        self.device
            .queue_submit(self.queue, &[submit], fence)
            .expect("shot submit");
        self.device
            .wait_for_fences(&[fence], true, u64::MAX)
            .expect("shot wait");
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

    /// Execute a hot-loop rendering batch: acquire swapchain image, update uniforms,
    /// submit command buffers, and present to the display engine.
    unsafe fn draw(
        &mut self,
        pose: &Pose,
        _u: &Controls,
        view_proj: &Mat4,
        origin: glam::Vec3,
        eye_rel: glam::Vec3,
        time: f32,
        fx: &Effects,
        sim_stepped: bool,
        stats: &mut StageStats,
        cam_frame: &sim::camera::CameraFrame,
        wind_strength: f32,
    ) -> DrawResult {
        // Hot loop: wait, update part uniforms, submit, present.
        // The wait includes WSI backpressure and GPU completion. Skipping on timeout was
        // measured slower: released images beat skipped frames.
        let t0 = std::time::Instant::now();
        let acquire_sem = self.acquire_semaphores[self.acquire_index];
        let next = self.swap_loader.acquire_next_image(
            self.swapchain,
            u64::MAX,
            acquire_sem,
            vk::Fence::null(),
        );
        let image_index = match next {
            Ok((index, _)) => {
                self.acquire_index = (self.acquire_index + 1) % self.acquire_semaphores.len();
                index as usize
            }
            Err(vk::Result::TIMEOUT) | Err(vk::Result::NOT_READY) => {
                return DrawResult::Skipped;
            }
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => return DrawResult::Rebuild,
            Err(error) => panic!("acquire failed: {error:?}"),
        };
        let t_acq = std::time::Instant::now();
        let frame = &self.frames[image_index];
        self.device
            .wait_for_fences(&[frame.fence], true, u64::MAX)
            .expect("frame fence");
        self.device
            .reset_fences(&[frame.fence])
            .expect("reset fence");
        let t_fence = std::time::Instant::now();
        // Ordered command buffers reuse these timestamps; the final result
        // measures the final complete render of the batch: q0 start, q1
        // opaque, q2 sky+ground, q3 clouds, q4 plume, q5 trail, q6 glass,
        // q7 composite end.
        if self.submitted[image_index] {
            let mut stamps = [0u64; 8];
            let query_ok = self
                .device
                .get_query_pool_results(
                    self.plane.query_pool(),
                    (image_index * 8) as u32,
                    &mut stamps,
                    vk::QueryResultFlags::TYPE_64,
                )
                .is_ok();
            if query_ok && stamps[7] >= stamps[0] {
                let period = self.timestamp_period_ns as f64 / 1000.0;
                let mut prev = stamps[0];
                for (i, pass) in stamps.iter().skip(1).enumerate() {
                    if *pass >= prev {
                        stats.add_gpu_pass(i, ((*pass - prev) as f64 * period) as u64);
                        prev = *pass;
                    } else {
                        break;
                    }
                }
                stats.add_gpu(((stamps[7] - stamps[0]) as f64 * period) as u64);
            }
        }
        // FX ribbon/cone buffers refill only when the 144 Hz sim advanced;
        // at 1400 presents/s most frames reuse the slot's last fill.
        let t_fx = std::time::Instant::now();
        for render in 0..self.burst {
            self.plane.update(
                pose,
                view_proj,
                origin,
                eye_rel,
                time,
                image_index * self.burst as usize + render as usize,
                render + 1 == self.burst,
                fx,
                sim_stepped,
                cam_frame,
                wind_strength,
            );
        }
        stats.add_fx(t_fx.elapsed().as_nanos() as u64);
        let wait_sems = [acquire_sem];
        let wait_stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
        let signal = [frame.frame_done];
        let empty_cmds = [];
        let no_gpu = std::env::var_os("EXPLORA_NO_GPU").is_some();
        let submit = vk::SubmitInfo::default()
            .wait_semaphores(&wait_sems)
            .wait_dst_stage_mask(&wait_stages)
            .command_buffers(if no_gpu { &empty_cmds } else { &frame.cmds })
            .signal_semaphores(&signal);
        let t1 = std::time::Instant::now();
        self.device
            .queue_submit(self.queue, &[submit], frame.fence)
            .expect("submit");
        self.submitted[image_index] = true;
        let t2 = std::time::Instant::now();
        let swapchains = [self.swapchain];
        let indices = [image_index as u32];
        self.present_id += 1;
        let ids = [self.present_id];
        let mut id_info = vk::PresentIdKHR::default().present_ids(&ids);
        let mut present_info = vk::PresentInfoKHR::default()
            .wait_semaphores(&signal)
            .swapchains(&swapchains)
            .image_indices(&indices);
        if self.presentation_feedback {
            present_info = present_info.push_next(&mut id_info);
        }
        match self.swap_loader.queue_present(self.present_queue, &present_info) {
            Ok(suboptimal) => {
                let t3 = std::time::Instant::now();
                stats.add_batch(
                    t_acq.duration_since(t0).as_micros() as u64,
                    t_fence.duration_since(t_acq).as_micros() as u64,
                    t2.duration_since(t1).as_micros() as u64,
                    t3.duration_since(t2).as_micros() as u64,
                    self.burst as u64,
                );
                if suboptimal {
                    return DrawResult::Rebuild;
                }
                self.last_presented = image_index as u32;
                DrawResult::Presented(self.burst)
            }
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => DrawResult::Rebuild,
            Err(error) => panic!("present failed: {error:?}"),
        }
    }
}

/// Result of a frame draw call.
#[derive(PartialEq, Eq)]
enum DrawResult {
    /// Number of render passes successfully completed and presented.
    Presented(u32),
    /// Frame acquisition was skipped due to timeout or non-ready status.
    Skipped,
    /// Swapchain is suboptimal or out of date and must be recreated.
    Rebuild,
}

/// Custom user event forwarded from background worker threads to the winit event loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UserEvent {
    /// Render thread has completed all work and shut down.
    RenderDone,
}

/// Global exit flag set by POSIX signal handlers (SIGINT / SIGTERM).
static SIGNAL_EXIT: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_signal(_sig: libc::c_int) {
    if SIGNAL_EXIT.swap(true, Ordering::SeqCst) {
        // Second signal: force immediate termination
        unsafe {
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            libc::signal(libc::SIGTERM, libc::SIG_DFL);
            libc::raise(libc::SIGINT);
        }
    }
}

/// Thread-safe state shared between the main UI event thread and the rendering thread.
struct Shared {
    /// Atomic exit flag signaled by window close, ESC/Ctrl+C, or SIGINT.
    exit: AtomicBool,
    /// Bitmask of currently depressed flight control keys.
    keys: AtomicU32,
    ui: AtomicU32,
}

impl Shared {
    /// Check whether exit has been requested either via application event or POSIX signal.
    #[inline]
    fn should_exit(&self) -> bool {
        self.exit.load(Ordering::Acquire) || SIGNAL_EXIT.load(Ordering::Acquire)
    }

    /// Request application shutdown across all threads.
    #[inline]
    fn request_exit(&self) {
        self.exit.store(true, Ordering::Release);
        SIGNAL_EXIT.store(true, Ordering::Release);
    }
}

const KEY_W: u32 = 1;
const KEY_S: u32 = 1 << 1;
const KEY_A: u32 = 1 << 2;
const KEY_D: u32 = 1 << 3;
const KEY_Q: u32 = 1 << 4;
const KEY_E: u32 = 1 << 5;
const KEY_SHIFT: u32 = 1 << 6;

/// Map a physical keycode into its corresponding bitmask flag.
fn key_bit(code: KeyCode) -> u32 {
    match code {
        KeyCode::KeyW | KeyCode::ArrowUp => KEY_W,
        KeyCode::KeyS | KeyCode::ArrowDown => KEY_S,
        KeyCode::KeyA | KeyCode::ArrowLeft => KEY_A,
        KeyCode::KeyD | KeyCode::ArrowRight => KEY_D,
        KeyCode::KeyQ => KEY_Q,
        KeyCode::KeyE => KEY_E,
        KeyCode::ShiftLeft | KeyCode::ShiftRight => KEY_SHIFT,
        _ => 0,
    }
}

/// Decode the current pressed-key bitmask into normalized flight control inputs.
fn controls_from(bits: u32) -> Controls {
    Controls {
        pitch: (if bits & KEY_S != 0 { 1.0 } else { 0.0 })
            - (if bits & KEY_W != 0 { 1.0 } else { 0.0 }),
        bank: (if bits & KEY_A != 0 { 1.0 } else { 0.0 })
            - (if bits & KEY_D != 0 { 1.0 } else { 0.0 }),
        yaw: (if bits & KEY_E != 0 { 1.0 } else { 0.0 })
            - (if bits & KEY_Q != 0 { 1.0 } else { 0.0 }),
        boost: bits & KEY_SHIFT != 0,
    }
}

/// Main winit application handler managing window creation, input routing, and render thread lifecycle.
struct App {
    window: Option<std::sync::Arc<Window>>,
    shared: std::sync::Arc<Shared>,
    proxy: winit::event_loop::EventLoopProxy<UserEvent>,
    render_thread: Option<std::thread::JoinHandle<()>>,
    ctrl_pressed: bool,
}
impl ApplicationHandler<UserEvent> for App {
    /// Window creation and render thread startup when the application is resumed.
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

    /// Handle window-level input events: close requests, fullscreen toggle (F11), and flight controls.
    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                self.shared.request_exit();
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.ctrl_pressed = modifiers.state().control_key();
            }
            WindowEvent::Focused(false) => {
                self.shared.keys.store(0, Ordering::Relaxed);
                self.ctrl_pressed = false;
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key,
                        logical_key,
                        state,
                        repeat,
                        ..
                    },
                ..
            } => {
                let pressed = state.is_pressed();

                // Track physical Control key presses
                if let PhysicalKey::Code(code) = physical_key {
                    if code == KeyCode::ControlLeft || code == KeyCode::ControlRight {
                        self.ctrl_pressed = pressed;
                    }
                }

                // Exit requested via ESC shortcut or Ctrl+C combination
                let is_esc = matches!(logical_key, Key::Named(NamedKey::Escape))
                    || matches!(physical_key, PhysicalKey::Code(KeyCode::Escape));
                let is_ctrl_c = (matches!(physical_key, PhysicalKey::Code(KeyCode::KeyC))
                    && self.ctrl_pressed)
                    || matches!(&logical_key, Key::Character(s) if s == "\x03");

                if pressed && (is_esc || is_ctrl_c) {
                    self.shared.request_exit();
                    return;
                }

                if let PhysicalKey::Code(code) = physical_key {
                    let bit = key_bit(code);
                    if pressed {
                        if !repeat {
                            let toggle = match code {
                                KeyCode::KeyH => hud::VISIBLE,
                                KeyCode::F1 => hud::HELP,
                                KeyCode::KeyM => hud::AUDIO,
                                KeyCode::KeyP => hud::PAUSED,
                                _ => 0,
                            };
                            self.shared.ui.fetch_xor(toggle, Ordering::Relaxed);
                            if code == KeyCode::KeyR { self.shared.ui.fetch_or(hud::RESET, Ordering::Relaxed); }
                        }
                        if code == KeyCode::F11 && !repeat {
                            if let Some(window) = self.window.as_ref() {
                                let full = window.fullscreen().is_none();
                                window.set_fullscreen(
                                    full.then(|| winit::window::Fullscreen::Borderless(None)),
                                );
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

    /// Process user events forwarded from background threads.
    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        if event == UserEvent::RenderDone {
            if let Some(handle) = self.render_thread.take() {
                let _ = handle.join();
            }
            event_loop.exit();
        }
    }

    /// Put event thread to sleep when no UI events are pending, or exit if shutdown requested.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.shared.should_exit() {
            event_loop.exit();
        } else {
            // Event thread sleeps. The render thread never waits on it.
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}

/// Dedicated high-frequency render and simulation loop thread.
///
/// Runs decoupled from the window event loop:
/// - Advances aerodynamic flight simulation in fixed `SIM_STEP` intervals.
/// - Calculates camera-relative floating-origin transforms.
/// - Submits batched Vulkan command buffers and handles swapchain presentation.
/// - Profiles CPU/GPU stage latencies and updates window title with real-time telemetry.
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
    let wind_strength = std::env::var("EXPLORA_WIND")
        .map(|value| {
            value.parse::<f32>()
                .expect("EXPLORA_WIND must be a number from 0 to 3")
        })
        .unwrap_or(1.0);
    let wind = Wind::new(wind_strength);
    println!(
        "weather wind strength: {} (0 = calm, 1 = breeze, 3 = strong)",
        wind.strength()
    );
    let mut pose = Pose::start();
    pose.x = world::SPAWN_X;
    pose.z = world::SPAWN_Z;
    pose.y = world::SPAWN_ALTITUDE;
    if let Ok(alt_str) = std::env::var("EXPLORA_ALT") {
        if let Ok(alt) = alt_str.parse::<f32>() {
            if alt.is_finite() { pose.y = alt; }
        }
    }
    if let Ok(hdg_str) = std::env::var("EXPLORA_HEADING") {
        if let Ok(hdg) = hdg_str.parse::<f32>() {
            if hdg.is_finite() {
                pose.heading = hdg;
                pose.orientation = glam::Quat::from_rotation_y(hdg);
                pose.velocity = pose.orientation * glam::Vec3::Z * pose.speed;
            }
        }
    }
    // Start trimmed relative to the air mass, with its drift already included
    // in world velocity. This avoids a sudden sideslip impulse at spawn.
    pose.velocity += wind.velocity(glam::Vec3::new(pose.x, pose.y, pose.z), 0.0);
    pose.y = pose.y.max(world::collision_height_at(pose.x as f64, pose.z as f64) + world::CLEARANCE_METRES);
    let spawn_pose = pose;
    let audio = audio::Audio::start();
    let mut prev_pose = pose;
    let mut fx = effects::Effects::new();
    let mut chase_cam = ChaseCamera::new();
    let mut accumulator = 0.0f32;
    let mut simulation_time = 0.0f32;
    let mut last = Instant::now();
    let boot = last;
    let mut stages = StageStats::default();
    let mut stat_timer = 0.0f32;
    let mut stat_frames = 0u32;
    let mut stat_presents = 0u32;
    let mut stat_skipped = 0u64;
    let shot_path = std::env::var("EXPLORA_SHOT").ok();
    let shot_frames: Vec<u64> = std::env::var("EXPLORA_SHOT_FRAME")
        .map(|v| v.split(',').filter_map(|s| s.trim().parse().ok()).collect())
        .unwrap_or_else(|_| vec![120]);
    let mut presented_total = 0u64;
    // Benchmark: allow two seconds for GPU clocks/window transitions, skip
    // another 120 warmup frames, then average N submitted
    // frames and exit. Report wall cadence separately: CPU submission and GPU
    // execution overlap, so their sum is not a frame time or a throughput limit.
    let bench_target: Option<u64> = std::env::args()
        .position(|a| a == "--benchmark")
        .and_then(|i| std::env::args().nth(i + 1))
        .and_then(|v| v.parse().ok());
    let mut bench_seen = 0u64;
    let mut bench_presents = 0u64;
    let mut bench_start = Instant::now();
    let frozen = std::env::var("EXPLORA_FREEZE").is_ok();
    // EXPLORA_FREEZE=pose freezes the sim pose but lets time run,
    // isolating time-driven terms from pose-driven ones.
    let freeze_pose = frozen || std::env::var("EXPLORA_FREEZE_POSE").is_ok();
    loop {
        if shared.should_exit() {
            break;
        }
        let now = Instant::now();
        let dt = (now - last).as_secs_f32().clamp(0.0, 0.1);
        last = now;
        accumulator += dt;
        let ui = shared.ui.fetch_and(!hud::RESET, Ordering::Relaxed);
        let paused = ui & hud::PAUSED != 0;
        if ui & hud::RESET != 0 {
            pose = spawn_pose;
            prev_pose = pose;
            fx = Effects::new();
            chase_cam.snap(&pose);
            accumulator = 0.0;
        }
        if paused { accumulator = 0.0; prev_pose = pose; }
        let cpu0 = Instant::now();
        let mut controls = controls_from(shared.keys.load(Ordering::Relaxed));
        // Screenshot helpers: force flight regimes without keyboard input.
        // EXPLORA_BOOST=1 holds full burner, EXPLORA_BANK=1 holds a hard left turn.
        if std::env::var_os("EXPLORA_BOOST").is_some() {
            controls.boost = true;
        }
        if std::env::var_os("EXPLORA_BANK").is_some() {
            controls.bank = 1.0;
        }
        if let Ok(pitch_str) = std::env::var("EXPLORA_PITCH") {
            if let Ok(p) = pitch_str.parse::<f32>() {
                controls.pitch = p;
            }
        }
        let mut steps = 0;
        while accumulator >= SIM_STEP && steps < 5 {
            prev_pose = pose;
            let air_motion = wind.velocity(
                glam::Vec3::new(pose.x, pose.y, pose.z),
                simulation_time,
            );
            if !freeze_pose {
                pose.step_with_wind(&controls, SIM_STEP, air_motion);
                // Gentle free-flight safety floor, including lake and landmark roofs.
                let floor = world::collision_height_at(pose.x as f64, pose.z as f64) + world::CLEARANCE_METRES;
                if pose.y < floor { pose.y = floor; pose.velocity.y = pose.velocity.y.max(0.0); }
            }
            if !frozen {
                gfx.plane.step_animation(&controls, &pose, SIM_STEP);
                // FX sim at same 144 Hz: emitters from node path, load from wing G.
                let (epos, edir) = gfx.plane.emitter_world(&pose);
                let load = pose.load.clamp(-4.0, 3.5);
                fx.step_with_wind(
                    SIM_STEP,
                    &epos,
                    &edir,
                    gfx.plane.engine_spool(),
                    pose.speed,
                    pose.y,
                    load,
                    air_motion,
                );
                simulation_time += SIM_STEP;
            }
            accumulator -= SIM_STEP;
            steps += 1;
        }
        if steps == 5 {
            accumulator = 0.0;
        }
        // Sub-step pose interpolation: eliminates simulation-to-render beat-frequency
        // micro-stutter and tail jitter at any display refresh rate (60Hz, 165Hz, 240Hz, or uncapped).
        let alpha = if freeze_pose || paused {
            0.0
        } else {
            (accumulator / SIM_STEP).clamp(0.0, 1.0)
        };
        let render_pose = prev_pose.interpolate(&pose, alpha);
        audio.update(render_pose.speed, gfx.plane.engine_spool(), render_pose.load,
            ui & hud::AUDIO != 0 && !paused && !frozen);
        gfx.plane.set_hud(hud::pack(render_pose.speed, render_pose.y, render_pose.heading,
            render_pose.velocity.y, gfx.plane.engine_spool(),
            render_pose.y - world::collision_height_at(render_pose.x as f64, render_pose.z as f64), ui));

        let sim_stepped = steps > 0;
        let cpu1 = Instant::now();
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            continue;
        }
        let aspect = size.width as f32 / size.height as f32;
        // Floating origin at the plane. World coordinates reach
        // kilometers; rendering relative keeps float32 exact.
        let origin = glam::Vec3::new(render_pose.x, render_pose.y, render_pose.z);
        fx.set_origin(origin);
        let camera_wind = wind.velocity(origin, simulation_time);
        let mut cam_frame = chase_cam.step_with_wind(
            &render_pose, &controls, dt, aspect, origin, camera_wind,
        );
        let eye_floor = world::collision_height_at(cam_frame.eye_world.x as f64, cam_frame.eye_world.z as f64) + 8.0;
        if cam_frame.eye_world.y < eye_floor {
            let lift = eye_floor - cam_frame.eye_world.y;
            cam_frame.eye_world.y += lift;
            cam_frame.eye_rel.y += lift;
            cam_frame.target_rel.y += lift;
            let mut proj = Mat4::perspective_rh(cam_frame.fov_y, aspect, sim::camera::NEAR, sim::camera::FAR);
            proj.y_axis.y *= -1.0;
            cam_frame.view_proj = proj * Mat4::look_at_rh(cam_frame.eye_rel, cam_frame.target_rel, cam_frame.camera_up);
        }
        let view_proj = cam_frame.view_proj;
        let eye_rel = cam_frame.eye_rel;
        let cpu2 = Instant::now();
        stages.add_cpu(
            cpu1.duration_since(cpu0).as_nanos() as u64,
            cpu2.duration_since(cpu1).as_nanos() as u64,
        );
        if gfx.extent.width != size.width || gfx.extent.height != size.height {
            unsafe { gfx.recreate(&window) };
            bench_seen = 0;
            bench_presents = 0;
            stages = StageStats::default();
            continue;
        }
        match unsafe {
            gfx.draw(
                &render_pose,
                &controls,
                &view_proj,
                origin,
                eye_rel,
                simulation_time,
                &fx,
                sim_stepped,
                &mut stages,
                &cam_frame,
                wind.strength(),
            )
        }
 {
            DrawResult::Rebuild => {
                unsafe { gfx.recreate(&window) };
                bench_seen = 0;
                bench_presents = 0;
                stages = StageStats::default();
                continue;
            }
            DrawResult::Skipped => {
                stat_skipped += 1;
            }
            DrawResult::Presented(rendered) => {
                stat_frames += rendered;
                stat_presents += 1;
                presented_total += 1;
                if let Some(path) = shot_path.as_ref() {
                    if shot_frames.contains(&presented_total) {
                        let numbered = match path.rfind('.') {
                            Some(dot) => {
                                format!("{}-{}{}", &path[..dot], presented_total, &path[dot..])
                            }
                            None => format!("{}-{}", path, presented_total),
                        };
                        unsafe { gfx.screenshot(&numbered) };
                    }
                }
                if let Some(target) = bench_target {
                    if boot.elapsed().as_secs_f64() < 2.0 {
                        continue;
                    }
                    bench_seen += rendered as u64;
                    bench_presents += 1;
                    const BENCH_WARMUP: u64 = 500;
                    if bench_seen >= BENCH_WARMUP && bench_seen - (rendered as u64) < BENCH_WARMUP {
                        stages = StageStats::default();
                        stat_frames = 0;
                        bench_presents = 0;
                        bench_start = Instant::now();
                    }
                    if bench_seen >= BENCH_WARMUP + target {
                        let (acq, wait_fence, sub, pre, sim_ns, cam_ns, fx_ns, gpu_us) =
                            stages.report();
                        let seconds = bench_start.elapsed().as_secs_f64();
                        let wall_us = seconds * 1_000_000.0 / stat_frames.max(1) as f64;
                        let theoretical_fps = 1_000_000.0 / wall_us.max(1.0);
                        let real_fps = bench_presents as f64 / seconds;
                        let gp = stages.gpu_pass_avg();
                        println!(
                            "benchmark: theoretical fps: {theoretical_fps:.1} FPS ({} frames, {wall_us:.1} us/frame) | real fps: {real_fps:.1} FPS ({} presents) | acquire {acq} us fence {wait_fence} us submit {sub} us present {pre} us | sim+camera {:.1} us fx {:.1} us gpu {gpu_us} us [opq {} sky {} cld {} plu {} trl {} gls {} cmp {}]",
                            stat_frames,
                            bench_presents,
                            sim_ns as f64 / 1000.0 + cam_ns as f64 / 1000.0,
                            fx_ns as f64 / 1000.0,
                            gp[0],
                            gp[1],
                            gp[2],
                            gp[3],
                            gp[4],
                            gp[5],
                            gp[6],
                        );
                        break;
                    }
                }
            }
        }
        stat_timer += dt;
        if stat_timer >= 1.0 && bench_target.is_none() {
            let render_pass_rate = stat_frames as f32 / stat_timer;
            let present_rate = stat_presents as f32 / stat_timer;
            stat_timer = 0.0;
            stat_frames = 0;
            stat_presents = 0;
            let skipped = stat_skipped;
            stat_skipped = 0;
            let (acq, wait_fence, sub, pre, sim_ns, cam_ns, fx_ns, gpu_us) = stages.report();
            stages = StageStats::default();
            let stats = vendor.sample();
            println!(
                "theoretical fps: {:.1} FPS ({:.1} us/frame) | real fps: {:.1} FPS | acq {acq} fence {wait_fence} sub {sub} pre {pre} us | sim {sim_ns} cam {cam_ns} fx {fx_ns} ns gpu {gpu_us} us | skipped {} | speed {:.0} kt {} | GPU {}C {}MHz | fx noz {} tip {} plume {:.1}m M{:.2} lam{:.2}",
                render_pass_rate,
                1_000_000.0 / render_pass_rate.max(1.0),
                present_rate,
                skipped,
                pose.speed * 1.944,
                if pose.boost > 0.5 { "BOOST" } else { "glide" },
                stats.temp_c,
                stats.clock_mhz,
                fx.live_count(effects::EMITTER_NOZZLE),
                fx.live_count(effects::EMITTER_TIP_L) + fx.live_count(effects::EMITTER_TIP_R),
                fx.plume.length_m,
                fx.plume.mj,
                fx.plume.cell_lambda,
            );
            window.set_title(&format!(
                "explora | theoretical {:.0} FPS | real {:.0} FPS | {:.0} kt {} | GPU {}C {}MHz",
                render_pass_rate,
                present_rate,
                pose.speed * 1.944,
                if pose.boost > 0.5 { "BOOST" } else { "glide" },
                stats.temp_c,
                stats.clock_mhz,
            ));
        }
    }
    // Ensure all in-flight GPU operations have completed before destroying resources.
    unsafe {
        gfx.device.device_wait_idle().ok();
    }
    let _ = proxy.send_event(UserEvent::RenderDone);
}

/// Application entry point: initializes the winit event loop and runs the application.
fn main() {
    // Low-level hardware & driver tuning for fixed RTX 4060 target:
    // - Uncap driver vblank lock to enable true uncapped presentation (>1,400 FPS)
    // - Set yield policy to NOTHING: replaces thread sleep/yield with busy spinlock for zero-latency WSI dispatch
    // - Enable multi-threaded driver command optimizations
    if std::env::var_os("__GL_SYNC_TO_VBLANK").is_none() {
        std::env::set_var("__GL_SYNC_TO_VBLANK", "0");
    }
    if std::env::var_os("__GL_YIELD").is_none() {
        std::env::set_var("__GL_YIELD", "NOTHING");
    }
    if std::env::var_os("__GL_THREADED_OPTIMIZATIONS").is_none() {
        std::env::set_var("__GL_THREADED_OPTIMIZATIONS", "1");
    }
    if std::env::var_os("__GL_GSYNC_ALLOWED").is_none() {
        std::env::set_var("__GL_GSYNC_ALLOWED", "1");
    }

    // Install SIGINT and SIGTERM handlers to cleanly terminate via terminal Ctrl+C
    unsafe {
        libc::signal(
            libc::SIGINT,
            handle_signal as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGTERM,
            handle_signal as *const () as libc::sighandler_t,
        );
    }
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .expect("event loop");
    let proxy = event_loop.create_proxy();
    let mut app = App {
        window: None,
        shared: std::sync::Arc::new(Shared {
            exit: AtomicBool::new(false),
            keys: AtomicU32::new(0),
            ui: AtomicU32::new(hud::DEFAULT),
        }),
        proxy,
        render_thread: None,
        ctrl_pressed: false,
    };
    event_loop.run_app(&mut app).expect("run");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plane_spv_blobs() {
        // Offline modules from build.rs must load with valid SPIR-V magic.
        assert!(!spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/plane.vert.spv"))).is_empty());
        for samples in ["4", "8", "16", "32", "128"] {
            let path = format!("plane-{samples}.frag.spv");
            let full = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join(&path);
            let bytes = std::fs::read(&full).expect("plane frag spv missing");
            assert!(!spv_words(&bytes).is_empty(), "{path} empty");
        }
        assert!(!spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/sky.vert.spv"))).is_empty());
        assert!(!spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/sky.frag.spv"))).is_empty());
        assert!(!spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/clouds.frag.spv"))).is_empty());
        assert!(!spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/ground.vert.spv"))).is_empty());
        assert!(!spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/ground.frag.spv"))).is_empty());
        assert!(!spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/depth.frag.spv"))).is_empty());
    }

    #[test]
    fn test_fx_spv_blobs() {
        for name in [
            "plume.vert.spv",
            "plume.frag.spv",
            "trail.vert.spv",
            "trail.frag.spv",
            "composite.vert.spv",
            "composite.frag.spv",
        ] {
            let full = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join(name);
            let bytes = std::fs::read(&full).expect("fx spv missing");
            assert!(!spv_words(&bytes).is_empty(), "{name} empty");
        }
    }

    #[test]
    fn test_shared_exit_and_shortcuts() {
        let shared = Shared {
            exit: AtomicBool::new(false),
            keys: AtomicU32::new(0),
            ui: AtomicU32::new(hud::DEFAULT),
        };
        assert!(!shared.should_exit());
        shared.request_exit();
        assert!(shared.should_exit());
    }
}
