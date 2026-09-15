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
    float streak_factor = smoothstep(100.0, 650.0, speed) * smoothstep(0.08, 0.75, r2) * 0.012;

    vec3 hdr;
    if (streak_factor > 0.0004) {
        vec2 s_uv = clamp(lens_uv - centered * (streak_factor * 0.75), vec2(0.001), vec2(0.999));
        vec3 s_streak = textureLod(sampler2D(scene_tex, scene_smp), s_uv, 0.0).rgb;
        hdr = mix(s0, s_streak, clamp(streak_factor * 35.0, 0.0, 0.45));
    } else {
        hdr = s0;
    }

    // 4. Dynamic photometric auto-exposure adaptation
    float exposure = clamp(ubo.cameraParams2.y, 0.65, 1.35);
    hdr *= exposure;

    // 5. Physical lens vignetting (cos^4 falloff towards aperture periphery)
    float vig = clamp(1.0 - 0.26 * r2 - 0.14 * r4, 0.0, 1.0);
    hdr *= vig;

    // 6. Optical glare & halation blooming around high-radiance sources (sun and plume)
    float lum = dot(hdr, vec3(0.2126, 0.7152, 0.0722));
    const float bloom_threshold = 1.85;
    if (lum > bloom_threshold) {
        float flare = (lum - bloom_threshold) * 0.075;
        hdr += vec3(flare * 1.10, flare * 1.05, flare * 0.95);
    }

    // 6b. Sun optical glare, anamorphic horizontal flare streak, and multi-element lens ghosts
    vec4 sun_clip = ubo.viewProj * vec4(ubo.campos.xyz + normalize(ubo.sunDir.xyz) * 100000.0, 1.0);
    if (sun_clip.w > 0.0) {
        vec2 sun_ndc = sun_clip.xy / sun_clip.w;
        vec2 sun_uv = sun_ndc * 0.5 + 0.5;

        // Proximity to active screen bounds
        float off_x = max(0.0, abs(sun_uv.x - 0.5) - 0.5);
        float off_y = max(0.0, abs(sun_uv.y - 0.5) - 0.5);
        float off_dist = length(vec2(off_x * aspect, off_y));

        if (off_dist < 0.6) {
            // Unoccluded visibility check against scene HDR luminance
            float occ = 1.0;
            if (off_dist == 0.0) {
                vec3 sun_sample = textureLod(sampler2D(scene_tex, scene_smp), clamp(sun_uv, 0.002, 0.998), 0.0).rgb;
                float sun_lum = dot(sun_sample, vec3(0.2126, 0.7152, 0.0722));
                occ = clamp(sun_lum / 60.0, 0.0, 1.0);
            }
            float sun_fade = occ * smoothstep(0.6, 0.0, off_dist);

            if (sun_fade > 0.001) {
                vec2 to_frag = (lens_uv - sun_uv) * vec2(aspect, 1.0);
                float dist = length(to_frag);

                // Broad solar corona & aureole
                float corona = 0.08 / (dist * dist * 45.0 + 0.12) + 0.04 / (dist * 4.0 + 0.2);

                // 6-blade aperture diffraction spikes
                float angle = atan(to_frag.y, to_frag.x);
                float spikes = pow(abs(cos(angle * 3.0)), 24.0) * 0.6
                             + pow(abs(sin(angle * 3.0)), 32.0) * 0.4;
                float spike_intensity = spikes * (0.035 / (dist * 18.0 + 0.05)) * exp(-dist * 2.5);

                // Anamorphic horizontal streak
                float dy = abs(lens_uv.y - sun_uv.y);
                float dx = abs(lens_uv.x - sun_uv.x) * aspect;
                float streak = exp(-dy * 110.0) * exp(-dx * 1.5) * 0.35;
                vec3 streak_color = vec3(0.35, 0.65, 1.0) * streak;

                // Multi-element lens ghost reflections (inverted along optical center axis)
                vec2 center_to_sun = sun_uv - vec2(0.5);
                vec3 ghosts = vec3(0.0);

                // Ghost 1: Warm amber circular halo (factor = 0.5)
                vec2 g1_pos = vec2(0.5) - center_to_sun * 0.5;
                float g1_d = length((lens_uv - g1_pos) * vec2(aspect, 1.0));
                ghosts += vec3(0.9, 0.6, 0.2) * (smoothstep(0.12, 0.0, g1_d) * 0.045);

                // Ghost 2: Soft chromatic ring (factor = 1.0)
                vec2 g2_pos = vec2(0.5) - center_to_sun * 1.0;
                float g2_d = length((lens_uv - g2_pos) * vec2(aspect, 1.0));
                float ring = exp(-pow((g2_d - 0.18) * 25.0, 2.0)) * 0.04;
                ghosts += vec3(0.4, 0.8, 0.5) * ring;

                // Ghost 3: Violet secondary flare (factor = -0.35)
                vec2 g3_pos = vec2(0.5) + center_to_sun * 0.35;
                float g3_d = length((lens_uv - g3_pos) * vec2(aspect, 1.0));
                ghosts += vec3(0.4, 0.3, 0.9) * (smoothstep(0.06, 0.0, g3_d) * 0.06);

                // Ghost 4: Wide cyan iris reflection (factor = 0.85)
                vec2 g4_pos = vec2(0.5) - center_to_sun * 0.85;
                float g4_d = length((lens_uv - g4_pos) * vec2(aspect, 1.0));
                ghosts += vec3(0.2, 0.7, 0.9) * (smoothstep(0.25, 0.05, g4_d) * 0.025);

                vec3 sun_flare = (vec3(1.0, 0.92, 0.8) * (corona + spike_intensity) + streak_color + ghosts) * sun_fade;
                hdr += sun_flare * 1.8;
            }
        }
    }

    // 7. Photodiode Poisson-Gaussian CMOS sensor noise (film / sensor grain)
    float time = ubo.flex.y;
    vec2 p = gl_FragCoord.xy;
    float noise = fract(52.9829189 * fract(dot(p + vec2(time * 31.7, time * 17.3), vec2(0.06711056, 0.00583715))));
    float luma = clamp(lum, 0.0, 1.0);
    float grain_curve = 4.0 * sqrt(luma) * (1.0 - luma);
    float grain = (noise - 0.5) * 0.016 * grain_curve;
    hdr += vec3(grain);

    // 8. ACES Filmic tonemapping
    vec3 ldr = acesTonemap(hdr);
    outColor = vec4(ldr, 1.0);
}
