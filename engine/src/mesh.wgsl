struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) color: vec3<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec3<f32>,
};

@group(0) @binding(0) var<uniform> mvp: mat4x4<f32>;

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    out.clip = mvp * vec4(in.pos, 1.0);
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(@location(0) color: vec3<f32>) -> @location(0) vec4<f32> {
    return vec4(color, 1.0);
}
