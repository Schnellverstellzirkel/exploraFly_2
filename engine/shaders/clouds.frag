#version 450

// sky_atmo.inc and cloud_weather.inc are injected here by build.rs.

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
    vec4 cameraParams2; // w: prevailing wind strength
    vec4 groundOrigin;
} ubo;

layout(location = 0) in vec3 vRay;
layout(location = 0) out vec4 outColor;

// Eight cubic intervals keep nearby in-cloud detail at a bounded march cost.
const int CUMULUS_STEPS = 8;
const int CIRRUS_STEPS = 2;
const float CUMULUS_MAX_DIST = 45000.0;
const float CIRRUS_MAX_DIST = 85000.0;

// Henyey-Greenstein phase function (Henyey & Greenstein 1941)
float henyeyGreenstein(float cosTheta, float g) {
    float g2 = g * g;
    float den = max(1.0 + g2 - 2.0 * g * cosTheta, 1e-4);
    return (1.0 - g2) / (4.0 * ATMO_PI * pow(den, 1.5));
}

// Dual-lobe Henyey-Greenstein phase function for cloud water droplets
// (Bouthors et al. 2008, Wrenninge 2012, Schneider 2015).
// Water droplets (10-20 um) exhibit a pronounced forward diffraction peak (silver lining)
// and a retroreflection backscatter peak (cloud glory / opposition surge).
float cloudPhase(float cosTheta) {
    float fwd = henyeyGreenstein(cosTheta, 0.82);
    float bwd = henyeyGreenstein(cosTheta, -0.22);
    return 0.78 * fwd + 0.22 * bwd;
}

bool intersectSlab(float y, float dirY, float bottom, float top,
                   float maxDistance, out vec2 interval) {
    if (abs(dirY) < 1e-6) {
        interval = vec2(0.0, maxDistance);
        return y >= bottom && y <= top;
    }
    float t1 = (bottom - y) / dirY;
    float t2 = (top - y) / dirY;
    interval = vec2(max(0.0, min(t1, t2)), min(maxDistance, max(t1, t2)));
    return interval.y > interval.x;
}

vec4 marchCumulus(vec3 pos, vec3 dir, vec2 interval, vec3 sun, vec3 directLight,
                  vec3 skyLight, vec3 groundLight, float phase, float multiPhase,
                  out float firstHit) {
    float marchSpan = interval.y - interval.x;
    float transmittance = 1.0;
    vec3 scatter = vec3(0.0);
    firstHit = -1.0;
    for (int i = 0; i < CUMULUS_STEPS; ++i) {
        float u0 = float(i) / float(CUMULUS_STEPS);
        float u1 = float(i + 1) / float(CUMULUS_STEPS);
        float segmentStart = interval.x + marchSpan * u0 * u0 * u0;
        float segmentEnd = interval.x + marchSpan * u1 * u1 * u1;
        float stepSize = segmentEnd - segmentStart;
        float t = (segmentStart + segmentEnd) * 0.5;
        vec3 samplePos = pos + dir * t;
        float weatherTime = ubo.flex.y * ubo.cameraParams2.w;
        float distFade = 1.0 - smoothstep(32000.0, 44000.0, t);
        float density = cloudCumulusDensity(samplePos, ubo.groundOrigin, weatherTime) * distFade;
        if (density <= 0.001) continue;
        if (firstHit < 0.0) firstHit = t;
        float sampleTransmittance = exp(-density * CUMULUS_EXTINCTION * stepSize);

        // A short light probe distinguishes bright billow rims from their
        // shaded interiors without a second full ray march.
        float lightDensity = cloudCumulusDensity(samplePos + sun * 450.0,
            ubo.groundOrigin, weatherTime) * distFade;
        float sunCloudTau = lightDensity * (CUMULUS_EXTINCTION * 600.0);
        float directSingle = exp(-sunCloudTau) * phase * 1.2;
        float directMulti = exp(-sunCloudTau * 0.25) * (multiPhase * 0.8 + 0.36);
        float heightFraction = clamp((samplePos.y - CUMULUS_BOTTOM)
            / (CUMULUS_TOP - CUMULUS_BOTTOM), 0.0, 1.0);
        vec3 ambient = mix(groundLight, skyLight, smoothstep(0.0, 0.85, heightFraction));
        vec3 source = directLight * (directSingle + directMulti)
            + ambient * (0.55 + 0.45 * (1.0 - density * 0.5));
        scatter += source * (1.0 - sampleTransmittance) * transmittance;
        transmittance *= sampleTransmittance;
        if (transmittance < 0.01) break;
    }
    return vec4(scatter, 1.0 - transmittance);
}

vec4 marchCirrus(vec3 pos, vec3 dir, vec2 interval, vec3 directLight,
                 vec3 ambient, float phase, out float firstHit) {
    float stepSize = (interval.y - interval.x) / float(CIRRUS_STEPS);
    float transmittance = 1.0;
    vec3 scatter = vec3(0.0);
    firstHit = -1.0;
    for (int i = 0; i < CIRRUS_STEPS; ++i) {
        float t = interval.x + (float(i) + 0.5) * stepSize;
        float density = cloudCirrusDensity(pos + dir * t, ubo.groundOrigin,
            ubo.flex.y * ubo.cameraParams2.w) * (1.0 - smoothstep(60000.0, 84000.0, t));
        if (density <= 0.001) continue;
        if (firstHit < 0.0) firstHit = t;
        float sampleTransmittance = exp(-density * 0.002 * stepSize);
        vec3 source = directLight * (phase * 1.5 + 0.35) + ambient;
        scatter += source * (1.0 - sampleTransmittance) * transmittance;
        transmittance *= sampleTransmittance;
        if (transmittance < 0.01) break;
    }
    return vec4(scatter, 1.0 - transmittance);
}

void main() {
    vec3 dir = normalize(vRay);
    // Keep vertical sampling in altitude coordinates through every origin
    // rebase, just as the split XZ anchor keeps horizontal noise stationary.
    vec3 pos = vec3(ubo.campos.x, ubo.campos.y - ubo.groundBase.w, ubo.campos.z);
    vec2 cumulusInterval, cirrusInterval;
    bool hitCumulus = intersectSlab(pos.y, dir.y,
        CUMULUS_BOTTOM - CUMULUS_BASE_VARIATION,
        CUMULUS_TOP + CUMULUS_BASE_VARIATION, CUMULUS_MAX_DIST, cumulusInterval);
    bool hitCirrus = intersectSlab(pos.y, dir.y,
        CIRRUS_BOTTOM, CIRRUS_TOP, CIRRUS_MAX_DIST, cirrusInterval);
    if (!hitCumulus && !hitCirrus) discard;

    vec3 sun = normalize(ubo.sunDir.xyz);
    float cosTheta = dot(dir, sun);
    float phase = cloudPhase(cosTheta);
    float multiPhase = henyeyGreenstein(cosTheta, 0.35);
    vec3 skyLight = (2.0 * ubo.skyZenith.rgb + ubo.skyHorizon.rgb) / 3.0;
    vec3 groundAtmo = vec3(0.0, ATMO_GROUND_R, 0.0);
    vec3 groundSun = ubo.sunColor.rgb * exp(-atmoSunOpticalDepth(groundAtmo, sun))
        * max(sun.y, 0.0);
    vec3 groundSky = ATMO_PI * (ubo.skyZenith.rgb + 2.0 * ubo.skyHorizon.rgb) / 3.0;
    vec3 groundAlbedo = max(ubo.groundBase.rgb * 2.2, vec3(0.08, 0.11, 0.06));
    vec3 groundLight = groundAlbedo * (groundSun + groundSky) * (0.90 / ATMO_PI);
    vec3 midAtmo = vec3(0.0, ATMO_GROUND_R + 4000.0, 0.0);
    vec3 sunTransmittance = exp(-atmoSunOpticalDepth(midAtmo, sun));
    vec3 directLight = ubo.sunColor.rgb * sunTransmittance;

    float cumulusHit = -1.0;
    float cirrusHit = -1.0;
    vec4 cumulus = vec4(0.0);
    vec4 cirrus = vec4(0.0);
    if (hitCumulus) {
        cumulus = marchCumulus(pos, dir, cumulusInterval, sun, directLight,
            skyLight, groundLight, phase, multiPhase, cumulusHit);
    }
    if (hitCirrus) {
        vec3 cirrusAtmo = vec3(0.0, ATMO_GROUND_R + 10500.0, 0.0);
        vec3 cirrusLight = ubo.sunColor.rgb * exp(-atmoSunOpticalDepth(cirrusAtmo, sun));
        float icePhase = 0.85 * henyeyGreenstein(cosTheta, 0.82) + 0.15 / (4.0 * ATMO_PI);
        cirrus = marchCirrus(pos, dir, cirrusInterval, cirrusLight,
            skyLight * 0.70 + groundLight * 0.30, icePhase, cirrusHit);
    }

    // Premultiplied front-to-back composition in distance order also works
    // when diving through cirrus toward the cumulus deck below the camera.
    bool cirrusInFront = cirrusHit >= 0.0 && (cumulusHit < 0.0 || cirrusHit < cumulusHit);
    vec4 front = cirrusInFront ? cirrus : cumulus;
    vec4 back = cirrusInFront ? cumulus : cirrus;
    vec4 cloud = front + back * (1.0 - front.a);
    if (cloud.a < 0.01) discard;
    float firstHit = cirrusInFront ? cirrusHit : cumulusHit;

    // Distant cloud radiance picks up the same atmospheric veil as land.
    // Haze is multiplied by opacity to preserve premultiplied alpha edges.
    vec3 atmoOrigin = atmoModelOrigin(ubo.campos.xyz, ubo.groundBase.w);
    float cameraHeight = max(pos.y, 0.0);
    vec3 extinction = ATMO_BETA_RAYLEIGH * exp(-cameraHeight / ATMO_H_RAYLEIGH)
        + ATMO_BETA_MIE_EXTINCT * exp(-cameraHeight / ATMO_H_MIE);
    vec3 viewTransmittance = exp(-max(firstHit, 0.0) * extinction);
    vec3 haze = atmoRadianceCheapTr(atmoOrigin, dir, sun, ubo.sunColor.rgb,
        exp(-atmoSunOpticalDepth(atmoOrigin, sun)));
    cloud.rgb = cloud.rgb * viewTransmittance + haze * cloud.a * (1.0 - viewTransmittance);

    vec4 clip = ubo.viewProj * vec4(ubo.campos.xyz + dir * max(firstHit, 0.1), 1.0);
    gl_FragDepth = clip.w <= 0.0 ? 0.999999 : clamp(clip.z / clip.w, 0.0, 0.999999);
    outColor = cloud;
}
