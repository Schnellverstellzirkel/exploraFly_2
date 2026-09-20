//! Deterministic vegetation rules shared by scatter placement, collision
//! clearance, and the terrain material shader.
//!
//! Every function here has an exact GLSL mirror in
//! `engine/shaders/vegetation.inc`. Changes to thresholds, smoothstep
//! ranges, or species boundaries must be applied to both files.

use super::{smooth, mix, WATER_LEVEL};

// ── Treeline ────────────────────────────────────────────────────────────

/// Altitude above which forest cover fades to alpine rock and shrubs.
/// Wetter slopes sustain trees higher; drier ridges lose them sooner.
#[inline]
pub fn treeline(moisture: f32) -> f32 {
    1500.0 + (moisture - 0.5) * 320.0
}

// ── Tree placement probability ──────────────────────────────────────────

/// Probability that a scatter slot holds a tree or boulder, given the
/// terrain-cache sample at that slot's jittered world position. The
/// caller supplies `altitude` (raw land height from the cache, not
/// water-clamped), `slope` (1 − normal.y), and `moisture` (baked
/// landform wetness in 0..1).
///
/// This mirrors the biome gate in `ground.vert` line-for-line: the same
/// smoothstep edges, the same weighting, and the same clamp to \[0, 1\].
#[inline]
pub fn tree_presence_probability(altitude: f32, slope: f32, moisture: f32) -> f32 {
    let forest = smooth(0.34, 0.52, moisture);
    let tl = treeline(moisture);
    let above_water = smooth(WATER_LEVEL + 1.5, WATER_LEVEL + 3.0, altitude);
    let below_treeline = 1.0 - smooth(tl - 40.0, tl + 60.0, altitude);
    (0.06 + forest * 0.92 + smooth(0.10, 0.16, slope) * 0.12)
        * above_water
        * below_treeline
}

// ── Species ─────────────────────────────────────────────────────────────

/// Species constants, matching the `kind` float packed into `vMoisture`
/// in `ground.vert` and consumed by `ground.frag`.
pub const SPECIES_SPRUCE: u8 = 0;
pub const SPECIES_BROADLEAF: u8 = 1;
pub const SPECIES_LARCH: u8 = 2;
pub const SPECIES_STONE_PINE: u8 = 3;
pub const SPECIES_ALPENROSE: u8 = 4;

/// Select the tree species for a surviving scatter slot. `altitude` is
/// the raw land height, `moisture` the baked landform wetness, and
/// `species_hash` the deterministic hash(hx, hz, 431) in 0..1.
///
/// The `forest` parameter is `smooth(0.34, 0.52, moisture)` — the same
/// value used in [`tree_presence_probability`]. Passing it in avoids
/// recomputing it when both are needed.
#[inline]
pub fn tree_species(altitude: f32, species_hash: f32, forest: f32) -> u8 {
    if altitude > 1550.0 && species_hash < 0.32 {
        SPECIES_ALPENROSE
    } else if altitude > 1350.0 {
        if species_hash > 0.48 { SPECIES_LARCH } else { SPECIES_STONE_PINE }
    } else if species_hash > 0.88 {
        SPECIES_LARCH // Autumn Alpine Larch in valley
    } else if species_hash < mix(0.30, 0.72, forest) {
        SPECIES_SPRUCE
    } else {
        SPECIES_BROADLEAF
    }
}

/// Whether a slot with the given slope and species hash places a boulder
/// instead of a tree. Boulder slots never carry a companion tree.
#[inline]
pub fn is_boulder(slope: f32, species_hash: f32) -> bool {
    slope > 0.13 && species_hash > 0.40
}

// ── Visual forest density (terrain albedo) ──────────────────────────────

/// Continuous forest-canopy density for the terrain material shader's
/// green-tinting mix. This is deliberately smoother and broader than
/// [`tree_presence_probability`]: the albedo represents the aggregate
/// canopy seen from altitude, not individual tree placement.
///
/// Mirrors `ground.frag` L731–736 exactly.
#[inline]
pub fn forest_cover(altitude: f32, slope: f32, moisture: f32) -> f32 {
    let tl = treeline(moisture);
    smooth(0.40, 0.58, moisture)
        * smooth(300.0, 520.0, altitude)
        * (1.0 - smooth(tl, tl + 170.0, altitude))
        * (1.0 - smooth(0.55, 0.90, slope))
}

// ── Tree geometry sizing ────────────────────────────────────────────────

/// Crown radius and apex height for collision and LOD sizing, indexed by
/// species. Returns `(radius, apex_height)` as multiples of `size`.
#[inline]
pub fn tree_crown_envelope(species: u8, size: f32, _size_r: f32) -> (f32, f32) {
    match species {
        0 => (2.9 * size, 26.0 * size),          // Spruce
        1 => (7.0 * size, 21.2 * size),           // Broadleaf
        2 => (4.8 * size, 24.0 * size),            // Larch
        3 => (3.8 * size, 20.3 * size),            // Stone Pine
        4 => (3.2 * size, 2.8 * size),             // Alpenrose
        _ => unreachable!("invalid species {species}"),
    }
}

/// Crown radius and apex height for a boulder slot.
#[inline]
pub fn boulder_envelope(size: f32, size_r: f32) -> (f32, f32) {
    ((1.6 + 2.4 * size_r) * 1.25, 5.88 * size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn treeline_rises_with_moisture() {
        assert!(treeline(0.8) > treeline(0.2));
    }

    #[test]
    fn presence_zero_underwater() {
        assert!(tree_presence_probability(100.0, 0.0, 0.5) < 0.001);
    }

    #[test]
    fn presence_zero_above_treeline() {
        assert!(tree_presence_probability(2500.0, 0.0, 0.5) < 0.001);
    }

    #[test]
    fn presence_positive_in_moist_valley() {
        assert!(tree_presence_probability(400.0, 0.05, 0.7) > 0.5);
    }

    #[test]
    fn species_spruce_dominates_low_forest() {
        // species_hash < mix(0.30, 0.72, forest) where forest ≈ 1.0 → threshold 0.72
        assert_eq!(tree_species(400.0, 0.10, 1.0), SPECIES_SPRUCE);
    }

    #[test]
    fn species_alpenrose_above_1550() {
        assert_eq!(tree_species(1600.0, 0.20, 0.5), SPECIES_ALPENROSE);
    }

    #[test]
    fn species_larch_at_high_altitude() {
        assert_eq!(tree_species(1400.0, 0.60, 0.5), SPECIES_LARCH);
    }

    #[test]
    fn boulder_on_steep_slope() {
        assert!(is_boulder(0.20, 0.50));
        assert!(!is_boulder(0.10, 0.50));
        assert!(!is_boulder(0.20, 0.30));
    }

    #[test]
    fn forest_cover_positive_in_moist_midslopes() {
        assert!(forest_cover(600.0, 0.15, 0.7) > 0.3);
    }

    #[test]
    fn forest_cover_zero_on_dry_ridges() {
        assert!(forest_cover(2000.0, 0.7, 0.2) < 0.01);
    }
}
