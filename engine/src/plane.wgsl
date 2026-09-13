// One uber-shader for the whole airframe. Opaque and glass share
// it; only blend and depth-write state differ between pipelines.

struct UBO {
    mvp: mat4x4<f32>,
    model: mat4x4<f32>,
    albedo: vec4<f32>,
    emissive: vec4<f32>,
    flex: vec4<f32>,
    campos: vec4<f32>,
};

@group(0) @binding(0) var<uniform> ubo: UBO;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) oct: vec2<u32>,
    @location(2) uv: vec2<f32>,
    @location(3) flex: f32,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) world: vec3<f32>,
    @location(2) uv_weave: vec3<f32>,
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
    var out: VsOut;
    let world4 = ubo.model * vec4(p, 1.0);
    out.clip = ubo.mvp * vec4(p, 1.0);
    out.normal = normalize((ubo.model * vec4(n, 0.0)).xyz);
    out.world = world4.xyz;
    out.uv_weave = vec3(in.uv, ubo.flex.w);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let sun_dir = normalize(vec3(-0.42, 0.78, 0.46));
    let sun_color = vec3(1.15, 1.1, 1.02);
    let diff = max(dot(n, sun_dir), 0.0);
    var albedo = ubo.albedo.rgb;
    let metal = ubo.albedo.a;
    let rough = clamp(ubo.emissive.a, 0.05, 1.0);
    let weave = fract(in.uv_weave.z * 0.5) * 2.0;
    if (weave >= 1.0) {
        let gx = step(fract(in.uv_weave.x * 16.0), 0.25);
        let gy = step(fract(in.uv_weave.y * 16.0), 0.25);
        albedo = albedo * (1.0 - 0.06 * max(gx, gy));
    }
    let glass = weave >= 2.0;
    let view_dir = normalize(ubo.campos.xyz - in.world);
    let h = normalize(sun_dir + view_dir);
    let spec = pow(max(dot(n, h), 0.0), mix(8.0, 160.0, 1.0 - rough))
        * mix(0.04, 1.0, metal);
    let ambient = vec3(0.38, 0.44, 0.52);
    var color = albedo * (ambient + sun_color * diff)
        + sun_color * spec
        + ubo.emissive.rgb;
    var alpha = 1.0;
    if (glass) {
        alpha = 0.72;
    }
    return vec4(color, alpha);
}
