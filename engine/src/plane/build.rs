//! One-time GPU resource construction for the renderer: geometry and
//! index upload, ray-traced shadow structures, boot-time textures,
//! descriptor layouts, and every graphics pipeline.

use ash::vk;
use crate::quality::Quality;

use crate::anim::Anim;
use super::Plane;
use super::geometry::GeometryBuffers;
use super::textures::TextureResources;
use super::pipelines::{FxPipelines, ScenePipelines};
use super::rt::RtResources;
use super::fx_volumes::FxVolumes;


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
        mesh_shaders: bool,
        quality: Quality,
    ) -> Self {
        let quality = quality.settings();
        let ibl_samples = std::env::var("EXPLORA_IBL_SAMPLES")
            .map(|s| s.parse::<u32>().expect("invalid EXPLORA_IBL_SAMPLES"))
            .unwrap_or(quality.ibl_samples);
        let _shading_rate = |size: [u32; 2]| vk::Extent2D { width: size[0], height: size[1] };
        let mesh = super::airframe_mesh::airframe_mesh();
        let mem_props = instance.get_physical_device_memory_properties(physical);
        let vegetation_database = world::vegetation::build_database();
        println!(
            "vegetation database: {} instances, {} tree cells, {} canopy cells",
            vegetation_database.instances.len(),
            vegetation_database
                .cells
                .iter()
                .filter(|cell| cell.instance_count != 0)
                .count(),
            vegetation_database
                .canopies
                .iter()
                .filter(|record| world::vegetation::canopy_density(**record) > 0.0)
                .count(),
        );
        let (vegetation_buffer, vegetation_memory) = super::vegetation::upload_database(
            device,
            &mem_props,
            queue_family,
            queue,
            &vegetation_database,
        );
        let (canopy_buffer, canopy_memory) = super::vegetation::upload_canopy_database(
            device,
            &mem_props,
            queue_family,
            queue,
            &vegetation_database,
        );
        let (vegetation_cells_buffer, vegetation_cells_memory) =
            super::vegetation::upload_cells(
                device,
                &mem_props,
                queue_family,
                queue,
                &vegetation_database,
            );
        let geometry = super::geometry::upload_geometry(
            device,
            instance,
            physical,
            queue_family,
            queue,
            rt_supported,
            mesh_shaders,
            &mesh,
        );
        let GeometryBuffers {
            vertex_buffer,
            vertex_memory,
            index_buffer,
            index_memory,
            rt_vertex_address: _,
            rt_terrain_index_address: _,
            glass_first,
            glass_count,
            opaque_count,
            terrain_index_offset,
            cloud_index_offset,
            mesh: ref mesh_hierarchy,
        } = geometry;
        let (
            meshlet_buffer,
            meshlet_memory,
            part_buffer,
            part_memory,
            lod_buffer,
            lod_memory,
            vertex_index_buffer,
            vertex_index_memory,
            triangle_buffer,
            triangle_memory,
            meshlet_count,
        ) = match mesh_hierarchy {
            Some(h) => (
                h.meshlet_buffer,
                h.meshlet_memory,
                h.part_buffer,
                h.part_memory,
                h.lod_buffer,
                h.lod_memory,
                h.vertex_index_buffer,
                h.vertex_index_memory,
                h.triangle_buffer,
                h.triangle_memory,
                h.meshlet_count,
            ),
            None => (
                vk::Buffer::null(),
                vk::DeviceMemory::null(),
                vk::Buffer::null(),
                vk::DeviceMemory::null(),
                vk::Buffer::null(),
                vk::DeviceMemory::null(),
                vk::Buffer::null(),
                vk::DeviceMemory::null(),
                vk::Buffer::null(),
                vk::DeviceMemory::null(),
                0,
            ),
        };
        let terrain_lod_step = quality.terrain_lod_step;
        let terrain_chunk_count = if terrain_lod_step == world::PERFORMANCE_TERRAIN_STEP {
            world::PERFORMANCE_TERRAIN_COMMAND_COUNT
        } else {
            world::TERRAIN_CHUNK_COUNT
        };

        let RtResources {
            loader: rt_loader,
            instance_count: rt_instance_count,
            vertex_address: rt_vertex_address,
            index_buffer: rt_index_buffer,
            index_memory: rt_index_memory,
            index_address: rt_index_address,
            blas: rt_blas,
            blas_addresses: rt_blas_addresses,
            geom_nodes: rt_geom_nodes,
            blas_buffer: rt_blas_buffer,
            blas_memory: rt_blas_memory,
            terrain_vertex_buffer: rt_terrain_vertex_buffer,
            terrain_vertex_memory: rt_terrain_vertex_memory,
            terrain_blas_address: rt_terrain_blas_address,
            structures_vertex_buffer: rt_structures_vertex_buffer,
            structures_vertex_memory: rt_structures_vertex_memory,
            structures_blas_address: rt_structures_blas_address,
        } = super::rt::build_rt(
            device,
            instance,
            physical,
            &mem_props,
            queue_family,
            queue,
            rt_supported,
            &geometry,
            &mesh,
        );

        let TextureResources {
            terrain_image,
            terrain_memory,
            terrain_view,
            weave_image,
            weave_memory,
            weave_view,
            weave_sampler,
            lut_view,
            lut_sampler,
            atmo_transmittance_image,
            atmo_transmittance_memory,
            atmo_transmittance_view,
            atmo_multiscattering_image,
            atmo_multiscattering_memory,
            atmo_multiscattering_view,
            atmo_sampler,
            detail_textures,
        } = super::textures::upload_textures(
            device,
            instance,
            physical,
            &mem_props,
            queue_family,
            queue,
            max_aniso,
        );

        let ScenePipelines {
            set_layout,
            layout,
            opaque_pipeline,
            opaque_mesh_pipeline,
            glass_pipeline,
            sky_pipeline,
            terrain_pipeline,
            ground_pipeline,
            vegetation_pipeline,
            canopy_pipeline,
            vegetation_compact_pipeline,
            vegetation_cull_pipeline,
            vegetation_finalize_pipeline,
            cloud_pipeline,
            void_pipeline,
            void_mesh_pipeline,
        } = super::pipelines::create_scene_pipelines(
            device,
            instance.get_physical_device_properties(physical).driver_version,
            rt_supported,
            mesh_shaders,
            format,
            samples,
            ground_fsr,
            &quality,
            ibl_samples,
        );
        // FX noise volumes: Nubis-style tileable Perlin-Worley generated on
        // CPU once at boot (see noise.rs). R8G8B8A8_UNORM data, not sRGB.
        let FxVolumes {
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
        } = super::fx_volumes::upload_fx_volumes(device, &mem_props, queue_family, queue);
        // the HDR target switch.
        let FxPipelines {
            fx_layout,
            fx_pipeline_layout,
            composite_set_layout,
            composite_layout,
            plume_pipeline,
            trail_pipeline,
            composite_pipeline,
            hud_pipeline,
        } = super::pipelines::create_fx_pipelines(
            device,
            instance.get_physical_device_properties(physical).driver_version,
            set_layout,
            format,
            samples,
            ground_fsr,
            &quality,
        );
        let (cone_verts, _) = crate::fx_gpu::build_plume_cone(1.0, 1.0);
        let unit_cone: Vec<[f32; 5]> = cone_verts
            .iter()
            .map(|v| [v.pos[0], v.pos[1], v.pos[2], v.axial, v.radial])
            .collect();
        let query_info = vk::QueryPoolCreateInfo::default()
            .query_type(vk::QueryType::TIMESTAMP)
            .query_count(8);
        let query_pool = device.create_query_pool(&query_info, None).expect("qpool");
        Self {
            hud: [0.0; 8],
            opaque_count,
            glass_first,
            glass_count,
            vertex_buffer,
            vertex_memory,
            index_buffer,
            index_memory,
            terrain_index_offset,
            cloud_index_offset,
            terrain_image,
            terrain_memory,
            terrain_view,
            terrain_lod_step,
            terrain_chunk_count,
            terrain_draw_batch: if instance.get_physical_device_features(physical).multi_draw_indirect != 0 {
                instance.get_physical_device_properties(physical).limits.max_draw_indirect_count
                    .min(terrain_chunk_count)
            } else { 1 },
            cloud_puffs: quality.cloud_puffs,
            cloud_cells: quality.cloud_grid * quality.cloud_grid,
            vegetation_buffer,
            vegetation_memory,
            vegetation_cells_buffer,
            vegetation_cells_memory,
            vegetation_database,
            vegetation_output_buffers: Vec::new(),
            vegetation_output_memories: Vec::new(),
            gpu_vegetation_cull: std::env::var("EXPLORA_GPU_VEGETATION")
                .map(|value| value != "0")
                // Keep the measured legacy path as the shipping default until
                // the compact representation wins on a representative dense
                // flight. The GPU path remains opt-in for A/B profiling.
                .unwrap_or(false),
            vegetation_draw_batch: if instance.get_physical_device_features(physical).multi_draw_indirect != 0 {
                instance
                    .get_physical_device_properties(physical)
                    .limits
                    .max_draw_indirect_count
                    .min(world::vegetation::VEGETATION_COMMAND_CAPACITY)
            } else {
                1
            },
            canopy_draw_batch: if instance.get_physical_device_features(physical).multi_draw_indirect != 0 {
                instance
                    .get_physical_device_properties(physical)
                    .limits
                    .max_draw_indirect_count
                    .min(world::vegetation::CANOPY_COMMAND_CAPACITY)
            } else {
                1
            },
            canopy_buffer,
            canopy_memory,
            set_layout,
            descriptor_pool: vk::DescriptorPool::null(),
            weave_view,
            weave_sampler,
            lut_view,
            lut_sampler,
            atmo_transmittance_image,
            atmo_transmittance_memory,
            atmo_transmittance_view,
            atmo_multiscattering_image,
            atmo_multiscattering_memory,
            atmo_multiscattering_view,
            atmo_sampler,
            detail_textures,
            weave_image,
            weave_memory,
            opaque_pipeline,
            opaque_mesh_pipeline,
            glass_pipeline,
            sky_pipeline,
            terrain_pipeline,
            ground_pipeline,
            vegetation_pipeline,
            canopy_pipeline,
            vegetation_compact_pipeline,
            vegetation_cull_pipeline,
            vegetation_finalize_pipeline,
            cloud_pipeline,
            void_pipeline,
            void_mesh_pipeline,
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
            plume_pipeline,
            trail_pipeline,
            composite_pipeline,
            hud_pipeline,
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
            rt_geom_nodes: rt_geom_nodes.clone(),
            rt_blas_buffer,
            rt_blas_memory,
            rt_terrain_vertex_buffer,
            rt_terrain_vertex_memory,
            rt_terrain_blas_address,
            rt_structures_vertex_buffer,
            rt_structures_vertex_memory,
            rt_structures_blas_address,
            rt_instance_buffers: Vec::new(),
            rt_instance_memories: Vec::new(),
            rt_instance_mapped: Vec::new(),
            rt_tlas: Vec::new(),
            rt_tlas_buffers: Vec::new(),
            rt_tlas_memories: Vec::new(),
            rt_scratch: Vec::new(),
            rt_scratch_memories: Vec::new(),
            rt_instance_count,
            mesh_shaders,
            mesh_loader: ash::ext::mesh_shader::Device::new(instance, device),
            meshlet_buffer,
            #[allow(dead_code)]
            meshlet_memory,
            part_buffer,
            #[allow(dead_code)]
            part_memory,
            lod_buffer,
            #[allow(dead_code)]
            lod_memory,
            vertex_index_buffer,
            #[allow(dead_code)]
            vertex_index_memory,
            triangle_buffer,
            #[allow(dead_code)]
            triangle_memory,
            meshlet_count,
            anim: Anim::new(),
        }
    }
}
