#version 450

// Airframe vertex stage. Same 28-byte stream as plane.rs VERTEX_BYTES:
// pos12 + oct4 + uvHalf4 + flex4 + ids4. Node matrices from the UBO.

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
layout(location = 1) in ivec2 oct;
layout(location = 2) in vec2 uv;
layout(location = 3) in float flexW;
layout(location = 4) in uvec2 ids;

layout(location = 0) out vec3 vNormal;
layout(location = 1) out vec3 vWorld;
layout(location = 2) out vec2 vUv;
layout(location = 3) flat out uint vMaterial;

vec3 octDecode(ivec2 pair) {
    float x = float(pair.x) / 32767.0;
    float y = float(pair.y) / 32767.0;
    float z = 1.0 - abs(x) - abs(y);
    float nx = x;
    float ny = y;
    if (z < 0.0) {
        nx = (1.0 - abs(y)) * (x >= 0.0 ? 1.0 : -1.0);
        ny = (1.0 - abs(x)) * (y >= 0.0 ? 1.0 : -1.0);
    }
    return normalize(vec3(nx, ny, z));
}

void main() {
    vec3 p = pos;
    vec3 n = octDecode(oct);
    float span = abs(flexW);
    float side = sign(flexW);

    // Dynamic aeroelastic wing flex, same terms as the WGSL original.
    if (span > 0.0) {
        float bend = ubo.flex.x;
        float t = ubo.flex.y;
        float pressure = ubo.flex.z;
        float gust = sin(t * 5.1 - span * 3.0 + side) * 0.65
            + sin(t * 8.3 - span * 5.0) * 0.35;
        p.y = p.y + bend * span * span
            + pressure * 0.022 * span * span * span * gust;

        float slope = 2.0 * bend * span;
        n.x = n.x - slope * side * n.y;
        n = normalize(n);
    }

    mat4 model = ubo.nodes[ids.x];
    vec4 world4 = model * vec4(p, 1.0);
    gl_Position = ubo.viewProj * world4;
    vNormal = normalize((model * vec4(n, 0.0)).xyz);
    vWorld = world4.xyz;
    vUv = uv;
    vMaterial = ids.y;
}
