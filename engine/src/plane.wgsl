// Physically based sky, atmosphere, and merged airframe shader.
// Implements Sébastien Hillaire (EGSR 2020) / Alexander Wilkie (SIGGRAPH 2021)
// atmospheric scattering model with Pierce solar limb darkening,
// circumsolar Mie aureole, ozone Chappuis absorption, and Cook-Torrance GGX PBR.

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
    /// Alignment padding to 1728 bytes (offset 1712..1728).
    pad: vec4<f32>,
};

@group(0) @binding(0) var<uniform> ubo: UBO;
@group(0) @binding(1) var weave_tex: texture_2d<f32>;
@group(0) @binding(2) var weave_smp: sampler;

const PI: f32 = 3.141592653589793;

/// Full physical atmospheric sky dome radiance combining Rayleigh in-scattering,
/// horizon distance haze, ground terrain reflectance, circumsolar Mie aureole, and Pierce solar limb darkening.
fn physical_atmosphere_sky(view_dir: vec3<f32>, sun_dir: vec3<f32>, sun_irr: vec3<f32>) -> vec3<f32> {
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
    if (cos_gamma > 0.4) {
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
    @location(2) uv_mat: vec3<f32>,
};

/// Decode an octahedron-projected normal from signed 16-bit integers to a unit vec3.
fn oct_decode(pair: vec2<i32>) -> vec3<f32> {
    let x = f32(pair.x) / 32767.0;
    let y = f32(pair.y) / 32767.0;
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

    // Dynamic aeroelastic wing flex: quadratic parabolic deflection combined with gust oscillations.
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

/// Material albedo color (RGB) and metallic factor (Alpha).
/// IDs: 0 = Sail, 1 = Composite, 2 = Graphite, 3 = Titanium, 4 = Dark, 5 = Seat, 6 = Glass, 7 = Glow.
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
        return vec4(0.38, 0.43, 0.47, 0.04);
    }
    return vec4(0.80, 0.83, 1.0, 0.2);
}

/// Material emissive color (RGB) and baseline roughness factor (Alpha).
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
    let sun_dir = normalize(ubo.sunDir.xyz);
    let sun_irr = ubo.sunColor.rgb;

    let mat_id = u32(round(in.uv_mat.z));
    let albedo = material_albedo(mat_id);
    let emissive = material_emissive(mat_id);
    let metal = albedo.a;

    // Geometric specular AA: normal variance widens roughness to eliminate specular aliasing:
    let dnx = dpdx(n);
    let dny = dpdy(n);
    let variance = max(dot(dnx, dnx), dot(dny, dny));
    let rough = clamp(
        sqrt(emissive.a * emissive.a + clamp(2.0 * variance, 0.0, 0.35)),
        0.04,
        1.0,
    );

    // Sail cloth weave texture with anisotropic mips
    let wgrad_x = dpdx(in.uv_mat.xy);
    let wgrad_y = dpdy(in.uv_mat.xy);
    var tint = albedo.rgb;
    if (mat_id == 0u) {
        let cloth = textureSampleGrad(weave_tex, weave_smp, in.uv_mat.xy, wgrad_x, wgrad_y).rgb;
        let fw = abs(wgrad_x.x) + abs(wgrad_x.y) + abs(wgrad_y.x) + abs(wgrad_y.y);
        let calm = clamp(1.0 - fw * 0.35, 0.0, 1.0);
        tint = albedo.rgb * mix(vec3(1.0), cloth / vec3(0.70, 0.67, 0.57), calm);
    }

    let view_dir = normalize(ubo.campos.xyz - in.world);
    let h = normalize(sun_dir + view_dir);
    let n_dot_l = max(dot(n, sun_dir), 0.0);
    let n_dot_v = max(dot(n, view_dir), 0.001);
    let n_dot_h = max(dot(n, h), 0.0);
    let v_dot_h = max(dot(view_dir, h), 0.0);

    // Microfacet Cook-Torrance GGX Specular BRDF
    let alpha_rough = rough * rough;
    let alpha_sq = alpha_rough * alpha_rough;
    let d_denom = (n_dot_h * n_dot_h * (alpha_sq - 1.0) + 1.0);
    let d_ggx = alpha_sq / (PI * d_denom * d_denom + 1e-7);

    // Fresnel-Schlick
    let f0 = mix(vec3<f32>(0.04), tint, metal);
    let f_schlick = f0 + (vec3<f32>(1.0) - f0) * pow(clamp(1.0 - v_dot_h, 0.0, 1.0), 5.0);

    // Schlick-Smith Geometric Visibility
    let k = (rough + 1.0) * (rough + 1.0) / 8.0;
    let g_v = n_dot_v / (n_dot_v * (1.0 - k) + k);
    let g_l = n_dot_l / (n_dot_l * (1.0 - k) + k);
    let g_smith = g_v * g_l;

    let spec_brdf = (d_ggx * f_schlick * g_smith) / (4.0 * n_dot_v * n_dot_l + 1e-4);
    let kd = (vec3<f32>(1.0) - f_schlick) * (1.0 - metal);
    let diff_brdf = kd * (tint / PI);

    // Direct physical sun illumination
    let direct_sun = (diff_brdf + spec_brdf) * sun_irr * n_dot_l * PI;

    // Hemispherical atmospheric sky ambient using precomputed values
    let sky_zenith = ubo.skyZenith.rgb;
    let ground_refl = ubo.groundBase.rgb;
    let amb_weight = n.y * 0.5 + 0.5;
    let sky_irradiance = mix(ground_refl, sky_zenith, amb_weight) * 0.65;
    let ambient = tint * sky_irradiance * (1.0 - metal) + f0 * sky_irradiance * (1.0 - rough);

    let linear_color = direct_sun + ambient + emissive.rgb * ubo.flex.w;
    let tone_color = aces_tonemap(linear_color);

    var alpha = 1.0;
    if (mat_id == 6u) {
        // Physical Fresnel transparency for canopy glass
        let fresnel_glass = 0.04 + 0.96 * pow(clamp(1.0 - n_dot_v, 0.0, 1.0), 4.0);
        alpha = mix(0.35, 0.92, fresnel_glass);
    }
    return vec4(tone_color, alpha);
}

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

    let hdr_sky = physical_atmosphere_sky(view_dir, sun_dir, sun_irr);
    let ldr_sky = aces_tonemap(hdr_sky);
    return vec4<f32>(ldr_sky, 1.0);
}
