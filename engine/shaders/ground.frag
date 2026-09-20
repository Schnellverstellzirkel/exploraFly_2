#version 460

// Rasterized alpine landscape and medieval landmarks. Hardware depth handles
// mountain silhouettes and occlusion; no fragment terrain ray march is used.
//
// Vegetation cover follows baked landform moisture, altitude, and slope.
// Grass combines photographic blades, filtered tussock relief, and broad
// pasture color variation. Moist soil supports grass; it does not give the
// grass canopy a smooth water coating. See docs/rendering/alpine-grass.md.
// Inputs are world-stable, filtered by the pixel footprint, and evaluated
// as linear-light material inputs:
//   - landform moisture, altitude, and slope decide grass, soil, scree,
//     forest, snow, and damp ground;
//   - blade textures and metre-scale tussocks carry albedo and surface relief;
//   - Burley diffuse + GGX specular receive atmospheric sun and sky light;
//   - indirect light is occluded, while direct sun receives a soft aircraft
//     shadow and cloud shadows replayed from the same mesh cloud field the
//     cloud vertex shader draws (cloud.inc).

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
layout(location = 6) flat in vec3 vExtinction;
layout(location = 10) flat in vec3 vAtmoTrSun;
layout(location = 11) flat in vec3 vAtmoMulti;
layout(location = 12) flat in vec3 vAtmoDensities;
layout(location = 7) flat in uint vType;
layout(location = 8) flat in uint vPart;
layout(location = 9) flat in vec2 vShape;
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

vec3 groundValueNoiseGradient(vec2 local_m, float cell_m, uint seed) {
    // Reconstruct the absolute cell index from the split origin. Every
    // supported scale is an integer multiple of the 25 cm anchor cell.
    float cells_per_scale = cell_m * 4.0; // / GROUND_FINE_CELL (0.25)
    vec2 coarse_cell = floor(ubo.groundOrigin.xy / cells_per_scale);
    vec2 coarse_remainder = mod(ubo.groundOrigin.xy, vec2(cells_per_scale))
        * GROUND_FINE_CELL + ubo.groundOrigin.zw;
    float inv_cell_m = 1.0 / cell_m;
    vec2 q = (local_m + coarse_remainder) * inv_cell_m;
    vec2 cell = coarse_cell + floor(q);
    vec2 f = fract(q);
    vec2 w = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);

    // Vectorized 4-corner integer hash: evaluates 4 lattice corners in SIMD uvec4
    ivec2 icell = ivec2(cell);
    uvec2 u00 = uvec2(icell) & 0x000fffffu;
    uvec2 u11 = (u00 + uvec2(1u)) & 0x000fffffu;
    uint seed_term = seed * 2246822519u;
    uint x0 = u00.x * 1664525u + seed_term;
    uint x1 = u11.x * 1664525u + seed_term;
    uint y0 = u00.y * 1013904223u;
    uint y1 = u11.y * 1013904223u;
    uvec4 h = uvec4(x0 + y0, x1 + y0, x0 + y1, x1 + y1);
    h ^= h >> 16u;
    h *= 2246822519u;
    h ^= h >> 13u;
    h *= 3266489917u;
    h ^= h >> 16u;
    vec4 n = vec4(h & 0x00ffffffu) * (1.0 / 16777215.0);
    float nx0 = mix(n.x, n.y, w.x);
    float nx1 = mix(n.z, n.w, w.x);
    // Analytic derivatives of the same quintic interpolation. Relief and
    // color share a feature without extra finite-difference noise samples.
    vec2 dw = 30.0 * f * f * (f * (f - 2.0) + 1.0);
    return vec3(mix(nx0, nx1, w.y),
        mix(n.y - n.x, n.w - n.z, w.y) * dw.x / cell_m,
        (nx1 - nx0) * dw.y / cell_m);
}

float groundValueNoise(vec2 local_m, float cell_m, uint seed) {
    // Reconstruct the absolute cell index from the split origin. Every
    // supported scale is an integer multiple of the 25 cm anchor cell.
    float cells_per_scale = cell_m * 4.0; // / GROUND_FINE_CELL (0.25)
    vec2 coarse_cell = floor(ubo.groundOrigin.xy / cells_per_scale);
    vec2 coarse_remainder = mod(ubo.groundOrigin.xy, vec2(cells_per_scale))
        * GROUND_FINE_CELL + ubo.groundOrigin.zw;
    float inv_cell_m = 1.0 / cell_m;
    vec2 q = (local_m + coarse_remainder) * inv_cell_m;
    vec2 cell = coarse_cell + floor(q);
    vec2 f = fract(q);
    vec2 w = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);

    // Vectorized 4-corner integer hash: evaluates 4 lattice corners in SIMD uvec4
    ivec2 icell = ivec2(cell);
    uvec2 u00 = uvec2(icell) & 0x000fffffu;
    uvec2 u11 = (u00 + uvec2(1u)) & 0x000fffffu;
    uint seed_term = seed * 2246822519u;
    uint x0 = u00.x * 1664525u + seed_term;
    uint x1 = u11.x * 1664525u + seed_term;
    uint y0 = u00.y * 1013904223u;
    uint y1 = u11.y * 1013904223u;
    uvec4 h = uvec4(x0 + y0, x1 + y0, x0 + y1, x1 + y1);
    h ^= h >> 16u;
    h *= 2246822519u;
    h ^= h >> 13u;
    h *= 3266489917u;
    h ^= h >> 16u;
    vec4 n = vec4(h & 0x00ffffffu) * (1.0 / 16777215.0);
    float nx0 = mix(n.x, n.y, w.x);
    float nx1 = mix(n.z, n.w, w.x);
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

vec3 groundFilteredNoiseGradient(vec2 local_m, float cell_m, float footprint, uint seed) {
    if (footprint >= cell_m * 1.8) return vec3(0.5, 0.0, 0.0);
    float coverage = 1.0 - smoothstep(cell_m * 0.45, cell_m * 1.8, footprint);
    if (coverage <= 0.0) return vec3(0.5, 0.0, 0.0);
    vec3 sample_value = groundValueNoiseGradient(local_m, cell_m, seed);
    return vec3(mix(0.5, sample_value.x, coverage), sample_value.yz * coverage);
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

vec3 groundEnvironmentBRDF(vec3 f0, float roughness, float no_v) {
    // Karis 2014's analytic GGX environment BRDF fit. A bare Schlick
    // interface tends to white at grazing even on a rough meadow; the
    // integrated response accounts for masking across the rough lobe.
    vec4 fit = roughness * vec4(-1.0, -0.0275, -0.572, 0.022)
        + vec4(1.0, 0.0425, 1.04, -0.04);
    float grazing = min(fit.x * fit.x, exp2(-9.28 * no_v)) * fit.x + fit.y;
    vec2 scale_bias = vec2(-1.04, 1.04) * grazing + fit.zw;
    return clamp(f0 * scale_bias.x + scale_bias.y, 0.0, 1.0);
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

bool rtOccluded(vec3 origin, vec3 dir, float t_min, float t_max) {
    rayQueryEXT q;
    rayQueryInitializeEXT(q, scene_tlas, gl_RayFlagsOpaqueEXT | gl_RayFlagsTerminateOnFirstHitEXT,
        0xFF, origin, t_min, dir, t_max);
    rayQueryProceedEXT(q);
    return rayQueryGetIntersectionTypeEXT(q, true) != gl_RayQueryCommittedIntersectionNoneEXT;
}

float rtSunVisibility(vec3 origin, float t_min, float t_max) {
    vec3 sun = normalize(ubo.sunDir.xyz);
    float sun_radius = ubo.sunDir.w;
    
    // Sample 0: Center ray towards solar core
    if (!rtOccluded(origin, sun, t_min, t_max)) {
        vec3 up = (abs(sun.y) < 0.99) ? vec3(0.0, 1.0, 0.0) : vec3(1.0, 0.0, 0.0);
        vec3 s_t = normalize(cross(up, sun));
        vec3 s_b = cross(sun, s_t);
        float rot = fract(sin(dot(gl_FragCoord.xy, vec2(12.9898, 78.233))) * 43758.5453) * 6.2831853;
        float c_r = cos(rot);
        float s_r = sin(rot);
        vec2 p1 = SUN_POINTS[1];
        vec2 pr1 = vec2(p1.x * c_r - p1.y * s_r, p1.x * s_r + p1.y * c_r);
        vec3 dir1 = normalize(sun + (s_t * pr1.x + s_b * pr1.y) * sun_radius);
        if (!rtOccluded(origin, dir1, t_min, t_max)) {
            return 1.0; // Fully lit penumbra-free early exit
        }
        float lit = 1.0;
        for (uint i = 2u; i < SHADOW_RAYS; i += 1u) {
            vec2 p = SUN_POINTS[i];
            vec2 pr = vec2(p.x * c_r - p.y * s_r, p.x * s_r + p.y * c_r);
            vec3 dir = normalize(sun + (s_t * pr.x + s_b * pr.y) * sun_radius);
            if (!rtOccluded(origin, dir, t_min, t_max)) {
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
        if (rtOccluded(origin, dir1, t_min, t_max)) {
            return 0.0; // Deep umbra early exit
        }
        float lit = 0.0;
        for (uint i = 2u; i < SHADOW_RAYS; i += 1u) {
            vec2 p = SUN_POINTS[i];
            vec2 pr = vec2(p.x * c_r - p.y * s_r, p.x * s_r + p.y * c_r);
            vec3 dir = normalize(sun + (s_t * pr.x + s_b * pr.y) * sun_radius);
            if (!rtOccluded(origin, dir, t_min, t_max)) {
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
    vec2 ground_dx = dFdx(local_xz);
    vec2 ground_dy = dFdy(local_xz);
    float footprint = max(length(ground_dx), length(ground_dy));
    footprint = clamp(footprint, 0.0, 100000.0);

    vec3 n = normalize(cross(dFdx(hit), dFdy(hit)));
    if (vMaterial == 0u) n = normalize(vTerrainNormal);
    if (vMaterial != 0u && dot(n, -view_dir) < 0.0) n = -n;
    float slope = 1.0 - clamp(n.y, 0.0, 1.0);
    float altitude = hit.y - ubo.groundBase.w;
    float water = vMaterial == 0u ? 1.0 - smoothstep(TERRAIN_WATER - 0.5,
        TERRAIN_WATER + 1.5, vLandHeight) : 0.0;

    // Landform controls the primary cover. Filtered detail varies texture
    // within that cover without changing the valley/forest/scree geography.
    float moist = vMoisture;
    float grain = (footprint > 2.7) ? 0.5 : groundFilteredNoise(local_xz, 1.5, footprint, 79u);

    // Distance fades decide which detail work is exactly zero-weighted. Every
    // texture fetch and extra noise octave below sits behind one of these, so
    // mid/far terrain skips the whole photographic detail stack with
    // bit-identical output: each skipped term was mixed in with weight zero.
    float micro_fade = 1.0 - smoothstep(1.0, 45.0, footprint);
    float rock_fade  = 1.0 - smoothstep(12.0, 120.0, footprint);

    float soil_mask = clamp((1.0 - moist) * 0.8
        + smoothstep(0.30, 0.60, slope) * 0.5, 0.0, 1.0);
    float grass_mask = 1.0 - soil_mask;
    float scree_mask = smoothstep(0.42, 0.68, slope);
    float wetness = smoothstep(0.72, 0.92, moist);

    vec3 albedo = vec3(0.15, 0.20, 0.10);
    float roughness = 0.80;
    float ao = 0.90;
    float vegetation = 0.0;

    if (vMaterial == 0u) {
        vec3 world_pos = vec3(world_xz.x, altitude, world_xz.y);

        // 6. Solar aspect and snow cover (hoisted above the detail stack: these
        // masks gate the fetches below, and each gate below is an exact zero
        // weight in the original mixes, so skipped work is bit-identical).
        float sun_aspect = dot(n.xz, normalize(ubo.sunDir.xz));

        float snowLine = 1860.0 + (0.5 - moist) * 220.0;
        float snow = smoothstep(snowLine, snowLine + 240.0, altitude)
            * (1.0 - smoothstep(0.34, 0.66, slope));

        // 7. Avalanche Couloirs & Talus Fans
        // Steep mountain gullies funnel scree downward into natural talus fans
        float couloir_flow = sin(world_xz.x * 0.012 + world_xz.y * 0.006) * 0.5 + 0.5;
        float couloir = smoothstep(0.64, 0.86, couloir_flow)
            * smoothstep(0.38, 0.65, slope)
            * smoothstep(700.0, 1800.0, altitude)
            * (1.0 - snow);
        scree_mask = clamp(scree_mask + couloir * 0.55, 0.0, 1.0);

        // 8. Limestone Outcrops on Break-of-Slope Shelves
        // Natural bedrock ridges cropping out where slopes transition, with sun-lichen
        float shelf_zone = smoothstep(0.26, 0.48, slope)
            * (1.0 - smoothstep(1250.0, 1850.0, altitude))
            * (1.0 - water);

        // 9. Cattle Terracettes & Contour Paths (Kuhgänge)
        // Subalpine pasture hillsides feature subtle horizontal contour steps from grazing
        float contour_phase = fract(altitude / 16.0) - 0.5;
        float contour_path = (1.0 - smoothstep(0.02, 0.08, abs(contour_phase)))
            * smoothstep(0.20, 0.42, slope)
            * (1.0 - smoothstep(1300.0, 1750.0, altitude))
            * (1.0 - water);

        // 10. Shoreline Silt and Damp Hollows
        float mud_mask = smoothstep(0.80, 0.96, moist)
            * (1.0 - smoothstep(0.08, 0.24, slope))
            * (1.0 - smoothstep(TERRAIN_WATER + 1.5, TERRAIN_WATER + 9.0, vLandHeight));

        // Winding valley road (analytic; decides whether the road's scree
        // sample is consumed at any camera distance)
        float valleyX = mod(world_xz.x - terrainValleyCenter(mod(world_xz.y, TERRAIN_PERIOD))
            + 8192.0, 16384.0) - 8192.0;
        float roadDistance = abs(valleyX - 1050.0);
        float road = 1.0 - smoothstep(9.0, 13.0 + footprint, roadDistance);
        road *= (1.0 - smoothstep(0.2, 0.4, slope)) * (1.0 - water);

        float rock_mask = clamp(smoothstep(0.50, 0.85, slope)
            + smoothstep(1300.0, 2100.0, altitude) * 0.30, 0.0, 1.0);
        float cliff_weight = smoothstep(0.38, 0.72, slope);
        float strata_fade = 1.0 - smoothstep(4.0, 32.0, footprint);

        // Detail-consumption gates. micro_fade and rock_fade were hoisted to
        // the top of main(); every fetch below feeds only mixes weighted by
        // one of these masks, so unfetched defaults never reach the output.
        bool need_meadow = micro_fade > 0.0;
        bool need_scree  = micro_fade > 0.0 || road > 0.0;
        bool need_snow   = snow > 0.0;
        bool need_rock   = rock_mask > 0.0;

        vec2 warped_xz = world_xz;
        vec3 warped_pos = world_pos;
        bool need_detail = need_meadow || need_scree || need_snow || need_rock;
        if (need_detail) {
            // 1. Broad Continuous Domain Warping
            // AAA landscapes use low-frequency domain warping (wavelength 160m-300m) to break
            // rectilinear grid alignment without introducing high-frequency jitter.
            vec2 uv_warp = vec2(
                groundFilteredNoise(local_xz, 180.0, footprint, 311u),
                groundFilteredNoise(local_xz + vec2(113.7, 79.1), 180.0, footprint, 419u)
            ) * 2.0 - 1.0;
            warped_xz = world_xz + uv_warp * 14.0;
            warped_pos = vec3(warped_xz.x, altitude, warped_xz.y);
        }

        // 2. Grass blades at 2.5 m and a rotated 6.75 m sward layer. The
        // normal's XY components follow the inverse UV rotation as well.
        // Explicit pre-branch gradients keep mip choice valid across cover
        // gates; the slow domain warp deliberately does not sharpen the LOD.
        const mat2 ROT_41 = mat2(0.7490, 0.6626, -0.6626, 0.7490);
        vec2 uv_meadow1 = warped_xz * (1.0 / 2.5);
        vec2 uv_meadow2 = (ROT_41 * (warped_xz + vec2(137.4, -91.2))) * (1.0 / 6.75);
        vec3 diff_meadow = vec3(0.1232);
        vec2 meadow_slope = vec2(0.0);
        vec3 meadow_tuft = vec3(0.5, 0.0, 0.0);
        vec3 meadow_sward = vec3(0.5, 0.0, 0.0);
        if (need_meadow) {
            float meadow_scale_blend = groundFilteredNoise(local_xz, 90.0, footprint, 439u);
            float layer_blend = mix(0.22, 0.48, smoothstep(0.28, 0.72, meadow_scale_blend));
            vec2 dx1 = ground_dx * (1.0 / 2.5);
            vec2 dy1 = ground_dy * (1.0 / 2.5);
            vec2 dx2 = ROT_41 * ground_dx * (1.0 / 6.75);
            vec2 dy2 = ROT_41 * ground_dy * (1.0 / 6.75);
            vec3 diff_meadow1 = textureGrad(sampler2D(detail_meadow_diff_tex, detail_smp), uv_meadow1, dx1, dy1).rgb;
            vec3 diff_meadow2 = textureGrad(sampler2D(detail_meadow_diff_tex, detail_smp), uv_meadow2, dx2, dy2).rgb;
            diff_meadow = mix(diff_meadow1, diff_meadow2, layer_blend);

            vec3 normal1 = textureGrad(sampler2D(detail_meadow_nor_tex, detail_smp), uv_meadow1, dx1, dy1).rgb * 2.0 - 1.0;
            vec3 normal2 = textureGrad(sampler2D(detail_meadow_nor_tex, detail_smp), uv_meadow2, dx2, dy2).rgb * 2.0 - 1.0;
            meadow_slope = mix(normal1.xy / max(normal1.z, 0.35),
                transpose(ROT_41) * normal2.xy / max(normal2.z, 0.35), layer_blend);
            meadow_tuft = groundFilteredNoiseGradient(local_xz, 3.0, footprint, 461u);
            meadow_sward = groundFilteredNoiseGradient(local_xz + vec2(11.3, 7.7), 9.0, footprint, 547u);
        }

        // 3. Scree, Snow, and Rock Projections
        // Scree scale: 14.0m with domain warp
        vec2 uv_scree = warped_xz * (1.0 / 14.0);
        vec2 uv_snow  = warped_xz * (1.0 / 24.0);

        // Biplanar cliff projection avoids vertical texture stretching on steep cliffs
        vec2 uv_rock_top = warped_xz * (1.0 / 18.0);
        float abs_nx = abs(n.x);
        float abs_nz = abs(n.z);
        float cliff_side_blend = abs_nx / (abs_nx + abs_nz + 1e-4);
        vec2 uv_rock_side_x = vec2(warped_pos.z, warped_pos.y) * (1.0 / 18.0);
        vec2 uv_rock_side_z = vec2(warped_pos.x, warped_pos.y) * (1.0 / 18.0);
        vec2 uv_rock_side = mix(uv_rock_side_z, uv_rock_side_x, smoothstep(0.35, 0.65, cliff_side_blend));

        // Sample diffuse maps
        vec3 diff_scree = vec3(0.24);
        vec3 diff_snow  = vec3(0.5);
        vec3 diff_rock  = vec3(0.075);
        if (need_scree) {
            diff_scree = texture(sampler2D(detail_scree_diff_tex, detail_smp), uv_scree).rgb;
        }
        if (need_snow) {
            diff_snow = texture(sampler2D(detail_snow_diff_tex, detail_smp), uv_snow).rgb;
        }
        if (need_rock) {
            diff_rock = mix(
                texture(sampler2D(detail_rock_diff_tex, detail_smp), uv_rock_top).rgb,
                texture(sampler2D(detail_rock_diff_tex, detail_smp), uv_rock_side).rgb,
                cliff_weight
            );
        }

        // Sample normal maps: consumed only through gradient fades or the rock
        // micro-cavity term, never directly in albedo.
        vec3 nor_scree_raw = vec3(0.5, 0.5, 1.0);
        vec3 nor_snow_raw  = vec3(0.5, 0.5, 1.0);
        vec3 nor_rock_top  = vec3(0.5, 0.5, 1.0);
        vec3 nor_rock_side = vec3(0.5, 0.5, 1.0);
        if (micro_fade > 0.0) {
            nor_scree_raw = texture(sampler2D(detail_scree_nor_tex, detail_smp), uv_scree).rgb * 2.0 - 1.0;
        }
        if (snow > 0.0 && micro_fade > 0.0) {
            nor_snow_raw = texture(sampler2D(detail_snow_nor_tex, detail_smp), uv_snow).rgb * 2.0 - 1.0;
        }
        if (need_rock) {
            nor_rock_top = texture(sampler2D(detail_rock_nor_tex, detail_smp), uv_rock_top).rgb * 2.0 - 1.0;
        }
        if (rock_fade > 0.0) {
            nor_rock_side = texture(sampler2D(detail_rock_nor_tex, detail_smp), uv_rock_side).rgb * 2.0 - 1.0;
        }

        // 4. Orthonormal tangent basis for top-down and cliff projections.
        // Skipped entirely when every gradient consumer is exactly zero-weighted.
        vec3 T_h = vec3(1.0, 0.0, 0.0);
        vec3 B_h = vec3(0.0, 0.0, 1.0);
        vec3 T_v = vec3(0.0, 1.0, 0.0);
        vec3 B_v = vec3(1.0, 0.0, 0.0);

        // Surface gradient perturbations. The chain order of grad_terrain
        // mixes matches the original exactly; terms whose fade weight is
        // exactly zero evaluate nothing.
        vec3 grad_terrain = vec3(0.0);
        if (micro_fade > 0.0) {
            vec3 t_h = vec3(1.0 - n.x * n.x, -n.x * n.y, -n.x * n.z);
            float len_th = length(t_h);
            T_h = len_th > 1e-4 ? t_h / len_th : vec3(1.0, 0.0, 0.0);
            B_h = cross(n, T_h);

            // Project the world X/Z gradients onto the actual slope. +V is
            // +Z, whereas cross(n, +X) points toward -Z on level terrain.
            vec3 meadow_u = vec3(1.0, 0.0, 0.0) - n * n.x;
            vec3 meadow_v = vec3(0.0, 0.0, 1.0) - n * n.z;
            vec2 tuft_gradient = meadow_tuft.yz * 0.14 + meadow_sward.yz * 0.28;
            vec2 grass_gradient = meadow_slope * (0.65 * micro_fade) - tuft_gradient;
            vec3 grad_meadow = meadow_u * grass_gradient.x + meadow_v * grass_gradient.y;
            vec3 grad_scree  = (T_h * nor_scree_raw.x  + B_h * nor_scree_raw.y)  * (0.55 * micro_fade);
            grad_terrain = mix(grad_meadow, grad_scree, scree_mask);
        }

        // Rock gradients: consumed by the outcrop mix, the rock_mask mix, and
        // micro-cavity only needs the already-fetched nor_rock_top.
        float outcrop_n = 0.5;
        if (shelf_zone > 0.0) {
            outcrop_n = groundFilteredNoise(local_xz, 72.0, footprint, 853u);
        }
        float outcrop_mask = smoothstep(0.70, 0.90, outcrop_n) * shelf_zone;
        bool need_grad_rock = rock_fade > 0.0 || outcrop_mask > 0.0
            || (rock_mask > 0.0 && cliff_weight > 0.0 && strata_fade > 0.0);
        vec3 grad_rock = vec3(0.0);
        vec3 grad_crag = vec3(0.0);
        if (need_grad_rock) {
            vec3 t_h = vec3(1.0 - n.x * n.x, -n.x * n.y, -n.x * n.z);
            float len_th = length(t_h);
            T_h = len_th > 1e-4 ? t_h / len_th : vec3(1.0, 0.0, 0.0);
            B_h = cross(n, T_h);

            vec3 t_v = vec3(-n.y * n.x, 1.0 - n.y * n.y, -n.y * n.z);
            float len_tv = length(t_v);
            T_v = len_tv > 1e-4 ? t_v / len_tv : vec3(0.0, 1.0, 0.0);
            B_v = cross(n, T_v);

            vec3 grad_rock_top  = (T_h * nor_rock_top.x  + B_h * nor_rock_top.y)  * (0.85 * rock_fade);
            vec3 grad_rock_side = (B_v * nor_rock_side.x + T_v * nor_rock_side.y) * (1.10 * rock_fade);
            grad_rock = mix(grad_rock_top, grad_rock_side, cliff_weight);
            if (cliff_weight > 0.0 && strata_fade > 0.0) {
                // Geological horizontal strata banding on rock cliffs
                float strata_grad = (cos(altitude * 0.052) * 0.052 * 0.12
                    + cos(altitude * 0.125 + world_xz.x * 0.007) * 0.125 * 0.07) * 12.0;
                grad_rock += T_v * (strata_grad * cliff_weight * strata_fade);
            }
            if (rock_fade > 0.0) {
                // Meso-scale angular crags on high rock faces
                float crag_val = groundFilteredNoise(local_xz, 24.0, footprint, 211u) * 2.0 - 1.0;
                float crag_fine = groundFilteredNoise(local_xz, 8.0, footprint, 337u) * 2.0 - 1.0;
                grad_crag = (T_h * crag_val + B_h * crag_fine) * (0.35 * rock_fade);
            }
        }
        vec3 outcrop_albedo = mix(vec3(0.24, 0.25, 0.26), vec3(0.36, 0.37, 0.38), grain);
        float sun_lichen = smoothstep(0.10, 0.50, sun_aspect) * smoothstep(0.35, 0.70, grain);
        outcrop_albedo = mix(outcrop_albedo, vec3(0.42, 0.27, 0.08), sun_lichen * 0.60);

        // 10. Shoreline Silt and Damp Hollows
        vec3 mud_color = mix(vec3(0.065, 0.052, 0.038), vec3(0.095, 0.078, 0.058), grain);

        // 11. Bright alpine pasture palette (linear reflectance).
        // Continuous, geomorphically-driven color grading across elevation, moisture, and aspect.
        vec3 grass_valley = vec3(0.155, 0.340, 0.038); // Fresh valley pasture
        vec3 grass_golden = vec3(0.290, 0.390, 0.065); // Sunlit yellow-green tips
        vec3 grass_mossy  = vec3(0.075, 0.225, 0.035); // Sheltered green hollows
        vec3 grass_tundra = vec3(0.270, 0.295, 0.105); // Highland matgrass

        // Solar aspect: warm golden grass facing the sun vs rich cool moss in shadows
        vec3 pasture_base = mix(grass_mossy, grass_golden, smoothstep(-0.35, 0.45, sun_aspect));

        // Valley basin lushness: deep emerald green in low-altitude, moist flats
        float valley_basin = smoothstep(0.40, 0.75, moist)
            * (1.0 - smoothstep(350.0, 950.0, altitude))
            * (1.0 - smoothstep(0.10, 0.32, slope));
        pasture_base = mix(pasture_base, grass_valley, valley_basin * 0.85);

        // High alpine transition: stunted tawny matgrass climbing to the summits
        float tundra_trans = smoothstep(1150.0, 1750.0, altitude);
        pasture_base = mix(pasture_base, grass_tundra, tundra_trans);

        // Broad geographic field modulation (250m - 500m scale natural field swaths)
        float field_swath1 = groundFilteredNoise(local_xz, 480.0, footprint, 241u);
        float field_swath2 = groundFilteredNoise(local_xz + vec2(170.0, 290.0), 240.0, footprint, 389u);
        float macro_field = (field_swath1 - 0.5) * 0.22 + (field_swath2 - 0.5) * 0.15;
        pasture_base *= (1.0 + macro_field);

        // 12. Organic Wildflower Colonies in Sunlit Pastures
        // Natural irregular drifts of warm buttercups and mountain clover in gentle pastures.
        // The noise stack only runs where every other drift factor is nonzero.
        float flower_drift = 0.0;
        if (moist > 0.46 && slope < 0.24 && altitude < 1250.0) {
            float flower_noise = groundFilteredNoise(local_xz, 160.0, footprint, 617u)
                + groundFilteredNoise(local_xz + vec2(52.0, -85.0), 70.0, footprint, 719u) * 0.45;
            flower_drift = smoothstep(0.85, 1.20, flower_noise)
                * smoothstep(0.46, 0.70, moist)
                * (1.0 - smoothstep(0.08, 0.24, slope))
                * (1.0 - smoothstep(500.0, 1250.0, altitude));
        }
        vec3 flower_albedo = vec3(0.29, 0.32, 0.075); // Rich golden-amber buttercup bloom
        pasture_base = mix(pasture_base, flower_albedo, flower_drift * 0.55);

        // Rare edelweiss on high sunny limestone ridges (same exact-zero gating;
        // the altitude smoothstep saturates at 1.0 above 1680 m, so no upper cut)
        float edelweiss_drift = 0.0;
        if (altitude > 1250.0 && slope > 0.20 && sun_aspect > 0.20) {
            edelweiss_drift = smoothstep(1250.0, 1680.0, altitude)
                * smoothstep(0.20, 0.50, slope)
                * smoothstep(0.20, 0.60, sun_aspect)
                * smoothstep(0.76, 0.92, groundFilteredNoise(local_xz, 80.0, footprint, 743u));
        }
        pasture_base = mix(pasture_base, vec3(0.44, 0.47, 0.40), edelweiss_drift * 0.50);

        // 13. Retain photographed blade/thatch contrast without importing the
        // scan's brown cast into every meadow. 0.1232 is its measured linear
        // luminance mean; mip levels and faded detail converge to the palette.
        float meadow_lum = dot(diff_meadow, vec3(0.2126, 0.7152, 0.0722));
        float meadow_photo = clamp(meadow_lum / 0.1232, 0.48, 1.65);
        float tussock_color = (meadow_tuft.x - 0.5) * 0.30
            + (meadow_sward.x - 0.5) * 0.42;
        vec3 meadow_albedo = pasture_base * (1.0 + tussock_color)
            * mix(1.0, meadow_photo, micro_fade * 0.90);

        // Soil, contour trails, mud, and outcrops
        float scree_lum = dot(diff_scree, vec3(0.299, 0.587, 0.114));
        float scree_detail = mix(1.0, clamp(scree_lum / 0.24, 0.78, 1.28), micro_fade);
        vec3 soil_albedo = mix(vec3(0.15, 0.13, 0.10), vec3(0.23, 0.19, 0.15), grain) * scree_detail;

        meadow_albedo = mix(meadow_albedo, soil_albedo, soil_mask * 0.30);
        meadow_albedo = mix(meadow_albedo, soil_albedo * 1.12, contour_path * 0.60);
        meadow_albedo = mix(meadow_albedo, mud_color, mud_mask * 0.80);
        meadow_albedo = mix(meadow_albedo, outcrop_albedo, outcrop_mask * 0.85);

        // Scree slopes: gravel and talus fans beneath cliff faces
        vec3 scree_albedo = mix(vec3(0.25, 0.24, 0.22), vec3(0.33, 0.31, 0.28), grain) * scree_detail;
        albedo = mix(meadow_albedo, scree_albedo, scree_mask);
        grad_terrain = mix(grad_terrain, grad_rock, outcrop_mask * 0.75);

        // High rock cliffs
        float rock_lum = dot(diff_rock, vec3(0.299, 0.587, 0.114));
        float rock_factor = clamp(rock_lum / 0.075, 0.4, 2.0);
        vec3 rock_tint = mix(vec3(0.16, 0.17, 0.18), vec3(0.34, 0.35, 0.36), clamp(rock_factor * 0.5, 0.0, 1.0));
        // Geological horizontal strata banding (only consumed through rock_mask)
        float rock_strata = 0.0;
        if (need_rock) {
            rock_strata = sin(altitude * 0.052) * 0.12 + sin(altitude * 0.125 + world_xz.x * 0.007) * 0.07;
        }
        vec3 rock_albedo = mix(rock_tint, diff_rock * vec3(1.7, 2.3, 2.7), 0.35) * (1.0 + rock_strata);
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
        float pebble = (forest > 0.0 && micro_fade > 0.0 && footprint <= 0.9)
            ? groundFilteredNoise(local_xz, 0.5, footprint, 107u) : 0.5;
        vec3 grad_forest = (T_h * (grain - 0.5) + B_h * (pebble - 0.5)) * (0.75 * micro_fade);
        grad_terrain = mix(grad_terrain, grad_forest, forest * 0.80);

        // Snow cover on alpine peaks
        vec3 snow_albedo = clamp(diff_snow * 2.15, 0.0, 0.94) * vec3(0.96, 0.98, 1.0);
        albedo = mix(albedo, snow_albedo, snow);
        vec3 grad_snow = (T_h * nor_snow_raw.x + B_h * nor_snow_raw.y) * (0.35 * micro_fade);
        grad_terrain = mix(grad_terrain, grad_snow, snow);

        // Winding valley road
        albedo = mix(albedo, diff_scree * vec3(1.15, 0.98, 0.78), road * 0.85);

        // Soil moisture feeds vegetation, not a continuous glossy film.
        // Only exposed soil/stone and the narrow muddy shore get wet shading.
        vegetation = mix((1.0 - soil_mask * 0.30) * (1.0 - mud_mask)
            * (1.0 - scree_mask) * (1.0 - rock_mask) * (1.0 - outcrop_mask),
            1.0, forest * 0.88) * (1.0 - snow) * (1.0 - road) * (1.0 - water);
        float exposed_wetness = wetness * (1.0 - vegetation) * (1.0 - snow);
        albedo *= mix(vec3(1.0), vec3(0.70, 0.76, 0.73), exposed_wetness * 0.25);

        // Lake-perimeter swash zone: a narrow band of ground just above the
        // waterline reads as periodically submerged - darkened, cooled, and
        // smoothed with proximity, fading out under snow and at distance.
        float shore_wet = (1.0 - water) * (1.0 - snow)
            * (1.0 - smoothstep(0.0, 1.1 + footprint * 0.35, vLandHeight - TERRAIN_WATER));
        albedo *= mix(vec3(1.0), vec3(0.62, 0.66, 0.64), shore_wet * 0.55);

        roughness = mix(0.93, 0.99, meadow_tuft.x);
        roughness = mix(roughness, 0.84, rock_mask);
        roughness = mix(roughness, 0.88, scree_mask);
        roughness = mix(roughness, 0.92, forest * 0.88);
        roughness = mix(roughness, 0.52, snow);
        roughness = mix(roughness, 0.36, mud_mask * 0.85);
        roughness = mix(roughness, 0.42, exposed_wetness);
        roughness = mix(roughness, 0.30, shore_wet * 0.85);

        // Ambient occlusion and micro-cavity shadow
        ao = clamp(0.90 - rock_mask * 0.08 + grass_mask * 0.06 - scree_mask * 0.08 - forest * 0.28, 0.55, 1.0);
        float micro_cavity = clamp(mix(1.0, nor_rock_top.z, rock_mask * 0.5), 0.65, 1.0);
        ao *= micro_cavity;
        float grass_cavity = mix(0.86, 1.0, smoothstep(0.55, 1.15, meadow_photo));
        ao *= mix(1.0, grass_cavity, vegetation * micro_fade);

        // Apply physical normal map perturbation to geometric terrain normal
        n = normalize(n + grad_terrain * (1.0 - water));

        if (water > 0.0) {
            // Short-crested alpine lake wave synthesis (Tessendorf 2001, Finch 2004,
            // Bruneton et al. 2010, Dupuy & Bruneton 2012, Jeschke & Wojtan 2017).
            // Replaces 1D parallel swells with fetch-limited short-crested wave packets,
            // transversal crest modulation, dual-tier domain warping, and Beer-Lambert depth extinction.
            vec2 world_tile_xz = mod(world_xz, vec2(16384.0));
            float t = ubo.flex.y;

            // 1. Dual-Tier Organic Domain Warping (130m coarse + 42m fine)
            // Completely eliminates rectilinear wave front alignment without adding high-frequency noise.
            vec2 warp_coarse = vec2(
                groundFilteredNoise(local_xz, 130.0, footprint, 587u),
                groundFilteredNoise(local_xz + vec2(83.7, 121.2), 130.0, footprint, 613u)
            ) * 2.0 - 1.0;
            vec2 warp_fine = vec2(
                groundFilteredNoise(local_xz + vec2(31.4, -47.1), 42.0, footprint, 349u),
                groundFilteredNoise(local_xz + vec2(-53.2, 29.8), 42.0, footprint, 421u)
            ) * 2.0 - 1.0;
            vec2 wave_xz = world_tile_xz + warp_coarse * 12.0 + warp_fine * 4.5;

            // 2. Wind Gust & Mountain-Sheltered Calm Slicks (360m and 140m scales)
            // Natural alpine lakes feature glassy mountain-sheltered mirror slicks interspersed with wind lanes.
            float gust_n1 = groundFilteredNoise(local_xz, 360.0, footprint, 727u);
            float gust_n2 = groundFilteredNoise(local_xz + vec2(130.0, -90.0), 140.0, footprint, 853u);
            float wind_streak = gust_n1 * 0.65 + gust_n2 * 0.35;
            float gust_factor = smoothstep(0.36, 0.70, wind_streak);
            float slick_mask  = 1.0 - smoothstep(0.28, 0.48, wind_streak);
            float chop_damp   = mix(1.0, 0.10, slick_mask);

            // 3. Short-Crested Wave Octaves (Fetch-limited alpine lake spectrum)
            // Directional spreading (Mitsuyasu et al. 1975) and short-crested transversal modulation
            // break up continuous wavefronts into localized 3D chop mounds.
            const vec2 WAVE_DIRS[8] = vec2[8](
                vec2(0.8192, 0.5736),   // 35 deg (primary wind chop)
                vec2(0.9397, -0.3420),  // -20 deg (secondary wind chop)
                vec2(0.3420, 0.9397),   // 70 deg (cross-valley chop)
                vec2(-0.5736, 0.8192),  // 125 deg (shore reflection)
                vec2(0.6428, 0.7660),   // 50 deg (surface ripple)
                vec2(0.9848, 0.1736),   // 10 deg (capillary ripple)
                vec2(-0.7660, 0.6428),  // 140 deg (micro-chop)
                vec2(0.1736, -0.9848)   // -80 deg (micro-sparkle)
            );
            const float WAVE_LENS[8] = float[8](14.8, 10.2, 7.1, 4.8, 3.2, 2.1, 1.4, 0.95);
            const float WAVE_SPEEDS[8] = float[8](4.81, 4.00, 3.33, 2.74, 2.24, 1.81, 1.48, 1.22);
            // Cox & Munk 1954 calibrated light-breeze wave steepness
            const float WAVE_STEEPNESS[8] = float[8](0.024, 0.020, 0.017, 0.014, 0.011, 0.008, 0.006, 0.004);

            vec2 grad = vec2(0.0);
            float roughness_acc = 0.0;

            for (int i = 0; i < 8; ++i) {
                float wlen = WAVE_LENS[i];
                // Distance filtering: analytic pre-filtering (Bruneton et al. 2010, Zirr & Kaplanyan 2016)
                float fade = 1.0 - smoothstep(wlen * 0.35, wlen * 1.60, footprint);
                if (fade > 0.001) {
                    vec2 d = WAVE_DIRS[i];
                    vec2 d_perp = vec2(-d.y, d.x);
                    float k = 6.2831853 / wlen;
                    float phase = k * (dot(d, wave_xz) - WAVE_SPEEDS[i] * t);
                    float s = sin(phase);
                    float c = cos(phase);

                    // Transversal crest modulation (Jeschke & Wojtan 2017)
                    // Bounds crest length to ~2.8 wavelengths, forming discrete 3D wave mounds rather than stripes.
                    float u_perp = dot(d_perp, wave_xz);
                    float k_trans = k * 0.36;
                    float crest_env = 0.5 + 0.5 * cos(u_perp * k_trans + float(i) * 1.618034);
                    float crest_env_d = -0.5 * k_trans * sin(u_perp * k_trans + float(i) * 1.618034);

                    // Gerstner crest sharpening (Finch 2004)
                    float slope = c * (1.0 + 0.55 * s);
                    float octave_gust = (i >= 2) ? (gust_factor * chop_damp) : mix(1.0, 0.15, slick_mask);
                    float amp = WAVE_STEEPNESS[i] * fade * octave_gust;

                    // Combined longitudinal slope and transversal peak curvature
                    grad += d * (amp * slope * crest_env) + d_perp * (amp * s * crest_env_d);
                }
                // Pre-filter unresolved geometric slope variance into GGX roughness (Bruneton et al. 2010)
                roughness_acc += (1.0 - fade) * WAVE_STEEPNESS[i] * 0.55;
            }

            // 4. Physical Alpine Lake Depth Absorption & Glacial Palette (Beer-Lambert Law)
            float water_depth = max(TERRAIN_WATER - vLandHeight, 0.0);
            vec3 water_shallow = vec3(0.040, 0.225, 0.215); // Crystalline turquoise shallows
            vec3 water_mid     = vec3(0.012, 0.110, 0.140); // Luminous emerald teal shelf
            vec3 water_deep    = vec3(0.002, 0.022, 0.052); // Deep alpine sapphire abyss

            // Multi-spectral exponential extinction: red absorbed in 4m, green in 12m, blue penetrates
            float shallow_trans = 1.0 - exp(-water_depth * 0.24);
            float deep_trans    = 1.0 - exp(-water_depth * 0.048);
            vec3 water_body     = mix(water_shallow, water_mid, shallow_trans);
            water_body          = mix(water_body, water_deep, deep_trans);

            // Submerged bed visibility & animated shallow water caustics (Stam 1996)
            vec3 submerged_bed = albedo * vec3(0.55, 0.62, 0.58);
            if (water_depth < 4.0 && footprint < 6.0) {
                // Refraction parallax: surface refraction bends the view ray
                // ~25% toward vertical (1 - 1/1.33), so the caustic network
                // slides horizontally with the camera relative to the surface
                // point - a depth cue that sells the water volume.
                vec3 to_cam = normalize(ubo.campos.xyz - hit);
                vec2 caustic_xz = world_tile_xz
                    - (to_cam.xz / max(to_cam.y, 0.30)) * water_depth * 0.25;
                float c_phase1 = dot(caustic_xz, vec2(1.15, 0.82)) + t * 2.1;
                float c_phase2 = dot(caustic_xz, vec2(-0.74, 1.35)) - t * 1.7;
                float caustic = (sin(c_phase1) * 0.5 + 0.5) * (sin(c_phase2) * 0.5 + 0.5);
                caustic = caustic * caustic * caustic * 3.5;
                float caustic_fade = exp(-water_depth * 0.75) * (1.0 - smoothstep(0.5, 6.0, footprint));
                submerged_bed *= (1.0 + caustic * caustic_fade * 0.65);
            }
            float bed_visibility = exp(-water_depth * 0.70);
            vec3 water_albedo = mix(water_body, submerged_bed, bed_visibility * 0.75);

            // 5. Shoreline Animated Wave Lapping and Soft Foam Fringe
            float lap_phase = (world_xz.x * 0.20 + world_xz.y * 0.16) + t * 1.5;
            float shore_lap = sin(lap_phase) * 0.14 + sin(lap_phase * 1.67 + 1.3) * 0.07;
            float shore_dist = (TERRAIN_WATER - vLandHeight) + shore_lap;
            float shore_foam = smoothstep(-0.12, 0.04, shore_dist)
                * (1.0 - smoothstep(0.04, 0.48, shore_dist));

            // Wave crest foam (Dupuy & Bruneton 2012 steepness threshold)
            float crest_steepness = length(grad);
            float crest_foam = smoothstep(0.038, 0.068, crest_steepness) * gust_factor
                * (1.0 - smoothstep(1.5, 8.0, footprint));
            float total_foam = clamp(shore_foam * 0.75 + crest_foam * 0.35, 0.0, 1.0);
            vec3 foam_color = vec3(0.85, 0.92, 0.94);
            water_albedo = mix(water_albedo, foam_color, total_foam);

            // 6. Surface Roughness (Bruneton et al. 2010 normal variance integration)
            float base_roughness = mix(0.020, 0.082, gust_factor);
            float water_roughness = clamp(base_roughness + roughness_acc + total_foam * 0.20, 0.018, 0.16);

            // 7. Apply Physical Normal Perturbation from Wave Gradient
            vec3 n_water = normalize(vec3(-grad.x, 1.0, -grad.y));
            n = normalize(mix(n, n_water, water));
            albedo = mix(albedo, water_albedo, water);
            roughness = mix(roughness, water_roughness, water);
            ao = mix(ao, 1.0, water);
        }
    } else if (vMaterial == 1u) {
        // 3D ashlar stone masonry on fortifications and civic stone, half
        // timber plaster on village houses, plus the roofline band and the
        // jettied-storey soffit that the detail tiers added.
        bool isFort = (vType <= 8u || vType == 34u || vType == 39u || vType == 41u || vType == 42u || vType == 44u || vType == 45u || vType == 46u || vType == 48u || vType == 50u);
        bool isTimber = (vType >= 9u && vType <= 23u && vType != 22u)
            || vType == 26u || vType == 27u || vType == 38u || vType == 40u || vType == 43u || vType == 47u || vType == 49u || vType == 51u;
        float wall_u = (abs(n.x) > abs(n.z)) ? vObjectPos.z : vObjectPos.x;
        float wall_v = vObjectPos.y;

        // Tangent frame on vertical wall faces
        vec3 T_wall = vec3(abs(n.z) > 0.5 ? 1.0 : 0.0, 0.0, abs(n.x) > 0.5 ? 1.0 : 0.0)
            * (abs(n.x) > 0.5 ? -sign(n.x) : sign(n.z));
        vec3 B_wall = vec3(0.0, 1.0, 0.0);

        if (vPart == 7u) {
            // Roofline band: machicolation corbels and merlons on castles, a
            // dark timber eave-seat fascia on houses. Vertical slats repeat
            // down the wall run and catch the light at their proud edges.
            float por_u = fract(wall_u / 0.85);
            float por_hash = groundHash(vec2(floor(wall_u / 0.85), wall_v), 211u);
            float por_edge = smoothstep(0.34, 0.50, por_u) * (1.0 - smoothstep(0.50, 0.66, por_u));
            vec3 por_col = isFort
                ? mix(vec3(0.23, 0.225, 0.22), vec3(0.17, 0.165, 0.16), por_hash)
                : mix(vec3(0.17, 0.125, 0.08), vec3(0.26, 0.20, 0.13), por_hash);
            por_col = mix(por_col, por_col * 1.45, por_edge);
            if (isFort) {
                // Corbels step out between the slats in the lower half of the
                // band, so each bracket casts onto the stone beneath it.
                float corbel = smoothstep(0.16, 0.24, por_u) - smoothstep(0.72, 0.80, por_u);
                por_col = mix(por_col, por_col * 0.42, corbel * (1.0 - smoothstep(2.2, 2.85, fract(wall_v))));
            }
            albedo = por_col;
            roughness = 0.9;
            ao = 0.68;
        } else if (vPart == 6u && n.y < -0.5 && isTimber) {
            // Underside of the jettied upper storey: beetles and dirt live
            // under the overhang, so it reads darker than the lit walls.
            albedo = mix(vec3(0.12, 0.09, 0.06), vec3(0.05, 0.045, 0.04),
                groundHash(vec2(wall_u * 0.37, 0.0), 227u));
            roughness = 0.96;
            ao = 0.5;
        } else if (isTimber && vPart == 0u) {
            if (vType == 40u) {
                // Alpine Almhütte: horizontal pine log construction (Blockbau)
                float log_row = floor(wall_v / 0.38);
                float log_v = fract(wall_v / 0.38);
                float log_bevel = 1.0 - smoothstep(0.04, 0.12, min(log_v, 1.0 - log_v));
                float log_hash = groundHash(vec2(log_row, floor(wall_u / 1.5)), 151u);
                vec3 log_wood = mix(vec3(0.28, 0.17, 0.09), vec3(0.18, 0.11, 0.06), log_hash * 0.6);
                albedo = mix(log_wood, log_wood * 0.35, log_bevel * 0.8);
                roughness = 0.94;
                ao = 0.82;
            } else if (vType == 43u) {
                // Meadow Hay Barn (Heustadel): weathered timber with aeration slats
                float plank_v = fract(wall_v / 0.28);
                float plank_slit = smoothstep(0.88, 0.98, plank_v);
                float plank_h = groundHash(vec2(floor(wall_v / 0.28), floor(wall_u / 2.0)), 167u);
                vec3 barn_wood = mix(vec3(0.22, 0.18, 0.14), vec3(0.15, 0.12, 0.09), plank_h * 0.5);
                albedo = mix(barn_wood, vec3(0.02, 0.02, 0.02), plank_slit * 0.75);
                roughness = 0.96;
                ao = 0.80;
            } else if (vType == 47u) {
                // Lakeside Boat House: creosote-tarred dark wharf timber
                float plank_v = fract(wall_v / 0.25);
                float plank_h = groundHash(vec2(floor(wall_v / 0.25), floor(wall_u / 1.8)), 179u);
                vec3 dock_wood = mix(vec3(0.13, 0.10, 0.08), vec3(0.07, 0.06, 0.05), plank_h);
                // Wet algae shoreline band
                float algae = (1.0 - smoothstep(0.0, 2.5, wall_v)) * step(wall_v, 2.5);
                albedo = mix(dock_wood, vec3(0.08, 0.14, 0.06), algae * 0.6);
                roughness = mix(0.92, 0.45, algae);
                ao = 0.84;
            } else if (vType == 49u) {
                // Cliffside Hermitage: rough-hewn weathered cedar timber
                float plank_v = fract(wall_v / 0.32);
                float plank_h = groundHash(vec2(floor(wall_v / 0.32), floor(wall_u / 1.6)), 197u);
                vec3 cell_wood = mix(vec3(0.24, 0.20, 0.16), vec3(0.16, 0.13, 0.10), plank_h * 0.6);
                albedo = cell_wood;
                roughness = 0.94;
                ao = 0.82;
            } else if (vType == 51u) {
                // Alpine Sawmill: heavy pine beam flume & mill
                float beam_v = fract(wall_v / 0.45);
                float beam_bevel = 1.0 - smoothstep(0.04, 0.12, min(beam_v, 1.0 - beam_v));
                float beam_h = groundHash(vec2(floor(wall_v / 0.45), floor(wall_u / 2.2)), 223u);
                vec3 mill_wood = mix(vec3(0.30, 0.22, 0.14), vec3(0.20, 0.15, 0.10), beam_h * 0.5);
                albedo = mix(mill_wood, mill_wood * 0.4, beam_bevel * 0.7);
                roughness = 0.92;
                ao = 0.84;
            } else {
                // Half-timber frame: square oak posts and headers over plaster
                // infill panels, 3 m wide and 2 m tall, staggered per storey.
                float panel_row = floor(wall_v / 2.0);
                float post = abs(fract((wall_u + mod(panel_row, 2.0) * -1.5) / 3.0) - 0.5) * 2.0;
                float header = abs(fract(wall_v / 2.0) - 0.5) * 2.0;
                float frame = clamp(smoothstep(0.80, 0.95, post) + smoothstep(0.82, 0.95, header), 0.0, 1.0);
                // Spark the diagonal brace between every other post pair.
                float brace_u = mod(wall_u + mod(panel_row, 2.0) * -1.5, 6.0);
                float brace = smoothstep(0.85, 1.0, abs(abs(brace_u - 1.5 - fract(wall_v / 3.0) * 3.0) - 1.5) - 1.2)
                    * step(0.2, brace_u) * (1.0 - step(0.5, brace_u));
                frame = clamp(frame + brace, 0.0, 1.0);
                float panel_hash = groundHash(vec2(floor(wall_u / 3.0), panel_row), 173u);
                vec3 plaster = mix(vec3(0.70, 0.64, 0.53), vec3(0.82, 0.76, 0.62), panel_hash * 0.55);
                vec3 timber = mix(vec3(0.16, 0.115, 0.075), vec3(0.26, 0.19, 0.12), panel_hash * 0.45);
                albedo = mix(plaster, timber, frame);
                // Small leaded window at the storey middle of every other panel.
                float win_u = abs(fract((wall_u + 1.5) / 3.0) - 0.5);
                float win_v = abs(fract(wall_v / 2.0) - 0.5);
                float window = (1.0 - smoothstep(0.06, 0.12, win_u))
                    * (1.0 - smoothstep(0.02, 0.08, win_v))
                    * step(0.5, mod(floor(wall_u / 3.0), 2.0));
                albedo = mix(albedo, vec3(0.012, 0.014, 0.015), window * 0.85);
                roughness = 0.93;
                ao = mix(0.86, 0.76, frame * 0.6);
            }
        } else {
            // 3D Ashlar stone masonry for fortifications and civic stone.
            // Ashlar stone courses (1.2m course height, 2.4m block length, staggered)
            float course_idx = floor(wall_v / 1.2);
            float course_v = fract(wall_v / 1.2);
            float block_u_raw = (wall_u + mod(course_idx, 2.0) * 1.2) / 2.4;
            float block_idx = floor(block_u_raw);
            float block_u = fract(block_u_raw);

            // Rounded bevel on ashlar block edges down to recessed mortar joints
            vec2 edge_dist = min(vec2(block_u, course_v), vec2(1.0 - block_u, 1.0 - course_v));
            float bevel_u = smoothstep(0.02, 0.08, edge_dist.x);
            float bevel_v = smoothstep(0.03, 0.10, edge_dist.y);
            float mortar_depth = 1.0 - bevel_u * bevel_v;

            // Slope normal inwards along block perimeter bevels
            float d_bu = (edge_dist.x < 0.08) ? (block_u < 0.5 ? -1.0 : 1.0) * (1.0 - bevel_u) : 0.0;
            float d_bv = (edge_dist.y < 0.10) ? (course_v < 0.5 ? -1.0 : 1.0) * (1.0 - bevel_v) : 0.0;

            // Photo rock normal and diffuse texture integration
            vec2 face_uv = vec2(wall_u, wall_v) * (1.0 / 8.0);
            float wall_micro_fade = 1.0 - smoothstep(0.5, 6.0, footprint);
            vec3 nor_rock = texture(sampler2D(detail_rock_nor_tex, detail_smp), face_uv).rgb * 2.0 - 1.0;
            vec3 diff_rock = texture(sampler2D(detail_rock_diff_tex, detail_smp), face_uv).rgb;

            vec3 grad_masonry = (T_wall * (d_bu * 0.32 + nor_rock.x * 0.40 * wall_micro_fade)
                + B_wall * (d_bv * 0.32 + nor_rock.y * 0.40 * wall_micro_fade));
            n = normalize(n + grad_masonry);

            // Stone block albedo variation & mortar darkening
            float block_hash = groundHash(vec2(block_idx, course_idx), 73u);
            vec3 stone_tint = mix(vec3(0.30, 0.31, 0.32), vec3(0.44, 0.43, 0.41), block_hash * 0.55);
            float rock_lum = dot(diff_rock, vec3(0.299, 0.587, 0.114));
            stone_tint *= clamp(rock_lum / 0.075, 0.65, 1.45);

            vec3 mortar_col = vec3(0.16, 0.16, 0.17);
            albedo = mix(stone_tint, mortar_col, mortar_depth * 0.55);

            if (vType == 41u) {
                // Stone bridge: water stain and cutwater river spray
                float water_stain = 1.0 - smoothstep(1.0, 5.0, wall_v);
                albedo = mix(albedo, vec3(0.18, 0.20, 0.17), water_stain * 0.60);
            }

            // Arrow slits and lancet windows only on tall fortification walls
            vec2 window = abs(fract(vec2(wall_u, wall_v) / vec2(8.0, 14.0)) - 0.5);
            float slit = (1.0 - smoothstep(0.05, 0.09, window.x))
                * (1.0 - smoothstep(0.15, 0.19, window.y))
                * smoothstep(6.0, 10.0, wall_v) * (1.0 - abs(n.y))
                * (1.0 - smoothstep(1.5, 6.0, footprint));
            slit *= isFort ? 1.0 : 0.0;
            albedo = mix(albedo, vec3(0.012, 0.015, 0.014), slit);

            // Foundation contact AO and rising damp at base of walls
            float foundation_ao = smoothstep(0.0, 3.0, wall_v);
            ao = mix(0.55, 0.88, foundation_ao) * (1.0 - mortar_depth * 0.22);
            roughness = mix(0.78, 0.90, mortar_depth);
        }
    } else if (vMaterial == 2u) {
        if (vPart == 5u) {
            // Canvas windmill sail stretched on a radial spar: slat ribs
            // across the panel mottle it, and the spar bolthole stays dark.
            float ax = abs(vObjectPos.x);
            float ay = abs(vObjectPos.y);
            float along = max(ax, ay);
            float sail_hash = groundHash(vec2(floor(along * 0.18), 0.0), 263u);
            vec3 canvas = mix(vec3(0.60, 0.55, 0.45), vec3(0.44, 0.40, 0.32), sail_hash);
            float slat_s = abs(fract(along * 0.55) - 0.5) * 2.0;
            float sail_slat = 1.0 - smoothstep(0.82, 0.97, slat_s);
            albedo = mix(canvas, canvas * 0.3, sail_slat * 0.85);
            float bolt = smoothstep(0.0, 0.12, along) * (1.0 - smoothstep(0.12, 0.22, along));
            albedo = mix(albedo, vec3(0.05, 0.05, 0.05), bolt);
            roughness = 0.72;
            ao = 0.82;
        } else if (vPart == 4u) {
            // Weathered slate spire: gentle vertical rib bands over a dark
            // slate base, with chalky patina on the windward quarter.
            float spire_hash = groundHash(vec2(floor(vObjectPos.y * 0.4), 0.0), 163u);
            vec3 spire_slate = mix(vec3(0.17, 0.18, 0.20), vec3(0.27, 0.28, 0.30), spire_hash);
            float rib = smoothstep(0.04, 0.10, abs(fract(vObjectPos.y * 0.9 + 0.1) - 0.5) * 2.0);
            albedo = mix(spire_slate, spire_slate * 1.4, rib);
            float capping_hash = groundHash(vec2(floor(vObjectPos.y * 0.7), 0.0), 181u);
            albedo = mix(albedo, vec3(0.80, 0.78, 0.72), capping_hash * 0.03);
            roughness = 0.80;
            ao = 0.86;
        } else {
        // Alpine roofs: layered terracotta, weathered slate, and cedar timber shingles
        float roof_u = (abs(n.x) > abs(n.z)) ? vObjectPos.z : vObjectPos.x;
        float roof_v = vObjectPos.y;

        // Tangent frame down the roof slope
        vec3 T_slope = normalize(cross(n, vec3(0.0, 1.0, 0.0)));
        vec3 B_slope = cross(n, T_slope);

        // Shingle rows stepped down the slope (0.40m row spacing, 0.30m tile width)
        float shingle_row_coord = roof_v / 0.40;
        float shingle_row = floor(shingle_row_coord);
        float shingle_v = fract(shingle_row_coord);

        float shingle_col_coord = (roof_u + mod(shingle_row, 2.0) * 0.15) / 0.30;
        float shingle_col = floor(shingle_col_coord);
        float shingle_u = fract(shingle_col_coord);

        // Sawtooth overlap tilt: shingle tilts down slope, steps back at lip
        float saw_tilt = (shingle_v - 0.5) * 0.32;
        float lip_crevice = smoothstep(0.85, 0.96, shingle_v);
        float side_crevice = 1.0 - smoothstep(0.04, 0.10, min(shingle_u, 1.0 - shingle_u));

        float shingle_fade = 1.0 - smoothstep(0.4, 4.5, footprint);
        vec3 grad_roof = (B_slope * (-saw_tilt) + T_slope * ((shingle_u - 0.5) * 0.18)) * shingle_fade;
        n = normalize(n + grad_roof);

        // Material differentiation:
        // High castle towers/keep, cloister, beacon, watch-post use weathered alpine slate;
        // Almhütte and barns use weathered wood shingles with stone weights;
        // stone bridge uses cobblestone roadbed.
        float shingle_hash = groundHash(vec2(shingle_col, shingle_row), 131u);
        vec3 terracotta = mix(vec3(0.32, 0.12, 0.05), vec3(0.46, 0.19, 0.08), shingle_hash);
        vec3 cedar      = mix(vec3(0.24, 0.15, 0.09), vec3(0.34, 0.22, 0.12), shingle_hash);
        vec3 slate      = mix(vec3(0.19, 0.20, 0.22), vec3(0.28, 0.29, 0.31), shingle_hash);

        bool isFortRoof = (vType <= 8u || vType == 34u || vType == 39u || vType == 42u || vType == 45u || vType == 46u || vType == 48u || vType == 50u || roof_v > 50.0);
        vec3 village_roof = mix(terracotta, cedar, smoothstep(0.40, 0.65, shingle_hash));
        if (vType == 43u || vType == 47u || vType == 49u || vType == 51u) village_roof = cedar;
        vec3 roof_color = isFortRoof ? slate : village_roof;

        if (vType == 41u) {
            // Arched stone bridge roadbed: granite cobblestone pavers
            float cobble_u = fract(roof_u / 0.55);
            float cobble_v = fract(roof_v / 0.55);
            float cobble_h = groundHash(vec2(floor(roof_u / 0.55), floor(roof_v / 0.55)), 197u);
            vec3 cobble_col = mix(vec3(0.25, 0.255, 0.26), vec3(0.39, 0.38, 0.36), cobble_h);
            float cobble_joint = smoothstep(0.04, 0.10, min(cobble_u, 1.0 - cobble_u))
                * smoothstep(0.04, 0.10, min(cobble_v, 1.0 - cobble_v));
            roof_color = mix(vec3(0.14, 0.14, 0.15), cobble_col, cobble_joint);
        } else if (vType == 40u) {
            // Almhütte: stone-weighted alpine cedar shingles (Schwersteine on Legschindeldach)
            float ballast_u = fract((roof_u + 0.3) / 1.4);
            float ballast_v = fract((roof_v + 0.2) / 1.0);
            float is_stone = step(0.65, ballast_u) * step(0.60, ballast_v);
            vec3 ballast_col = mix(vec3(0.32, 0.33, 0.35), vec3(0.42, 0.40, 0.38), shingle_hash);
            roof_color = mix(cedar, ballast_col, is_stone * 0.9);
        } else if ((vType == 42u && (vPart == 1u || vObjectPos.y > 44.0)) || (vType == 48u && (vPart == 1u || vObjectPos.y > 6.0))) {
            // Signal Fire Beacon: glowing fire brazier at summit
            float ember_h = groundHash(vec2(floor(vObjectPos.x * 3.0), floor(vObjectPos.z * 3.0)), 283u);
            roof_color = mix(vec3(0.12, 0.11, 0.10), vec3(1.4, 0.42, 0.06), smoothstep(0.45, 0.85, ember_h));
        }

        // Darken in crevices and edges
        float crevice_total = max(lip_crevice, side_crevice * 0.70) * shingle_fade;
        albedo = mix(roof_color, roof_color * 0.40, crevice_total * 0.65);
        roughness = mix(0.68, 0.85, crevice_total);
        ao = mix(0.92, 0.65, crevice_total);
        }
    } else if (vMaterial >= 3u) {
        // Procedural scatter: painterly foliage canopies and mineral
        // erratic boulders. vMoisture carries (species + random) packed by the
        // vertex stage; the shared sun/sky PBR and cloud-shadow terms below
        // apply unchanged so trees sit in the same light as the terrain.
        float rnd = fract(vMoisture);
        float species = floor(vMoisture);
        if (vMaterial == 5u) {
            // Mineral erratic boulders: alpine granodiorite & weathered limestone
            // with yellow-green crustose map lichen (Rhizocarpon geographicum)
            // and orange sunburst lichen (Xanthoria elegans).
            float rock_grain = groundHash(vec2(floor(vObjectPos.x * 3.5), floor(vObjectPos.z * 3.5)), 311u);
            vec3 granite = mix(vec3(0.28, 0.285, 0.29), vec3(0.42, 0.41, 0.39), rnd);
            granite = mix(granite, granite * 0.72, rock_grain * 0.4);

            // Crustose map lichen: distinctive lime/olive patchwork with dark apothecia edges
            float lichen_noise = groundHash(vec2(floor(vObjectPos.x * 7.0), floor(vObjectPos.y * 7.0)), 337u);
            float is_lichen = smoothstep(0.48, 0.68, lichen_noise);
            vec3 lichen_map = vec3(0.38, 0.44, 0.16); // Yellow-green Rhizocarpon
            vec3 lichen_orange = vec3(0.68, 0.34, 0.08); // Orange Xanthoria on sunny faces
            vec3 lichen_col = mix(lichen_map, lichen_orange, smoothstep(0.3, 0.8, n.y * 0.5 + 0.5));
            albedo = mix(granite, lichen_col, is_lichen * 0.75);

            // Quartz vein streaks cutting through alpine boulders
            float vein = smoothstep(0.02, 0.06, abs(fract(vObjectPos.x * 0.8 + vObjectPos.y * 0.6) - 0.5));
            albedo = mix(albedo, vec3(0.65, 0.67, 0.70), (1.0 - vein) * 0.35);

            // Top surfaces catch green velvet moss in moist hollows
            float top_moss = smoothstep(0.60, 0.95, n.y) * smoothstep(0.35, 0.65, 1.0 - slope);
            albedo = mix(albedo, vec3(0.12, 0.22, 0.06), top_moss * 0.55);

            // Snow line accumulation
            float boulder_snow = smoothstep(1620.0, 1920.0, altitude) * smoothstep(0.5, 0.9, n.y);
            albedo = mix(albedo, vec3(0.72, 0.78, 0.84), boulder_snow * 0.70);

            roughness = mix(0.86, 0.96, top_moss);
            ao = mix(0.85, 0.72, top_moss);
        } else if (vMaterial == 4u) {
            // Bark: vertical grain on the trunk box.
            // species 0: Norway spruce / pine (rugged charcoal brown)
            // species 1: Mountain broadleaf / birch (papery silver-white with dark lentils)
            // species 2: Alpine larch (deeply furrowed reddish-ochre bark)
            // species 3: Swiss stone pine / Zirbe (ash-grey plated resinous bark)
            // species 4: Subalpine dwarf shrub (gnarled weathered dwarf wood)
            float bark_hash = groundHash(vec2(floor(vObjectPos.y * 0.85), 0.0), 229u);
            float bark_stripe = 1.0 - smoothstep(0.04, 0.10, abs(fract(vObjectPos.y * 4.0) - 0.5) * 2.0);
            vec3 pine = vec3(0.225, 0.14, 0.085);
            vec3 birch = vec3(0.60, 0.56, 0.47);
            vec3 larch_bark = vec3(0.32, 0.17, 0.09);
            vec3 zirbe_bark = vec3(0.20, 0.18, 0.16);
            vec3 shrub_bark = vec3(0.18, 0.13, 0.08);
            vec3 base_bark = pine;
            if (species == 1.0) base_bark = birch;
            else if (species == 2.0) base_bark = larch_bark;
            else if (species == 3.0) base_bark = zirbe_bark;
            else if (species == 4.0) base_bark = shrub_bark;
            albedo = base_bark * (0.78 + 0.48 * bark_hash);
            albedo = mix(albedo, albedo * 0.62, bark_stripe * 0.35);
            roughness = 0.95;
            ao = 0.82;
        } else {
            // Distinct alpine flora palettes:
            // 0 = Norway Spruce: deep forest green conifer needles
            vec3 spruce_col = vec3(0.048, 0.122, 0.058) * (0.78 + 0.44 * rnd);
            // 1 = Mountain Broadleaf: lush emerald leaves with golden autumn accents
            vec3 broadleaf_col = vec3(0.105, 0.168, 0.062) * (0.76 + 0.48 * rnd);
            float autumn = smoothstep(0.82, 0.98, rnd);
            vec3 autumn_col = mix(vec3(0.72, 0.28, 0.05), vec3(0.85, 0.52, 0.08), rnd);
            broadleaf_col = mix(broadleaf_col, autumn_col, autumn * 0.78);
            // 2 = Alpine Larch (Larix decidua): luminous golden-amber autumn foliage
            float larch_var = groundHash(vec2(floor(vObjectPos.y * 1.5), rnd * 10.0), 241u);
            vec3 larch_gold = mix(vec3(0.88, 0.60, 0.10), vec3(0.94, 0.44, 0.06), larch_var * 0.45);
            larch_gold *= (0.85 + 0.35 * rnd);
            // 3 = Swiss Stone Pine / Zirbe (Pinus cembra): cool blue-green silvery needles
            vec3 zirbe_col = vec3(0.045, 0.108, 0.085) * (0.82 + 0.38 * rnd);
            // 4 = Subalpine Dwarf Shrub / Alpenrose: dark leathery leaves with blooming magenta flowers
            vec3 alpen_col = vec3(0.055, 0.110, 0.048);
            float flower_specks = smoothstep(0.68, 0.88, groundHash(vec2(floor(vObjectPos.x * 6.0), floor(vObjectPos.z * 6.0)), 379u));
            vec3 magenta_bloom = vec3(0.72, 0.08, 0.32); // Rhododendron ferrugineum
            alpen_col = mix(alpen_col, magenta_bloom, flower_specks * 0.85);

            albedo = spruce_col;
            if (species == 1.0) albedo = broadleaf_col;
            else if (species == 2.0) albedo = larch_gold;
            else if (species == 3.0) albedo = zirbe_col;
            else if (species == 4.0) albedo = alpen_col;

            // Early snowline frosting on high canopies
            float canopy_snow = smoothstep(1600.0, 1880.0, altitude) * max(n.y, 0.0);
            albedo = mix(albedo, vec3(0.68, 0.74, 0.80), canopy_snow * 0.60);
            roughness = 0.90;
            ao = 0.78;
        }
    }
    vec3 v = normalize(ubo.campos.xyz - hit);
    float no_v = max(dot(n, v), 0.001);
    vec3 sun = normalize(ubo.sunDir.xyz);
    float no_l = max(dot(n, sun), 0.0);
    vec3 f0 = vec3(0.020);

    vec3 sky_irradiance = groundSkyIrradiance(n);
    float land_response = vMaterial == 0u ? 1.0 - water : 0.0;
    vec3 interface_fresnel = fresnel(f0, no_v);
    vec3 land_reflectance = groundEnvironmentBRDF(f0, roughness, no_v);
    vec3 diffuse_transmission = vec3(1.0)
        - mix(interface_fresnel, land_reflectance, land_response);
    vec3 color = albedo * diffuse_transmission * sky_irradiance * ao;

    vec3 reflected = reflect(-v, n);
    vec3 env_dir = normalize(mix(reflected, n, roughness * roughness * 0.85));
    vec3 env;
    if (water > 0.0) {
        // Water mirrors the directional sky (circumsolar aureole + azimuthal
        // horizon gradient) where Fresnel makes reflection matter - grazing
        // slicks. Near-vertical views from altitude keep the elevation LUT:
        // through a ~2% Fresnel the broad warm circumsolar lobe would
        // otherwise warm-cast the whole alpine blue surface toward murk.
        // The reflected radiance is capped at the scene's linear scale
        // (chromaticity preserved); the sun-glitter hotspot belongs to the
        // GGX direct specular driven by the wave normals.
        vec3 atmo_origin = atmoModelOrigin(ubo.campos.xyz, ubo.groundBase.w);
        vec3 env_sky = atmoRadianceCheapTrHoisted(atmo_origin, env_dir, sun,
            ubo.sunColor.rgb, vAtmoTrSun, vAtmoMulti, vAtmoDensities);
        float env_lum = dot(env_sky, vec3(0.2126, 0.7152, 0.0722));
        env_sky *= min(1.0, 2.5 / max(env_lum, 1e-4));
        env = mix(fastSkyAtmosphere(env_dir.y), env_sky, smoothstep(0.75, 0.45, no_v));
    } else {
        env = fastSkyAtmosphere(env_dir.y);
    }
    float spec_ao = clamp(
        pow(no_v + ao, exp2(-16.0 * roughness - 1.0)) - 1.0 + ao,
        0.0, 1.0);
    vec3 env_reflectance = mix(interface_fresnel * (0.45 + 0.55 * (1.0 - roughness)),
        land_reflectance, land_response);
    color += env * env_reflectance * spec_ao;

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
        // A bounded wrap approximation softens the grass canopy response.
        // This is an aggregate leaf-orientation cue, not blade transmission.
        float leaf_diffuse = clamp((dot(n, sun) + 0.35) / 1.8225, 0.0, 1.0);
        float diffuse_response = mix(no_l * fd_v * fd_l, leaf_diffuse, vegetation * 0.70);
        if (vMaterial == 3u) {
            float backlight = pow(clamp(dot(-v, sun), 0.0, 1.0), 3.0) * clamp(dot(n, -sun) * 0.4 + 0.6, 0.0, 1.0);
            diffuse_response += backlight * 0.45;
        }
        vec3 direct_diffuse = albedo * (vec3(1.0) - fresnel(f0, no_l))
            * ubo.sunColor.rgb * diffuse_response;

        #ifdef ENABLE_RT
        float visibility = 1.0;
        bool is_structure = (vMaterial == 1u || vMaterial == 2u);
        bool trace_aircraft = (rtShadowGate(local_xz, hit.y) > 0.0);

        // Check if terrain is inside a landmark settlement footprint
        bool trace_settlement_terrain = false;
        if (vMaterial == 0u && hit_t < 3500.0) {
            float s_tile_z = floor(world_xz.y / 16384.0);
            float s_base_z = s_tile_z * 16384.0 + 3450.0;
            float s_tile_x = floor(world_xz.x / 16384.0);
            float s_base_x = s_tile_x * 16384.0 + terrainValleyCenter(mod(s_base_z, 65536.0)) + 1180.0;
            vec2 s_delta = world_xz - vec2(s_base_x - 100.0, s_base_z - 150.0);
            trace_settlement_terrain = (dot(s_delta, s_delta) < 202500.0); // 450m radius
        }

        if (trace_aircraft) {
            vec3 geo_n = (vMaterial == 0u) ? normalize(vTerrainNormal) : n;
            vec3 probe = hit + geo_n * 0.8;
            float light_t = (hit.y - ubo.nodes[0][3].y) / (-ubo.sunDir.y);
            float t_max = clamp(light_t + 25.0, 30.0, 8000.0);
            float t_min = (vMaterial == 0u) ? 0.35 : 0.05;
            visibility = rtSunVisibility(probe, t_min, t_max);
        } else if (is_structure) {
            // Precise probe offset along wall/roof normal to catch sharp roof eave and tower shadows
            vec3 probe = hit + n * 0.18;
            visibility = rtSunVisibility(probe, 0.05, 2500.0);
        } else if (trace_settlement_terrain) {
            vec3 geo_n = normalize(vTerrainNormal);
            vec3 probe = hit + geo_n * 0.7;
            visibility = rtSunVisibility(probe, 0.35, 800.0);
        }
#else
        float shadow = groundAircraftShadow(local_xz, hit.y);
        float visibility = 1.0 - shadow * 0.30;
#endif
        float cloud_visibility = cloudSunVisibility(world_xz, altitude, sun,
            mod(ubo.flex.y * ubo.cameraParams2.w * CLOUD_DRIFT_SPEED,
                CLOUD_FIELD_PERIOD));
        color += (direct_diffuse + direct_spec) * visibility * cloud_visibility;
    }

    // Foliage canopy forward transmission / backlight scattering:
    // When viewing foliage backlit by the sun (sun behind canopy), thin leaves,
    // pine needle tufts, and golden larch canopies transmit sunlight forward,
    // creating a luminous warm rim and glowing translucent canopy volume.
    if (vMaterial == 3u) {
        float forward_phase = pow(max(dot(-v, sun), 0.0), 3.2);
        float back_incidence = max(dot(-n, sun), 0.0);
        float species_kind = floor(vMoisture);
        float trans_strength = (species_kind == 2.0) ? 1.6 : ((species_kind == 1.0) ? 1.2 : 0.85);
        vec3 trans_color = (species_kind == 2.0) ? vec3(0.95, 0.65, 0.15) : (albedo * 2.4);
        vec3 forward_scatter = trans_color * ubo.sunColor.rgb
            * (forward_phase * 0.70 + back_incidence * 0.30) * trans_strength;
        float cloud_vis = cloudSunVisibility(world_xz, altitude, sun,
            mod(ubo.flex.y * ubo.cameraParams2.w * CLOUD_DRIFT_SPEED, CLOUD_FIELD_PERIOD));
        color += forward_scatter * cloud_vis * 0.55;
    }

    // Multi-spectral atmospheric perspective: physical wavelength-dependent Rayleigh
    // and Mie extinction preserves authentic alpine depth layers across receding ridges.
    // The ground meets the sky with zero seam at the horizon.
    vec3 haze = ubo.skyHorizon.rgb;
    // Wavelength extinction arrived flat from the vertex stage (camera-only
    // value, identical across each triangle), replacing four exps per pixel.
    vec3 ext = vExtinction;
    vec3 transmittance = exp(-hit_t * ext);
    vec3 inscatter = haze * (vec3(1.0) - transmittance);
    vec3 final_color = color * transmittance + inscatter;
    outColor = vec4(final_color, 1.0);
}
