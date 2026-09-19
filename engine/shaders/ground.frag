#version 460

// Rasterized alpine landscape and medieval landmarks. Hardware depth handles
// mountain silhouettes and occlusion; no fragment terrain ray march is used.
//
// The material is a temperate meadow/soil layer rather than a single noisy
// RGB value. Its procedural channels are world-stable, filtered by the
// camera-ray footprint, and evaluated as linear-light PBR inputs:
//   - broad value noise makes grass, exposed soil, and damp patches;
//   - smaller noise controls albedo, roughness, and a micro-relief normal;
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
    if (vMaterial == 0u && n.y < 0.0) n = -n;
    if (vMaterial != 0u && dot(n, -view_dir) < 0.0) n = -n;
    float slope = 1.0 - clamp(n.y, 0.0, 1.0);
    float altitude = hit.y - ubo.groundBase.w;
    float water = vMaterial == 0u ? 1.0 - smoothstep(TERRAIN_WATER - 0.5,
        TERRAIN_WATER + 1.5, vLandHeight) : 0.0;

    float macro = groundFilteredNoise(local_xz, 96.0, footprint, 11u);
    float patch_noise = groundFilteredNoise(local_xz, 24.0, footprint, 29u);
    float clump = groundFilteredNoise(local_xz, 6.0, footprint, 53u);
    float grain = (footprint > 2.7) ? 0.5 : groundFilteredNoise(local_xz, 1.5, footprint, 79u);
    float pebble = (footprint > 0.9) ? 0.5 : groundFilteredNoise(local_xz, 0.5, footprint, 107u);

    float grass_mask = smoothstep(0.33, 0.67,
        macro * 0.55 + patch_noise * 0.30 + clump * 0.15);
    float soil_mask = smoothstep(0.52, 0.84, patch_noise * 0.55 + clump * 0.45)
        * (1.0 - grass_mask * 0.45);
    float pebble_mask = smoothstep(0.80, 0.96, pebble) * (1.0 - grass_mask * 0.50);
    float wetness = smoothstep(0.70, 0.92, patch_noise) * (1.0 - grass_mask * 0.45);

    vec3 grass = mix(vec3(0.045, 0.085, 0.018), vec3(0.20, 0.29, 0.065), clump);
    vec3 soil = mix(vec3(0.055, 0.031, 0.014), vec3(0.185, 0.090, 0.030), grain);
    vec3 albedo = mix(grass, soil, soil_mask);
    albedo = mix(albedo, vec3(0.16, 0.145, 0.105), pebble_mask * 0.24);
    albedo *= 0.88 + macro * 0.18 + grain * 0.08;
    // A thin water film lowers apparent diffuse albedo and shifts it toward a
    // neutral dark reflection; its stronger effect is the roughness change.
    albedo *= mix(vec3(1.0), vec3(0.70, 0.76, 0.73), wetness * 0.25);

    float roughness = clamp(mix(0.90, 0.34, wetness) + pebble_mask * 0.04, 0.26, 0.94);
    float ao = clamp(0.86 + grass_mask * 0.10 - pebble_mask * 0.12, 0.65, 1.0);

    if (vMaterial == 0u) {
        float rock = smoothstep(0.18, 0.48, slope)
            + smoothstep(1300.0, 2100.0, altitude) * 0.30;
        vec3 stone = mix(vec3(0.14, 0.155, 0.16), vec3(0.30, 0.285, 0.24), patch_noise);
        albedo = mix(albedo, stone, clamp(rock, 0.0, 1.0));
        float forest = smoothstep(0.46, 0.71, macro)
            * smoothstep(350.0, 650.0, altitude) * (1.0 - smoothstep(1200.0, 1650.0, altitude))
            * (1.0 - smoothstep(0.28, 0.50, slope));
        albedo = mix(albedo, vec3(0.018, 0.065, 0.039) * (0.8 + clump * 0.4), forest * 0.85);
        float snowLine = 1860.0 + (macro - 0.5) * 300.0;
        float snow = smoothstep(snowLine, snowLine + 240.0, altitude)
            * (1.0 - smoothstep(0.34, 0.66, slope));
        albedo = mix(albedo, vec3(0.77, 0.84, 0.88), snow);
        roughness = mix(roughness, 0.68, snow);

        // A pale winding track follows the valley's eastern bank to the village.
        float valleyX = mod(world_xz.x - terrainValleyCenter(mod(world_xz.y, TERRAIN_PERIOD))
            + 8192.0, 16384.0) - 8192.0;
        float roadDistance = abs(valleyX - 1050.0);
        float road = 1.0 - smoothstep(9.0, 13.0 + footprint, roadDistance);
        road *= (1.0 - smoothstep(0.2, 0.4, slope)) * (1.0 - water);
        albedo = mix(albedo, vec3(0.32, 0.24, 0.14), road * 0.8);
        vec3 relief = groundNormal(local_xz, footprint);
        n = normalize(n + vec3(relief.x, 0.0, relief.z) * (1.0 - snow) * 0.5);
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
        albedo = mix(vec3(0.42, 0.36, 0.25), vec3(0.25, 0.25, 0.235), patch_noise * 0.7);
        albedo *= 1.0 - mortar * 0.2;
        vec2 face = vec2(abs(n.x) > abs(n.z) ? vObjectPos.z : vObjectPos.x, vObjectPos.y);
        vec2 window = abs(fract(face / vec2(12.0, 20.0)) - 0.5);
        float slit = (1.0 - smoothstep(0.06, 0.10, window.x))
            * (1.0 - smoothstep(0.15, 0.19, window.y))
            * smoothstep(8.0, 12.0, vObjectPos.y) * (1.0 - abs(n.y))
            * (1.0 - smoothstep(2.0, 8.0, footprint));
        albedo = mix(albedo, vec3(0.012, 0.017, 0.015), slit);
        roughness = 0.82;
        ao = 0.85;
    } else {
        albedo = mix(vec3(0.08, 0.13, 0.15), vec3(0.23, 0.075, 0.035), patch_noise);
        roughness = 0.70;
        ao = 0.95;
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

    // For ground near the camera, atmospheric extinction is < 0.005 (< 0.5% haze).
    // Skipping aerial perspective for hit_t < 150.0 saves chord length ray-sphere
    // intersection and atmo evaluations on the highest-fill foreground fragments.
    if (hit_t < 150.0) {
        outColor = vec4(color, 1.0);
        return;
    }

    // Physical atmospheric perspective: the in-scattered sky radiance and
    // extinction come from the same model as the sky dome, so the ground
    // blends seamlessly into the horizon.
    vec3 atmo_origin = atmoModelOrigin(ubo.campos.xyz, ubo.groundBase.w);
    vec3 trSun = exp(-atmoSunOpticalDepth(atmo_origin, sun));
    vec3 haze = atmoRadianceCheapTr(atmo_origin, view_dir, sun, ubo.sunColor.rgb, trSun);
    // Per-species exponential extinction along the view ray. The coefficients
    // come from the sea-level density at camera altitude, giving warm blue
    // extinction that thickens with Mie haze near the horizon.
    float cam_h = max(ubo.campos.y - ubo.groundBase.w, 0.0);
    float dR = exp(-cam_h / 8000.0);
    float dM = exp(-cam_h / 1200.0);
    vec3 ext = ATMO_BETA_RAYLEIGH * dR + ATMO_BETA_MIE_EXTINCT * dM;
    vec3 transmittance = exp(-hit_t * ext);
    outColor = vec4(mix(haze, color, transmittance), 1.0);
}
