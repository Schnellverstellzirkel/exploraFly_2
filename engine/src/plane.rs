// Glider renderer. One uber-shader, two pipelines, ~50 rigid parts.
// Indexed parts share one interleaved device-local stream.
// Each part owns one uniform buffer per swapchain image holding
// its MVP, material, and flex terms. Command buffers stay
// pre-recorded. Per-frame CPU work is node matrices plus small
// coherent copies.

use super::airframe::{build_airframe, MatId, Node};
use super::airframe_util::{f32_to_f16, oct_encode};
use super::flight::Controls;
use ash::vk;
use glam::{Mat4, Vec3};

const UBO_BYTES: usize = 192;

fn mat_params(mat: MatId) -> ([f32; 4], [f32; 4], f32) {
    // (albedo_rgb + metalness, emissive_rgb + roughness, weave flag)
    match mat {
        MatId::Sail => ([0.59, 0.59, 0.55, 0.04], [0.0, 0.0, 0.0, 0.65], 1.0),
        MatId::Composite => ([0.47, 0.48, 0.48, 0.12], [0.0, 0.0, 0.0, 0.34], 0.0),
        MatId::Graphite => ([0.15, 0.17, 0.19, 0.25], [0.0, 0.0, 0.0, 0.34], 0.0),
        MatId::Titanium => ([0.55, 0.58, 0.59, 0.88], [0.0, 0.0, 0.0, 0.24], 0.0),
        MatId::Dark => ([0.09, 0.11, 0.13, 0.82], [0.0, 0.0, 0.0, 0.32], 0.0),
        MatId::Seat => ([0.40, 0.28, 0.22, 0.0], [0.0, 0.0, 0.0, 0.85], 0.0),
        MatId::Glass => ([0.38, 0.43, 0.47, 0.1], [0.0, 0.0, 0.0, 0.06], 0.0),
        MatId::Glow => ([0.80, 0.83, 1.0, 0.2], [0.52, 0.61, 1.0, 0.15], 0.0),
    }
}

fn damp(current: f32, target: f32, lambda: f32, dt: f32) -> f32 {
    current + (target - current) * (1.0 - (-lambda * dt).exp())
}

pub struct Anim {
    spool: f32,
    bend: f32,
    bend_vel: f32,
    flaps: [f32; 6],
    elevators: [f32; 2],
    rotor: f32,
    petals: [f32; 10],
}

impl Anim {
    pub fn new() -> Self {
        Self {
            spool: 0.0,
            bend: 0.0,
            bend_vel: 0.0,
            flaps: [0.0; 6],
            elevators: [0.0; 2],
            rotor: 0.0,
            petals: [-0.12; 10],
        }
    }

    pub fn step(&mut self, u: &Controls, load: f32, speed: f32, boost: f32, time: f32, dt: f32) {
        self.spool = damp(self.spool, boost, 4.0, dt);
        // Wing bend spring, same rates as the prototype.
        let target = ((load - 1.0) * 0.15).clamp(-0.4, 1.1);
        let steps = (dt * 120.0).ceil().max(1.0) as usize;
        let h = dt / steps as f32;
        for _ in 0..steps {
            self.bend_vel += (45.0 * (target - self.bend) - 10.0 * self.bend_vel) * h;
            self.bend += self.bend_vel * h;
        }
        for (j, flap) in self.flaps.iter_mut().enumerate() {
            let side = if j < 3 { -1.0 } else { 1.0 };
            let k = j % 3;
            let goal = u.bank * side * (0.22 + k as f32 * 0.04) - u.pitch * 0.07;
            *flap = damp(*flap, goal, 12.0 - k as f32 * 2.0, dt);
        }
        for (i, elev) in self.elevators.iter_mut().enumerate() {
            let rudder = if i == 1 { 1.0 } else { -1.0 };
            *elev = damp(*elev, -u.pitch * 0.23 + u.yaw * rudder * 0.16, 10.0, dt);
        }
        self.rotor += (2.5 + self.spool * 14.0) * dt;
        for petal in self.petals.iter_mut() {
            *petal = damp(*petal, -0.12 - 0.42 * self.spool, 8.0, dt);
        }
        let _ = (speed, time);
    }

    pub fn pressure(speed: f32) -> f32 {
        (speed / 100.0).min(1.0)
    }
}

struct CpuPart {
    node: Node,
    mat: MatId,
    vert_offset: u32,
    first_index: u32,
    index_count: u32,
    transparent: bool,
}

pub struct Plane {
    parts: Vec<CpuPart>,
    vertex_buffer: vk::Buffer,
    #[allow(dead_code)]
    vertex_memory: vk::DeviceMemory,
    index_buffer: vk::Buffer,
    #[allow(dead_code)]
    index_memory: vk::DeviceMemory,
    set_layout: vk::DescriptorSetLayout,
    descriptor_pool: vk::DescriptorPool,
    opaque_pipeline: vk::Pipeline,
    glass_pipeline: vk::Pipeline,
    layout: vk::PipelineLayout,
    // ubos[image][part] flattened.
    ubo_buffers: Vec<vk::Buffer>,
    ubo_memories: Vec<vk::DeviceMemory>,
    ubo_mapped: Vec<*mut u8>,
    ubo_sets: Vec<vk::DescriptorSet>,
    image_count: usize,
    pub anim: Anim,
}

fn wing_point(side: f32, t: f32, chord: f32) -> Vec3 {
    let x = 0.42 + 10.4 * t;
    let leading = -1.4 + 0.9 * t + 2.7 * t * t;
    let width = (2.35 - 1.65 * t) * (1.0 - t.powi(12) * 0.87);
    let y = 0.08 + 0.22 * t + 0.65 * t.powi(5) + (chord * std::f32::consts::PI).sin() * 0.14 * (1.0 - t);
    Vec3::new(side * (x - 1.2), y, leading + width * chord)
}

fn flap_pivot(side: f32, k: usize) -> Vec3 {
    let start = 0.425 + k as f32 * 0.155;
    let end = start + 0.15;
    wing_point(side, (start + end) / 2.0, 0.77)
}

impl Plane {
    pub unsafe fn build(
        device: &ash::Device,
        instance: &ash::Instance,
        physical: vk::PhysicalDevice,
        queue_family: u32,
        queue: vk::Queue,
        format: vk::Format,
    ) -> Self {
        let raw = build_airframe();
        // Pack one interleaved stream: pos12 + oct4 + uvHalf4 + flex4.
        let mut stream: Vec<u8> = Vec::new();
        let mut indices: Vec<u16> = Vec::new();
        let mut parts = Vec::new();
        let mut tri_total = 0u32;
        for part in &raw {
            assert!(part.verts.len() < 65536, "part too large for u16");
            // Smooth normals from the indexed triangles.
            let mut normals = vec![Vec3::ZERO; part.verts.len()];
            for tri in part.idx.chunks_exact(3) {
                let a = Vec3::from_array(part.verts[tri[0] as usize].pos);
                let b = Vec3::from_array(part.verts[tri[1] as usize].pos);
                let c = Vec3::from_array(part.verts[tri[2] as usize].pos);
                let n = (b - a).cross(c - a);
                normals[tri[0] as usize] += n;
                normals[tri[1] as usize] += n;
                normals[tri[2] as usize] += n;
            }
            let reordered = super::forsyth::reorder(&part.idx);
            let vert_offset = (stream.len() / 24) as u32;
            for (v, n) in part.verts.iter().zip(normals.iter()) {
                let n = n.normalize_or_zero();
                let oct = oct_encode(n);
                stream.extend_from_slice(&v.pos[0].to_le_bytes());
                stream.extend_from_slice(&v.pos[1].to_le_bytes());
                stream.extend_from_slice(&v.pos[2].to_le_bytes());
                stream.extend_from_slice(&oct[0].to_le_bytes());
                stream.extend_from_slice(&oct[1].to_le_bytes());
                stream.extend_from_slice(&f32_to_f16(v.uv[0]).to_le_bytes());
                stream.extend_from_slice(&f32_to_f16(v.uv[1]).to_le_bytes());
                stream.extend_from_slice(&v.flex.to_le_bytes());
            }
            let first_index = indices.len() as u32;
            for i in reordered {
                indices.push(i as u16);
            }
            tri_total += part.idx.len() as u32 / 3;
            parts.push(CpuPart {
                node: part.node,
                mat: part.mat,
                vert_offset,
                first_index,
                index_count: part.idx.len() as u32,
                transparent: part.mat == MatId::Glass,
            });
        }
        println!(
            "airframe: {} parts, {} tris, {:.1} KiB verts, {:.1} KiB indices",
            parts.len(),
            tri_total,
            stream.len() as f32 / 1024.0,
            indices.len() as f32 * 2.0 / 1024.0,
        );
        let mem_props = instance.get_physical_device_memory_properties(physical);
        let upload = |size: u64, usage: vk::BufferUsageFlags| {
            let info = vk::BufferCreateInfo::default().size(size).usage(usage).sharing_mode(vk::SharingMode::EXCLUSIVE);
            let buffer = device.create_buffer(&info, None).expect("buffer");
            let req = device.get_buffer_memory_requirements(buffer);
            let index = super::find_memory_type(
                &mem_props,
                req.memory_type_bits,
                vk::MemoryPropertyFlags::DEVICE_LOCAL,
            );
            let alloc = vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(index);
            let memory = device.allocate_memory(&alloc, None).expect("mem");
            device.bind_buffer_memory(buffer, memory, 0).expect("bind");
            (buffer, memory)
        };
        let (vertex_buffer, vertex_memory) = upload(
            stream.len() as u64,
            vk::BufferUsageFlags::VERTEX_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
        );
        let index_bytes = indices.len() * 2;
        let (index_buffer, index_memory) = upload(
            index_bytes as u64,
            vk::BufferUsageFlags::INDEX_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
        );
        // Staging upload on the graphics queue, then free it.
        let pool_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(queue_family)
            .flags(vk::CommandPoolCreateFlags::TRANSIENT);
        let pool = device.create_command_pool(&pool_info, None).expect("spool");
        let alloc = vk::CommandBufferAllocateInfo::default()
            .command_pool(pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let cmd = device.allocate_command_buffers(&alloc).expect("scmd")[0];
        let stage_info = vk::BufferCreateInfo::default()
            .size((stream.len() + index_bytes) as u64)
            .usage(vk::BufferUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let stage = device.create_buffer(&stage_info, None).expect("stage");
        let stage_req = device.get_buffer_memory_requirements(stage);
        let stage_index = super::find_memory_type(
            &mem_props,
            stage_req.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        );
        let stage_alloc = vk::MemoryAllocateInfo::default()
            .allocation_size(stage_req.size)
            .memory_type_index(stage_index);
        let stage_mem = device.allocate_memory(&stage_alloc, None).expect("smem");
        device.bind_buffer_memory(stage, stage_mem, 0).expect("sbind");
        let mapped = device
            .map_memory(stage_mem, 0, stage_req.size, vk::MemoryMapFlags::empty())
            .expect("smap") as *mut u8;
        std::ptr::copy_nonoverlapping(stream.as_ptr(), mapped, stream.len());
        std::ptr::copy_nonoverlapping(
            indices.as_ptr() as *const u8,
            mapped.add(stream.len()),
            index_bytes,
        );
        device.unmap_memory(stage_mem);
        let begin = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        device.begin_command_buffer(cmd, &begin).expect("sbegin");
        let copy_v = vk::BufferCopy::default().size(stream.len() as u64);
        device.cmd_copy_buffer(cmd, stage, vertex_buffer, &[copy_v]);
        let copy_i = vk::BufferCopy::default()
            .src_offset(stream.len() as u64)
            .size(index_bytes as u64);
        device.cmd_copy_buffer(cmd, stage, index_buffer, &[copy_i]);
        device.end_command_buffer(cmd).expect("send");
        let fence_info = vk::FenceCreateInfo::default();
        let fence = device.create_fence(&fence_info, None).expect("sfence");
        let cmd_ref = [cmd];
        let submit = vk::SubmitInfo::default().command_buffers(&cmd_ref);
        device.queue_submit(queue, &[submit], fence).expect("ssubmit");
        device.wait_for_fences(&[fence], true, u64::MAX).expect("swait");
        device.destroy_fence(fence, None);
        device.destroy_command_pool(pool, None);
        device.destroy_buffer(stage, None);
        device.free_memory(stage_mem, None);

        // Descriptor layout and the two pipelines.
        let binding = [vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)];
        let dsl_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&binding);
        let set_layout = device.create_descriptor_set_layout(&dsl_info, None).expect("pdsl");
        let layout_info =
            vk::PipelineLayoutCreateInfo::default().set_layouts(std::slice::from_ref(&set_layout));
        let layout = device.create_pipeline_layout(&layout_info, None).expect("playout");
        let words = super::wgsl_to_spirv(include_str!("plane.wgsl"));
        let module_info = vk::ShaderModuleCreateInfo::default().code(&words);
        let module = device.create_shader_module(&module_info, None).expect("pmodule");
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
        let binding_desc = [vk::VertexInputBindingDescription::default()
            .binding(0)
            .stride(24)
            .input_rate(vk::VertexInputRate::VERTEX)];
        let attrs = [
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(0)
                .format(vk::Format::R32G32B32_SFLOAT)
                .offset(0),
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(1)
                .format(vk::Format::R16G16_UINT)
                .offset(12),
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(2)
                .format(vk::Format::R16G16_SFLOAT)
                .offset(16),
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(3)
                .format(vk::Format::R32_SFLOAT)
                .offset(20),
        ];
        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&binding_desc)
            .vertex_attribute_descriptions(&attrs);
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
        let depth_state = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(true)
            .depth_write_enable(true)
            .depth_compare_op(vk::CompareOp::LESS);
        let blend_off = [vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(false)
            .color_write_mask(vk::ColorComponentFlags::RGBA)];
        let blend_on = [vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_write_mask(vk::ColorComponentFlags::RGBA)];
        let blend_off_state =
            vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend_off);
        let blend_on_state =
            vk::PipelineColorBlendStateCreateInfo::default().attachments(&blend_on);
        let dynamic = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic_state = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic);
        let formats = [format];
        let depth_format = [vk::Format::D32_SFLOAT];
        let mut rendering = vk::PipelineRenderingCreateInfo::default()
            .color_attachment_formats(&formats)
            .depth_attachment_format(depth_format[0]);
        let mut rendering_glass = vk::PipelineRenderingCreateInfo::default()
            .color_attachment_formats(&formats)
            .depth_attachment_format(depth_format[0]);
        let opaque_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&raster)
            .multisample_state(&multisample)
            .depth_stencil_state(&depth_state)
            .color_blend_state(&blend_off_state)
            .dynamic_state(&dynamic_state)
            .layout(layout)
            .push_next(&mut rendering);
        let glass_depth = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(true)
            .depth_write_enable(false)
            .depth_compare_op(vk::CompareOp::LESS);
        let glass_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&raster)
            .multisample_state(&multisample)
            .depth_stencil_state(&glass_depth)
            .color_blend_state(&blend_on_state)
            .dynamic_state(&dynamic_state)
            .layout(layout)
            .push_next(&mut rendering_glass);
        let pipelines = device
            .create_graphics_pipelines(
                vk::PipelineCache::null(),
                &[opaque_info, glass_info],
                None,
            )
            .expect("ppipes");
        device.destroy_shader_module(module, None);
        Self {
            parts: parts
                .into_iter()
                .map(|p| CpuPart {
                    node: p.node,
                    mat: p.mat,
                    vert_offset: p.vert_offset,
                    first_index: p.first_index,
                    index_count: p.index_count,
                    transparent: p.transparent,
                })
                .collect(),
            vertex_buffer,
            vertex_memory,
            index_buffer,
            index_memory,
            set_layout,
            descriptor_pool: vk::DescriptorPool::null(),
            opaque_pipeline: pipelines[0],
            glass_pipeline: pipelines[1],
            layout,
            ubo_buffers: Vec::new(),
            ubo_memories: Vec::new(),
            ubo_mapped: Vec::new(),
            ubo_sets: Vec::new(),
            image_count: 0,
            anim: Anim::new(),
        }
    }

    pub unsafe fn build_frames(&mut self, device: &ash::Device, instance: &ash::Instance, physical: vk::PhysicalDevice, images: usize) {
        let total = images * self.parts.len();
        let pool_size = [vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(total as u32)];
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .pool_sizes(&pool_size)
            .max_sets(total as u32);
        self.descriptor_pool = device.create_descriptor_pool(&pool_info, None).expect("ppool");
        let layouts = vec![self.set_layout; total];
        let alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(self.descriptor_pool)
            .set_layouts(&layouts);
        let sets = device.allocate_descriptor_sets(&alloc_info).expect("psets");
        let mem_props = instance.get_physical_device_memory_properties(physical);
        for set in sets.into_iter() {
            let buffer_info = vk::BufferCreateInfo::default()
                .size(UBO_BYTES as u64)
                .usage(vk::BufferUsageFlags::UNIFORM_BUFFER)
                .sharing_mode(vk::SharingMode::EXCLUSIVE);
            let buffer = device.create_buffer(&buffer_info, None).expect("pubo");
            let req = device.get_buffer_memory_requirements(buffer);
            let index = super::find_memory_type(
                &mem_props,
                req.memory_type_bits,
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            );
            let alloc = vk::MemoryAllocateInfo::default()
                .allocation_size(req.size)
                .memory_type_index(index);
            let memory = device.allocate_memory(&alloc, None).expect("pmem");
            device.bind_buffer_memory(buffer, memory, 0).expect("pbind");
            let mapped = device
                .map_memory(memory, 0, UBO_BYTES as u64, vk::MemoryMapFlags::empty())
                .expect("pmap") as *mut u8;
            let _ = libc::mlock(mapped as *const libc::c_void, UBO_BYTES);
            let buffer_ref = [vk::DescriptorBufferInfo::default()
                .buffer(buffer)
                .offset(0)
                .range(UBO_BYTES as u64)];
            let write = [vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .buffer_info(&buffer_ref)];
            device.update_descriptor_sets(&write, &[]);
            self.ubo_buffers.push(buffer);
            self.ubo_memories.push(memory);
            self.ubo_mapped.push(mapped);
            self.ubo_sets.push(set);
        }
        self.image_count = images;
    }

    pub unsafe fn destroy_frames(&mut self, device: &ash::Device) {
        for memory in self.ubo_memories.drain(..) {
            device.unmap_memory(memory);
            device.free_memory(memory, None);
        }
        for buffer in self.ubo_buffers.drain(..) {
            device.destroy_buffer(buffer, None);
        }
        self.ubo_mapped.clear();
        self.ubo_sets.clear();
        device.destroy_descriptor_pool(self.descriptor_pool, None);
        self.descriptor_pool = vk::DescriptorPool::null();
        self.image_count = 0;
    }

    fn node_matrix(&self, node: Node) -> Mat4 {
        // Static group offsets live in plane space, nose +z.
        match node {
            Node::Hull => Mat4::IDENTITY,
            Node::Canopy => Mat4::from_translation(Vec3::new(0.0, 0.37, 1.25)),
            Node::WingL => Mat4::from_translation(Vec3::new(-1.2, 0.15, 0.0)),
            Node::WingR => Mat4::from_translation(Vec3::new(1.2, 0.15, 0.0)),
            Node::Flap(id) => {
                let side = if id < 3 { -1.0 } else { 1.0 };
                let pivot = flap_pivot(side, (id % 3) as usize);
                let comp = Vec3::new(side * 1.2, 0.15, 0.0);
                let p = Vec3::new(pivot.x + comp.x, pivot.y + comp.y, -(pivot.z + comp.z));
                Mat4::from_translation(p) * Mat4::from_rotation_x(self.anim.flaps[id as usize])
            }
            Node::Rotor => {
                Mat4::from_translation(Vec3::new(0.0, 0.34, -2.63))
                    * Mat4::from_rotation_z(self.anim.rotor)
            }
            Node::Petal(i) => {
                let a = i as f32 / 10.0 * std::f32::consts::TAU;
                let hinge = Vec3::new(-a.sin() * 0.46, a.cos() * 0.46 + 0.34, -(1.3 + 1.35));
                Mat4::from_translation(hinge)
                    * Mat4::from_rotation_z(a)
                    * Mat4::from_rotation_x(self.anim.petals[i as usize])
            }
            Node::Fin(i) => {
                let side = if i == 0 { -1.0 } else { 1.0 };
                let p = Vec3::new(side * 0.65, 0.65 + 0.2, -(2.0 + 2.5));
                Mat4::from_translation(p)
                    * Mat4::from_rotation_z(side * 0.5)
                    * Mat4::from_rotation_x(self.anim.elevators[i as usize])
            }
        }
    }

    pub unsafe fn record(
        &self,
        device: &ash::Device,
        cmd: vk::CommandBuffer,
        image: vk::Image,
        view: vk::ImageView,
        depth_image: vk::Image,
        depth_view: vk::ImageView,
        extent: vk::Extent2D,
        image_index: usize,
    ) {
        let begin = vk::CommandBufferBeginInfo::default();
        device.begin_command_buffer(cmd, &begin).expect("pbegin");
        let clear_color = vk::ClearValue {
            color: vk::ClearColorValue { float32: [0.596, 0.796, 0.945, 1.0] },
        };
        let clear_depth = vk::ClearValue {
            depth_stencil: vk::ClearDepthStencilValue { depth: 1.0, stencil: 0 },
        };
        let color_info = vk::RenderingAttachmentInfo::default()
            .image_view(view)
            .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .clear_value(clear_color);
        let depth_info = vk::RenderingAttachmentInfo::default()
            .image_view(depth_view)
            .image_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::DONT_CARE)
            .clear_value(clear_depth);
        let colors = [color_info];
        let rendering = vk::RenderingInfo::default()
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent,
            })
            .layer_count(1)
            .color_attachments(&colors)
            .depth_attachment(&depth_info);
        let color_range = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::COLOR)
            .base_mip_level(0)
            .level_count(1)
            .base_array_layer(0)
            .layer_count(1);
        let depth_range = vk::ImageSubresourceRange::default()
            .aspect_mask(vk::ImageAspectFlags::DEPTH)
            .base_mip_level(0)
            .level_count(1)
            .base_array_layer(0)
            .layer_count(1);
        let to_draw = [
            vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::empty())
                .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .image(image)
                .subresource_range(color_range),
            vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::empty())
                .dst_access_mask(vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)
                .image(depth_image)
                .subresource_range(depth_range),
        ];
        device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS
                | vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &to_draw,
        );
        device.cmd_begin_rendering(cmd, &rendering);
        let viewport = vk::Viewport::default()
            .x(0.0)
            .y(0.0)
            .width(extent.width as f32)
            .height(extent.height as f32)
            .min_depth(0.0)
            .max_depth(1.0);
        device.cmd_set_viewport(cmd, 0, &[viewport]);
        let scissor = vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent,
        };
        device.cmd_set_scissor(cmd, 0, &[scissor]);
        device.cmd_bind_vertex_buffers(cmd, 0, &[self.vertex_buffer], &[0]);
        device.cmd_bind_index_buffer(cmd, self.index_buffer, 0, vk::IndexType::UINT16);
        // Opaque parts first, then glass.
        for pass in [false, true] {
            device.cmd_bind_pipeline(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                if pass { self.glass_pipeline } else { self.opaque_pipeline },
            );
            for (pi, part) in self.parts.iter().enumerate() {
                if part.transparent != pass {
                    continue;
                }
                let set = self.ubo_sets[image_index * self.parts.len() + pi];
                device.cmd_bind_descriptor_sets(
                    cmd,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.layout,
                    0,
                    &[set],
                    &[],
                );
                device.cmd_draw_indexed(
                    cmd,
                    part.index_count,
                    1,
                    part.first_index,
                    part.vert_offset as i32,
                    0,
                );
            }
        }
        device.cmd_end_rendering(cmd);
        let to_present = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
            .dst_access_mask(vk::AccessFlags::empty())
            .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
            .image(image)
            .subresource_range(color_range);
        device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[to_present],
        );
        device.end_command_buffer(cmd).expect("pend");
    }

    pub unsafe fn update(
        &mut self,
        u: &Controls,
        pose: &super::flight::Pose,
        view_proj: &Mat4,
        time: f32,
        dt: f32,
        image_index: usize,
    ) {
        let load = (1.0 / pose.bank.cos().max(0.3)).min(3.0);
        self.anim.step(u, load, pose.speed, pose.boost, time, dt);
        let pressure = Anim::pressure(pose.speed);
        let yaw = Mat4::from_rotation_y(pose.heading);
        let pitch = Mat4::from_rotation_x(-pose.pitch);
        let roll = Mat4::from_rotation_z(-pose.bank);
        let model = Mat4::from_translation(Vec3::new(pose.x, pose.y, pose.z)) * yaw * pitch * roll;
        let campos = Vec3::new(pose.x, pose.y, pose.z);
        for (pi, part) in self.parts.iter().enumerate() {
            let (albedo, emissive_base, weave) = mat_params(part.mat);
            let pulse = if part.mat == MatId::Glow {
                0.8 + self.anim.spool * 2.2 + (time * 5.0).sin() * 0.09
            } else {
                1.0
            };
            let part_model = model * self.node_matrix(part.node);
            let mvp = *view_proj * part_model;
            let cols = mvp.to_cols_array();
            let mod_cols = part_model.to_cols_array();
            let dst = self.ubo_mapped[image_index * self.parts.len() + pi] as *mut f32;
            std::ptr::copy_nonoverlapping(cols.as_ptr(), dst, 16);
            std::ptr::copy_nonoverlapping(mod_cols.as_ptr(), dst.add(16), 16);
            let glass = if part.mat == MatId::Glass { 2.0 } else { 0.0 };
            let body: [f32; 16] = [
                albedo[0], albedo[1], albedo[2], albedo[3],
                emissive_base[0] * pulse,
                emissive_base[1] * pulse,
                emissive_base[2] * pulse,
                emissive_base[3],
                self.anim.bend, time, pressure, weave + glass,
                campos.x, campos.y, campos.z, 0.0,
            ];
            std::ptr::copy_nonoverlapping(body.as_ptr(), dst.add(32), 16);
        }
    }
}
