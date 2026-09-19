#version 450

// Final composite: HDR linear scene to swapchain sRGB with ACES.
// Restrained airborne camera treatment: lens projection, contrast-adaptive
// reconstruction, peripheral motion blur, spatial HDR glare, and sensor grain.
// Output remains linear; the sRGB swapchain performs the transfer function.

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

float luminance(vec3 color) {
    return dot(color, vec3(0.2126, 0.7152, 0.0722));
}

vec3 sampleScene(vec2 uv, vec2 texel) {
    // Clamp to actual texel centers, including every glare / motion-blur tap.
    // A fixed UV margin otherwise discards different amounts at each resolution.
    vec2 half_texel = 0.5 * texel;
    return max(textureLod(sampler2D(scene_tex, scene_smp),
        clamp(uv, half_texel, vec2(1.0) - half_texel), 0.0).rgb, vec3(0.0));
}

vec3 highlight(vec3 exposed) {
    // Soft threshold avoids a visible contour around sunlit clouds and exhaust.
    // Cap the scattered luminance so the sun cannot wash out the whole filter.
    const float threshold = 1.85;
    const float knee = 0.65;
    float lum = luminance(exposed);
    float soft = clamp(lum - threshold + knee, 0.0, 2.0 * knee);
    soft = soft * soft / (4.0 * knee);
    float scattered = min(max(lum - threshold, soft), 6.0);
    return exposed * (scattered / max(lum, 0.0001));
}

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

    vec2 texel = 1.0 / vec2(textureSize(sampler2D(scene_tex, scene_smp), 0));
    vec3 s0 = sampleScene(lens_uv, texel);
    vec3 north = sampleScene(lens_uv + vec2(0.0, -texel.y), texel);
    vec3 south = sampleScene(lens_uv + vec2(0.0, texel.y), texel);
    vec3 east = sampleScene(lens_uv + vec2(texel.x, 0.0), texel);
    vec3 west = sampleScene(lens_uv + vec2(-texel.x, 0.0), texel);

    // 2. Recover a little detail lost when the lower-resolution scene is enlarged.
    // Back off at high-contrast silhouettes, then constrain the result to the
    // neighborhood range: sharpening must not invent bright or dark edge halos.
    vec3 local_min = min(s0, min(min(north, south), min(east, west)));
    vec3 local_max = max(s0, max(max(north, south), max(east, west)));
    float lum_min = min(luminance(s0), min(min(luminance(north), luminance(south)),
        min(luminance(east), luminance(west))));
    float lum_max = max(luminance(s0), max(max(luminance(north), luminance(south)),
        max(luminance(east), luminance(west))));
    float contrast = (lum_max - lum_min) / max(lum_max, 0.15);
    float sharpen = 0.20 * (1.0 - smoothstep(0.15, 0.65, contrast));
    vec3 neighbors = 0.25 * (north + south + east + west);
    vec3 hdr = clamp(s0 + (s0 - neighbors) * sharpen, local_min, local_max);

    // 3. High-speed velocity streaking / peripheral radial motion blur
    float speed = ubo.cameraParams.z;
    if (speed > 100.0 && r2 > 0.08) {
        float streak_factor = smoothstep(100.0, 650.0, speed) * smoothstep(0.08, 0.75, r2) * 0.012;
        if (streak_factor > 0.0004) {
            vec2 s_uv = lens_uv - centered * (streak_factor * 0.75);
            vec3 s_streak = sampleScene(s_uv, texel);
            hdr = mix(hdr, s_streak, clamp(streak_factor * 35.0, 0.0, 0.45));
        }
    }

    // 4. Camera exposure; glare extraction uses the same linear HDR exposure.
    float exposure = clamp(ubo.cameraParams2.y, 0.65, 1.35);
    hdr *= exposure;

    // 5. Spatial highlight glare. Reuse reconstruction taps for the dense core;
    // four diagonal and four outer taps provide a small, soft neighborhood glow.
    // Weights sum to one and bound added luminance to 6 * 0.07 = 0.42.
    // This costs 13 scene samples total (14 where motion blur is active).
    vec3 glare = highlight(s0 * exposure) * 0.16;
    glare += (highlight(north * exposure) + highlight(south * exposure)
        + highlight(east * exposure) + highlight(west * exposure)) * 0.12;
    for (int y = -1; y <= 1; y += 2) {
        for (int x = -1; x <= 1; x += 2) {
            vec2 offset = vec2(float(x), float(y)) * texel * 1.5;
            glare += highlight(sampleScene(lens_uv + offset, texel) * exposure) * 0.065;
        }
    }
    glare += (highlight(sampleScene(lens_uv + vec2(texel.x * 4.0, 0.0), texel) * exposure)
        + highlight(sampleScene(lens_uv - vec2(texel.x * 4.0, 0.0), texel) * exposure)
        + highlight(sampleScene(lens_uv + vec2(0.0, texel.y * 4.0), texel) * exposure)
        + highlight(sampleScene(lens_uv - vec2(0.0, texel.y * 4.0), texel) * exposure)) * 0.025;
    hdr += glare * 0.07;

    // 6. Smooth lens vignetting attenuates the image and its scattered light.
    float vig = clamp(1.0 - 0.26 * r2 - 0.14 * r4, 0.0, 1.0);
    hdr *= vig;

    // 7. Subtle midtone sensor grain; keep distant detail clear and shadows clean.
    float time = ubo.flex.y;
    vec2 p = gl_FragCoord.xy;
    float noise = fract(52.9829189 * fract(dot(p + vec2(time * 31.7, time * 17.3), vec2(0.06711056, 0.00583715))));
    float luma = clamp(luminance(hdr), 0.0, 1.0);
    float grain_curve = 4.0 * luma * (1.0 - luma);
    float grain = (noise - 0.5) * 0.006 * grain_curve;
    hdr = max(hdr + vec3(grain), vec3(0.0));

    // 8. ACES Filmic tonemapping
    vec3 ldr = acesTonemap(hdr);
    outColor = vec4(ldr, 1.0);
}
