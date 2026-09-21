//! Deterministic vegetation rules shared by scatter placement, collision
//! clearance, and the terrain material shader.
//!
//! Every function here has an exact GLSL mirror in
//! `engine/shaders/vegetation.inc`. Changes to thresholds, smoothstep
//! ranges, or species boundaries must be applied to both files.

use super::{hash, mix, noise, smooth, SCATTER_PITCH, WATER_LEVEL, WORLD_PERIOD};

/// Spatial granularity of the persistent vegetation database. Cell metadata
/// is small enough to keep both CPU fallback culling and the GPU compaction
/// pass bounded while grouping the periodic field into cache-friendly ranges.
pub const VEGETATION_CELL_METRES: f32 = 128.0;
pub const VEGETATION_CELLS_PER_AXIS: u32 = 512;
pub const VEGETATION_CELL_COUNT: u32 = VEGETATION_CELLS_PER_AXIS * VEGETATION_CELLS_PER_AXIS;
pub const VEGETATION_INSTANCE_VERTICES: u32 = 108;
/// Compact crossed-plane tree used after the full crown falls below a few
/// pixels. The shader reserves the first 108 vertex indices for the full
/// crown and addresses this LOD from the indirect command's firstVertex.
pub const VEGETATION_MID_VERTICES: u32 = 12;
pub const VEGETATION_COMMAND_CAPACITY: u32 = 4096;
/// One four-by-four raised canopy surface forms one far-field cell. The
/// geometry is expanded procedurally by `canopy.vert`, so this remains a
/// fixed draw budget rather than a per-cell mesh allocation.
pub const CANOPY_INSTANCE_VERTICES: u32 = 54;
pub const CANOPY_COMMAND_CAPACITY: u32 = 16_384;
pub const CANOPY_MAX_HEIGHT: f32 = 64.0;
/// Extra near-field candidate density inside an occupied stand. The spatial
/// mask still owns the stand footprint; the thresholded cover ramp fills the
/// stand interior and the square-root edge remap reaches its soft boundary
/// without leaking trees into meadow cells before the aggregate HLOD appears.
pub const VEGETATION_NEAR_DENSITY_SCALE: f32 = 2.4;
/// Shared visible-stand mask thresholds. These values are mirrored in
/// `engine/shaders/vegetation.inc`; the cutoff is part of the mask rather
/// than a separate far-LOD discard so every representation has one footprint.
pub const VEGETATION_STAND_COVER_START: f32 = 0.10;
pub const VEGETATION_STAND_COVER_END: f32 = 0.18;
pub const VEGETATION_STAND_EDGE_START: f32 = 0.02;
pub const VEGETATION_STAND_EDGE_END: f32 = 0.06;
pub const VEGETATION_STAND_MIN_COVERAGE: f32 = 0.035;
/// Horizontal draw range for the near vegetation representation. The HLOD
/// canopy begins fading in before this edge so there is no hard forest cutoff.
pub const VEGETATION_RANGE_METRES: f32 = 5_000.0;
/// Conservative envelope used by GPU instance culling. The actual species
/// envelopes are smaller; these limits prevent false-negative culls.
pub const VEGETATION_MAX_RADIUS: f32 = 18.0;
pub const VEGETATION_MAX_HEIGHT: f32 = 64.0;
/// The GPU walks this many cells in each axis around the camera cell. The
/// extra cell keeps the 5 km range conservative at cell boundaries.
pub const VEGETATION_CULL_RADIUS_CELLS: u32 = 41;
pub const VEGETATION_CULL_DIAMETER: u32 = VEGETATION_CULL_RADIUS_CELLS * 2 + 1;
pub const VEGETATION_CULL_CELL_COUNT: u32 = VEGETATION_CULL_DIAMETER * VEGETATION_CULL_DIAMETER;
/// Enough headroom for the visible 5 km disk at the generated density while
/// keeping each per-frame device-local compaction buffer small (4 MiB).
pub const VEGETATION_VISIBLE_CAPACITY: u32 = 262_144;

/// One cell's contiguous range in [`VegetationDatabase::instances`]. Empty
/// cells retain a zero count and do not consume an indirect command.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VegetationCell {
    pub first_instance: u32,
    pub instance_count: u32,
}

/// Startup-generated, periodic vegetation field. Instances are four raw u32
/// words so the renderer can upload them as a tightly packed std430 `uvec4`:
/// x bits, z bits, ground-height bits, and packed scale/species/material data.
pub struct VegetationDatabase {
    pub instances: Vec<[u32; 4]>,
    pub cells: Vec<VegetationCell>,
    /// One packed aggregate field record per canonical 128 m cell. Zero
    /// density records are retained so `first_instance` can be the canonical
    /// cell index in the far indirect command stream.
    pub canopies: Vec<[u32; 4]>,
}

// ── Treeline ────────────────────────────────────────────────────────────

/// Altitude above which forest cover fades to alpine rock and shrubs.
/// Wetter slopes sustain trees higher; drier ridges lose them sooner.
#[inline]
pub fn treeline(moisture: f32) -> f32 {
    1500.0 + (moisture - 0.5) * 320.0
}

/// Correlated forest-patch mask shared by individual placement and the far
/// canopy field. The broad octave creates contiguous stands while the smaller
/// octave breaks their edges up; both use the same periodic terrain noise as
/// the height field. The zero floor is intentional: trees do not leak through
/// meadows where the aggregate forest field is absent.
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
    smooth(0.36, 0.68, field)
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
/// This is the same continuous forest-cover term used by the aggregate far
/// canopy. Keeping the near probability on this field makes silhouettes and
/// HLOD patches describe one forest instead of two overlapping biomes.
#[inline]
pub fn tree_presence_probability(altitude: f32, slope: f32, moisture: f32) -> f32 {
    forest_cover(altitude, slope, moisture)
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
    let stand_coverage = forest_stand_coverage(altitude, slope, moisture, x, z);
    (smooth(
        0.12,
        0.42,
        tree_presence_probability(altitude, slope, moisture),
    ) * forest_patch(x, z).sqrt()
        * if stand_coverage > 0.0 { 1.0 } else { 0.0 }
        * VEGETATION_NEAR_DENSITY_SCALE)
        .clamp(0.0, 1.0)
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

/// Shared continuous forest-cover density for near placement, terrain
/// green-tinting, and the far canopy material. Keeping this term shared makes
/// the altitude, moisture, and slope edges agree across all representations;
/// [`forest_patch`] supplies the remaining spatial stand structure.
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

/// One visible forest-stand footprint shared by terrain tinting, tree
/// placement, and the far aggregate canopy. The minimum coverage floor is
/// deliberately part of this function; it must not be reimplemented as a
/// representation-specific discard.
#[inline]
pub fn forest_stand_coverage(
    altitude: f32,
    slope: f32,
    moisture: f32,
    x: f32,
    z: f32,
) -> f32 {
    let cover = smooth(
        VEGETATION_STAND_COVER_START,
        VEGETATION_STAND_COVER_END,
        forest_cover(altitude, slope, moisture),
    );
    let stand_edge = smooth(
        VEGETATION_STAND_EDGE_START,
        VEGETATION_STAND_EDGE_END,
        forest_patch(x, z),
    );
    let coverage = cover * stand_edge;
    if coverage >= VEGETATION_STAND_MIN_COVERAGE {
        coverage
    } else {
        0.0
    }
}

// ── Far-field aggregate canopy field ──────────────────────────────────

/// Pack one aggregate canopy sample. The first three words preserve the
/// canonical cell centre and ground height; the final word stores density,
/// canopy height, representative species, and a stable material random.
fn pack_canopy(
    x: f32,
    z: f32,
    ground: f32,
    density: f32,
    height: f32,
    species: u8,
    random: f32,
) -> [u32; 4] {
    let density_q = (density.clamp(0.0, 1.0) * 255.0).round() as u32;
    let height_q = ((height / CANOPY_MAX_HEIGHT).clamp(0.0, 1.0) * 255.0).round() as u32;
    let random_q = (random.clamp(0.0, 1.0) * 255.0).round() as u32;
    [
        x.to_bits(),
        z.to_bits(),
        ground.to_bits(),
        density_q | (height_q << 8) | ((species as u32 & 0x7) << 16) | (random_q << 24),
    ]
}

#[inline]
pub fn canopy_density(record: [u32; 4]) -> f32 {
    (record[3] & 0xff) as f32 * (1.0 / 255.0)
}

#[inline]
pub fn canopy_height(record: [u32; 4]) -> f32 {
    ((record[3] >> 8) & 0xff) as f32 * (CANOPY_MAX_HEIGHT / 255.0)
}

#[inline]
pub fn canopy_species(record: [u32; 4]) -> u8 {
    ((record[3] >> 16) & 0x7) as u8
}

#[inline]
pub fn canopy_random(record: [u32; 4]) -> f32 {
    (record[3] >> 24) as f32 * (1.0 / 255.0)
}

/// Build one aggregate field sample from the same cached height/moisture
/// recipe used by individual placement. Four 64 m lattice samples keep the
/// 128 m cell continuous enough for the far renderer without storing an 8x8
/// field per cell; the tree database remains the high-frequency representation.
fn canopy_record(cx: u32, cz: u32, terrain_cache: &[[f32; 4]]) -> [u32; 4] {
    let center_x = (cx as f32 + 0.5) * VEGETATION_CELL_METRES;
    let center_z = (cz as f32 + 0.5) * VEGETATION_CELL_METRES;
    let mut density = 0.0;
    let mut ground = 0.0;
    let mut highest_ground = WATER_LEVEL;
    let mut altitude = 0.0;
    let mut moisture = 0.0;
    let terrain_axis = super::TERRAIN_GRID_CELLS as usize;
    for sz in 0..2 {
        for sx in 0..2 {
            let sample = terrain_cache[((cz * 2 + sz) as usize % terrain_axis) * terrain_axis
                + (cx * 2 + sx) as usize % terrain_axis];
            let slope = 1.0 / (sample[1] * sample[1] + sample[2] * sample[2] + 1.0).sqrt();
            let slope = 1.0 - slope;
            let patch_x = cx as f32 * VEGETATION_CELL_METRES + (sx as f32 + 0.5) * 64.0;
            let patch_z = cz as f32 * VEGETATION_CELL_METRES + (sz as f32 + 0.5) * 64.0;
            density += forest_stand_coverage(
                sample[0], slope, sample[3], patch_x, patch_z);
            ground += sample[0].max(WATER_LEVEL);
            highest_ground = highest_ground.max(sample[0].max(WATER_LEVEL));
            altitude += sample[0];
            moisture += sample[3];
        }
    }
    density *= 0.25;
    // A single cell record cannot carry the full 8x8 terrain height field.
    // Apply only a small, capped uphill bias so the tile remains visible on a
    // slope without lifting high-alpine cells into the sky.
    ground *= 0.25;
    ground += ((highest_ground - ground) * 0.08).clamp(0.0, 6.0);
    altitude *= 0.25;
    moisture *= 0.25;
    let forest = smooth(0.34, 0.52, moisture);
    let species_hash = super::hash(cx.wrapping_mul(2), cz.wrapping_mul(2), 431);
    let species = tree_species(altitude, species_hash, forest);
    let random = super::hash(cx, cz, 739);
    let height = (8.0 + 42.0 * density * (0.78 + 0.22 * random)).clamp(8.0, CANOPY_MAX_HEIGHT);
    pack_canopy(center_x, center_z, ground, density, height, species, random)
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

fn canonical_cell(x: f32, z: f32) -> u32 {
    let wx = x.rem_euclid(WORLD_PERIOD as f32);
    let wz = z.rem_euclid(WORLD_PERIOD as f32);
    let cx = ((wx / VEGETATION_CELL_METRES).floor() as u32).min(VEGETATION_CELLS_PER_AXIS - 1);
    let cz = ((wz / VEGETATION_CELL_METRES).floor() as u32).min(VEGETATION_CELLS_PER_AXIS - 1);
    cz * VEGETATION_CELLS_PER_AXIS + cx
}

/// Pack one exact CPU placement into the four-word SSBO representation.
/// Positions and ground stay as f32 bits; only the stable material metadata is
/// quantized, keeping the near-camera silhouette aligned with collision.
fn pack_instance(item: &super::ScatterItem) -> [u32; 4] {
    // 15 bits cover the primary and smaller companion size range at roughly
    // 0.00005 resolution. The spare low bits carry species, a stable material
    // random, and boulder state.
    let scale_q = (((item.size - 0.70) / 1.60).clamp(0.0, 1.0) * 32767.0).round() as u32;
    let random_q = (item.size_r.clamp(0.0, 1.0) * 255.0).round() as u32;
    let metadata = (item.species as u32 & 0x7)
        | (random_q << 8)
        | ((if item.boulder { 1 } else { 0 }) << 16)
        | (scale_q << 17);
    [
        item.x.rem_euclid(WORLD_PERIOD as f32).to_bits(),
        item.z.rem_euclid(WORLD_PERIOD as f32).to_bits(),
        item.ground.to_bits(),
        metadata,
    ]
}

/// Generate the immutable periodic forest database once at renderer startup.
/// The source lattice retains the deterministic scatter/collision recipe; the
/// output is sorted by 128 m cell so one indirect command can draw each visible
/// range without re-running hashes, terrain sampling, or settlement checks in
/// the vertex stage.
pub fn build_database() -> VegetationDatabase {
    // 24 m source slots cover 65,520 m of the 65,536 m terrain period. The
    // final 16 m seam is intentionally left to the nearest periodic copy; it
    // avoids inventing a second hash lattice with a different density.
    let source_axis = (WORLD_PERIOD / SCATTER_PITCH as f64).floor() as i64;
    let worker_count = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(source_axis as usize)
        .max(1);
    let rows_per_worker = (source_axis as usize + worker_count - 1) / worker_count;
    let mut entries: Vec<(u32, [u32; 4])> = Vec::new();
    std::thread::scope(|scope| {
        let mut workers = Vec::with_capacity(worker_count);
        for worker in 0..worker_count {
            let start = (worker * rows_per_worker) as i64;
            let end = ((worker + 1) * rows_per_worker).min(source_axis as usize) as i64;
            if start >= end {
                continue;
            }
            workers.push(scope.spawn(move || {
                let mut local = Vec::new();
                for cz in start..end {
                    for cx in 0..source_axis {
                        let Some(primary) = super::scatter_slot(cx, cz) else {
                            continue;
                        };
                        let cell = canonical_cell(primary.x, primary.z);
                        local.push((cell, pack_instance(&primary)));
                        if let Some(companion) = super::scatter_companion(cx, cz, &primary) {
                            local.push((
                                canonical_cell(companion.x, companion.z),
                                pack_instance(&companion),
                            ));
                        }
                    }
                }
                local
            }));
        }
        for worker in workers {
            entries.extend(
                worker
                    .join()
                    .expect("vegetation generation worker panicked"),
            );
        }
    });
    entries.sort_unstable_by_key(|(cell, instance)| (*cell, instance[0], instance[1], instance[3]));

    let mut cells = vec![VegetationCell::default(); VEGETATION_CELL_COUNT as usize];
    let mut instances = Vec::with_capacity(entries.len());
    for (cell, instance) in entries {
        let range = &mut cells[cell as usize];
        if range.instance_count == 0 {
            range.first_instance = instances.len() as u32;
        }
        range.instance_count += 1;
        instances.push(instance);
    }
    // Reuse one complete terrain cache for the aggregate field. Calling the
    // memoized lattice sampler once per sub-cell would evict its small direct
    // map repeatedly and turn this coarse pass into a second 5x5 terrain
    // evaluation. The cache is the same height/slope/moisture recipe consumed
    // by the renderer's terrain texture upload.
    let terrain_cache = super::terrain_samples();
    let mut canopies = vec![[0u32; 4]; VEGETATION_CELL_COUNT as usize];
    for cz in 0..VEGETATION_CELLS_PER_AXIS {
        for cx in 0..VEGETATION_CELLS_PER_AXIS {
            let index = (cz * VEGETATION_CELLS_PER_AXIS + cx) as usize;
            canopies[index] = canopy_record(cx, cz, &terrain_cache);
        }
    }
    VegetationDatabase {
        instances,
        cells,
        canopies,
    }
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
        // The shared forest-cover ramp is partway through its 300--520 m
        // valley-floor transition at 400 m; it should still admit a healthy
        // population without treating every low meadow as dense forest.
        assert!(tree_presence_probability(400.0, 0.05, 0.7) > 0.4);
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
        assert!((0.0..=1.0).contains(&a));
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

    #[test]
    #[ignore = "startup database measurement"]
    fn database_ranges_are_sorted_and_periodic() {
        let started = std::time::Instant::now();
        let database = build_database();
        let mut occupied = 0usize;
        for (index, cell) in database.cells.iter().enumerate() {
            if cell.instance_count == 0 {
                continue;
            }
            occupied += 1;
            let end = cell.first_instance as usize + cell.instance_count as usize;
            assert!(
                end <= database.instances.len(),
                "cell {index} exceeds instance array"
            );
            for packed in &database.instances[cell.first_instance as usize..end] {
                let x = f32::from_bits(packed[0]);
                let z = f32::from_bits(packed[1]);
                assert!((0.0..WORLD_PERIOD as f32).contains(&x));
                assert!((0.0..WORLD_PERIOD as f32).contains(&z));
            }
        }
        let canopy_occupied = database
            .canopies
            .iter()
            .filter(|record| canopy_density(**record) > 0.0)
            .count();
        eprintln!(
            "vegetation database: {} instances in {occupied} tree cells, {canopy_occupied} canopy cells, {:.3}s",
            database.instances.len(),
            started.elapsed().as_secs_f32()
        );
        assert!(!database.instances.is_empty());
        assert!(occupied > 100);
        assert_eq!(database.canopies.len(), VEGETATION_CELL_COUNT as usize);
        assert!(database
            .canopies
            .iter()
            .any(|record| canopy_density(*record) > 0.0));
    }
}
