#version 450

// Airframe fragment stage. Linear HDR output, tone mapped once in composite.
// NOTE: build.rs prepends a generated header defining ENV_SAMPLES and
// ENV_POINTS (Hammersley azimuths) before compiling. Do not define them here.
// Material IDs, GGX, Charlie sheen, clearcoat, Dupuy-Benyoub VNDF sampling
// match the previous WGSL implementation term for term.

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
} ubo;

layout(set = 0, binding = 1) uniform texture2D weave_tex;
layout(set = 0, binding = 2) uniform sampler weave_smp;
layout(set = 0, binding = 3) uniform texture2D eir_tex;
layout(set = 0, binding = 4) uniform sampler eir_smp;

layout(location = 0) in vec3 vNormal;
layout(location = 1) in vec3 vWorld;
layout(location = 2) in vec2 vUv;
layout(location = 3) flat in uint vMaterial;

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
        return Material(vec3(0.59, 0.57, 0.51), 0.78, vec3(0.035), 0.0, 0.20, 0.0, 0.0, 0.25);
    } else if (id == 1u) {
        return Material(vec3(0.47, 0.48, 0.46), 0.44, vec3(0.004), 0.0, 0.0, 1.0, 0.24, 0.0);
    } else if (id == 2u) {
        return Material(vec3(0.022, 0.027, 0.032), 0.48, vec3(0.025), 0.0, 0.60, 1.0, 0.29, 0.0);
    } else if (id == 3u) {
        return Material(vec3(0.0), 0.30, vec3(0.55, 0.58, 0.61), 1.0, 0.72, 0.0, 0.0, 0.0);
    } else if (id == 4u) {
        return Material(vec3(0.014, 0.019, 0.024), 0.76, vec3(0.04), 0.0, 0.0, 0.0, 0.0, 0.0);
    } else if (id == 5u) {
        return Material(vec3(0.19, 0.085, 0.043), 0.83, vec3(0.035), 0.0, 0.0, 0.0, 0.0, 0.10);
    } else if (id == 6u) {
        return Material(vec3(0.0), 0.075, vec3(0.04), 0.0, 0.0, 0.0, 0.0, 0.0);
    } else {
        return Material(vec3(0.12, 0.16, 0.24), 0.38, vec3(0.04), 0.0, 0.0, 0.0, 0.0, 0.0);
    }
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
        n = normalize(n + (t * wave.x + b * wave.y) * (0.045 * resolved));
    } else if (id == 2u) {
        float twill = wave.x * wave.y * resolved;
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
    mat3 frame = mat3(t, b, n);
    vec3 sv = vec3(dot(view, t), dot(view, b), max(dot(view, n), 0.001));
    vec3 sun = ubo.sunDir.xyz;
    vec3 lv = vec3(dot(sun, t), dot(sun, b), dot(sun, n));
    vec3 dnx = dFdx(n);
    vec3 dny = dFdy(n);
    float variance = min(0.5 * (dot(dnx, dnx) + dot(dny, dny)), 0.18);
    float alpha = sqrt(m.roughness * m.roughness * m.roughness * m.roughness + variance);
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
    vec3 color = ambient + direct;
    if (m.coat > 0.0) {
        float fc_view = fresnel(vec3(0.04), sv.z).x;
        float fc_light = fresnel(vec3(0.04), max(lv.z, 0.0)).x;
        float coat_alpha = max(sqrt(pow(m.coat_roughness, 4.0) + variance), 0.004);
        ambient *= (1.0 - m.coat * fc_view) * (1.0 - m.coat * fc_view);
        direct *= (1.0 - m.coat * fc_view) * (1.0 - m.coat * fc_light);
        color = ambient + direct;
        color += m.coat * environmentSpecular(frame, sv, vec2(coat_alpha), vec3(0.04));
        if (lv.z > 0.0) {
            color += m.coat * ggxDirect(sv, lv, vec2(coat_alpha), vec3(0.04)) * ubo.sunColor.rgb * PI;
        }
    }
    if (id == 7u) {
        color += vec3(0.52, 0.61, 1.0) * ubo.flex.w;
    }
    if (id == 6u) {
        float opacity = clamp(fv.x + 0.025 * (1.0 - fv.x), 0.0, 1.0);
        outColor = vec4(color, opacity);
        return;
    }
    outColor = vec4(color, 1.0);
}
