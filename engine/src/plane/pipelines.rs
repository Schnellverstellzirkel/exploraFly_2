//! Graphics pipeline construction: the seven scene passes (airframe
//! opaque/glass, sky, terrain, persistent vegetation, clouds, void warm-up) with their shared
//! layouts, plus the FX set (plume, trail, composite).

use ash::vk;
use crate::quality::Settings;
use super::VERTEX_BYTES;
use super::descriptors::material_descriptor_bindings;
use super::spirv::{ground_frag_spv_rt, plane_frag_spv, plane_frag_spv_rt};

fn shading_rate(size: [u32; 2]) -> vk::Extent2D {
    vk::Extent2D { width: size[0], height: size[1] }
}

/// The scene passes and their shared layouts.
pub(super) struct ScenePipelines {
    pub(super) set_layout: vk::DescriptorSetLayout,
    pub(super) layout: vk::PipelineLayout,
    pub(super) opaque_pipeline: vk::Pipeline,
    pub(super) glass_pipeline: vk::Pipeline,
    pub(super) sky_pipeline: vk::Pipeline,
    pub(super) ground_pipeline: vk::Pipeline,
    pub(super) vegetation_pipeline: vk::Pipeline,
    pub(super) cloud_pipeline: vk::Pipeline,
    pub(super) void_pipeline: vk::Pipeline,
}

/// FX passes plus the composite stack.
pub(super) struct FxPipelines {
    pub(super) fx_layout: vk::DescriptorSetLayout,
    pub(super) fx_pipeline_layout: vk::PipelineLayout,
    pub(super) composite_set_layout: vk::DescriptorSetLayout,
    pub(super) composite_layout: vk::PipelineLayout,
    pub(super) plume_pipeline: vk::Pipeline,
    pub(super) trail_pipeline: vk::Pipeline,
    pub(super) composite_pipeline: vk::Pipeline,
}

pub(super) unsafe fn create_scene_pipelines(
    device: &ash::Device,
    driver_version: u32,
    rt_supported: bool,
    format: vk::Format,
    samples: vk::SampleCountFlags,
    ground_fsr: bool,
    quality: &Settings,
    _ibl_samples: u32,
) -> ScenePipelines {
    let bindings = material_descriptor_bindings(rt_supported);
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
        .unwrap_or(quality.ibl_samples);
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
    let vegetation_vert_words = crate::spv_words(include_bytes!(concat!(
        env!("OUT_DIR"),
        "/vegetation.vert.spv"
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
    let cloud_vert_words = crate::spv_words(include_bytes!(concat!(
        env!("OUT_DIR"),
        "/cloud.vert.spv"
    )));
    let cloud_frag_words = crate::spv_words(include_bytes!(concat!(
        env!("OUT_DIR"),
        "/cloud.frag.spv"
    )));
    let plane_vert = mk_module(&plane_vert_words);
    let plane_frag = mk_module(if rt_supported { &plane_frag_rt_words } else { &plane_frag_words });
    let sky_vert = mk_module(&sky_vert_words);
    let ground_vert = mk_module(&ground_vert_words);
    let vegetation_vert = mk_module(&vegetation_vert_words);
    let sky_frag = mk_module(&sky_frag_words);
    let ground_frag = mk_module(if rt_supported { &ground_frag_rt_words } else { &ground_frag_words });
    let depth_frag = mk_module(&depth_frag_words);
    let cloud_vert = mk_module(&cloud_vert_words);
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
        .fragment_size(shading_rate(quality.sky))
        .combiner_ops([
            vk::FragmentShadingRateCombinerOpKHR::KEEP,
            vk::FragmentShadingRateCombinerOpKHR::KEEP,
        ]);
    if ground_fsr {
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
        .fragment_size(shading_rate(quality.ground))
        .combiner_ops([
            vk::FragmentShadingRateCombinerOpKHR::KEEP,
            vk::FragmentShadingRateCombinerOpKHR::KEEP,
        ]);
    if ground_fsr {
        ground_info = ground_info.push_next(&mut ground_rate);
    }
    let vegetation_stages = [
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(vegetation_vert)
            .name(main_entry),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(ground_frag)
            .name(main_entry),
    ];
    let mut rendering_vegetation = vk::PipelineRenderingCreateInfo::default()
        .color_attachment_formats(&formats)
        .depth_attachment_format(vk::Format::D32_SFLOAT);
    let mut vegetation_info = vk::GraphicsPipelineCreateInfo::default()
        .stages(&vegetation_stages)
        .vertex_input_state(&sky_vertex_input)
        .input_assembly_state(&input_assembly)
        .viewport_state(&viewport_state)
        .rasterization_state(&raster)
        .multisample_state(&multisample)
        .depth_stencil_state(&ground_depth)
        .color_blend_state(&blend_off_state)
        .dynamic_state(&dynamic_state)
        .layout(layout)
        .push_next(&mut rendering_vegetation);
    let mut vegetation_rate = vk::PipelineFragmentShadingRateStateCreateInfoKHR::default()
        .fragment_size(shading_rate(quality.ground))
        .combiner_ops([
            vk::FragmentShadingRateCombinerOpKHR::KEEP,
            vk::FragmentShadingRateCombinerOpKHR::KEEP,
        ]);
    if ground_fsr {
        vegetation_info = vegetation_info.push_next(&mut vegetation_rate);
    }
    // Mesh clouds: procedural puff-cluster geometry decoded from
    // gl_VertexIndex (cloud.vert), drawn between the sky and the terrain
    // like the landmark field. Opaque with depth writes, so clouds and
    // mountains z-occlude each other exactly through the shared depth
    // buffer; billows are smooth-shaded and melted into the horizon by
    // aerial haze.
    let cloud_stages = [
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(cloud_vert)
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
    let cloud_info = vk::GraphicsPipelineCreateInfo::default()
        .stages(&cloud_stages)
        .vertex_input_state(&sky_vertex_input)
        .input_assembly_state(&input_assembly)
        .viewport_state(&viewport_state)
        .rasterization_state(&raster)
        .multisample_state(&multisample)
        .depth_stencil_state(&cloud_depth)
        .color_blend_state(&blend_off_state)
        .dynamic_state(&dynamic_state)
        .layout(layout)
        .push_next(&mut rendering_cloud);
    // Disk-backed cache: fold every consumed SPIR-V module into the recipe
    // so a shader edit starts a fresh cache instead of feeding the driver
    // stale entries it would have to discard.
    let recipe = [plane_vert_words.as_slice(), sky_vert_words.as_slice(),
        ground_vert_words.as_slice(), sky_frag_words.as_slice(),
        ground_frag_words.as_slice(), depth_frag_words.as_slice(),
        vegetation_vert_words.as_slice(), plane_frag_words.as_slice(), cloud_vert_words.as_slice(),
        cloud_frag_words.as_slice()]
        .iter().fold(0xcbf2_9ce4_8422_2325u64, |h, w| super::pipeline_cache::hash_module(h, w));
    let pipeline_cache = super::pipeline_cache::load(device, driver_version, recipe);
    let pipelines = device
        .create_graphics_pipelines(
            pipeline_cache,
            &[
                opaque_info,
                glass_info,
                sky_info,
                ground_info,
                vegetation_info,
                void_info,
                cloud_info,
            ],
            None,
        )
        .expect("ppipes");
    let pipelines = pipelines;
    let opaque_pipeline = pipelines[0];
    let glass_pipeline = pipelines[1];
    let sky_pipeline = pipelines[2];
    let ground_pipeline = pipelines[3];
    let vegetation_pipeline = pipelines[4];
    let void_pipeline = pipelines[5];
    let cloud_pipeline = pipelines[6];
    super::pipeline_cache::store(device, pipeline_cache, driver_version, recipe);
    super::pipeline_cache::destroy(device, pipeline_cache);
    device.destroy_shader_module(plane_vert, None);
    device.destroy_shader_module(plane_frag, None);
    device.destroy_shader_module(sky_vert, None);
    device.destroy_shader_module(ground_vert, None);
    device.destroy_shader_module(vegetation_vert, None);
    device.destroy_shader_module(sky_frag, None);
    device.destroy_shader_module(ground_frag, None);
    device.destroy_shader_module(cloud_vert, None);
    device.destroy_shader_module(cloud_frag, None);
    device.destroy_shader_module(depth_frag, None);
    ScenePipelines {
        set_layout,
        layout,
        opaque_pipeline,
        glass_pipeline,
        sky_pipeline,
        ground_pipeline,
        vegetation_pipeline,
        cloud_pipeline,
        void_pipeline,
    }
}

pub(super) unsafe fn create_fx_pipelines(
    device: &ash::Device,
    driver_version: u32,
    set_layout: vk::DescriptorSetLayout,
    format: vk::Format,
    _samples: vk::SampleCountFlags,
    ground_fsr: bool,
    quality: &Settings,
) -> FxPipelines {
    let main_entry = c"main";
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
        .stride(crate::fx_gpu::PLUME_VERT_BYTES as u32)
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
        .stride(crate::fx_gpu::TRAIL_VERT_BYTES as u32)
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
        .fragment_size(shading_rate(quality.plume))
        .combiner_ops([
            vk::FragmentShadingRateCombinerOpKHR::KEEP,
            vk::FragmentShadingRateCombinerOpKHR::KEEP,
        ]);
    if ground_fsr {
        // Keep plume edges at full rate in the image-quality presets.
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
        .fragment_size(shading_rate(quality.composite))
        .combiner_ops([
            vk::FragmentShadingRateCombinerOpKHR::KEEP,
            vk::FragmentShadingRateCombinerOpKHR::KEEP,
        ]);
    if ground_fsr {
        comp_info = comp_info.push_next(&mut comp_rate);
    }
    let fx_recipe = [plume_vert_words.as_slice(), plume_frag_words.as_slice(),
        trail_vert_words.as_slice(), trail_frag_words.as_slice(),
        comp_vert_words.as_slice(), comp_frag_words.as_slice()]
        .iter().fold(0xcbf2_9ce4_8422_2325u64, |h, w| super::pipeline_cache::hash_module(h, w));
    let fx_cache = super::pipeline_cache::load(device, driver_version, fx_recipe);
    let fx_pipes = device
        .create_graphics_pipelines(
            fx_cache,
            &[plume_info, trail_info, comp_info],
            None,
        )
        .expect("fxpipes");
    let plume_pipeline = fx_pipes[0];
    let trail_pipeline = fx_pipes[1];
    let composite_pipeline = fx_pipes[2];
    super::pipeline_cache::store(device, fx_cache, driver_version, fx_recipe);
    super::pipeline_cache::destroy(device, fx_cache);
    device.destroy_shader_module(plume_vert, None);
    device.destroy_shader_module(plume_frag, None);
    device.destroy_shader_module(trail_vert, None);
    device.destroy_shader_module(trail_frag, None);
    device.destroy_shader_module(comp_vert, None);
    device.destroy_shader_module(comp_frag, None);
    println!("fx pipelines: plume + trail + composite ready");
    // Unit volume bounds cached once; update() scales it to nozzle state per frame.
    FxPipelines {
        fx_layout,
        fx_pipeline_layout,
        composite_set_layout,
        composite_layout,
        plume_pipeline,
        trail_pipeline,
        composite_pipeline,
    }
}
