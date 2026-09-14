#version 450

// Final composite: HDR linear scene to swapchain sRGB with ACES.

layout(set = 0, binding = 0) uniform texture2D scene_tex;
layout(set = 0, binding = 1) uniform sampler scene_smp;

layout(location = 0) in vec2 vUv;
layout(location = 0) out vec4 outColor;

vec3 acesTonemap(vec3 x) {
    const float a = 2.51;
    const float b = 0.03;
    const float c = 2.43;
    const float d = 0.59;
    const float e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3(0.0), vec3(1.0));
}

void main() {
    // Exposure fixed at 1.0; matches sun irradiance scale 3.2 in plane.rs update.
    vec3 hdr = texture(sampler2D(scene_tex, scene_smp), vUv).rgb;
    vec3 ldr = acesTonemap(hdr);
    outColor = vec4(ldr, 1.0);
}
