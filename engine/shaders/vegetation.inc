// Deterministic vegetation rules: GLSL mirror of crates/world/src/vegetation.rs.
// Changes to thresholds, smoothstep ranges, or species boundaries must be
// applied to both files.  This file is injected by build.rs after the
// terrain.inc constants, so TERRAIN_WATER is already defined.

// ── Treeline ────────────────────────────────────────────────────────────

float vegetationTreeline(float moisture) {
    return 1500.0 + (moisture - 0.5) * 320.0;
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
