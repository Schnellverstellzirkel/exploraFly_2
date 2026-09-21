#version 450

// Far-field forest HLOD. One invocation expands one canonical 128 m canopy
// cell into one terrain-following raised surface. The aggregate record was
// derived from the same cached terrain/moisture/forest-cover field as the
// individual tree database; this stage performs no per-candidate placement.

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
    vec4 groundOrigin;
} ubo;

layout(set = 0, binding = 10) uniform texture2D terrain_tex;
layout(set = 0, binding = 11) uniform sampler terrain_smp;

// Binding 22 is reserved for the compact cull pass's cell ranges; keep the
// far-field aggregate in its own slot so both pipelines can share one set.
layout(set = 0, binding = 24, std430) readonly buffer CanopyInstances {
    uvec4 instances[];
};

layout(location = 0) out vec3 vPosition;
layout(location = 1) out float vLandHeight;
layout(location = 2) flat out uint vMaterial;
layout(location = 3) out vec3 vObjectPos;
layout(location = 4) out vec3 vTerrainNormal;
layout(location = 5) out float vMoisture;
layout(location = 6) flat out vec3 vExtinction;
layout(location = 7) flat out uint vType;
layout(location = 8) flat out uint vPart;
layout(location = 9) out vec2 vShape;
layout(location = 10) out float vCloudVisibility;

const vec3 ATMO_BETA_RAYLEIGH = vec3(5.802e-6, 13.558e-6, 33.1e-6);
const vec3 ATMO_BETA_MIE_EXTINCT = vec3(4.44e-6);
const vec3 ATMO_BETA_OZONE = vec3(0.650e-6, 1.881e-6, 0.085e-6);

// A shared four-by-four canopy surface follows the immutable terrain cache.
// The per-cell record decides whether this patch is submitted; the local
// height is reconstructed from the same forest-density recipe at each grid
// sample, so adjacent occupied cells meet without a center-pinned crown.
const uint CANOPY_GRID = 4u;
const float CANOPY_CELL_METRES = 128.0;
const float CANOPY_TERRAIN_CLEARANCE = 3.0;

// Terrain vertices use the same 64 m lattice, but canopy vertices also land
// between lattice points. Reconstruct the exact piecewise-linear surface used
// by terrain_indices instead of snapping to one texel or using a bilinear
// curve; otherwise a steep cell can put the patch through the terrain and
// depth-test it in and out as the camera moves.
vec4 canopyTerrainSample(vec2 absoluteXZ) {
    vec2 terrainCoord = absoluteXZ / TERRAIN_CELL_METRES;
    ivec2 base = ivec2(floor(terrainCoord));
    vec2 fraction = fract(terrainCoord);
    ivec2 mask = ivec2(int(TERRAIN_CELLS - 1u));
    vec4 t00 = texelFetch(sampler2D(terrain_tex, terrain_smp), base & mask, 0);
    vec4 t10 = texelFetch(sampler2D(terrain_tex, terrain_smp), (base + ivec2(1, 0)) & mask, 0);
    vec4 t01 = texelFetch(sampler2D(terrain_tex, terrain_smp), (base + ivec2(0, 1)) & mask, 0);
    vec4 t11 = texelFetch(sampler2D(terrain_tex, terrain_smp), (base + ivec2(1, 1)) & mask, 0);
    if (fraction.x + fraction.y <= 1.0) {
        return t00 + fraction.x * (t10 - t00) + fraction.y * (t01 - t00);
    }
    return t11 + (1.0 - fraction.y) * (t10 - t11)
        + (1.0 - fraction.x) * (t01 - t11);
}

float atmoOzoneDensity(float h) {
    float density = (h < 25000.0)
        ? h / 15000.0 - 2.0 / 3.0
        : -h / 15000.0 + 8.0 / 3.0;
    return clamp(density, 0.0, 1.0);
}

void main() {
    uvec4 packed = instances[gl_InstanceIndex];
    vec2 canonicalXZ = vec2(uintBitsToFloat(packed.x), uintBitsToFloat(packed.y));
    vec2 origin = terrainOrigin(ubo.groundOrigin);
    vec2 cameraWorld = origin + ubo.campos.xz;
    vec2 cameraWrapped = mod(mod(cameraWorld, vec2(TERRAIN_PERIOD))
        + vec2(TERRAIN_PERIOD), vec2(TERRAIN_PERIOD));
    vec2 delta = canonicalXZ - cameraWrapped;
    delta -= floor(delta / TERRAIN_PERIOD + 0.5) * TERRAIN_PERIOD;
    vec2 localXZ = ubo.campos.xz + delta;

    uint triangle = uint(gl_VertexIndex) / 3u;
    uint corner = uint(gl_VertexIndex) % 3u;
    uint quad = triangle / 2u;
    uint quad_x = quad % (CANOPY_GRID - 1u);
    uint quad_z = quad / (CANOPY_GRID - 1u);
    uint a = quad_z * CANOPY_GRID + quad_x;
    uint b = a + 1u;
    uint c = a + CANOPY_GRID;
    uint d = c + 1u;
    bool second = (triangle & 1u) != 0u;
    uint grid_index;
    if (!second) {
        grid_index = (corner == 0u) ? a : ((corner == 1u) ? c : b);
    } else {
        grid_index = (corner == 0u) ? b : ((corner == 1u) ? c : d);
    }
    vec2 grid_uv = vec2(
        float(grid_index % CANOPY_GRID),
        float(grid_index / CANOPY_GRID)) / float(CANOPY_GRID - 1u);
    vec2 offset = (grid_uv - 0.5) * CANOPY_CELL_METRES;
    vec2 localPosition = localXZ + offset;
    vec2 absoluteXZ = origin + localPosition;
    vec4 terrain = canopyTerrainSample(absoluteXZ);
    float terrainGround = max(terrain.x, TERRAIN_WATER);
    float terrainSlope = 1.0 - inversesqrt(
        terrain.y * terrain.y + terrain.z * terrain.z + 1.0);
    // Use the same thresholded stand mask as the terrain forest material.
    // Feeding the raw cover*patch product here made the far surface contract
    // at biome/patch edges while the mid-range terrain stayed dark, so one
    // forest appeared to change footprint as the camera advanced.
    float field_density = vegetationForestStandCoverage(
        terrain.x, terrainSlope, terrain.w, absoluteXZ);
    // A low-frequency raised surface is the far representation. The explicit
    // clearance keeps even sparse patches above the terrain triangle while
    // the matching piecewise sample prevents them from floating over steep
    // interpolation changes.
    float canopy_surface = CANOPY_TERRAIN_CLEARANCE + 18.0 * field_density;
    vec3 normal = normalize(vec3(-terrain.y, 1.0, -terrain.z));

    float distance_to_camera = length(localPosition - ubo.campos.xz);
    float canopy_fade = smoothstep(
        VEGETATION_CANOPY_FADE_START,
        VEGETATION_CANOPY_FADE_FAR_END,
        distance_to_camera);

    vPosition = vec3(localPosition.x, terrainGround + ubo.groundBase.w + canopy_surface,
        localPosition.y);
    vLandHeight = terrain.x;
    vMaterial = 3u;
    vObjectPos = vec3(offset.x, canopy_surface, offset.y);
    vTerrainNormal = normal;
    // Carry the terrain moisture so the fragment stage can evaluate the
    // authoritative stand mask at the actual canopy pixel. Material
    // variation is derived from the stable cell coordinate there.
    vMoisture = terrain.w;
    vType = 52u;
    vPart = 7u;
    vCloudVisibility = cloudSunVisibility(
        absoluteXZ,
        terrainGround + canopy_surface,
        normalize(ubo.sunDir.xyz),
        mod(ubo.flex.y * ubo.cameraParams2.w * CLOUD_DRIFT_SPEED,
            CLOUD_FIELD_PERIOD));
    // x is the shared tree/canopy dither fade; y retains aggregate density.
    vShape = vec2(canopy_fade, field_density);

    float cam_h = max(ubo.campos.y - ubo.groundBase.w, 0.0);
    float dR = exp(-cam_h / 8000.0);
    float dM = exp(-cam_h / 1200.0);
    float dO = atmoOzoneDensity(cam_h);
    vExtinction = ATMO_BETA_RAYLEIGH * dR + ATMO_BETA_MIE_EXTINCT * dM
        + ATMO_BETA_OZONE * dO;
    gl_Position = ubo.viewProj * vec4(vPosition, 1.0);
}
