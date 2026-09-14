#version 450

// Final composite vertex stage. Fullscreen triangle from vertex index.

layout(location = 0) out vec2 vUv;

void main() {
    vec2 pos = vec2(-1.0, -1.0);
    if (gl_VertexIndex == 1 || gl_VertexIndex == 4) {
        pos = vec2(1.0, -1.0);
    } else if (gl_VertexIndex == 2 || gl_VertexIndex == 3) {
        pos = vec2(-1.0, 1.0);
    } else if (gl_VertexIndex == 5) {
        pos = vec2(1.0, 1.0);
    }
    gl_Position = vec4(pos, 0.0, 1.0);
    vUv = pos * 0.5 + 0.5;
}
