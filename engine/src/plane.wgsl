// Merged airframe shader. One draw for opaque, one for glass.
// Per-vertex node and material ids index the node array and a
// branchless-ish material table. Parts of one material sit
// contiguous in the index buffer, so warps stay coherent.

struct UBO {
    viewProj: mat4x4<f32>,
    nodes: array<mat4x4<f32>, 23>,
    flex: vec4<f32>,
    campos: vec4<f32>,
};

@group(0) @binding(0) var<uniform> ubo: UBO;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) oct: vec2<u32>,
    @location(2) uv: vec2<f32>,
    @location(3) flex: f32,
    @location(4) ids: vec2<u32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) world: vec3<f32>,
    @location(2) uv_mat: vec3<f32>,
};

fn oct_decode(pair: vec2<u32>) -> vec3<f32> {
    let x = f32(bitcast<i32>(pair.x)) / 32767.0;
    let y = f32(bitcast<i32>(pair.y)) / 32767.0;
    var z = 1.0 - abs(x) - abs(y);
    var nx = x;
    var ny = y;
    if (z < 0.0) {
        nx = (1.0 - abs(y)) * sign(x);
        ny = (1.0 - abs(x)) * sign(y);
        z = 1.0 - abs(nx) - abs(ny);
    }
    return normalize(vec3(nx, ny, z));
}

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var p = in.pos;
    var n = oct_decode(in.oct);
    let span = abs(in.flex);
    let side = sign(in.flex);
    if (span > 0.0) {
        let bend = ubo.flex.x;
        let t = ubo.flex.y;
        let pressure = ubo.flex.z;
        let gust = sin(t * 5.1 - span * 3.0 + side) * 0.65
            + sin(t * 8.3 - span * 5.0) * 0.35;
        p.y = p.y + bend * span * span
            + pressure * 0.022 * span * span * span * gust;
        let ahead = bend * (span + 0.01) * (span + 0.01);
        let slope = (ahead - bend * span * span) / 0.01;
        n.x = n.x - slope * side * n.y;
        n = normalize(n);
    }
    let model = ubo.nodes[in.ids.x];
    var out: VsOut;
    let world4 = model * vec4(p, 1.0);
    out.clip = ubo.viewProj * world4;
    out.normal = normalize((model * vec4(n, 0.0)).xyz);
    out.world = world4.xyz;
    out.uv_mat = vec3(in.uv, f32(in.ids.y));
    return out;
}


fn material_albedo(id: u32) -> vec4<f32> {
    if (id == 0u) {
        return vec4(0.59, 0.59, 0.55, 0.04);
    }
    if (id == 1u) {
        return vec4(0.47, 0.48, 0.48, 0.12);
    }
    if (id == 2u) {
        return vec4(0.15, 0.17, 0.19, 0.25);
    }
    if (id == 3u) {
        return vec4(0.55, 0.58, 0.59, 0.88);
    }
    if (id == 4u) {
        return vec4(0.09, 0.11, 0.13, 0.82);
    }
    if (id == 5u) {
        return vec4(0.40, 0.28, 0.22, 0.0);
    }
    if (id == 6u) {
        return vec4(0.38, 0.43, 0.47, 0.1);
    }
    return vec4(0.80, 0.83, 1.0, 0.2);
}

fn material_emissive(id: u32) -> vec4<f32> {
    if (id == 7u) {
        return vec4(0.52, 0.61, 1.0, 0.15);
    }
    if (id == 0u) {
        return vec4(0.0, 0.0, 0.0, 0.65);
    }
    if (id == 1u) {
        return vec4(0.0, 0.0, 0.0, 0.34);
    }
    if (id == 2u) {
        return vec4(0.0, 0.0, 0.0, 0.34);
    }
    if (id == 3u) {
        return vec4(0.0, 0.0, 0.0, 0.24);
    }
    if (id == 4u) {
        return vec4(0.0, 0.0, 0.0, 0.32);
    }
    if (id == 5u) {
        return vec4(0.0, 0.0, 0.0, 0.85);
    }
    return vec4(0.0, 0.0, 0.0, 0.06);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let sun_dir = normalize(vec3(-0.42, 0.78, 0.46));
    let sun_color = vec3(1.15, 1.1, 1.02);
    let diff = max(dot(n, sun_dir), 0.0);
    let mat_id = u32(in.uv_mat.z);
    let albedo = material_albedo(mat_id);
    let emissive = material_emissive(mat_id);
    let metal = albedo.a;
    let rough = clamp(emissive.a, 0.05, 1.0);
    let weave_on = select(0.0, 1.0, mat_id == 0u);
    let grid = max(
        step(fract(in.uv_mat.x * 16.0), 0.25),
        step(fract(in.uv_mat.y * 16.0), 0.25));
    let tint = albedo.rgb * (1.0 - 0.06 * grid * weave_on);
    let view_dir = normalize(ubo.campos.xyz - in.world);
    let h = normalize(sun_dir + view_dir);
    let spec = pow(max(dot(n, h), 0.0), mix(8.0, 160.0, 1.0 - rough))
        * mix(0.04, 1.0, metal);
    let ambient = vec3(0.38, 0.44, 0.52);
    var color = tint * (ambient + sun_color * diff)
        + sun_color * spec
        + emissive.rgb * ubo.flex.w;
    var alpha = 1.0;
    if (mat_id == 6u) {
        alpha = 0.72;
    }
    return vec4(color, alpha);
}
