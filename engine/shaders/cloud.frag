#version 450

// Mesh cloud shading. Linear HDR output, matching the sun/sky conventions of
// ground.frag: solar irradiance attenuated by altitude transmittance, zenith
// and horizon sky radiance for ambient, and the same analytic aerial haze so
// distant clouds meet the horizon without a seam. sky_atmo.inc and cloud.inc
// are injected by build.rs after #version.

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
    vec4 trailShift;
    vec4 cameraParams;
    vec4 cameraParams2;
    vec4 groundOrigin;
} ubo;

layout(location = 0) in vec3 vPosition;
layout(location = 1) in vec3 vNormal;
layout(location = 2) in float vHeight01;
layout(location = 3) in float vTone;
layout(location = 4) flat in vec3 vExtinction;
layout(location = 0) out vec4 outColor;

// Henyey-Greenstein phase (Henyey & Greenstein 1941) for the forward-scattered
// silver lining on silhouette edges.
float cloudHG(float cosTheta, float g) {
    float g2 = g * g;
    float den = max(1.0 + g2 - 2.0 * g * cosTheta, 1e-4);
    return (1.0 - g2) / (4.0 * ATMO_PI * pow(den, 1.5));
}

void main() {
    // Analytic smooth normals from the vertex stage: soft, rounded billows
    // instead of faceted crystals.
    vec3 n = normalize(vNormal);
    vec3 sun = normalize(ubo.sunDir.xyz);
    vec3 view = ubo.campos.xyz - vPosition;
    float dist = max(length(view), 1.0);
    view /= dist;

    // Same aerial perspective as ground.frag: wavelength-dependent extinction
    // from the camera altitude melts distant clouds into the horizon haze.
    vec3 transmittance = exp(-dist * vExtinction);
    vec3 haze = ubo.skyHorizon.rgb;

    vec3 color;
    if (gl_FrontFacing) {
        if (dot(n, view) < 0.0) n = -n;
        float cloud_alt = max(vPosition.y - ubo.groundBase.w, 0.0);
        vec3 atmo_origin = vec3(0.0, ATMO_GROUND_R + cloud_alt, 0.0);
        vec3 tr_sun = exp(-atmoSunOpticalDepth(atmo_origin, sun));
        // Wrapped diffuse keeps grazing facets lit while the vertical ambient
        // gradient does the shape work: bright domes, cooler shaded bases.
        float wrap = clamp((dot(n, sun) + 0.55) / 1.55, 0.0, 1.0);
        vec3 direct = ubo.sunColor.rgb * tr_sun * wrap;
        vec3 ambient = mix(ubo.skyHorizon.rgb, ubo.skyZenith.rgb,
            clamp(n.y * 0.5 + 0.5, 0.0, 1.0)) * (0.55 + 0.60 * vHeight01);
        vec3 albedo = vec3(0.965, 0.975, 1.0) * vTone;
        float cos_theta = dot(view, sun);
        float rim = pow(1.0 - clamp(dot(n, view), 0.0, 1.0), 3.0)
            * (0.30 + 2.6 * cloudHG(cos_theta, 0.62));
        color = albedo * (direct + ambient)
            + albedo * ubo.sunColor.rgb * tr_sun * rim;
    } else {
        // Back faces mean the camera is inside the shell. A flat, softly lit
        // veil reads as flying through cloud without any volumetric march.
        vec3 ambient = ubo.skyHorizon.rgb * 0.85 + ubo.skyZenith.rgb * 0.75;
        color = vec3(0.60, 0.645, 0.70) * ambient + ubo.sunColor.rgb * 0.05;
    }

    outColor = vec4(color * transmittance + haze * (1.0 - transmittance), 1.0);
}
