//! Deterministic alpine terrain shared by flight clearance and the ground shader.
//!
//! Heights are metres above sea level. Query with absolute world X/Z, never a
//! camera-relative position. The 65.536 km period is deliberate: reducing f64
//! coordinates before converting to f32 preserves local precision after rebasing.
//! Keep the terrain recipe synchronized with `engine/shaders/terrain.inc`.

pub const WORLD_PERIOD: f64 = 65_536.0;
pub const WATER_LEVEL: f32 = 185.0;
pub const MAX_TERRAIN_HEIGHT: f32 = 3_123.0;
pub const TERRAIN_GRID_CELLS: u32 = 64;
pub const TERRAIN_VERTEX_COUNT: u32 = TERRAIN_GRID_CELLS * TERRAIN_GRID_CELLS * 6;
pub const LANDMARK_STRUCTURES: u32 = 24;
pub const LANDMARK_VERTEX_COUNT: u32 = 9 * LANDMARK_STRUCTURES * 54;
pub const DRAW_VERTEX_COUNT: u32 = TERRAIN_VERTEX_COUNT + LANDMARK_VERTEX_COUNT;
pub const CLEARANCE_METRES: f32 = 45.0;
pub const SPAWN_X: f32 = 0.0;
pub const SPAWN_Z: f32 = 1_050.0;
pub const SPAWN_ALTITUDE: f32 = 1_100.0;
const SETTLEMENT_SPACING: f32 = 16_384.0;

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
    let shoulder = smooth(420.0, 3_800.0, cross_valley.abs());
    let ridge = 1.0 - (2.0 * noise(p, 2_048.0, 31) - 1.0).abs();
    let crag = 1.0 - (2.0 * noise(p, 512.0, 67) - 1.0).abs();
    let elevation = 235.0 + noise(p, 1_024.0, 101) * 90.0
        + shoulder * (950.0 + 1_550.0 * ridge * ridge + 280.0 * crag * crag * crag)
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

/// Conservative collision surface for the nearest settlement and water/terrain.
/// Buildings use their highest roof point across the footprint. This is an
/// intentionally forgiving arcade floor, not detailed rigid-body collision.
pub fn collision_height_at(x: f64, z: f64) -> f32 {
    let mut height = surface_height_at(x, z);
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
                if (p[0] - sx).abs() <= s.half_x && (p[1] - sz).abs() <= s.half_z {
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
    fn procedural_draw_budget_is_fixed() {
        assert_eq!(TERRAIN_VERTEX_COUNT, 24_576);
        assert_eq!(DRAW_VERTEX_COUNT, 36_240);
    }
}
