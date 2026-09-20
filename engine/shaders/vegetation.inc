// Deterministic vegetation rules: GLSL mirror of crates/world/src/vegetation.rs.
// Changes to thresholds, smoothstep ranges, or species boundaries must be
// applied to both files.  This file is injected by build.rs after the
// terrain.inc constants, so TERRAIN_WATER is already defined.

// ── Treeline ────────────────────────────────────────────────────────────

float vegetationTreeline(float moisture) {
    return 1500.0 + (moisture - 0.5) * 320.0;
}

// Correlated stand field. terrainNoise is the same periodic value-noise
// implementation used by terrain.inc, so CPU placement and raster placement
// agree across the world boundary.
// Both raster LOD stages use the same world-stable hash for dither coverage;
// their explicit ranges overlap so transitions remain stable across cell and
// floating-origin boundaries.
const float VEGETATION_TREE_FADE_START = 3000.0;
const float VEGETATION_TREE_FADE_END = 5000.0;
const float VEGETATION_CANOPY_FADE_START = 4500.0;
const float VEGETATION_CANOPY_FADE_FAR_END = 6500.0;
const float VEGETATION_CANOPY_MAX_HEIGHT = 64.0;

float vegetationForestPatch(vec2 absoluteXZ) {
    vec2 p = mod(absoluteXZ, vec2(TERRAIN_PERIOD));
    float broad = terrainNoise(p, 1024.0, 509u);
    float edge = terrainNoise(p + vec2(173.0, -91.0), 256.0, 521u);
    float field = broad * 0.65 + edge * 0.35;
    return 0.35 + 0.65 * smoothstep(0.28, 0.72, field);
}

// Hashed permutations of a 2x2 slot block produce a blue-noise-like minimum
// distance. The small residual jitter keeps the field from reading as a grid.
vec2 vegetationCandidateOffset(ivec2 slot) {
    uvec2 slotHash = uvec2(slot) & uvec2(65535u);
    // Hash the wrapped slot's block, not a signed block computed before the
    // 16-bit wrap. This keeps slot -1 identical to slot 65535.
    uvec2 blockHash = slotHash / uvec2(2u);
    uint pattern = uint(terrainHash(blockHash, 503u) * 8.0);
    vec2 sign = vec2(slotHash & uvec2(1u)) * 2.0 - 1.0;
    if ((pattern & 1u) != 0u) sign.x = -sign.x;
    if ((pattern & 2u) != 0u) sign.y = -sign.y;
    if ((pattern & 4u) != 0u) sign = sign.yx;
    vec2 jitter = vec2(terrainHash(slotHash, 401u), terrainHash(slotHash, 407u)) - 0.5;
    return sign * (0.25 * SCATTER_PITCH) + jitter * (0.16 * SCATTER_PITCH);
}

// ── Tree placement probability ──────────────────────────────────────────
// Mirrors vegetation::tree_presence_probability exactly.

float vegetationPresenceProbability(float altitude, float slope, float moisture) {
    float forest = smoothstep(0.34, 0.52, moisture);
    float tl = vegetationTreeline(moisture);
    float aboveWater = smoothstep(TERRAIN_WATER + 1.5, TERRAIN_WATER + 3.0, altitude);
    float belowTreeline = 1.0 - smoothstep(tl - 40.0, tl + 60.0, altitude);
    return (0.06 + forest * 0.92 + smoothstep(0.10, 0.16, slope) * 0.12)
        * aboveWater * belowTreeline;
}

float vegetationPresenceProbabilityAt(
    float altitude, float slope, float moisture, vec2 absoluteXZ
) {
    return vegetationPresenceProbability(altitude, slope, moisture)
        * vegetationForestPatch(absoluteXZ);
}

// ── Species ─────────────────────────────────────────────────────────────
// Returns the species index as a float matching the vertex-stage encoding:
//   0 = Spruce, 1 = Broadleaf, 2 = Larch, 3 = Stone Pine, 4 = Alpenrose.

float vegetationSpecies(float altitude, float speciesHash, float forest) {
    if (altitude > 1550.0 && speciesHash < 0.32) return 4.0;
    if (altitude > 1350.0) return (speciesHash > 0.48) ? 2.0 : 3.0;
    if (speciesHash > 0.88) return 2.0;
    if (speciesHash < mix(0.30, 0.72, forest)) return 0.0;
    return 1.0;
}

bool vegetationIsBoulder(float slope, float speciesHash) {
    return slope > 0.13 && speciesHash > 0.40;
}

// ── Visual forest density (terrain albedo) ──────────────────────────────
// Mirrors vegetation::forest_cover.  Used by ground.frag for the forest
// canopy green-tinting mix.

float vegetationForestCover(float altitude, float slope, float moisture) {
    float tl = vegetationTreeline(moisture);
    return smoothstep(0.40, 0.58, moisture)
        * smoothstep(300.0, 520.0, altitude)
        * (1.0 - smoothstep(tl, tl + 170.0, altitude))
        * (1.0 - smoothstep(0.55, 0.90, slope));
}
