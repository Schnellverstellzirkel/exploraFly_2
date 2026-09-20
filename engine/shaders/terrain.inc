// Periodic alpine world. Mirrored by crates/world/src/lib.rs for CPU clearance.
// The origin is reduced before adding local coordinates, preserving float phase.
const float TERRAIN_PERIOD = 65536.0;
const float TERRAIN_WATER = 185.0;
const uint TERRAIN_CELLS = 1024u;
const float TERRAIN_CELL_METRES = 64.0;
const uint TERRAIN_VERTICES = (TERRAIN_CELLS + 1u) * (TERRAIN_CELLS + 1u);
const uint TERRAIN_STRUCTURES = 39u;
const uint LANDMARK_VERTEX_COUNT = 9u * TERRAIN_STRUCTURES * 54u;

// Procedural scatter grid (trees, boulders), mirrored by
// crates/world/src/lib.rs. 121×121 slots at a 24 m pitch span ~2.9 km around
// the camera; each slot owns 48 corners decoded in ground.vert.
const uint SCATTER_GRID = 61u;
const float SCATTER_PITCH = 48.0;
const uint SCATTER_CORNERS = 48u;
const uint SCATTER_VERTEX_COUNT = SCATTER_GRID * SCATTER_GRID * SCATTER_CORNERS;

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
    if (index <= 21u) {
        uint house = index - 9u;
        wallHeight = 18.0 + float(house % 3u) * 5.0; roofHeight = 15.0;
        return vec4(-260.0 + float(house % 5u) * 64.0,
            -220.0 - float(house / 5u) * 68.0, 16.0, 13.0);
    }
    // Chapel: narrow tall nave with a steep roof, west of the house grid.
    if (index == 22u) {
        wallHeight = 26.0; roofHeight = 24.0;
        return vec4(-388.0, -254.0, 11.0, 17.0);
    }
    // Barn: wide and low with a long shallow roof.
    if (index == 23u) {
        wallHeight = 12.0; roofHeight = 13.0;
        return vec4(-388.0, -364.0, 24.0, 15.0);
    }
    // Watermill tower at the village stream side.
    if (index == 24u) {
        wallHeight = 32.0; roofHeight = 9.0;
        return vec4(96.0, -430.0, 11.0, 11.0);
    }
    // Well house on the square between keep and houses.
    if (index == 25u) {
        wallHeight = 3.5; roofHeight = 4.0;
        return vec4(34.0, -166.0, 4.5, 4.5);
    }
    // Tavern: the largest house, facing the keep across the square.
    if (index == 26u) {
        wallHeight = 20.0; roofHeight = 17.0;
        return vec4(-196.0, -158.0, 22.0, 16.0);
    }
    // Granary at the village's west edge.
    if (index == 27u) {
        wallHeight = 9.0; roofHeight = 10.0;
        return vec4(-452.0, -160.0, 13.0, 10.0);
    }
    // Six standing stones on the western meadow, heights cycling 8/11/14.
    if (index <= 33u) {
        uint k = index - 28u;
        float angle = float(k) * (6.283185307179586 / 6.0);
        wallHeight = 8.0 + float(k % 3u) * 3.0; roofHeight = 1.2;
        return vec4(-560.0 + 44.0 * cos(angle), 140.0 + 44.0 * sin(angle), 2.4, 2.4);
    }
    // Watchtower on the eastern valley shoulder.
    if (index == 34u) {
        wallHeight = 58.0; roofHeight = 14.0;
        return vec4(640.0, -220.0, 12.0, 12.0);
    }
    // Ruined tower: tall walls with a collapsed roof, on the ridge foot.
    if (index == 35u) {
        wallHeight = 44.0; roofHeight = 2.5;
        return vec4(720.0, 300.0, 9.0, 9.0);
    }
    // Windmill on the western shoulder above the stone circle.
    if (index == 36u) {
        wallHeight = 34.0; roofHeight = 11.0;
        return vec4(-760.0, -120.0, 10.0, 10.0);
    }
    // Mountain shrine on the south-western shoulder.
    if (index == 37u) {
        wallHeight = 9.0; roofHeight = 9.0;
        return vec4(-700.0, 320.0, 5.0, 5.0);
    }
    // Lakeside stilt hut: over the lake the foundation clamps to the water
    // level and the box rises straight out of the water.
    wallHeight = 7.0; roofHeight = 8.0;
    return vec4(560.0, 1350.0, 12.0, 9.0);
}

