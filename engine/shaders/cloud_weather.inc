// Shared cloud field for the visible volumes and their ground shadows.
// Inject after sky_atmo.inc. Positions use local XZ and height above ground;
// origin is the existing split groundOrigin uniform, not a new GPU binding.

const float CUMULUS_BOTTOM = 2600.0;
const float CUMULUS_TOP = 5600.0;
const float CUMULUS_BASE_VARIATION = 300.0;
const float CUMULUS_EXTINCTION = 0.0035;
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

float cloudCumulusDensity(vec3 pos, vec4 origin, float time) {
    if (pos.y < CUMULUS_BOTTOM - CUMULUS_BASE_VARIATION ||
        pos.y > CUMULUS_TOP + CUMULUS_BASE_VARIATION) {
        return 0.0;
    }
    // Positive X/Z prevailing breeze; subtract its displacement to advect
    // the complete field coherently instead of deforming its base separately.
    vec3 advected = pos - vec3(time * 8.5, 0.0, time * 8.5);
    vec3 baseCoord = vec3(advected.x, 0.0, advected.z);
    float offset = (cloudWeatherNoise(baseCoord, origin, 16000.0, 200u) * 2.0 - 1.0)
        * CUMULUS_BASE_VARIATION;
    float heightFraction = (pos.y - CUMULUS_BOTTOM - offset)
        / (CUMULUS_TOP - CUMULUS_BOTTOM);
    float profile = smoothstep(0.0, 0.07, heightFraction)
        * (1.0 - smoothstep(0.65, 1.0, heightFraction));
    if (profile <= 0.001) return 0.0;

    vec3 shapePos = vec3(advected.x, pos.y * 2.2, advected.z);
    float coverage = 0.56 + 0.24 * smoothstep(0.35, 0.92, heightFraction)
        + 0.12 * (1.0 - smoothstep(0.0, 0.14, heightFraction));
    float baseOctave = cloudWeatherNoise(shapePos, origin, 4000.0, 42u);
    // An exact bound skips the finer octave for empty sky and unshadowed land.
    if (0.65 * baseOctave + 0.35 < coverage) return 0.0;
    float detailOctave = cloudWeatherNoise(shapePos, origin, 2000.0, 59u);
    float density = max(0.65 * baseOctave + 0.35 * detailOctave - coverage, 0.0)
        / (1.0 - coverage);
    return density * density * (3.0 - 2.0 * density) * profile;
}

float cloudCirrusDensity(vec3 pos, vec4 origin, float time) {
    if (pos.y < CIRRUS_BOTTOM || pos.y > CIRRUS_TOP) return 0.0;
    float heightFraction = (pos.y - CIRRUS_BOTTOM) / (CIRRUS_TOP - CIRRUS_BOTTOM);
    float profile = smoothstep(0.0, 0.20, heightFraction)
        * (1.0 - smoothstep(0.80, 1.0, heightFraction));
    vec3 shapePos = vec3(pos.x - time * 8.5, pos.y * 1.5, pos.z - time * 8.5);
    float noise = cloudWeatherNoise(shapePos, origin, 8000.0, 71u);
    return max(noise - 0.68, 0.0) / 0.32 * profile * 0.12;
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
        opticalDepth += cloudCumulusDensity(samplePos, origin, time)
            * CUMULUS_EXTINCTION * stepHeight / sun.y;
    }
    return mix(1.0, exp(-opticalDepth), smoothstep(0.02, 0.06, sun.y));
}
