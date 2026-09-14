#version 450

// Nozzle-local volume integration with advected density and shock cells.

layout(set = 0, binding = 0) uniform UBO {
    mat4 viewProj;
    mat4 invViewProj;
    mat4 nodes[23];
    vec4 flex;
    vec4 campos;
    vec4 sunDir;
    vec4 sunColor;
    vec4 skyZenith;
    vec4 skyHorizon;
    vec4 groundBase;
    vec4 detail;
} ubo;

layout(set = 1, binding = 0) uniform texture3D base_vol;
layout(set = 1, binding = 1) uniform sampler base_smp;
layout(set = 1, binding = 2) uniform texture3D detail_vol;
layout(set = 1, binding = 3) uniform sampler detail_smp;
layout(set = 1, binding = 4) uniform texture2D curl_tex;
layout(set = 1, binding = 5) uniform sampler curl_smp;

layout(location = 0) in vec3 vWorld;
layout(location = 1) in float vAxial;
layout(location = 2) in float vRadial;
layout(location = 3) in vec3 vNozzle;

layout(location = 0) out vec4 outColor;

float remapRange(float x, float a, float b, float c, float d) {
    return c + (d - c) * clamp((x - a) / max(b - a, 1e-5), 0.0, 1.0);
}

float phaseHg(float mu, float g) {
    float gg = g * g;
    return (1.0 - gg) / (12.566371 * pow(max(1.0 + gg - 2.0 * g * mu, 1e-4), 1.5));
}

float phaseCs(float mu, float g) {
    float gg = g * g;
    float p1 = 1.5 * (1.0 - gg) / (2.0 + gg);
    float p2 = (1.0 + mu * mu) / pow(max(1.0 + gg - 2.0 * g * mu, 1e-4), 1.5);
    return p1 * p2 / 12.566371;
}

vec3 blackbody(float t) {
    float u = clamp((t - 800.0) / 1800.0, 0.0, 1.0);
    float r = 0.25 + 0.75 * smoothstep(0.0, 0.45, u);
    float g = 0.12 + 0.62 * smoothstep(0.15, 0.7, u);
    float b = 0.55 * smoothstep(0.0, 0.25, u) + 0.45 * smoothstep(0.5, 1.0, u);
    return vec3(r, g, b) * (0.4 + 2.6 * u * u);
}

void main() {
    float spool = ubo.detail.z;
    float length_m = max(ubo.detail.w, 0.5);
    float radius = ubo.campos.w;
    float bound = radius + length_m * 0.10;
    float time = ubo.flex.y;
    vec3 nozzle = vNozzle;
    mat3 frame = mat3(ubo.nodes[0]);
    vec3 ray = normalize(vWorld - ubo.campos.xyz);
    vec3 ro = transpose(frame) * (ubo.campos.xyz - nozzle);
    vec3 rd = transpose(frame) * ray;
    ro.z = -ro.z;
    rd.z = -rd.z;
    // Ray/box interval: each pixel traverses the interior once, including
    // views into the exhaust and cameras inside the bounding volume.
    vec3 safe_rd = mix(vec3(1e-6), rd, greaterThan(abs(rd), vec3(1e-6)));
    vec3 t0 = (vec3(-bound, -bound, 0.0) - ro) / safe_rd;
    vec3 t1 = (vec3(bound, bound, length_m) - ro) / safe_rd;
    vec3 lo = min(t0, t1), hi = max(t0, t1);
    float enter = max(max(lo.x, lo.y), max(lo.z, 0.0));
    float leave = min(min(hi.x, hi.y), hi.z);
    if (leave <= enter) discard;
    // Every marching sample is skipped beyond 1.35 times its local plume
    // width. Reject rays whose closest approach misses even the widest point;
    // this avoids depth work, curl sampling, and the full march in box corners.
    float closest_t = clamp(
        -dot(ro.xy, rd.xy) / max(dot(rd.xy, rd.xy), 1e-8), enter, leave);
    vec2 closest_xy = ro.xy + rd.xy * closest_t;
    float max_width = max(radius * (1.0 + 0.12 * spool) + length_m * 0.055, 0.05);
    float miss_radius = 1.35 * max_width;
    if (dot(closest_xy, closest_xy) > miss_radius * miss_radius) discard;
    vec4 clip = ubo.viewProj * vec4(ubo.campos.xyz + ray * max(enter, 0.001), 1.0);
    gl_FragDepth = clamp(clip.z / clip.w, 0.0, 1.0);

    bool found_density = false;
    // Spool-tiered march length: idle vapor converges in few steps, full
    // burner keeps 32. Spool is uniform, so the break stays coherent.
    int spool_steps = spool > 0.66 ? 32 : (spool > 0.25 ? 28 : 24);
    // Interval-proportional steps keep a ~0.5 m stride. Side views cross only
    // a few metres; axial views retain up to 32 samples for shock-cell detail.
    float interval = max(leave - enter, 0.0);
    int interval_steps = clamp(int(interval / 0.5), 12, 32);
    int live_steps = min(spool_steps, interval_steps);
    float step_m = (leave - enter) / float(live_steps);
    float trans = 1.0;
    vec3 radiance = vec3(0.0);
    float lambda = max(ubo.groundBase.w, 0.25);
    float flick = 0.90 + 0.06 * sin(time * 57.0) + 0.04 * sin(time * 91.0);
    float phase = 0.7 * phaseHg(dot(-ray, ubo.sunDir.xyz), 0.65)
                + 0.3 * phaseCs(dot(-ray, ubo.sunDir.xyz), 0.55);
    // Single domain-warp fetch at the ray midpoint, reused for all steps.
    // The warp field is low frequency; per-step variation was subtle shimmer
    // at 1 fetch/step. Hoisting cuts a full texture fetch from every step.
    vec3 pmid = ro + rd * (enter + leave) * 0.5;
    vec2 warp = texture(sampler2D(curl_tex, curl_smp),
        pmid.xy * 0.5 + vec2(pmid.z * 0.12 - time * 0.7, time * 0.13)).rg * 2.0 - 1.0;
    for (int i = 0; i < 32; ++i) {
        if (i >= live_steps) break;
        vec3 p = ro + rd * (enter + (float(i) + 0.5) * step_m);
        float axial = p.z / length_m;
        float band = 0.5 + 0.5 * cos(6.2831853 * p.z / lambda);
        float cell = band * band * band * exp(-p.z * 0.28) * spool;
        float width = radius * (1.0 + 0.12 * sin(p.z * 6.2831853 / lambda) * spool)
                    + p.z * 0.055;
        // Analytic miss first: warp can only pull samples 0.24*width inward,
        // so anything past 1.35 widths never contributes. Skips all fetches.
        if (length(p.xy) / max(width, 0.05) > 1.35) continue;
        vec2 cross_p = p.xy + warp * width * 0.24 * smoothstep(0.0, 1.5, p.z);
        float radial = length(cross_p) / max(width, 0.05);
        if (radial >= 1.0) continue;
        vec3 uvw = vec3(cross_p * 1.6, p.z * 0.38 - time * (2.5 + spool * 4.0));
        vec4 base = texture(sampler3D(base_vol, base_smp), uvw);
        // Detail breakup from the base volume's own high-frequency Worley
        // channel (B, 16 cells) at the same UV: no second 3D fetch.
        float detail = base.b;
        float envelope = exp(-radial * radial * 3.0) * (1.0 - smoothstep(0.65, 1.0, radial));
        float tail = 1.0 - smoothstep(0.35, 1.0, axial + (base.g - 0.5) * 0.25);
        float structure = smoothstep(0.23, 0.72, base.r * 0.65 + detail * 0.35);
        float dens = envelope * tail * mix(0.85, structure * 1.8, smoothstep(0.1, 1.2, p.z));
        if (!found_density && dens > 0.01) {
            vec3 first = ubo.campos.xyz + ray * (enter + (float(i) + 0.5) * step_m);
            vec4 first_clip = ubo.viewProj * vec4(first, 1.0);
            gl_FragDepth = clamp(first_clip.z / first_clip.w, 0.0, 1.0);
            found_density = true;
        }
        float temp = mix(900.0, 800.0 + spool * 1300.0, exp(-p.z * 0.22)) + cell * 700.0;
        vec3 emit = blackbody(temp) * (1.2 + cell * 3.2) * flick;
        vec3 chem = vec3(0.35, 0.5, 1.0) * cell * exp(-p.z * 0.9) * 2.0;
        vec3 scatter = ubo.sunColor.rgb * phase * 0.15 + ubo.skyHorizon.rgb * 0.12;
        float a = 1.0 - exp(-dens * 2.0 * step_m);
        radiance += trans * a * (emit + chem + scatter);
        trans *= 1.0 - a;
        if (trans < 0.01) break;
    }
    float alpha = 1.0 - trans;
    if (alpha < 0.001) discard;
    // Existing pipeline uses straight alpha; integration above is premultiplied.
    outColor = vec4(radiance / alpha, alpha);
}
