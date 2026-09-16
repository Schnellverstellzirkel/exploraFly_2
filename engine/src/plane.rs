// Glider renderer, merged pass. All opaque parts draw in one call,
// glass in a second. Per-vertex node and material ids replace the
// old per-part uniforms. One uniform block per image holds the
// view-projection, all node matrices, and the shared flex terms.
// Per-frame CPU work is 23 matrices plus one coherent copy.

use sim::flight::{Controls, SIM_STEP};
use airframe::{build_airframe, MatId, Node};
use airframe::{f32_to_f16, oct_encode};
use ash::khr;
use ash::vk;
use glam::{Mat4, Vec3};

const UBO_BYTES: usize = 1792;
const NODE_COUNT: usize = 23;
const VERTEX_BYTES: usize = 28;
// VkAccelerationStructureInstanceKHR stride (transform 48 + 2 packed u32 +
// device reference 8 + 16 B padding to 16-byte instance alignment).
const RT_INSTANCE_BYTES: usize = std::mem::size_of::<vk::AccelerationStructureInstanceKHR>();
const GROUND_LEVEL: f32 = 0.0;
// The ground shader receives the floating-origin anchor as a split coordinate:
// an integer number of 25 cm cells plus a sub-cell remainder. This preserves
// stable material phase without adding double-precision work to the fragment
// path.
const GROUND_FINE_CELL: f32 = 0.25;
const PLUME_WARP_BOUND: f32 = 1.35;
const PLUME_AXIAL_BOUND: f32 = 1.35;
// Fixed sun/sky terms. Keeping these precomputed avoids repeating the
// celestial atmosphere setup in the present-rate update path.
// Mean solar angular half-radius at 1 AU (~16 arcminutes).
const SUN_RADIUS: f32 = 0.00465;
const SUN_ELEVATION: f32 = 0.38;
const SUN_DIR: Vec3 = Vec3::new(0.48540115, 0.37092048, 0.79170936);
const SUN_IRRADIANCE: Vec3 = Vec3::new(3.05, 3.00, 2.90);
const SKY_ZENITH: Vec3 = Vec3::new(0.08, 0.22, 0.68);
const SKY_HORIZON: Vec3 = Vec3::new(0.55, 0.72, 0.90);
const GROUND_BASE: Vec3 = Vec3::new(0.04335021, 0.05573598, 0.03715732);
const SUN_COS_RADIUS: f32 = 1.0 - SUN_RADIUS * SUN_RADIUS * 0.5;
const INV_ONE_MINUS_SUN_COS_RADIUS: f32 = 1.0 / (1.0 - SUN_COS_RADIUS);

/// Round an acceleration structure offset up to the 256-byte alignment
/// required by acceleration structure storage and device address rules.
fn align_256(v: u64) -> u64 {
    (v + 255) & !255
}

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

/// Pack an absolute X/Z origin into the representation consumed by ground.frag.
/// Keeping the fractional remainder separate prevents adding a large world
/// coordinate to a small camera-relative hit from erasing sub-meter detail.
fn ground_origin_pack(origin: Vec3) -> [f32; 4] {
    let cell = [
        (origin.x / GROUND_FINE_CELL).floor(),
        (origin.z / GROUND_FINE_CELL).floor(),
    ];
    [
        cell[0],
        cell[1],
        origin.x - cell[0] * GROUND_FINE_CELL,
        origin.z - cell[1] * GROUND_FINE_CELL,
    ]
}

/// Procedural animation state tracking physical deflections and turbine dynamics.
pub struct Anim {
    /// Engine spool RPM factor [0.0..1.0] driving thrust glow and rotor speed.
    spool: f32,
    time: f32,
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
            time: 0.0,
            bend: 0.0,
            bend_vel: 0.0,
            flaps: [0.0; 6],
            elevators: [0.0; 2],
            rotor: 0.0,
            petals: [0.12; 10],
        }
    }

    pub fn step(&mut self, u: &Controls, load: f32, boost: f32, dt: f32) {
        self.time += dt;
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
            *petal = damp(*petal, 0.12 + 0.30 * self.spool, 8.0, dt);
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

/// Fresnel-free directional albedo for height-correlated Smith GGX.
/// Uses deterministic Hammersley NDF quadrature, once at initialization.
/// The same visibility is used in shaders/plane.frag; anisotropic compensation uses
/// the geometric mean alpha as an approximation.
pub fn energy_lut() -> Vec<f32> {
    const N: usize = 32;
    const SAMPLES: u32 = 4096;
    let mut lut = vec![1.0f32; N * N];
    for j in 0..N {
        let alpha = (j as f32 + 0.5) / N as f32;

        for i in 0..N {
            let mu_o = (i as f32 + 0.5) / N as f32;
            let sin_o = (1.0 - mu_o * mu_o).max(0.0).sqrt();
            let o = glam::Vec3::new(sin_o, 0.0, mu_o);
            let mut acc = 0.0f32;
            for sample in 0..SAMPLES {
                let xi1 = (sample as f32 + 0.5) / SAMPLES as f32;
                let xi2 = sample.reverse_bits() as f32 * (1.0 / 4294967296.0);
                let cos_h = ((1.0 - xi1) / (1.0 + (alpha * alpha - 1.0) * xi1)).sqrt();
                let sin_h = (1.0 - cos_h * cos_h).max(0.0).sqrt();
                let phi = std::f32::consts::TAU * xi2;
                let h = glam::Vec3::new(sin_h * phi.cos(), sin_h * phi.sin(), cos_h);
                let oh = o.dot(h);
                let inc = 2.0 * oh * h - o;
                if inc.z <= 0.0 {
                    continue; // reflected light direction below the surface
                }
                let root_o = (alpha * alpha * (1.0 - mu_o * mu_o) + mu_o * mu_o).sqrt();
                let root_i = (alpha * alpha * (1.0 - inc.z * inc.z) + inc.z * inc.z).sqrt();
                let g2 = 2.0 * mu_o * inc.z / (inc.z * root_o + mu_o * root_i);
                acc += g2 * oh / (mu_o * cos_h);
            }
            lut[j * N + i] = acc / SAMPLES as f32;
        }
    }
    lut
}

/// Average albedo Eavg = integrate Ess over mu with weight 2 mu, for tests.
#[cfg(test)]
fn energy_avg(lut: &[f32], j: usize) -> f32 {
    const N: usize = 32;
    let mut acc = 0.0;
    for i in 0..N {
        let mu = (i as f32 + 0.5) / N as f32;
        acc += 2.0 * mu * lut[j * N + i];
    }
    acc / N as f32
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

/// IBL quality variants are compiled offline by build.rs (shaderc) from
/// shaders/plane.frag with an injected ENV header. Large counts are an offline
/// visual reference for the same sky/BRDF, not a separate cheaper model.
fn plane_frag_spv(samples: u32) -> Vec<u32> {
    match samples {
        4 => crate::spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/plane-4.frag.spv"))),
        8 => crate::spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/plane-8.frag.spv"))),
        16 => crate::spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/plane-16.frag.spv"))),
        32 => crate::spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/plane-32.frag.spv"))),
        128 => {
            crate::spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/plane-128.frag.spv")))
        }
        _ => panic!("EXPLORA_IBL_SAMPLES must be 4, 8, 16, 32, or 128"),
    }
}

/// Ray-query variants of the material fragment shaders (ENABLE_RT header).
fn plane_frag_spv_rt(samples: u32) -> Vec<u32> {
    match samples {
        4 => crate::spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/plane-4-rt.frag.spv"))),
        8 => crate::spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/plane-8-rt.frag.spv"))),
        16 => crate::spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/plane-16-rt.frag.spv"))),
        32 => crate::spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/plane-32-rt.frag.spv"))),
        128 => {
            crate::spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/plane-128-rt.frag.spv")))
        }
        _ => panic!("EXPLORA_IBL_SAMPLES must be 4, 8, 16, 32, or 128"),
    }
}

fn ground_frag_spv_rt() -> Vec<u32> {
    crate::spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/ground-rt.frag.spv")))
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
    lut_view: vk::ImageView,
    lut_sampler: vk::Sampler,
    #[allow(dead_code)]
    weave_image: vk::Image,
    #[allow(dead_code)]
    weave_memory: vk::DeviceMemory,
    opaque_pipeline: vk::Pipeline,
    glass_pipeline: vk::Pipeline,
    sky_pipeline: vk::Pipeline,
    ground_pipeline: vk::Pipeline,
    cloud_pipeline: vk::Pipeline,
    void_pipeline: vk::Pipeline,
    layout: vk::PipelineLayout,
    // FX volumetric resources (noise volumes + descriptor layout).
    // Pipelines and HDR targets land in the next increment; layout first
    // so descriptor indices freeze before record() changes.
    fx_layout: vk::DescriptorSetLayout,
    #[allow(dead_code)]
    noise_base_image: vk::Image,
    #[allow(dead_code)]
    noise_base_memory: vk::DeviceMemory,
    noise_base_view: vk::ImageView,
    noise_base_sampler: vk::Sampler,
    #[allow(dead_code)]
    noise_detail_image: vk::Image,
    #[allow(dead_code)]
    noise_detail_memory: vk::DeviceMemory,
    noise_detail_view: vk::ImageView,
    noise_detail_sampler: vk::Sampler,
    // Curl warp texture. Image + memory owned until process exit like the
    // other boot-time resources; only view/sampler are referenced per frame.
    noise_curl_view: vk::ImageView,
    noise_curl_sampler: vk::Sampler,
    // FX passes: plume volume raymarch, trail ribbons, HDR composite.
    plume_pipeline: vk::Pipeline,
    trail_pipeline: vk::Pipeline,
    composite_pipeline: vk::Pipeline,
    fx_pipeline_layout: vk::PipelineLayout,
    composite_layout: vk::PipelineLayout,
    composite_set_layout: vk::DescriptorSetLayout,
    fx_pool: vk::DescriptorPool,
    fx_sets: Vec<vk::DescriptorSet>,
    // Per-frame host-visible cone (rewritten from nozzle state) + ribbons.
    cone_buffers: Vec<vk::Buffer>,
    cone_memories: Vec<vk::DeviceMemory>,
    cone_mapped: Vec<*mut u8>,
    trail_buffers: Vec<vk::Buffer>,
    trail_memories: Vec<vk::DeviceMemory>,
    trail_mapped: Vec<*mut u8>,
    // Last origin each trail slot was packed against + whether it holds a
    // full pack. Presents without a sim step only translate centers by the
    // origin delta instead of rescanning the pools.
    trail_origin: Vec<Vec3>,
    trail_filled: Vec<bool>,
    // Static index buffers (one copy per frame slot, filled once per
    // swapchain rebuild with the fixed cone grid + ribbon chain pattern).
    cone_ibos: Vec<vk::Buffer>,
    cone_ibo_mems: Vec<vk::DeviceMemory>,
    trail_ibos: Vec<vk::Buffer>,
    trail_ibo_mems: Vec<vk::DeviceMemory>,
    trail_index_count: u32,
    // Unit plume proxy (length 1, radius 1 along -Z) transformed per frame.
    unit_cone: Vec<[f32; 5]>,
    query_pool: vk::QueryPool,
    ubo_buffers: Vec<vk::Buffer>,
    ubo_memories: Vec<vk::DeviceMemory>,
    ubo_mapped: Vec<*mut u8>,
    ubo_sets: Vec<vk::DescriptorSet>,
    image_count: usize,
    samples: vk::SampleCountFlags,
    // Ray-traced soft shadows (VK_KHR_ray_query on the aircraft TLAS).
    rt_supported: bool,
    rt_loader: khr::acceleration_structure::Device,
    #[allow(dead_code)]
    rt_vertex_address: vk::DeviceAddress,
    #[allow(dead_code)]
    rt_index_buffer: vk::Buffer,
    #[allow(dead_code)]
    rt_index_memory: vk::DeviceMemory,
    #[allow(dead_code)]
    rt_index_address: vk::DeviceAddress,
    // One bottom-level structure per animated node, built once at boot.
    // Kept alive for the process lifetime (like the mesh buffers).
    #[allow(dead_code)]
    rt_blas: Vec<vk::AccelerationStructureKHR>,
    rt_blas_addresses: Vec<vk::DeviceAddress>,
    rt_geom_nodes: Vec<u32>,
    #[allow(dead_code)]
    rt_blas_buffer: vk::Buffer,
    #[allow(dead_code)]
    rt_blas_memory: vk::DeviceMemory,
    #[allow(dead_code)]
    rt_blas_scratch_buffer: vk::Buffer,
    #[allow(dead_code)]
    rt_blas_scratch_memory: vk::DeviceMemory,
    // Per swapchain slot: host instance transforms + top-level structure.
    rt_instance_buffers: Vec<vk::Buffer>,
    #[allow(dead_code)]
    rt_instance_memories: Vec<vk::DeviceMemory>,
    rt_instance_mapped: Vec<*mut u8>,
    rt_tlas: Vec<vk::AccelerationStructureKHR>,
    #[allow(dead_code)]
    rt_tlas_buffers: Vec<vk::Buffer>,
    #[allow(dead_code)]
    rt_tlas_memories: Vec<vk::DeviceMemory>,
    #[allow(dead_code)]
    rt_scratch: Vec<vk::Buffer>,
    #[allow(dead_code)]
    rt_scratch_memories: Vec<vk::DeviceMemory>,
    rt_instance_count: u32,
    pub anim: Anim,
}

impl Plane {
    /// Construct the plane renderer: loads offline SPIR-V, creates graphics pipelines,
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
        ground_fsr: bool,
        rt_supported: bool,
    ) -> Self {
        let raw = build_airframe();
        // One 28 byte stream: pos12 + oct4 + uvHalf4 + flex4 + ids4.
        let mut stream: Vec<u8> = Vec::new();
        let mut opaque: Vec<u16> = Vec::new();
        let mut glass: Vec<u16> = Vec::new();
        // Ray-traced shadow casters: raw absolute indices grouped per node so
        // each BLAS gets a contiguous index range. Glass panels never cast a
        // hard sun shadow (their shade pass ignores direct light).
        let mut rt_nodes: Vec<Vec<u16>> = vec![Vec::new(); NODE_COUNT];
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
            for i in &reordered {
                target.push((base + i) as u16);
            }
            if part.mat != MatId::Glass {
                rt_nodes[node_index(part.node)].extend((&reordered).iter().map(|i| (base + i) as u16));
            }
            tri_total += part.idx.len() as u32 / 3;
        }
        // Contiguous per-node ranges; nodes without casters are dropped.
        let mut rt_idx: Vec<u16> = Vec::new();
        let mut rt_node_ranges: Vec<(u32, u32)> = Vec::new();
        let mut rt_geom_nodes: Vec<u32> = Vec::new();
        for (n, idx) in rt_nodes.iter().enumerate() {
            if idx.is_empty() {
                continue;
            }
            rt_node_ranges.push((rt_idx.len() as u32, idx.len() as u32));
            rt_geom_nodes.push(n as u32);
            rt_idx.extend_from_slice(idx);
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
            let memory = if usage.contains(vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS) {
                // Memory backing device-addressable buffers must be allocated
                // with the DEVICE_ADDRESS flag (VUID-vkBindBufferMemory-bufferDeviceAddress-03339).
                let mut addr_flags = vk::MemoryAllocateFlagsInfo::default()
                    .flags(vk::MemoryAllocateFlags::DEVICE_ADDRESS);
                device
                    .allocate_memory(&alloc.push_next(&mut addr_flags), None)
                    .expect("mem")
            } else {
                device.allocate_memory(&alloc, None).expect("mem")
            };
            device.bind_buffer_memory(buffer, memory, 0).expect("bind");
            (buffer, memory)
        };
        let mut vertex_usage = vk::BufferUsageFlags::VERTEX_BUFFER | vk::BufferUsageFlags::TRANSFER_DST;
        if rt_supported {
            vertex_usage |= vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
                | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR;
        }
        let (vertex_buffer, vertex_memory) = upload(stream.len() as u64, vertex_usage);
        let rt_vertex_address = if rt_supported {
            device
                .get_buffer_device_address(
                    &vk::BufferDeviceAddressInfo::default().buffer(vertex_buffer),
                )
        } else {
            0
        };
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

        // Ray-traced soft shadows (VK_KHR_ray_query): one bottom-level
        // structure per animated node, built once here into a shared
        // device-local buffer. Each frame slot references them through a
        // top-level structure updated with per-node instance transforms.
        let rt_loader = khr::acceleration_structure::Device::new(instance, device);
        let mut rt_blas_buffer = vk::Buffer::null();
        let mut rt_blas_memory = vk::DeviceMemory::null();
        let mut rt_blas_scratch_buffer = vk::Buffer::null();
        let mut rt_blas_scratch_memory = vk::DeviceMemory::null();
        let mut rt_blas = Vec::new();
        let mut rt_blas_addresses = Vec::new();
        let mut rt_index_buffer = vk::Buffer::null();
        let mut rt_index_memory = vk::DeviceMemory::null();
        let mut rt_index_address = 0;
        let rt_instance_count = rt_geom_nodes.len() as u32;
        if rt_supported && rt_instance_count > 0 {
            let rt_index_bytes = (rt_idx.len() * 2) as u64;
            let (ribuf, rimem) = upload(
                rt_index_bytes,
                vk::BufferUsageFlags::INDEX_BUFFER
                    | vk::BufferUsageFlags::TRANSFER_DST
                    | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
                    | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR,
            );
            let rt_stage_info = vk::BufferCreateInfo::default()
                .size(rt_index_bytes)
                .usage(vk::BufferUsageFlags::TRANSFER_SRC)
                .sharing_mode(vk::SharingMode::EXCLUSIVE);
            let rt_stage = device.create_buffer(&rt_stage_info, None).expect("rtstage");
            let rt_stage_req = device.get_buffer_memory_requirements(rt_stage);
            let rt_stage_index = super::find_memory_type(
                &mem_props,
                rt_stage_req.memory_type_bits,
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            );
            let rt_stage_alloc = vk::MemoryAllocateInfo::default()
                .allocation_size(rt_stage_req.size)
                .memory_type_index(rt_stage_index);
            let rt_stage_mem = device.allocate_memory(&rt_stage_alloc, None).expect("rtsmem");
            device.bind_buffer_memory(rt_stage, rt_stage_mem, 0).expect("rtsbind");
            let rt_stage_map = device
                .map_memory(rt_stage_mem, 0, rt_index_bytes, vk::MemoryMapFlags::empty())
                .expect("rtsmap") as *mut u8;
            if !rt_idx.is_empty() {
                std::ptr::copy_nonoverlapping(
                    rt_idx.as_ptr() as *const u8,
                    rt_stage_map,
                    rt_idx.len() * 2,
                );
            }
            device.unmap_memory(rt_stage_mem);
            let rt_pool_info = vk::CommandPoolCreateInfo::default()
                .queue_family_index(queue_family)
                .flags(vk::CommandPoolCreateFlags::TRANSIENT);
            let rt_pool = device.create_command_pool(&rt_pool_info, None).expect("rtpool");
            let rt_alloc = vk::CommandBufferAllocateInfo::default()
                .command_pool(rt_pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1);
            let rt_cmd = device.allocate_command_buffers(&rt_alloc).expect("rtcmd")[0];
            let rt_begin = vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            device.begin_command_buffer(rt_cmd, &rt_begin).expect("rtcbegin");
            let rt_copy = vk::BufferCopy::default().size(rt_index_bytes);
            device.cmd_copy_buffer(rt_cmd, rt_stage, ribuf, &[rt_copy]);
            device.end_command_buffer(rt_cmd).expect("rtcend");
            let rt_fence_info = vk::FenceCreateInfo::default();
            let rt_fence = device.create_fence(&rt_fence_info, None).expect("rtfence");
            let rt_cmd_ref = [rt_cmd];
            let rt_submit = vk::SubmitInfo::default().command_buffers(&rt_cmd_ref);
            device.queue_submit(queue, &[rt_submit], rt_fence).expect("rtsubmit");
            device.wait_for_fences(&[rt_fence], true, u64::MAX).expect("rtfwait");
            device.destroy_fence(rt_fence, None);
            device.destroy_command_pool(rt_pool, None);
            device.destroy_buffer(rt_stage, None);
            device.free_memory(rt_stage_mem, None);
            rt_index_buffer = ribuf;
            rt_index_memory = rimem;
            rt_index_address = device
                .get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(ribuf));

            // Triangle geometries for the casters, one per node range.
            let mut geoms: Vec<vk::AccelerationStructureGeometryKHR> = Vec::with_capacity(rt_geom_nodes.len());
            let mut ranges: Vec<vk::AccelerationStructureBuildRangeInfoKHR> = Vec::with_capacity(rt_geom_nodes.len());
            let max_vertex = (stream.len() / VERTEX_BYTES) as u32 - 1;
            for (off, cnt) in &rt_node_ranges {
                let triangles = vk::AccelerationStructureGeometryTrianglesDataKHR::default()
                    .vertex_format(vk::Format::R32G32B32_SFLOAT)
                    .vertex_data(vk::DeviceOrHostAddressConstKHR {
                        device_address: rt_vertex_address,
                    })
                    .vertex_stride(VERTEX_BYTES as u64)
                    .max_vertex(max_vertex)
                    .index_type(vk::IndexType::UINT16)
                    .index_data(vk::DeviceOrHostAddressConstKHR {
                        device_address: rt_index_address + (*off as u64) * 2,
                    });
                geoms.push(
                    vk::AccelerationStructureGeometryKHR::default()
                        .geometry_type(vk::GeometryTypeKHR::TRIANGLES)
                        .geometry(vk::AccelerationStructureGeometryDataKHR { triangles })
                        .flags(vk::GeometryFlagsKHR::OPAQUE),
                );
                ranges.push(
                    vk::AccelerationStructureBuildRangeInfoKHR::default()
                        .primitive_count(cnt / 3)
                        .primitive_offset(0)
                        .first_vertex(0),
                );
            }
            // Query the hardware sizes, then pack all BLAS into one buffer.
            let mut sizes: Vec<vk::AccelerationStructureBuildSizesInfoKHR> =
                (0..geoms.len()).map(|_| Default::default()).collect();
            for (i, g) in geoms.iter().enumerate() {
                let build_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
                    .ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL)
                    .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
                    .flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
                    .geometries(std::slice::from_ref(g));
                rt_loader.get_acceleration_structure_build_sizes(
                    vk::AccelerationStructureBuildTypeKHR::DEVICE,
                    &build_info,
                    &[ranges[i].primitive_count],
                    &mut sizes[i],
                );
            }
            // Scratch regions must not overlap within one
            // cmd_build_acceleration_structures call (`VUID-...-scratchData-03704`),
            // so give each node its own 256-aligned slot in a summed buffer.
            let scratch_bytes = sizes
                .iter()
                .fold(0u64, |off, s| align_256(off + s.build_scratch_size));
            let mut scratch_offsets: Vec<u64> = Vec::with_capacity(sizes.len());
            let mut scratch_off = 0u64;
            for s in &sizes {
                scratch_offsets.push(scratch_off);
                scratch_off = align_256(scratch_off + s.build_scratch_size);
            }
            let mut as_offset = 0u64;
            for s in &sizes {
                as_offset = align_256(as_offset + s.acceleration_structure_size);
            }
            let total_as = as_offset;
            let (abuf, amem) = upload(
                total_as,
                vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR
                    | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
            );
            let (sbuf, smem) = upload(
                scratch_bytes,
                vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
            );
            let as_address = device
                .get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(abuf));
            let scratch_address = device
                .get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(sbuf));
            let mut as_offset = 0u64;
            for (i, s) in sizes.iter().enumerate() {
                let ci = vk::AccelerationStructureCreateInfoKHR::default()
                    .create_flags(vk::AccelerationStructureCreateFlagsKHR::empty())
                    .buffer(abuf)
                    .offset(as_offset)
                    .size(s.acceleration_structure_size)
                    .ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL);
                rt_blas.push(rt_loader.create_acceleration_structure(&ci, None).expect("blas"));
                rt_blas_addresses.push(as_address + as_offset);
                as_offset = align_256(as_offset + s.acceleration_structure_size);
                let _ = i;
            }
            // Build every BLAS in one command.
            let rt_pool_info = vk::CommandPoolCreateInfo::default()
                .queue_family_index(queue_family)
                .flags(vk::CommandPoolCreateFlags::TRANSIENT);
            let rt_pool = device.create_command_pool(&rt_pool_info, None).expect("rtpool2");
            let rt_alloc = vk::CommandBufferAllocateInfo::default()
                .command_pool(rt_pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1);
            let rt_cmd = device.allocate_command_buffers(&rt_alloc).expect("rtcmd2")[0];
            let rt_begin = vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            device.begin_command_buffer(rt_cmd, &rt_begin).expect("rtcbegin2");
            let mut build_infos: Vec<vk::AccelerationStructureBuildGeometryInfoKHR> = Vec::with_capacity(geoms.len());
            let mut build_ranges: Vec<Vec<vk::AccelerationStructureBuildRangeInfoKHR>> = Vec::with_capacity(geoms.len());
            for (i, g) in geoms.iter().enumerate() {
                build_infos.push(
                    vk::AccelerationStructureBuildGeometryInfoKHR::default()
                        .ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL)
                        .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
                        .flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
                        .geometries(std::slice::from_ref(g))
                        .dst_acceleration_structure(rt_blas[i])
                        .scratch_data(vk::DeviceOrHostAddressKHR {
                            device_address: scratch_address + scratch_offsets[i],
                        }),
                );
                build_ranges.push(vec![ranges[i]]);
            }
            let build_range_refs: Vec<&[vk::AccelerationStructureBuildRangeInfoKHR]> =
                build_ranges.iter().map(|v| v.as_slice()).collect();
            rt_loader.cmd_build_acceleration_structures(rt_cmd, &build_infos, &build_range_refs);
            let build_barrier = vk::MemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_WRITE_KHR)
                .dst_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_READ_KHR);
            device.cmd_pipeline_barrier(
                rt_cmd,
                vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
                vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
                vk::DependencyFlags::empty(),
                &[build_barrier],
                &[],
                &[],
            );
            device.end_command_buffer(rt_cmd).expect("rtcend2");
            let rt_fence = device.create_fence(&rt_fence_info, None).expect("rtfence2");
            let rt_cmd_ref = [rt_cmd];
            let rt_submit = vk::SubmitInfo::default().command_buffers(&rt_cmd_ref);
            device.queue_submit(queue, &[rt_submit], rt_fence).expect("rtsubmit2");
            device.wait_for_fences(&[rt_fence], true, u64::MAX).expect("rtfwait2");
            device.destroy_fence(rt_fence, None);
            device.destroy_command_pool(rt_pool, None);
            rt_blas_buffer = abuf;
            rt_blas_memory = amem;
            rt_blas_scratch_buffer = sbuf;
            rt_blas_scratch_memory = smem;
            println!(
                "RT: {} node BLAS, {:.1} KiB casters, {:.1} KiB structures",
                rt_blas.len(),
                rt_index_bytes as f32 / 1024.0,
                total_as as f32 / 1024.0,
            );
        }

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
        let lut_index = super::find_memory_type(
            &mem_props,
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
        let lut_stage_index = super::find_memory_type(
            &mem_props,
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
            vk::DescriptorSetLayoutBinding::default()
                .binding(3)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(4)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(5)
                .descriptor_type(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR)
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
        let ibl_samples = std::env::var("EXPLORA_IBL_SAMPLES")
            .map(|s| s.parse::<u32>().expect("invalid EXPLORA_IBL_SAMPLES"))
            .unwrap_or(4);
        println!("material IBL: {ibl_samples} VNDF samples/lobe");
        // Offline SPIR-V from build.rs (shaderc). One module per stage.
        let mk_module = |words: &[u32]| {
            device
                .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(words), None)
                .expect("pmodule")
        };
        let plane_vert_words = crate::spv_words(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/plane.vert.spv"
        )));
        let plane_frag_words = plane_frag_spv(ibl_samples);
        let plane_frag_rt_words = plane_frag_spv_rt(ibl_samples);
        let sky_vert_words = crate::spv_words(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/sky.vert.spv"
        )));
        let ground_vert_words = crate::spv_words(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/ground.vert.spv"
        )));
        let sky_frag_words = crate::spv_words(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/sky.frag.spv"
        )));
        let ground_frag_words = crate::spv_words(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/ground.frag.spv"
        )));
        let ground_frag_rt_words = ground_frag_spv_rt();
        let depth_frag_words = crate::spv_words(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/depth.frag.spv"
        )));
        let cloud_frag_words = crate::spv_words(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/clouds.frag.spv"
        )));
        let plane_vert = mk_module(&plane_vert_words);
        let plane_frag = mk_module(if rt_supported { &plane_frag_rt_words } else { &plane_frag_words });
        let sky_vert = mk_module(&sky_vert_words);
        let ground_vert = mk_module(&ground_vert_words);
        let sky_frag = mk_module(&sky_frag_words);
        let ground_frag = mk_module(if rt_supported { &ground_frag_rt_words } else { &ground_frag_words });
        let depth_frag = mk_module(&depth_frag_words);
        let cloud_frag = mk_module(&cloud_frag_words);
        let main_entry = c"main";
        let stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(plane_vert)
                .name(main_entry),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(plane_frag)
                .name(main_entry),
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
            .src_color_blend_factor(vk::BlendFactor::ONE)
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
        // Null pipeline for never-presented intermediate passes: full vertex
        // stage + rasterization of the animated airframe, zero attachments,
        // nothing stored (depth.frag is empty).
        let void_stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(plane_vert)
                .name(main_entry),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(depth_frag)
                .name(main_entry),
        ];
        let depth_off = vk::PipelineDepthStencilStateCreateInfo::default();
        let mut rendering_void = vk::PipelineRenderingCreateInfo::default();
        let void_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&void_stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&raster)
            .multisample_state(&multisample)
            .depth_stencil_state(&depth_off)
            .dynamic_state(&dynamic_state)
            .layout(layout)
            .push_next(&mut rendering_void);

        let sky_stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(sky_vert)
                .name(main_entry),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(sky_frag)
                .name(main_entry),
        ];
        let sky_vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
        let sky_depth = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(true)
            .depth_write_enable(false)
            .depth_compare_op(vk::CompareOp::LESS_OR_EQUAL);
        let mut rendering_sky = vk::PipelineRenderingCreateInfo::default()
            .color_attachment_formats(&formats)
            .depth_attachment_format(vk::Format::D32_SFLOAT);
        let mut sky_info = vk::GraphicsPipelineCreateInfo::default()
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
        let mut sky_rate = vk::PipelineFragmentShadingRateStateCreateInfoKHR::default()
            .fragment_size(vk::Extent2D { width: 4, height: 4 })
            .combiner_ops([
                vk::FragmentShadingRateCombinerOpKHR::KEEP,
                vk::FragmentShadingRateCombinerOpKHR::KEEP,
            ]);
        if ground_fsr {
            // 4x4 coarse rate cuts 94% of transcendental view-ray marching
            // while preserving smooth planetary sky gradients.
            sky_info = sky_info.push_next(&mut sky_rate);
        }
        let ground_stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(ground_vert)
                .name(main_entry),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(ground_frag)
                .name(main_entry),
        ];
        let ground_depth = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(true)
            .depth_write_enable(true)
            .depth_compare_op(vk::CompareOp::LESS_OR_EQUAL);
        let mut rendering_ground = vk::PipelineRenderingCreateInfo::default()
            .color_attachment_formats(&formats)
            .depth_attachment_format(vk::Format::D32_SFLOAT);
        let mut ground_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&ground_stages)
            .vertex_input_state(&sky_vertex_input)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&raster)
            .multisample_state(&multisample)
            .depth_stencil_state(&ground_depth)
            .color_blend_state(&blend_off_state)
            .dynamic_state(&dynamic_state)
            .layout(layout)
            .push_next(&mut rendering_ground);
        let mut ground_rate = vk::PipelineFragmentShadingRateStateCreateInfoKHR::default()
            .fragment_size(vk::Extent2D { width: 2, height: 2 })
            .combiner_ops([
                vk::FragmentShadingRateCombinerOpKHR::KEEP,
                vk::FragmentShadingRateCombinerOpKHR::KEEP,
            ]);
        if ground_fsr {
            ground_info = ground_info.push_next(&mut ground_rate);
        }
        // Volumetric clouds: fullscreen pass after ground, uses sky.vert.
        // Premultiplied alpha blend so partially transparent clouds composite
        // correctly over opaque scene content. Depth write lets clouds sort
        // against the aircraft. FSR 2×2 matches the ground rate.
        let cloud_stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(sky_vert)
                .name(main_entry),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(cloud_frag)
                .name(main_entry),
        ];
        let cloud_depth = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(true)
            .depth_write_enable(true)
            .depth_compare_op(vk::CompareOp::LESS_OR_EQUAL);
        let mut rendering_cloud = vk::PipelineRenderingCreateInfo::default()
            .color_attachment_formats(&formats)
            .depth_attachment_format(vk::Format::D32_SFLOAT);
        let mut cloud_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&cloud_stages)
            .vertex_input_state(&sky_vertex_input)
            .input_assembly_state(&input_assembly)
            .viewport_state(&viewport_state)
            .rasterization_state(&raster)
            .multisample_state(&multisample)
            .depth_stencil_state(&cloud_depth)
            .color_blend_state(&blend_on_state)
            .dynamic_state(&dynamic_state)
            .layout(layout)
            .push_next(&mut rendering_cloud);
        let mut cloud_rate = vk::PipelineFragmentShadingRateStateCreateInfoKHR::default()
            .fragment_size(vk::Extent2D { width: 4, height: 4 })
            .combiner_ops([
                vk::FragmentShadingRateCombinerOpKHR::KEEP,
                vk::FragmentShadingRateCombinerOpKHR::KEEP,
            ]);
        if ground_fsr {
            cloud_info = cloud_info.push_next(&mut cloud_rate);
        }
        let pipelines = device
            .create_graphics_pipelines(
                vk::PipelineCache::null(),
                &[opaque_info, glass_info, sky_info, ground_info, void_info, cloud_info],
                None,
            )
            .expect("ppipes");
        device.destroy_shader_module(plane_vert, None);
        device.destroy_shader_module(plane_frag, None);
        device.destroy_shader_module(sky_vert, None);
        device.destroy_shader_module(ground_vert, None);
        device.destroy_shader_module(sky_frag, None);
        device.destroy_shader_module(ground_frag, None);
        device.destroy_shader_module(cloud_frag, None);
        device.destroy_shader_module(depth_frag, None);
        // FX noise volumes: Nubis-style tileable Perlin-Worley generated on
        // CPU once at boot (see noise.rs). R8G8B8A8_UNORM data, not sRGB.
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
            let idx = super::find_memory_type(
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
            let sidx = super::find_memory_type(
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
            let idx = super::find_memory_type(
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
            let sidx = super::find_memory_type(
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
        // the HDR target switch.
        let fx_bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(2)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(3)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(4)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(5)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
        ];
        let fx_layout = device
            .create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&fx_bindings),
                None,
            )
            .expect("fxdsl");
        // FX pipelines: plume volume raymarch + trail ribbons share the airframe
        // UBO (group 0) plus noise volumes (group 1). Composite samples HDR.
        let fx_layouts = [set_layout, fx_layout];
        let fx_pipeline_layout_info =
            vk::PipelineLayoutCreateInfo::default().set_layouts(&fx_layouts);
        let fx_pipeline_layout = device
            .create_pipeline_layout(&fx_pipeline_layout_info, None)
            .expect("fxplayout");
        let comp_bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
        ];
        let composite_set_layout = device
            .create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::default().bindings(&comp_bindings),
                None,
            )
            .expect("compdsl");
        let comp_layouts = [set_layout, composite_set_layout];
        let composite_layout_info =
            vk::PipelineLayoutCreateInfo::default().set_layouts(&comp_layouts);
        let composite_layout = device
            .create_pipeline_layout(&composite_layout_info, None)
            .expect("complayout");
        let plume_vert_words = crate::spv_words(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/plume.vert.spv"
        )));
        let plume_frag_words = crate::spv_words(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/plume.frag.spv"
        )));
        let trail_vert_words = crate::spv_words(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/trail.vert.spv"
        )));
        let trail_frag_words = crate::spv_words(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/trail.frag.spv"
        )));
        let comp_vert_words = crate::spv_words(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/composite.vert.spv"
        )));
        let comp_frag_words = crate::spv_words(include_bytes!(concat!(
            env!("OUT_DIR"),
            "/composite.frag.spv"
        )));
        let mk_module = |words: &[u32]| {
            device
                .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(words), None)
                .expect("fxmodule")
        };
        let plume_vert = mk_module(&plume_vert_words);
        let plume_frag = mk_module(&plume_frag_words);
        let trail_vert = mk_module(&trail_vert_words);
        let trail_frag = mk_module(&trail_frag_words);
        let comp_vert = mk_module(&comp_vert_words);
        let comp_frag = mk_module(&comp_frag_words);
        // HDR linear target format for all FX color attachments.
        // Matches the Gfx HDR targets: packed 32-bit float, no alpha.
        let hdr_format = vk::Format::B10G11R11_UFLOAT_PACK32;
        let hdr_formats = [hdr_format];
        let swap_formats = [format];
        let mut rendering_hdr_plume = vk::PipelineRenderingCreateInfo::default()
            .color_attachment_formats(&hdr_formats)
            .depth_attachment_format(vk::Format::D32_SFLOAT);
        let mut rendering_hdr_trail = vk::PipelineRenderingCreateInfo::default()
            .color_attachment_formats(&hdr_formats)
            .depth_attachment_format(vk::Format::D32_SFLOAT);
        let mut rendering_swap = vk::PipelineRenderingCreateInfo::default()
            .color_attachment_formats(&swap_formats);
        let plume_bind = [vk::VertexInputBindingDescription::default()
            .binding(0)
            .stride(super::fx_gpu::PLUME_VERT_BYTES as u32)
            .input_rate(vk::VertexInputRate::VERTEX)];
        let plume_attrs = [
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(0)
                .format(vk::Format::R32G32B32_SFLOAT)
                .offset(0),
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(1)
                .format(vk::Format::R32_SFLOAT)
                .offset(12),
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(2)
                .format(vk::Format::R32_SFLOAT)
                .offset(16),
        ];
        let trail_bind = [vk::VertexInputBindingDescription::default()
            .binding(0)
            .stride(super::fx_gpu::TRAIL_VERT_BYTES as u32)
            .input_rate(vk::VertexInputRate::VERTEX)];
        let trail_attrs = [
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
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(2)
                .format(vk::Format::R32_SFLOAT)
                .offset(24),
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(3)
                .format(vk::Format::R32_SFLOAT)
                .offset(28),
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(4)
                .format(vk::Format::R32G32_SFLOAT)
                .offset(32),
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(5)
                .format(vk::Format::R32_SFLOAT)
                .offset(40),
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(6)
                .format(vk::Format::R32_SFLOAT)
                .offset(44),
            vk::VertexInputAttributeDescription::default()
                .binding(0)
                .location(7)
                .format(vk::Format::R32_SFLOAT)
                .offset(48),
        ];
        let plume_vi = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&plume_bind)
            .vertex_attribute_descriptions(&plume_attrs);
        let trail_vi = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&trail_bind)
            .vertex_attribute_descriptions(&trail_attrs);
        let empty_vi = vk::PipelineVertexInputStateCreateInfo::default();
        let fx_assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
        let fx_viewport = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let fx_raster = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .line_width(1.0);
        let fx_ms = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        // Depth test on, write off: gas never occludes, always occluded.
        let fx_depth = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(true)
            .depth_write_enable(false)
            .depth_compare_op(vk::CompareOp::LESS);
        let no_depth = vk::PipelineDepthStencilStateCreateInfo::default();
        let alpha_blend = [vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .alpha_blend_op(vk::BlendOp::ADD)
            .color_write_mask(vk::ColorComponentFlags::RGBA)];
        let plume_blend = [vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::ONE)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ONE)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .alpha_blend_op(vk::BlendOp::ADD)
            .color_write_mask(vk::ColorComponentFlags::RGBA)];
        let no_blend = [vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(false)
            .color_write_mask(vk::ColorComponentFlags::RGBA)];
        let alpha_state = vk::PipelineColorBlendStateCreateInfo::default().attachments(&alpha_blend);
        let plume_state = vk::PipelineColorBlendStateCreateInfo::default().attachments(&plume_blend);
        let opaque_state = vk::PipelineColorBlendStateCreateInfo::default().attachments(&no_blend);
        let fx_dynamic = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let fx_dyn_state =
            vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&fx_dynamic);
        let plume_stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(plume_vert)
                .name(main_entry),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(plume_frag)
                .name(main_entry),
        ];
        let trail_stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(trail_vert)
                .name(main_entry),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(trail_frag)
                .name(main_entry),
        ];
        let comp_stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(comp_vert)
                .name(main_entry),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(comp_frag)
                .name(main_entry),
        ];
        let plume_raster = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::FRONT)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .line_width(1.0);
        // Plume volume raymarch: front faces are culled so the camera can penetrate the cone
        // without near-plane clipping holes. Disabling depth testing ensures the volume raymarch
        // renders in front of the engine nozzle rather than having its back-face fragments culled
        // by the nozzle's pre-existing depth.
        let plume_depth = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(false)
            .depth_write_enable(false);
        let mut plume_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&plume_stages)
            .vertex_input_state(&plume_vi)
            .input_assembly_state(&fx_assembly)
            .viewport_state(&fx_viewport)
            .rasterization_state(&plume_raster)
            .multisample_state(&fx_ms)
            .depth_stencil_state(&plume_depth)
            .color_blend_state(&plume_state)
            .dynamic_state(&fx_dyn_state)
            .layout(fx_pipeline_layout)
            .push_next(&mut rendering_hdr_plume);
        let mut plume_rate = vk::PipelineFragmentShadingRateStateCreateInfoKHR::default()
            .fragment_size(vk::Extent2D { width: 2, height: 2 })
            .combiner_ops([
                vk::FragmentShadingRateCombinerOpKHR::KEEP,
                vk::FragmentShadingRateCombinerOpKHR::KEEP,
            ]);
        if ground_fsr {
            // Plume is alpha-blended and depth-test-only, so 2x2 preserves
            // scene occlusion while avoiding coarse full-rate edges.
            plume_info = plume_info.push_next(&mut plume_rate);
        }
        let trail_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&trail_stages)
            .vertex_input_state(&trail_vi)
            .input_assembly_state(&fx_assembly)
            .viewport_state(&fx_viewport)
            .rasterization_state(&fx_raster)
            .multisample_state(&fx_ms)
            .depth_stencil_state(&fx_depth)
            .color_blend_state(&alpha_state)
            .dynamic_state(&fx_dyn_state)
            .layout(fx_pipeline_layout)
            .push_next(&mut rendering_hdr_trail);
        let mut comp_info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&comp_stages)
            .vertex_input_state(&empty_vi)
            .input_assembly_state(&fx_assembly)
            .viewport_state(&fx_viewport)
            .rasterization_state(&fx_raster)
            .multisample_state(&fx_ms)
            .depth_stencil_state(&no_depth)
            .color_blend_state(&opaque_state)
            .dynamic_state(&fx_dyn_state)
            .layout(composite_layout)
            .push_next(&mut rendering_swap);
        let mut comp_rate = vk::PipelineFragmentShadingRateStateCreateInfoKHR::default()
            .fragment_size(vk::Extent2D { width: 2, height: 2 })
            .combiner_ops([
                vk::FragmentShadingRateCombinerOpKHR::KEEP,
                vk::FragmentShadingRateCombinerOpKHR::KEEP,
            ]);
        if ground_fsr {
            comp_info = comp_info.push_next(&mut comp_rate);
        }
        let fx_pipes = device
            .create_graphics_pipelines(
                vk::PipelineCache::null(),
                &[plume_info, trail_info, comp_info],
                None,
            )
            .expect("fxpipes");
        device.destroy_shader_module(plume_vert, None);
        device.destroy_shader_module(plume_frag, None);
        device.destroy_shader_module(trail_vert, None);
        device.destroy_shader_module(trail_frag, None);
        device.destroy_shader_module(comp_vert, None);
        device.destroy_shader_module(comp_frag, None);
        println!("fx pipelines: plume + trail + composite ready");
        // Unit volume bounds cached once; update() scales it to nozzle state per frame.
        let (cone_verts, _) = super::fx_gpu::build_plume_cone(1.0, 1.0);
        let unit_cone: Vec<[f32; 5]> = cone_verts
            .iter()
            .map(|v| [v.pos[0], v.pos[1], v.pos[2], v.axial, v.radial])
            .collect();
        let query_info = vk::QueryPoolCreateInfo::default()
            .query_type(vk::QueryType::TIMESTAMP)
            .query_count(8);
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
            lut_view,
            lut_sampler,
            weave_image,
            weave_memory,
            opaque_pipeline: pipelines[0],
            glass_pipeline: pipelines[1],
            sky_pipeline: pipelines[2],
            ground_pipeline: pipelines[3],
            cloud_pipeline: pipelines[5],
            void_pipeline: pipelines[4],
            layout,
            fx_layout,
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
            plume_pipeline: fx_pipes[0],
            trail_pipeline: fx_pipes[1],
            composite_pipeline: fx_pipes[2],
            fx_pipeline_layout,
            composite_layout,
            composite_set_layout,
            fx_pool: vk::DescriptorPool::null(),
            fx_sets: Vec::new(),
            cone_buffers: Vec::new(),
            cone_memories: Vec::new(),
            cone_mapped: Vec::new(),
            trail_buffers: Vec::new(),
            trail_memories: Vec::new(),
            trail_mapped: Vec::new(),
            trail_origin: Vec::new(),
            trail_filled: Vec::new(),
            cone_ibos: Vec::new(),
            cone_ibo_mems: Vec::new(),
            trail_ibos: Vec::new(),
            trail_ibo_mems: Vec::new(),
            trail_index_count: 0,
            unit_cone,
            query_pool,
            ubo_buffers: Vec::new(),
            ubo_memories: Vec::new(),
            ubo_mapped: Vec::new(),
            ubo_sets: Vec::new(),
            image_count: 0,
            samples,
            rt_supported,
            rt_loader,
            rt_vertex_address,
            rt_index_buffer,
            rt_index_memory,
            rt_index_address,
            rt_blas,
            rt_blas_addresses,
            rt_geom_nodes,
            rt_blas_buffer,
            rt_blas_memory,
            rt_blas_scratch_buffer,
            rt_blas_scratch_memory,
            rt_instance_buffers: Vec::new(),
            rt_instance_memories: Vec::new(),
            rt_instance_mapped: Vec::new(),
            rt_tlas: Vec::new(),
            rt_tlas_buffers: Vec::new(),
            rt_tlas_memories: Vec::new(),
            rt_scratch: Vec::new(),
            rt_scratch_memories: Vec::new(),
            rt_instance_count,
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
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR)
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
        let upload = |size: u64, usage: vk::BufferUsageFlags| {
            let info = vk::BufferCreateInfo::default()
                .size(size)
                .usage(usage)
                .sharing_mode(vk::SharingMode::EXCLUSIVE);
            let buffer = device.create_buffer(&info, None).expect("fbuf");
            let req = device.get_buffer_memory_requirements(buffer);
            let index = super::find_memory_type(
                &mem_props,
                req.memory_type_bits,
                vk::MemoryPropertyFlags::DEVICE_LOCAL,
            );
            let alloc = vk::MemoryAllocateInfo::default()
                .allocation_size(req.size)
                .memory_type_index(index);
            let memory = if usage.contains(vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS) {
                let mut addr_flags = vk::MemoryAllocateFlagsInfo::default()
                    .flags(vk::MemoryAllocateFlags::DEVICE_ADDRESS);
                device
                    .allocate_memory(&alloc.push_next(&mut addr_flags), None)
                    .expect("fmem")
            } else {
                device.allocate_memory(&alloc, None).expect("fmem")
            };
            device.bind_buffer_memory(buffer, memory, 0).expect("fbind");
            (buffer, memory)
        };
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
            let lut_ref = [vk::DescriptorImageInfo::default()
                .image_view(self.lut_view)
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
            let write_lut = [vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(3)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(&lut_ref)];
            let lut_sampler_ref =
                [vk::DescriptorImageInfo::default().sampler(self.lut_sampler)];
            let write_lut_smp = [vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(4)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .image_info(&lut_sampler_ref)];
            device.update_descriptor_sets(&write_ubo, &[]);
            device.update_descriptor_sets(&write_tex, &[]);
            device.update_descriptor_sets(&write_smp, &[]);
            device.update_descriptor_sets(&write_lut, &[]);
            device.update_descriptor_sets(&write_lut_smp, &[]);
            // Ray-traced shadows: per-slot top-level structure + host-written
            // instance transforms. Built each measured pass in record().
            if self.rt_supported && self.rt_instance_count > 0 {
                let inst_bytes = (self.rt_instance_count as usize) * RT_INSTANCE_BYTES;
                let inst_info = vk::BufferCreateInfo::default()
                    .size(inst_bytes as u64)
                    .usage(
                        vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
                            | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR,
                    )
                    .sharing_mode(vk::SharingMode::EXCLUSIVE);
                let inst_buffer = device.create_buffer(&inst_info, None).expect("rtinstbuf");
                let inst_req = device.get_buffer_memory_requirements(inst_buffer);
                let inst_index = super::find_memory_type(
                    &mem_props,
                    inst_req.memory_type_bits,
                    vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
                );
                let inst_alloc = vk::MemoryAllocateInfo::default()
                    .allocation_size(inst_req.size)
                    .memory_type_index(inst_index);
                let inst_memory = {
                    let mut addr_flags = vk::MemoryAllocateFlagsInfo::default()
                        .flags(vk::MemoryAllocateFlags::DEVICE_ADDRESS);
                    device
                        .allocate_memory(&inst_alloc.push_next(&mut addr_flags), None)
                        .expect("rtinstmem")
                };
                device.bind_buffer_memory(inst_buffer, inst_memory, 0).expect("rtinstbind");
                let inst_map = device
                    .map_memory(inst_memory, 0, inst_bytes as u64, vk::MemoryMapFlags::empty())
                    .expect("rtinstmap") as *mut u8;
                let inst_address = device
                    .get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(inst_buffer));
                let instances_geom = vk::AccelerationStructureGeometryInstancesDataKHR::default()
                    .array_of_pointers(false)
                    .data(vk::DeviceOrHostAddressConstKHR {
                        device_address: inst_address,
                    });
                let tlas_geometry = vk::AccelerationStructureGeometryKHR::default()
                    .geometry_type(vk::GeometryTypeKHR::INSTANCES)
                    .geometry(vk::AccelerationStructureGeometryDataKHR {
                        instances: instances_geom,
                    });
                let mut tlas_size = vk::AccelerationStructureBuildSizesInfoKHR::default();
                let tlas_build_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
                    .ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL)
                    .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
                    .flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
                    .geometries(std::slice::from_ref(&tlas_geometry));
                self.rt_loader.get_acceleration_structure_build_sizes(
                    vk::AccelerationStructureBuildTypeKHR::DEVICE,
                    &tlas_build_info,
                    &[self.rt_instance_count],
                    &mut tlas_size,
                );
                let (tlas_buffer, tlas_memory) = upload(
                    tlas_size.acceleration_structure_size,
                    vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR
                        | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
                );
                let (tlas_scratch, tlas_scratch_memory) = upload(
                    tlas_size.build_scratch_size,
                    vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
                );
                let tlas_create = vk::AccelerationStructureCreateInfoKHR::default()
                    .create_flags(vk::AccelerationStructureCreateFlagsKHR::empty())
                    .buffer(tlas_buffer)
                    .offset(0)
                    .size(tlas_size.acceleration_structure_size)
                    .ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL);
                let tlas = self
                    .rt_loader
                    .create_acceleration_structure(&tlas_create, None)
                    .expect("tlas");
                let tlas_ref = [tlas];
                let mut rt_as_write = vk::WriteDescriptorSetAccelerationStructureKHR::default()
                    .acceleration_structures(&tlas_ref);
                let write_rt_as = vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(5)
                    .descriptor_count(1)
                    .descriptor_type(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR)
                    .push_next(&mut rt_as_write);
                device.update_descriptor_sets(&[write_rt_as], &[]);
                self.rt_instance_buffers.push(inst_buffer);
                self.rt_instance_memories.push(inst_memory);
                self.rt_instance_mapped.push(inst_map);
                self.rt_tlas.push(tlas);
                self.rt_tlas_buffers.push(tlas_buffer);
                self.rt_tlas_memories.push(tlas_memory);
                self.rt_scratch.push(tlas_scratch);
                self.rt_scratch_memories.push(tlas_scratch_memory);
            } else {
                self.rt_instance_buffers.push(vk::Buffer::null());
                self.rt_instance_memories.push(vk::DeviceMemory::null());
                self.rt_instance_mapped.push(std::ptr::null_mut());
                self.rt_tlas.push(vk::AccelerationStructureKHR::null());
                self.rt_tlas_buffers.push(vk::Buffer::null());
                self.rt_tlas_memories.push(vk::DeviceMemory::null());
                self.rt_scratch.push(vk::Buffer::null());
                self.rt_scratch_memories.push(vk::DeviceMemory::null());
            }
            self.ubo_buffers.push(buffer);
            self.ubo_memories.push(memory);
            self.ubo_mapped.push(mapped);
            self.ubo_sets.push(set);
        }
        // FX sets (group 1): noise volumes + curl warp shared across frames.
        let fx_pool_sizes = [
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(images as u32 * 3),
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::SAMPLER)
                .descriptor_count(images as u32 * 3),
        ];
        self.fx_pool = device
            .create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .pool_sizes(&fx_pool_sizes)
                    .max_sets(images as u32),
                None,
            )
            .expect("fxpool");
        let fx_layouts = vec![self.fx_layout; images];
        self.fx_sets = device
            .allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(self.fx_pool)
                    .set_layouts(&fx_layouts),
            )
            .expect("fxsets");
        for set in &self.fx_sets {
            let base_ref = [vk::DescriptorImageInfo::default()
                .image_view(self.noise_base_view)
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
            let base_smp = [vk::DescriptorImageInfo::default().sampler(self.noise_base_sampler)];
            let det_ref = [vk::DescriptorImageInfo::default()
                .image_view(self.noise_detail_view)
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
            let det_smp =
                [vk::DescriptorImageInfo::default().sampler(self.noise_detail_sampler)];
            let curl_ref = [vk::DescriptorImageInfo::default()
                .image_view(self.noise_curl_view)
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
            let curl_smp =
                [vk::DescriptorImageInfo::default().sampler(self.noise_curl_sampler)];
            device.update_descriptor_sets(
                &[vk::WriteDescriptorSet::default()
                    .dst_set(*set)
                    .dst_binding(0)
                    .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                    .image_info(&base_ref)],
                &[],
            );
            device.update_descriptor_sets(
                &[vk::WriteDescriptorSet::default()
                    .dst_set(*set)
                    .dst_binding(1)
                    .descriptor_type(vk::DescriptorType::SAMPLER)
                    .image_info(&base_smp)],
                &[],
            );
            device.update_descriptor_sets(
                &[vk::WriteDescriptorSet::default()
                    .dst_set(*set)
                    .dst_binding(2)
                    .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                    .image_info(&det_ref)],
                &[],
            );
            device.update_descriptor_sets(
                &[vk::WriteDescriptorSet::default()
                    .dst_set(*set)
                    .dst_binding(3)
                    .descriptor_type(vk::DescriptorType::SAMPLER)
                    .image_info(&det_smp)],
                &[],
            );
            device.update_descriptor_sets(
                &[vk::WriteDescriptorSet::default()
                    .dst_set(*set)
                    .dst_binding(4)
                    .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                    .image_info(&curl_ref)],
                &[],
            );
            device.update_descriptor_sets(
                &[vk::WriteDescriptorSet::default()
                    .dst_set(*set)
                    .dst_binding(5)
                    .descriptor_type(vk::DescriptorType::SAMPLER)
                    .image_info(&curl_smp)],
                &[],
            );
        }
        // Per-frame host-visible cone (4 KiB) and ribbon (256 KiB) buffers.
        // Written in update(), read by plume/trail draws in the same frame's
        // final pass only, so one buffer per swapchain image is race-free.
        for _ in 0..images {
            let mk_host = |size: u64| {
                let info = vk::BufferCreateInfo::default()
                    .size(size)
                    .usage(vk::BufferUsageFlags::VERTEX_BUFFER)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE);
                let buffer = device.create_buffer(&info, None).expect("fxvbo");
                let req = device.get_buffer_memory_requirements(buffer);
                let index = super::find_memory_type(
                    &mem_props,
                    req.memory_type_bits,
                    vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
                );
                let alloc = vk::MemoryAllocateInfo::default()
                    .allocation_size(req.size)
                    .memory_type_index(index);
                let memory = device.allocate_memory(&alloc, None).expect("fxvmem");
                device.bind_buffer_memory(buffer, memory, 0).expect("fxvbind");
                let mapped = device
                    .map_memory(memory, 0, size, vk::MemoryMapFlags::empty())
                    .expect("fxvmap") as *mut u8;
                (buffer, memory, mapped)
            };
            let (cb, cm, cmapped) = mk_host(4096);
            std::ptr::write_bytes(cmapped, 0, 4096);
            self.cone_buffers.push(cb);
            self.cone_memories.push(cm);
            self.cone_mapped.push(cmapped);
            let (tb, tm, tmapped) = mk_host(262144);
            std::ptr::write_bytes(tmapped, 0, 262144);
            self.trail_buffers.push(tb);
            self.trail_memories.push(tm);
            self.trail_mapped.push(tmapped);
            self.trail_origin.push(Vec3::ZERO);
            self.trail_filled.push(false);
            // Static index data, identical per slot. Host-visible for direct fill.
            let mk_index = |data: &[u16]| {
                let size = (data.len() * 2) as u64;
                let info = vk::BufferCreateInfo::default()
                    .size(size)
                    .usage(vk::BufferUsageFlags::INDEX_BUFFER)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE);
                let buffer = device.create_buffer(&info, None).expect("fxibo");
                let req = device.get_buffer_memory_requirements(buffer);
                let index = super::find_memory_type(
                    &mem_props,
                    req.memory_type_bits,
                    vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
                );
                let alloc = vk::MemoryAllocateInfo::default()
                    .allocation_size(req.size)
                    .memory_type_index(index);
                let memory = device.allocate_memory(&alloc, None).expect("fximem");
                device.bind_buffer_memory(buffer, memory, 0).expect("fxibind");
                let mapped = device
                    .map_memory(memory, 0, size, vk::MemoryMapFlags::empty())
                    .expect("fximap") as *mut u16;
                std::ptr::copy_nonoverlapping(data.as_ptr(), mapped, data.len());
                device.unmap_memory(memory);
                (buffer, memory)
            };
            let (_, cone_idx) = super::fx_gpu::build_plume_cone(1.0, 1.0);
            debug_assert_eq!(cone_idx.len() as u32, super::fx_gpu::CONE_INDEX_COUNT);
            let (cb_ibo, cm_ibo) = mk_index(&cone_idx);
            self.cone_ibos.push(cb_ibo);
            self.cone_ibo_mems.push(cm_ibo);
            let trail_idx = super::fx_gpu::build_trail_indices();
            let (tb_ibo, tm_ibo) = mk_index(&trail_idx);
            self.trail_ibos.push(tb_ibo);
            self.trail_ibo_mems.push(tm_ibo);
            self.trail_index_count = trail_idx.len() as u32;
        }
        self.image_count = images;
    }

    pub unsafe fn destroy_frames(&mut self, device: &ash::Device) {
        for memory in self.cone_ibo_mems.drain(..) {
            device.free_memory(memory, None);
        }
        for buffer in self.cone_ibos.drain(..) {
            device.destroy_buffer(buffer, None);
        }
        for memory in self.trail_ibo_mems.drain(..) {
            device.free_memory(memory, None);
        }
        for buffer in self.trail_ibos.drain(..) {
            device.destroy_buffer(buffer, None);
        }
        self.trail_index_count = 0;
        for memory in self.cone_memories.drain(..) {
            device.unmap_memory(memory);
            device.free_memory(memory, None);
        }
        for buffer in self.cone_buffers.drain(..) {
            device.destroy_buffer(buffer, None);
        }
        self.cone_mapped.clear();
        for memory in self.trail_memories.drain(..) {
            device.unmap_memory(memory);
            device.free_memory(memory, None);
        }
        for buffer in self.trail_buffers.drain(..) {
            device.destroy_buffer(buffer, None);
        }
        self.trail_mapped.clear();
        self.trail_origin.clear();
        self.trail_filled.clear();
        self.fx_sets.clear();
        if self.fx_pool != vk::DescriptorPool::null() {
            device.destroy_descriptor_pool(self.fx_pool, None);
            self.fx_pool = vk::DescriptorPool::null();
        }
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
        // Ray-traced shadow per-slot resources.
        for buffer in self.rt_scratch.drain(..) {
            if buffer != vk::Buffer::null() {
                device.destroy_buffer(buffer, None);
            }
        }
        for memory in self.rt_scratch_memories.drain(..) {
            if memory != vk::DeviceMemory::null() {
                device.free_memory(memory, None);
            }
        }
        for accel in self.rt_tlas.drain(..) {
            if accel != vk::AccelerationStructureKHR::null() {
                self.rt_loader.destroy_acceleration_structure(accel, None);
            }
        }
        for buffer in self.rt_tlas_buffers.drain(..) {
            if buffer != vk::Buffer::null() {
                device.destroy_buffer(buffer, None);
            }
        }
        for memory in self.rt_tlas_memories.drain(..) {
            if memory != vk::DeviceMemory::null() {
                device.free_memory(memory, None);
            }
        }
        for memory in self.rt_instance_memories.drain(..) {
            if memory != vk::DeviceMemory::null() {
                device.unmap_memory(memory);
                device.free_memory(memory, None);
            }
        }
        for buffer in self.rt_instance_buffers.drain(..) {
            if buffer != vk::Buffer::null() {
                device.destroy_buffer(buffer, None);
            }
        }
        self.rt_instance_mapped.clear();
        self.image_count = 0;
    }

    pub unsafe fn prepare_query_pool(&mut self, device: &ash::Device, image_count: usize) {
        device.destroy_query_pool(self.query_pool, None);
        let query_info = vk::QueryPoolCreateInfo::default()
            .query_type(vk::QueryType::TIMESTAMP)
            .query_count((image_count * 8) as u32);
        self.query_pool = device
            .create_query_pool(&query_info, None)
            .expect("query pool");
    }

    pub(crate) fn node_matrix(&self, node: usize) -> Mat4 {
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

    /// Absolute plane root matrix (origin at world zero) for emitter sim.
    pub fn model_abs(pose: &sim::flight::Pose) -> Mat4 {
        let rotation = Mat4::from_quat(pose.orientation);
        Mat4::from_translation(Vec3::new(pose.x, pose.y, pose.z)) * rotation
    }

    pub fn engine_spool(&self) -> f32 {
        self.anim.spool
    }

    /// Exit follows the actual animated petal tips, including their aperture.
    fn nozzle_exit(&self) -> (Vec3, f32) {
        let tips: [Vec3; 10] = std::array::from_fn(|i|
            self.node_matrix(11 + i).transform_point3(Vec3::new(0.0, 0.0, -0.63)));
        let center = tips.iter().copied().sum::<Vec3>() / 10.0;
        let radius = tips.iter().map(|p| p.distance(center)).sum::<f32>() / 10.0;
        (center, radius)
    }

    fn wing_emitter(&self, side: f32, speed: f32) -> Vec3 {
        let span: f32 = 1.0;
        let mut p = wing_point(side, span, 1.0);
        p.z = -p.z; // Same mesh-space mirror as Part::vert.
        let t = self.anim.time;
        let gust = (t * 5.1 - span * 3.0 + side).sin() * 0.65
            + (t * 8.3 - span * 5.0).sin() * 0.35;
        p.y += self.anim.bend * span * span
            + Anim::pressure(speed) * 0.022 * span.powi(3) * gust;
        p
    }

    /// World emitter positions + dirs for the 5 FX sources.
    /// Order matches effects::EMITTER_* : nozzle, tipL, tipR, flapL, flapR.
    /// Same node-transform path as the vertex shader so vapor starts on geometry.
    pub fn emitter_world(
        &self,
        pose: &sim::flight::Pose,
    ) -> ([Vec3; 5], [Vec3; 5]) {
        let model = Self::model_abs(pose);
        let fwd = (model * glam::Vec4::new(0.0, 0.0, 1.0, 0.0)).truncate().normalize_or_zero();
        let back = -fwd;
        // Locals in node space.
        let tip_l = self.wing_emitter(-1.0, pose.speed);
        let tip_r = self.wing_emitter(1.0, pose.speed);
        let flap_edge = |side: f32| {
            let span: f32 = 0.885;
            let p = wing_point(side, span, 1.0) - flap_pivot(side, 2);
            let gust = (self.anim.time * 5.1 - span * 3.0 + side).sin() * 0.65
                + (self.anim.time * 8.3 - span * 5.0).sin() * 0.35;
            Vec3::new(p.x, p.y + self.anim.bend * span * span
                + Anim::pressure(pose.speed) * 0.022 * span.powi(3) * gust, -p.z)
        };
        let (nozzle_local, _) = self.nozzle_exit();
        let p_noz = model * nozzle_local.extend(1.0);
        let p_tl = model * self.node_matrix(2) * glam::Vec4::new(tip_l.x, tip_l.y, tip_l.z, 1.0);
        let p_tr = model * self.node_matrix(3) * glam::Vec4::new(tip_r.x, tip_r.y, tip_r.z, 1.0);
        // Trailing outboard corners, deformed before the animated flap transform.
        let p_fl = model * self.node_matrix(6) * flap_edge(-1.0).extend(1.0);
        let p_fr = model * self.node_matrix(9) * flap_edge(1.0).extend(1.0);
        let pos = [
            Vec3::new(p_noz.x, p_noz.y, p_noz.z) / p_noz.w.max(1e-6),
            Vec3::new(p_tl.x, p_tl.y, p_tl.z) / p_tl.w.max(1e-6),
            Vec3::new(p_tr.x, p_tr.y, p_tr.z) / p_tr.w.max(1e-6),
            Vec3::new(p_fl.x, p_fl.y, p_fl.z) / p_fl.w.max(1e-6),
            Vec3::new(p_fr.x, p_fr.y, p_fr.z) / p_fr.w.max(1e-6),
        ];
        (pos, [back; 5])
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
        hdr_image: vk::Image,
        hdr_view: vk::ImageView,
        comp_set: vk::DescriptorSet,
        scene_extent: vk::Extent2D,
        output_extent: vk::Extent2D,
        image_index: usize,
        query_base: u32,
        measure_gpu: bool,
    ) {
        let begin = vk::CommandBufferBeginInfo::default();
        device.begin_command_buffer(cmd, &begin).expect("pbegin");
        if measure_gpu {
            device.cmd_reset_query_pool(cmd, self.query_pool, query_base, 8);
        }
        let scene_viewport = vk::Viewport::default()
            .x(0.0)
            .y(0.0)
            .width(scene_extent.width as f32)
            .height(scene_extent.height as f32)
            .min_depth(0.0)
            .max_depth(1.0);
        let scene_scissor = vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: scene_extent,
        };
        let output_viewport = vk::Viewport::default()
            .x(0.0)
            .y(0.0)
            .width(output_extent.width as f32)
            .height(output_extent.height as f32)
            .min_depth(0.0)
            .max_depth(1.0);
        let output_scissor = vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: output_extent,
        };
        let set = self.ubo_sets[image_index];
        // Ray-traced shadows: rebuild this slot's top-level structure from the
        // host-written instance transforms. Only the measured pass traces.
        if self.rt_supported && self.rt_instance_count > 0 && measure_gpu {
            let inst_address = device.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default()
                .buffer(self.rt_instance_buffers[image_index]));
            let host_write = [vk::MemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::HOST_WRITE)
                .dst_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_READ_KHR)];
            device.cmd_pipeline_barrier(
                cmd,
                vk::PipelineStageFlags::HOST,
                vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
                vk::DependencyFlags::empty(),
                &host_write,
                &[],
                &[],
            );
            let instances_geom =
                vk::AccelerationStructureGeometryInstancesDataKHR::default()
                    .array_of_pointers(false)
                    .data(vk::DeviceOrHostAddressConstKHR {
                        device_address: inst_address,
                    });
            let tlas_geometry = vk::AccelerationStructureGeometryKHR::default()
                .geometry_type(vk::GeometryTypeKHR::INSTANCES)
                .geometry(vk::AccelerationStructureGeometryDataKHR {
                    instances: instances_geom,
                });
            let scratch_address = device.get_buffer_device_address(
                &vk::BufferDeviceAddressInfo::default().buffer(self.rt_scratch[image_index]),
            );
            let tlas_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
                .ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL)
                .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
                .flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
                .geometries(std::slice::from_ref(&tlas_geometry))
                .dst_acceleration_structure(self.rt_tlas[image_index])
                .scratch_data(vk::DeviceOrHostAddressKHR {
                    device_address: scratch_address,
                });
            let range = vk::AccelerationStructureBuildRangeInfoKHR::default()
                .primitive_count(self.rt_instance_count)
                .primitive_offset(0);
            self.rt_loader
                .cmd_build_acceleration_structures(cmd, &[tlas_info], &[&[range]]);
            let tlas_ready = [vk::MemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_WRITE_KHR)
                .dst_access_mask(vk::AccessFlags::ACCELERATION_STRUCTURE_READ_KHR)];
            device.cmd_pipeline_barrier(
                cmd,
                vk::PipelineStageFlags::ACCELERATION_STRUCTURE_BUILD_KHR,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::DependencyFlags::empty(),
                &tlas_ready,
                &[],
                &[],
            );
        }
        if !measure_gpu {
            // Intermediate pass: zero-attachment null rendering. The vertex
            // stage evaluates the animated airframe and the rasterizer
            // traverses every triangle, but nothing is stored and no
            // attachment layouts are touched, so passes need no barriers and
            // can overlap freely on the GPU timeline.
            let void_rendering = vk::RenderingInfo::default()
                .render_area(vk::Rect2D {
                    offset: vk::Offset2D { x: 0, y: 0 },
                    extent: scene_extent,
                })
                .layer_count(1);
            device.cmd_begin_rendering(cmd, &void_rendering);
            device.cmd_set_viewport(cmd, 0, &[scene_viewport]);
            device.cmd_set_scissor(cmd, 0, &[scene_scissor]);
            device.cmd_bind_vertex_buffers(cmd, 0, &[self.vertex_buffer], &[0]);
            device.cmd_bind_index_buffer(cmd, self.index_buffer, 0, vk::IndexType::UINT16);
            device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.void_pipeline);
            device.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.layout,
                0,
                &[set],
                &[],
            );
            device.cmd_draw_indexed(cmd, self.opaque_count, 1, 0, 0, 0);
            device.cmd_end_rendering(cmd);
            device.end_command_buffer(cmd).expect("pend");
            return;
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
        // HDR scene pass: opaque + sky + plume + trail + glass all compose in
        // linear HDR. The swapchain only sees the final composite triangle.
        let mut color_info = vk::RenderingAttachmentInfo::default()
            .image_view(if self.samples == vk::SampleCountFlags::TYPE_1 {
                hdr_view
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
                .resolve_image_view(hdr_view)
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
                extent: scene_extent,
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
        // This is the only attachment-writing pass of the burst: the void
        // passes never touch the images, so the layouts are established here
        // with discard transitions (depth clear is the presented pass's own).
        // HDR target carries the scene; the swapchain is touched only by the
        // trailing composite triangle.
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
                .image(hdr_image)
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
        device.cmd_set_viewport(cmd, 0, &[scene_viewport]);
        device.cmd_set_scissor(cmd, 0, &[scene_scissor]);
        device.cmd_bind_vertex_buffers(cmd, 0, &[self.vertex_buffer], &[0]);
        device.cmd_bind_index_buffer(cmd, self.index_buffer, 0, vk::IndexType::UINT16);
        device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.opaque_pipeline);
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
            // Per-pass GPU breakdown: q0 start, then one stamp per pass.
            let stamp = |device: &ash::Device, q: u32| {
                device.cmd_write_timestamp(
                    cmd,
                    vk::PipelineStageFlags::TOP_OF_PIPE,
                    self.query_pool,
                    query_base + q,
                );
            };
            stamp(device, 1);
            device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.sky_pipeline);
            device.cmd_draw(cmd, 6, 1, 0, 0);
            device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.ground_pipeline);
            device.cmd_draw(cmd, 6, 1, 0, 0);
            stamp(device, 2);
            // Volumetric clouds into HDR (premultiplied alpha blend, depth
            // write). Fullscreen quad, same descriptor set as sky/ground.
            device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.cloud_pipeline);
            device.cmd_draw(cmd, 6, 1, 0, 0);
            stamp(device, 3);
            // Plume cone raymarch into HDR (forward alpha blend, no depth write).
            let fx_set = self.fx_sets[image_index];
            device.cmd_bind_pipeline(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.plume_pipeline,
            );
            device.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.fx_pipeline_layout,
                0,
                &[set, fx_set],
                &[],
            );
            device.cmd_bind_vertex_buffers(cmd, 0, &[self.cone_buffers[image_index]], &[0]);
            device.cmd_bind_index_buffer(
                cmd,
                self.cone_ibos[image_index],
                0,
                vk::IndexType::UINT16,
            );
            device.cmd_draw_indexed(cmd, super::fx_gpu::CONE_INDEX_COUNT, 1, 0, 0, 0);
            stamp(device, 4);
            // Persistent ribbons into HDR. Fixed index range; unused verts are
            // zero density and discard in the fragment shader.
            device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.trail_pipeline);
            device.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.fx_pipeline_layout,
                0,
                &[set, fx_set],
                &[],
            );
            device.cmd_bind_vertex_buffers(cmd, 0, &[self.trail_buffers[image_index]], &[0]);
            device.cmd_bind_index_buffer(
                cmd,
                self.trail_ibos[image_index],
                0,
                vk::IndexType::UINT16,
            );
            device.cmd_draw_indexed(cmd, self.trail_index_count, 1, 0, 0, 0);
            stamp(device, 5);
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
            stamp(device, 6);
        }
        device.cmd_end_rendering(cmd);
        if measure_gpu {
            // HDR scene to shader-readable for the composite triangle.
            let hdr_to_read = vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                .dst_access_mask(vk::AccessFlags::SHADER_READ)
                .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .image(hdr_image)
                .subresource_range(color_range);
            // Swapchain discard transition; composite overwrites every pixel.
            let swap_to_draw = vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::empty())
                .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .image(image)
                .subresource_range(color_range);
            device.cmd_pipeline_barrier(
                cmd,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT
                    | vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[hdr_to_read, swap_to_draw],
            );
            let swap_clear = vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: [0.0, 0.0, 0.0, 1.0],
                },
            };
            let swap_color = vk::RenderingAttachmentInfo::default()
                .image_view(view)
                .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .load_op(vk::AttachmentLoadOp::DONT_CARE)
                .store_op(vk::AttachmentStoreOp::STORE)
                .clear_value(swap_clear);
            let swap_colors = [swap_color];
            let swap_rendering = vk::RenderingInfo::default()
                .render_area(vk::Rect2D {
                    offset: vk::Offset2D { x: 0, y: 0 },
                    extent: output_extent,
                })
                .layer_count(1)
                .color_attachments(&swap_colors);
            device.cmd_begin_rendering(cmd, &swap_rendering);
            device.cmd_set_viewport(cmd, 0, &[output_viewport]);
            device.cmd_set_scissor(cmd, 0, &[output_scissor]);
            device.cmd_bind_pipeline(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.composite_pipeline,
            );
            device.cmd_bind_descriptor_sets(
                cmd,
                vk::PipelineBindPoint::GRAPHICS,
                self.composite_layout,
                0,
                &[set, comp_set],
                &[],
            );
            device.cmd_draw(cmd, 6, 1, 0, 0);
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
            device.cmd_write_timestamp(
                cmd,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                self.query_pool,
                query_base + 7,
            );
        }
        device.end_command_buffer(cmd).expect("pend");
    }

    /// Advance physics-driven airframe animation states (wing bending, control flaps, rotor spin, and vectoring petals).
    pub fn step_animation(&mut self, u: &Controls, pose: &sim::flight::Pose, dt: f32) {
        let load = pose.load.clamp(-4.0, 3.5);
        self.anim.step(u, load, pose.boost, dt);
    }

    /// Update host-coherent UBO buffer memory with the latest view-projection, camera eye vector,
    /// 23 kinematic node transform matrices, and aeroelastic flex coefficients.
    pub unsafe fn update(
        &mut self,
        pose: &sim::flight::Pose,
        view_proj: &Mat4,
        origin: Vec3,
        eye_rel: Vec3,
        time: f32,
        image_index: usize,
        presented: bool,
        fx: &sim::effects::Effects,
        sim_stepped: bool,
        cam_frame: &sim::camera::CameraFrame,
    ) {
        let pressure = Anim::pressure(pose.speed);
        // Use the same body attitude as physics and world-space emitters.
        let rotation = Mat4::from_quat(pose.orientation);
        let rel = Vec3::new(pose.x, pose.y, pose.z) - origin;
        let model = Mat4::from_translation(rel) * rotation;
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
        // Per-node instance transforms for the ray-traced shadow TLAS.
        // Host-coherent: the measured pass builds the structure from these.
        if self.rt_supported
            && self.rt_instance_count > 0
            && presented
            && image_index < self.rt_instance_mapped.len()
        {
            let instances = self.rt_instance_mapped[image_index] as *mut vk::AccelerationStructureInstanceKHR;
            for (i, node) in self.rt_geom_nodes.iter().enumerate() {
                let m = model * self.node_matrix(*node as usize);
                let c = m.to_cols_array();
                let mask: u8 = if *node == 2 || (*node >= 4 && *node <= 6) {
                    0x02
                } else if *node == 3 || (*node >= 7 && *node <= 9) {
                    0x04
                } else {
                    0x01
                };
                let inst = vk::AccelerationStructureInstanceKHR {
                    transform: vk::TransformMatrixKHR {
                        matrix: [
                            c[0], c[4], c[8], c[12],
                            c[1], c[5], c[9], c[13],
                            c[2], c[6], c[10], c[14],
                        ],
                    },
                    instance_custom_index_and_mask: vk::Packed24_8::new(*node as u32, mask),
                    instance_shader_binding_table_record_offset_and_flags: vk::Packed24_8::new(
                        0,
                        vk::GeometryInstanceFlagsKHR::TRIANGLE_FACING_CULL_DISABLE.as_raw() as u8,
                    ),
                    acceleration_structure_reference: vk::AccelerationStructureReferenceKHR {
                        device_handle: self.rt_blas_addresses[i],
                    },
                };
                unsafe {
                    *instances.add(i) = inst;
                }
            }
        }
        let sun_radius = SUN_RADIUS;
        let sun_elevation = SUN_ELEVATION;
        let sun_dir = SUN_DIR;
        let sun_irr = SUN_IRRADIANCE;
        let flicker = 0.90 + 0.06 * (time * 57.0).sin() + 0.04 * (time * 91.0).sin();
        let glow = (0.8 + self.anim.spool * 2.2) * flicker;
        let zenith_sky = SKY_ZENITH;
        let horizon_haze = SKY_HORIZON;
        let ground_base = GROUND_BASE;
        let cos_radius = SUN_COS_RADIUS;
        let inv_one_minus_cos_radius = INV_ONE_MINUS_SUN_COS_RADIUS;
        // FX uniforms in spare tail slots.
        let spool = self.anim.spool;
        let lambda = fx.plume.cell_lambda;
        let (_, exit_radius) = self.nozzle_exit();
        // The ground is authored at world Y=0. Store it relative to the same
        // floating origin used by the camera and airframe.
        let ground_height = GROUND_LEVEL - origin.y;
        let ground_origin = ground_origin_pack(origin);
        let mut shift = Vec3::ZERO;
        if presented {
            self.fill_cone(image_index, fx, model, rotation);
            let filled = self.trail_filled.get(image_index).copied().unwrap_or(false);
            if sim_stepped || !filled {
                self.fill_trail(image_index, fx, origin, eye_rel);
                if image_index < self.trail_origin.len() {
                    self.trail_origin[image_index] = origin;
                    self.trail_filled[image_index] = true;
                }
            } else if image_index < self.trail_origin.len() {
                shift = self.trail_origin[image_index] - origin;
            }
        }

        let tail: [f32; 48] = [
            self.anim.bend,
            time,
            pressure,
            glow,
            campos.x,
            campos.y,
            campos.z,
            exit_radius,
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
            ground_height,
            // detail: shared plume flicker, shock-cell wavelength, spool, length.
            flicker,
            lambda,
            spool,
            fx.plume.length_m,
            // packed-origin minus current origin for trail.vert.
            shift.x,
            shift.y,
            shift.z,
            0.0,
            // Camera & optics parameters:
            cam_frame.fov_y,
            cam_frame.aspect,
            cam_frame.speed,
            cam_frame.load,
            cam_frame.shake_intensity,
            cam_frame.exposure,
            cam_frame.mach,
            0.0,
            // groundOrigin: floor(origin.xz / 0.25 m), then the positive
            // sub-cell remainder. ground.frag reconstructs stable global noise
            // cells from this split without large-coordinate cancellation.
            ground_origin[0],
            ground_origin[1],
            ground_origin[2],
            ground_origin[3],
        ];
        std::ptr::copy_nonoverlapping(tail.as_ptr(), dst.add(32 + NODE_COUNT * 16), 48);
    }

    /// Rewrite the host-visible volume bounds to nozzle state (relative to origin).
    /// Uses the exact same model and rotation transforms as the aircraft airframe and
    /// plume shaders, guaranteeing 1-to-1 sync during violent rolls, dives, and pitch maneuvers.
    unsafe fn fill_cone(
        &mut self,
        image_index: usize,
        fx: &sim::effects::Effects,
        model: Mat4,
        rotation: Mat4,
    ) {
        if image_index >= self.cone_mapped.len() {
            return;
        }
        let (nozzle_local, exit_radius) = self.nozzle_exit();
        let nozzle = (model * nozzle_local.extend(1.0)).truncate();
        let dir = (rotation * glam::Vec4::new(0.0, 0.0, -1.0, 0.0)).truncate().normalize_or_zero();
        let u = (rotation * glam::Vec4::new(1.0, 0.0, 0.0, 0.0)).truncate().normalize_or_zero();
        let v = (rotation * glam::Vec4::new(0.0, 1.0, 0.0, 0.0)).truncate().normalize_or_zero();
        // Conservative axial margin: the proxy extends 35% beyond nominal fluid
        // length so the downstream end cap and polygon edges remain in empty air
        // where fluid density and emission have already reached strictly zero.
        let len = fx.plume.length_m.max(0.5) * PLUME_AXIAL_BOUND;
        // The generated curl field has components bounded to +/-1.0, so
        // 1.35 widths conservatively contain every warped contributing ray.
        // Circumscribe that envelope at each z so the proxy cannot clip it.
        let radial_bound =
            PLUME_WARP_BOUND
                / (std::f32::consts::PI / super::fx_gpu::CONE_SEGMENTS as f32).cos();
        let spool = self.anim.spool;
        let dst = self.cone_mapped[image_index] as *mut f32;
        let n = self.unit_cone.len();
        for (i, uv) in self.unit_cone.iter().enumerate().take(n) {
            let z = -uv[2] * len;
            let width = (exit_radius * (1.0 + 0.12 * spool) + z * 0.055).max(0.05);
            let rad = width * radial_bound;
            let w = nozzle + u * (uv[0] * rad) + v * (uv[1] * rad) + dir * z;
            *dst.add(i * 5) = w.x;
            *dst.add(i * 5 + 1) = w.y;
            *dst.add(i * 5 + 2) = w.z;
            *dst.add(i * 5 + 3) = uv[3];
            *dst.add(i * 5 + 4) = uv[4];
        }
    }

    /// Pack newest live trail segments into the host-visible ribbon buffer.
    /// Scans back from each pool head (newest first), capped per emitter.
    unsafe fn fill_trail(
        &mut self,
        image_index: usize,
        fx: &sim::effects::Effects,
        origin: Vec3,
        eye_rel: Vec3,
    ) {
        use sim::effects::{EMITTER_COUNT, POOL_N};
        use super::fx_gpu::{ribbon_quad, TRAIL_MAX_QUADS_PER_EMITTER};
        if image_index >= self.trail_mapped.len() {
            return;
        }
        const SCAN_PER_EMITTER: usize = 1200;
        const MAX_VERTS: usize = EMITTER_COUNT * TRAIL_MAX_QUADS_PER_EMITTER * 2;
        let dst = self.trail_mapped[image_index] as *mut f32;
        // Fixed emitter slots must agree with build_trail_indices.
        std::ptr::write_bytes(dst, 0, MAX_VERTS * 13);
        for e in 0..EMITTER_COUNT {
            let pool = &fx.pools[e];
            let mut quads = 0usize;
            let mut prev_center = fx.emitters[e].pos - origin;
            let mut previous_side = Vec3::ZERO;
            let mut last = [[0.0; 13]; 2];
            // Dead pools stay short: never scan past what was ever pushed.
            let scan = SCAN_PER_EMITTER.min(pool.live);
            for k in 0..scan {
                if quads == TRAIL_MAX_QUADS_PER_EMITTER { break; }
                let idx = (pool.head + POOL_N - 1 - k) % POOL_N;
                let s = &pool.segs[idx];
                if s.density <= 0.0 || s.age >= s.life { continue; }
                let center = s.pos - origin;
                let older = &pool.segs[(idx + POOL_N - 1) % POOL_N];
                let tangent = if older.density > 0.0 && older.age < older.life {
                    older.pos - (if quads == 0 { fx.emitters[e].pos } else { prev_center + origin })
                } else { center - prev_center };
                let cam_dir = (eye_rel - center).normalize_or_zero();
                let formation = if e == 0 { 12.0 } else { 3.0 } / fx.speed_ms.max(20.0);
                let fade = ((s.age - formation) / formation).clamp(0.0, 1.0);
                let fade = fade * fade * (3.0 - 2.0 * fade);
                let tail = ((TRAIL_MAX_QUADS_PER_EMITTER - 1 - quads) as f32 / 24.0).clamp(0.0, 1.0);
                let mut quad = ribbon_quad(
                    center, center - tangent, cam_dir, s.radius.max(0.025), s.age,
                    s.density * fade * tail, [s.flow_uv[0] * 2.0, 0.0], e as f32 * 0.173, s.ice,
                );
                // Keep the ribbon's two edges continuous when viewed along its axis.
                let side = Vec3::new(quad[0][3], quad[0][4], quad[0][5]);
                if side.dot(previous_side) < 0.0 {
                    for v in &mut quad { for c in &mut v[3..6] { *c = -*c; } }
                }
                previous_side = Vec3::new(quad[0][3], quad[0][4], quad[0][5]);
                let base = (e * TRAIL_MAX_QUADS_PER_EMITTER + quads) * 26;
                std::ptr::copy_nonoverlapping(quad[0].as_ptr(), dst.add(base), 13);
                std::ptr::copy_nonoverlapping(quad[1].as_ptr(), dst.add(base + 13), 13);
                last = quad;
                quads += 1;
                prev_center = center;
            }
            // Collapse the unused tail at the final point, never at world origin.
            if quads > 0 {
                for v in &mut last { v[7] = 0.0; }
                for q in quads..TRAIL_MAX_QUADS_PER_EMITTER {
                    let base = (e * TRAIL_MAX_QUADS_PER_EMITTER + q) * 26;
                    std::ptr::copy_nonoverlapping(last[0].as_ptr(), dst.add(base), 13);
                    std::ptr::copy_nonoverlapping(last[1].as_ptr(), dst.add(base + 13), 13);
                }
            }
        }
    }

    pub(crate) fn query_pool(&self) -> vk::QueryPool {
        self.query_pool
    }

    pub(crate) fn composite_set_layout(&self) -> vk::DescriptorSetLayout {
        self.composite_set_layout
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_quality_variants_compile() {
        for samples in [4, 8, 16, 32, 128] {
            assert!(!plane_frag_spv(samples).is_empty());
        }
    }

    #[test]
    fn test_energy_lut_anchors() {
        let lut = energy_lut();
        const N: usize = 32;
        assert!(lut[N - 1] > 0.97, "mirror must conserve energy");
        // Exact correlated Smith at alpha=1 and normal incidence:
        // directional albedo = 1 - ln(2), approximately 0.30685.
        assert!((lut[N * N - 1] - (1.0 - 2.0f32.ln())).abs() < 0.025);
        // At grazing, correlated masking tends to unit directional albedo.
        assert!(lut[(N - 1) * N] > 0.90);
        assert!((0.40..0.48).contains(&energy_avg(&lut, N - 1)));
        for value in lut {
            assert!(value.is_finite() && value > 0.0 && value <= 1.05);
        }
    }

    #[test]
    fn test_ubo_tail_and_bytes_alignment() {
        assert_eq!(UBO_BYTES, 1792);
        assert_eq!(UBO_BYTES % 16, 0);
        let matrix_floats = 16 + 16 + NODE_COUNT * 16;
        let tail_floats = 48;
        assert_eq!((matrix_floats + tail_floats) * std::mem::size_of::<f32>(), UBO_BYTES);
    }

    #[test]
    fn test_rt_instance_size() {
        use ash::vk::AccelerationStructureInstanceKHR;
        assert_eq!(std::mem::size_of::<AccelerationStructureInstanceKHR>(), 64);
        let inst = AccelerationStructureInstanceKHR {
            transform: ash::vk::TransformMatrixKHR { matrix: [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0] },
            instance_custom_index_and_mask: ash::vk::Packed24_8::new(0, 0xFF),
            instance_shader_binding_table_record_offset_and_flags: ash::vk::Packed24_8::new(0, 0),
            acceleration_structure_reference: ash::vk::AccelerationStructureReferenceKHR {
                device_handle: 12345678,
            },
        };
        let bytes: [u8; 64] = unsafe { std::mem::transmute(inst) };
        assert_eq!(bytes.len(), 64);
    }

    #[test]
    fn ground_origin_pack_preserves_sub_cell_phase() {
        let origin = Vec3::new(12345.67, 1500.0, -9876.54);
        let packed = ground_origin_pack(origin);
        assert!((0.0..GROUND_FINE_CELL).contains(&packed[2]));
        assert!((0.0..GROUND_FINE_CELL).contains(&packed[3]));
        assert!((packed[0] * GROUND_FINE_CELL + packed[2] - origin.x).abs() < 0.002);
        assert!((packed[1] * GROUND_FINE_CELL + packed[3] - origin.z).abs() < 0.002);
    }
}
