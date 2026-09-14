#version 450

// Exhaust plume raymarch. Same terms as the previous WGSL version:
// residual ratio tracking approx, Nubis remap density, Prandtl shock cells,
// dual temperature emission plus chemiluminescence, HG plus Cornette-Shanks.

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
    float lambda = max(ubo.groundBase.w, 0.25);
    float time = ubo.flex.y;
    vec3 sun_dir = ubo.sunDir.xyz;
    vec3 view = normalize(ubo.campos.xyz - vWorld);
    float mu_sun = dot(view, sun_dir);

    float march_len = mix(2.0, 9.0, clamp(vAxial, 0.0, 1.0)) * (0.6 + 0.4 * spool);
    float trans = 1.0;
    vec3 radiance = vec3(0.0);
    const uint steps = 10u;
    float flick = 0.85 + 0.15 * (sin(time * 57.0 + vAxial * 9.0) * 0.5 + sin(time * 91.0) * 0.3 + sin(time * 23.0 + vRadial * 5.0) * 0.2);
    for (uint s = 0u; s < steps; s += 1u) {
        float t = (float(s) + 0.5) / float(steps);
        float adv = t * march_len;
        vec3 p = vWorld - view * (t - 0.5) * march_len * 0.35;
        // Curl warp (Nubis 2015): 2D divergence-free offset bends sample
        // positions for swirl without a velocity grid. Grows downstream.
        vec2 warp = (texture(sampler2D(curl_tex, curl_smp), fract(vec2(vAxial * 1.7, t * 2.3) + time * 0.015)).rg - 0.5) * (0.25 + vAxial * 0.55);
        vec3 uvw = vec3(vAxial - time * 0.35 - adv * 0.06 + warp.x * 0.2, vRadial + warp.y * 0.2, t);
        vec4 base = texture(sampler3D(base_vol, base_smp), fract(uvw));
        vec4 det = texture(sampler3D(detail_vol, detail_smp), fract(uvw * 2.3 + 0.17));
        float coverage = clamp(0.35 + spool * 0.5 - vAxial * 0.55 - vRadial * 0.45, 0.0, 1.0);
        float dens = base.r * (1.0 - coverage) + (base.g * 0.6 + base.b * 0.4) * coverage;
        dens = remapRange(dens, det.r * 0.55, 1.0, 0.0, 1.0);
        // Shell proxy draws front and back faces over each other: coverage
        // is denser at the cone center than at the silhouette, which gives
        // soft edges without interior vertices. Axial decay only.
        dens *= 1.0 - vAxial * 0.75;
        if (dens < 0.004) {
            continue;
        }
        float x_m = vAxial * mix(3.0, 13.0, spool);
        float band = 0.5 + 0.5 * cos(6.2831853 * x_m / max(lambda, 0.2));
        float cell = pow(band, 3.0) * exp(-x_m * 0.28) * step(0.05, lambda - 0.01);
        float temp = mix(900.0, 800.0 + spool * 1300.0, exp(-x_m * 0.22)) + cell * 700.0 * spool;
        vec3 emit = blackbody(temp) * (dens * (1.2 + cell * 3.2 * spool) * flick);
        vec3 chem = vec3(0.35, 0.5, 1.0) * cell * exp(-x_m * 0.9) * spool * dens * 2.0;
        float shadow = exp(-dens * 2.2 * (0.5 + 0.5 * vAxial));
        float phase = 0.7 * phaseHg(mu_sun, 0.65) + 0.3 * phaseCs(mu_sun, 0.55);
        vec3 scatter = (ubo.sunColor.rgb * phase * shadow + ubo.skyHorizon.rgb * 0.25) * dens;
        float mu_t = dens * 3.0 + 0.02;
        float mu_bar = 3.2;
        float a = 1.0 - exp(-mu_t * march_len / float(steps));
        radiance += trans * (emit + chem + scatter * 0.6);
        trans *= 1.0 - a * (mu_t / mu_bar);
        if (trans < 0.02) {
            break;
        }
    }
    float alpha = clamp(1.0 - trans, 0.0, 1.0);
    outColor = vec4(radiance, alpha);
}
