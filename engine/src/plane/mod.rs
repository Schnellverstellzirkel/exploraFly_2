// Glider renderer, merged pass. All opaque parts draw in one call,
// glass in a second. Per-vertex node and material ids replace the
// old per-part uniforms. One uniform block per image holds the
// view-projection, all node matrices, and the shared flex terms.
// Per-frame CPU work is 23 matrices plus one coherent copy.

use sim::flight::Controls;
use ash::khr;
use ash::vk;
use glam::{Mat4, Vec3};

use crate::anim::Anim;
use crate::ubo::*;

pub const VERTEX_BYTES: usize = 28;
pub(super) const TERRAIN_COMMAND_BYTES: usize = std::mem::size_of::<vk::DrawIndexedIndirectCommand>();
pub(super) const VEGETATION_COMMAND_BYTES: usize = std::mem::size_of::<vk::DrawIndirectCommand>();
pub(super) const VEGETATION_COMMAND_OFFSET: usize =
    UBO_BYTES + world::TERRAIN_CHUNK_COUNT as usize * TERRAIN_COMMAND_BYTES;
pub(super) const VEGETATION_COMMAND_COUNT: u32 = world::vegetation::VEGETATION_COMMAND_CAPACITY;
pub(super) const FRAME_BYTES: usize = VEGETATION_COMMAND_OFFSET
    + VEGETATION_COMMAND_COUNT as usize * VEGETATION_COMMAND_BYTES;
// Timestamps per measured frame: q0 start, then one stamp after each pass —
// opaque(+TLAS build), terrain, vegetation, clouds, sky, plume, trail, glass,
// composite.
pub const GPU_STAMPS_PER_FRAME: u32 = 10;
// VkAccelerationStructureInstanceKHR stride (transform 48 + 2 packed u32 +
// device reference 8 + 16 B padding to 16-byte instance alignment).
pub const RT_INSTANCE_BYTES: usize = std::mem::size_of::<vk::AccelerationStructureInstanceKHR>();

mod airframe_mesh;
mod build;
mod geometry;
mod descriptors;
mod frames;
mod fx_volumes;
mod terrain_cmds;
mod vegetation;
mod pipelines;
mod rt;
mod pipeline_cache;
mod spirv;
mod textures;
mod uniforms;


/// High-performance GPU renderer for the glider airframe.
///
/// Encapsulates merged single-pass vertex/index buffers, descriptor sets,
/// procedural sail cloth weave textures, uniform buffers, and dynamic rendering pipelines.
pub struct Plane {
    hud: [f32; 8],
    opaque_count: u32,
    glass_first: u32,
    glass_count: u32,
    vertex_buffer: vk::Buffer,
    #[allow(dead_code)]
    vertex_memory: vk::DeviceMemory,
    index_buffer: vk::Buffer,
    #[allow(dead_code)]
    index_memory: vk::DeviceMemory,
    terrain_index_offset: u64,
    cloud_index_offset: u64,
    #[allow(dead_code)]
    terrain_image: vk::Image,
    #[allow(dead_code)]
    terrain_memory: vk::DeviceMemory,
    terrain_view: vk::ImageView,
    terrain_draw_batch: u32,
    vegetation_draw_batch: u32,
    vegetation_buffer: vk::Buffer,
    #[allow(dead_code)]
    vegetation_memory: vk::DeviceMemory,
    vegetation_database: world::vegetation::VegetationDatabase,
    set_layout: vk::DescriptorSetLayout,
    descriptor_pool: vk::DescriptorPool,
    weave_view: vk::ImageView,
    weave_sampler: vk::Sampler,
    lut_view: vk::ImageView,
    lut_sampler: vk::Sampler,
    #[allow(dead_code)]
    atmo_transmittance_image: vk::Image,
    #[allow(dead_code)]
    atmo_transmittance_memory: vk::DeviceMemory,
    atmo_transmittance_view: vk::ImageView,
    #[allow(dead_code)]
    atmo_multiscattering_image: vk::Image,
    #[allow(dead_code)]
    atmo_multiscattering_memory: vk::DeviceMemory,
    atmo_multiscattering_view: vk::ImageView,
    atmo_sampler: vk::Sampler,
    detail_textures: crate::detail::DetailTextures,
    #[allow(dead_code)]
    weave_image: vk::Image,
    #[allow(dead_code)]
    weave_memory: vk::DeviceMemory,
    opaque_pipeline: vk::Pipeline,
    glass_pipeline: vk::Pipeline,
    sky_pipeline: vk::Pipeline,
    ground_pipeline: vk::Pipeline,
    vegetation_pipeline: vk::Pipeline,
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
    rt_terrain_vertex_buffer: vk::Buffer,
    #[allow(dead_code)]
    rt_terrain_vertex_memory: vk::DeviceMemory,
    rt_terrain_blas_address: vk::DeviceAddress,
    #[allow(dead_code)]
    rt_structures_vertex_buffer: vk::Buffer,
    #[allow(dead_code)]
    rt_structures_vertex_memory: vk::DeviceMemory,
    rt_structures_blas_address: vk::DeviceAddress,
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
    pub fn reset_flight(&mut self) { reset_flight_state(&mut self.anim, &mut self.trail_filled); }

    pub fn set_hud(&mut self, telemetry: [f32; 8]) { self.hud = telemetry; }

    pub(crate) fn node_matrix(&self, node: usize) -> Mat4 {
        self.anim.node_matrix(node)
    }

    /// Absolute plane root matrix (origin at world zero) for emitter sim.
    #[inline]
    #[allow(dead_code)]
    pub fn model_abs(pose: &sim::flight::Pose) -> Mat4 {
        Anim::model_abs(pose)
    }

    #[inline]
    pub fn engine_spool(&self) -> f32 {
        self.anim.spool
    }

    /// Exit follows the actual animated petal tips, including their aperture.
    #[inline]
    pub fn nozzle_exit(&self) -> (Vec3, f32) {
        self.anim.nozzle_exit()
    }

    #[inline]
    #[allow(dead_code)]
    pub fn wing_emitter(&self, side: f32, speed: f32) -> Vec3 {
        self.anim.wing_emitter(side, speed)
    }

    /// World emitter positions + dirs for the 5 FX sources.
    /// Order matches effects::EMITTER_* : nozzle, tipL, tipR, flapL, flapR.
    /// Same node-transform path as the vertex shader so vapor starts on geometry.
    #[inline]
    pub fn emitter_world(
        &self,
        pose: &sim::flight::Pose,
    ) -> ([Vec3; 5], [Vec3; 5]) {
        self.anim.emitter_world(pose)
    }


    /// Advance physics-driven airframe animation states (wing bending, control flaps, rotor spin, and vectoring petals).
    pub fn step_animation(&mut self, u: &Controls, pose: &sim::flight::Pose, dt: f32) {
        let load = pose.load.clamp(-4.0, 3.5);
        self.anim.step(u, load, pose.boost, dt);
    }

    pub(crate) fn query_pool(&self) -> vk::QueryPool {
        self.query_pool
    }

    pub(crate) fn composite_set_layout(&self) -> vk::DescriptorSetLayout {
        self.composite_set_layout
    }
}

fn reset_flight_state(anim: &mut Anim, trail_filled: &mut [bool]) {
    *anim = Anim::new();
    trail_filled.fill(false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_clears_animation_and_every_swapchain_trail_cache() {
        let mut anim = Anim::new();
        anim.spool = 1.0;
        anim.time = 30.0;
        anim.flaps.fill(0.8);
        let mut filled = [true, false, true];
        reset_flight_state(&mut anim, &mut filled);
        assert_eq!(anim.spool, 0.0);
        assert_eq!(anim.time, 0.0);
        assert_eq!(anim.flaps, Anim::new().flaps);
        assert_eq!(filled, [false; 3]);
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
}
