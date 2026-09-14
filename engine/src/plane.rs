// Glider renderer, merged pass. All opaque parts draw in one call,
// glass in a second. Per-vertex node and material ids replace the
// old per-part uniforms. One uniform block per image holds the
// view-projection, all node matrices, and the shared flex terms.
// Per-frame CPU work is 23 matrices plus one coherent copy.

use super::flight::{Controls, SIM_STEP};
use airframe::{build_airframe, MatId, Node};
use airframe::{f32_to_f16, oct_encode};
use ash::vk;
use glam::{Mat4, Vec3};

const UBO_BYTES: usize = 1728;
const NODE_COUNT: usize = 23;
const VERTEX_BYTES: usize = 28;

fn node_index(node: Node) -> usize {
    match node {
        Node::Hull => 0,
        Node::Canopy => 1,
        Node::WingL => 2,
        Node::WingR => 3,
        Node::Flap(id) => 4 + id as usize,
        Node::Rotor => 10,
        Node::Petal(i) => 11 + i as usize,
        Node::Fin(i) => 21 + i as usize,
    }
}

fn mat_index(mat: MatId) -> u16 {
    match mat {
        MatId::Sail => 0,
        MatId::Composite => 1,
        MatId::Graphite => 2,
        MatId::Titanium => 3,
        MatId::Dark => 4,
        MatId::Seat => 5,
        MatId::Glass => 6,
        MatId::Glow => 7,
    }
}

fn damp(current: f32, target: f32, lambda: f32, dt: f32) -> f32 {
    current + (target - current) * (1.0 - (-lambda * dt).exp())
}

/// Procedural animation state tracking physical deflections and turbine dynamics.
pub struct Anim {
    /// Engine spool RPM factor [0.0..1.0] driving thrust glow and rotor speed.
    spool: f32,
    /// Wing structural bending deflection angle (radians) driven by G-load.
    bend: f32,
    /// Wing bending harmonic oscillation velocity.
    bend_vel: f32,
    /// Trailing edge flap deflection angles for 6 control flaps.
    flaps: [f32; 6],
    /// Canted V-tail elevator/rudder deflection angles for port and starboard fins.
    elevators: [f32; 2],
    /// Cumulative turbine rotor spin angle (radians).
    rotor: f32,
    /// Articulation opening angles for 10 exhaust vectoring petals.
    petals: [f32; 10],
}

impl Anim {
    /// Initialize default neutral animation state.
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

    pub fn step(&mut self, u: &Controls, load: f32, boost: f32, dt: f32) {
        self.spool = damp(self.spool, boost, 4.0, dt);
        let target = ((load - 1.0) * 0.15).clamp(-0.4, 1.1);
        let steps = (dt / SIM_STEP).ceil().max(1.0) as usize;
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
    }

    pub fn pressure(speed: f32) -> f32 {
        (speed / 100.0).min(1.0)
    }
}

fn wing_point(side: f32, t: f32, chord: f32) -> Vec3 {
    let x = 0.42 + 10.4 * t;
    let leading = -1.4 + 0.9 * t + 2.7 * t * t;
    let width = (2.35 - 1.65 * t) * (1.0 - t.powi(12) * 0.87);
    let y = 0.08
        + 0.22 * t
        + 0.65 * t.powi(5)
        + (chord * std::f32::consts::PI).sin() * 0.14 * (1.0 - t);
    Vec3::new(side * (x - 1.2), y, leading + width * chord)
}

fn flap_pivot(side: f32, k: usize) -> Vec3 {
    let start = 0.425 + k as f32 * 0.155;
    let end = start + 0.15;
    wing_point(side, (start + end) / 2.0, 0.77)
}

/// Sail cloth weave, same pattern as the web prototype: warm gray
/// base, fine grid, heavier lines every sixteen pixels. Returns all
/// mip levels with CPU box filtering so minification never aliases
/// into static. Each entry is (width, height, rgba bytes).
fn weave_mips() -> Vec<(u32, u32, Vec<u8>)> {
    let base = [0xDAu8, 0xD6, 0xC7, 0xFF];
    let fine = [0xC3u8, 0xBF, 0xAF, 0xFF];
    let heavy = [0xAAu8, 0xA9, 0x9A, 0xFF];
    let mut level = vec![0u8; 64 * 64 * 4];
    for y in 0..64 {
        for x in 0..64 {
            let color = if x % 16 == 0 || y % 16 == 0 {
                heavy
            } else if x % 4 == 0 || y % 4 == 0 {
                fine
            } else {
                base
            };
            level[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4].copy_from_slice(&color);
        }
    }
    let mut out = vec![(64u32, 64u32, level)];
    while out.last().map(|(w, _, _)| *w).unwrap_or(1) > 1 {
        let (w, h, prev) = out.last().unwrap().clone();
        let (nw, nh) = (w / 2, h / 2);
        let mut next = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..4 {
                    let sum = prev[(((2 * y) * w + 2 * x) * 4 + c) as usize] as u32
                        + prev[(((2 * y) * w + 2 * x + 1) * 4 + c) as usize] as u32
                        + prev[(((2 * y + 1) * w + 2 * x) * 4 + c) as usize] as u32
                        + prev[(((2 * y + 1) * w + 2 * x + 1) * 4 + c) as usize] as u32;
                    next[((y * nw + x) * 4 + c) as usize] = (sum / 4) as u8;
                }
            }
        }
        out.push((nw, nh, next));
    }
    out
}

/// High-performance GPU renderer for the glider airframe.
///
/// Encapsulates merged single-pass vertex/index buffers, descriptor sets,
/// procedural sail cloth weave textures, uniform buffers, and dynamic rendering pipelines.
pub struct Plane {
    opaque_count: u32,
    glass_first: u32,
    glass_count: u32,
    vertex_buffer: vk::Buffer,
    #[allow(dead_code)]
    vertex_memory: vk::DeviceMemory,
    index_buffer: vk::Buffer,
    #[allow(dead_code)]
    index_memory: vk::DeviceMemory,
    set_layout: vk::DescriptorSetLayout,
    descriptor_pool: vk::DescriptorPool,
    weave_view: vk::ImageView,
    weave_sampler: vk::Sampler,
    #[allow(dead_code)]
    weave_image: vk::Image,
    #[allow(dead_code)]
    weave_memory: vk::DeviceMemory,
    opaque_pipeline: vk::Pipeline,
    glass_pipeline: vk::Pipeline,
    sky_pipeline: vk::Pipeline,
    layout: vk::PipelineLayout,
    query_pool: vk::QueryPool,
    ubo_buffers: Vec<vk::Buffer>,
    ubo_memories: Vec<vk::DeviceMemory>,
    ubo_mapped: Vec<*mut u8>,
    ubo_sets: Vec<vk::DescriptorSet>,
    image_count: usize,
    samples: vk::SampleCountFlags,
    pub anim: Anim,
}

impl Plane {
    /// Construct the plane renderer: compiles WGSL shader via Naga, creates graphics pipelines,
    /// merges airframe geometry into indexed device-local GPU buffers, generates weave mipmaps,
    /// and allocates host-coherent UBO buffers for all swapchain frames.
    pub unsafe fn build(
        device: &ash::Device,
        instance: &ash::Instance,
        physical: vk::PhysicalDevice,
        queue_family: u32,
        queue: vk::Queue,
        format: vk::Format,
        max_aniso: f32,
        samples: vk::SampleCountFlags,
    ) -> Self {
        let raw = build_airframe();
        // One 28 byte stream: pos12 + oct4 + uvHalf4 + flex4 + ids4.
        let mut stream: Vec<u8> = Vec::new();
        let mut opaque: Vec<u16> = Vec::new();
        let mut glass: Vec<u16> = Vec::new();
        let mut tri_total = 0u32;
        for part in &raw {
            let base = (stream.len() / VERTEX_BYTES) as u32;
            assert!(
                base + part.verts.len() as u32 <= 65536,
                "merged verts exceed u16"
            );
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
            let reordered = airframe::forsyth::reorder(&part.idx);
            let node = node_index(part.node) as u16;
            let mat = mat_index(part.mat);
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
                stream.extend_from_slice(&node.to_le_bytes());
                stream.extend_from_slice(&mat.to_le_bytes());
            }
            let target = if part.mat == MatId::Glass {
                &mut glass
            } else {
                &mut opaque
            };
            for i in reordered {
                target.push((base + i) as u16);
            }
            tri_total += part.idx.len() as u32 / 3;
        }
        for part in &raw {
            let mut min = [f32::INFINITY; 3];
            let mut max = [f32::NEG_INFINITY; 3];
            for v in &part.verts {
                for k in 0..3 {
                    min[k] = min[k].min(v.pos[k]);
                    max[k] = max[k].max(v.pos[k]);
                }
            }
            println!(
                "part {:?}/{:?} v{} t{} bbox [{:.2},{:.2},{:.2}]-[{:.2},{:.2},{:.2}]",
                part.node,
                part.mat,
                part.verts.len(),
                part.idx.len() / 3,
                min[0],
                min[1],
                min[2],
                max[0],
                max[1],
                max[2],
            );
        }
        println!(
            "airframe: {} tris merged, {:.1} KiB verts, {:.1} KiB indices, 2 draws",
            tri_total,
            stream.len() as f32 / 1024.0,
            (opaque.len() + glass.len()) as f32 * 2.0 / 1024.0,
        );
        let mem_props = instance.get_physical_device_memory_properties(physical);
        let upload = |size: u64, usage: vk::BufferUsageFlags| {
            let info = vk::BufferCreateInfo::default()
                .size(size)
                .usage(usage)
                .sharing_mode(vk::SharingMode::EXCLUSIVE);
            let buffer = device.create_buffer(&info, None).expect("buffer");
            let req = device.get_buffer_memory_requirements(buffer);
            let index = super::find_memory_type(
                &mem_props,
                req.memory_type_bits,
                vk::MemoryPropertyFlags::DEVICE_LOCAL,
            );
            let alloc = vk::MemoryAllocateInfo::default()
                .allocation_size(req.size)
                .memory_type_index(index);
            let memory = device.allocate_memory(&alloc, None).expect("mem");
            device.bind_buffer_memory(buffer, memory, 0).expect("bind");
            (buffer, memory)
        };
        let (vertex_buffer, vertex_memory) = upload(
            stream.len() as u64,
            vk::BufferUsageFlags::VERTEX_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
        );
        // Opaque then glass in one index buffer.
        let mut indices = opaque;
        let glass_first = indices.len() as u32;
        let glass_count = glass.len() as u32;
        let opaque_count = glass_first;
        indices.extend_from_slice(&glass);
        let index_bytes = indices.len() * 2;
        let (index_buffer, index_memory) = upload(
            index_bytes as u64,
            vk::BufferUsageFlags::INDEX_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
        );
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
        device
            .bind_buffer_memory(stage, stage_mem, 0)
            .expect("sbind");
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
        device
            .queue_submit(queue, &[submit], fence)
            .expect("ssubmit");
        device
            .wait_for_fences(&[fence], true, u64::MAX)
            .expect("swait");
        device.destroy_fence(fence, None);
        device.destroy_command_pool(pool, None);
        device.destroy_buffer(stage, None);
        device.free_memory(stage_mem, None);

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
        let tex_index = super::find_memory_type(
            &mem_props,
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
        let stage2_index = super::find_memory_type(
            &mem_props,
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

        let bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(2)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
        ];
        let dsl_info = vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings);
        let set_layout = device
            .create_descriptor_set_layout(&dsl_info, None)
            .expect("pdsl");
        let layout_info =
            vk::PipelineLayoutCreateInfo::default().set_layouts(std::slice::from_ref(&set_layout));
        let layout = device
            .create_pipeline_layout(&layout_info, None)
            .expect("playout");
        let words = super::wgsl_to_spirv(include_str!("plane.wgsl"));
        let module_info = vk::ShaderModuleCreateInfo::default().code(&words);
        let module = device
            .create_shader_module(&module_info, None)
            .expect("pmodule");
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
            .stride(VERTEX_BYTES as u32)
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
                .format(vk::Format::R16G16_SINT)
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
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(4)
                .format(vk::Format::R16G16_UINT)
                .offset(24),
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
        let multisample =
            vk::PipelineMultisampleStateCreateInfo::default().rasterization_samples(samples);
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
        let mut rendering = vk::PipelineRenderingCreateInfo::default()
            .color_attachment_formats(&formats)
            .depth_attachment_format(vk::Format::D32_SFLOAT);
        let mut rendering_glass = vk::PipelineRenderingCreateInfo::default()
            .color_attachment_formats(&formats)
            .depth_attachment_format(vk::Format::D32_SFLOAT);
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
        let vs_sky_entry = c"vs_sky";
        let fs_sky_entry = c"fs_sky";
        let sky_stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(module)
                .name(vs_sky_entry),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(module)
                .name(fs_sky_entry),
        ];
        let sky_vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
        let sky_depth = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(true)
            .depth_write_enable(false)
            .depth_compare_op(vk::CompareOp::LESS_OR_EQUAL);
        let mut rendering_sky = vk::PipelineRenderingCreateInfo::default()
            .color_attachment_formats(&formats)
            .depth_attachment_format(vk::Format::D32_SFLOAT);
        let sky_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&sky_stages)
            .vertex_input_state(&sky_vertex_input)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&raster)
            .multisample_state(&multisample)
            .depth_stencil_state(&sky_depth)
            .color_blend_state(&blend_off_state)
            .dynamic_state(&dynamic_state)
            .layout(layout)
            .push_next(&mut rendering_sky);
        let pipelines = device
            .create_graphics_pipelines(
                vk::PipelineCache::null(),
                &[opaque_info, glass_info, sky_info],
                None,
            )
            .expect("ppipes");
        device.destroy_shader_module(module, None);
        let query_info = vk::QueryPoolCreateInfo::default()
            .query_type(vk::QueryType::TIMESTAMP)
            .query_count(2);
        let query_pool = device.create_query_pool(&query_info, None).expect("qpool");
        Self {
            opaque_count,
            glass_first,
            glass_count,
            vertex_buffer,
            vertex_memory,
            index_buffer,
            index_memory,
            set_layout,
            descriptor_pool: vk::DescriptorPool::null(),
            weave_view,
            weave_sampler,
            weave_image,
            weave_memory,
            opaque_pipeline: pipelines[0],
            glass_pipeline: pipelines[1],
            sky_pipeline: pipelines[2],
            layout,
            query_pool,
            ubo_buffers: Vec::new(),
            ubo_memories: Vec::new(),
            ubo_mapped: Vec::new(),
            ubo_sets: Vec::new(),
            image_count: 0,
            samples,
            anim: Anim::new(),
        }
    }

    pub unsafe fn build_frames(
        &mut self,
        device: &ash::Device,
        instance: &ash::Instance,
        physical: vk::PhysicalDevice,
        images: usize,
    ) {
        let pool_sizes = [
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::UNIFORM_BUFFER)
                .descriptor_count(images as u32),
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(images as u32),
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::SAMPLER)
                .descriptor_count(images as u32),
        ];
        let pool_info = vk::DescriptorPoolCreateInfo::default()
            .pool_sizes(&pool_sizes)
            .max_sets(images as u32);
        self.descriptor_pool = device
            .create_descriptor_pool(&pool_info, None)
            .expect("ppool");
        let layouts = vec![self.set_layout; images];
        let alloc_info = vk::DescriptorSetAllocateInfo::default()
            .descriptor_pool(self.descriptor_pool)
            .set_layouts(&layouts);
        let sets = device.allocate_descriptor_sets(&alloc_info).expect("psets");
        let mem_props = instance.get_physical_device_memory_properties(physical);
        for set in sets {
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
            let write_ubo = [vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                .buffer_info(&buffer_ref)];
            let image_ref = [vk::DescriptorImageInfo::default()
                .image_view(self.weave_view)
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
            let write_tex = [vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(1)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(&image_ref)];
            let sampler_ref = [vk::DescriptorImageInfo::default().sampler(self.weave_sampler)];
            let write_smp = [vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(2)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .image_info(&sampler_ref)];
            device.update_descriptor_sets(&write_ubo, &[]);
            device.update_descriptor_sets(&write_tex, &[]);
            device.update_descriptor_sets(&write_smp, &[]);
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

    pub unsafe fn prepare_query_pool(&mut self, device: &ash::Device, image_count: usize) {
        device.destroy_query_pool(self.query_pool, None);
        let query_info = vk::QueryPoolCreateInfo::default()
            .query_type(vk::QueryType::TIMESTAMP)
            .query_count((image_count * 2) as u32);
        self.query_pool = device
            .create_query_pool(&query_info, None)
            .expect("query pool");
    }

    fn node_matrix(&self, node: usize) -> Mat4 {
        match node {
            0 => Mat4::IDENTITY,
            1 => Mat4::from_translation(Vec3::new(0.0, 0.37, 1.25)),
            2 => Mat4::from_translation(Vec3::new(-1.2, 0.15, 0.0)),
            3 => Mat4::from_translation(Vec3::new(1.2, 0.15, 0.0)),
            4..=9 => {
                let id = node - 4;
                let side = if id < 3 { -1.0 } else { 1.0 };
                let pivot = flap_pivot(side, (id % 3) as usize);
                let comp = Vec3::new(side * 1.2, 0.15, 0.0);
                let p = Vec3::new(pivot.x + comp.x, pivot.y + comp.y, -(pivot.z + comp.z));
                Mat4::from_translation(p) * Mat4::from_rotation_x(self.anim.flaps[id])
            }
            10 => {
                Mat4::from_translation(Vec3::new(0.0, 0.34, -2.63))
                    * Mat4::from_rotation_z(self.anim.rotor)
            }
            11..=20 => {
                let i = node - 11;
                let a = i as f32 / 10.0 * std::f32::consts::TAU;
                let hinge = Vec3::new(-a.sin() * 0.46, a.cos() * 0.46 + 0.34, -(1.3 + 1.35));
                Mat4::from_translation(hinge)
                    * Mat4::from_rotation_z(a)
                    * Mat4::from_rotation_x(self.anim.petals[i])
            }
            21..=22 => {
                let side = if node == 21 { -1.0 } else { 1.0 };
                let p = Vec3::new(side * 0.65, 0.65 + 0.2, -(2.0 + 2.5));
                Mat4::from_translation(p)
                    * Mat4::from_rotation_z(side * 0.5)
                    * Mat4::from_rotation_x(self.anim.elevators[(node - 21) as usize])
            }
            _ => Mat4::IDENTITY,
        }
    }

    pub unsafe fn record(
        &self,
        device: &ash::Device,
        cmd: vk::CommandBuffer,
        image: vk::Image,
        view: vk::ImageView,
        msaa_image: vk::Image,
        msaa_view: vk::ImageView,
        depth_image: vk::Image,
        depth_view: vk::ImageView,
        extent: vk::Extent2D,
        image_index: usize,
        query_base: u32,
        measure_gpu: bool,
    ) {
        let begin = vk::CommandBufferBeginInfo::default();
        device.begin_command_buffer(cmd, &begin).expect("pbegin");
        if measure_gpu {
            device.cmd_reset_query_pool(cmd, self.query_pool, query_base, 2);
        }
        let clear_color = vk::ClearValue {
            color: vk::ClearColorValue {
                float32: [0.0, 0.0, 0.0, 1.0],
            },
        };
        let clear_depth = vk::ClearValue {
            depth_stencil: vk::ClearDepthStencilValue {
                depth: 1.0,
                stencil: 0,
            },
        };
        let mut color_info = vk::RenderingAttachmentInfo::default()
            .image_view(if self.samples == vk::SampleCountFlags::TYPE_1 {
                view
            } else {
                msaa_view
            })
            .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::DONT_CARE)
            .store_op(if self.samples == vk::SampleCountFlags::TYPE_1 {
                vk::AttachmentStoreOp::STORE
            } else {
                vk::AttachmentStoreOp::DONT_CARE
            })
            .clear_value(clear_color);
        if self.samples != vk::SampleCountFlags::TYPE_1 {
            color_info = color_info
                .resolve_mode(vk::ResolveModeFlags::AVERAGE)
                .resolve_image_view(view)
                .resolve_image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
        }
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
        let mut to_draw = Vec::with_capacity(3);
        if self.samples != vk::SampleCountFlags::TYPE_1 {
            to_draw.push(
                vk::ImageMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::empty())
                    .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                    .old_layout(vk::ImageLayout::UNDEFINED)
                    .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                    .image(msaa_image)
                    .subresource_range(color_range),
            );
        }
        to_draw.push(
            vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::empty())
                .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .image(image)
                .subresource_range(color_range),
        );
        to_draw.push(
            vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::empty())
                .dst_access_mask(vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)
                .image(depth_image)
                .subresource_range(depth_range),
        );
        let attachment_dependency = [vk::MemoryBarrier::default()
            .src_access_mask(
                vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                    | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
            )
            .dst_access_mask(
                vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                    | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
            )];
        let attachment_stages = vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
            | vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS
            | vk::PipelineStageFlags::LATE_FRAGMENT_TESTS;
        device.cmd_pipeline_barrier(
            cmd,
            attachment_stages,
            attachment_stages,
            vk::DependencyFlags::empty(),
            &attachment_dependency,
            &[],
            &to_draw,
        );
        if measure_gpu {
            device.cmd_write_timestamp(
                cmd,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                self.query_pool,
                query_base,
            );
        }
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
        device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.opaque_pipeline);
        let set = self.ubo_sets[image_index];
        device.cmd_bind_descriptor_sets(
            cmd,
            vk::PipelineBindPoint::GRAPHICS,
            self.layout,
            0,
            &[set],
            &[],
        );
        device.cmd_draw_indexed(cmd, self.opaque_count, 1, 0, 0, 0);
        if measure_gpu {
            device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.sky_pipeline);
            device.cmd_draw(cmd, 6, 1, 0, 0);
        }
        device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.glass_pipeline);
        device.cmd_bind_descriptor_sets(
            cmd,
            vk::PipelineBindPoint::GRAPHICS,
            self.layout,
            0,
            &[set],
            &[],
        );
        device.cmd_draw_indexed(cmd, self.glass_count, 1, self.glass_first, 0, 0);
        device.cmd_end_rendering(cmd);
        if measure_gpu {
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
            device.cmd_write_timestamp(
                cmd,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                self.query_pool,
                query_base + 1,
            );
        }
        device.end_command_buffer(cmd).expect("pend");
    }

    /// Advance physics-driven airframe animation states (wing bending, control flaps, rotor spin, and vectoring petals).
    pub fn step_animation(&mut self, u: &Controls, pose: &super::flight::Pose, dt: f32) {
        let load = ((1.0 + u.pitch.max(0.0) * 1.5) / pose.bank.cos().max(0.3)).min(3.5);
        self.anim.step(u, load, pose.boost, dt);
    }

    /// Update host-coherent UBO buffer memory with the latest view-projection, camera eye vector,
    /// 23 kinematic node transform matrices, and aeroelastic flex coefficients.
    pub unsafe fn update(
        &mut self,
        pose: &super::flight::Pose,
        view_proj: &Mat4,
        origin: Vec3,
        eye_rel: Vec3,
        time: f32,
        image_index: usize,
    ) {
        let pressure = Anim::pressure(pose.speed);
        // Orientation relative to lift plane: body pitch around wings (X),
        // banked around roll axis (Z), then oriented along compass heading (Y).
        let yaw = Mat4::from_rotation_y(pose.heading);
        let roll = Mat4::from_rotation_z(-pose.bank);
        let pitch = Mat4::from_rotation_x(-pose.pitch);
        let rel = Vec3::new(pose.x, pose.y, pose.z) - origin;
        let model = Mat4::from_translation(rel) * yaw * roll * pitch;
        let campos = eye_rel;
        let inv_view_proj = view_proj.inverse();
        // One coherent copy: view-proj, inv-view-proj, all nodes, plane frame, flex, camera, sun.
        let dst = self.ubo_mapped[image_index] as *mut f32;
        std::ptr::copy_nonoverlapping(view_proj.to_cols_array().as_ptr(), dst, 16);
        std::ptr::copy_nonoverlapping(inv_view_proj.to_cols_array().as_ptr(), dst.add(16), 16);
        for n in 0..NODE_COUNT {
            let m = model * self.node_matrix(n);
            std::ptr::copy_nonoverlapping(m.to_cols_array().as_ptr(), dst.add(32 + n * 16), 16);
        }
        let sun_radius: f32 = 0.020; // Authentic visible angular radius (~1.15 degrees)
        // Fixed celestial astronomical coordinate:
        // Azimuth = 0.55 rad (~31.5 degrees East of North)
        // Elevation = 0.38 rad (~21.8 degrees elevation above horizon)
        let sun_elevation: f32 = 0.38;
        let sun_azimuth: f32 = 0.55;
        let sun_dir = Vec3::new(
            sun_azimuth.sin() * sun_elevation.cos(),
            sun_elevation.sin(),
            sun_azimuth.cos() * sun_elevation.cos(),
        ).normalize();
        // Physical atmospheric transmittance along solar ray (Rayleigh + Mie + Ozone):
        let m_ray = 1.0
            / (sun_elevation.max(0.0)
                + 0.0548 * (1.01 - sun_elevation.max(0.0)).powf(1.8).max(0.0)
                + 0.001);
        let m_oz = 1.0 / (sun_elevation * sun_elevation + 0.0045).max(1e-6).sqrt();
        let tau_r = Vec3::new(0.046416, 0.108464, 0.264800) * m_ray;
        let tau_m = Vec3::new(0.010123, 0.010123, 0.010123) * m_ray;
        let tau_oz = Vec3::new(0.009750, 0.028215, 0.001275) * m_oz;
        let tau = tau_r + tau_m + tau_oz;
        let twilight = ((sun_elevation + 0.08) / 0.10).clamp(0.0, 1.0);
        let sun_trans = Vec3::new((-tau.x).exp(), (-tau.y).exp(), (-tau.z).exp()) * twilight;
        let sun_irr = sun_trans * 3.2;
        let glow = 0.8 + self.anim.spool * 2.2 + (time * 5.0).sin() * 0.09;
        let smoothstep = |e0: f32, e1: f32, x: f32| -> f32 {
            let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        let zenith_t = smoothstep(-0.1, 0.4, sun_dir.y);
        let zenith_sky = Vec3::new(0.06, 0.15, 0.42).lerp(Vec3::new(0.14, 0.32, 0.68), zenith_t);
        let horizon_t = smoothstep(0.0, 0.35, sun_dir.y);
        let horizon_haze = Vec3::new(0.85, 0.48, 0.25).lerp(Vec3::new(0.66, 0.79, 0.90), horizon_t);
        let ground_base = Vec3::new(0.07, 0.09, 0.06) * (sun_dir.y.max(0.05) * 1.4 + 0.1);
        let cos_radius = sun_radius.cos();
        let inv_one_minus_cos_radius = 1.0 / (1.0 - cos_radius).max(1e-7);

        let tail: [f32; 32] = [
            self.anim.bend,
            time,
            pressure,
            glow,
            campos.x,
            campos.y,
            campos.z,
            0.0,
            sun_dir.x,
            sun_dir.y,
            sun_dir.z,
            sun_radius,
            sun_irr.x,
            sun_irr.y,
            sun_irr.z,
            sun_elevation,
            zenith_sky.x,
            zenith_sky.y,
            zenith_sky.z,
            cos_radius,
            horizon_haze.x,
            horizon_haze.y,
            horizon_haze.z,
            inv_one_minus_cos_radius,
            ground_base.x,
            ground_base.y,
            ground_base.z,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
        ];
        std::ptr::copy_nonoverlapping(tail.as_ptr(), dst.add(32 + NODE_COUNT * 16), 32);
    }

    pub(crate) fn query_pool(&self) -> vk::QueryPool {
        self.query_pool
    }
}
