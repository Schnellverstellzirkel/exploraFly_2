#version 450

// Sky + analytic infinite flat-ground fragment. Linear HDR output, same
// atmosphere terms as plane.frag. The ground is ray/plane intersected in the
// fullscreen pass, so it has no finite mesh edge or world-coordinate drift.

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

layout(location = 0) in vec3 vRay;
layout(location = 0) out vec4 outColor;

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

void main() {
    vec3 view_dir = normalize(vRay);

    // The fullscreen primitive is placed at the far depth. A ground hit
    // replaces that depth with the exact projected intersection, allowing the
    // existing aircraft and future terrain objects to occlude it correctly.
    gl_FragDepth = 1.0;
    float denom = view_dir.y;
    if (abs(denom) > 1e-5) {
        float hit_t = (ubo.groundBase.w - ubo.campos.y) / denom;
        if (hit_t > 0.0) {
            vec3 hit = ubo.campos.xyz + view_dir * hit_t;
            vec4 clip = ubo.viewProj * vec4(hit, 1.0);
            if (clip.w > 0.0) {
                // Keep the mathematically infinite plane visible beyond the
                // camera far distance; atmospheric fade hides the clamp.
                gl_FragDepth = clamp(clip.z / clip.w, 0.0, 0.999999);

                float sun_cos = max(ubo.sunDir.y, 0.0);
                vec3 sky_irradiance = mix(ubo.skyHorizon.rgb, ubo.skyZenith.rgb, 0.22) * 0.22;
                vec3 direct_irradiance = ubo.sunColor.rgb * sun_cos * 0.82;
                vec3 ground_radiance = ubo.groundBase.rgb * 3.0
                    * (sky_irradiance + direct_irradiance);
                vec3 haze = physicalAtmosphereSky(
                    view_dir, ubo.sunDir.xyz, ubo.sunColor.rgb, false);
                float transmittance = exp(-hit_t * 0.000035);
                outColor = vec4(mix(haze, ground_radiance, transmittance), 1.0);
                return;
            }
        }
    }

    vec3 hdr_sky = physicalAtmosphereSky(view_dir, ubo.sunDir.xyz, ubo.sunColor.rgb, true);
    outColor = vec4(hdr_sky, 1.0);
}
