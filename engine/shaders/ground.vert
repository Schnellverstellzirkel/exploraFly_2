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
// (or 50 tree / 51 boulder for vegetation), vPart names the decoded submesh
// (0 wall, 1 roof, 2/3 detail A/B, 6 detail C, 7 roofline band, 4 spire,
// 5 sails, or 7 crown / 8 trunk in vegetation), and vShape carries the wall and
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
        // The terrain index stream contains no procedural scatter vertices.
        // Vegetation is a separate instanced draw; retain a defensive collapse
        // for malformed command buffers rather than leaving varyings undefined.
        vPosition = vec3(0.0);
        vLandHeight = 0.0;
        vMaterial = 0u;
        vObjectPos = vec3(0.0);
        vTerrainNormal = vec3(0.0, 1.0, 0.0);
        vMoisture = 0.0;
        vType = 0u;
        vPart = 0u;
        vShape = vec2(0.0);
        gl_Position = vec4(0.0, 0.0, 2.0, 1.0);
        return;
    }
    float cam_h = max(ubo.campos.y - ubo.groundBase.w, 0.0);
    float dR = exp(-cam_h / 8000.0);
    float dM = exp(-cam_h / 1200.0);
    float dO = atmoOzoneDensity(cam_h);
    vExtinction = ATMO_BETA_RAYLEIGH * dR + ATMO_BETA_MIE_EXTINCT * dM + ATMO_BETA_OZONE * dO;
    gl_Position = ubo.viewProj * vec4(vPosition, 1.0);
}
