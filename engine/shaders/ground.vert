#version 450

// Screen-space ground envelope. The fragment shader still performs the exact
// ray/plane hit and owns the material; this vertex stage avoids invoking
// that expensive material when screen regions are purely sky.

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

// In Vulkan NDC, y = -1.0 is the top of the viewport and y = +1.0 is the bottom.
float horizonAt(float x) {
    vec3 ray_top = rayAt(vec2(x, -1.0));
    vec3 ray_bottom = rayAt(vec2(x, 1.0));
    float delta = ray_bottom.y - ray_top.y;
    if (abs(delta) <= 1e-5) {
        return 0.0;
    }
    float t = clamp(-ray_top.y / delta, 0.0, 1.0);
    return mix(-1.0, 1.0, t);
}

void main() {
    float ground_sign = ubo.groundBase.w - ubo.campos.y;
    ground_sign = abs(ground_sign) > 1e-5 ? sign(ground_sign) : -1.0;

    // Test each corner to see if its unprojected ray points toward the ground.
    // Vulkan NDC: top-left is (-1, -1), bottom-right is (1, 1).
    bool tl_ground = rayAt(vec2(-1.0, -1.0)).y * ground_sign > 0.0;
    bool tr_ground = rayAt(vec2( 1.0, -1.0)).y * ground_sign > 0.0;
    bool bl_ground = rayAt(vec2(-1.0,  1.0)).y * ground_sign > 0.0;
    bool br_ground = rayAt(vec2( 1.0,  1.0)).y * ground_sign > 0.0;

    vec2 p;
    if (!tl_ground && !tr_ground && !bl_ground && !br_ground) {
        // Entire viewport is sky: cull the draw call by collapsing to zero area.
        p = vec2(-1.0, -1.0);
    } else if (!tl_ground && !tr_ground && bl_ground && br_ground) {
        // Normal flight: sky at top, ground at bottom. Horizon cuts left and right edges.
        // Add a small safety margin (-0.01) towards sky to guarantee no ground pixels are clipped.
        float left_horizon = max(-1.0, horizonAt(-1.0) - 0.01);
        float right_horizon = max(-1.0, horizonAt(1.0) - 0.01);
        if (gl_VertexIndex == 0) p = vec2(-1.0, left_horizon);
        else if (gl_VertexIndex == 1) p = vec2(1.0, right_horizon);
        else if (gl_VertexIndex == 2 || gl_VertexIndex == 3) p = vec2(-1.0, 1.0);
        else if (gl_VertexIndex == 4) p = vec2(1.0, right_horizon);
        else p = vec2(1.0, 1.0);
    } else if (tl_ground && tr_ground && !bl_ground && !br_ground) {
        // Inverted flight: ground at top, sky at bottom. Horizon cuts left and right edges.
        // Add a small safety margin (+0.01) towards sky to guarantee no ground pixels are clipped.
        float left_horizon = min(1.0, horizonAt(-1.0) + 0.01);
        float right_horizon = min(1.0, horizonAt(1.0) + 0.01);
        if (gl_VertexIndex == 0) p = vec2(-1.0, -1.0);
        else if (gl_VertexIndex == 1) p = vec2(1.0, -1.0);
        else if (gl_VertexIndex == 2 || gl_VertexIndex == 3) p = vec2(-1.0, left_horizon);
        else if (gl_VertexIndex == 4) p = vec2(1.0, -1.0);
        else p = vec2(1.0, right_horizon);
    } else {
        // Pitching down / diving (all 4 corners ground), or banking / complex horizon:
        // Fullscreen quad safely covers all ground fragments; ground.frag discards sky hits.
        if (gl_VertexIndex == 0) p = vec2(-1.0, -1.0);
        else if (gl_VertexIndex == 1) p = vec2(1.0, -1.0);
        else if (gl_VertexIndex == 2 || gl_VertexIndex == 3) p = vec2(-1.0, 1.0);
        else if (gl_VertexIndex == 4) p = vec2(1.0, -1.0);
        else p = vec2(1.0, 1.0);
    }

    vRay = rayAt(p);
    gl_Position = vec4(p, 1.0, 1.0);
}
