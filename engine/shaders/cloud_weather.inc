// Shared cloud field for the visible volumes and their ground shadows.
// Inject after sky_atmo.inc. Positions use local XZ and height above ground;
// origin is the existing split groundOrigin uniform, not a new GPU binding.

const float CUMULUS_BOTTOM = 2600.0;
const float CUMULUS_TOP = 5600.0;
const float CUMULUS_BASE_VARIATION = 300.0;
const float CUMULUS_EXTINCTION = 0.0012;
const float CIRRUS_BOTTOM = 9500.0;
const float CIRRUS_TOP = 11500.0;

float cloudWeatherNoise(vec3 localPos, vec4 origin, float cell, uint seed) {
    // Carry the large world anchor as an integer lattice coordinate. Only the
    // remainder enters interpolation, so rebasing cannot make weather slide
    // with the aircraft or change its small-scale shape at large distances.
    float anchorCells = cell / 0.25;
    vec2 originCell = floor(origin.xy / anchorCells);
    vec2 originRemainder = mod(origin.xy, vec2(anchorCells)) * 0.25 + origin.zw;
    vec2 horizontal = (localPos.xz + originRemainder) / cell;
    vec3 q = vec3(horizontal.x, localPos.y / cell, horizontal.y);
    vec3 base = floor(q) + vec3(originCell.x, 0.0, originCell.y);
    vec3 f = fract(q);
    vec3 w = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    uint period = uint(ATMO_CLOUD_TILE / cell + 0.5);
    uvec3 c0 = uvec3(mod(base, float(period)));
    uvec3 c1 = (c0 + 1u) & (period - 1u);

    float n00 = mix(atmoCloudHash(uvec3(c0.x, c0.y, c0.z), seed),
                      atmoCloudHash(uvec3(c1.x, c0.y, c0.z), seed), w.x);
    float n10 = mix(atmoCloudHash(uvec3(c0.x, c1.y, c0.z), seed),
                      atmoCloudHash(uvec3(c1.x, c1.y, c0.z), seed), w.x);
    float n01 = mix(atmoCloudHash(uvec3(c0.x, c0.y, c1.z), seed),
                      atmoCloudHash(uvec3(c1.x, c0.y, c1.z), seed), w.x);
    float n11 = mix(atmoCloudHash(uvec3(c0.x, c1.y, c1.z), seed),
                      atmoCloudHash(uvec3(c1.x, c1.y, c1.z), seed), w.x);
    return mix(mix(n00, n10, w.y), mix(n01, n11, w.y), w.z);
}

float cloudCumulusDensityCoarse(vec3 pos, vec4 origin, float time) {
    if (pos.y < CUMULUS_BOTTOM - CUMULUS_BASE_VARIATION ||
        pos.y > CUMULUS_TOP + CUMULUS_BASE_VARIATION) {
        return 0.0;
    }
    vec3 advected = pos - vec3(time * 8.5, 0.0, time * 8.5);
    vec3 baseCoord = vec3(advected.x, 0.0, advected.z);
    float offset = (cloudWeatherNoise(baseCoord, origin, 16000.0, 200u) * 2.0 - 1.0)
        * CUMULUS_BASE_VARIATION;
    float heightFraction = (pos.y - CUMULUS_BOTTOM - offset)
        / (CUMULUS_TOP - CUMULUS_BOTTOM);
    if (heightFraction <= 0.0 || heightFraction >= 1.0) return 0.0;

    float baseFade = smoothstep(0.0, 0.08, heightFraction);
    float topFade = 1.0 - smoothstep(0.65, 1.0, heightFraction);
    float profile = baseFade * topFade;
    if (profile <= 0.001) return 0.0;

    vec3 shapePos = vec3(advected.x, pos.y * 1.15, advected.z);
    float coverage = 0.63 + 0.18 * smoothstep(0.30, 0.88, heightFraction)
        + 0.08 * (1.0 - smoothstep(0.0, 0.12, heightFraction));
    float baseOctave = cloudWeatherNoise(shapePos, origin, 4000.0, 42u);
    if (baseOctave < coverage - 0.20) return 0.0;
    float n1 = cloudWeatherNoise(shapePos, origin, 1800.0, 59u);
    float billow1 = 1.0 - abs(n1 * 2.0 - 1.0);
    float baseShape = 0.60 * baseOctave + 0.40 * billow1;
    if (baseShape < coverage) return 0.0;
    float density = (baseShape - coverage) / max(1.0 - coverage, 1e-4);
    density = density * density * (3.0 - 2.0 * density);
    return density * profile;
}

float cloudCumulusDensity(vec3 pos, vec4 origin, float time) {
    if (pos.y < CUMULUS_BOTTOM - CUMULUS_BASE_VARIATION ||
        pos.y > CUMULUS_TOP + CUMULUS_BASE_VARIATION) {
        return 0.0;
    }
    // Positive X/Z prevailing breeze advection
    vec3 advected = pos - vec3(time * 8.5, 0.0, time * 8.5);
    vec3 baseCoord = vec3(advected.x, 0.0, advected.z);
    float offset = (cloudWeatherNoise(baseCoord, origin, 16000.0, 200u) * 2.0 - 1.0)
        * CUMULUS_BASE_VARIATION;
    float heightFraction = (pos.y - CUMULUS_BOTTOM - offset)
        / (CUMULUS_TOP - CUMULUS_BOTTOM);
    if (heightFraction <= 0.0 || heightFraction >= 1.0) return 0.0;

    vec3 shapePos = vec3(advected.x, pos.y * 1.15, advected.z);

    // 3D domain warp: organic swirling turbulence and vertical thermal plumes with 2 noise calls
    float w1 = cloudWeatherNoise(shapePos, origin, 3600.0, 101u);
    float w2 = cloudWeatherNoise(shapePos, origin, 3600.0, 202u);
    vec3 warp = vec3(
        (w1 - 0.5) * 480.0,
        (w1 + w2 - 1.0) * 280.0,
        (w2 - 0.5) * 480.0
    );
    vec3 warpedPos = shapePos + warp;

    // Macro cloud coverage envelope: defines distinct scattered cumulus clouds
    float coverage = 0.63 + 0.18 * smoothstep(0.30, 0.88, heightFraction)
        + 0.08 * (1.0 - smoothstep(0.0, 0.12, heightFraction));
    float baseOctave = cloudWeatherNoise(warpedPos, origin, 4000.0, 42u);
    // Fast early-out for empty sky (used by raymarch and ground shadow)
    if (baseOctave < coverage - 0.22) return 0.0;

    float dist = length(pos.xz);
    float billowFade = 1.0 - smoothstep(14000.0, 32000.0, dist);
    float detailFade = 1.0 - smoothstep(6000.0, 20000.0, dist);

    // Billow octave 1: medium billow lobes (faded at horizon to prevent aliasing)
    float n1 = cloudWeatherNoise(warpedPos, origin, 1800.0, 59u);
    float billow1 = 1.0 - abs(n1 * 2.0 - 1.0);

    // Convective cauliflower tops: billows push cloud tops higher than intervening gaps
    float topFade = 1.0 - smoothstep(0.55 + 0.35 * billow1, 1.0, heightFraction);
    float baseFade = smoothstep(0.0, 0.08, heightFraction);
    float profile = baseFade * topFade;
    if (profile <= 0.001) return 0.0;

    float billowWeight = 0.38 * billowFade;
    float baseShape = (1.0 - billowWeight) * baseOctave + billowWeight * billow1;
    if (baseShape < coverage) return 0.0;

    // Billow octave 2: fine cauliflower lobes and micro-wisps (near-to-mid field only)
    float detail = 0.0;
    if (detailFade > 0.01) {
        float n2 = cloudWeatherNoise(warpedPos * 2.2, origin, 1800.0, 83u);
        float billow2 = 1.0 - abs(n2 * 2.0 - 1.0);
        float n3 = cloudWeatherNoise(warpedPos * 4.4, origin, 1800.0, 117u);
        detail = (0.65 * billow2 + 0.35 * n3) * detailFade;
    }

    // Edge erosion: core stays dense while perimeter is carved into wisps and billows
    float density = (baseShape - coverage) / max(1.0 - coverage, 1e-4);
    if (detail > 0.0) {
        density = clamp(density - (1.0 - density) * (1.0 - detail) * (0.40 * detailFade), 0.0, 1.0);
    }
    // Soft quadratic onset at cloud edges for natural volumetric fluffiness
    density = density * density * (3.0 - 2.0 * density);

    return density * profile;
}

float cloudCirrusDensity(vec3 pos, vec4 origin, float time) {
    if (pos.y < CIRRUS_BOTTOM || pos.y > CIRRUS_TOP) return 0.0;
    float heightFraction = (pos.y - CIRRUS_BOTTOM) / (CIRRUS_TOP - CIRRUS_BOTTOM);
    float profile = smoothstep(0.0, 0.20, heightFraction)
        * (1.0 - smoothstep(0.80, 1.0, heightFraction));
    vec3 shapePos = vec3(pos.x - time * 8.5, pos.y * 1.5, pos.z - time * 8.5);
    float noise1 = cloudWeatherNoise(shapePos, origin, 8000.0, 71u);
    float noise2 = cloudWeatherNoise(shapePos * 2.4, origin, 8000.0, 93u);
    float noise = 0.70 * noise1 + 0.30 * noise2;
    return max(noise - 0.62, 0.0) / 0.38 * profile * 0.22;
}

float cloudGroundSunVisibility(vec3 groundPos, vec3 sun, vec4 origin, float time) {
    if (sun.y <= 0.02) return 1.0;
    // Three fixed strata sample the same moving volume that is visible above.
    // Its continuous density already softens cloud edges; ambient skylight
    // remains untouched so shadowed meadows keep their cool, readable fill.
    const int STEPS = 3;
    float slabBottom = CUMULUS_BOTTOM - CUMULUS_BASE_VARIATION;
    float slabHeight = CUMULUS_TOP + CUMULUS_BASE_VARIATION - slabBottom;
    float stepHeight = slabHeight / float(STEPS);
    float opticalDepth = 0.0;
    for (int i = 0; i < STEPS; ++i) {
        float sampleHeight = slabBottom + (float(i) + 0.5) * stepHeight;
        float t = (sampleHeight - groundPos.y) / sun.y;
        vec3 samplePos = groundPos + sun * t;
        opticalDepth += cloudCumulusDensityCoarse(samplePos, origin, time)
            * CUMULUS_EXTINCTION * stepHeight / sun.y;
    }
    return mix(1.0, exp(-opticalDepth), smoothstep(0.02, 0.06, sun.y));
}
