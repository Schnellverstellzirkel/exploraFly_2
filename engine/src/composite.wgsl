// Final composite: HDR linear scene to swapchain sRGB with ACES.
// Reads HDR image (B10G11R11 or RGBA16F) as sampled texture, writes LDR.

@group(0) @binding(0) var scene_tex: texture_2d<f32>;
@group(0) @binding(1) var scene_smp: sampler;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
    var out: VsOut;
    var pos = vec2(-1.0, -1.0);
    if (vid == 1u || vid == 4u) {
        pos = vec2(1.0, -1.0);
    } else if (vid == 2u || vid == 3u) {
        pos = vec2(-1.0, 1.0);
    } else if (vid == 5u) {
        pos = vec2(1.0, 1.0);
    }
    out.clip = vec4(pos, 0.0, 1.0);
    out.uv = pos * 0.5 + 0.5;
    return out;
}

fn aces_tonemap(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3(0.0), vec3(1.0));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Exposure fixed at 1.0; matches sun irradiance scale 3.2 in plane.rs update.
    let hdr = textureSampleLevel(scene_tex, scene_smp, in.uv, 0.0).rgb;
    let ldr = aces_tonemap(hdr);
    // WGSL output to B8G8R8A8_SRGB framebuffer handles linear to sRGB.
    return vec4(ldr, 1.0);
}
