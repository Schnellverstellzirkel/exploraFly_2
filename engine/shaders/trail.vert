#version 450

// Trail ribbon vertex stage. Side arrives pre-scaled by radius from fill_trail.

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
    // Packed-origin minus current origin, refreshed every present in update().
    // Ribbon centers stay packed against the last sim step; this translates
    // them on GPU, so inter-step presents cost O(1) CPU with exact positions.
    vec4 trailShift;
} ubo;

layout(location = 0) in vec3 center;
layout(location = 1) in vec3 side;
layout(location = 2) in float age;
layout(location = 3) in float density;
layout(location = 4) in vec2 flowUv;
layout(location = 5) in float seed;
layout(location = 6) in float radius;
layout(location = 7) in float ice;

layout(location = 0) out vec2 vUv;
layout(location = 1) out float vAge;
layout(location = 2) out float vDensity;
layout(location = 3) out vec3 vWorld;
layout(location = 4) out float vSeed;
layout(location = 5) out float vIce;
layout(location = 6) out float vAcross;

void main() {
    vec3 world = center + side + ubo.trailShift.xyz;
    gl_Position = ubo.viewProj * vec4(world, 1.0);
    vUv = flowUv;
    vAge = age;
    vDensity = density;
    vWorld = world;
    vSeed = seed;
    vIce = ice;
    vAcross = (gl_VertexIndex & 1) == 0 ? -1.0 : 1.0;
}
