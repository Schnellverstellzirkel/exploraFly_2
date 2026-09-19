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
    vec4 groundOrigin;
    vec4 hudFlight;    // knots, altitude m, heading degrees, climb m/s
    vec4 hudState;     // visible, boost, flags, clearance m
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


// Atlas-free overlay: two vec4 updates, no new draw, texture, descriptor or allocation.
const uint HUD_GLYPH[40] = uint[40](31599u,29850u,29671u,31207u,18925u,31183u,31695u,9383u,31727u,31215u,23530u,15083u,25166u,15211u,29391u,4815u,27470u,23533u,29847u,11044u,23277u,29257u,23549u,24573u,11114u,4843u,28522u,23275u,14478u,9367u,31597u,11117u,24557u,23213u,9389u,29351u,4772u,448u,8192u,1040u);
float hudGlyph(vec2 p, uint id) {
    if (id >= 40u || any(lessThan(p, vec2(0))) || any(greaterThanEqual(p, vec2(3,5)))) return 0.0;
    uint bit = uint(floor(p.x)) + 3u * uint(floor(p.y));
    return float((HUD_GLYPH[id] >> bit) & 1u);
}
float hudText(vec2 p, uvec2 words, float size) {
    p /= size;
    if (p.x < 0.0 || p.x >= 40.0 || p.y < 0.0 || p.y >= 5.0) return 0.0;
    uint cell = uint(p.x) / 4u;
    uint word = cell < 5u ? words.x : words.y;
    return hudGlyph(vec2(mod(p.x, 4.0), p.y), (word >> (6u * (cell % 5u))) & 63u);
}
float hudNumber(vec2 p, float value, int digits, float size) {
    p /= size;
    if (p.x < 0.0 || p.y < 0.0 || p.y >= 5.0 || p.x >= float(digits * 4)) return 0.0;
    int column = int(p.x) / 4;
    int divisor = int(pow(10.0, float(digits - column - 1)));
    return hudGlyph(vec2(mod(p.x, 4.0), p.y), uint(int(value + 0.5) / divisor % 10));
}
float hudBox(vec2 p, vec2 lo, vec2 hi) {
    return float(all(greaterThanEqual(p, lo)) && all(lessThan(p, hi)));
}
vec3 flightHud(vec3 scene, vec2 pixels, vec2 viewport) {
    if (ubo.hudState.x < 0.5) return scene;
    float scale = clamp(viewport.y / 900.0, 0.65, 1.6);
    vec2 p = pixels / scale;
    vec2 extent = viewport / scale;
    const vec3 ink = vec3(0.008,0.020,0.026);
    const vec3 paper = vec3(0.89,0.86,0.72);
    const vec3 brass = vec3(0.64,0.39,0.13);
    const vec3 teal = vec3(0.11,0.60,0.48);
    uint flags = uint(ubo.hudState.z);
    if (hudBox(p, vec2(24,24), vec2(266,99)) > 0.5) {
        vec2 q = p - vec2(24,24);
        vec3 c = mix(scene, ink, 0.78);
        float accent = hudBox(q, vec2(0), vec2(3,75));
        float title = hudText(q-vec2(18,16), uvec2(408262734u, 1073738395u), 4.0);
        float subtitle = hudText(q-vec2(18,47), uvec2(255387343u, 491062421u), 2.0);
        c = mix(c, brass, accent);
        c = mix(c, paper, title);
        return mix(c, teal, subtitle);
    }
    // Compass ribbon uses continuous heading offsets; label is true heading.
    vec2 cp = p - vec2(extent.x * 0.5 - 142.0, 24);
    if (hudBox(cp, vec2(0), vec2(284,54)) > 0.5) {
        vec3 c = mix(scene, ink, 0.68);
        float tick = float(mod(cp.x - 142.0 + ubo.hudFlight.z * 2.0, 20.0) < 1.0)
            * hudBox(cp, vec2(8,36), vec2(276,44));
        float num = hudNumber(cp-vec2(125,12), ubo.hudFlight.z, 3, 3.0);
        float mark = hudBox(cp, vec2(141,43), vec2(143,53));
        c = mix(c, brass, max(tick * 0.65, mark));
        return mix(c, paper, num);
    }
    vec2 left = p - vec2(24, extent.y-130);
    if (hudBox(left, vec2(0), vec2(252,106)) > 0.5) {
        vec3 c = mix(scene, ink, 0.79);
        float label = hudText(left-vec2(16,12), uvec2(426882186u, 1073533838u), 2.0);
        float num = hudNumber(left-vec2(16,32), ubo.hudFlight.x, 4, 6.0);
        float units = hudText(left-vec2(120,48), uvec2(1073739604u, 1073741823u), 2.0);
        float boost = hudText(left-vec2(16,82), uvec2(493979147u, 1073741823u), 2.0);
        float track = hudBox(left, vec2(70,83), vec2(236,91));
        float fill = hudBox(left, vec2(70,83), vec2(70 + 166.0*ubo.hudState.y,91));
        c = mix(c, paper * 0.50, max(label, boost));
        c = mix(c, paper, max(num, units));
        c = mix(c, teal * 0.15, track);
        return mix(c, teal, fill);
    }
    vec2 right = p - vec2(extent.x-276, extent.y-130);
    if (hudBox(right, vec2(0), vec2(252,106)) > 0.5) {
        vec3 c = mix(scene, ink, 0.79);
        float label = hudText(right-vec2(16,12), uvec2(491377994u, 1073537886u), 2.0);
        float num = hudNumber(right-vec2(16,32), ubo.hudFlight.y, 5, 6.0);
        float units = hudText(right-vec2(144,48), uvec2(1073741782u, 1073741823u), 2.0);
        float agl = hudText(right-vec2(16,82), uvec2(1073566730u, 1073741823u), 2.0);
        float ground = hudNumber(right-vec2(60,80), ubo.hudState.w, 5, 2.5);
        c = mix(c, paper * 0.5, label);
        c = mix(c, paper, max(num, units));
        return mix(c, ubo.hudState.w < 100.0 ? brass : teal, max(agl, ground));
    }
    vec2 center = p - extent * 0.5;
    if ((flags & 8u) != 0u && hudBox(center, vec2(-108,-35), vec2(108,45)) > 0.5) {
        vec3 c = mix(scene, ink, 0.86);
        float text = hudText(center + vec2(69,17), uvec2(242344601u, 1073741773u), 6.0);
        float hint = hudText(center - vec2(-47,22), uvec2(473546713u, 1073538462u), 2.0);
        return mix(c, paper, max(text,hint));
    }
    // Keep help away from instrument panels on narrow windows.
    vec2 hp = p - vec2(extent.x*0.5-146, extent.y-124);
    if ((flags & 2u) != 0u && extent.x > 920.0 && hudBox(hp,vec2(0),vec2(292,100)) > 0.5) {
        vec3 c = mix(scene, ink, 0.70);
        float t = hudText(hp-vec2(14,12), uvec2(436062496u, 1061472082u), 2.0);
        t = max(t, hudText(hp-vec2(158,12), uvec2(201120010u, 1073563082u), 2.0));
        t = max(t, hudText(hp-vec2(14,34), uvec2(587000090u, 1073739786u), 2.0));
        t = max(t, hudText(hp-vec2(158,34), uvec2(490546268u, 493979147u), 2.0));
        t = max(t, hudText(hp-vec2(14,56), uvec2(506044377u, 1073738652u), 2.0));
        t = max(t, hudText(hp-vec2(158,56), uvec2(473546715u, 1073739598u), 2.0));
        t = max(t, hudText(hp-vec2(14,78), uvec2(509726678u, 1073738583u), 2.0));
        t = max(t, hudText(hp-vec2(158,78), uvec2(239595599u, 1073739349u), 2.0));
        return mix(c, paper * 0.7, t);
    }
    return scene;
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
    // Derivatives are evaluated uniformly; overlay resolution is swapchain,
    // independent of the scene render scale. Optical filters never blur text.
    vec2 viewport = 1.0 / max(fwidth(vUv), vec2(0.00001));
    ldr = flightHud(ldr, vUv * viewport, viewport);
    outColor = vec4(ldr, 1.0);
}
