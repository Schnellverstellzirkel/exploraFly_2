//! Deterministic alpine terrain shared by flight clearance and the ground shader.
//!
//! Heights are metres above sea level. Query with absolute world X/Z, never a
//! camera-relative position. The 65.536 km period is deliberate: reducing f64
//! coordinates before converting to f32 preserves local precision after rebasing.
//! Keep the terrain recipe synchronized with `engine/shaders/terrain.inc`.

pub const WORLD_PERIOD: f64 = 65_536.0;
pub const WATER_LEVEL: f32 = 185.0;
pub const MAX_TERRAIN_HEIGHT: f32 = 3_123.0;
pub const TERRAIN_GRID_CELLS: u32 = 1024;
pub const TERRAIN_CELL_METRES: f32 = 64.0;
pub const TERRAIN_VERTEX_COUNT: u32 = (TERRAIN_GRID_CELLS + 1) * (TERRAIN_GRID_CELLS + 1);
pub const TERRAIN_INDEX_COUNT: u32 = TERRAIN_GRID_CELLS * TERRAIN_GRID_CELLS * 6;
pub const TERRAIN_CHUNK_CELLS: u32 = 32;
pub const TERRAIN_CHUNKS_PER_AXIS: u32 = TERRAIN_GRID_CELLS / TERRAIN_CHUNK_CELLS;
pub const TERRAIN_CHUNK_COUNT: u32 = TERRAIN_CHUNKS_PER_AXIS * TERRAIN_CHUNKS_PER_AXIS;
pub const TERRAIN_CHUNK_INDICES: u32 = TERRAIN_CHUNK_CELLS * TERRAIN_CHUNK_CELLS * 6;
pub const LANDMARK_STRUCTURES: u32 = 24;
pub const LANDMARK_VERTEX_COUNT: u32 = 9 * LANDMARK_STRUCTURES * 54;
pub const DRAW_INDEX_COUNT: u32 = TERRAIN_INDEX_COUNT + LANDMARK_VERTEX_COUNT;
pub const CLEARANCE_METRES: f32 = 45.0;
pub const SPAWN_X: f32 = 0.0;
pub const SPAWN_Z: f32 = 1_050.0;
pub const SPAWN_ALTITUDE: f32 = 1_100.0;
const SETTLEMENT_SPACING: f32 = 16_384.0;

/// Immutable topology. Adjacent triangles reuse the same height/normal fetch.
pub fn terrain_indices() -> Vec<u32> {
    let mut indices = Vec::with_capacity(DRAW_INDEX_COUNT as usize);
    let stride = TERRAIN_GRID_CELLS + 1;
    for cz in 0..TERRAIN_CHUNKS_PER_AXIS {
        for cx in 0..TERRAIN_CHUNKS_PER_AXIS {
            for z in 0..TERRAIN_CHUNK_CELLS {
                for x in 0..TERRAIN_CHUNK_CELLS {
                    let a = (cz * TERRAIN_CHUNK_CELLS + z) * stride + cx * TERRAIN_CHUNK_CELLS + x;
                    indices.extend_from_slice(&[a, a + stride, a + 1, a + 1, a + stride, a + stride + 1]);
                }
            }
        }
    }
    indices.extend(TERRAIN_VERTEX_COUNT..TERRAIN_VERTEX_COUNT + LANDMARK_VERTEX_COUNT);
    indices
}

/// One periodic, immutable height/normal cache, sampled at world lattice points.
/// RGB stores land height and the two surface slopes; water has a flat normal.
/// W stores landform moisture in [0,1]: valley floors and hollows hold water,
/// steep high ground sheds it. Local proxy for the topographic wetness index
/// ln(a/tan beta) (Beven & Kirkby 1979): height above water stands in for
/// upslope contributing area, local slope for tan beta, and smoothed-profile
/// concavity marks hollows. Full flow routing is skipped: the tile is periodic
/// and the cache builds once at startup.
pub fn terrain_samples() -> Vec<[f32; 4]> {
    let n = TERRAIN_GRID_CELLS as usize;
    let step = TERRAIN_CELL_METRES as f64;
    let heights: Vec<f32> = (0..n * n)
        .map(|i| height_at((i % n) as f64 * step, (i / n) as f64 * step))
        .collect();
    let surface = |x: usize, z: usize| heights[(z % n) * n + x % n].max(WATER_LEVEL);
    // Micro relief (128 m octave and below) would dominate a raw laplacian and
    // turn moisture back into fine noise. A 5x5 box blur keeps valley/ridge
    // structure while discarding sub-300 m roughness.
    let blurred: Vec<f32> = (0..n * n)
        .map(|i| {
            let x = i % n;
            let z = i / n;
            let mut acc = 0.0f32;
            for dz in 0..5 {
                for dx in 0..5 {
                    acc += surface((x + n + dx - 2) % n, (z + n + dz - 2) % n);
                }
            }
            acc / 25.0
        })
        .collect();
    let smooth_h = |x: usize, z: usize| blurred[(z % n) * n + x % n];
    (0..n * n).map(|i| {
        let x = i % n;
        let z = i / n;
        let dzdx = (surface(x + 1, z) - surface(x + n - 1, z)) / (2.0 * TERRAIN_CELL_METRES);
        let dzdz = (surface(x, z + 1) - surface(x, z + n - 1)) / (2.0 * TERRAIN_CELL_METRES);
        let c = smooth_h(x, z);
        let slope = ((smooth_h(x + 1, z) - smooth_h(x + n - 1, z)).powi(2)
            + (smooth_h(x, z + 1) - smooth_h(x, z + n - 1)).powi(2))
        .sqrt() / (2.0 * TERRAIN_CELL_METRES);
        let laplacian = (smooth_h(x + 1, z) + smooth_h(x + n - 1, z)
            + smooth_h(x, z + 1) + smooth_h(x, z + n - 1) - 4.0 * c)
            / (TERRAIN_CELL_METRES * TERRAIN_CELL_METRES);
        [heights[i], dzdx, dzdz, moisture_at(c, slope, laplacian)]
    }).collect()
}

/// Landform moisture from smoothed height `h`, slope magnitude, and profile
/// concavity (positive in hollows). Valley floors approach 1, dry ridges
/// approach the 0.08 floor, and hollow gullies gain up to 0.45 over the same
/// altitude and slope.
fn moisture_at(surface_h: f32, slope: f32, laplacian: f32) -> f32 {
    let above_water = (surface_h - WATER_LEVEL).max(0.0);
    let valley = (-above_water / 250.0).exp();
    let drain = 1.0 - smooth(0.45, 1.0, slope);
    let hollow = smooth(-0.0003, 0.0012, laplacian);
    (0.75 * valley * drain + 0.45 * hollow + 0.08).clamp(0.0, 1.0)
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn hash(x: u32, z: u32, seed: u32) -> f32 {
    let mut h = x.wrapping_mul(1_664_525)
        .wrapping_add(z.wrapping_mul(1_013_904_223))
        .wrapping_add(seed.wrapping_mul(2_246_822_519));
    h ^= h >> 16;
    h = h.wrapping_mul(2_246_822_519);
    h ^= h >> 13;
    h = h.wrapping_mul(3_266_489_917);
    h ^= h >> 16;
    (h & 0x00ff_ffff) as f32 * (1.0 / 16_777_215.0)
}

fn noise(p: [f32; 2], cell: f32, seed: u32) -> f32 {
    let q = [p[0] / cell, p[1] / cell];
    let c = [q[0].floor() as u32, q[1].floor() as u32];
    let mask = (WORLD_PERIOD as f32 / cell) as u32 - 1;
    let f = [q[0] - q[0].floor(), q[1] - q[1].floor()];
    let w = f.map(|x| x * x * x * (x * (x * 6.0 - 15.0) + 10.0));
    mix(
        mix(hash(c[0] & mask, c[1] & mask, seed), hash((c[0] + 1) & mask, c[1] & mask, seed), w[0]),
        mix(hash(c[0] & mask, (c[1] + 1) & mask, seed), hash((c[0] + 1) & mask, (c[1] + 1) & mask, seed), w[0]),
        w[1],
    )
}

fn valley_center(z: f32) -> f32 {
    900.0 * (z * (std::f32::consts::TAU / 32_768.0)).sin()
        + 380.0 * (z * (std::f32::consts::TAU / 8_192.0)).sin()
}

/// Rock/soil elevation. Lake beds remain below the water level.
pub fn height_at(x: f64, z: f64) -> f32 {
    let p = [x.rem_euclid(WORLD_PERIOD) as f32, z.rem_euclid(WORLD_PERIOD) as f32];
    let cross_valley = (p[0] - valley_center(p[1]) + 8_192.0).rem_euclid(16_384.0) - 8_192.0;
    let ax = cross_valley.abs();
    // Glacial trough: flat floor, steep sides.
    let shoulder = smooth(420.0, 3_800.0, ax).powf(1.25);
    let ridge = 1.0 - (2.0 * noise(p, 2_048.0, 31) - 1.0).abs();
    let ridge = ridge.powf(0.75);
    let crag = 1.0 - (2.0 * noise(p, 512.0, 67) - 1.0).abs();
    // Massif envelope: slow height variation decides which ranges become high
    // Alps and which stay rolling foothills, so peaks stop sharing one height.
    // The remap gives flat-topped high zones where full-height peaks form.
    let massif = (0.30 + 0.70 * smooth(0.20, 0.75, noise(p, 16_384.0, 311)))
        * (0.80 + 0.20 * noise(p, 8_192.0, 317));
    // Foothill belt: rolling pre-alpine hills between floor and high rock.
    let foothill_belt = smooth(500.0, 1_500.0, ax) * (1.0 - shoulder);
    let foothill = noise(p, 1_024.0, 331) * 2.0 - 1.0;
    let elevation = 235.0 + noise(p, 1_024.0, 101) * 90.0
        + foothill_belt * (180.0 + 220.0 * foothill * massif).max(0.0)
        + shoulder * massif * (950.0 + 1_550.0 * ridge * ridge + 280.0 * crag * crag * crag)
        + noise(p, 128.0, 223) * 18.0 * (0.2 + 0.8 * shoulder);
    let lake_z = (p[1] - 4_800.0 + 8_192.0).rem_euclid(16_384.0) - 8_192.0;
    let lake_d = (cross_valley / 850.0).powi(2) + (lake_z / 1_400.0).powi(2);
    let basin = 1.0 - smooth(0.62, 1.30, lake_d);
    mix(elevation, 110.0 + 14.0 * noise(p, 256.0, 193), basin)
}

/// Visible surface, including the flat alpine lakes.
pub fn surface_height_at(x: f64, z: f64) -> f32 {
    height_at(x, z).max(WATER_LEVEL)
}

/// The same fixed diagonal and clamped corner heights used by the indexed mesh.
/// Collision must cover interpolation above the analytic surface in concavities.
fn mesh_height_at(x: f64, z: f64) -> f32 {
    let step = TERRAIN_CELL_METRES as f64;
    let x0 = (x / step).floor() * step;
    let z0 = (z / step).floor() * step;
    let u = ((x - x0) / step) as f32;
    let v = ((z - z0) / step) as f32;
    let b = surface_height_at(x0 + step, z0);
    let c = surface_height_at(x0, z0 + step);
    if u + v <= 1.0 {
        surface_height_at(x0, z0) * (1.0 - u - v) + b * u + c * v
    } else {
        surface_height_at(x0 + step, z0 + step) * (u + v - 1.0)
            + b * (1.0 - v) + c * (1.0 - u)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Structure {
    pub x: f32,
    pub z: f32,
    pub half_x: f32,
    pub half_z: f32,
    pub wall_height: f32,
    pub roof_height: f32,
}

/// A keep, four towers, four curtain walls, and fifteen gabled village houses.
/// Local coordinates are mirrored by the procedural landmark vertex shader.
pub fn structure(index: u32) -> Structure {
    assert!(index < LANDMARK_STRUCTURES);
    let (x, z, half_x, half_z, wall_height, roof_height) = match index {
        0 => (0.0, 0.0, 42.0, 32.0, 100.0, 34.0),
        1..=4 => {
            let corner = index - 1;
            (if corner & 1 == 0 { -64.0 } else { 64.0 },
             if corner & 2 == 0 { -52.0 } else { 52.0 }, 15.0, 15.0, 135.0, 40.0)
        }
        5..=6 => (if index == 5 { -64.0 } else { 64.0 }, 0.0, 7.0, 52.0, 40.0, 5.0),
        7..=8 => (0.0, if index == 7 { -52.0 } else { 52.0 }, 64.0, 7.0, 40.0, 5.0),
        _ => {
            let house = index - 9;
            (-260.0 + (house % 5) as f32 * 64.0, -220.0 - (house / 5) as f32 * 68.0,
             16.0, 13.0, 18.0 + (house % 3) as f32 * 5.0, 15.0)
        }
    };
    Structure { x, z, half_x, half_z, wall_height, roof_height }
}

const BOX_CORNERS: [[f32; 3]; 8] = [
    [-1.0, 0.0, -1.0], [1.0, 0.0, -1.0],
    [-1.0, 1.0, -1.0], [1.0, 1.0, -1.0],
    [-1.0, 0.0, 1.0],  [1.0, 0.0, 1.0],
    [-1.0, 1.0, 1.0],  [1.0, 1.0, 1.0],
];
const BOX_TRIS: [usize; 36] = [
    0, 2, 1, 1, 2, 3,
    5, 7, 4, 4, 7, 6,
    4, 6, 0, 0, 6, 2,
    1, 3, 5, 5, 3, 7,
    2, 6, 3, 3, 6, 7,
    4, 0, 5, 5, 0, 1,
];
const ROOF_CORNERS: [[f32; 3]; 6] = [
    [-1.0, 0.0, -1.0], [1.0, 0.0, -1.0],
    [-1.0, 0.0, 1.0],  [1.0, 0.0, 1.0],
    [0.0, 1.0, -1.0],  [0.0, 1.0, 1.0],
];
const ROOF_TRIS: [usize; 18] = [
    0, 2, 5, 0, 5, 4,
    1, 4, 5, 1, 5, 3,
    0, 4, 1,
    2, 3, 5,
];

/// Triangle vertices (x, y, z floats) for all landmark structures across one world period.
pub fn landmark_structure_triangles() -> Vec<f32> {
    let mut verts = Vec::with_capacity(16 * LANDMARK_STRUCTURES as usize * 54 * 3);
    for tz in 0..4 {
        let base_z = tz as f32 * SETTLEMENT_SPACING + 3_450.0;
        let valley = valley_center(base_z.rem_euclid(WORLD_PERIOD as f32));
        for tx in 0..4 {
            let base_x = tx as f32 * SETTLEMENT_SPACING + valley + 1_180.0;
            for index in 0..LANDMARK_STRUCTURES {
                let s = structure(index);
                let center_x = base_x + s.x;
                let center_z = base_z + s.z;
                let foundation = height_at(center_x as f64, center_z as f64).max(WATER_LEVEL);
                
                // Wall box (12 triangles, 36 vertices)
                for &idx in &BOX_TRIS {
                    let c = BOX_CORNERS[idx];
                    let vx = center_x + c[0] * s.half_x;
                    let vy = foundation + c[1] * s.wall_height;
                    let vz = center_z + c[2] * s.half_z;
                    verts.extend_from_slice(&[vx, vy, vz]);
                }
                
                // Roof (6 triangles, 18 vertices)
                for &idx in &ROOF_TRIS {
                    let c = ROOF_CORNERS[idx];
                    let vx = center_x + c[0] * (s.half_x + 1.0);
                    let vy = foundation + s.wall_height + c[1] * s.roof_height;
                    let vz = center_z + c[2] * (s.half_z + 1.0);
                    verts.extend_from_slice(&[vx, vy, vz]);
                }
            }
        }
    }
    verts
}

/// Conservative collision surface for the nearest settlement and water/terrain.
/// Buildings use their highest roof point across the footprint. This is an
/// intentionally forgiving arcade floor, not detailed rigid-body collision.
pub fn collision_height_at(x: f64, z: f64) -> f32 {
    let mut height = surface_height_at(x, z).max(mesh_height_at(x, z));
    // Reduce first, just as the vertex shader does; inspecting adjacent cells
    // catches the village extents even at settlement boundaries.
    let p = [x.rem_euclid(WORLD_PERIOD) as f32, z.rem_euclid(WORLD_PERIOD) as f32];
    let tile = [(p[0] / SETTLEMENT_SPACING).floor(), (p[1] / SETTLEMENT_SPACING).floor()];
    for dz in -1..=1 {
        let base_z = (tile[1] + dz as f32) * SETTLEMENT_SPACING + 3_450.0;
        let valley = valley_center(base_z.rem_euclid(WORLD_PERIOD as f32));
        for dx in -1..=1 {
            let base_x = (tile[0] + dx as f32) * SETTLEMENT_SPACING + valley + 1_180.0;
            if (p[0] - base_x).abs() > 320.0 || (p[1] - base_z).abs() > 400.0 { continue; }
            for index in 0..LANDMARK_STRUCTURES {
                let s = structure(index);
                let sx = base_x + s.x;
                let sz = base_z + s.z;
                // Match the shader's one-metre eaves and pad for the glider's
                // approximately 11 m half-span while approaching a roof edge.
                if (p[0] - sx).abs() <= s.half_x + 12.0 && (p[1] - sz).abs() <= s.half_z + 12.0 {
                    height = height.max(surface_height_at(sx as f64, sz as f64)
                        + s.wall_height + s.roof_height);
                }
            }
        }
    }
    height
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_is_in_open_valley_with_clearance() {
        assert!(SPAWN_ALTITUDE > collision_height_at(SPAWN_X as f64, SPAWN_Z as f64) + 500.0);
    }

    #[test]
    fn lake_is_flat_and_has_a_submerged_bed() {
        let z = 4_800.0;
        let x = valley_center(z);
        assert!(height_at(x as f64, z as f64) < WATER_LEVEL - 50.0);
        assert_eq!(surface_height_at(x as f64, z as f64), WATER_LEVEL);
        assert_eq!(surface_height_at(x as f64 + 100.0, z as f64), WATER_LEVEL);
    }

    #[test]
    fn alpine_relief_is_bounded_and_has_high_peaks() {
        let mut maximum = 0.0f32;
        for z in (0..65_536).step_by(512) {
            for x in (0..65_536).step_by(512) {
                let h = height_at(x as f64, z as f64);
                assert!(h.is_finite() && (100.0..=MAX_TERRAIN_HEIGHT).contains(&h), "{x}, {z}: {h}");
                maximum = maximum.max(h);
            }
        }
        assert!(maximum > 2_800.0, "maximum {maximum}");
    }

    #[test]
    fn wrapping_and_rebasing_preserve_elevation() {
        for (x, z) in [(17.25, 1050.5), (-900.125, -12_002.75), (32_767.5, 12_001.25)] {
            let expected = height_at(x, z);
            assert_eq!(expected, height_at(x + WORLD_PERIOD * 1_000_000.0, z - WORLD_PERIOD * 900_000.0));
            let anchor = [4096.0, -8192.0];
            assert_eq!(expected, height_at(anchor[0] + (x - anchor[0]), anchor[1] + (z - anchor[1])));
        }
    }

    #[test]
    fn world_period_has_no_height_seam() {
        for z in (0..65_536).step_by(127) {
            assert!((height_at(-0.01, z as f64) - height_at(0.01, z as f64)).abs() < 0.1);
            assert!((height_at(z as f64, -0.01) - height_at(z as f64, 0.01)).abs() < 0.1);
        }
    }

    #[test]
    fn castle_collision_includes_roof() {
        let z = 3_450.0;
        let x = valley_center(z) + 1_180.0;
        let floor = surface_height_at(x as f64, z as f64);
        assert!((collision_height_at(x as f64, z as f64) - floor - 134.0).abs() < 0.01);
    }

    #[test]
    fn keep_roof_overhang_receives_collision_clearance() {
        let z = 3450.0;
        let x = valley_center(z) + 1180.0;
        let ridge = surface_height_at(x as f64, z as f64) + 134.0;
        assert!((collision_height_at(x as f64, z as f64 + 32.5) - ridge).abs() < 0.01);
    }

    #[test]
    fn procedural_draw_budget_is_fixed() {
        assert_eq!(TERRAIN_VERTEX_COUNT, 1_050_625);
        assert_eq!(DRAW_INDEX_COUNT, 6_303_120);
        assert_eq!(TERRAIN_GRID_CELLS as f64 * TERRAIN_CELL_METRES as f64, WORLD_PERIOD);
        // Recentring keeps a minimum 32.7 km radius around the camera.
        assert!((TERRAIN_GRID_CELLS / 2 - 1) as f32 * TERRAIN_CELL_METRES > 30_000.0);
        let indices = terrain_indices();
        assert_eq!(indices.len(), DRAW_INDEX_COUNT as usize);
        assert!(indices.iter().all(|&i| i < TERRAIN_VERTEX_COUNT + LANDMARK_VERTEX_COUNT));
        assert_eq!(&indices[..6], &[0, 1025, 1, 1, 1025, 1026]);
        assert_eq!(indices[TERRAIN_INDEX_COUNT as usize], TERRAIN_VERTEX_COUNT);
    }

    #[test]
    fn landform_moisture_follows_water_and_sheds_ridges() {
        let n = TERRAIN_GRID_CELLS as usize;
        let cell = TERRAIN_CELL_METRES as f64;
        let samples = terrain_samples();
        assert!(samples.iter().all(|s| (0.0..=1.0).contains(&s[3])));
        let spot = |x: f64, z: f64| {
            let ix = (x.rem_euclid(WORLD_PERIOD) / cell) as usize % n;
            let iz = (z.rem_euclid(WORLD_PERIOD) / cell) as usize % n;
            samples[iz * n + ix][3]
        };
        // Valley floor near spawn holds water; a high ridge sheds it.
        let floor = spot(0.0, 1_050.0);
        let ridge = spot(valley_center(1_050.0) as f64 + 6_500.0, 1_050.0);
        assert!(floor > 0.55, "valley floor {floor}");
        assert!(ridge < 0.6, "ridge {ridge}");
        assert!(floor > ridge, "floor {floor} ridge {ridge}");
        // Lakeshore is wet.
        let shore = spot(valley_center(4_800.0) as f64 + 750.0, 4_800.0);
        assert!(shore > 0.6, "lakeshore {shore}");
        // Wet valley floors are a minority; dry high ground is substantial;
        // low ground is wetter on average than high ground.
        let wet = samples.iter().filter(|s| s[3] > 0.7).count();
        let dry = samples.iter().filter(|s| s[3] < 0.3).count();
        assert!((0.02..0.4).contains(&(wet as f32 / samples.len() as f32)), "wet {wet}");
        assert!(dry as f32 / samples.len() as f32 > 0.3, "dry {dry}");
        let (mut low_sum, mut low_n, mut high_sum, mut high_n) = (0.0f32, 0u32, 0.0f32, 0u32);
        for s in &samples {
            if s[0] < 400.0 {
                low_sum += s[3];
                low_n += 1;
            } else if s[0] > 1800.0 {
                high_sum += s[3];
                high_n += 1;
            }
        }
        assert!(low_sum / low_n as f32 > high_sum / high_n as f32 + 0.15,
            "low {} high {}", low_sum / low_n as f32, high_sum / high_n as f32);
    }

    #[test]
    fn cached_terrain_matches_world_and_wraps_normals() {
        let samples = terrain_samples();
        let n = TERRAIN_GRID_CELLS as usize;
        assert!(samples.iter().flatten().all(|v| v.is_finite()));
        for (x, z) in [(0, 0), (n - 1, n - 1), (17, 75), (400, 600)] {
            let wx = x as f64 * TERRAIN_CELL_METRES as f64;
            let wz = z as f64 * TERRAIN_CELL_METRES as f64;
            let sample = samples[z * n + x];
            assert_eq!(sample[0], height_at(wx, wz));
            assert_eq!(sample[1], (surface_height_at(wx + 64.0, wz)
                - surface_height_at(wx - 64.0, wz)) / 128.0);
            assert_eq!(sample[2], (surface_height_at(wx, wz + 64.0)
                - surface_height_at(wx, wz - 64.0)) / 128.0);
        }
    }

    #[test]
    fn collision_covers_triangle_interpolation_and_rebases() {
        for z in (0..65_536).step_by(397) {
            for x in (0..65_536).step_by(431) {
                let x = x as f64 + 17.25;
                let z = z as f64 + 29.5;
                let height = mesh_height_at(x, z);
                assert!(collision_height_at(x, z) >= height);
                assert_eq!(height, mesh_height_at(x - WORLD_PERIOD, z + WORLD_PERIOD));
            }
        }
    }

    #[test]
    fn landmark_triangles_count_and_bounds() {
        let tris = landmark_structure_triangles();
        assert_eq!(tris.len(), 16 * LANDMARK_STRUCTURES as usize * 54 * 3);
        for i in 0..tris.len() / 3 {
            let x = tris[i * 3];
            let y = tris[i * 3 + 1];
            let z = tris[i * 3 + 2];
            assert!(x.is_finite() && y.is_finite() && z.is_finite());
            assert!(y >= WATER_LEVEL);
        }
    }
}
