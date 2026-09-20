// cloud.inc
// ---------------------------------------------------------------------------
// Procedural mesh cloud field shared by cloud.vert (visible geometry) and
// ground.frag (sun shadows). Injected by build.rs after sky_atmo.inc.
//
// The field is a hash of absolute world cells, exactly like the terrain
// landmark tiles: there is no cloud state anywhere, so floating-origin
// rebasing, world wrap, and drift wrapping cannot shift or reshape clouds.
// Keep the constants and hash mirrored in engine/src/clouds.rs, which guards
// the vertex budget and the coverage statistics with tests.

const float CLOUD_CELL = 2048.0;            // placement pitch (m)
const int   CLOUD_GRID = 19;                // grid side drawn around the camera
const float CLOUD_DRAW_RADIUS = 20000.0;    // instance draw radius (m)
const uint  CLOUD_PERIOD_CELLS = 32u;       // field repeats every 65,536 m
const float CLOUD_FIELD_PERIOD = 65536.0;
const float CLOUD_DRIFT_SPEED = 8.5;        // m/s per unit of EXPLORA_WIND

const uint  CLOUD_PUFFS = 8u;               // icosphere puffs per cloud
const uint  CLOUD_PUFF_TRIS = 1280u;        // icosphere with three subdivisions
const uint  CLOUD_CORNERS = CLOUD_PUFF_TRIS * 3u;

// Altitude bands in metres above sea level. Cumulus sit just above the high
// peaks so ridges can pierce the deck; thin streaks ride far above.
const float CLOUD_CUMULUS_BASE = 2400.0;
const float CLOUD_CUMULUS_SPAN = 2000.0;
const float CLOUD_STREAK_BASE = 7600.0;
const float CLOUD_STREAK_SPAN = 1800.0;

// Same mixer family as terrain/atmosphere hashes (wrapping uints).
uint cloudHashU(uvec2 cell, uint seed) {
    uint h = cell.x * 1664525u + cell.y * 1013904223u + seed * 2246822519u;
    h ^= h >> 16;
    h *= 2246822519u;
    h ^= h >> 13;
    h *= 3266489917u;
    return h ^ (h >> 16);
}

float cloudHash(uvec2 cell, uint seed) {
    return float(cloudHashU(cell, seed) & 0x00ffffffu) * (1.0 / 16777215.0);
}

// Reduce the split floating-origin anchor exactly like terrainOrigin().
vec2 cloudWorldAnchor(vec4 packedOrigin) {
    return mod(packedOrigin.xy, vec2(CLOUD_FIELD_PERIOD * 4.0)) * 0.25
        + packedOrigin.zw;
}

// 0 = empty, 1 = cumulus, 2 = high streak.
uint cloudCellFamily(uvec2 cell) {
    if (cloudHash(cell, 17u) < 0.38) {
        return 1u;
    }
    if (cloudHash(cell, 91u) < 0.10) {
        return 2u;
    }
    return 0u;
}

// xy = in-cell offset (m), z = altitude ASL (m), w = footprint radius (m).
// Footprints are skewed so most clouds are small while a few grow huge, and a
// rare independent hash doubles down into giant congestus towers.
vec4 cloudCellPlacement(uvec2 cell, uint family) {
    bool streak = family == 2u;
    vec2 offset = vec2(cloudHash(cell, 3u), cloudHash(cell, 7u)) * CLOUD_CELL;
    float altitude = (streak ? CLOUD_STREAK_BASE : CLOUD_CUMULUS_BASE)
        + cloudHash(cell, 11u) * (streak ? CLOUD_STREAK_SPAN : CLOUD_CUMULUS_SPAN);
    float hr = cloudHash(cell, 13u);
    float radius = streak
        ? mix(520.0, 1500.0, hr)
        : mix(170.0, 980.0, pow(hr, 1.7));
    if (!streak && cloudHash(cell, 37u) < 0.06) {
        radius *= 1.6;
    }
    return vec4(offset, altitude, radius);
}

bool cloudIsGiant(uvec2 cell) {
    return cloudHash(cell, 37u) < 0.06;
}

// How many of the eight spiral puffs the cluster actually uses; smaller
// clouds drop their outer puffs so size and silhouette density track.
float cloudPuffCount(uvec2 cell, uint family) {
    if (family != 2u && cloudIsGiant(cell)) {
        return 8.0;
    }
    return mix(4.0, 8.0, cloudHash(cell, 29u));
}

// Shape family per cloud: height scale, lobed-displacement amplitude,
// horizontal stretch of the whole cluster, its yaw, vertical squash, how
// strongly tall clusters lean downwind (the drift direction is +X+Z), and
// the spread profile sh(dir.y) = mix(shBase, shTop, dir.y) that places puffs
// horizontally: domes widen toward the top, columns stay narrow, flat
// fair-weather heaps spread evenly. Every parameter carries an independent
// continuous hash jitter, so the base families blur into a wide spectrum.
void cloudArchetype(uvec2 cell, uint family, out float heightFactor,
                    out float lumpAmp, out float stretch, out float yaw,
                    out float squash, out float shear, out float shBase,
                    out float shTop) {
    yaw = cloudHash(cell, 23u) * 6.2831853;
    if (family == 2u) {
        heightFactor = 0.15;
        lumpAmp = 0.12;
        stretch = 1.5 + 0.8 * cloudHash(cell, 31u);
        squash = 0.30;
        shear = 0.0;
        shBase = 1.0;
        shTop = 1.0;
        return;
    }
    float arch = cloudHash(cell, 19u);
    if (arch < 0.28) {
        heightFactor = 0.58;   // fair-weather heap: even spread, low
        lumpAmp = 0.16;
        shBase = 1.0;
        shTop = 1.0;
    } else if (arch < 0.60) {
        heightFactor = 1.0;    // classic domed mediocris
        lumpAmp = 0.22;
        shBase = 0.60;
        shTop = 1.0;
    } else if (arch < 0.85) {
        heightFactor = 1.55;   // towering column: narrow, tall
        lumpAmp = 0.26;
        shBase = 0.45;
        shTop = 0.85;
    } else {
        heightFactor = 1.2;    // ragged cauliflower
        lumpAmp = 0.38;
        shBase = 0.75;
        shTop = 1.05;
    }
    heightFactor *= 0.75 + 0.5 * cloudHash(cell, 43u);
    lumpAmp *= mix(0.8, 1.3, cloudHash(cell, 47u));
    stretch = 0.9 * cloudHash(cell, 31u) - 0.45;
    squash = mix(0.62, 0.95, cloudHash(cell, 53u));
    shear = (arch >= 0.85 || arch < 0.60)
        ? 0.04 + 0.08 * cloudHash(cell, 59u)
        : 0.22;
}

// Ground sun visibility through the same instances the vertex shader draws.
// The receiver is projected onto each nearby cloud's own altitude, so shadows
// track the visible clouds one to one across the 2400-4400 m deck.
// Unique bounding-box traversal evaluates each cumulus cell once without redundant loops.
float cloudSunVisibility(vec2 worldXZ, float groundY, vec3 sun, float drift) {
    if (sun.y <= 0.05) {
        return 1.0;
    }
    float inv_sun_y = 1.0 / max(sun.y, 0.05);
    vec2 ray_step = sun.xz * inv_sun_y;
    vec2 center0 = worldXZ + ray_step * (2600.0 - groundY);
    vec2 center1 = worldXZ + ray_step * (4200.0 - groundY);
    ivec2 mid0 = ivec2(floor(center0 / CLOUD_CELL));
    ivec2 mid1 = ivec2(floor(center1 / CLOUD_CELL));
    ivec2 min_cell = min(mid0, mid1) - ivec2(1);
    ivec2 max_cell = min(max(mid0, mid1) + ivec2(1), min_cell + ivec2(3, 3));

    float visibility = 1.0;
    for (int dz = min_cell.y; dz <= max_cell.y; ++dz) {
        for (int dx = min_cell.x; dx <= max_cell.x; ++dx) {
            ivec2 wc = ivec2(dx, dz);
            uvec2 cell = uvec2(wc) & uvec2(CLOUD_PERIOD_CELLS - 1u);
            if (cloudHash(cell, 17u) >= 0.38) {
                continue;
            }
            vec4 place = cloudCellPlacement(cell, 1u);
            if (place.z <= groundY + 40.0) {
                continue;
            }
            vec2 shadowXZ = worldXZ + ray_step * (place.z - groundY);
            vec2 cloudXZ = vec2(wc) * CLOUD_CELL + place.xy + vec2(drift);
            float d = distance(shadowXZ, cloudXZ);
            float r = place.w;
            visibility *= mix(0.38, 1.0, smoothstep(r * 0.40, r * 1.2, d));
        }
    }
    return visibility;
}

