//! Vulkan device/swapchain/presentation state: frame resources, the
//! submit loop, screenshots, resize handling, and latency telemetry.

use ash::{vk, Entry};
use glam::Mat4;
use crate::plane;
use crate::plane::Plane;
use crate::quality::Quality;
use sim::flight::{Controls, Pose};
use sim::effects::Effects;
use std::ffi::CStr;
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::Window;
use crate::frame_budget;

const NVIDIA_VENDOR: u32 = 0x10DE;
/// Attachment resources at or above this size get a dedicated allocation so
/// the per-frame HDR/depth targets sit outside the suballocation heap.
pub(crate) const DEDICATE_ABOVE: u64 = 16 * 1024 * 1024;
const RENDER_BURST_DEFAULT: u32 = 1;
const RENDER_SAMPLES: vk::SampleCountFlags = vk::SampleCountFlags::TYPE_1;
pub(crate) const SHADER_MARKER: &str = include_str!("../shaders/plane.frag");

pub(crate) fn scaled_scene_extent(extent: vk::Extent2D, quality: Quality) -> vk::Extent2D {
    let [width, height] = quality.scene_size(extent.width, extent.height);
    vk::Extent2D {
        width,
        height,
    }
}

/// Render once per present for maximum presentation cadence. Extra passes only
/// exercise geometry without writing attachments and reduce real FPS.
/// Overridable for throughput experiments with EXPLORA_BURST (clamped 1..=256).
pub(crate) fn render_burst() -> u32 {
    std::env::var("EXPLORA_BURST")
        .ok()
        .and_then(|v| v.parse().ok())
        .map_or(RENDER_BURST_DEFAULT, |v: u32| v.clamp(1, 256))
}

/// Fine-grained microsecond timing breakdown across CPU stages, GPU timestamps, and presentation.
#[derive(Default)]
pub(crate) struct StageStats {
    acquire_us: u64,
    fence_wait_us: u64,
    submit_us: u64,
    present_us: u64,
    sim_us: u64,
    camera_us: u64,
    fx_us: u64,
    gpu_us: u64,
    gpu_pass_us: [u64; 10],
    gpu_samples: u64,
    presents: u64,
}

impl StageStats {
    pub(crate) fn add_batch(&mut self, acquire: u64, fence_wait: u64, submit: u64, present: u64) {
        self.acquire_us += acquire;
        self.fence_wait_us += fence_wait;
        self.submit_us += submit;
        self.present_us += present;
        self.presents += 1;
    }

    pub(crate) fn add_cpu(&mut self, sim: u64, camera: u64) {
        self.sim_us += sim;
        self.camera_us += camera;
    }

    pub(crate) fn add_fx(&mut self, fx: u64) {
        self.fx_us += fx;
    }

    pub(crate) fn add_gpu(&mut self, gpu_us: u64) {
        self.gpu_us += gpu_us;
        self.gpu_samples += 1;
    }

    pub(crate) fn add_gpu_pass(&mut self, pass: usize, us: u64) {
        if pass < 10 {
            self.gpu_pass_us[pass] += us;
        }
    }

    pub(crate) fn gpu_pass_avg(&self) -> [u64; 10] {
        let n = self.gpu_samples.max(1);
        let mut out = [0u64; 10];
        for (i, v) in self.gpu_pass_us.iter().enumerate() {
            out[i] = v / n;
        }
        out
    }

    pub(crate) fn report(&self) -> (u64, u64, u64, u64, u64, u64, u64, u64) {
        let n = self.presents.max(1);
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

pub(crate) fn pick_present(modes: &[vk::PresentModeKHR]) -> vk::PresentModeKHR {
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

/// Gameplay presentation paces to the display refresh (FIFO): rendering
/// faster than the refresh with mailbox or immediate makes the completed-
/// frame age at each scanout drift on a beat cycle (a 6.3 ms GPU frame
/// against an 8.3 ms vblank sweeps its completion phase across every
/// vblank), which reads as rubber-banding, most visibly along the long
/// high-contrast horizon edge. FIFO blocks each present at the vblank so
/// every displayed frame has a consistent, minimal age. Benchmarks keep the
/// uncapped preference because they measure submission capability, not
/// display pacing; an explicit EXPLORA_PRESENT overrides either path.
pub(crate) fn pick_present_mode(modes: &[vk::PresentModeKHR], pace_to_display: bool) -> vk::PresentModeKHR {
    if pace_to_display && modes.contains(&vk::PresentModeKHR::FIFO) {
        vk::PresentModeKHR::FIFO
    } else {
        pick_present(modes)
    }
}

pub(crate) fn enable_present_feedback<'a>(
    device_info: vk::DeviceCreateInfo<'a>,
    present_id: &'a mut vk::PhysicalDevicePresentIdFeaturesKHR<'a>,
    present_wait: &'a mut vk::PhysicalDevicePresentWaitFeaturesKHR<'a>,
) -> vk::DeviceCreateInfo<'a> {
    // Capability queries leave pNext links in place. Rebuild requested features
    // before insertion so ash does not splice the old query chain into itself.
    *present_id = vk::PhysicalDevicePresentIdFeaturesKHR::default().present_id(true);
    *present_wait = vk::PhysicalDevicePresentWaitFeaturesKHR::default().present_wait(true);
    device_info.push_next(present_id).push_next(present_wait)
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
pub(crate) struct Frame {
    pool: vk::CommandPool,
    cmds: Vec<vk::CommandBuffer>,
    frame_done: vk::Semaphore,
    fence: vk::Fence,
}

/// Per-swapchain-image depth stencil and optional MSAA color resolve attachments.
pub(crate) struct RenderTargets {
    depth_image: vk::Image,
    depth_memory: vk::DeviceMemory,
    depth_view: vk::ImageView,
    msaa_image: vk::Image,
    msaa_memory: vk::DeviceMemory,
    msaa_view: vk::ImageView,
}

/// Main Vulkan graphics device context, swapchain manager, and renderer state.
pub(crate) struct Gfx {
    pub(crate) _entry: Entry,
    pub(crate) instance: ash::Instance,
    pub(crate) physical_device: vk::PhysicalDevice,
    pub(crate) surface_loader: ash::khr::surface::Instance,
    pub(crate) surface: vk::SurfaceKHR,
    pub(crate) device: ash::Device,
    pub(crate) queue: vk::Queue,
    pub(crate) present_queue: vk::Queue,
    pub(crate) queue_family: u32,
    pub(crate) burst: u32,
    pub(crate) swap_loader: ash::khr::swapchain::Device,
    pub(crate) swapchain: vk::SwapchainKHR,
    pub(crate) images: Vec<vk::Image>,
    pub(crate) views: Vec<vk::ImageView>,
    pub(crate) format: vk::Format,
    pub(crate) present_mode: vk::PresentModeKHR,
    pub(crate) extent: vk::Extent2D,
    pub(crate) scene_extent: vk::Extent2D,
    pub(crate) quality: Quality,
    pub(crate) frames: Vec<Frame>,
    pub(crate) submitted: Vec<bool>,
    pub(crate) submitted_ids: Vec<u64>,
    pub(crate) acquire_semaphores: Vec<vk::Semaphore>,
    pub(crate) acquire_fences: Vec<vk::Fence>,
    pub(crate) acquire_index: usize,
    pub(crate) present_id: u64,
    /// VK_KHR_present_wait loader; Some when the device supports display-paced frames.
    pub(crate) present_wait: Option<ash::khr::present_wait::Device>,
    pub(crate) display_timing: Option<ash::google::display_timing::Device>,
    pub(crate) benchmark: Option<frame_budget::Capture>,
    pub(crate) benchmark_metadata: String,
    pub(crate) gpu_readback_every: u64,
    pub(crate) plane: Plane,
    pub(crate) targets: Vec<RenderTargets>,
    // Linear HDR scene targets (one per swapchain image) plus composite sets
    // sampling them for the final ACES pass to the swapchain.
    pub(crate) hdr_images: Vec<vk::Image>,
    pub(crate) hdr_memories: Vec<vk::DeviceMemory>,
    pub(crate) hdr_views: Vec<vk::ImageView>,
    pub(crate) comp_pool: vk::DescriptorPool,
    pub(crate) comp_sets: Vec<vk::DescriptorSet>,
    pub(crate) comp_sampler: vk::Sampler,
    pub(crate) last_presented: u32,
    pub(crate) timestamp_period_ns: f32,
    pub(crate) timestamp_valid_bits: u32,
}

impl Gfx {
    /// Initialize Vulkan instance, select target GPU, create swapchain, and allocate frame rendering resources.
    ///
    /// # Safety
    /// Must only be called once during engine initialization with a valid native window handle.
    pub(crate) unsafe fn new(window: &Window) -> Self {
        let quality = Quality::from_env();
        let benchmark = std::env::args()
            .position(|a| a == "--benchmark")
            .map(|i| std::env::args().nth(i + 1)
                .expect("--benchmark requires a positive present count")
                .parse::<u64>().expect("invalid benchmark present count"))
            .map(frame_budget::Capture::new);
        // An empty submission never transitions a new swapchain image to its
        // presentation layout, and cannot measure a rendered frame's budget.
        assert!(std::env::var_os("EXPLORA_NO_GPU").is_none(),
            "EXPLORA_NO_GPU is unsupported: an empty submit cannot present a rendered frame");
        let gpu_readback_every = if benchmark.is_some() { 1 } else {
            std::env::var("EXPLORA_GPU_READBACK_EVERY")
                .map(|v| v.parse::<u64>().expect("invalid GPU readback interval"))
                .unwrap_or(16)
        };
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
        let queue_count = queue_props[queue_family as usize].queue_count.min(2);
        let timestamp_valid_bits = queue_props[queue_family as usize].timestamp_valid_bits;
        assert!(timestamp_valid_bits > 0, "graphics queue does not support required timestamp queries");
        let priorities = [1.0f32, 1.0f32];
        let queue_info = vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family)
            .queue_priorities(&priorities[..queue_count as usize]);
        println!("queues: family {queue_family}, {queue_count} requested / {} available, timestamp bits {timestamp_valid_bits}",
            queue_props[queue_family as usize].queue_count);
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
        let deferred_ext = dev_ext_names.iter().any(|n| n == ash::khr::deferred_host_operations::NAME.to_str().unwrap());
        let rt_available = accel_ext
            && rayq_ext
            && deferred_ext
            && accel_features.acceleration_structure == vk::TRUE
            && ray_query_features.ray_query == vk::TRUE
            && bda_features.buffer_device_address == vk::TRUE;
        let rt_supported = match std::env::var("EXPLORA_RT_SHADOWS").as_deref() {
            Ok("off") => false,
            Ok("on") => { assert!(rt_available, "ray-query shadows are unavailable"); true },
            Ok("auto") | Err(_) => rt_available,
            _ => panic!("EXPLORA_RT_SHADOWS must be auto, on, or off"),
        };
        println!(
            "ray-traced shadows: {}",
            if rt_supported { "on" } else { "off (analytic fallback)" }
        );
        // Diagnostic only: NVIDIA WSI requests wp_presentation feedback for
        // present IDs. WAYLAND_DEBUG=1 then exposes actual display/zero-copy flags.
        let has_extension = |name: &CStr| dev_ext_names.iter().any(|n| n == name.to_str().unwrap());
        let feedback_requested = std::env::var_os("EXPLORA_PRESENT_FEEDBACK").is_some();
        let mut present_id_features = vk::PhysicalDevicePresentIdFeaturesKHR::default();
        let mut present_wait_features = vk::PhysicalDevicePresentWaitFeaturesKHR::default();
        let mut feedback_features = vk::PhysicalDeviceFeatures2::default()
            .push_next(&mut present_id_features).push_next(&mut present_wait_features);
        instance.get_physical_device_features2(physical, &mut feedback_features);
        let presentation_feedback = feedback_requested
            && has_extension(ash::khr::present_id::NAME)
            && has_extension(ash::khr::present_wait::NAME)
            && present_id_features.present_id == vk::TRUE
            && present_wait_features.present_wait == vk::TRUE;
        if feedback_requested && !presentation_feedback {
            eprintln!("present ID/wait feedback unavailable; continuing without it");
        }
        // Display pacing needs present IDs and present_wait regardless of the
        // diagnostic feedback flag: presenting without waiting for scanout lets
        // the render thread sample the simulation at irregular times, and the
        // compositor then displays those frames on its own regular cadence —
        // a periodic freeze-and-lurch the eye reads as rubberbanding.
        let present_wait_supported = has_extension(ash::khr::present_id::NAME)
            && has_extension(ash::khr::present_wait::NAME)
            && present_id_features.present_id == vk::TRUE
            && present_wait_features.present_wait == vk::TRUE;
        let display_timing_enabled = benchmark.is_some() && has_extension(ash::google::display_timing::NAME);
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
        if present_wait_supported {
            device_exts.extend([
                ash::khr::present_id::NAME.as_ptr(),
                ash::khr::present_wait::NAME.as_ptr(),
            ]);
        }
        if display_timing_enabled {
            device_exts.push(ash::google::display_timing::NAME.as_ptr());
        }
        let mut dyn_feat = vk::PhysicalDeviceVulkan13Features::default()
            .dynamic_rendering(true)
            .shader_demote_to_helper_invocation(true);
        let mut device_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(std::slice::from_ref(&queue_info))
            .enabled_extension_names(&device_exts)
            .push_next(&mut dyn_feat);
        let core_features = vk::PhysicalDeviceFeatures::default().multi_draw_indirect(
            instance.get_physical_device_features(physical).multi_draw_indirect != 0,
        );
        device_info = device_info.enabled_features(&core_features);
        if present_wait_supported {
            device_info = enable_present_feedback(
                device_info, &mut present_id_features, &mut present_wait_features,
            );
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
        let ground_rate = quality.settings().ground;
        println!(
            "ground shading rate: {}",
            if ground_fsr && ground_rate != [1, 1] {
                format!("{}x{}", ground_rate[0], ground_rate[1])
            } else {
                "native (1x1)".to_string()
            }
        );
        let device = instance
            .create_device(physical, &device_info, None)
            .expect("device");
        let queue = device.get_device_queue(queue_family, 0);
        let present_queue = device.get_device_queue(queue_family, queue_count - 1);
        let display_timing = display_timing_enabled
            .then(|| ash::google::display_timing::Device::new(&instance, &device));
        let swap_loader = ash::khr::swapchain::Device::new(&instance, &device);
        let present_wait = present_wait_supported
            .then(|| ash::khr::present_wait::Device::new(&instance, &device));
        println!(
            "display pacing: {}",
            if present_wait.is_some() { "present_wait" } else { "unpaced (VK_KHR_present_wait unavailable)" }
        );

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
        let present = pick_present_mode(&modes, benchmark.is_none());
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
        let benchmark_metadata = format!(
            "{{\"quality\":\"{quality:?}\",\"burst\":{},\"rt_shadows\":{rt_supported},\"queue_count\":{queue_count},\"display_timing_supported\":{display_timing_enabled},\"scene_rendered\":true,\"gpu_vendor_id\":{},\"gpu_device_id\":{},\"driver_version\":{}}}",
            render_burst(), instance.get_physical_device_properties(physical).vendor_id,
            instance.get_physical_device_properties(physical).device_id,
            instance.get_physical_device_properties(physical).driver_version,
        );
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
            physical_device: physical,
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
            present_mode: present,
            extent,
            scene_extent,
            quality,
            frames: Vec::new(),
            submitted: Vec::new(),
            submitted_ids: Vec::new(),
            acquire_semaphores: Vec::new(),
            acquire_fences: Vec::new(),
            acquire_index: 0,
            present_id: 0,
            present_wait,
            display_timing,
            benchmark,
            benchmark_metadata,
            gpu_readback_every,
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
            timestamp_valid_bits,
        };
        gfx.build_swap_views();
        gfx.build_depth();
        gfx.build_frames();
        gfx
    }

    /// Create 2D color image views for each swapchain image.
    pub(crate) unsafe fn build_swap_views(&mut self) {
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

    /// Retrieve the exact physical device used to create this logical device.
    pub(crate) unsafe fn physical(&self) -> vk::PhysicalDevice {
        self.physical_device
    }

    /// Allocate dedicated device-local depth buffers and optional MSAA transient color buffers
    /// for every swapchain image.
    pub(crate) unsafe fn build_depth(&mut self) {
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
            let memory = if req.size >= DEDICATE_ABOVE {
                let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
                device.allocate_memory(&alloc.push_next(&mut dedicated), None).expect("target mem")
            } else {
                device.allocate_memory(&alloc, None).expect("target mem")
            };
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
            let hdr_memory = if req.size >= DEDICATE_ABOVE {
                let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(hdr_image);
                device.allocate_memory(&alloc.push_next(&mut dedicated), None).expect("hdr mem")
            } else {
                device.allocate_memory(&alloc, None).expect("hdr mem")
            };
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
    pub(crate) unsafe fn build_frames(&mut self) {
        let sem_info = vk::SemaphoreCreateInfo::default();
        self.acquire_semaphores = (0..self.images.len())
            .map(|_| {
                self.device
                    .create_semaphore(&sem_info, None)
                    .expect("acquire sem")
            })
            .collect();
        self.acquire_index = 0;
        self.acquire_fences = vec![vk::Fence::null(); self.acquire_semaphores.len()];
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
            self.queue_family,
            self.queue,
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
                .queue_family_index(self.queue_family);
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
            self.submitted_ids.push(0);
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
                    (index * plane::GPU_STAMPS_PER_FRAME as usize) as u32,
                    render + 1 == self.burst as usize,
                );
            }
        }
    }

    /// Tear down all swapchain-dependent GPU resources prior to resize or exit.
    pub(crate) unsafe fn destroy_swap_side(&mut self) {
        for sem in self.acquire_semaphores.drain(..) {
            self.device.destroy_semaphore(sem, None);
        }
        for frame in self.frames.drain(..) {
            self.device.destroy_semaphore(frame.frame_done, None);
            self.device.destroy_fence(frame.fence, None);
            self.device.destroy_command_pool(frame.pool, None);
        }
        self.submitted.clear();
        self.submitted_ids.clear();
        self.acquire_fences.clear();
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
    pub(crate) unsafe fn recreate(&mut self, window: &Window) {
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
        let present = pick_present_mode(&modes, self.benchmark.is_none());
        self.present_mode = present;
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
    pub(crate) unsafe fn screenshot(&mut self, path: &str) {
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
    pub(crate) unsafe fn draw(
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
        let acquire_slot = self.acquire_index;
        let acquire_fence = self.acquire_fences[acquire_slot];
        if acquire_fence != vk::Fence::null() {
            // Binary semaphore reuse follows completion of its consuming submit,
            // not the index of the image returned by the next acquire operation.
            self.device.wait_for_fences(&[acquire_fence], true, u64::MAX)
                .expect("acquire semaphore reuse fence");
        }
        let t_acquire_start = std::time::Instant::now();
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
        // Forget completed uses before resetting this fence for different work.
        for fence in &mut self.acquire_fences {
            if *fence == frame.fence { *fence = vk::Fence::null(); }
        }
        self.device
            .reset_fences(&[frame.fence])
            .expect("reset fence");
        let t_fence = std::time::Instant::now();
        // Ordered command buffers reuse these timestamps; the final result
        // measures the final complete render of the batch: q0 start, q1
        // opaque, q2 terrain, q3 trees, q4 canopy, q5 clouds, q6 sky,
        // q7 plume, q8 trail, q9 glass, q10 composite end.
        if self.submitted[image_index]
            && self.gpu_readback_every != 0
            && self.present_id % self.gpu_readback_every == 0
        {
            let mut stamps = [0u64; plane::GPU_STAMPS_PER_FRAME as usize];
            let query_ok = self
                .device
                .get_query_pool_results(
                    self.plane.query_pool(),
                    (image_index * plane::GPU_STAMPS_PER_FRAME as usize) as u32,
                    &mut stamps,
                    vk::QueryResultFlags::TYPE_64,
                )
                .is_ok();
            if query_ok {
                let period = self.timestamp_period_ns as f64 / 1000.0;
                let mut prev = stamps[0];
                let mut pass_us = [0u64; 10];
                for (i, pass) in stamps.iter().skip(1).enumerate() {
                    let ticks = frame_budget::timestamp_delta(prev, *pass, self.timestamp_valid_bits);
                    pass_us[i] = (ticks as f64 * period) as u64;
                    stats.add_gpu_pass(i, pass_us[i]);
                    prev = *pass;
                }
                let ticks = frame_budget::timestamp_delta(stamps[0], stamps[10], self.timestamp_valid_bits);
                let gpu_ns = (ticks as f64 * self.timestamp_period_ns as f64) as u64;
                stats.add_gpu(gpu_ns / 1000);
                // Hitch diagnostics: EXPLORA_GPU_SPIKE_US (default 9000) logs the
                // per-pass split of any sampled frame whose GPU time exceeds it,
                // so a sporadic hitch can be attributed instead of averaged away.
                let spike_threshold_us: u64 = std::env::var("EXPLORA_GPU_SPIKE_US")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(9000);
                if gpu_ns / 1000 > spike_threshold_us {
                    println!(
                        "gpu spike: {} us [opq+rt {} ter {} trees {} canopy {} cld {} sky {} plu {} trl {} gls {} cmp {}] present {}",
                        gpu_ns / 1000,
                        pass_us[0], pass_us[1], pass_us[2], pass_us[3],
                        pass_us[4], pass_us[5], pass_us[6], pass_us[7], pass_us[8], pass_us[9],
                        self.present_id,
                    );
                }
                if let Some(capture) = self.benchmark.as_mut() {
                    capture.gpu_sample(self.submitted_ids[image_index], gpu_ns);
                }
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
        let submit = vk::SubmitInfo::default()
            .wait_semaphores(&wait_sems)
            .wait_dst_stage_mask(&wait_stages)
            .command_buffers(&frame.cmds)
            .signal_semaphores(&signal);
        let t1 = std::time::Instant::now();
        self.device
            .queue_submit(self.queue, &[submit], frame.fence)
            .expect("submit");
        self.submitted[image_index] = true;
        self.acquire_fences[acquire_slot] = frame.fence;
        let t2 = std::time::Instant::now();
        let swapchains = [self.swapchain];
        let indices = [image_index as u32];
        self.present_id += 1;
        self.submitted_ids[image_index] = self.present_id;
        let ids = [self.present_id];
        let mut id_info = vk::PresentIdKHR::default().present_ids(&ids);
        let mut present_info = vk::PresentInfoKHR::default()
            .wait_semaphores(&signal)
            .swapchains(&swapchains)
            .image_indices(&indices);
        if self.present_wait.is_some() {
            present_info = present_info.push_next(&mut id_info);
        }
        let display_times = [vk::PresentTimeGOOGLE::default()
            .present_id(self.present_id as u32).desired_present_time(0)];
        let mut display_info = vk::PresentTimesInfoGOOGLE::default().times(&display_times);
        if self.display_timing.is_some() {
            present_info = present_info.push_next(&mut display_info);
        }
        match self.swap_loader.queue_present(self.present_queue, &present_info) {
            Ok(suboptimal) => {
                let t3 = std::time::Instant::now();
                stats.add_batch(
                    t_acq.duration_since(t_acquire_start).as_micros() as u64,
                    (t_fence.duration_since(t_acq) + t_acquire_start.duration_since(t0)).as_micros() as u64,
                    t2.duration_since(t1).as_micros() as u64,
                    t3.duration_since(t2).as_micros() as u64,
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

    /// Block until the most recently submitted frame reaches the display.
    ///
    /// Pacing the loop on scanout keeps the simulation sampled once per
    /// refresh; without it the render thread presents as fast as the GPU
    /// allows and the compositor displays those irregular frames on its own
    /// regular grid, which the eye reads as a periodic freeze-and-lurch.
    /// Returns false when unsupported, timed out, or errored so the caller
    /// can fall back to unpaced rendering.
    pub(crate) unsafe fn wait_displayed(&mut self, timeout_ns: u64) -> bool {
        let Some(wait) = self.present_wait.as_ref() else {
            return false;
        };
        // Throttle two frames behind: waiting for the frame just submitted
        // serializes every frame against its own scanout (66 FPS measured),
        // while two frames of slack keeps the render pipeline overlapped with
        // scanout and locks the cadence to the refresh (118-119 FPS on the
        // 119.96 Hz panel). One frame behind measured 106-108 FPS.
        if self.present_id <= 2 {
            return true;
        }
        matches!(
            wait.wait_for_present(self.swapchain, self.present_id - 2, timeout_ns),
            Ok(())
        )
    }

    pub(crate) unsafe fn collect_display_timings(&mut self) {
        if let (Some(timing), Some(capture)) = (&self.display_timing, &mut self.benchmark) {
            match timing.get_past_presentation_timing(self.swapchain) {
                Ok(timings) => {
                    for sample in timings {
                        capture.displayed(sample.present_id as u64, sample.actual_present_time);
                    }
                }
                Err(error) => eprintln!("display timing unavailable for this poll: {error:?}"),
            }
        }
    }

    /// Drain only after the measured interval ends; this wait is not hidden in
    /// the measured cadence. Every image's last submit has not yet been read.
    pub(crate) unsafe fn finish_gpu_capture(&mut self) {
        self.device.device_wait_idle().expect("benchmark drain");
        if let Some(capture) = self.benchmark.as_mut() {
            for (index, &id) in self.submitted_ids.iter().enumerate() {
                if !self.submitted[index] { continue; }
                let mut stamps = [0u64; plane::GPU_STAMPS_PER_FRAME as usize];
                if self.device.get_query_pool_results(self.plane.query_pool(),
                    (index * plane::GPU_STAMPS_PER_FRAME as usize) as u32, &mut stamps, vk::QueryResultFlags::TYPE_64).is_ok() {
                    let ticks = frame_budget::timestamp_delta(stamps[0], stamps[10], self.timestamp_valid_bits);
                    capture.gpu_sample(id, (ticks as f64 * self.timestamp_period_ns as f64) as u64);
                }
            }
        }
    }
}

/// Result of a frame draw call.
#[derive(PartialEq, Eq)]
pub(crate) enum DrawResult {
    /// Number of recorded passes submitted with a successful present request.
    Presented(u32),
    /// Frame acquisition was skipped due to timeout or non-ready status.
    Skipped,
    /// Swapchain is suboptimal or out of date and must be recreated.
    Rebuild,
}
