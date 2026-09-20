#version 450

// Dedicated raster vegetation pass. One invocation decodes one persistent
// tree/boulder instance; the 108 vertices below only expand that descriptor
// into a trunk and three crown octahedra. Placement, biome tests, settlement
// clearance, and terrain height lookup have already happened at startup.

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

layout(set = 0, binding = 21, std430) readonly buffer VegetationInstances {
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
layout(location = 9) flat out vec2 vShape;

const vec3 ATMO_BETA_RAYLEIGH = vec3(5.802e-6, 13.558e-6, 33.1e-6);
const vec3 ATMO_BETA_MIE_EXTINCT = vec3(4.44e-6);
const vec3 ATMO_BETA_OZONE = vec3(0.650e-6, 1.881e-6, 0.085e-6);

float atmoOzoneDensity(float h) {
    float density = (h < 25000.0)
        ? h / 15000.0 - 2.0 / 3.0
        : -h / 15000.0 + 8.0 / 3.0;
    return clamp(density, 0.0, 1.0);
}

const vec3 BOX[8] = vec3[8](vec3(-1, 0, -1), vec3(1, 0, -1),
    vec3(-1, 1, -1), vec3(1, 1, -1), vec3(-1, 0, 1), vec3(1, 0, 1),
    vec3(-1, 1, 1), vec3(1, 1, 1));
const int BOX_TRI[36] = int[36](0, 2, 1, 1, 2, 3, 5, 7, 4, 4, 7, 6,
    4, 6, 0, 0, 6, 2, 1, 3, 5, 5, 3, 7, 2, 6, 3, 3, 6, 7, 4, 0, 5, 5, 0, 1);
const vec3 OCTA[6] = vec3[6](vec3(1, 0, 0), vec3(-1, 0, 0), vec3(0, 0, 1),
    vec3(0, 0, -1), vec3(0, 1, 0), vec3(0, -1, 0));
const int OCTA_TRI[24] = int[24](0, 2, 4, 2, 1, 4, 1, 3, 4, 3, 0, 4,
    2, 0, 5, 1, 2, 5, 3, 1, 5, 0, 3, 5);

void main() {
    uvec4 packed = instances[gl_InstanceIndex];
    vec2 canonicalXZ = vec2(uintBitsToFloat(packed.x), uintBitsToFloat(packed.y));
    float ground = uintBitsToFloat(packed.z);
    uint metadata = packed.w;
    uint speciesCode = metadata & 7u;
    float sizeR = float((metadata >> 8u) & 255u) * (1.0 / 255.0);
    bool isBoulder = ((metadata >> 16u) & 1u) != 0u;
    float size = 0.70 + 1.60 * float((metadata >> 17u) & 32767u) * (1.0 / 32767.0);

    // Select the periodic copy nearest to the camera. The database stores
    // canonical coordinates in [0, TERRAIN_PERIOD), while the floating-origin
    // camera can cross either signed world boundary.
    vec2 origin = terrainOrigin(ubo.groundOrigin);
    vec2 cameraWorld = origin + ubo.campos.xz;
    vec2 cameraWrapped = mod(mod(cameraWorld, vec2(TERRAIN_PERIOD))
        + vec2(TERRAIN_PERIOD), vec2(TERRAIN_PERIOD));
    vec2 delta = canonicalXZ - cameraWrapped;
    delta -= floor(delta / TERRAIN_PERIOD + 0.5) * TERRAIN_PERIOD;
    vec2 localXZ = ubo.campos.xz + delta;

    float trunkW;
    float trunkH;
    float c0w, c0h, c0y;
    float c1w, c1h, c1y;
    float c2w, c2h, c2y;
    vec2 c0xz = vec2(0.0);
    vec2 c1xz = vec2(0.0);
    vec2 c2xz = vec2(0.0);
    float kind = float(speciesCode);
    if (isBoulder) {
        trunkW = 1.6 + 2.4 * sizeR;
        trunkH = 4.2 * size;
        c0w = trunkW * 0.85; c0h = trunkH * 0.55; c0y = trunkH * 0.45;
        c1w = trunkW * 0.70; c1h = trunkH * 0.75; c1y = trunkH * 0.65;
        c2w = trunkW * 0.60; c2h = trunkH * 0.40; c2y = trunkH * 0.30;
        c0xz = vec2(trunkW * 0.35, -trunkW * 0.25);
        c1xz = vec2(-trunkW * 0.25, trunkW * 0.30);
        c2xz = vec2(-trunkW * 0.35, -trunkW * 0.20);
    } else if (kind < 0.5) {
        trunkW = 0.55 * size; trunkH = 4.5 * size;
        c0w = 2.9 * size; c0h = 4.2 * size; c0y = 8.6 * size;
        c1w = 2.0 * size; c1h = 4.2 * size; c1y = 14.6 * size;
        c2w = 1.2 * size; c2h = 5.2 * size; c2y = 20.8 * size;
    } else if (kind < 1.5) {
        trunkW = 0.6 * size; trunkH = 3.6 * size;
        c0w = 7.0 * size; c0h = 3.4 * size; c0y = 1.2;
        c1w = 5.0 * size; c1h = 3.8 * size; c1y = 8.2 * size;
        c2w = 3.0 * size; c2h = 3.2 * size; c2y = 18.0 * size;
    } else if (kind < 2.5) {
        trunkW = 0.50 * size; trunkH = 5.0 * size;
        c0w = 4.8 * size; c0h = 2.8 * size; c0y = 7.0 * size;
        c1w = 3.4 * size; c1h = 3.2 * size; c1y = 13.0 * size;
        c2w = 1.8 * size; c2h = 4.5 * size; c2y = 19.5 * size;
    } else if (kind < 3.5) {
        trunkW = 0.75 * size; trunkH = 4.0 * size;
        c0w = 3.8 * size; c0h = 4.0 * size; c0y = 5.5 * size;
        c1w = 3.2 * size; c1h = 4.2 * size; c1y = 11.5 * size;
        c2w = 2.2 * size; c2h = 3.8 * size; c2y = 16.5 * size;
    } else {
        trunkW = 0.30 * size; trunkH = 0.8 * size;
        c0w = 3.2 * size; c0h = 1.2 * size; c0y = 0.6 * size;
        c1w = 2.2 * size; c1h = 1.0 * size; c1y = 1.4 * size;
        c2w = 1.2 * size; c2h = 0.8 * size; c2y = 2.0 * size;
    }

    uint corner = uint(gl_VertexIndex);
    vec3 p;
    if (corner < 36u) {
        p = BOX[BOX_TRI[corner]] * vec3(trunkW, trunkH, trunkW);
        vPart = 8u;
    } else if (corner < 60u) {
        p = OCTA[OCTA_TRI[corner - 36u]] * vec3(c0w, c0h, c0w);
        p.y += c0y;
        p.xz += c0xz;
        vPart = 7u;
    } else if (corner < 84u) {
        p = OCTA[OCTA_TRI[corner - 60u]] * vec3(c1w, c1h, c1w);
        p.y += c1y;
        p.xz += c1xz;
        vPart = 7u;
    } else {
        p = OCTA[OCTA_TRI[corner - 84u]] * vec3(c2w, c2h, c2w);
        p.y += c2y;
        p.xz += c2xz;
        vPart = 7u;
    }

    vType = isBoulder ? 51u : 50u;
    vShape = vec2(0.0);
    vMaterial = isBoulder ? 5u : (vPart == 8u ? 4u : 3u);
    vObjectPos = p;
    vLandHeight = ground;
    vTerrainNormal = vec3(0.0, 1.0, 0.0);
    vMoisture = kind + sizeR;

    // The exact source slot is intentionally not part of the compact record;
    // a stable position-derived phase preserves wind coherence without adding
    // another word to every instance.
    if (vPart == 7u && !isBoulder && p.y > 1.0) {
        uvec2 phaseCell = uvec2(floor(canonicalXZ / SCATTER_PITCH)) & uvec2(65535u);
        float phase = terrainHash(phaseCell, 443u) * 6.2831853;
        float sway = (p.y - 1.0) * (p.y - 1.0) * 3.0e-4;
        p.xz += vec2(sin(ubo.flex.y * 1.35 + phase),
            cos(ubo.flex.y * 1.13 + phase * 0.7)) * sway;
        vObjectPos = p;
    }
    vPosition = vec3(localXZ.x + p.x, ground + ubo.groundBase.w + p.y, localXZ.y + p.z);

    float cam_h = max(ubo.campos.y - ubo.groundBase.w, 0.0);
    float dR = exp(-cam_h / 8000.0);
    float dM = exp(-cam_h / 1200.0);
    float dO = atmoOzoneDensity(cam_h);
    vExtinction = ATMO_BETA_RAYLEIGH * dR + ATMO_BETA_MIE_EXTINCT * dM + ATMO_BETA_OZONE * dO;
    gl_Position = ubo.viewProj * vec4(vPosition, 1.0);
}
