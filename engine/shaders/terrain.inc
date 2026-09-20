// Periodic alpine world. Mirrored by crates/world/src/lib.rs for CPU clearance.
// The origin is reduced before adding local coordinates, preserving float phase.
const float TERRAIN_PERIOD = 65536.0;
const float TERRAIN_WATER = 185.0;
const uint TERRAIN_CELLS = 1024u;
const float TERRAIN_CELL_METRES = 64.0;
const uint TERRAIN_VERTICES = (TERRAIN_CELLS + 1u) * (TERRAIN_CELLS + 1u);
const uint TERRAIN_STRUCTURES = 52u;
// Per-structure corner budget, mirrored by crates/world/src/lib.rs.
const uint STRUCTURE_WALL_VERTICES = 36u;
const uint STRUCTURE_ROOF_VERTICES = 18u;
const uint STRUCTURE_DETAIL_VERTICES = 36u;
const uint STRUCTURE_BAND_VERTICES = 36u;
const uint STRUCTURE_TIP_VERTICES = 24u;
const uint STRUCTURE_VERTICES = STRUCTURE_WALL_VERTICES + STRUCTURE_ROOF_VERTICES
    + STRUCTURE_DETAIL_VERTICES + STRUCTURE_DETAIL_VERTICES
    + STRUCTURE_DETAIL_VERTICES + STRUCTURE_BAND_VERTICES
    + STRUCTURE_TIP_VERTICES;
const uint LANDMARK_VERTEX_COUNT = 9u * TERRAIN_STRUCTURES * STRUCTURE_VERTICES;

// Procedural scatter grid (trees, boulders), mirrored by
// crates/world/src/lib.rs. 121×121 slots at a 24 m pitch span ~2.9 km around
// the camera; each slot owns 108 corners: a trunk or boulder box (36) and
// three stacked crown octahedra (24 each), decoded in ground.vert.
const uint SCATTER_GRID = 121u;
const float SCATTER_PITCH = 24.0;
const uint SCATTER_CORNERS = 216u;
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
    if (index == 38u) {
        wallHeight = 7.0; roofHeight = 8.0;
        return vec4(560.0, 1350.0, 12.0, 9.0);
    }
    // High Alpine Cloister: mountain monastery on eastern promontory.
    if (index == 39u) {
        wallHeight = 28.0; roofHeight = 18.0;
        return vec4(740.0, -680.0, 14.0, 22.0);
    }
    // Alpine Almhütte: high pasture herder chalet with low stone roof.
    if (index == 40u) {
        wallHeight = 8.5; roofHeight = 6.0;
        return vec4(-480.0, 540.0, 11.0, 8.5);
    }
    // Valley Arched Stone Bridge across the stream.
    if (index == 41u) {
        wallHeight = 14.0; roofHeight = 1.5;
        return vec4(180.0, -340.0, 5.0, 28.0);
    }
    // Mountain Signal Fire / Beacon Tower on western ridge.
    if (index == 42u) {
        wallHeight = 46.0; roofHeight = 5.0;
        return vec4(-740.0, -460.0, 7.5, 7.5);
    }
    // Meadow Hay Barn (Heustadel).
    if (index == 43u) {
        wallHeight = 9.0; roofHeight = 9.0;
        return vec4(360.0, -580.0, 12.0, 9.0);
    }
    // Wayside Shrine (Bildstock) at path fork.
    if (index == 44u) {
        wallHeight = 5.0; roofHeight = 3.0;
        return vec4(-110.0, -310.0, 1.8, 1.8);
    }
    // Castle Barbican / Gatehouse with flanking defense turrets.
    if (index == 45u) {
        wallHeight = 36.0; roofHeight = 8.0;
        return vec4(0.0, -76.0, 18.0, 11.0);
    }
    // High Pass Watch-Post.
    if (index == 46u) {
        wallHeight = 18.0; roofHeight = 12.0;
        return vec4(710.0, 780.0, 11.0, 11.0);
    }
    // Lakeside Boat House & Timber Pier.
    if (index == 47u) {
        wallHeight = 7.5; roofHeight = 7.0;
        return vec4(480.0, 1200.0, 15.0, 10.0);
    }
    // High Summit Cross & Beacon on highest horn.
    if (index == 48u) {
        wallHeight = 8.0; roofHeight = 2.0;
        return vec4(680.0, -880.0, 4.0, 4.0);
    }
    // Cliffside Hermitage on western precipice.
    if (index == 49u) {
        wallHeight = 16.0; roofHeight = 7.0;
        return vec4(-620.0, 240.0, 9.0, 11.0);
    }
    // Avalanche Shelter Gallery on eastern pass.
    if (index == 50u) {
        wallHeight = 14.0; roofHeight = 4.0;
        return vec4(420.0, -120.0, 7.0, 28.0);
    }
    // Alpine Sawmill & Log Flume on mountain stream.
    wallHeight = 9.0; roofHeight = 8.0;
    return vec4(220.0, -480.0, 11.0, 14.0);
}

// Roof add-on boxes, mirroring crates/world/src/lib.rs detail_a/b/c/d.
// Returns (dx, dz, half_x, half_z); absent add-ons collapse (yBase=0, height=0).
vec4 terrainDetailA(uint index, float wallHeight, float roofHeight,
        out float yBase, out float height) {
    float apex = wallHeight + roofHeight;
    if (index == 0u) {
        yBase = apex - 6.0; height = 10.0; return vec4(0.0, 0.0, 7.0, 7.0);
    }
    if (index <= 4u) {
        yBase = wallHeight - 11.0; height = 7.0; return vec4(0.0, 0.0, 17.0, 17.0);
    }
    if (index >= 9u && index <= 21u) {
        float house = float(index - 9u);
        float side = (mod(house, 2.0) > 0.5) ? -1.0 : 1.0;
        yBase = apex - 4.0; height = 6.0;
        return vec4(side * 13.0, 0.0, 1.1, 1.5);
    }
    if (index == 22u) {
        yBase = apex - 12.0; height = 16.0; return vec4(0.0, -8.0, 4.4, 4.4);
    }
    if (index == 23u) {
        yBase = apex - 10.0; height = 12.0; return vec4(0.0, 0.0, 4.2, 6.0);
    }
    if (index == 24u) {
        yBase = wallHeight - 2.0; height = 10.0; return vec4(0.0, 0.0, 2.2, 2.2);
    }
    if (index == 26u) {
        yBase = apex - 4.0; height = 6.0; return vec4(-18.0, 0.0, 1.1, 1.5);
    }
    if (index == 27u) {
        yBase = 12.0; height = 6.0; return vec4(0.0, 0.0, 3.4, 4.0);
    }
    if (index == 34u) {
        yBase = 50.0; height = 5.0; return vec4(0.0, 0.0, 13.5, 13.5);
    }
    if (index == 35u) {
        yBase = 36.0; height = 8.0; return vec4(3.0, -3.0, 2.6, 2.6);
    }
    if (index == 36u) {
        yBase = 38.0; height = 8.0; return vec4(0.0, 0.0, 5.6, 5.6);
    }
    if (index == 37u) {
        yBase = 11.0; height = 4.0; return vec4(0.0, 0.0, 3.2, 3.2);
    }
    if (index == 38u) {
        yBase = apex - 2.0; height = 3.0; return vec4(0.0, 0.0, 1.3, 1.3);
    }
    if (index == 39u) {
        yBase = apex - 6.0; height = 18.0; return vec4(0.0, -10.0, 4.0, 4.0);
    }
    if (index == 40u) {
        yBase = apex - 2.0; height = 5.0; return vec4(7.0, 0.0, 1.4, 1.6);
    }
    if (index == 41u) {
        yBase = 0.0; height = 15.0; return vec4(0.0, -26.0, 6.5, 4.0);
    }
    if (index == 42u) {
        yBase = apex - 1.0; height = 6.0; return vec4(0.0, 0.0, 3.2, 3.2);
    }
    if (index == 43u) {
        yBase = 8.0; height = 4.0; return vec4(0.0, 8.5, 2.5, 0.8);
    }
    if (index == 44u) {
        yBase = 2.0; height = 2.2; return vec4(0.0, 0.0, 1.5, 1.5);
    }
    if (index == 45u) {
        yBase = 0.0; height = 14.0; return vec4(0.0, 0.0, 5.0, 12.0);
    }
    if (index == 46u) {
        yBase = 16.0; height = 14.0; return vec4(6.0, 6.0, 4.0, 4.0);
    }
    if (index == 47u) {
        yBase = 0.0; height = 6.0; return vec4(0.0, 9.0, 6.0, 2.0);
    }
    if (index == 48u) {
        yBase = 12.0; height = 1.6; return vec4(0.0, 0.0, 4.5, 1.0);
    }
    if (index == 49u) {
        yBase = 8.0; height = 6.0; return vec4(6.0, 0.0, 2.5, 8.0);
    }
    if (index == 50u) {
        yBase = 0.0; height = 18.0; return vec4(5.5, 0.0, 2.0, 30.0);
    }
    if (index == 51u) {
        yBase = 6.5; height = 3.0; return vec4(0.0, -16.0, 2.2, 8.0);
    }
    yBase = 0.0; height = 0.0; return vec4(0.0);
}

vec4 terrainDetailB(uint index, float wallHeight, float roofHeight,
        out float yBase, out float height) {
    float apex = wallHeight + roofHeight;
    if (index >= 9u && index <= 21u && (index - 9u) % 3u == 0u) {
        yBase = 0.0; height = 7.5; return vec4(1.6, 13.6, 3.2, 1.7);
    }
    if (index == 26u) {
        yBase = apex - 4.0; height = 6.0; return vec4(18.0, 0.0, 1.2, 1.6);
    }
    if (index == 24u) {
        yBase = 1.0; height = 7.0; return vec4(0.0, 11.6, 5.6, 0.6);
    }
    if (index == 35u) {
        yBase = 40.0; height = 6.0; return vec4(-4.0, 4.0, 2.2, 2.2);
    }
    if (index == 39u) {
        yBase = 0.0; height = 9.0; return vec4(-16.0, 0.0, 3.5, 18.0);
    }
    if (index == 40u) {
        yBase = 0.0; height = 6.5; return vec4(-12.0, 0.0, 2.5, 7.0);
    }
    if (index == 41u) {
        yBase = 0.0; height = 15.0; return vec4(0.0, 26.0, 6.5, 4.0);
    }
    if (index == 42u) {
        yBase = 0.0; height = 12.0; return vec4(0.0, 0.0, 10.5, 10.5);
    }
    if (index == 43u) {
        yBase = 0.0; height = 1.5; return vec4(0.0, 0.0, 12.5, 9.5);
    }
    if (index == 44u) {
        yBase = 0.0; height = 1.0; return vec4(0.0, 0.0, 2.8, 2.8);
    }
    if (index == 45u) {
        yBase = 0.0; height = 44.0; return vec4(-16.0, -2.0, 5.0, 5.0);
    }
    if (index == 46u) {
        yBase = 0.0; height = 3.2; return vec4(0.0, 0.0, 16.0, 16.0);
    }
    if (index == 47u) {
        yBase = -3.0; height = 3.5; return vec4(0.0, 0.0, 16.0, 11.0);
    }
    if (index == 48u) {
        yBase = 12.0; height = 1.6; return vec4(0.0, 0.0, 1.0, 4.5);
    }
    if (index == 49u) {
        yBase = 16.0; height = 12.0; return vec4(-4.0, -6.0, 2.5, 2.5);
    }
    if (index == 50u) {
        yBase = 0.0; height = 12.0; return vec4(-5.5, 0.0, 1.8, 30.0);
    }
    if (index == 51u) {
        yBase = 0.0; height = 6.5; return vec4(11.0, 0.0, 3.0, 5.0);
    }
    yBase = 0.0; height = 0.0; return vec4(0.0);
}

vec4 terrainDetailC(uint index, float wallHeight, float roofHeight,
        out float yBase, out float height) {
    float apex = wallHeight + roofHeight;
    if (index == 0u) {
        yBase = 62.0; height = 36.0; return vec4(0.0, 0.0, 30.0, 22.0);
    }
    if (index <= 4u) {
        yBase = 88.0; height = 40.0; return vec4(0.0, 0.0, 12.0, 12.0);
    }
    if (index <= 8u) {
        yBase = 0.0; height = wallHeight + 2.0; return vec4(0.0, 0.0, 7.6, 7.6);
    }
    if (index >= 9u && index <= 21u) {
        float house = float(index - 9u);
        float wh = 18.0 + mod(house, 3.0) * 5.0;
        yBase = wh * 0.52; height = wh * 0.44;
        return vec4(0.0, 0.0, 16.6, 13.6);
    }
    if (index == 22u) {
        yBase = 0.0; height = 13.0; return vec4(-13.0, 0.0, 2.6, 7.0);
    }
    if (index == 23u) {
        yBase = 0.0; height = 5.0; return vec4(0.0, -15.7, 24.7, 3.2);
    }
    if (index == 24u) {
        yBase = 18.0; height = 1.6; return vec4(0.0, 0.0, 13.0, 13.0);
    }
    if (index == 25u) {
        yBase = 0.0; height = 0.9; return vec4(0.0, 0.0, 5.0, 5.0);
    }
    if (index == 26u) {
        yBase = 9.0; height = 11.0; return vec4(0.0, 0.0, 20.0, 15.0);
    }
    if (index == 27u) {
        yBase = 1.2; height = 1.5; return vec4(0.0, 0.0, 13.5, 10.5);
    }
    if (index == 35u) {
        yBase = 0.0; height = 3.5; return vec4(6.0, 4.0, 11.0, 9.0);
    }
    if (index == 36u) {
        yBase = 26.0; height = 1.6; return vec4(0.0, 0.0, 11.2, 11.2);
    }
    if (index == 37u) {
        yBase = 0.0; height = 2.0; return vec4(0.0, 0.0, 6.2, 6.2);
    }
    if (index == 38u) {
        yBase = 0.0; height = 1.1; return vec4(0.0, -13.0, 9.0, 4.0);
    }
    if (index == 39u) {
        yBase = 0.0; height = 4.5; return vec4(0.0, 0.0, 18.0, 26.0);
    }
    if (index == 40u) {
        yBase = 3.2; height = 4.5; return vec4(0.0, -9.2, 9.5, 1.8);
    }
    if (index == 41u) {
        yBase = 0.0; height = 10.0; return vec4(0.0, 0.0, 7.0, 6.0);
    }
    if (index == 42u) {
        yBase = 38.0; height = 2.0; return vec4(0.0, 0.0, 9.2, 9.2);
    }
    if (index == 43u) {
        yBase = 0.0; height = 4.0; return vec4(0.0, -10.0, 4.0, 3.0);
    }
    if (index == 44u) {
        yBase = 0.0; height = 0.4; return vec4(0.0, 0.0, 3.8, 3.8);
    }
    if (index == 45u) {
        yBase = 0.0; height = 44.0; return vec4(16.0, -2.0, 5.0, 5.0);
    }
    if (index == 46u) {
        yBase = 0.0; height = 8.0; return vec4(-12.0, 0.0, 3.5, 9.0);
    }
    if (index == 47u) {
        yBase = 0.0; height = 1.0; return vec4(-16.0, 4.0, 3.0, 14.0);
    }
    if (index == 48u) {
        yBase = 0.0; height = 3.5; return vec4(0.0, 0.0, 6.5, 6.5);
    }
    if (index == 49u) {
        yBase = 0.0; height = 12.0; return vec4(-7.0, 0.0, 3.0, 10.0);
    }
    if (index == 50u) {
        yBase = 0.0; height = 2.2; return vec4(0.0, 0.0, 4.5, 32.0);
    }
    if (index == 51u) {
        yBase = 0.0; height = 5.5; return vec4(-10.0, 4.0, 3.5, 6.5);
    }
    yBase = 0.0; height = 0.0; return vec4(0.0);
}

vec4 terrainDetailD(uint index, float wallHeight, float roofHeight,
        out float yBase, out float height) {
    if ((index >= 28u && index <= 33u) || index == 35u || index == 41u || index == 44u) {
        yBase = 0.0; height = 0.0; return vec4(0.0);
    }
    vec4 shape = terrainStructure(index, wallHeight, roofHeight);
    if (index <= 8u || index == 34u || index == 42u || index == 45u || index == 46u || index == 48u || index == 50u) {
        yBase = wallHeight - 2.8; height = 3.0;
        return vec4(0.0, 0.0, shape.z + 0.6, shape.w + 0.6);
    }
    yBase = wallHeight - 1.8; height = 2.0;
    return vec4(0.0, 0.0, shape.z + 0.5, shape.w + 0.5);
}

// Roof tip: x = kind (1 spire, 2 windmill sails), yz = offset, w = half width;
// the half height is returned separately. Mirrors crates/world/src/lib.rs.
vec4 terrainTip(uint index, float wallHeight, float roofHeight, out float halfH) {
    float apex = wallHeight + roofHeight;
    if (index == 0u) { halfH = 14.0; return vec4(1.0, 0.0, apex + 4.0, 2.6); }
    if (index <= 4u) { halfH = 26.0; return vec4(1.0, 0.0, apex, 8.5); }
    if (index == 22u) { halfH = 12.0; return vec4(1.0, 0.0, apex + 4.0, 3.2); }
    if (index == 23u) { halfH = 8.0; return vec4(1.0, 0.0, apex + 2.0, 2.4); }
    if (index == 24u) { halfH = 7.0; return vec4(1.0, 0.0, apex, 5.6); }
    if (index == 25u) { halfH = 3.6; return vec4(1.0, 0.0, apex, 2.2); }
    if (index == 27u) { halfH = 4.0; return vec4(1.0, 0.0, apex, 1.8); }
    if (index == 34u) { halfH = 13.0; return vec4(1.0, 0.0, apex, 8.2); }
    if (index == 36u) { halfH = 0.35; return vec4(2.0, 0.0, 30.0, 12.0); }
    if (index == 37u) { halfH = 8.0; return vec4(1.0, 0.0, apex, 2.6); }
    if (index == 39u) { halfH = 16.0; return vec4(1.0, 0.0, apex + 6.0, 2.8); }
    if (index == 42u) { halfH = 4.5; return vec4(1.0, 0.0, apex, 2.2); }
    if (index == 43u) { halfH = 3.0; return vec4(1.0, 0.0, apex, 1.2); }
    if (index == 44u) { halfH = 2.5; return vec4(1.0, 0.0, apex, 0.8); }
    if (index == 45u) { halfH = 10.0; return vec4(1.0, -16.0, 44.0, 3.5); }
    if (index == 46u) { halfH = 7.0; return vec4(1.0, 6.0, 30.0, 2.5); }
    if (index == 48u) { halfH = 10.0; return vec4(1.0, 0.0, 8.0, 1.0); }
    if (index == 49u) { halfH = 4.0; return vec4(1.0, -4.0, 28.0, 2.0); }
    if (index == 51u) { halfH = 2.5; return vec4(1.0, 4.0, apex - 2.0, 1.2); }
    halfH = 0.0; return vec4(0.0);
}

