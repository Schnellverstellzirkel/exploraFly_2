//! Deterministic vegetation rules shared by scatter placement, collision
//! clearance, and the terrain material shader.
//!
//! Every function here has an exact GLSL mirror in
//! `engine/shaders/vegetation.inc`. Changes to thresholds, smoothstep
//! ranges, or species boundaries must be applied to both files.

use super::{SCATTER_PITCH, WATER_LEVEL, WORLD_PERIOD, hash, mix, noise, smooth};

// ── Treeline ────────────────────────────────────────────────────────────

/// Altitude above which forest cover fades to alpine rock and shrubs.
/// Wetter slopes sustain trees higher; drier ridges lose them sooner.
#[inline]
pub fn treeline(moisture: f32) -> f32 {
    1500.0 + (moisture - 0.5) * 320.0
}

/// Correlated forest-patch multiplier for individual candidates. The broad
/// octave creates contiguous stands while the smaller octave breaks their
/// edges up; both use the same periodic terrain noise as the height field.
/// The result never reaches zero, so a meadow can still contain an occasional
/// isolated tree and collision remains useful at patch boundaries.
#[inline]
pub fn forest_patch(x: f32, z: f32) -> f32 {
    let p = [
        x.rem_euclid(WORLD_PERIOD as f32),
        z.rem_euclid(WORLD_PERIOD as f32),
    ];
    let broad = noise(p, 1024.0, 509);
    let edge = noise(
        [
            (p[0] + 173.0).rem_euclid(WORLD_PERIOD as f32),
            (p[1] - 91.0).rem_euclid(WORLD_PERIOD as f32),
        ],
        256.0,
        521,
    );
    let field = broad * 0.65 + edge * 0.35;
    0.35 + 0.65 * smooth(0.28, 0.72, field)
}

/// Candidate offset inside one scatter slot. A hashed permutation of a 2×2
/// block keeps adjacent candidates separated (jitter is only 16% of the
/// pitch), which gives the slot lattice a blue-noise-like minimum distance
/// without changing its deterministic world-coordinate addressing.
#[inline]
pub fn candidate_offset(cx: i64, cz: i64) -> [f32; 2] {
    // Derive both the slot hash and its 2×2 block from the same wrapped
    // coordinates. Computing a signed block first would make slot -1 differ
    // from slot 65535 even though all existing scatter hashes agree there.
    let hx = (cx as u32) & 65535;
    let hz = (cz as u32) & 65535;
    let block_x = hx / 2;
    let block_z = hz / 2;
    let pattern = (hash(block_x, block_z, 503) * 8.0) as u32;
    let mut sx = if hx & 1 == 0 { -1.0 } else { 1.0 };
    let mut sz = if hz & 1 == 0 { -1.0 } else { 1.0 };
    if pattern & 1 != 0 {
        sx = -sx;
    }
    if pattern & 2 != 0 {
        sz = -sz;
    }
    if pattern & 4 != 0 {
        std::mem::swap(&mut sx, &mut sz);
    }
    let jitter_x = (hash(hx, hz, 401) - 0.5) * 0.16 * SCATTER_PITCH;
    let jitter_z = (hash(hx, hz, 407) - 0.5) * 0.16 * SCATTER_PITCH;
    [
        sx * 0.25 * SCATTER_PITCH + jitter_x,
        sz * 0.25 * SCATTER_PITCH + jitter_z,
    ]
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
    (0.06 + forest * 0.92 + smooth(0.10, 0.16, slope) * 0.12) * above_water * below_treeline
}

/// Placement probability including the spatial forest-patch field.
#[inline]
pub fn tree_presence_probability_at(
    altitude: f32,
    slope: f32,
    moisture: f32,
    x: f32,
    z: f32,
) -> f32 {
    tree_presence_probability(altitude, slope, moisture) * forest_patch(x, z)
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
        if species_hash > 0.48 {
            SPECIES_LARCH
        } else {
            SPECIES_STONE_PINE
        }
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
/// Mirrors `ground.frag`'s continuous canopy recipe. Individual tree patches
/// deliberately remain a separate multiplier so the far material can stay
/// smooth while tree silhouettes cluster.
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
        0 => (2.9 * size, 26.0 * size), // Spruce
        1 => (7.0 * size, 21.2 * size), // Broadleaf
        2 => (4.8 * size, 24.0 * size), // Larch
        3 => (3.8 * size, 20.3 * size), // Stone Pine
        4 => (3.2 * size, 2.8 * size),  // Alpenrose
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

    #[test]
    fn forest_patch_is_periodic_and_bounded() {
        let a = forest_patch(123.0, 456.0);
        let b = forest_patch(123.0 + WORLD_PERIOD as f32, 456.0 - WORLD_PERIOD as f32);
        assert!((a - b).abs() < 1e-5);
        assert!((0.35..=1.0).contains(&a));
    }

    #[test]
    fn candidate_offsets_keep_neighbouring_slots_apart() {
        let mut minimum = f32::MAX;
        for z in -8..=8 {
            for x in -8..=8 {
                let a = candidate_offset(x, z);
                for (dx, dz) in [(1, 0), (0, 1), (1, 1), (1, -1)] {
                    let b = candidate_offset(x + dx, z + dz);
                    let delta = [
                        (dx as f32 * SCATTER_PITCH + b[0] - a[0]),
                        (dz as f32 * SCATTER_PITCH + b[1] - a[1]),
                    ];
                    minimum = minimum.min(delta[0].hypot(delta[1]));
                }
            }
        }
        assert!(
            minimum > SCATTER_PITCH * 0.25,
            "minimum separation {minimum}"
        );
    }
}
