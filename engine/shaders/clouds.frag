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
                  vec3 ambient, float phase, out float firstHit) {
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
        float density = cloudCumulusDensity(samplePos, ubo.groundOrigin, weatherTime);
        if (density <= 0.001) continue;
        if (firstHit < 0.0) firstHit = t;
        float sampleTransmittance = exp(-density * CUMULUS_EXTINCTION * stepSize);

        // A short light probe distinguishes bright billow rims from their
        // shaded interiors without a second full ray march.
        float lightDensity = cloudCumulusDensity(samplePos + sun * 650.0,
            ubo.groundOrigin, weatherTime);
        float lightVisibility = exp(-(density + lightDensity) * 0.5
            * CUMULUS_EXTINCTION * 650.0);
        float powder = 1.0 - exp(-density * 3.0);
        float heightFill = smoothstep(CUMULUS_BOTTOM, CUMULUS_TOP, samplePos.y);
        vec3 source = directLight * (phase * 1.5 + 0.15) * powder * lightVisibility
            + ambient * mix(0.58, 1.0, heightFill) * (0.7 + 0.3 * (1.0 - density));
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
            ubo.flex.y * ubo.cameraParams2.w);
        if (density <= 0.001) continue;
        if (firstHit < 0.0) firstHit = t;
        float sampleTransmittance = exp(-density * 0.002 * stepSize);
        vec3 source = directLight * (phase * 1.2 + 0.2) + ambient * 0.85;
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
    float phase = atmoMiePhase(dot(dir, sun));
    vec3 ambient = ubo.skyZenith.xyz * 0.4 + ubo.skyHorizon.xyz * 0.3;
    vec3 midAtmo = vec3(0.0, ATMO_GROUND_R + 4000.0, 0.0);
    vec3 sunTransmittance = exp(-atmoSunOpticalDepth(midAtmo, sun));
    vec3 directLight = ubo.sunColor.rgb * sunTransmittance;

    float cumulusHit = -1.0;
    float cirrusHit = -1.0;
    vec4 cumulus = vec4(0.0);
    vec4 cirrus = vec4(0.0);
    if (hitCumulus) {
        cumulus = marchCumulus(pos, dir, cumulusInterval, sun, directLight,
            ambient, phase, cumulusHit);
    }
    if (hitCirrus) {
        cirrus = marchCirrus(pos, dir, cirrusInterval, directLight,
            ambient, phase, cirrusHit);
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
