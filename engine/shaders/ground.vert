#version 450

// Screen-space ground envelope. The fragment shader still performs the exact
// ray/plane hit and owns the material; this vertex stage only avoids invoking
// that expensive material on the sky side of the horizon.

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

layout(location = 0) out vec3 vRay;

vec3 rayAt(vec2 ndc) {
    vec4 world_far = ubo.invViewProj * vec4(ndc, 1.0, 1.0);
    return world_far.xyz / world_far.w - ubo.campos.xyz;
}

float horizonAt(float x) {
    vec3 bottom = rayAt(vec2(x, -1.0));
    vec3 top = rayAt(vec2(x, 1.0));
    float delta = top.y - bottom.y;
    if (abs(delta) <= 1e-5) {
        return bottom.y < 0.0 ? 1.0 : -1.0;
    }
    return mix(-1.0, 1.0, clamp(-bottom.y / delta, 0.0, 1.0));
}

void main() {
    float left_horizon = horizonAt(-1.0);
    float right_horizon = horizonAt(1.0);
    float ground_sign = ubo.groundBase.w - ubo.campos.y;
    ground_sign = abs(ground_sign) > 1e-5 ? sign(ground_sign) : -1.0;
    bool bottom_is_ground = rayAt(vec2(-1.0, -1.0)).y * ground_sign > 0.0;

    vec2 p;
    if (bottom_is_ground) {
        if (gl_VertexIndex == 0) p = vec2(-1.0, -1.0);
        else if (gl_VertexIndex == 1) p = vec2(1.0, -1.0);
        else if (gl_VertexIndex == 2 || gl_VertexIndex == 3) p = vec2(-1.0, left_horizon);
        else if (gl_VertexIndex == 4) p = vec2(1.0, -1.0);
        else p = vec2(1.0, right_horizon);
    } else {
        if (gl_VertexIndex == 0) p = vec2(-1.0, left_horizon);
        else if (gl_VertexIndex == 1) p = vec2(1.0, right_horizon);
        else if (gl_VertexIndex == 2 || gl_VertexIndex == 3) p = vec2(-1.0, 1.0);
        else if (gl_VertexIndex == 4) p = vec2(1.0, right_horizon);
        else p = vec2(1.0, 1.0);
    }

    vRay = rayAt(p);
    gl_Position = vec4(p, 0.0, 1.0);
}
