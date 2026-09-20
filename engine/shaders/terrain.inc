// Periodic alpine terrain equations. Constants and landmark tables are
// injected from crates/world as world_generated.inc by engine/build.rs. The
// origin is reduced before adding local coordinates, preserving float phase.

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
    float ax = abs(crossValley);
    // Glacial trough: flat floor, steep sides.
    float shoulder = pow(smoothstep(420.0, 3800.0, ax), 1.25);
    float ridge = 1.0 - abs(2.0 * terrainNoise(p, 2048.0, 31u) - 1.0);
    ridge = pow(ridge, 0.75);
    float crag = 1.0 - abs(2.0 * terrainNoise(p, 512.0, 67u) - 1.0);
    // Massif envelope: slow height variation decides which ranges become high
    // Alps and which stay rolling foothills, so peaks stop sharing one height.
    // The remap gives flat-topped high zones where full-height peaks form.
    float massif = (0.30 + 0.70 * smoothstep(0.20, 0.75, terrainNoise(p, 16384.0, 311u)))
        * (0.80 + 0.20 * terrainNoise(p, 8192.0, 317u));
    // Foothill belt: rolling pre-alpine hills between floor and high rock.
    float foothillBelt = smoothstep(500.0, 1500.0, ax) * (1.0 - shoulder);
    float foothill = terrainNoise(p, 1024.0, 331u) * 2.0 - 1.0;
    float elevation = 235.0 + terrainNoise(p, 1024.0, 101u) * 90.0
        + foothillBelt * max(180.0 + 220.0 * foothill * massif, 0.0)
        + shoulder * massif * (950.0 + 1550.0 * ridge * ridge + 280.0 * crag * crag * crag)
        + terrainNoise(p, 128.0, 223u) * 18.0 * (0.2 + 0.8 * shoulder);
    float lakeZ = mod(p.y - 4800.0 + 8192.0, 16384.0) - 8192.0;
    vec2 lakeP = vec2(crossValley / 850.0, lakeZ / 1400.0);
    float basin = 1.0 - smoothstep(0.62, 1.30, dot(lakeP, lakeP));
    return mix(elevation, 110.0 + 14.0 * terrainNoise(p, 256.0, 193u), basin);
}
