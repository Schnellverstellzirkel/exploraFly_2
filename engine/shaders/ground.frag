#version 450

// Analytic infinite flat-ground material. The plane is still intersected in
// this fullscreen pass, but its material is kept separate from sky.frag so
// the sky path remains cheap on pixels that never hit the ground.
//
// The material is a temperate meadow/soil layer rather than a single noisy
// RGB value. Its procedural channels are world-stable, filtered by the
// camera-ray footprint, and evaluated as linear-light PBR inputs:
//   - broad value noise makes grass, exposed soil, and damp patches;
//   - smaller noise controls albedo, roughness, and a micro-relief normal;
//   - Burley diffuse + GGX specular receive atmospheric sun and sky light;
//   - indirect light is occluded, while direct sun receives a soft aircraft
//     shadow using the actual sun angular radius.

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
    vec4 trailShift;
    vec4 cameraParams;
    vec4 cameraParams2;
    // xy: floor(abs world origin XZ / 0.25 m); zw: positive remainder.
    // Splitting the anchor avoids losing near-camera detail to float ULPs.
    vec4 groundOrigin;
} ubo;

layout(location = 0) in vec3 vRay;
layout(location = 0) out vec4 outColor;

const float PI = 3.141592653589793;
const float GROUND_FINE_CELL = 0.25;
// Hash coordinates wrap only after 2^20 cells. This is a deterministic,
// seamless fallback period, not the visible 0.25 m detail scale.
const float GROUND_HASH_PERIOD = 1048576.0;

vec3 fastSkyAtmosphere(float y) {
    vec3 zenith_sky = ubo.skyZenith.rgb;
    vec3 horizon_haze = ubo.skyHorizon.rgb;
    if (y >= 0.0) {
        float u = 1.0 - clamp(y, 0.0, 1.0);
        float u2 = u * u;
        float sky_factor = u2 * u * (0.85 * u + 0.15);
        return mix(zenith_sky, horizon_haze, sky_factor);
    } else {
        vec3 ground_base = ubo.groundBase.rgb;
        float h = clamp(1.0 + y * 3.5, 0.0, 1.0);
        return mix(ground_base, horizon_haze * 0.88, h * h);
    }
}

uint groundHashU(uvec2 cell, uint seed) {
    uint h = cell.x * 1664525u + cell.y * 1013904223u + seed * 2246822519u;
    h ^= h >> 16;
    h *= 2246822519u;
    h ^= h >> 13;
    h *= 3266489917u;
    return h ^ (h >> 16);
}

float groundHash(vec2 cell, uint seed) {
    // Wrapping the integer cell before hashing makes the very long fallback
    // period seamless while retaining exact integer behaviour for negatives.
    vec2 wrapped = mod(cell, vec2(GROUND_HASH_PERIOD));
    uvec2 integer_cell = uvec2(wrapped);
    return float(groundHashU(integer_cell, seed) & 0x00ffffffu)
        * (1.0 / 16777215.0);
}

float groundValueNoise(vec2 local_m, float cell_m, uint seed) {
    // Reconstruct the absolute cell index from the split origin. Every
    // supported scale is an integer multiple of the 25 cm anchor cell.
    float cells_per_scale = cell_m / GROUND_FINE_CELL;
    vec2 coarse_cell = floor(ubo.groundOrigin.xy / cells_per_scale);
    vec2 coarse_remainder = mod(ubo.groundOrigin.xy, vec2(cells_per_scale))
        * GROUND_FINE_CELL + ubo.groundOrigin.zw;
    vec2 q = (local_m + coarse_remainder) / cell_m;
    vec2 cell = coarse_cell + floor(q);
    vec2 f = fract(q);
    vec2 w = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);

    float n00 = groundHash(cell, seed);
    float n10 = groundHash(cell + vec2(1.0, 0.0), seed);
    float n01 = groundHash(cell + vec2(0.0, 1.0), seed);
    float n11 = groundHash(cell + vec2(1.0, 1.0), seed);
    float nx0 = mix(n00, n10, w.x);
    float nx1 = mix(n01, n11, w.x);
    return mix(nx0, nx1, w.y);
}

float groundFilteredNoise(vec2 local_m, float cell_m, float footprint, uint seed) {
    // A procedural texture has no hardware mip chain. Blend detail to its
    // analytic mean once a pixel covers the feature, avoiding shimmer and
    // preserving the multiscale behaviour described by Zirr/Kaplanyan 2016.
    float coverage = 1.0 - smoothstep(cell_m * 0.45, cell_m * 1.8, footprint);
    if (coverage <= 0.0) {
        return 0.5;
    }
    float raw = groundValueNoise(local_m, cell_m, seed);
    return mix(0.5, raw, coverage);
}

float groundRelief(vec2 local_m, float footprint) {
    // One filtered octave is enough for a stable normal. Albedo still carries
    // the finer 50 cm breakup, while avoiding four extra hashes per sample.
    float grain = groundFilteredNoise(local_m, 1.5, footprint, 137u);
    return (grain - 0.5) * 0.045;
}

vec3 groundNormal(vec2 local_m, float footprint) {
    // When footprint exceeds the grain filter band (2.7 m), relief is mathematically
    // flat (coverage == 0.0). Pruning the 4 relief samples saves 8 noise evaluations
    // for >99% of ground pixels.
    if (footprint > 2.7) {
        return vec3(0.0, 1.0, 0.0);
    }
    float step_m = clamp(0.10 + footprint * 0.35, 0.10, 0.40);
    float left = groundRelief(local_m - vec2(step_m, 0.0), footprint);
    float right = groundRelief(local_m + vec2(step_m, 0.0), footprint);
    float down = groundRelief(local_m - vec2(0.0, step_m), footprint);
    float up = groundRelief(local_m + vec2(0.0, step_m), footprint);
    return normalize(vec3(
        (left - right) / (2.0 * step_m),
        1.0,
        (down - up) / (2.0 * step_m)));
}

vec3 groundSkyIrradiance(vec3 n) {
    // Analytic hemisphere irradiance from the smooth vertical sky gradient.
    // Evaluating the 1D curve at zenith and skirt directions matches 3D hemisphere
    // integration with zero vector basis or trigonometric overhead.
    float ny = clamp(n.y, 0.0, 1.0);
    vec3 sky_zenith = fastSkyAtmosphere(ny);
    vec3 sky_skirt = fastSkyAtmosphere(ny * 0.78);
    return (sky_zenith * 0.3333333 + sky_skirt * 0.6666667) * (PI * 0.70);
}

float pow5(float x) {
    float x2 = x * x;
    return x2 * x2 * x;
}

vec3 fresnel(vec3 f0, float cosine) {
    return f0 + (vec3(1.0) - f0) * pow5(1.0 - clamp(cosine, 0.0, 1.0));
}

float groundAircraftShadow(vec2 local_xz, float ground_y) {
    // The aircraft is the only current shadow caster. Its projected ellipse
    // grows with distance according to the sun's angular radius, providing a
    // useful low-altitude shadow without pretending a shadow map exists.
    float sun_y = ubo.sunDir.y;
    if (sun_y <= 0.02) {
        return 0.0;
    }
    vec3 aircraft = ubo.nodes[0][3].xyz;
    float light_t = (ground_y - aircraft.y) / (-sun_y);
    if (light_t <= 0.0) {
        return 0.0;
    }
    vec2 center = aircraft.xz - ubo.sunDir.xz * light_t;
    vec2 delta = local_xz - center;
    float penumbra = light_t * ubo.sunDir.w;
    float major = 9.0 + penumbra * 1.10;
    float max_dist = major * 1.05;
    if (dot(delta, delta) > max_dist * max_dist) {
        return 0.0;
    }
    vec2 axis = ubo.nodes[0][2].xz;
    if (dot(axis, axis) < 1e-4) {
        axis = vec2(0.0, 1.0);
    }
    axis = normalize(axis);
    vec2 side = vec2(-axis.y, axis.x);
    float minor = 3.5 + penumbra * 0.70;
    float ellipse = length(vec2(dot(delta, axis) / major, dot(delta, side) / minor));
    return 1.0 - smoothstep(0.62, 1.05, ellipse);
}

void main() {
    vec3 view_dir = normalize(vRay);
    float denom = view_dir.y;
    if (abs(denom) <= 1e-5) {
        discard;
    }

    float hit_t = (ubo.groundBase.w - ubo.campos.y) / denom;
    if (hit_t <= 0.0) {
        discard;
    }
    if (hit_t > 150000.0) {
        outColor = vec4(fastSkyAtmosphere(view_dir.y), 1.0);
        return;
    }

    vec3 hit = ubo.campos.xyz + view_dir * hit_t;
    vec4 clip = ubo.viewProj * vec4(hit, 1.0);
    if (clip.w <= 0.0) {
        discard;
    }
    // Make the implicit surface participate in the same depth buffer as the
    // airframe and later terrain objects.
    gl_FragDepth = clamp(clip.z / clip.w, 0.0, 0.999999);

    vec2 local_xz = hit.xz;
    float footprint = max(length(dFdx(local_xz)), length(dFdy(local_xz)));
    footprint = clamp(footprint, 0.0, 100000.0);

    float macro = groundFilteredNoise(local_xz, 96.0, footprint, 11u);
    float patch_noise = groundFilteredNoise(local_xz, 24.0, footprint, 29u);
    float clump = groundFilteredNoise(local_xz, 6.0, footprint, 53u);
    float grain = groundFilteredNoise(local_xz, 1.5, footprint, 79u);
    float pebble = groundFilteredNoise(local_xz, 0.5, footprint, 107u);

    float grass_mask = smoothstep(0.33, 0.67,
        macro * 0.55 + patch_noise * 0.30 + clump * 0.15);
    float soil_mask = smoothstep(0.52, 0.84, patch_noise * 0.55 + clump * 0.45)
        * (1.0 - grass_mask * 0.45);
    float pebble_mask = smoothstep(0.80, 0.96, pebble) * (1.0 - grass_mask * 0.50);
    float wetness = smoothstep(0.70, 0.92, patch_noise) * (1.0 - grass_mask * 0.45);

    vec3 grass = mix(vec3(0.035, 0.070, 0.018), vec3(0.125, 0.185, 0.050), clump);
    vec3 soil = mix(vec3(0.055, 0.031, 0.014), vec3(0.185, 0.090, 0.030), grain);
    vec3 albedo = mix(grass, soil, soil_mask);
    albedo = mix(albedo, vec3(0.16, 0.145, 0.105), pebble_mask * 0.24);
    albedo *= 0.88 + macro * 0.18 + grain * 0.08;
    // A thin water film lowers apparent diffuse albedo and shifts it toward a
    // neutral dark reflection; its stronger effect is the roughness change.
    albedo *= mix(vec3(1.0), vec3(0.70, 0.76, 0.73), wetness * 0.25);

    float roughness = clamp(mix(0.90, 0.34, wetness) + pebble_mask * 0.04, 0.26, 0.94);
    float ao = clamp(0.86 + grass_mask * 0.10 - pebble_mask * 0.12, 0.65, 1.0);
    vec3 n = groundNormal(local_xz, footprint);
    vec3 v = normalize(ubo.campos.xyz - hit);
    float no_v = max(dot(n, v), 0.001);
    vec3 sun = normalize(ubo.sunDir.xyz);
    float no_l = max(dot(n, sun), 0.0);
    vec3 f0 = vec3(0.020);

    vec3 sky_irradiance = groundSkyIrradiance(n);
    vec3 color = albedo * (vec3(1.0) - fresnel(f0, no_v)) * sky_irradiance * ao;

    vec3 reflected = reflect(-v, n);
    vec3 env_dir = normalize(mix(reflected, n, roughness * roughness * 0.85));
    vec3 env = fastSkyAtmosphere(env_dir.y);
    float spec_ao = clamp(
        pow(no_v + ao, exp2(-16.0 * roughness - 1.0)) - 1.0 + ao,
        0.0, 1.0);
    color += env * fresnel(f0, no_v) * spec_ao * (0.45 + 0.55 * (1.0 - roughness));

    if (no_l > 0.0) {
        vec3 h = normalize(v + sun);
        float no_h = max(dot(n, h), 0.001);
        float vo_h = max(dot(v, h), 0.0);
        float alpha = roughness * roughness;
        float alpha2 = alpha * alpha;
        float d_denom = no_h * no_h * (alpha2 - 1.0) + 1.0;
        float d = alpha2 / max(PI * d_denom * d_denom, 1e-5);
        float k = (roughness + 1.0) * (roughness + 1.0) * 0.125;
        float gv = no_v / max(no_v * (1.0 - k) + k, 1e-5);
        float gl = no_l / max(no_l * (1.0 - k) + k, 1e-5);
        vec3 f = fresnel(f0, vo_h);
        vec3 direct_spec = f * (d * gv * gl / max(4.0 * no_v * no_l, 1e-5))
            * ubo.sunColor.rgb * no_l;

        float fd90 = 0.5 + 2.0 * roughness * vo_h * vo_h;
        float fd_v = 1.0 + (fd90 - 1.0) * pow5(1.0 - no_v);
        float fd_l = 1.0 + (fd90 - 1.0) * pow5(1.0 - no_l);
        vec3 direct_diffuse = albedo * (vec3(1.0) - fresnel(f0, no_l))
            * ubo.sunColor.rgb * (no_l * fd_v * fd_l);

        float shadow = groundAircraftShadow(local_xz, ubo.groundBase.w);
        float visibility = 1.0 - shadow * 0.30;
        color += (direct_diffuse + direct_spec) * visibility;
    }

    // Atmospheric perspective attenuates the ground toward the same horizon
    // radiance used by the sky, with slightly stronger blue extinction.
    vec3 haze = fastSkyAtmosphere(view_dir.y);
    vec3 transmittance = exp(-hit_t * vec3(0.000022, 0.000030, 0.000043));
    outColor = vec4(mix(haze, color, transmittance), 1.0);
}
