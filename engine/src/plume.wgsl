// Exhaust plume raymarch. Cone proxy from nozzle lip downstream.
// Volume transport: residual ratio tracking approx with fixed majorant steps.
// Density: Nubis remap of base Perlin-Worley plus detail erosion plus curl warp.
// Shock cells: Prandtl 1904 lambda passed via groundBase.w, emission bands via
// axial cosine falloff. Scattering: HG plus Cornette-Shanks. Emission: dual
// temperature blackbody approx plus chemiluminescence in first cells.
// Output is standard-alpha (radiance, alpha) into HDR linear; heat shimmer
// distortion arrives as a later composite step, not a feedback sample here.

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
@group(1) @binding(2) var detail_vol: texture_3d<f32>;
@group(1) @binding(3) var detail_smp: sampler;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) axial: f32,
    @location(2) radial: f32,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) axial: f32,
    @location(2) radial: f32,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    // Cone proxy verts are already in world space on CPU (nozzle frame baked).
    out.clip = ubo.viewProj * vec4(in.pos, 1.0);
    out.world = in.pos;
    out.axial = in.axial;
    out.radial = in.radial;
    return out;
}

fn remap(x: f32, a: f32, b: f32, c: f32, d: f32) -> f32 {
    return c + (d - c) * clamp((x - a) / max(b - a, 1e-5), 0.0, 1.0);
}

fn phase_hg(mu: f32, g: f32) -> f32 {
    let gg = g * g;
    return (1.0 - gg) / (12.566371 * pow(max(1.0 + gg - 2.0 * g * mu, 1e-4), 1.5));
}

fn phase_cs(mu: f32, g: f32) -> f32 {
    let gg = g * g;
    let p1 = 1.5 * (1.0 - gg) / (2.0 + gg);
    let p2 = (1.0 + mu * mu) / pow(max(1.0 + gg - 2.0 * g * mu, 1e-4), 1.5);
    return p1 * p2 / 12.566371;
}

// Blackbody approx 800..2600 K mapped to linear RGB. Fit to Planckian locus.
fn blackbody(t: f32) -> vec3<f32> {
    let u = clamp((t - 800.0) / 1800.0, 0.0, 1.0);
    let r = 0.25 + 0.75 * smoothstep(0.0, 0.45, u);
    let g = 0.12 + 0.62 * smoothstep(0.15, 0.7, u);
    let b = 0.55 * smoothstep(0.0, 0.25, u) + 0.45 * smoothstep(0.5, 1.0, u);
    return vec3(r, g, b) * (0.4 + 2.6 * u * u);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let spool = ubo.detail.z;
    let lambda = max(ubo.groundBase.w, 0.25);
    let time = ubo.flex.y;
    let sun_dir = ubo.sunDir.xyz;
    let view = normalize(ubo.campos.xyz - in.world);
    let mu_sun = dot(view, sun_dir);

    // March along view ray through cone slab. Proxy gives entry; fixed
    // depth approximates exit. 10 steps matches CPU majorant budget.
    let march_len = mix(2.0, 9.0, clamp(in.axial, 0.0, 1.0)) * (0.6 + 0.4 * spool);
    var trans = 1.0;
    var radiance = vec3(0.0);
    let steps = 10u;
    // Flicker from spool plus noise jitter, 23/57/91 Hz mix on CPU mirrored here.
    let flick = 0.85 + 0.15 * (sin(time * 57.0 + in.axial * 9.0) * 0.5 + sin(time * 91.0) * 0.3 + sin(time * 23.0 + in.radial * 5.0) * 0.2);
    for (var s = 0u; s < steps; s += 1u) {
        let t = (f32(s) + 0.5) / f32(steps);
        // Sample point pushed downstream with time advection.
        let adv = t * march_len;
        let p = in.world - view * (t - 0.5) * march_len * 0.35;
        let uvw = vec3(in.axial - time * 0.35 - adv * 0.06, in.radial, t);
        let base = textureSampleLevel(base_vol, base_smp, fract(uvw), 0.0);
        let det = textureSampleLevel(detail_vol, detail_smp, fract(uvw * 2.3 + 0.17), 0.0);
        // Nubis remap: base inflated, detail erodes edges.
        let coverage = clamp(0.35 + spool * 0.5 - in.axial * 0.55 - in.radial * 0.45, 0.0, 1.0);
        var dens = base.r * (1.0 - coverage) + (base.g * 0.6 + base.b * 0.4) * coverage;
        dens = remap(dens, det.r * 0.55, 1.0, 0.0, 1.0);
        dens *= (1.0 - in.axial * 0.75) * (1.0 - in.radial * in.radial);
        if (dens < 0.004) {
            continue;
        }
        // Shock cells: axial bands from Prandtl spacing.
        let x_m = in.axial * mix(3.0, 13.0, spool);
        let band = 0.5 + 0.5 * cos(6.2831853 * x_m / max(lambda, 0.2));
        let cell = pow(band, 3.0) * exp(-x_m * 0.28) * step(0.05, lambda - 0.01);
        let temp = mix(900.0, 800.0 + spool * 1300.0, exp(-x_m * 0.22)) + cell * 700.0 * spool;
        let emit = blackbody(temp) * (dens * (1.2 + cell * 3.2 * spool) * flick);
        // Chemiluminescence blue in first two cells at high spool.
        let chem = vec3(0.35, 0.5, 1.0) * cell * exp(-x_m * 0.9) * spool * dens * 2.0;
        // Sun scattering through 6-tap style single march approx.
        let shadow = exp(-dens * 2.2 * (0.5 + 0.5 * in.axial));
        let phase = 0.7 * phase_hg(mu_sun, 0.65) + 0.3 * phase_cs(mu_sun, 0.55);
        let scatter = (ubo.sunColor.rgb * phase * shadow + ubo.skyHorizon.rgb * 0.25) * dens;
        // Ratio-tracking style weight with fixed majorant.
        let mu_t = dens * 3.0 + 0.02;
        let mu_bar = 3.2;
        let a = 1.0 - exp(-mu_t * march_len / f32(steps));
        radiance += trans * (emit + chem + scatter * 0.6);
        trans *= 1.0 - a * (mu_t / mu_bar);
        if (trans < 0.02) {
            break;
        }
    }
    // Standard alpha blend over HDR linear target: src*alpha + dst*(1-alpha).
    let alpha = clamp(1.0 - trans, 0.0, 1.0);
    return vec4(radiance, alpha);
}
