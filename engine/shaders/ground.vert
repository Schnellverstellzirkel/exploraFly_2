#version 450

// Fixed topology, camera-centered exponential lattice. It has no shared LOD
// boundaries or T junctions: a single rasterized surface replaces the old plane.
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

layout(location = 0) out vec3 vPosition;
layout(location = 1) out float vLandHeight;
layout(location = 2) flat out uint vMaterial;
layout(location = 3) out vec3 vObjectPos;

const ivec2 QUAD[6] = ivec2[6](ivec2(0, 0), ivec2(0, 1), ivec2(1, 0),
    ivec2(1, 0), ivec2(0, 1), ivec2(1, 1));
const vec3 BOX[8] = vec3[8](vec3(-1, 0, -1), vec3(1, 0, -1),
    vec3(-1, 1, -1), vec3(1, 1, -1), vec3(-1, 0, 1), vec3(1, 0, 1),
    vec3(-1, 1, 1), vec3(1, 1, 1));
const int BOX_TRI[36] = int[36](0, 2, 1, 1, 2, 3, 5, 7, 4, 4, 7, 6,
    4, 6, 0, 0, 6, 2, 1, 3, 5, 5, 3, 7, 2, 6, 3, 3, 6, 7, 4, 0, 5, 5, 0, 1);
const vec3 ROOF[6] = vec3[6](vec3(-1, 0, -1), vec3(1, 0, -1),
    vec3(-1, 0, 1), vec3(1, 0, 1), vec3(0, 1, -1), vec3(0, 1, 1));
const int ROOF_TRI[18] = int[18](0, 2, 5, 0, 5, 4, 1, 4, 5, 1, 5, 3, 0, 4, 1, 2, 3, 5);

void main() {
    uint vertex = uint(gl_VertexIndex);
    vec2 origin = terrainOrigin(ubo.groundOrigin);
    vObjectPos = vec3(0.0);
    if (vertex < TERRAIN_VERTICES) {
        uint cell = vertex / 6u;
        ivec2 grid = ivec2(int(cell % TERRAIN_CELLS), int(cell / TERRAIN_CELLS))
            + QUAD[vertex % 6u] - ivec2(32);
        vec2 g = vec2(grid);
        // Centre edge spacing 8.32 m, outside radius 51.98 km. Smooth movement
        // avoids snapped LOD transitions; heights stay tied to absolute world.
        vec2 xz = ubo.campos.xz + sign(g) * 32.0 * (exp2(abs(g) / 3.0) - 1.0);
        float ground = terrainHeight(origin + xz);
        vPosition = vec3(xz.x, max(ground, TERRAIN_WATER) + ubo.groundBase.w, xz.y);
        vLandHeight = ground;
        vMaterial = 0u;
    } else {
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
        vPosition = vec3(center.x - origin.x + p.x,
            foundation + ubo.groundBase.w + p.y, center.y - origin.y + p.z);
    }
    gl_Position = ubo.viewProj * vec4(vPosition, 1.0);
}
