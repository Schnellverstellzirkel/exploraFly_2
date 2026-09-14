// Material-specific GGX, resin layers, cloth sheen, and analytic sky lighting.
// The sky is an artistic analytic approximation, not an implementation of
// Hillaire/Wilkie volumetric scattering. See docs/rendering/material-realism.md.

/// Global uniform buffer layout bound at group 0, binding 0 (1728 bytes total).
struct UBO {
    /// Combined view * projection matrix in floating-origin coordinates (offset 0..64).
    viewProj: mat4x4<f32>,
    /// Inverse view * projection matrix for camera ray unprojection (offset 64..128).
    invViewProj: mat4x4<f32>,
    /// Pre-multiplied model matrices for each airframe kinematic node 0..22 (offset 128..1600).
    nodes: array<mat4x4<f32>, 23>,
    /// Aeroelastic & time parameters: (bend, time, dynamic_pressure, glow_intensity) (offset 1600..1616).
    flex: vec4<f32>,
    /// Camera position in world space relative to floating origin (offset 1616..1632).
    campos: vec4<f32>,
    /// Unit direction toward Sun (xyz) and solar angular radius (w, radians) (offset 1632..1648).
    sunDir: vec4<f32>,
    /// Precomputed physical solar irradiance (rgb) and elevation angle (w) (offset 1648..1664).
    sunColor: vec4<f32>,
    /// Precomputed sky zenith color (rgb) and cos(sun_radius) (w) (offset 1664..1680).
    skyZenith: vec4<f32>,
    /// Precomputed sky horizon haze (rgb) and inv_one_minus_cos_radius (w) (offset 1680..1696).
    skyHorizon: vec4<f32>,
    /// Precomputed ground base terrain color (rgb) (offset 1696..1712).
    groundBase: vec4<f32>,
    /// Shading detail level: 0.0 = presented pass (full PBR + analytic IBL
    /// reflections), 1.0 = intermediate never-presented pass (direct sun +
    /// flat ambient only). Branch is draw-uniform, so no warp divergence
    /// (offset 1712..1728).
    detail: vec4<f32>,
};

@group(0) @binding(0) var<uniform> ubo: UBO;
@group(0) @binding(1) var weave_tex: texture_2d<f32>;
@group(0) @binding(2) var weave_smp: sampler;
@group(0) @binding(3) var eir_tex: texture_2d<f32>;
@group(0) @binding(4) var eir_smp: sampler;

const PI: f32 = 3.141592653589793;

/// Full physical atmospheric sky dome radiance combining Rayleigh in-scattering,
/// horizon distance haze, ground terrain reflectance, circumsolar Mie aureole, and Pierce solar limb darkening.
/// `with_sun` gates the solar disk and aureole: IBL irradiance taps disable them
/// so the direct-light sun term is never double-counted in ambient lighting.
fn physical_atmosphere_sky(view_dir: vec3<f32>, sun_dir: vec3<f32>, sun_irr: vec3<f32>, with_sun: bool) -> vec3<f32> {
    let cos_gamma = dot(view_dir, sun_dir);
    let y = view_dir.y;

    let zenith_sky = ubo.skyZenith.rgb;
    let horizon_haze = ubo.skyHorizon.rgb;

    // Atmospheric Rayleigh scattering gradient across the sky dome:
    // Fast polynomial gradient matching Rayleigh profile without SFU transcendental stalls
    let u = clamp(1.0 - max(y, 0.0), 0.0, 1.0);
    let u2 = u * u;
    let sky_factor = u2 * u * (0.85 * u + 0.15);
    var sky = mix(zenith_sky, horizon_haze, sky_factor);

    // Below horizon: ground terrain with distance atmospheric haze
    if (y < 0.0) {
        let ground_base = ubo.groundBase.rgb;
        let h = clamp(1.0 + y * 3.5, 0.0, 1.0);
        let haze = h * h;
        sky = mix(ground_base, horizon_haze * 0.88, haze);
    }

    // Circumsolar HDR zone (Mie forward aureole and Pierce limb-darkened solar disk):
    if (with_sun && cos_gamma > 0.4) {
        let p = cos_gamma;
        let p2 = p * p;
        let p4 = p2 * p2;
        let p8 = p4 * p4;
        let p12 = p8 * p4;
        let p16_val = p8 * p8;
        let p64_val = p16_val * p16_val;
        let p80_val = p64_val * p16_val;
        let aureole = p12 * 0.40 + p80_val * 1.6;
        sky += sun_irr * (aureole * 0.45);

        let cos_radius = ubo.skyZenith.w;
        if (cos_gamma >= cos_radius - 0.0001) {
            let inv_rad = ubo.skyHorizon.w;
            let rho2 = clamp((1.0 - cos_gamma) * inv_rad, 0.0, 1.0);
            let mu = sqrt(max(1.0 - rho2, 0.0));
            let u_coeff = vec3<f32>(0.54, 0.63, 0.72);
            let v_coeff = vec3<f32>(0.18, 0.16, 0.14);
            let one_minus_mu = 1.0 - mu;
            let limb = vec3<f32>(1.0) - u_coeff * one_minus_mu - v_coeff * (one_minus_mu * one_minus_mu);
            let edge_aa = smoothstep(cos_radius - 0.00005, cos_radius + 0.00005, cos_gamma);
            sky += limb * (42.0 * edge_aa * (sun_irr * 0.3125));
        }
    }

    return sky;
}

/// ACES filmic high dynamic range tonemapper.
fn aces_tonemap(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3<f32>(0.0), vec3<f32>(1.0));
}

// --- Airframe Vertex / Fragment Pass ---

/// Interleaved vertex attributes (28 bytes per vertex).
struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) oct: vec2<i32>,
    @location(2) uv: vec2<f32>,
    @location(3) flex: f32,
    @location(4) ids: vec2<u32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) world: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) material: u32,
};

/// Decode an octahedron-projected normal from signed 16-bit integers to a unit vec3.
fn oct_decode(pair: vec2<i32>) -> vec3<f32> {
    let x = f32(pair.x) / 32767.0;
    let y = f32(pair.y) / 32767.0;
    let z = 1.0 - abs(x) - abs(y);
    var nx = x;
    var ny = y;
    if (z < 0.0) {
        nx = (1.0 - abs(y)) * select(-1.0, 1.0, x >= 0.0);
        ny = (1.0 - abs(x)) * select(-1.0, 1.0, y >= 0.0);
    }
    return normalize(vec3(nx, ny, z));
}

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var p = in.pos;
    var n = oct_decode(in.oct);
    let span = abs(in.flex);
    let side = sign(in.flex);

    // Dynamic aeroelastic wing flex: quadratic parabolic deflection combined with gust oscillations.
    if (span > 0.0) {
        let bend = ubo.flex.x;
        let t = ubo.flex.y;
        let pressure = ubo.flex.z;
        let gust = sin(t * 5.1 - span * 3.0 + side) * 0.65
            + sin(t * 8.3 - span * 5.0) * 0.35;
        p.y = p.y + bend * span * span
            + pressure * 0.022 * span * span * span * gust;

        let slope = 2.0 * bend * span;
        n.x = n.x - slope * side * n.y;
        n = normalize(n);
    }

    let model = ubo.nodes[in.ids.x];
    var out: VsOut;
    let world4 = model * vec4(p, 1.0);
    out.clip = ubo.viewProj * world4;
    out.normal = normalize((model * vec4(n, 0.0)).xyz);
    out.world = world4.xyz;
    out.uv = in.uv;
    out.material = in.ids.y;
    return out;
}

// Models and limitations: docs/rendering/material-realism.md.
// Colors are linear reflectances, not display sRGB values. These are authored
// material priors, not measurements of a manufactured aircraft.
struct Material {
    color: vec3<f32>,
    roughness: f32,
    f0: vec3<f32>,
    metal: f32,
    anisotropy: f32,
    coat: f32,
    coat_roughness: f32,
    sheen: f32,
};

fn material(id: u32) -> Material {
    switch id {
        case 0u: { // Woven polyester sail: dielectric fibers, broad grazing sheen.
            return Material(vec3(0.59, 0.57, 0.51), 0.78, vec3(0.035), 0.0, 0.20, 0.0, 0.0, 0.25);
        }
        case 1u: { // Painted composite / gelcoat: diffuse substrate under clear resin.
            return Material(vec3(0.47, 0.48, 0.46), 0.44, vec3(0.004), 0.0, 0.0, 1.0, 0.24, 0.0);
        }
        case 2u: { // Carbon laminate: dark directional substrate and resin interface.
            return Material(vec3(0.022, 0.027, 0.032), 0.48, vec3(0.025), 0.0, 0.60, 1.0, 0.29, 0.0);
        }
        case 3u: { // Bare satin titanium: conductor, no diffuse lobe.
            return Material(vec3(0.0), 0.30, vec3(0.55, 0.58, 0.61), 1.0, 0.72, 0.0, 0.0, 0.0);
        }
        case 4u: { // Matte black coating: not exposed metal.
            return Material(vec3(0.014, 0.019, 0.024), 0.76, vec3(0.04), 0.0, 0.0, 0.0, 0.0, 0.0);
        }
        case 5u: { // Leather cushion: rough dielectric with weak fiber sheen.
            return Material(vec3(0.19, 0.085, 0.043), 0.83, vec3(0.035), 0.0, 0.0, 0.0, 0.0, 0.10);
        }
        case 6u: { // Canopy dielectric, air/interface IOR approximately 1.5.
            return Material(vec3(0.0), 0.075, vec3(0.04), 0.0, 0.0, 0.0, 0.0, 0.0);
        }
        default: {
            return Material(vec3(0.12, 0.16, 0.24), 0.38, vec3(0.04), 0.0, 0.0, 0.0, 0.0, 0.0);
        }
    }
}

fn pow5(x: f32) -> f32 {
    let x2 = x * x;
    return x2 * x2 * x;
}

fn fresnel(f0: vec3<f32>, cosine: f32) -> vec3<f32> {
    return f0 + (vec3(1.0) - f0) * pow5(1.0 - clamp(cosine, 0.0, 1.0));
}

// Height-correlated Smith GGX, same visibility in direct lighting and VNDF IBL.
fn ggx_visibility(v: vec3<f32>, l: vec3<f32>, a: vec2<f32>) -> f32 {
    let gv = l.z * length(vec3(a * v.xy, v.z));
    let gl = v.z * length(vec3(a * l.xy, l.z));
    return 0.5 / max(gv + gl, 1e-6);
}

fn ggx_direct(v: vec3<f32>, l: vec3<f32>, a: vec2<f32>, f0: vec3<f32>) -> vec3<f32> {
    let h = normalize(v + l);
    let q = vec3(h.xy / a, h.z);
    let q2 = dot(q, q);
    let d = 1.0 / max(PI * a.x * a.y * q2 * q2, 1e-8);
    return fresnel(f0, dot(v, h)) * (d * ggx_visibility(v, l, a) * max(l.z, 0.0));
}

// Deterministic Hammersley azimuths, generated offline. No per-tap sin/cos,
// random noise, temporal history, extra descriptors, or frame allocations.
const ENV_SAMPLES: u32 = 8u;
const ENV_POINTS = array<vec3<f32>, 8>(
    vec3(1.000000000, 0.000000000, 0.062500000),
    vec3(-1.000000000, 0.000000000, 0.187500000),
    vec3(0.000000000, 1.000000000, 0.312500000),
    vec3(-0.000000000, -1.000000000, 0.437500000),
    vec3(0.707106781, 0.707106781, 0.562500000),
    vec3(-0.707106781, -0.707106781, 0.687500000),
    vec3(-0.707106781, 0.707106781, 0.812500000),
    vec3(0.707106781, -0.707106781, 0.937500000)
);

// Dupuy & Benyoub 2023 spherical-cap visible GGX sampling. Sample half vectors
// in stretched view space, then reflect; sampling around the mirror ray is not GGX.
fn environment_specular(frame: mat3x3<f32>, v: vec3<f32>, a: vec2<f32>, f0: vec3<f32>) -> vec3<f32> {
    let stretched = normalize(vec3(a * v.xy, v.z));
    let gv_root = length(vec3(a * v.xy, v.z));
    var sum = vec3(0.0);
    for (var i = 0u; i < ENV_SAMPLES; i += 1u) {
        let u = ENV_POINTS[i];
        let z = 1.0 - u.z * (1.0 + stretched.z);
        let r = sqrt(max(1.0 - z * z, 0.0));
        let cap = vec3(u.xy * r, z);
        let hh = cap + stretched;
        let h = normalize(vec3(a * hh.xy, max(hh.z, 0.0)));
        let l = reflect(-v, h);
        if (l.z > 0.0) {
            // f*cos / pdf_VNDF = F * G2/G1; no arbitrary normalized blur.
            let gl_root = length(vec3(a * l.xy, l.z));
            let weight = l.z * (v.z + gv_root) / max(l.z * gv_root + v.z * gl_root, 1e-6);
            let radiance = physical_atmosphere_sky(frame * l, ubo.sunDir.xyz, ubo.sunColor.rgb, false);
            sum += radiance * fresnel(f0, dot(v, h)) * weight;
        }
    }
    return sum / f32(ENV_SAMPLES);
}

// Cosine-weighted hemisphere quadrature for diffuse sky irradiance / PI.
fn environment_diffuse(frame: mat3x3<f32>) -> vec3<f32> {
    var sum = vec3(0.0);
    for (var i = 0u; i < 8u; i += 1u) {
        let angle = f32(i) * (2.0 * PI / 8.0);
        let azimuth = vec2(cos(angle), sin(angle));
        let r2 = (f32(i) + 0.5) / 8.0;
        let l = vec3(azimuth * sqrt(r2), sqrt(1.0 - r2));
        sum += physical_atmosphere_sky(frame * l, ubo.sunDir.xyz, ubo.sunColor.rgb, false);
    }
    return sum * 0.125;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let id = in.material;
    var m = material(id);
    let view = normalize(ubo.campos.xyz - in.world);
    // Geometry is two-sided. Face the shading frame toward the visible side.
    let ng = normalize(in.normal);
    var n = select(-ng, ng, dot(ng, view) >= 0.0);
    let dx = dpdx(in.world);
    let dy = dpdy(in.world);
    let ux = dpdx(in.uv);
    let uy = dpdy(in.uv);
    let det = ux.x * uy.y - ux.y * uy.x;
    let tangent_raw = (dx * uy.y - dy * ux.y) * select(-1.0, 1.0, det >= 0.0);
    let tangent_projected = tangent_raw - n * dot(n, tangent_raw);
    let helper = select(vec3(0.0, 1.0, 0.0), vec3(1.0, 0.0, 0.0), abs(n.y) > 0.95);
    var t = normalize(cross(helper, n));
    if (dot(tangent_projected, tangent_projected) > 1e-12) {
        t = normalize(tangent_projected);
    }
    var b = cross(n, t);
    // Continuous, footprint-filtered microstructure. Subpixel weave vanishes
    // into the roughness lobe instead of producing camera-dependent glitter.
    let frequency = select(28.0, 64.0, id == 3u);
    let footprint = max(length(ux), length(uy)) * frequency;
    let resolved = 1.0 - smoothstep(0.15, 0.65, footprint);
    let phase = in.uv * (2.0 * PI * frequency);
    let wave = sin(phase);
    if (id == 0u) {
        let cloth = textureSampleGrad(weave_tex, weave_smp, in.uv, ux, uy).rgb;
        m.color *= cloth / vec3(0.70, 0.67, 0.57);
        n = normalize(n + (t * wave.x + b * wave.y) * (0.045 * resolved));
    } else if (id == 2u) {
        let twill = wave.x * wave.y * resolved;
        m.color *= 1.0 + 0.18 * twill;
        m.roughness += 0.035 * twill;
        n = normalize(n + (t * wave.x - b * wave.y) * (0.022 * resolved));
    } else if (id == 3u) {
        m.roughness += 0.025 * wave.y * resolved;
        n = normalize(n + b * (0.012 * wave.y * resolved));
    } else if (id == 1u || id == 4u || id == 5u) {
        n = normalize(n + (t * wave.x + b * wave.y) * (0.009 * resolved));
    }
    t = normalize(t - n * dot(n, t));
    b = cross(n, t);
    let frame = mat3x3(t, b, n);
    let v = vec3(dot(view, t), dot(view, b), max(dot(view, n), 0.001));
    let sun = ubo.sunDir.xyz;
    let l = vec3(dot(sun, t), dot(sun, b), dot(sun, n));
    // Filter in microfacet slope space, not perceptual roughness space.
    let dnx = dpdx(n);
    let dny = dpdy(n);
    let variance = min(0.5 * (dot(dnx, dnx) + dot(dny, dny)), 0.18);
    let alpha = sqrt(m.roughness * m.roughness * m.roughness * m.roughness + variance);
    let aspect = sqrt(1.0 - 0.9 * m.anisotropy);
    let a = max(vec2(alpha / aspect, alpha * aspect), vec2(0.004));
    // Isotropic directional-albedo LUT approximates compensation for anisotropic
    // lobes using their geometric-mean slope. See the report's accuracy limits.
    let ess = textureSampleLevel(eir_tex, eir_smp, vec2(v.z, alpha), 0.0).r;
    let gain = vec3(1.0) + m.f0 * (1.0 / max(ess, 0.05) - 1.0);
    let fv = fresnel(m.f0, v.z);
    let fl = fresnel(m.f0, max(l.z, 0.0));
    let irradiance = environment_diffuse(frame);
    let diffuse_albedo = m.color * (1.0 - m.metal) * (vec3(1.0) - fv);
    var ambient = diffuse_albedo * irradiance;
    var direct = diffuse_albedo * ubo.sunColor.rgb * max(l.z, 0.0) * (vec3(1.0) - fl);
    ambient += environment_specular(frame, v, a, m.f0) * gain;
    if (l.z > 0.0) {
        direct += ggx_direct(v, l, a, m.f0) * gain * ubo.sunColor.rgb * PI;
    }
    if (m.sheen > 0.0) {
        // Charlie fiber distribution (Estevez & Kulla 2017); broad velvet-like
        // grazing response. Ambient sheen is a low-frequency approximation.
        let h = normalize(v + l);
        let inv_r = 1.0 / max(m.roughness, 0.1);
        let d = (2.0 + inv_r) * pow(max(1.0 - h.z * h.z, 0.0), 0.5 * inv_r) / (2.0 * PI);
        let visibility = 1.0 / max(4.0 * (max(l.z, 0.0) + v.z - max(l.z, 0.0) * v.z), 1e-4);
        let sheen_color = sqrt(m.color) * m.sheen;
        ambient = ambient * (1.0 - m.sheen * 0.25)
            + sheen_color * irradiance * pow5(1.0 - v.z);
        direct = direct * (1.0 - m.sheen * 0.25)
            + sheen_color * ubo.sunColor.rgb * PI * d * visibility * max(l.z, 0.0);
    }
    var color = ambient + direct;
    if (m.coat > 0.0) {
        // Attenuate incoming and outgoing paths through the clearcoat separately.
        // The integrated environment term uses the view angle for both paths;
        // only the explicit sun path has its true incident direction here.
        let fc_view = fresnel(vec3(0.04), v.z).x;
        let fc_light = fresnel(vec3(0.04), max(l.z, 0.0)).x;
        let coat_alpha = max(sqrt(pow(m.coat_roughness, 4.0) + variance), 0.004);
        ambient *= (1.0 - m.coat * fc_view) * (1.0 - m.coat * fc_view);
        direct *= (1.0 - m.coat * fc_view) * (1.0 - m.coat * fc_light);
        color = ambient + direct;
        color += m.coat * environment_specular(frame, v, vec2(coat_alpha), vec3(0.04));
        if (l.z > 0.0) {
            color += m.coat * ggx_direct(v, l, vec2(coat_alpha), vec3(0.04)) * ubo.sunColor.rgb * PI;
        }
    }
    if (id == 7u) {
        color += vec3(0.52, 0.61, 1.0) * ubo.flex.w;
    }
    if (id == 6u) {
        // Premultiplied reflection + scalar transmission. No diffuse glass.
        // Linear HDR output: the final composite pass tone maps once, so
        // glass blends over linear plume and trail light without double mapping.
        let opacity = clamp(fv.x + 0.025 * (1.0 - fv.x), 0.0, 1.0);
        return vec4(color, opacity);
    }
    return vec4(color, 1.0);
}

// Geometry-only benchmark pass, never presented.
@fragment
fn fs_depth(_in: VsOut) { }

// --- Procedural Fullscreen Sky Dome & Solar Photosphere Pass ---

struct VsSkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ray: vec3<f32>,
};

@vertex
fn vs_sky(@builtin(vertex_index) vid: u32) -> VsSkyOut {
    var out: VsSkyOut;
    // Exact 6-vertex two-triangle quad pinned to far plane within screen bounds:
    // Tri 1: (-1, -1), (1, -1), (-1, 1)
    // Tri 2: (-1, 1), (1, -1), (1, 1)
    var pos = vec2<f32>(-1.0, -1.0);
    if (vid == 1u || vid == 4u) {
        pos = vec2<f32>(1.0, -1.0);
    } else if (vid == 2u || vid == 3u) {
        pos = vec2<f32>(-1.0, 1.0);
    } else if (vid == 5u) {
        pos = vec2<f32>(1.0, 1.0);
    }
    out.clip = vec4<f32>(pos.x, pos.y, 1.0, 1.0);
    let world_far = ubo.invViewProj * vec4<f32>(pos.x, pos.y, 1.0, 1.0);
    out.ray = world_far.xyz / world_far.w - ubo.campos.xyz;
    return out;
}

@fragment
fn fs_sky(in: VsSkyOut) -> @location(0) vec4<f32> {
    let view_dir = normalize(in.ray);
    let sun_dir = ubo.sunDir.xyz;
    let sun_irr = ubo.sunColor.rgb;

    let hdr_sky = physical_atmosphere_sky(view_dir, sun_dir, sun_irr, true);
    return vec4<f32>(hdr_sky, 1.0);
}
