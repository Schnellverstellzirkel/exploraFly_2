#version 450

// Plume cone proxy vertex stage. World-space verts rewritten per frame.

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

layout(location = 0) in vec3 pos;
layout(location = 1) in float axial;
layout(location = 2) in float radial;

layout(location = 0) out vec3 vWorld;
layout(location = 1) out float vAxial;
layout(location = 2) out float vRadial;
layout(location = 3) out vec3 vNozzle;

void main() {
    gl_Position = ubo.viewProj * vec4(pos, 1.0);
    vWorld = pos;
    vAxial = axial;
    vRadial = radial;
    // Lip center from the ten petal transforms. Per-vertex (78 verts), not
    // per-pixel: identical value across the proxy, flat-shaded by constancy.
    vec3 nozzle = vec3(0.0);
    for (int i = 11; i <= 20; ++i)
        nozzle += (ubo.nodes[i] * vec4(0.0, 0.0, -0.63, 1.0)).xyz * 0.1;
    vNozzle = nozzle;
}
