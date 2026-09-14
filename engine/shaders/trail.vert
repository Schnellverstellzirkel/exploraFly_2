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

void main() {
    vec3 world = center + side;
    gl_Position = ubo.viewProj * vec4(world, 1.0);
    vUv = flowUv;
    vAge = age;
    vDensity = density;
    vWorld = world;
    vSeed = seed;
    vIce = ice;
}
