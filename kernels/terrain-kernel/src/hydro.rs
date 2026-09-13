use crate::noise::{noise, smooth, valley_center};

pub(crate) const SEA_LEVEL: f64 = 125.0;
const LAKE_Z_NORTH: f64 = -3400.0;
const LAKE_Z_SOUTH: f64 = 700.0;

#[inline]
fn river_path_x(z: f64, seed: u32) -> f64 {
    valley_center(z)
        + (z / 430.0).sin() * 75.0
        + (z / 920.0).sin() * 35.0
        + (noise(z / 1100.0, 0.0, seed.wrapping_add(601)) - 0.5) * 80.0
}

#[inline]
pub(crate) fn river_at(z: f64, seed: u32) -> (f64, f64, f64, f64) {
    let x = river_path_x(z, seed);
    let upstream_south = (z - LAKE_Z_SOUTH).max(0.0);
    let upstream_north = (LAKE_Z_NORTH - z).max(0.0);
    let upstream = upstream_south.max(upstream_north);
    let delta_bonus = (1.0 - smooth(0.0, 2200.0, upstream)) * 32.0;
    let width = 36.0 + noise(z / 710.0, 0.0, seed.wrapping_add(602)) * 16.0 + delta_bonus;
    let mut level = SEA_LEVEL;
    if z > LAKE_Z_SOUTH {
        let d = z - LAKE_Z_SOUTH;
        level = SEA_LEVEL + 40.0 * (1.0 - 1.0 / (1.0 + (d / 6200.0).powf(1.35)));
    } else if z < LAKE_Z_NORTH {
        let d = LAKE_Z_NORTH - z;
        level = SEA_LEVEL + 35.0 * (1.0 - 1.0 / (1.0 + (d / 6200.0).powf(1.35)));
    }
    (x, width, level, upstream)
}

#[inline]
pub(crate) fn river_bed(x: f64, z: f64, seed: u32, ground: f64) -> f64 {
    let (rx, width, level, upstream) = river_at(z, seed);
    let across = (x - rx).abs() / width;
    if across >= 2.0 {
        return ground;
    }
    let channel_depth = 4.8 + (1.0 - smooth(0.0, 3000.0, upstream)) * 3.5;
    let bed = level - channel_depth + (across * across).min(1.0) * 2.2;
    ground + (ground.min(bed) - ground) * (1.0 - smooth(1.0, 2.0, across))
}

#[inline]
pub(crate) fn water_level_at(x: f64, z: f64, seed: u32) -> f64 {
    let (rx, width, level, _) = river_at(z, seed);
    let across = (x - rx).abs() / width;
    if level <= SEA_LEVEL + 0.001 {
        return SEA_LEVEL;
    }
    if across <= 1.5 {
        return level;
    }
    SEA_LEVEL + (level - SEA_LEVEL) * (1.0 - smooth(1.5, 2.4, across))
}
