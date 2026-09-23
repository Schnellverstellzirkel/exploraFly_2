#version 460

// Airframe fragment stage. Linear HDR output, tone mapped once in composite.
// NOTE: build.rs prepends a generated header defining ENV_SAMPLES and
// ENV_POINTS (Hammersley azimuths) before compiling. Do not define them here.
// Expedition livery is evaluated in mesh UVs without new textures/bindings.
// GGX/Charlie/clearcoat remain established approximations, not a full OpenPBR
// implementation. Research dates and deliberate omissions are in
// docs/rendering/aircraft-art-and-fx.md.

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
    vec4 originShift;
} ubo;

layout(set = 0, binding = 1) uniform texture2D weave_tex;
layout(set = 0, binding = 2) uniform sampler weave_smp;
layout(set = 0, binding = 3) uniform texture2D eir_tex;
layout(set = 0, binding = 4) uniform sampler eir_smp;

#ifdef ENABLE_RT
layout(set = 0, binding = 5) uniform accelerationStructureEXT scene_tlas;

bool rtOccluded(vec3 origin, vec3 dir, float t_max, uint cull_mask) {
    rayQueryEXT q;
    rayQueryInitializeEXT(q, scene_tlas, gl_RayFlagsOpaqueEXT | gl_RayFlagsTerminateOnFirstHitEXT,
        cull_mask, origin, 1e-4, dir, t_max);
    rayQueryProceedEXT(q);
    return rayQueryGetIntersectionTypeEXT(q, true) != gl_RayQueryCommittedIntersectionNoneEXT;
}

// Soft sun visibility via single ray query trace of the sun disc.
// First hits (including the center ray) gate the disc samples, per Laine
// et al. 2005: hard umbra/penumbra split is resolved by the center ray, and
// penumbra fragments cost the full loop.
float rtSunVisibility(vec3 origin, float t_max, uint cull_mask) {
    vec3 sun = normalize(ubo.sunDir.xyz);
    float sun_radius = ubo.sunDir.w;
    
    // Sample 0: Center ray towards solar core
    if (!rtOccluded(origin, sun, t_max, cull_mask)) {
        vec3 up = (abs(sun.y) < 0.99) ? vec3(0.0, 1.0, 0.0) : vec3(1.0, 0.0, 0.0);
        vec3 s_t = normalize(cross(up, sun));
        vec3 s_b = cross(sun, s_t);
        float rot = fract(sin(dot(gl_FragCoord.xy, vec2(12.9898, 78.233))) * 43758.5453) * 6.2831853;
        float c_r = cos(rot);
        float s_r = sin(rot);
        vec2 p1 = SUN_POINTS[1];
        vec2 pr1 = vec2(p1.x * c_r - p1.y * s_r, p1.x * s_r + p1.y * c_r);
        vec3 dir1 = normalize(sun + (s_t * pr1.x + s_b * pr1.y) * sun_radius);
        if (!rtOccluded(origin, dir1, t_max, cull_mask)) {
            return 1.0; // Fully lit penumbra-free early exit
        }
        float lit = 1.0;
        for (uint i = 2u; i < SHADOW_RAYS; i += 1u) {
            vec2 p = SUN_POINTS[i];
            vec2 pr = vec2(p.x * c_r - p.y * s_r, p.x * s_r + p.y * c_r);
            vec3 dir = normalize(sun + (s_t * pr.x + s_b * pr.y) * sun_radius);
            if (!rtOccluded(origin, dir, t_max, cull_mask)) {
                lit += 1.0;
            }
        }
        return lit / float(SHADOW_RAYS);
    } else {
        vec3 up = (abs(sun.y) < 0.99) ? vec3(0.0, 1.0, 0.0) : vec3(1.0, 0.0, 0.0);
        vec3 s_t = normalize(cross(up, sun));
        vec3 s_b = cross(sun, s_t);
        float rot = fract(sin(dot(gl_FragCoord.xy, vec2(12.9898, 78.233))) * 43758.5453) * 6.2831853;
        float c_r = cos(rot);
        float s_r = sin(rot);
        vec2 p1 = SUN_POINTS[1];
        vec2 pr1 = vec2(p1.x * c_r - p1.y * s_r, p1.x * s_r + p1.y * c_r);
        vec3 dir1 = normalize(sun + (s_t * pr1.x + s_b * pr1.y) * sun_radius);
        if (rtOccluded(origin, dir1, t_max, cull_mask)) {
            return 0.0; // Deep umbra early exit
        }
        float lit = 0.0;
        for (uint i = 2u; i < SHADOW_RAYS; i += 1u) {
            vec2 p = SUN_POINTS[i];
            vec2 pr = vec2(p.x * c_r - p.y * s_r, p.x * s_r + p.y * c_r);
            vec3 dir = normalize(sun + (s_t * pr.x + s_b * pr.y) * sun_radius);
            if (!rtOccluded(origin, dir, t_max, cull_mask)) {
                lit += 1.0;
            }
        }
        return lit / float(SHADOW_RAYS);
    }
}
#endif

layout(location = 0) in vec3 vNormal;
layout(location = 1) in vec3 vWorld;
layout(location = 2) in vec2 vUv;
layout(location = 3) flat in uint vMaterial;
layout(location = 4) flat in uint vNode;

layout(location = 0) out vec4 outColor;

const float PI = 3.141592653589793;

vec3 physicalAtmosphereSky(vec3 view_dir, vec3 sun_dir, vec3 sun_irr, bool with_sun) {
    float cos_gamma = dot(view_dir, sun_dir);
    float y = view_dir.y;

    vec3 zenith_sky = ubo.skyZenith.rgb;
    vec3 horizon_haze = ubo.skyHorizon.rgb;

    float u = clamp(1.0 - max(y, 0.0), 0.0, 1.0);
    float u2 = u * u;
    float sky_factor = u2 * u * (0.85 * u + 0.15);
    vec3 sky = mix(zenith_sky, horizon_haze, sky_factor);

    if (y < 0.0) {
        vec3 ground_base = ubo.groundBase.rgb;
        float h = clamp(1.0 + y * 3.5, 0.0, 1.0);
        float haze = h * h;
        sky = mix(ground_base, horizon_haze * 0.88, haze);
    }

    if (with_sun && cos_gamma > 0.4) {
        float p = cos_gamma;
        float p2 = p * p;
        float p4 = p2 * p2;
        float p8 = p4 * p4;
        float p12 = p8 * p4;
        float p16_val = p8 * p8;
        float p64_val = p16_val * p16_val;
        float p80_val = p64_val * p16_val;
        float aureole = p12 * 0.40 + p80_val * 1.6;
        sky += sun_irr * (aureole * 0.45);

        float cos_radius = ubo.skyZenith.w;
        if (cos_gamma >= cos_radius - 0.0001) {
            float inv_rad = ubo.skyHorizon.w;
            float rho2 = clamp((1.0 - cos_gamma) * inv_rad, 0.0, 1.0);
            float mu = sqrt(max(1.0 - rho2, 0.0));
            vec3 u_coeff = vec3(0.54, 0.63, 0.72);
            vec3 v_coeff = vec3(0.18, 0.16, 0.14);
            float one_minus_mu = 1.0 - mu;
            vec3 limb = vec3(1.0) - u_coeff * one_minus_mu - v_coeff * (one_minus_mu * one_minus_mu);
            float edge_aa = smoothstep(cos_radius - 0.00005, cos_radius + 0.00005, cos_gamma);
            sky += limb * (42.0 * edge_aa * (sun_irr * 0.3125));
        }
    }

    return sky;
}

struct Material {
    vec3 color;
    float roughness;
    vec3 f0;
    float metal;
    float anisotropy;
    float coat;
    float coat_roughness;
    float sheen;
};

Material getMaterial(uint id) {
    if (id == 0u) {
        return Material(vec3(0.76, 0.63, 0.43), 0.79, vec3(0.035), 0.0, 0.20, 0.0, 0.0, 0.24);
    } else if (id == 1u) {
        return Material(vec3(0.025, 0.19, 0.17), 0.42, vec3(0.04), 0.0, 0.0, 0.62, 0.23, 0.0);
    } else if (id == 2u) {
        return Material(vec3(0.018, 0.045, 0.042), 0.47, vec3(0.04), 0.0, 0.45, 0.36, 0.29, 0.0);
    } else if (id == 3u) {
        // Hot nozzle/rotor keep a cool steel response. Structural fittings use
        // a broad warm brass highlight, preserving the existing material ID.
        vec3 conductor = (vNode >= 10u && vNode <= 20u)
            ? vec3(0.55, 0.58, 0.61) : vec3(0.78, 0.56, 0.25);
        return Material(vec3(0.0), 0.34, conductor, 1.0, 0.52, 0.0, 0.0, 0.0);
    } else if (id == 4u) {
        return Material(vec3(0.014, 0.019, 0.024), 0.76, vec3(0.04), 0.0, 0.0, 0.0, 0.0, 0.0);
    } else if (id == 5u) {
        return Material(vec3(0.28, 0.10, 0.043), 0.83, vec3(0.035), 0.0, 0.0, 0.0, 0.0, 0.10);
    } else if (id == 6u) {
        return Material(vec3(0.0), 0.075, vec3(0.04), 0.0, 0.0, 0.0, 0.0, 0.0);
    } else {
        return Material(vec3(0.035, 0.16, 0.14), 0.38, vec3(0.04), 0.0, 0.0, 0.0, 0.0, 0.0);
    }
}

float band(float p, float lo, float hi, float aa) {
    return smoothstep(lo - aa, lo + aa, p) * (1.0 - smoothstep(hi - aa, hi + aa, p));
}

void expeditionCanvas(inout Material m, vec2 uv, vec2 footprint) {
    vec2 aa = max(footprint, vec2(0.0001));
    vec3 ink = vec3(0.024, 0.16, 0.145);
    // Three broad hand-painted stripes read from chase distance. The thin
    // seam lines fade once unresolved instead of turning into moire.
    float stripe = band(uv.x, 7.56, 8.05, aa.x)
        + band(uv.x, 8.20, 8.32, aa.x) + band(uv.x, 8.48, 8.60, aa.x);
    m.color = mix(m.color, ink, clamp(stripe, 0.0, 1.0) * 0.92);
    float border = max(1.0 - smoothstep(0.15 - aa.y, 0.18 + aa.y, uv.y),
        smoothstep(0.745 - aa.y, 0.78 + aa.y, uv.y));
    float panel = abs(fract(uv.x / 1.2 + 0.5) - 0.5) * 1.2;
    float seam = (1.0 - smoothstep(0.009, 0.009 + aa.x, panel))
        * (1.0 - smoothstep(0.025, 0.10, aa.x));
    m.color *= 1.0 - 0.20 * max(seam, border);
    // Four-point compass stencil, authored in the same UV space as the cloth.
    vec2 p = (uv - vec2(5.65, 0.46)) / vec2(0.40, 0.22);
    float diamond = abs(p.x) + abs(p.y);
    float mark_aa = max(aa.x / 0.40, aa.y / 0.22);
    float mark = 1.0 - smoothstep(0.94 - mark_aa, 0.94 + mark_aa, diamond);
    float center = 1.0 - smoothstep(0.27 - mark_aa, 0.27 + mark_aa, diamond);
    m.color = mix(m.color, ink, mark * (1.0 - center) * 0.9);
    m.roughness = mix(m.roughness, 0.68, clamp(stripe + mark, 0.0, 1.0));
}

float pow5(float x) {
    float x2 = x * x;
    return x2 * x2 * x;
}

vec3 fresnel(vec3 f0, float cosine) {
    return f0 + (vec3(1.0) - f0) * pow5(1.0 - clamp(cosine, 0.0, 1.0));
}

float ggxVisibility(vec3 v, vec3 l, vec2 a) {
    float gv = l.z * length(vec3(a * v.xy, v.z));
    float gl = v.z * length(vec3(a * l.xy, l.z));
    return 0.5 / max(gv + gl, 1e-6);
}

vec3 ggxDirect(vec3 v, vec3 l, vec2 a, vec3 f0) {
    vec3 h = normalize(v + l);
    vec3 q = vec3(h.xy / a, h.z);
    float q2 = dot(q, q);
    float d = 1.0 / max(PI * a.x * a.y * q2 * q2, 1e-8);
    return fresnel(f0, dot(v, h)) * (d * ggxVisibility(v, l, a) * max(l.z, 0.0));
}

// Dupuy and Benyoub 2023 spherical-cap visible GGX sampling.
vec3 environmentSpecular(mat3 frame, vec3 v, vec2 a, vec3 f0) {
    vec3 stretched = normalize(vec3(a * v.xy, v.z));
    float gv_root = length(vec3(a * v.xy, v.z));
    vec3 sum = vec3(0.0);
    for (uint i = 0u; i < ENV_SAMPLES; i += 1u) {
        vec3 upoint = ENV_POINTS[i];
        float z = 1.0 - upoint.z * (1.0 + stretched.z);
        float r = sqrt(max(1.0 - z * z, 0.0));
        vec3 cap = vec3(upoint.xy * r, z);
        vec3 hh = cap + stretched;
        vec3 h = normalize(vec3(a * hh.xy, max(hh.z, 0.0)));
        vec3 l = reflect(-v, h);
        if (l.z > 0.0) {
            float hl_root = length(vec3(a * l.xy, l.z));
            float weight = l.z * (v.z + gv_root) / max(l.z * gv_root + v.z * hl_root, 1e-6);
            vec3 radiance = physicalAtmosphereSky(frame * l, ubo.sunDir.xyz, ubo.sunColor.rgb, false);
            sum += radiance * fresnel(f0, dot(v, h)) * weight;
        }
    }
    return sum / float(ENV_SAMPLES);
}

vec3 environmentDiffuse(mat3 frame) {
    // Four taps suffice: the sky gradient is smooth, so diffuse irradiance
    // converges with few samples. Keeps the 8-tap layout positions.
    vec3 sum = vec3(0.0);
    for (uint i = 0u; i < 4u; i += 1u) {
        float angle = float(i) * (2.0 * PI / 4.0);
        vec2 azimuth = vec2(cos(angle), sin(angle));
        float r2 = (float(i) + 0.5) / 4.0;
        vec3 l = vec3(azimuth * sqrt(r2), sqrt(1.0 - r2));
        sum += physicalAtmosphereSky(frame * l, ubo.sunDir.xyz, ubo.sunColor.rgb, false);
    }
    return sum * 0.25;
}

void main() {
    // If terrain forces the camera inside the airframe near-plane envelope,
    // fade the aircraft with stable screen-space coverage so partial near
    // clipping does not leave a harsh sliced silhouette. The normal path is
    // exactly one and skips this branch.
    float airframe_visibility = clamp(ubo.originShift.w, 0.0, 1.0);
    if (airframe_visibility < 0.999) {
        float coverage = fract(sin(dot(gl_FragCoord.xy, vec2(12.9898, 78.233))) * 43758.5453);
        if (coverage > airframe_visibility) discard;
    }
    uint id = vMaterial;
    Material m = getMaterial(id);
    vec3 view = normalize(ubo.campos.xyz - vWorld);
    vec3 ng = normalize(vNormal);
    vec3 n = (dot(ng, view) >= 0.0) ? ng : -ng;
    vec3 dx = dFdx(vWorld);
    vec3 dy = dFdy(vWorld);
    vec2 ux = dFdx(vUv);
    vec2 uy = dFdy(vUv);
    float det = ux.x * uy.y - ux.y * uy.x;
    vec3 tangent_raw = (dx * uy.y - dy * ux.y) * ((det >= 0.0) ? 1.0 : -1.0);
    vec3 tangent_projected = tangent_raw - n * dot(n, tangent_raw);
    vec3 helper = (abs(n.y) > 0.95) ? vec3(1.0, 0.0, 0.0) : vec3(0.0, 1.0, 0.0);
    vec3 t = normalize(cross(helper, n));
    if (dot(tangent_projected, tangent_projected) > 1e-12) {
        t = normalize(tangent_projected);
    }
    vec3 b = cross(n, t);
    float frequency = (id == 3u) ? 64.0 : 28.0;
    float footprint = max(length(ux), length(uy)) * frequency;
    float resolved = 1.0 - smoothstep(0.15, 0.65, footprint);
    vec2 phase = vUv * (2.0 * PI * frequency);
    vec2 wave = sin(phase);
    if (id == 0u) {
        vec3 cloth = textureGrad(sampler2D(weave_tex, weave_smp), vUv, ux, uy).rgb;
        m.color *= cloth / vec3(0.70, 0.67, 0.57);
        expeditionCanvas(m, vUv, abs(ux) + abs(uy));
        n = normalize(n + (t * wave.x + b * wave.y) * (0.045 * resolved));
    } else if (id == 1u) {
        vec2 aa = max(abs(ux) + abs(uy), vec2(0.0001));
        // Cream nose collar and side pinstripes wrap around the real hull UVs.
        float collar = band(vUv.y, 0.14, 0.20, aa.y);
        float sideStripe = band(vUv.x, 0.012, 0.036, aa.x)
            + band(vUv.x, 0.464, 0.488, aa.x);
        float longitudinal = band(vUv.y, 0.22, 0.77, aa.y);
        m.color = mix(m.color, vec3(0.76, 0.62, 0.40),
            clamp(collar + sideStripe * longitudinal, 0.0, 1.0));
        n = normalize(n + (t * wave.x + b * wave.y) * (0.007 * resolved));
    } else if (id == 2u) {
        float twill = wave.x * wave.y * resolved;
        m.color *= 1.0 + 0.18 * twill;
        m.roughness += 0.035 * twill;
        n = normalize(n + (t * wave.x - b * wave.y) * (0.022 * resolved));
    } else if (id == 3u) {
        m.roughness += 0.025 * wave.y * resolved;
        n = normalize(n + b * (0.012 * wave.y * resolved));
    } else if (id == 4u || id == 5u) {
        n = normalize(n + (t * wave.x + b * wave.y) * (0.009 * resolved));
    }
    t = normalize(t - n * dot(n, t));
    b = cross(n, t);
    mat3 frame = mat3(t, b, n);
    vec3 sv = vec3(dot(view, t), dot(view, b), max(dot(view, n), 0.001));
    vec3 sun = ubo.sunDir.xyz;
    vec3 lv = vec3(dot(sun, t), dot(sun, b), dot(sun, n));
    float sun_vis = 1.0;
#ifdef ENABLE_RT
    if (lv.z > 0.001) {
        uint cull_mask = 0xFFu;
        if (vNode == 2u || (vNode >= 4u && vNode <= 6u)) {
            cull_mask = 0xFFu & ~0x02u;
        } else if (vNode == 3u || (vNode >= 7u && vNode <= 9u)) {
            cull_mask = 0xFFu & ~0x04u;
        }
        float alt = vWorld.y - ubo.groundBase.w;
        float max_t = 40.0;
        if (alt < 3123.0 && ubo.sunDir.y > 0.02) {
            float terrain_t = (3123.0 - alt) / ubo.sunDir.y;
            max_t = clamp(terrain_t, 40.0, 8000.0);
        } else if (alt >= 3123.0) {
            cull_mask &= ~0x10u; // Above mountains: skip terrain BVH
        }
        sun_vis = rtSunVisibility(vWorld + n * 0.02, max_t, cull_mask);
    } else {
        sun_vis = 0.0;
    }
#endif
    vec3 dnx = dFdx(n);
    vec3 dny = dFdy(n);
    float variance = min(0.5 * (dot(dnx, dnx) + dot(dny, dny)), 0.18);
    // OpenPBR notes (2025/2026), eq. 86: crossing a rough coat twice
    // broadens the base lobe. IOR 1.5 gives 2*(1 - 1/1.5) = 2/3.
    // Fractional coverage is approximated by mixing slope variances.
    float coat_r2 = m.coat_roughness * m.coat_roughness;
    float rough_r2 = m.roughness * m.roughness;
    float base_variance = min(1.0, rough_r2 * rough_r2
        + (2.0 / 3.0) * m.coat * coat_r2 * coat_r2);
    float alpha = sqrt(base_variance + variance);
    float aspect = sqrt(1.0 - 0.9 * m.anisotropy);
    vec2 a = max(vec2(alpha / aspect, alpha * aspect), vec2(0.004));
    float ess = textureLod(sampler2D(eir_tex, eir_smp), vec2(sv.z, alpha), 0.0).r;
    vec3 gain = vec3(1.0) + m.f0 * (1.0 / max(ess, 0.05) - 1.0);
    vec3 fv = fresnel(m.f0, sv.z);
    vec3 fl = fresnel(m.f0, max(lv.z, 0.0));
    vec3 irradiance = environmentDiffuse(frame);
    vec3 diffuse_albedo = m.color * (1.0 - m.metal) * (vec3(1.0) - fv);
    vec3 ambient = diffuse_albedo * irradiance;
    vec3 direct = diffuse_albedo * ubo.sunColor.rgb * max(lv.z, 0.0) * (vec3(1.0) - fl);
    ambient += environmentSpecular(frame, sv, a, m.f0) * gain;
    if (lv.z > 0.0) {
        direct += ggxDirect(sv, lv, a, m.f0) * gain * ubo.sunColor.rgb * PI;
    }
    if (m.sheen > 0.0) {
        vec3 h = normalize(sv + lv);
        float inv_r = 1.0 / max(m.roughness, 0.1);
        float d = (2.0 + inv_r) * pow(max(1.0 - h.z * h.z, 0.0), 0.5 * inv_r) / (2.0 * PI);
        float visibility = 1.0 / max(4.0 * (max(lv.z, 0.0) + sv.z - max(lv.z, 0.0) * sv.z), 1e-4);
        vec3 sheen_color = sqrt(m.color) * m.sheen;
        ambient = ambient * (1.0 - m.sheen * 0.25)
            + sheen_color * irradiance * pow5(1.0 - sv.z);
        direct = direct * (1.0 - m.sheen * 0.25)
            + sheen_color * ubo.sunColor.rgb * PI * d * visibility * max(lv.z, 0.0);
    }
    vec3 color = ambient + direct * sun_vis;
    if (m.coat > 0.0) {
        float fc_view = fresnel(vec3(0.04), sv.z).x;
        float fc_light = fresnel(vec3(0.04), max(lv.z, 0.0)).x;
        float coat_alpha = max(sqrt(pow(m.coat_roughness, 4.0) + variance), 0.004);
        ambient *= (1.0 - m.coat * fc_view) * (1.0 - m.coat * fc_view);
        direct *= (1.0 - m.coat * fc_view) * (1.0 - m.coat * fc_light);
        color = ambient + direct * sun_vis;
        color += m.coat * environmentSpecular(frame, sv, vec2(coat_alpha), vec3(0.04));
        if (lv.z > 0.0) {
            color += m.coat * ggxDirect(sv, lv, vec2(coat_alpha), vec3(0.04)) * ubo.sunColor.rgb * PI * sun_vis;
        }
    }
    if (id == 7u) {
        // Navigation lights are stable and distinguish left/right; only the
        // aether turbine follows spool. Avoid washing the canvas in neon.
        vec3 emission = vec3(0.10, 0.78, 0.64);
        float intensity = 0.72;
        // +Z is forward, -X is pilot-right: legacy WingL/node2 is starboard.
        if (vNode == 2u) emission = vec3(0.08, 0.85, 0.36);
        else if (vNode == 3u) emission = vec3(1.0, 0.15, 0.045);
        else if (vNode == 1u) emission = vec3(0.95, 0.53, 0.16);
        else intensity = ubo.flex.w * 0.85;
        color += emission * intensity;
    }
    if (id == 6u) {
        float opacity = clamp(fv.x + 0.025 * (1.0 - fv.x), 0.0, 1.0);
        outColor = vec4(color, opacity);
        return;
    }
    outColor = vec4(color, 1.0);
}
