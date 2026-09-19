#version 450

// Fullscreen sky triangle. Ray unprojected in the fragment stage.

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

layout(location = 0) out vec3 vRay;

void main() {
    vec2 pos = vec2(-1.0, -1.0);
    if (gl_VertexIndex == 1 || gl_VertexIndex == 4) {
        pos = vec2(1.0, -1.0);
    } else if (gl_VertexIndex == 2 || gl_VertexIndex == 3) {
        pos = vec2(-1.0, 1.0);
    } else if (gl_VertexIndex == 5) {
        pos = vec2(1.0, 1.0);
    }
    // Keep the background in front of the cleared depth (1.0) but behind every
    // finite scene fragment. Using exact depth 1.0 with LESS_OR_EQUAL allowed
    // far terrain vertices to be overwritten by the sky pass.
    gl_Position = vec4(pos.x, pos.y, 0.999999, 1.0);
    vec4 world_far = ubo.invViewProj * vec4(pos.x, pos.y, 1.0, 1.0);
    vRay = world_far.xyz / world_far.w - ubo.campos.xyz;
}
