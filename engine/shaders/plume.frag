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

const float WARP_BOUND = 1.35;

float remapRange(float x, float a, float b, float c, float d) {
    return c + (d - c) * clamp((x - a) / max(b - a, 1e-5), 0.0, 1.0);
}

float phaseHg(float mu, float g) {
    float gg = g * g;
    float d = max(1.0 + gg - 2.0 * g * mu, 1e-4);
    return (1.0 - gg) / (12.566371 * d * sqrt(d));
}

float phaseCs(float mu, float g) {
    float gg = g * g;
    float p1 = 1.5 * (1.0 - gg) / (2.0 + gg);
    float d = max(1.0 + gg - 2.0 * g * mu, 1e-4);
    float p2 = (1.0 + mu * mu) / (d * sqrt(d));
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
    // Conservative axial hull margin: the proxy bounding box extends 35%
    // past nominal fluid length so the downstream polygonal end cap is
    // placed strictly in empty space where fluid density and glow are zero.
    float proxy_len = length_m * 1.35;
    float radius = ubo.campos.w;
    float max_width = max(radius * (1.0 + 0.12 * abs(spool)) + proxy_len * 0.055, 0.05);
    float miss_radius = WARP_BOUND * max_width;
    float bound = miss_radius * 1.10;
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
    vec3 t1 = (vec3(bound, bound, proxy_len) - ro) / safe_rd;
    vec3 lo = min(t0, t1), hi = max(t0, t1);
    float enter = max(max(lo.x, lo.y), max(lo.z, 0.0));
    float leave = min(min(hi.x, hi.y), hi.z);
    if (leave <= enter) discard;
    // The generated curl field components are bounded to +/-1.0, so
    // 1.35 widths conservatively contain every warped contributing ray.
    // Reject rays beyond this envelope before the full march.
    float closest_t = clamp(
        -dot(ro.xy, rd.xy) / max(dot(rd.xy, rd.xy), 1e-8), enter, leave);
    vec2 closest_xy = ro.xy + rd.xy * closest_t;
    if (dot(closest_xy, closest_xy) > miss_radius * miss_radius) discard;

    // Spool-tiered march length: idle vapor converges in fewer steps, while
    // the full burner uses 18 samples. Spool is uniform across all pixels,
    // ensuring identical step counts and continuous, tear-free integration
    // across the entire volume proxy without integer-slicing artifacts.
    int live_steps = spool > 0.66 ? 18 : (spool > 0.25 ? 18 : 14);
    float interval = max(leave - enter, 0.0);
    float step_m = interval / float(live_steps);
    float trans = 1.0;
    vec3 radiance = vec3(0.0);
    float lambda = max(ubo.detail.y, 0.25);
    float phase_scale = 6.2831853 / lambda;
    // The CPU already evaluates the shared flicker once per present and packs
    // it in detail.x; avoid two transcendental calls for every plume pixel.
    float flick = ubo.detail.x;
    float mu = dot(-ray, ubo.sunDir.xyz);
    float phase = 0.7 * phaseHg(mu, 0.65)
                + 0.3 * phaseCs(mu, 0.55);
    vec3 scatter = ubo.sunColor.rgb * phase * 0.15 + ubo.skyHorizon.rgb * 0.12;
    // Samples advance at a fixed axial interval along each ray. Carry the
    // axial exponentials and shock-cell sine/cosine forward instead of
    // recomputing transcendental functions for every sample.
    float z0 = ro.z + rd.z * (enter + 0.5 * step_m);
    float dz = rd.z * step_m;
    float cell_e = exp(-z0 * 0.28);
    float cell_step = exp(-dz * 0.28);
    float temp_e = exp(-z0 * 0.22);
    float temp_step = exp(-dz * 0.22);
    float chem_e = exp(-z0 * 0.9);
    float chem_step = exp(-dz * 0.9);
    vec2 sc = vec2(sin(z0 * phase_scale), cos(z0 * phase_scale));
    vec2 dsc = vec2(sin(dz * phase_scale), cos(dz * phase_scale));
    // Single domain-warp fetch at the ray midpoint, reused for all steps.
    // The warp field is low frequency; per-step variation was subtle shimmer
    // at 1 fetch/step. Hoisting cuts a full texture fetch from every step.
    vec3 pmid = ro + rd * (enter + leave) * 0.5;
    vec2 warp = textureLod(sampler2D(curl_tex, curl_smp),
        pmid.xy * 0.5 + vec2(pmid.z * 0.12 - time * 0.7, time * 0.13), 0.0).rg * 2.0 - 1.0;
    for (int i = 0; i < 18; ++i) {
        if (i >= live_steps) break;
        vec3 p = ro + rd * (enter + (float(i) + 0.5) * step_m);
        // Analytic miss first: warp can only pull samples 0.24*width inward,
        // so anything past 1.18 widths never contributes. Use an upper bound
        // before the expensive phase, trigonometry, and density math.
        float width_upper = max(
            radius * (1.0 + 0.12 * abs(spool)) + p.z * 0.055, 0.05);
        float miss = WARP_BOUND * width_upper;
        if (dot(p.xy, p.xy) > miss * miss) {
            sc = vec2(sc.x * dsc.y + sc.y * dsc.x,
                      sc.y * dsc.y - sc.x * dsc.x);
            cell_e *= cell_step;
            temp_e *= temp_step;
            chem_e *= chem_step;
            continue;
        }
        float width = radius * (1.0 + 0.12 * sc.x * spool)
                    + p.z * 0.055;
        float inv_width = 1.0 / max(width, 0.05);
        vec2 cross_p = p.xy + warp * width * 0.24 * smoothstep(0.0, 1.5, p.z);
        float radial2 = dot(cross_p, cross_p) * inv_width * inv_width;
        if (radial2 >= 1.0) {
            sc = vec2(sc.x * dsc.y + sc.y * dsc.x,
                      sc.y * dsc.y - sc.x * dsc.x);
            cell_e *= cell_step;
            temp_e *= temp_step;
            chem_e *= chem_step;
            continue;
        }
        float radial = sqrt(radial2);
        float axial = p.z / length_m;
        // Early skip for samples in the conservative buffer zone past the fluid:
        if (axial >= 1.0) {
            sc = vec2(sc.x * dsc.y + sc.y * dsc.x,
                      sc.y * dsc.y - sc.x * dsc.x);
            cell_e *= cell_step;
            temp_e *= temp_step;
            chem_e *= chem_step;
            continue;
        }
        // Conical 3D supersonic shock diamonds:
        // Confined strictly to the supersonic core (radial < 0.55), with a 3D conical
        // wavefront angled at the Prandtl Mach angle. The outer shear layer (radial >= 0.55)
        // is smooth turbulent fluid with zero shock oscillation, completely eliminating
        // any flat horizontal stripe/overlay artifacts.
        float cell = 0.0;
        if (radial < 0.55) {
            float core_falloff = exp(-radial * radial * 8.0) * (1.0 - smoothstep(0.20, 0.55, radial));
            float conical_phase = (p.z - radial * lambda * 0.30) * phase_scale;
            float diamond_wave = 0.5 + 0.5 * cos(conical_phase);
            cell = diamond_wave * diamond_wave * diamond_wave * cell_e * spool * core_falloff;
        }
        float envelope = exp(-radial * radial * 3.0) * (1.0 - smoothstep(0.65, 1.0, radial));
        if (envelope < 0.001) {
            sc = vec2(sc.x * dsc.y + sc.y * dsc.x,
                      sc.y * dsc.y - sc.x * dsc.x);
            cell_e *= cell_step;
            temp_e *= temp_step;
            chem_e *= chem_step;
            continue;
        }

        vec3 uvw = vec3(cross_p * 1.6, p.z * 0.38 - time * (2.5 + spool * 4.0));
        vec4 base = textureLod(sampler3D(base_vol, base_smp), uvw, 0.0);
        // Detail breakup from the base volume's own high-frequency Worley
        // channel (B, 16 cells) at the same UV: no second 3D fetch.
        float detail = base.b;

        // Turbulent fluid dissipation: shear-layer mixing forms organic wisps
        // and flame tongues that decay smoothly to strictly zero before axial = 1.0.
        // Tapering the noise offset near the tip prevents noise from pushing density past 1.0,
        // leaving a wide buffer zone before the conservative proxy hull ends at 1.35.
        float noise_wisp = (base.g - 0.5) * 0.35 * (1.0 - smoothstep(0.65, 0.95, axial));
        float axial_fade = smoothstep(0.35, 0.92, axial + noise_wisp);
        float tail = clamp(1.0 - axial_fade, 0.0, 1.0);

        float structure = smoothstep(0.23, 0.72, base.r * 0.65 + detail * 0.35);
        float dens = envelope * tail * mix(0.85, structure * 1.8, smoothstep(0.1, 1.2, p.z));
        if (dens > 0.001) {
            // Thermal incandescence cools down as gas dissipates into ambient air:
            float glow_decay = smoothstep(0.0, 0.35, tail);
            float temp = mix(900.0, 800.0 + spool * 1300.0, temp_e) + cell * 700.0;
            vec3 emit = blackbody(temp) * (1.2 + cell * 3.2) * flick * glow_decay;
            vec3 chem = vec3(0.35, 0.5, 1.0) * cell * chem_e * 2.0 * glow_decay;
            float a = 1.0 - exp(-dens * 2.0 * step_m);
            radiance += trans * a * (emit + chem + scatter);
            trans *= 1.0 - a;
            if (trans < 0.01) break;
        }
        sc = vec2(sc.x * dsc.y + sc.y * dsc.x,
                  sc.y * dsc.y - sc.x * dsc.x);
        cell_e *= cell_step;
        temp_e *= temp_step;
        chem_e *= chem_step;
    }
    float alpha = 1.0 - trans;
    if (alpha < 0.001) discard;
    // The integration is premultiplied; the plume pipeline uses ONE source
    // blending so no per-fragment reciprocal is needed here.
    outColor = vec4(radiance, alpha);
}
