// Persistent vortex ribbon trails. Camera-facing strips built on CPU from
// TrailPool segments. Per-vertex age, density, radius, flow UV, seed.
// Shading: double HG forward peak for ice, Cornette-Shanks side lobe,
// flow-map UV noise lookup so detail sticks to fluid (VPFM idea),
// Crow sine displacement baked on CPU, erosion by age.

struct UBO {
    viewProj: mat4x4<f32>,
    invViewProj: mat4x4<f32>,
    nodes: array<mat4x4<f32>, 23>,
    flex: vec4<f32>,
    campos: vec4<f32>,
    sunDir: vec4<f32>,
    sunColor: vec4<f32>,
    skyZenith: vec4<f32>,
    skyHorizon: vec4<f32>,
    groundBase: vec4<f32>,
    detail: vec4<f32>,
};

@group(0) @binding(0) var<uniform> ubo: UBO;
@group(1) @binding(0) var base_vol: texture_3d<f32>;
@group(1) @binding(1) var base_smp: sampler;

struct VsIn {
    @location(0) center: vec3<f32>,
    @location(1) side: vec3<f32>,
    @location(2) age: f32,
    @location(3) density: f32,
    @location(4) flow_uv: vec2<f32>,
    @location(5) seed: f32,
    @location(6) radius: f32,
    @location(7) ice: f32,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) age: f32,
    @location(2) density: f32,
    @location(3) world: vec3<f32>,
    @location(4) seed: f32,
    @location(5) ice: f32,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    // side arrives pre-scaled by radius from the CPU ribbon builder.
    let world = in.center + in.side;
    out.clip = ubo.viewProj * vec4(world, 1.0);
    out.uv = in.flow_uv;
    out.age = in.age;
    out.density = in.density;
    out.world = world;
    out.seed = in.seed;
    out.ice = in.ice;
    return out;
}

fn phase_hg(mu: f32, g: f32) -> f32 {
    let gg = g * g;
    return (1.0 - gg) / (12.566371 * pow(max(1.0 + gg - 2.0 * g * mu, 1e-4), 1.5));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let view = normalize(ubo.campos.xyz - in.world);
    let mu = dot(view, ubo.sunDir.xyz);
    // Flow-map lookup: UV drifts with fluid age on CPU, noise sticks.
    let n = textureSample(base_vol, base_smp, vec3(fract(in.uv * 0.35), fract(in.seed + in.age * 0.004))).rgb;
    // Erode edges with age: young cores solid, old trails fibrous then milky.
    // uv.y drifts with the flow map, so the across-strip coordinate is fract.
    let across = fract(in.uv.y);
    let edge = 1.0 - abs(across * 2.0 - 1.0);
    var a = in.density * smoothstep(0.15, 0.75, edge + (n.g - 0.5) * 0.55 * (0.4 + min(in.age * 0.08, 1.2)));
    a *= exp(-in.age * 0.012) * smoothstep(0.0, 0.25, in.age);
    if (a < 0.004) {
        discard;
    }
    // Double HG: strong forward plus weak back. Ice fraction sharpens peak.
    let g_fwd = mix(0.55, 0.78, clamp(in.ice, 0.0, 1.0));
    let phase = 0.85 * phase_hg(mu, g_fwd) + 0.15 * phase_hg(mu, -0.25);
    let sun = ubo.sunColor.rgb * phase * 2.2;
    let amb = mix(ubo.skyHorizon.rgb, ubo.skyZenith.rgb, 0.45) * (0.55 + 0.45 * n.b);
    // Standard alpha blend over HDR linear target.
    // Powder-sugar rim darken from Nubis: edges cooler than core.
    let rim = mix(0.72, 1.0, smoothstep(0.0, 0.6, edge));    let color = (sun + amb) * rim * (0.75 + 0.5 * in.ice);
    return vec4(color, a);
}
