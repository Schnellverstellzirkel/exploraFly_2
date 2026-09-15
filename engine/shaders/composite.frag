#version 450

// Final composite: HDR linear scene to swapchain sRGB with ACES.
// Physical camera simulation:
// - Curvilinear lens projection (Brown-Conrady barrel distortion)
// - Transverse chromatic aberration (optical dispersion at lens periphery)
// - Physical cos^4 vignetting (natural light attenuation)
// - Photodiode Poisson-Gaussian CMOS sensor noise (luminance-weighted film grain)
// - Dynamic photometric auto-exposure (sun adaptation & shadow detail preservation)
// - High-speed peripheral optical streak blur (radial velocity flow)
// - Lens glare & halation blooming around high-radiance sources

layout(set = 0, binding = 0) uniform UBO {
    mat4 viewProj;
    mat4 invViewProj;
    mat4 nodes[23];
    vec4 flex;         // x: bend, y: time, z: pressure, w: glow
    vec4 campos;       // xyz: camera pos, w: exit radius
    vec4 sunDir;       // xyz: dir, w: radius
    vec4 sunColor;     // xyz: irr, w: elevation
    vec4 skyZenith;    // xyz: zenith, w: cos_radius
    vec4 skyHorizon;   // xyz: horizon, w: inv_one_minus_cos_radius
    vec4 groundBase;   // xyz: ground, w: lambda
    vec4 detail;       // x: plume_flicker, y: ambient_p, z: spool, w: plume_length
    vec4 trailShift;   // xyz: shift, w: 0
    vec4 cameraParams; // x: fov_y, y: aspect, z: speed, w: load
    vec4 cameraParams2;// x: shake, y: exposure, z: mach, w: unused
} ubo;

layout(set = 1, binding = 0) uniform texture2D scene_tex;
layout(set = 1, binding = 1) uniform sampler scene_smp;

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
    vec2 centered = vUv - 0.5;
    float aspect = max(ubo.cameraParams.y, 0.5);
    vec2 aspect_uv = vec2(centered.x * aspect, centered.y);
    float r2 = dot(aspect_uv, aspect_uv);
    float r4 = r2 * r2;

    // 1. Curvilinear lens projection (Brown-Conrady barrel distortion matching wide airborne camera)
    const float k1 = 0.042;
    const float k2 = 0.016;
    float dist = 1.0 + k1 * r2 + k2 * r4;
    const float overscan = 0.970;
    vec2 lens_uv = 0.5 + centered * (dist * overscan);

    // 2. Transverse chromatic aberration (optical wavelength dispersion towards lens periphery)
    float shake = ubo.cameraParams2.x;
    float ca_strength = (0.0030 + 0.0020 * shake) * r2;
    vec2 ca_offset = centered * ca_strength;

    vec2 uv_r = clamp(lens_uv + ca_offset, vec2(0.001), vec2(0.999));
    vec2 uv_g = clamp(lens_uv, vec2(0.001), vec2(0.999));
    vec2 uv_b = clamp(lens_uv - ca_offset, vec2(0.001), vec2(0.999));

    // 3. High-speed velocity streaking / peripheral radial motion blur
    float speed = ubo.cameraParams.z;
    float streak_factor = smoothstep(100.0, 650.0, speed) * smoothstep(0.08, 0.75, r2) * 0.012;

    vec3 s0 = vec3(
        texture(sampler2D(scene_tex, scene_smp), uv_r).r,
        texture(sampler2D(scene_tex, scene_smp), uv_g).g,
        texture(sampler2D(scene_tex, scene_smp), uv_b).b
    );

    vec3 hdr;
    if (streak_factor > 0.0004) {
        vec2 s1_uv = clamp(lens_uv - centered * (streak_factor * 0.5), vec2(0.001), vec2(0.999));
        vec2 s2_uv = clamp(lens_uv - centered * streak_factor, vec2(0.001), vec2(0.999));
        vec3 s1 = texture(sampler2D(scene_tex, scene_smp), s1_uv).rgb;
        vec3 s2 = texture(sampler2D(scene_tex, scene_smp), s2_uv).rgb;
        hdr = s0 * 0.55 + s1 * 0.30 + s2 * 0.15;
    } else {
        hdr = s0;
    }

    // 4. Dynamic photometric auto-exposure adaptation
    float exposure = clamp(ubo.cameraParams2.y, 0.65, 1.35);
    hdr *= exposure;

    // 5. Physical lens vignetting (cos^4 falloff towards aperture periphery)
    float vig = 1.0 - 0.26 * r2 - 0.14 * r4;
    vig = clamp(vig, 0.0, 1.0);
    hdr *= vig;

    // 6. Optical glare & halation blooming around high-radiance sources (sun and plume)
    float lum = dot(hdr, vec3(0.2126, 0.7152, 0.0722));
    const float bloom_threshold = 1.85;
    if (lum > bloom_threshold) {
        float flare = (lum - bloom_threshold) * 0.075;
        hdr += vec3(flare * 1.10, flare * 1.05, flare * 0.95);
    }

    // 7. Photodiode Poisson-Gaussian CMOS sensor noise (film / sensor grain)
    float time = ubo.flex.y;
    vec2 p = gl_FragCoord.xy;
    float noise = fract(sin(dot(p + vec2(time * 31.7, time * 17.3), vec2(12.9898, 78.233))) * 43758.5453);
    float luma = clamp(lum, 0.0, 1.0);
    float grain_curve = 4.0 * pow(luma, 0.55) * (1.0 - luma);
    float grain = (noise - 0.5) * 0.016 * grain_curve;
    hdr += vec3(grain);

    // 8. ACES Filmic tonemapping
    vec3 ldr = acesTonemap(hdr);
    outColor = vec4(ldr, 1.0);
}
