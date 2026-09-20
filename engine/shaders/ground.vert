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
// Landmark metadata for the material shader: vType is the structure index
// (or 39 tree / 40 boulder for scatter), vPart names the decoded submesh
// (0 wall, 1 roof, 2/3 detail A/B, 6 detail C, 7 roofline band, 4 spire,
// 5 sails, or 7 crown / 8 trunk in scatter), and vShape carries the wall and
// roof heights so roof parts can anchor gables, ridges, and eaves.
layout(location = 7) flat out uint vType;
layout(location = 8) flat out uint vPart;
layout(location = 9) flat out vec2 vShape;

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
// First corner of the roof-tip submesh inside one structure's slot.
const uint LANDMARK_TIP_START = STRUCTURE_WALL_VERTICES + STRUCTURE_ROOF_VERTICES
    + 3u * STRUCTURE_DETAIL_VERTICES + STRUCTURE_BAND_VERTICES;

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
        vType = 0u;
        vPart = 0u;
        vShape = vec2(0.0);
    } else if (vertex < TERRAIN_VERTICES + LANDMARK_VERTEX_COUNT) {
        uint landmarkVertex = vertex - TERRAIN_VERTICES;
        uint building = landmarkVertex / STRUCTURE_VERTICES;
        uint tile = building / TERRAIN_STRUCTURES;
        uint structure = building % TERRAIN_STRUCTURES;
        uint corner = landmarkVertex % STRUCTURE_VERTICES;
        vec2 cameraWorld = origin + ubo.campos.xz;
        vec2 tileCell = floor(cameraWorld / 16384.0)
            + vec2(int(tile % 3u) - 1, int(tile / 3u) - 1);
        float baseZ = tileCell.y * 16384.0 + 3450.0;
        vec2 base = vec2(tileCell.x * 16384.0
            + terrainValleyCenter(mod(baseZ, TERRAIN_PERIOD)) + 1180.0, baseZ);
        float wallHeight, roofHeight;
        vec4 shape = terrainStructure(structure, wallHeight, roofHeight);
        vec2 center = base + shape.xy;
        vType = structure;
        vPart = 0u;
        vShape = vec2(wallHeight, roofHeight);
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
        float yBase, addonH;
        vec4 addon;
        if (corner < STRUCTURE_WALL_VERTICES) {
            p = BOX[BOX_TRI[corner]] * vec3(shape.z, wallHeight, shape.w);
            vPart = 0u;
            vMaterial = 1u;
        } else if (corner < STRUCTURE_WALL_VERTICES + STRUCTURE_ROOF_VERTICES) {
            p = ROOF[ROOF_TRI[corner - STRUCTURE_WALL_VERTICES]]
                * vec3(shape.z + 1.0, roofHeight, shape.w + 1.0);
            p.y += wallHeight;
            vPart = 1u;
            vMaterial = 2u;
        } else if (corner < STRUCTURE_WALL_VERTICES + STRUCTURE_ROOF_VERTICES
                + STRUCTURE_DETAIL_VERTICES) {
            addon = terrainDetailA(structure, wallHeight, roofHeight, yBase, addonH);
            p = BOX[BOX_TRI[corner - STRUCTURE_WALL_VERTICES - STRUCTURE_ROOF_VERTICES]]
                * vec3(addon.z, addonH, addon.w);
            p.xy += vec2(addon.x, yBase);
            p.z += addon.y;
            vPart = 2u;
            vMaterial = 1u;
        } else if (corner < STRUCTURE_WALL_VERTICES + STRUCTURE_ROOF_VERTICES
                + 2u * STRUCTURE_DETAIL_VERTICES) {
            addon = terrainDetailB(structure, wallHeight, roofHeight, yBase, addonH);
            p = BOX[BOX_TRI[corner - STRUCTURE_WALL_VERTICES - STRUCTURE_ROOF_VERTICES
                    - STRUCTURE_DETAIL_VERTICES]]
                * vec3(addon.z, addonH, addon.w);
            p.xy += vec2(addon.x, yBase);
            p.z += addon.y;
            vPart = 3u;
            vMaterial = 1u;
        } else if (corner < STRUCTURE_WALL_VERTICES + STRUCTURE_ROOF_VERTICES
                + 3u * STRUCTURE_DETAIL_VERTICES) {
            addon = terrainDetailC(structure, wallHeight, roofHeight, yBase, addonH);
            p = BOX[BOX_TRI[corner - STRUCTURE_WALL_VERTICES - STRUCTURE_ROOF_VERTICES
                    - 2u * STRUCTURE_DETAIL_VERTICES]]
                * vec3(addon.z, addonH, addon.w);
            p.xy += vec2(addon.x, yBase);
            p.z += addon.y;
            vPart = 6u;
            vMaterial = 1u;
        } else if (corner < LANDMARK_TIP_START) {
            addon = terrainDetailD(structure, wallHeight, roofHeight, yBase, addonH);
            p = BOX[BOX_TRI[corner - STRUCTURE_WALL_VERTICES - STRUCTURE_ROOF_VERTICES
                    - 3u * STRUCTURE_DETAIL_VERTICES]]
                * vec3(addon.z, addonH, addon.w);
            p.xy += vec2(addon.x, yBase);
            p.z += addon.y;
            vPart = 7u;
            vMaterial = 1u;
        } else {
            // Roof tip: a spire octahedron on the ridge, or the windmill's
            // sail cross, mirroring world::landmark_structure_triangles()
            // corner-for-corner so the RT mesh matches the raster exactly.
            uint tipCorner = corner - LANDMARK_TIP_START;
            float tipHalfH;
            vec4 tip = terrainTip(structure, wallHeight, roofHeight, tipHalfH);
            if (tip.x < 0.5) {
                p = vec3(0.0);
                vPart = 4u;
            } else if (tip.x < 1.5) {
                p = OCTA[OCTA_TRI[tipCorner]] * vec3(tip.w, tipHalfH, tip.w);
                p.y += tip.z + tipHalfH;
                p.x += tip.y;
                vPart = 4u;
            } else {
                float sailZ = -(shape.w + 4.0);
                uint sailTri = tipCorner / 3u;
                uint sailVert = tipCorner % 3u;
                if (sailVert == 0u) {
                    p = vec3(tip.y, tip.z, sailZ);
                } else {
                    float ang0 = float(sailTri) * 6.283185307179586 / 8.0;
                    float ang = ang0 + (sailVert == 1u ? 0.0 : 6.283185307179586 / 8.0);
                    float len = (sailTri & 1u) == 0u ? tip.w : tip.w * 0.78;
                    p = vec3(tip.y + cos(ang) * len, tip.z + sin(ang) * len, sailZ);
                }
                vPart = 5u;
            }
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
        float forest = smoothstep(0.34, 0.52, moist);
        float treeline = 1500.0 + (moist - 0.5) * 320.0;
        float aboveWater = smoothstep(TERRAIN_WATER + 1.5, TERRAIN_WATER + 3.0, sampleData.x);
        float belowTreeline = 1.0 - smoothstep(treeline - 40.0, treeline + 60.0, sampleData.x);
        float presence_p = (0.06 + forest * 0.92 + smoothstep(0.10, 0.16, slope) * 0.12)
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
            vType = 39u;
            vPart = 0u;
            vShape = vec2(0.0);
            vMaterial = 3u;
            vMoisture = 0.0;
            gl_Position = vec4(0.0, 0.0, 2.0, 1.0);
            return;
        }

        // Stylized scale: canopies read at the same visual weight as the
        // chunky landmark buildings from cruise altitude. Per-species
        // silhouette: a trunk box and three stacked crown octahedra, matching
        // the collision apexes in world::scatter_slot (spruce 26*size,
        // broadleaf 21.2*size, boulder 5.6*size).
        float size = 1.35 + 0.9 * sizeR;
        float trunkW;
        float trunkH;
        float c0w, c0h, c0y;
        float c1w, c1h, c1y;
        float c2w, c2h, c2y;
        if (isBoulder) {
            trunkW = 1.6 + 2.4 * sizeR;
            trunkH = 5.6 * size;
            c0w = c0h = c1w = c1h = c2w = c2h = 0.0;
            c0y = c1y = c2y = 0.0;
        } else if (kind < 0.5) {
            // Spruce: tall trunks and three narrowing cones to a 26*size apex.
            trunkW = 0.55 * size; trunkH = 4.5 * size;
            c0w = 2.9 * size; c0h = 4.2 * size; c0y = 8.6 * size;
            c1w = 2.0 * size; c1h = 4.2 * size; c1y = 14.6 * size;
            c2w = 1.2 * size; c2h = 5.2 * size; c2y = 20.8 * size;
        } else {
            // Broadleaf: one broad low crown under two smaller rounded crowns;
            // the low crown sinks below the surface so no trunk gap shows.
            trunkW = 0.6 * size; trunkH = 3.6 * size;
            c0w = 7.0 * size; c0h = 3.4 * size; c0y = 1.2;
            c1w = 5.0 * size; c1h = 3.8 * size; c1y = 8.2 * size;
            c2w = 3.0 * size; c2h = 3.2 * size; c2y = 18.0 * size;
        }
        vec3 p;
        if (corner < 36u) {
            p = BOX[BOX_TRI[corner]] * vec3(trunkW, trunkH, trunkW);
            vPart = 8u;
        } else if (corner < 60u) {
            p = OCTA[OCTA_TRI[corner - 36u]] * vec3(c0w, c0h, c0w);
            p.y += c0y;
            vPart = 7u;
        } else if (corner < 84u) {
            p = OCTA[OCTA_TRI[corner - 60u]] * vec3(c1w, c1h, c1w);
            p.y += c1y;
            vPart = 7u;
        } else {
            p = OCTA[OCTA_TRI[corner - 84u]] * vec3(c2w, c2h, c2w);
            p.y += c2y;
            vPart = 7u;
        }
        vType = isBoulder ? 40u : 39u;
        vShape = vec2(0.0);
        vMaterial = isBoulder ? 5u : (vPart == 8u ? 4u : 3u);
        // Canopy sway: sub-metre drift keyed to the slot's phase, crowns only.
        if (vPart == 7u && p.y > 1.0) {
            float phase = terrainHash(hcell, 443u) * 6.2831853;
            float sway = (p.y - 1.0) * (p.y - 1.0) * 3.0e-4;
            p.xz += vec2(sin(ubo.flex.y * 1.35 + phase), cos(ubo.flex.y * 1.13 + phase * 0.7)) * sway;
        }
        // The rendered mesh is piecewise-linear per terrain cell with the
        // diagonal from corner (x+1, z) to (x, z+1). The stored height and
        // slopes belong to the lattice CORNER (texel x is the surface at
        // x * TERRAIN_CELL_METRES, not the cell centre), so the containing
        // triangle's three corner heights are interpolated here — the same
        // split world::height_at uses — with each corner water-clamped like
        // the terrain vertices. A single centre-referenced plane used to
        // float hillside trees by up to half a cell times the slope.
        vec2 cellF = slotWorld / TERRAIN_CELL_METRES;
        vec2 cellFloor = floor(cellF);
        vec2 f = cellF - cellFloor;
        ivec2 c00 = ivec2(int(cellFloor.x), int(cellFloor.y));
        float h00 = max(texelFetch(sampler2D(terrain_tex, terrain_smp),
            c00 & ivec2(int(TERRAIN_CELLS - 1u)), 0).x, TERRAIN_WATER);
        float h10 = max(texelFetch(sampler2D(terrain_tex, terrain_smp),
            (c00 + ivec2(1, 0)) & ivec2(int(TERRAIN_CELLS - 1u)), 0).x, TERRAIN_WATER);
        float h01 = max(texelFetch(sampler2D(terrain_tex, terrain_smp),
            (c00 + ivec2(0, 1)) & ivec2(int(TERRAIN_CELLS - 1u)), 0).x, TERRAIN_WATER);
        float h11 = max(texelFetch(sampler2D(terrain_tex, terrain_smp),
            (c00 + ivec2(1, 1)) & ivec2(int(TERRAIN_CELLS - 1u)), 0).x, TERRAIN_WATER);
        float baseH = (f.x + f.y <= 1.0)
            ? h00 * (1.0 - f.x - f.y) + h10 * f.x + h01 * f.y
            : h11 * (f.x + f.y - 1.0) + h10 * (1.0 - f.y) + h01 * (1.0 - f.x);
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
