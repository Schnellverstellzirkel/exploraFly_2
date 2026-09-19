#version 460

// Rasterized alpine landscape and medieval landmarks. Hardware depth handles
// mountain silhouettes and occlusion; no fragment terrain ray march is used.
//
// The material is a temperate meadow/soil layer rather than a single noisy
// RGB value. Vegetation distribution follows baked landform moisture,
// altitude, and slope: no patch-scale noise is used anywhere, so cover reads
// as valley meadows, slope forests, and exposed high ground instead of
// scattered blobs. Only sub-metre albedo texture still uses filtered noise.
// Its procedural inputs are world-stable, filtered by the
// camera-ray footprint, and evaluated as linear-light PBR inputs:
//   - landform moisture, altitude, and slope decide grass, soil, scree,
//     forest, snow, and damp ground;
//   - sub-metre noise controls albedo texture, roughness, and a micro-relief;
//   - Burley diffuse + GGX specular receive atmospheric sun and sky light;
//   - indirect light is occluded, while direct sun receives a soft aircraft
//     shadow and moving cloud shadows from the visible cloud density field.

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

layout(location = 0) in vec3 vPosition;
layout(location = 1) in float vLandHeight;
layout(location = 2) flat in uint vMaterial;
layout(location = 3) in vec3 vObjectPos;
layout(location = 4) in vec3 vTerrainNormal;
layout(location = 5) in float vMoisture;
layout(location = 0) out vec4 outColor;

// Photo detail textures (Poly Haven CC0 2K diffuse + normal sets)
layout(set = 0, binding = 12) uniform texture2D detail_meadow_diff_tex;
layout(set = 0, binding = 13) uniform texture2D detail_rock_diff_tex;
layout(set = 0, binding = 14) uniform texture2D detail_snow_diff_tex;
layout(set = 0, binding = 15) uniform texture2D detail_scree_diff_tex;

layout(set = 0, binding = 16) uniform texture2D detail_meadow_nor_tex;
layout(set = 0, binding = 17) uniform texture2D detail_rock_nor_tex;
layout(set = 0, binding = 18) uniform texture2D detail_snow_nor_tex;
layout(set = 0, binding = 19) uniform texture2D detail_scree_nor_tex;

layout(set = 0, binding = 20) uniform sampler detail_smp;

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
    if (footprint > 1.8) {
        return vec3(0.0, 1.0, 0.0);
    }
    float step_m = clamp(0.10 + footprint * 0.35, 0.10, 0.40);
    float center = groundRelief(local_m, footprint);
    float right = groundRelief(local_m + vec2(step_m, 0.0), footprint);
    float up = groundRelief(local_m + vec2(0.0, step_m), footprint);
    return normalize(vec3(
        (center - right) / step_m,
        1.0,
        (center - up) / step_m));
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
    // The aircraft's projected ellipse grows with distance according to the
    // sun's angular radius, providing a
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

#ifdef ENABLE_RT
layout(set = 0, binding = 5) uniform accelerationStructureEXT scene_tlas;

bool rtOccluded(vec3 origin, vec3 dir, float t_max) {
    rayQueryEXT q;
    rayQueryInitializeEXT(q, scene_tlas, gl_RayFlagsOpaqueEXT | gl_RayFlagsTerminateOnFirstHitEXT,
        0xFF, origin, 1e-4, dir, t_max);
    rayQueryProceedEXT(q);
    return rayQueryGetIntersectionTypeEXT(q, true) != gl_RayQueryCommittedIntersectionNoneEXT;
}

float rtSunVisibility(vec3 origin, float t_max) {
    vec3 sun = normalize(ubo.sunDir.xyz);
    float sun_radius = ubo.sunDir.w;
    
    // Sample 0: Center ray towards solar core
    if (!rtOccluded(origin, sun, t_max)) {
        vec3 up = (abs(sun.y) < 0.99) ? vec3(0.0, 1.0, 0.0) : vec3(1.0, 0.0, 0.0);
        vec3 s_t = normalize(cross(up, sun));
        vec3 s_b = cross(sun, s_t);
        float rot = fract(sin(dot(gl_FragCoord.xy, vec2(12.9898, 78.233))) * 43758.5453) * 6.2831853;
        float c_r = cos(rot);
        float s_r = sin(rot);
        vec2 p1 = SUN_POINTS[1];
        vec2 pr1 = vec2(p1.x * c_r - p1.y * s_r, p1.x * s_r + p1.y * c_r);
        vec3 dir1 = normalize(sun + (s_t * pr1.x + s_b * pr1.y) * sun_radius);
        if (!rtOccluded(origin, dir1, t_max)) {
            return 1.0; // Fully lit penumbra-free early exit
        }
        float lit = 1.0;
        for (uint i = 2u; i < SHADOW_RAYS; i += 1u) {
            vec2 p = SUN_POINTS[i];
            vec2 pr = vec2(p.x * c_r - p.y * s_r, p.x * s_r + p.y * c_r);
            vec3 dir = normalize(sun + (s_t * pr.x + s_b * pr.y) * sun_radius);
            if (!rtOccluded(origin, dir, t_max)) {
                lit += 1.0;
            }
        }
        return lit / float(SHADOW_RAYS);
    } else {
        vec3 up = (abs(sun.y) < 0.99) ? vec3(0.0, 1.0, 0.0) : vec3(1.0, 0.0, 0.0);
        vec3 s_t = normalize(cross(up, sun));
        vec3 s_b = cross(sun, s_t);
        float rot = fract(sin(dot(gl_FragCoord.xy, vec2(12.9898, 78.233))) * 43758.5453) * 6.2831853;
        float c_r = cos(rot);
        float s_r = sin(rot);
        vec2 p1 = SUN_POINTS[1];
        vec2 pr1 = vec2(p1.x * c_r - p1.y * s_r, p1.x * s_r + p1.y * c_r);
        vec3 dir1 = normalize(sun + (s_t * pr1.x + s_b * pr1.y) * sun_radius);
        if (rtOccluded(origin, dir1, t_max)) {
            return 0.0; // Deep umbra early exit
        }
        float lit = 0.0;
        for (uint i = 2u; i < SHADOW_RAYS; i += 1u) {
            vec2 p = SUN_POINTS[i];
            vec2 pr = vec2(p.x * c_r - p.y * s_r, p.x * s_r + p.y * c_r);
            vec3 dir = normalize(sun + (s_t * pr.x + s_b * pr.y) * sun_radius);
            if (!rtOccluded(origin, dir, t_max)) {
                lit += 1.0;
            }
        }
        return lit / float(SHADOW_RAYS);
    }
}

// Trace-region gate: expanded version of the analytic ellipse. Only pixels
// near the projected aircraft footprint run ray queries (and only when the
// sun is up). Expands for sun angle elongation (1/sun_y) and soft penumbra.
float rtShadowGate(vec2 local_xz, float ground_y) {
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
    float penumbra = light_t * ubo.sunDir.w;
    float gate = (20.0 + penumbra * 1.50) / max(sun_y, 0.15);
    if (dot(local_xz - center, local_xz - center) > gate * gate) {
        return 0.0;
    }
    return 1.0;
}
#endif

void main() {
    vec3 hit = vPosition;
    vec3 view_delta = hit - ubo.campos.xyz;
    float hit_t = length(view_delta);
    vec3 view_dir = view_delta / max(hit_t, 0.001);
    vec2 local_xz = hit.xz;
    vec2 world_xz = terrainOrigin(ubo.groundOrigin) + local_xz;
    float footprint = max(length(dFdx(local_xz)), length(dFdy(local_xz)));
    footprint = clamp(footprint, 0.0, 100000.0);

    vec3 n = normalize(cross(dFdx(hit), dFdy(hit)));
    if (vMaterial == 0u) n = normalize(vTerrainNormal);
    if (vMaterial != 0u && dot(n, -view_dir) < 0.0) n = -n;
    float slope = 1.0 - clamp(n.y, 0.0, 1.0);
    float altitude = hit.y - ubo.groundBase.w;
    float water = vMaterial == 0u ? 1.0 - smoothstep(TERRAIN_WATER - 0.5,
        TERRAIN_WATER + 1.5, vLandHeight) : 0.0;

    // Landform cover, not noise blobs: every distribution mask below is a
    // function of baked heightfield moisture, altitude, and slope, so meadows
    // sit on wet valley floors, forests follow moist mid-slopes and gullies,
    // and dry spurs and steeps stay exposed. Only sub-metre albedo texture
    // still uses noise; nothing at patch scale does.
    float moist = vMoisture;
    float grain = (footprint > 2.7) ? 0.5 : groundFilteredNoise(local_xz, 1.5, footprint, 79u);
    float pebble = (footprint > 0.9) ? 0.5 : groundFilteredNoise(local_xz, 0.5, footprint, 107u);

    float soil_mask = clamp((1.0 - moist) * 0.8
        + smoothstep(0.30, 0.60, slope) * 0.5, 0.0, 1.0);
    float grass_mask = 1.0 - soil_mask;
    float scree_mask = smoothstep(0.42, 0.68, slope);
    float wetness = smoothstep(0.72, 0.92, moist);

    vec3 albedo = vec3(0.15, 0.20, 0.10);
    float roughness = 0.80;
    float ao = 0.90;

    if (vMaterial == 0u) {
        vec3 world_pos = vec3(world_xz.x, altitude, world_xz.y);

        // 1. Organic domain warping eliminates rectilinear texture grid alignment
        vec2 uv_warp = vec2(
            groundFilteredNoise(local_xz, 24.0, footprint, 311u),
            groundFilteredNoise(local_xz + vec2(17.3, 11.9), 24.0, footprint, 419u)
        ) * 2.0 - 1.0;
        vec2 warped_xz = world_xz + uv_warp * 6.5;
        vec3 warped_pos = vec3(warped_xz.x, altitude, warped_xz.y);

        // 2. Dual-scale non-harmonic sampling for meadow (micro 19m, macro 37m rotated 37 degrees)
        const mat2 ROT_37 = mat2(0.7986, 0.6018, -0.6018, 0.7986);
        vec2 uv_meadow1 = warped_xz * (1.0 / 19.0);
        vec2 uv_meadow2 = (ROT_37 * (warped_xz + vec2(83.1, -47.6))) * (1.0 / 37.0);
        float meadow_blend = groundFilteredNoise(local_xz, 26.0, footprint, 439u);

        vec3 diff_meadow1 = texture(sampler2D(detail_meadow_diff_tex, detail_smp), uv_meadow1).rgb;
        vec3 diff_meadow2 = texture(sampler2D(detail_meadow_diff_tex, detail_smp), uv_meadow2).rgb;
        vec3 diff_meadow = mix(diff_meadow1, diff_meadow2, smoothstep(0.25, 0.75, meadow_blend));

        vec3 nor_meadow_raw1 = texture(sampler2D(detail_meadow_nor_tex, detail_smp), uv_meadow1).rgb * 2.0 - 1.0;
        vec3 nor_meadow_raw2 = texture(sampler2D(detail_meadow_nor_tex, detail_smp), uv_meadow2).rgb * 2.0 - 1.0;
        vec3 nor_meadow_raw = mix(nor_meadow_raw1, nor_meadow_raw2, smoothstep(0.25, 0.75, meadow_blend));

        // 3. Scree, Snow, and Rock UVs with domain warping
        vec2 uv_scree = warped_xz * (1.0 / 16.0);
        vec2 uv_snow  = warped_xz * (1.0 / 22.0);

        // Biplanar cliff projection for rock avoids vertical smearing on steep faces
        vec2 uv_rock_top = warped_xz * (1.0 / 20.0);
        float abs_nx = abs(n.x);
        float abs_nz = abs(n.z);
        float cliff_side_blend = abs_nx / (abs_nx + abs_nz + 1e-4);
        vec2 uv_rock_side_x = vec2(warped_pos.z, warped_pos.y) * (1.0 / 20.0);
        vec2 uv_rock_side_z = vec2(warped_pos.x, warped_pos.y) * (1.0 / 20.0);
        vec2 uv_rock_side = mix(uv_rock_side_z, uv_rock_side_x, smoothstep(0.35, 0.65, cliff_side_blend));
        float cliff_weight = smoothstep(0.38, 0.72, slope);

        // Sample diffuse maps
        vec3 diff_scree  = texture(sampler2D(detail_scree_diff_tex, detail_smp), uv_scree).rgb;
        vec3 diff_snow   = texture(sampler2D(detail_snow_diff_tex, detail_smp), uv_snow).rgb;
        vec3 diff_rock   = mix(
            texture(sampler2D(detail_rock_diff_tex, detail_smp), uv_rock_top).rgb,
            texture(sampler2D(detail_rock_diff_tex, detail_smp), uv_rock_side).rgb,
            cliff_weight
        );

        // Sample normal maps
        vec3 nor_scree_raw  = texture(sampler2D(detail_scree_nor_tex, detail_smp), uv_scree).rgb * 2.0 - 1.0;
        vec3 nor_snow_raw   = texture(sampler2D(detail_snow_nor_tex, detail_smp), uv_snow).rgb * 2.0 - 1.0;
        vec3 nor_rock_top   = texture(sampler2D(detail_rock_nor_tex, detail_smp), uv_rock_top).rgb * 2.0 - 1.0;
        vec3 nor_rock_side  = texture(sampler2D(detail_rock_nor_tex, detail_smp), uv_rock_side).rgb * 2.0 - 1.0;

        // 4. Orthonormal tangent basis for top-down and cliff projections
        vec3 t_h = vec3(1.0 - n.x * n.x, -n.x * n.y, -n.x * n.z);
        float len_th = length(t_h);
        vec3 T_h = len_th > 1e-4 ? t_h / len_th : vec3(1.0, 0.0, 0.0);
        vec3 B_h = cross(n, T_h);

        vec3 t_v = vec3(-n.y * n.x, 1.0 - n.y * n.y, -n.y * n.z);
        float len_tv = length(t_v);
        vec3 T_v = len_tv > 1e-4 ? t_v / len_tv : vec3(0.0, 1.0, 0.0);
        vec3 B_v = cross(n, T_v);

        // 5. Multi-scale distance fading for micro-textures
        float micro_fade = 1.0 - smoothstep(1.0, 5.0, footprint);
        float rock_fade  = 1.0 - smoothstep(16.0, 110.0, footprint);

        // Surface gradient perturbations
        vec3 grad_meadow = (T_h * nor_meadow_raw.x + B_h * nor_meadow_raw.y) * (0.50 * micro_fade);
        vec3 grad_scree  = (T_h * nor_scree_raw.x  + B_h * nor_scree_raw.y)  * (0.60 * micro_fade);
        vec3 grad_snow   = (T_h * nor_snow_raw.x   + B_h * nor_snow_raw.y)   * (0.35 * micro_fade);
        vec3 grad_rock_top  = (T_h * nor_rock_top.x  + B_h * nor_rock_top.y)  * (0.85 * rock_fade);
        vec3 grad_rock_side = (B_v * nor_rock_side.x + T_v * nor_rock_side.y) * (1.10 * rock_fade);
        vec3 grad_rock = mix(grad_rock_top, grad_rock_side, cliff_weight);

        // Geological horizontal strata banding on rock cliffs with distance-dependent normal smoothing
        float rock_strata = sin(altitude * 0.052) * 0.12 + sin(altitude * 0.125 + world_xz.x * 0.007) * 0.07;
        float strata_grad = (cos(altitude * 0.052) * 0.052 * 0.12
            + cos(altitude * 0.125 + world_xz.x * 0.007) * 0.125 * 0.07) * 12.0;
        float strata_fade = 1.0 - smoothstep(4.0, 32.0, footprint);
        grad_rock += T_v * (strata_grad * cliff_weight * strata_fade);

        // Meso-scale angular crags break up flat 64m mesh polygons on high slopes
        float crag_val = groundFilteredNoise(local_xz, 24.0, footprint, 211u) * 2.0 - 1.0;
        float crag_fine = groundFilteredNoise(local_xz, 8.0, footprint, 337u) * 2.0 - 1.0;
        vec3 grad_crag = (T_h * crag_val + B_h * crag_fine) * (0.35 * rock_fade);

        // 6. Solar aspect: sun-drenched warm slopes vs sheltered cool mossy slopes
        float sun_aspect = dot(n.xz, normalize(ubo.sunDir.xz));

        // Snow cover on alpine peaks (computed early for couloir and vegetation masks)
        float snowLine = 1860.0 + (0.5 - moist) * 220.0;
        float snow = smoothstep(snowLine, snowLine + 240.0, altitude)
            * (1.0 - smoothstep(0.34, 0.66, slope));

        // 7. Alpine Wildflower Belts: golden buttercups, blue gentians, and white edelweiss drifts
        float flower_zone = smoothstep(0.35, 0.68, moist) * (1.0 - smoothstep(0.12, 0.32, slope))
            * (1.0 - smoothstep(1200.0, 1650.0, altitude));
        float buttercup_n = groundFilteredNoise(local_xz, 28.0, footprint, 521u);
        float gentian_n   = groundFilteredNoise(local_xz + vec2(43.1, 71.7), 34.0, footprint, 631u);
        float edelweiss_n = groundFilteredNoise(local_xz - vec2(58.4, 23.9), 20.0, footprint, 743u);

        float buttercup_drift = smoothstep(0.56, 0.74, buttercup_n) * flower_zone;
        float gentian_drift   = smoothstep(0.60, 0.78, gentian_n) * flower_zone * (1.0 - buttercup_drift * 0.7);
        float edelweiss_drift = smoothstep(0.65, 0.82, edelweiss_n) * smoothstep(850.0, 1600.0, altitude) * (1.0 - slope);

        vec3 flower_albedo = vec3(0.54, 0.44, 0.08); // Golden buttercups
        flower_albedo = mix(flower_albedo, vec3(0.12, 0.22, 0.46), gentian_drift); // Blue gentian
        flower_albedo = mix(flower_albedo, vec3(0.70, 0.74, 0.68), edelweiss_drift); // White edelweiss
        float total_flowers = clamp(buttercup_drift + gentian_drift + edelweiss_drift, 0.0, 0.85);

        // 8. Glacial Erratics & Weathered Limestone Boulders in Meadows
        float boulder_n = groundFilteredNoise(local_xz, 14.0, footprint, 853u);
        float boulder_patch = smoothstep(0.66, 0.84, boulder_n) * (1.0 - wetness * 0.7)
            * (1.0 - smoothstep(1500.0, 1900.0, altitude));
        float shelf_rock = smoothstep(0.22, 0.44, slope) * smoothstep(0.46, 0.72, boulder_n);
        float meadow_boulders = clamp(boulder_patch * 0.90 + shelf_rock * 0.75, 0.0, 1.0);

        // 9. Cattle tracks & mountain footpaths contouring hillsides
        float path_phase = fract((altitude + groundFilteredNoise(local_xz, 36.0, footprint, 911u) * 14.0) / 20.0) - 0.5;
        float path = (1.0 - smoothstep(0.04, 0.12, abs(path_phase))) * smoothstep(0.16, 0.38, slope) * (1.0 - water);

        // 10. Avalanche chutes and couloirs on mountain sides
        float chute_n = groundFilteredNoise(vec2(local_xz.x * 0.07, local_xz.y * 0.02) * 16.0, 16.0, footprint, 1021u);
        float chute = smoothstep(0.58, 0.78, chute_n) * smoothstep(0.36, 0.62, slope) * (1.0 - snow);
        scree_mask = clamp(scree_mask + chute * 0.65, 0.0, 1.0);

        // 11. Saturated shoreline silt and muddy hollows
        float mud_mask = smoothstep(0.78, 0.94, moist) * (1.0 - smoothstep(0.10, 0.28, slope));
        vec3 mud_color = mix(vec3(0.055, 0.044, 0.034), vec3(0.085, 0.070, 0.052), grain);

        // 12. Painterly meadow palette with macro tone
        float macro_tone = mix(0.88, 1.12, groundFilteredNoise(local_xz, 128.0, footprint, 173u));
        float macro_patch = groundFilteredNoise(local_xz, 48.0, footprint, 241u);

        float meadow_lum = dot(diff_meadow, vec3(0.299, 0.587, 0.114));
        float meadow_detail = mix(1.0, clamp(meadow_lum / 0.125, 0.78, 1.28), micro_fade);

        vec3 grass_emerald = vec3(0.06, 0.26, 0.045);
        vec3 grass_golden  = vec3(0.20, 0.40, 0.080);
        vec3 grass_tawny   = vec3(0.26, 0.34, 0.110);

        float tone_select = moist + sun_aspect * 0.18 + (macro_patch - 0.5) * 0.25;
        vec3 grass_base = mix(grass_tawny, grass_golden, smoothstep(0.35, 0.60, tone_select));
        grass_base = mix(grass_base, grass_emerald, smoothstep(0.60, 0.85, tone_select));

        vec3 meadow_albedo = grass_base * meadow_detail;
        meadow_albedo = mix(meadow_albedo, flower_albedo * (0.85 + meadow_detail * 0.25), total_flowers);

        float scree_lum = dot(diff_scree, vec3(0.299, 0.587, 0.114));
        float scree_detail = mix(1.0, clamp(scree_lum / 0.24, 0.78, 1.28), micro_fade);
        vec3 soil_albedo = mix(vec3(0.14, 0.12, 0.09), vec3(0.22, 0.18, 0.14), grain) * scree_detail;
        meadow_albedo = mix(meadow_albedo, soil_albedo, soil_mask * 0.60);
        meadow_albedo = mix(meadow_albedo, soil_albedo * 1.15, path * 0.70);
        meadow_albedo = mix(meadow_albedo, mud_color, mud_mask * 0.85);

        // Boulders in meadows with orange sun-lichen
        vec3 boulder_albedo = mix(vec3(0.22, 0.23, 0.24), vec3(0.36, 0.37, 0.38), grain);
        float lichen = smoothstep(0.40, 0.75, grain) * smoothstep(0.0, 0.4, sun_aspect) * boulder_patch;
        boulder_albedo = mix(boulder_albedo, vec3(0.44, 0.28, 0.08), lichen * 0.65);
        meadow_albedo = mix(meadow_albedo, boulder_albedo, meadow_boulders);
        meadow_albedo *= macro_tone;

        // Scree slopes: gravel and talus fans beneath cliff faces
        vec3 scree_albedo = mix(vec3(0.24, 0.23, 0.21), vec3(0.32, 0.30, 0.27), grain) * scree_detail;
        albedo = mix(meadow_albedo, scree_albedo, scree_mask);
        vec3 grad_terrain = mix(grad_meadow, grad_scree, scree_mask);
        grad_terrain = mix(grad_terrain, grad_rock, meadow_boulders * 0.75);

        // Rock cliffs and high alpine plates
        float rock_lum = dot(diff_rock, vec3(0.299, 0.587, 0.114));
        float rock_factor = clamp(rock_lum / 0.075, 0.4, 2.0);
        vec3 rock_tint = mix(vec3(0.16, 0.17, 0.18), vec3(0.34, 0.35, 0.36), clamp(rock_factor * 0.5, 0.0, 1.0));
        vec3 rock_albedo = mix(rock_tint, diff_rock * vec3(1.7, 2.3, 2.7), 0.35) * (1.0 + rock_strata);
        float rock_mask = clamp(smoothstep(0.50, 0.85, slope) + smoothstep(1300.0, 2100.0, altitude) * 0.30, 0.0, 1.0);
        albedo = mix(albedo, rock_albedo, rock_mask);
        grad_terrain = mix(grad_terrain, grad_rock + grad_crag, rock_mask);

        // Subalpine forest canopy on moist mid-slopes
        float treeline = 1500.0 + (moist - 0.5) * 320.0;
        float forest = smoothstep(0.40, 0.58, moist)
            * smoothstep(300.0, 520.0, altitude)
            * (1.0 - smoothstep(treeline, treeline + 170.0, altitude))
            * (1.0 - smoothstep(0.55, 0.90, slope));
        vec3 forest_albedo = mix(vec3(0.020, 0.078, 0.035), vec3(0.042, 0.135, 0.058), grain);
        albedo = mix(albedo, forest_albedo, forest * 0.88);
        vec3 grad_forest = (T_h * (grain - 0.5) + B_h * (pebble - 0.5)) * (0.75 * micro_fade);
        grad_terrain = mix(grad_terrain, grad_forest, forest * 0.80);

        // Snow cover on alpine peaks
        vec3 snow_albedo = clamp(diff_snow * 2.15, 0.0, 0.94) * vec3(0.96, 0.98, 1.0);
        albedo = mix(albedo, snow_albedo, snow);
        grad_terrain = mix(grad_terrain, grad_snow, snow);

        // Winding valley road
        float valleyX = mod(world_xz.x - terrainValleyCenter(mod(world_xz.y, TERRAIN_PERIOD))
            + 8192.0, 16384.0) - 8192.0;
        float roadDistance = abs(valleyX - 1050.0);
        float road = 1.0 - smoothstep(9.0, 13.0 + footprint, roadDistance);
        road *= (1.0 - smoothstep(0.2, 0.4, slope)) * (1.0 - water);
        albedo = mix(albedo, diff_scree * vec3(1.15, 0.98, 0.78), road * 0.85);

        // Wetness darkening
        albedo *= mix(vec3(1.0), vec3(0.70, 0.76, 0.73), wetness * 0.25);

        // Physical material roughness
        roughness = mix(0.78, 0.84, rock_mask);
        roughness = mix(roughness, 0.88, scree_mask);
        roughness = mix(roughness, 0.92, forest * 0.88);
        roughness = mix(roughness, 0.52, snow);
        roughness = mix(roughness, 0.18, mud_mask * 0.85);
        roughness = mix(roughness, 0.30, wetness);

        // Ambient occlusion and micro-cavity shadow
        ao = clamp(0.90 - rock_mask * 0.08 + grass_mask * 0.06 - scree_mask * 0.08 - forest * 0.28, 0.55, 1.0);
        float micro_cavity = clamp(mix(1.0, nor_rock_top.z, rock_mask * 0.5), 0.65, 1.0);
        ao *= micro_cavity;

        // Apply physical normal map perturbation to geometric terrain normal
        n = normalize(n + grad_terrain * (1.0 - water));

        if (water > 0.0) {
            // Two filtered broad wave directions; no screen-space reflection
            // buffer, iterative intersection, or per-pixel terrain lookup.
            vec2 phase = mod(world_xz, vec2(16384.0));
            float waveFade = 1.0 - smoothstep(8.0, 28.0, footprint);
            const float wavePeriod = 6.28318530718 / 16384.0;
            float wx = sin(dot(phase, vec2(495.0, 287.0) * wavePeriod) + ubo.flex.y * 0.7);
            float wz = sin(dot(phase, vec2(-209.0, 600.0) * wavePeriod) - ubo.flex.y * 0.6);
            n = normalize(mix(n, vec3(wx * 0.035 * waveFade, 1.0, wz * 0.025 * waveFade), water));
            float shore = smoothstep(125.0, TERRAIN_WATER, vLandHeight);
            albedo = mix(albedo, mix(vec3(0.008, 0.055, 0.070), vec3(0.035, 0.17, 0.15), shore), water);
            roughness = mix(roughness, 0.13, water);
            ao = mix(ao, 1.0, water);
        }
    } else if (vMaterial == 1u) {
        // Large stone courses and dark arrow-slit windows survive an aerial view.
        float course = abs(fract(vObjectPos.y / 4.0) - 0.5);
        float mortar = smoothstep(0.43, 0.49, course) * (1.0 - smoothstep(2.0, 8.0, footprint));
        vec2 face = vec2(abs(n.x) > abs(n.z) ? vObjectPos.z : vObjectPos.x, vObjectPos.y);
        vec3 diff_castle_rock = texture(sampler2D(detail_rock_diff_tex, detail_smp), face * (1.0 / 12.0)).rgb;
        float stone_lum = dot(diff_castle_rock, vec3(0.299, 0.587, 0.114));
        float stone_detail = clamp(stone_lum / 0.075, 0.6, 1.6);
        vec3 castle_stone = mix(vec3(0.28, 0.29, 0.30), vec3(0.42, 0.43, 0.44), grain * 0.5) * stone_detail;
        albedo = mix(castle_stone, vec3(0.18, 0.19, 0.20), mortar * 0.35);
        vec2 window = abs(fract(face / vec2(12.0, 20.0)) - 0.5);
        float slit = (1.0 - smoothstep(0.06, 0.10, window.x))
            * (1.0 - smoothstep(0.15, 0.19, window.y))
            * smoothstep(8.0, 12.0, vObjectPos.y) * (1.0 - abs(n.y))
            * (1.0 - smoothstep(2.0, 8.0, footprint));
        albedo = mix(albedo, vec3(0.012, 0.017, 0.015), slit);
        roughness = 0.82;
        ao = 0.85;
    } else {
        // Alpine village roof tiles: warm terracotta / cedar shingles
        albedo = mix(vec3(0.18, 0.075, 0.04), vec3(0.32, 0.14, 0.07), grain);
        roughness = 0.72;
        ao = 0.92;
    }
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

        #ifdef ENABLE_RT
        float visibility = 1.0;
        if (rtShadowGate(local_xz, hit.y) > 0.0) {
            // Rays start a hair above the surface so the ground's own plane
            // and near-field relief never self-occlude the sun disc.
            vec3 probe = hit + n * 0.05;
            float light_t = (hit.y - ubo.nodes[0][3].y) / (-ubo.sunDir.y);
            float t_max = clamp(light_t + 25.0, 30.0, 5000.0);
            visibility = rtSunVisibility(probe, t_max);
        }
#else
        float shadow = groundAircraftShadow(local_xz, hit.y);
        float visibility = 1.0 - shadow * 0.30;
#endif
        float cloud_visibility = cloudGroundSunVisibility(
            vec3(hit.x, altitude, hit.z), sun, ubo.groundOrigin,
            ubo.flex.y * ubo.cameraParams2.w);
        color += (direct_diffuse + direct_spec) * visibility * cloud_visibility;
    }

    // Multi-spectral atmospheric perspective: physical wavelength-dependent Rayleigh
    // and Mie extinction preserves authentic alpine depth layers across receding ridges.
    // The ground meets the sky with zero seam at the horizon.
    vec3 haze = ubo.skyHorizon.rgb;
    float cam_h = max(ubo.campos.y - ubo.groundBase.w, 0.0);
    float dR = exp(-cam_h / 8000.0);
    float dM = exp(-cam_h / 1200.0);
    float dO = atmoOzoneDensity(cam_h);
    vec3 ext = ATMO_BETA_RAYLEIGH * dR + ATMO_BETA_MIE_EXTINCT * dM
        + ATMO_BETA_OZONE * dO;
    vec3 transmittance = exp(-hit_t * ext);
    vec3 inscatter = haze * (vec3(1.0) - transmittance);
    vec3 final_color = color * transmittance + inscatter;
    outColor = vec4(final_color, 1.0);
}
