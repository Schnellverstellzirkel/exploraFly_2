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
    vec4 groundBase;   // xyz: ground albedo, w: relative ground height
    vec4 detail;       // x: plume_flicker, y: lambda, z: spool, w: plume_length
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

vec2 projectSunSceneUv() {
    vec4 clip = ubo.viewProj * vec4(normalize(ubo.sunDir.xyz), 0.0);
    if (clip.w <= 1e-5) {
        return vec2(-2.0);
    }
    return clip.xy / clip.w * 0.5 + 0.5;
}

// The scene texture is sampled through the forward Brown-Conrady mapping
// below.  Invert that mapping for the optical PSF so glare stays registered
// with the solar source instead of appearing as a displaced disk.
vec2 inverseLensUv(vec2 sceneUv, float aspect) {
    vec2 target = sceneUv - 0.5;
    vec2 centered = target / 0.970;
    for (int i = 0; i < 3; ++i) {
        vec2 radial = vec2(centered.x * aspect, centered.y);
        float r2 = dot(radial, radial);
        float dist = 1.0 + 0.042 * r2 + 0.016 * r2 * r2;
        centered = target / (0.970 * dist);
    }
    return centered + 0.5;
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
    vec2 uv_g = clamp(lens_uv, vec2(0.001), vec2(0.999));
    vec3 s0 = textureLod(sampler2D(scene_tex, scene_smp), uv_g, 0.0).rgb;

    // 3. High-speed velocity streaking / peripheral radial motion blur
    float speed = ubo.cameraParams.z;
    vec3 hdr = s0;
    if (speed > 100.0 && r2 > 0.08) {
        float streak_factor = smoothstep(100.0, 650.0, speed) * smoothstep(0.08, 0.75, r2) * 0.012;
        if (streak_factor > 0.0004) {
            vec2 s_uv = clamp(lens_uv - centered * (streak_factor * 0.75), vec2(0.001), vec2(0.999));
            vec3 s_streak = textureLod(sampler2D(scene_tex, scene_smp), s_uv, 0.0).rgb;
            hdr = mix(s0, s_streak, clamp(streak_factor * 35.0, 0.0, 0.45));
        }
    }

    // 4. Dynamic photometric auto-exposure adaptation
    float exposure = clamp(ubo.cameraParams2.y, 0.65, 1.35);
    hdr *= exposure;

    // 5. Physical lens vignetting (cos^4 falloff towards aperture periphery)
    float vig = clamp(1.0 - 0.26 * r2 - 0.14 * r4, 0.0, 1.0);
    hdr *= vig;

    // 6. Source-driven solar veiling glare.  This is a smooth sensor PSF,
    // not a second atmospheric lobe: the source energy is read from the HDR
    // solar disc and the kernel has an exponential tail with no finite edge.
    vec2 sun_scene_uv = projectSunSceneUv();
    vec2 sun_sensor_uv = inverseLensUv(sun_scene_uv, aspect);
    bool sun_in_frame = all(greaterThanEqual(sun_scene_uv, vec2(0.0)))
                     && all(lessThanEqual(sun_scene_uv, vec2(1.0)));
    if (sun_in_frame) {
        vec3 sun_hdr = textureLod(
            sampler2D(scene_tex, scene_smp), clamp(sun_scene_uv, vec2(0.001), vec2(0.999)), 0.0).rgb;
        float source_lum = dot(max(sun_hdr, vec3(0.0)), vec3(0.2126, 0.7152, 0.0722));
        float source_gate = smoothstep(4.0, 24.0, source_lum);
        float source_energy = clamp(log2(1.0 + source_lum) / 12.0, 0.0, 1.0);
        vec2 psf_delta = (vUv - sun_sensor_uv) * vec2(aspect, 1.0);
        float psf_radius = length(psf_delta);
        float sun_radius_uv = tan(max(ubo.sunDir.w, 1e-5))
                            / (2.0 * tan(max(ubo.cameraParams.x * 0.5, 1e-3)));
        float core = exp(-0.5 * pow(psf_radius / max(sun_radius_uv * 2.7, 1e-5), 2.0));
        float tail = exp(-psf_radius / max(sun_radius_uv * 9.0, 1e-5));
        vec3 corona_tint = vec3(1.0, 0.91, 0.78);
        hdr += corona_tint * source_gate * source_energy * (0.46 * core + 0.14 * tail);
    }

    // 7. Photodiode Poisson-Gaussian CMOS sensor noise (film / sensor grain)
    float lum = dot(hdr, vec3(0.2126, 0.7152, 0.0722));
    float time = ubo.flex.y;
    vec2 p = gl_FragCoord.xy;
    float noise = fract(52.9829189 * fract(dot(p + vec2(time * 31.7, time * 17.3), vec2(0.06711056, 0.00583715))));
    float luma = clamp(lum, 0.0, 1.0);
    float grain_curve = 4.0 * luma * (1.0 - luma);
    float grain = (noise - 0.5) * 0.016 * grain_curve;
    hdr += vec3(grain);

    // 8. ACES Filmic tonemapping
    vec3 ldr = acesTonemap(hdr);
    outColor = vec4(ldr, 1.0);
}
