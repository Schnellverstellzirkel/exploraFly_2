#version 450

// Fixed world lattice: camera motion only changes which cells are submitted.
// Shared vertices fetch immutable heights/normals; no view-dependent resampling.
// Terrain and coarse medieval landmarks share this one vertex-pulled draw.
// terrain.inc is injected by build.rs after #version.
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

layout(location = 0) out vec3 vPosition;
layout(location = 1) out float vLandHeight;
layout(location = 2) flat out uint vMaterial;
layout(location = 3) out vec3 vObjectPos;
layout(location = 4) out vec3 vTerrainNormal;
layout(location = 5) out float vMoisture;
layout(location = 6) flat out vec3 vExtinction;

// Extinction depends only on camera altitude, so every vertex of a triangle
// computes the identical value and the flat interpolation is bit-exact.
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
const vec3 ROOF[6] = vec3[6](vec3(-1, 0, -1), vec3(1, 0, -1),
    vec3(-1, 0, 1), vec3(1, 0, 1), vec3(0, 1, -1), vec3(0, 1, 1));
const int ROOF_TRI[18] = int[18](0, 2, 5, 0, 5, 4, 1, 4, 5, 1, 5, 3, 0, 4, 1, 2, 3, 5);
// Scatter canopies: stacked octahedra read as conifer/broadleaf crowns from
// every camera angle, matching the cloud puffs' painterly language.
const vec3 OCTA[6] = vec3[6](vec3(1, 0, 0), vec3(-1, 0, 0), vec3(0, 0, 1),
    vec3(0, 0, -1), vec3(0, 1, 0), vec3(0, -1, 0));
const int OCTA_TRI[24] = int[24](0, 2, 4, 2, 1, 4, 1, 3, 4, 3, 0, 4,
    2, 0, 5, 1, 2, 5, 3, 1, 5, 0, 3, 5);

void main() {
    uint vertex = uint(gl_VertexIndex);
    vec2 origin = terrainOrigin(ubo.groundOrigin);
    vObjectPos = vec3(0.0);
    vTerrainNormal = vec3(0.0, 1.0, 0.0);
    if (vertex < TERRAIN_VERTICES) {
        uint stride = TERRAIN_CELLS + 1u;
        ivec2 grid = ivec2(int(vertex % stride), int(vertex / stride))
            - ivec2(int(TERRAIN_CELLS / 2u));
        ivec2 anchor = ivec2(floor((origin + ubo.campos.xz) / TERRAIN_CELL_METRES));
        ivec2 worldCell = anchor + grid;
        vec2 xz = vec2(worldCell) * TERRAIN_CELL_METRES - origin;
        ivec2 sampleCell = worldCell & ivec2(int(TERRAIN_CELLS - 1u));
        vec4 sampleData = texelFetch(sampler2D(terrain_tex, terrain_smp), sampleCell, 0);
        float ground = sampleData.x;
        vTerrainNormal = normalize(vec3(-sampleData.y, 1.0, -sampleData.z));
        vPosition = vec3(xz.x, max(ground, TERRAIN_WATER) + ubo.groundBase.w, xz.y);
        vLandHeight = ground;
        vMaterial = 0u;
        vMoisture = sampleData.w;
    } else if (vertex < TERRAIN_VERTICES + LANDMARK_VERTEX_COUNT) {
        uint landmarkVertex = vertex - TERRAIN_VERTICES;
        uint building = landmarkVertex / 54u;
        uint tile = building / TERRAIN_STRUCTURES;
        uint structure = building % TERRAIN_STRUCTURES;
        uint corner = landmarkVertex % 54u;
        vec2 cameraWorld = origin + ubo.campos.xz;
        vec2 tileCell = floor(cameraWorld / 16384.0)
            + vec2(int(tile % 3u) - 1, int(tile / 3u) - 1);
        float baseZ = tileCell.y * 16384.0 + 3450.0;
        vec2 base = vec2(tileCell.x * 16384.0
            + terrainValleyCenter(mod(baseZ, TERRAIN_PERIOD)) + 1180.0, baseZ);
        float wallHeight, roofHeight;
        vec4 shape = terrainStructure(structure, wallHeight, roofHeight);
        vec2 center = base + shape.xy;
        // Fixed landmark distance budget saves height evaluations without CPU
        // draw bookkeeping. This is a hard cutoff, not a pixel-error LOD rule.
        if (distance(center, cameraWorld) > 14000.0) {
            vPosition = vec3(0.0);
            vLandHeight = 0.0;
            vMaterial = 1u;
            vMoisture = 0.0;
            gl_Position = vec4(0.0, 0.0, 2.0, 1.0);
            return;
        }
        vec3 p;
        if (corner < 36u) {
            p = BOX[BOX_TRI[corner]] * vec3(shape.z, wallHeight, shape.w);
            vMaterial = 1u;
        } else {
            p = ROOF[ROOF_TRI[corner - 36u]] * vec3(shape.z + 1.0, roofHeight, shape.w + 1.0);
            p.y += wallHeight;
            vMaterial = 2u;
        }
        float foundation = max(terrainHeight(center), TERRAIN_WATER);
        vObjectPos = p;
        vLandHeight = foundation;
        vMoisture = 0.0;
        vPosition = vec3(center.x - origin.x + p.x,
            foundation + ubo.groundBase.w + p.y, center.y - origin.y + p.z);
    } else {
        // Procedural scatter: spruces, broadleaf trees, and boulders decoded
        // from a fixed slot grid around the camera, mirroring the
        // SCATTER_* constants in crates/world/src/lib.rs. Every slot owns 48
        // corners: two stacked canopy octahedra; empty slots and everything
        // past the range budget collapse here.
        uint scatterVertex = vertex - TERRAIN_VERTICES - LANDMARK_VERTEX_COUNT;
        uint slot = scatterVertex / SCATTER_CORNERS;
        uint corner = scatterVertex % SCATTER_CORNERS;
        ivec2 slotGrid = ivec2(int(slot % SCATTER_GRID), int(slot / SCATTER_GRID))
            - ivec2(int(SCATTER_GRID / 2u));
        vec2 cameraWorld = origin + ubo.campos.xz;
        ivec2 anchor = ivec2(floor(cameraWorld / SCATTER_PITCH));
        ivec2 worldSlot = anchor + slotGrid;
        uvec2 hcell = uvec2(worldSlot) & uvec2(65535u);
        vec2 jitter = vec2(terrainHash(hcell, 401u), terrainHash(hcell, 407u))
            - 0.5;
        float presence = terrainHash(hcell, 419u);
        float species = terrainHash(hcell, 431u);
        float sizeR = terrainHash(hcell, 433u);
        vec2 slotWorld = vec2(worldSlot) * SCATTER_PITCH + jitter * 34.0;

        // Biome gates from the containing terrain cell: no water, fade out at
        // the material shader's treeline, dense on moist forest ground,
        // sparse single trees on meadows, boulders on steep scree.
        ivec2 sampleCell = ivec2(floor(slotWorld / TERRAIN_CELL_METRES))
            & ivec2(int(TERRAIN_CELLS - 1u));
        vec4 sampleData = texelFetch(sampler2D(terrain_tex, terrain_smp), sampleCell, 0);
        vec3 cellNormal = normalize(vec3(-sampleData.y, 1.0, -sampleData.z));
        float slope = 1.0 - clamp(cellNormal.y, 0.0, 1.0);
        float moist = sampleData.w;
        float forest = smoothstep(0.40, 0.58, moist);
        float treeline = 1500.0 + (moist - 0.5) * 320.0;
        float aboveWater = smoothstep(TERRAIN_WATER + 1.5, TERRAIN_WATER + 3.0, sampleData.x);
        float belowTreeline = 1.0 - smoothstep(treeline - 40.0, treeline + 60.0, sampleData.x);
        float presence_p = (0.06 + forest * 0.80 + smoothstep(0.10, 0.16, slope) * 0.12)
            * aboveWater * belowTreeline;
        // 0 = spruce, 1 = broadleaf; steep slots prefer boulders (material 5).
        float kind = (species < mix(0.30, 0.72, forest)) ? 0.0 : 1.0;
        bool isBoulder = slope > 0.13 && species > 0.40;
        bool present = presence < presence_p;
        float dcam = distance(slotWorld, cameraWorld);
        // Village clearing: the same 3x3 settlement neighborhood the landmark
        // stage draws, tested against the full outbuilding spread, so trees
        // never poke through the castle or the outlying walls. Mirrored by
        // world::scatter_slot for collision parity.
        bool inVillage = false;
        ivec2 stile = ivec2(floor(slotWorld / 16384.0));
        for (int dz = -1; dz <= 1; dz++) {
            for (int dx = -1; dx <= 1; dx++) {
                float baseZ = float(stile.y + dz) * 16384.0 + 3450.0;
                float valley = terrainValleyCenter(mod(baseZ, TERRAIN_PERIOD));
                float baseX = float(stile.x + dx) * 16384.0 + valley + 1180.0;
                inVillage = inVillage
                    || (abs(slotWorld.x - baseX) < 820.0 && abs(slotWorld.y - baseZ) < 1450.0);
            }
        }
        if (!present || inVillage || dcam > 3300.0) {
            vMaterial = 3u;
            vMoisture = 0.0;
            gl_Position = vec4(0.0, 0.0, 2.0, 1.0);
            return;
        }

        // Stylized scale: canopies read at the same visual weight as the
        // chunky landmark buildings from cruise altitude. Per-species
        // silhouette: crown half width, lower crown half height and centre,
        // upper crown half width, half height and centre.
        float size = 1.35 + 0.9 * sizeR;
        float crownW;
        float lowH;
        float lowY;
        float upW;
        float upH;
        float upY;
        if (isBoulder) {
            crownW = 2.0 + 3.4 * sizeR;
            lowH = 1.5 * size;
            lowY = 0.4;
            upW = 1.3 * size;
            upH = 1.0 * size;
            upY = 1.2;
        } else if (kind < 0.5) {
            // Spruce: two stacked cones, tip high, skirt reaching the ground.
            crownW = 3.4 * size;
            lowH = 6.0 * size;
            lowY = 5.0 * size;
            upW = 2.1 * size;
            upH = 6.5 * size;
            upY = 12.5 * size;
        } else {
            // Broadleaf: one broad low crown under a smaller rounded crown.
            // The lower crown sinks below the surface so no trunk gap shows.
            crownW = 7.2 * size;
            lowH = 4.6 * size;
            lowY = 3.4 * size;
            upW = 5.0 * size;
            upH = 3.8 * size;
            upY = 8.6 * size;
        }
        vec3 p;
        if (corner < 24u) {
            p = OCTA[OCTA_TRI[corner]] * vec3(crownW, lowH, crownW);
            p.y += lowY;
        } else {
            p = OCTA[OCTA_TRI[corner - 24u]] * vec3(upW, upH, upW);
            p.y += upY;
        }
        vMaterial = isBoulder ? 5u : 3u;
        // Canopy sway: sub-metre drift keyed to the slot's phase.
        if (!isBoulder && p.y > 1.0) {
            float phase = terrainHash(hcell, 443u) * 6.2831853;
            float sway = (p.y - 1.0) * (p.y - 1.0) * 3.0e-4;
            p.xz += vec2(sin(ubo.flex.y * 1.35 + phase), cos(ubo.flex.y * 1.13 + phase * 0.7)) * sway;
        }
        // The rendered mesh is piecewise-linear per terrain cell: the cell's
        // height and slopes reconstruct the exact ground plane under the
        // slot, so trunks never float without an analytic height evaluation.
        vec2 cellCenter = (vec2(sampleCell) + 0.5) * TERRAIN_CELL_METRES;
        float baseH = sampleData.x
            + sampleData.y * (slotWorld.x - cellCenter.x)
            + sampleData.z * (slotWorld.y - cellCenter.y);
        vObjectPos = p;
        vLandHeight = baseH;
        vTerrainNormal = vec3(0.0, 1.0, 0.0);
        vMoisture = kind + sizeR;
        vPosition = vec3(slotWorld.x - origin.x + p.x,
            max(baseH, TERRAIN_WATER) + ubo.groundBase.w + p.y, slotWorld.y - origin.y + p.z);
    }
    float cam_h = max(ubo.campos.y - ubo.groundBase.w, 0.0);
    float dR = exp(-cam_h / 8000.0);
    float dM = exp(-cam_h / 1200.0);
    float dO = atmoOzoneDensity(cam_h);
    vExtinction = ATMO_BETA_RAYLEIGH * dR + ATMO_BETA_MIE_EXTINCT * dM + ATMO_BETA_OZONE * dO;
    gl_Position = ubo.viewProj * vec4(vPosition, 1.0);
}
