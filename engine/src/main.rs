// Native boot: window, Vulkan device, swapchain, fixed loop.
// One canvas. One GPU. Fixed passes. No fallback.

mod camera;
mod flight;
mod mesh;
mod vendor;

use ash::{vk, Entry};
use flight::{Controls, Pose};
use std::collections::HashSet;
use std::ffi::CStr;
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::{Window, WindowId};

const NVIDIA_VENDOR: u32 = 0x10DE;
const SIM_STEP: f32 = 1.0 / 90.0;

#[derive(Default)]
struct StageStats {
    acquire_us: u64,
    submit_us: u64,
    present_us: u64,
    frames: u64,
}

impl StageStats {
    fn add(&mut self, acquire: u64, submit: u64, present: u64) {
        self.acquire_us += acquire;
        self.submit_us += submit;
        self.present_us += present;
        self.frames += 1;
    }

    fn report(&self) -> (u64, u64, u64) {
        let n = self.frames.max(1);
        (self.acquire_us / n, self.submit_us / n, self.present_us / n)
    }
}

const MESH_WGSL: &str = include_str!("mesh.wgsl");

fn wgsl_to_spirv(src: &str) -> Vec<u32> {
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

struct Uniform {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    mapped: *mut u8,
    set: vk::DescriptorSet,
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
    layout: vk::PipelineLayout,
    set_layout: vk::DescriptorSetLayout,
    descriptor_pool: vk::DescriptorPool,
    pipeline: vk::Pipeline,
    frames: Vec<Frame>,
    uniforms: Vec<Uniform>,
    vertex_buffer: vk::Buffer,
    vertex_memory: vk::DeviceMemory,
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
        println!("present mode: {present:?}");
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
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(caps.current_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(present)
            .clipped(true);
        let swapchain = swap_loader.create_swapchain(&swap_info, None).expect("swapchain");

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
            layout: vk::PipelineLayout::null(),
            set_layout: vk::DescriptorSetLayout::null(),
            descriptor_pool: vk::DescriptorPool::null(),
            pipeline: vk::Pipeline::null(),
            frames: Vec::new(),
            uniforms: Vec::new(),
            vertex_buffer: vk::Buffer::null(),
            vertex_memory: vk::DeviceMemory::null(),
        };
        gfx.build_swap_views();
        gfx.build_set_layout();
        gfx.build_pipeline();
        gfx.build_uniforms();
        gfx.build_mesh();
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

    unsafe fn build_pipeline(&mut self) {
        let words = wgsl_to_spirv(MESH_WGSL);
        let module_info = vk::ShaderModuleCreateInfo::default().code(&words);
        let module = self.device.create_shader_module(&module_info, None).expect("module");
        let vs_entry = c"vs_main";
        let fs_entry = c"fs_main";
        let stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(module)
                .name(vs_entry),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(module)
                .name(fs_entry),
        ];
        let binding = [vk::VertexInputBindingDescription::default()
            .binding(0)
            .stride(24)
            .input_rate(vk::VertexInputRate::VERTEX)];
        let attributes = [
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(0)
                .format(vk::Format::R32G32B32_SFLOAT)
                .offset(0),
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(1)
                .format(vk::Format::R32G32B32_SFLOAT)
                .offset(12),
        ];
        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&binding)
            .vertex_attribute_descriptions(&attributes);
        let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
        let viewport_state = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let raster = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .line_width(1.0);
        let multisample = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let blend_attach = [vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(false)
            .color_write_mask(vk::ColorComponentFlags::RGBA)];
        let blend = vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend_attach);
        let dynamic = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic_state =
            vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic);
        let set_layouts = [self.set_layout];
        let layout_info = vk::PipelineLayoutCreateInfo::default().set_layouts(&set_layouts);
        self.layout = self.device.create_pipeline_layout(&layout_info, None).expect("layout");
        let formats = [self.format];
        let mut rendering = vk::PipelineRenderingCreateInfo::default()
            .color_attachment_formats(&formats);
        let pipeline_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&raster)
            .multisample_state(&multisample)
            .color_blend_state(&blend)
            .dynamic_state(&dynamic_state)
            .layout(self.layout)
            .push_next(&mut rendering);
        let pipelines = self
            .device
            .create_graphics_pipelines(vk::PipelineCache::null(), &[pipeline_info], None)
            .expect("pipeline");
        self.pipeline = pipelines[0];
        self.device.destroy_shader_module(module, None);
    }

    unsafe fn build_set_layout(&mut self) {
        let binding = [vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::VERTEX)];
        let info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&binding);
        self.set_layout = self.device.create_descriptor_set_layout(&info, None).expect("dsl");
    }

    unsafe fn build_uniforms(&mut self) {
        let count = self.images.len().max(1) as u32;
        let pool_size = [vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(count)];
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .pool_sizes(&pool_size)
            .max_sets(count);
        self.descriptor_pool =
            self.device.create_descriptor_pool(&pool_info, None).expect("dpool");
        let count_usize = count as usize;
        let layouts = vec![self.set_layout; count_usize];
        let alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(self.descriptor_pool)
            .set_layouts(&layouts);
        let sets = self.device.allocate_descriptor_sets(&alloc_info).expect("dsets");
        let mem_props =
            self.instance.get_physical_device_memory_properties(self.physical());
        for set in sets {
            let buffer_info = vk::BufferCreateInfo::default()
                .size(64)
                .usage(vk::BufferUsageFlags::UNIFORM_BUFFER)
                .sharing_mode(vk::SharingMode::EXCLUSIVE);
            let buffer = self.device.create_buffer(&buffer_info, None).expect("ubo");
            let req = self.device.get_buffer_memory_requirements(buffer);
            let index = find_memory_type(
                &mem_props,
                req.memory_type_bits,
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            );
            let alloc = vk::MemoryAllocateInfo::default()
                .allocation_size(req.size)
                .memory_type_index(index);
            let memory = self.device.allocate_memory(&alloc, None).expect("umem");
            self.device.bind_buffer_memory(buffer, memory, 0).expect("ubind");
            let mapped = self
                .device
                .map_memory(memory, 0, 64, vk::MemoryMapFlags::empty())
                .expect("umap") as *mut u8;
            let buffer_ref = [vk::DescriptorBufferInfo::default()
                .buffer(buffer)
                .offset(0)
                .range(64)];
            let write = [vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .buffer_info(&buffer_ref)];
            self.device.update_descriptor_sets(&write, &[]);
            self.uniforms.push(Uniform { buffer, memory, mapped, set });
        }
    }

    unsafe fn build_mesh(&mut self) {
        let bytes: &[u8] = unsafe {
            std::slice::from_raw_parts(
                mesh::GLIDER.as_ptr() as *const u8,
                std::mem::size_of_val(&mesh::GLIDER),
            )
        };
        let buffer_info = vk::BufferCreateInfo::default()
            .size(bytes.len() as u64)
            .usage(vk::BufferUsageFlags::VERTEX_BUFFER)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        self.vertex_buffer = self.device.create_buffer(&buffer_info, None).expect("vbo");
        let req = self.device.get_buffer_memory_requirements(self.vertex_buffer);
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
        self.vertex_memory = self.device.allocate_memory(&alloc, None).expect("vmem");
        self.device
            .bind_buffer_memory(self.vertex_buffer, self.vertex_memory, 0)
            .expect("bind");
        let mapped = self
            .device
            .map_memory(self.vertex_memory, 0, req.size, vk::MemoryMapFlags::empty())
            .expect("map") as *mut u8;
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped, bytes.len());
        self.device.unmap_memory(self.vertex_memory);
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

    unsafe fn build_frames(&mut self) {
        // One frame slot per swapchain image. Each records once.
        // The hot loop only waits, copies 64 bytes, submits, presents.
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
            self.record(cmd, *image, self.views[index], self.uniforms[index].set);
        }
    }

    unsafe fn record(
        &self,
        cmd: vk::CommandBuffer,
        image: vk::Image,
        view: vk::ImageView,
        set: vk::DescriptorSet,
    ) {
        let begin = vk::CommandBufferBeginInfo::default();
        self.device.begin_command_buffer(cmd, &begin).expect("begin");
        let clear = vk::ClearValue {
            color: vk::ClearColorValue { float32: [0.596, 0.796, 0.945, 1.0] },
        };
        let color_info = vk::RenderingAttachmentInfo::default()
            .image_view(view)
            .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .clear_value(clear);
        let color_infos = [color_info];
        let rendering = vk::RenderingInfo::default()
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: self.extent,
            })
            .layer_count(1)
            .color_attachments(&color_infos);
        let subresource = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .base_mip_level(0)
            .level_count(1)
            .base_array_layer(0)
            .layer_count(1);
        let barrier_to_draw = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::empty())
            .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .image(image)
            .subresource_range(subresource);
        self.device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[barrier_to_draw],
        );
        self.device.cmd_begin_rendering(cmd, &rendering);
        self.device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.pipeline);
        let viewport = vk::Viewport::default()
            .x(0.0)
            .y(0.0)
            .width(self.extent.width as f32)
            .height(self.extent.height as f32)
            .min_depth(0.0)
            .max_depth(1.0);
        self.device.cmd_set_viewport(cmd, 0, &[viewport]);
        let scissor = vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: self.extent,
        };
        self.device.cmd_set_scissor(cmd, 0, &[scissor]);
        self.device.cmd_bind_vertex_buffers(cmd, 0, &[self.vertex_buffer], &[0]);
        self.device.cmd_bind_descriptor_sets(
            cmd,
            vk::PipelineBindPoint::GRAPHICS,
            self.layout,
            0,
            &[set],
            &[],
        );
        self.device.cmd_draw(cmd, 18, 1, 0, 0);
        self.device.cmd_end_rendering(cmd);
        let barrier_to_present = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
            .dst_access_mask(vk::AccessFlags::empty())
            .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
            .image(image)
            .subresource_range(subresource);
        self.device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[barrier_to_present],
        );
        self.device.end_command_buffer(cmd).expect("end");
    }

    unsafe fn destroy_swap_side(&mut self) {
        for frame in self.frames.drain(..) {
            self.device.destroy_semaphore(frame.frame_done, None);
            self.device.destroy_fence(frame.fence, None);
            self.device.destroy_command_pool(frame.pool, None);
        }
        for uniform in self.uniforms.drain(..) {
            self.device.unmap_memory(uniform.memory);
            self.device.destroy_buffer(uniform.buffer, None);
            self.device.free_memory(uniform.memory, None);
        }
        self.device.destroy_pipeline(self.pipeline, None);
        self.device.destroy_descriptor_pool(self.descriptor_pool, None);
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
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(caps.current_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(present)
            .clipped(true);
        self.swapchain = self.swap_loader.create_swapchain(&swap_info, None).expect("swap");
        self.build_swap_views();
        // Pipeline bakes the color format, so rebuild it too.
        self.build_pipeline();
        self.build_uniforms();
        self.build_frames();
    }

    unsafe fn draw(&mut self, mvp: &[f32; 16], stats: &mut StageStats) -> DrawResult {
        // Hot loop: wait, copy 64 bytes, submit, present. Nothing else.
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
        let uniform = &self.uniforms[image_index];
        std::ptr::copy_nonoverlapping(
            mvp.as_ptr() as *const u8,
            uniform.mapped,
            64,
        );
        let buffers = [frame.cmd];
        let signal = [frame.frame_done];
        let submit = vk::SubmitInfo::default()
            .command_buffers(&buffers)
            .signal_semaphores(&signal);
        let t1 = std::time::Instant::now();
        self.device.queue_submit(self.queue, &[submit], frame.fence).expect("submit");
        let t2 = std::time::Instant::now();
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

struct App {
    window: Option<Window>,
    gfx: Option<Gfx>,
    pose: Pose,
    keys: HashSet<KeyCode>,
    accumulator: f32,
    last: Option<Instant>,
    vendor: Option<vendor::Vendor>,
    stat_timer: f32,
    stat_frames: u32,
    stat_skipped: u64,
    stages: StageStats,
}

impl App {
    fn controls(&self) -> Controls {
        let held = |code: KeyCode| self.keys.contains(&code);
        Controls {
            pitch: (if held(KeyCode::KeyS) { 1.0 } else { 0.0 })
                - (if held(KeyCode::KeyW) { 1.0 } else { 0.0 }),
            bank: (if held(KeyCode::KeyD) { 1.0 } else { 0.0 })
                - (if held(KeyCode::KeyA) { 1.0 } else { 0.0 }),
            yaw: (if held(KeyCode::KeyE) { 1.0 } else { 0.0 })
                - (if held(KeyCode::KeyQ) { 1.0 } else { 0.0 }),
            boost: held(KeyCode::ShiftLeft) || held(KeyCode::ShiftRight),
        }
    }
}

impl ApplicationHandler for App {
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
        let window = event_loop.create_window(attrs).expect("window");
        let gfx = unsafe { Gfx::new(&window) };
        self.vendor = Some(unsafe { vendor::Vendor::open() });
        self.window = Some(window);
        self.gfx = Some(gfx);
        self.last = Some(Instant::now());
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(_) => {
                if let (Some(gfx), Some(window)) = (self.gfx.as_mut(), self.window.as_ref()) {
                    unsafe { gfx.recreate(window) };
                }
            }
            WindowEvent::KeyboardInput { event: KeyEvent { physical_key, state, .. }, .. } => {
                if let PhysicalKey::Code(code) = physical_key {
                    if state.is_pressed() {
                        if code == KeyCode::F11 {
                            if let Some(window) = self.window.as_ref() {
                                let full = window.fullscreen().is_none();
                                window.set_fullscreen(full.then(|| winit::window::Fullscreen::Borderless(None)));
                            }
                        }
                        self.keys.insert(code);
                    } else {
                        self.keys.remove(&code);
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let mut dt = self.last.map(|last| (now - last).as_secs_f32()).unwrap_or(1.0 / 60.0);
                self.last = Some(now);
                if dt > 0.1 {
                    dt = 0.1;
                }
                if dt < 0.0 {
                    dt = 0.0;
                }
                self.accumulator += dt;
                let mut steps = 0;
                let controls = self.controls();
                while self.accumulator >= SIM_STEP && steps < 5 {
                    self.pose.step(&controls, SIM_STEP);
                    self.accumulator -= SIM_STEP;
                    steps += 1;
                }
                if steps == 5 {
                    self.accumulator = 0.0;
                }
                let (Some(gfx), Some(window)) = (self.gfx.as_mut(), self.window.as_ref()) else {
                    return;
                };
                let size = window.inner_size();
                if size.width == 0 || size.height == 0 {
                    return;
                }
                let aspect = size.width as f32 / size.height as f32;
                let view_proj = camera::view_proj(&self.pose, aspect);
                // Model: yaw, pitch, roll from the pose.
                let yaw = glam::Mat4::from_rotation_y(self.pose.heading);
                let pitch = glam::Mat4::from_rotation_x(-self.pose.pitch);
                let roll = glam::Mat4::from_rotation_z(-self.pose.bank);
                let model = glam::Mat4::from_translation(glam::Vec3::new(
                    self.pose.x,
                    self.pose.y,
                    self.pose.z,
                )) * yaw
                    * pitch
                    * roll;
                let mvp = (view_proj * model).to_cols_array();
                let size_now = window.inner_size();
                let caps_ok = size_now.width > 0 && size_now.height > 0;
                if caps_ok && (gfx.extent.width != size_now.width || gfx.extent.height != size_now.height)
                {
                    unsafe { gfx.recreate(window) };
                }
                match unsafe { gfx.draw(&mvp, &mut self.stages) } {
                    DrawResult::Rebuild => {
                        unsafe { gfx.recreate(window) };
                        return;
                    }
                    DrawResult::Skipped => {
                        self.stat_skipped += 1;
                    }
                    DrawResult::Presented => {
                        self.stat_frames += 1;
                    }
                }
                self.stat_timer += dt;
                if self.stat_timer >= 1.0 {
                    let fps = self.stat_frames as f32 / self.stat_timer;
                    self.stat_timer = 0.0;
                    self.stat_frames = 0;
                    let skipped = self.stat_skipped;
                    self.stat_skipped = 0;
                    let (acq, sub, pre) = self.stages.report();
                    self.stages = StageStats::default();
                    if let Some(vendor) = self.vendor.as_mut() {
                        let stats = vendor.sample();
                        println!(
                            "fps {:.1} ({:.1} us) | acq {acq} sub {sub} pre {pre} us | skipped {} | speed {:.0} kt {} | GPU {}C {}MHz",
                            fps,
                            1_000_000.0 / fps.max(1.0),
                            skipped,
                            self.pose.speed * 1.944,
                            if self.pose.boost > 0.5 { "BOOST" } else { "glide" },
                            stats.temp_c,
                            stats.clock_mhz,
                        );
                        window.set_title(&format!(
                            "explora | {:.0} fps {:.0} kt {} | GPU {}C {}MHz",
                            fps,
                            self.pose.speed * 1.944,
                            if self.pose.boost > 0.5 { "BOOST" } else { "glide" },
                            stats.temp_c,
                            stats.clock_mhz,
                        ));
                    }
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Poll);
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}

fn main() {
    vendor::pin_to_performance_cores();
    let event_loop = EventLoop::new().expect("event loop");
    let mut app = App {
        window: None,
        gfx: None,
        pose: Pose::start(),
        keys: HashSet::new(),
        accumulator: 0.0,
        last: None,
        vendor: None,
        stat_timer: 0.0,
        stat_frames: 0,
        stat_skipped: 0,
        stages: StageStats::default(),
    };
    event_loop.run_app(&mut app).expect("run");
}
