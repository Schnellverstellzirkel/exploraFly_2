// Periodic alpine world. Mirrored by crates/world/src/lib.rs for CPU clearance.
// The origin is reduced before adding local coordinates, preserving float phase.
const float TERRAIN_PERIOD = 65536.0;
const float TERRAIN_WATER = 185.0;
const uint TERRAIN_CELLS = 64u;
const uint TERRAIN_VERTICES = TERRAIN_CELLS * TERRAIN_CELLS * 6u;
const uint TERRAIN_STRUCTURES = 24u;

vec2 terrainOrigin(vec4 packedOrigin) {
    return mod(packedOrigin.xy, vec2(TERRAIN_PERIOD * 4.0)) * 0.25 + packedOrigin.zw;
}

uint terrainHashU(uvec2 c, uint seed) {
    uint h = c.x * 1664525u + c.y * 1013904223u + seed * 2246822519u;
    h ^= h >> 16;
    h *= 2246822519u;
    h ^= h >> 13;
    h *= 3266489917u;
    return h ^ (h >> 16);
}

float terrainHash(uvec2 c, uint seed) {
    return float(terrainHashU(c, seed) & 0x00ffffffu) * (1.0 / 16777215.0);
}

float terrainNoise(vec2 p, float cell, uint seed) {
    vec2 q = mod(p, vec2(TERRAIN_PERIOD)) / cell;
    uvec2 c = uvec2(floor(q));
    uint mask = uint(TERRAIN_PERIOD / cell) - 1u;
    uvec2 c1 = (c + 1u) & mask;
    c &= mask;
    vec2 f = fract(q);
    vec2 w = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    return mix(mix(terrainHash(c, seed), terrainHash(uvec2(c1.x, c.y), seed), w.x),
        mix(terrainHash(uvec2(c.x, c1.y), seed), terrainHash(c1, seed), w.x), w.y);
}

float terrainValleyCenter(float z) {
    return 900.0 * sin(z * (6.283185307179586 / 32768.0))
        + 380.0 * sin(z * (6.283185307179586 / 8192.0));
}

float terrainHeight(vec2 absoluteXZ) {
    vec2 p = mod(absoluteXZ, vec2(TERRAIN_PERIOD));
    float crossValley = mod(p.x - terrainValleyCenter(p.y) + 8192.0, 16384.0) - 8192.0;
    float shoulder = smoothstep(420.0, 3800.0, abs(crossValley));
    float ridge = 1.0 - abs(2.0 * terrainNoise(p, 2048.0, 31u) - 1.0);
    float crag = 1.0 - abs(2.0 * terrainNoise(p, 512.0, 67u) - 1.0);
    float elevation = 235.0 + terrainNoise(p, 1024.0, 101u) * 90.0
        + shoulder * (950.0 + 1550.0 * ridge * ridge + 280.0 * crag * crag * crag)
        + terrainNoise(p, 128.0, 223u) * 18.0 * (0.2 + 0.8 * shoulder);
    float lakeZ = mod(p.y - 4800.0 + 8192.0, 16384.0) - 8192.0;
    vec2 lakeP = vec2(crossValley / 850.0, lakeZ / 1400.0);
    float basin = 1.0 - smoothstep(0.62, 1.30, dot(lakeP, lakeP));
    return mix(elevation, 110.0 + 14.0 * terrainNoise(p, 256.0, 193u), basin);
}

// xz offset, xz half extent; two heights are returned separately.
vec4 terrainStructure(uint index, out float wallHeight, out float roofHeight) {
    if (index == 0u) {
        wallHeight = 100.0; roofHeight = 34.0;
        return vec4(0.0, 0.0, 42.0, 32.0);
    }
    if (index <= 4u) {
        uint corner = index - 1u;
        wallHeight = 135.0; roofHeight = 40.0;
        return vec4((corner & 1u) == 0u ? -64.0 : 64.0,
            (corner & 2u) == 0u ? -52.0 : 52.0, 15.0, 15.0);
    }
    if (index <= 6u) {
        wallHeight = 40.0; roofHeight = 5.0;
        return vec4(index == 5u ? -64.0 : 64.0, 0.0, 7.0, 52.0);
    }
    if (index <= 8u) {
        wallHeight = 40.0; roofHeight = 5.0;
        return vec4(0.0, index == 7u ? -52.0 : 52.0, 64.0, 7.0);
    }
    uint house = index - 9u;
    wallHeight = 18.0 + float(house % 3u) * 5.0; roofHeight = 15.0;
    return vec4(-260.0 + float(house % 5u) * 64.0,
        -220.0 - float(house / 5u) * 68.0, 16.0, 13.0);
}
