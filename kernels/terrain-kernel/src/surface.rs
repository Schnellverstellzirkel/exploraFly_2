use crate::geology::bedrock_height;
use crate::glacier::glacier_at;
use crate::hydro::SEA_LEVEL;
use crate::noise::{hash, hypot2, hypot3, js_round, noise, smooth01};

pub(crate) struct Snowpack {
    pub(crate) slope: f64,
    pub(crate) snow: f64,
    pub(crate) ice: f64,
    pub(crate) ice_depth: f64,
    pub(crate) ice_along: f64,
    pub(crate) ice_across: f64,
    pub(crate) depth: f64,
}

#[inline]
fn snowpack_at(x: f64, z: f64, seed: u32, h: f64) -> Snowpack {
    let west = bedrock_height(x - 48.0, z, seed);
    let east = bedrock_height(x + 48.0, z, seed);
    let north = bedrock_height(x, z - 48.0, seed);
    let south = bedrock_height(x, z + 48.0, seed);
    let dx = (east - west) / 96.0;
    let dz = (south - north) / 96.0;
    let slope = 1.0 / hypot3(dx, 1.0, dz);
    let basin = ((west + east + north + south - 4.0 * h) / 14.0)
        .max(-1.0)
        .min(1.0);
    let snowline = 1480.0 - dz.max(0.0) * 90.0 - basin * 180.0
        + (noise(x / 600.0, z / 600.0, seed.wrapping_add(88)) - 0.5) * 230.0;
    let ramp = ((h - snowline) / 200.0).max(0.0).min(1.0);
    let snow = ramp * ramp * (3.0 - 2.0 * ramp) * ((slope - 0.48) / 0.35).max(0.0).min(1.0);
    let glacier = glacier_at(x, z, seed, h);
    let depth = snow * (3.0 + basin.max(0.0) * 22.0) + glacier.depth;
    Snowpack {
        slope,
        snow,
        ice: glacier.cover,
        ice_depth: glacier.depth,
        ice_along: glacier.along,
        ice_across: glacier.across,
        depth,
    }
}

#[inline]
fn raw_height(x: f64, z: f64, seed: u32) -> f64 {
    let rock = bedrock_height(x, z, seed);
    if rock >= 1000.0 {
        rock + snowpack_at(x, z, seed, rock).depth
    } else {
        let ice = glacier_at(x, z, seed, rock).depth;
        if ice > 0.0 {
            rock + ice
        } else {
            rock
        }
    }
}

const PAD_LATTICE: f64 = 2048.0;
const PAD_JITTER: f64 = 500.0;
const PAD_BLEND: f64 = 1.9;
const LATTICE_MONUMENTS: [i32; 9] = [7, 8, 9, 10, 11, 20, 21, 22, 23];

#[inline]
fn pad_radius(kind: i32) -> Option<f64> {
    match kind {
        7 => Some(40.0),
        8 => Some(32.0),
        9 => Some(64.0),
        11 => Some(42.0),
        20 => Some(48.0),
        21 => Some(38.0),
        22 => Some(58.0),
        23 => Some(41.0),
        _ => None,
    }
}

#[inline]
fn pad_slope(kind: i32) -> f64 {
    match kind {
        7 => 0.7,
        8 => 0.52,
        9 => 0.62,
        11 => 0.64,
        20 => 0.66,
        21 => 0.66,
        22 => 0.6,
        23 => 0.64,
        _ => 0.6,
    }
}

#[inline]
fn pad_scale(kind: i32) -> f64 {
    match kind {
        7 => 2.8,
        8 => 3.5,
        9 => 2.1,
        11 => 2.6,
        20 => 2.7,
        21 => 2.4,
        22 => 2.3,
        23 => 2.5,
        _ => 2.5,
    }
}

struct PadParams {
    x: f64,
    z: f64,
    kind: i32,
    radius: f64,
}

#[inline]
fn pad_params(lx: i32, lz: i32, seed: u32) -> Option<PadParams> {
    if hash(lx, lz, seed.wrapping_add(914)) > 0.85 {
        return None;
    }
    let kind = LATTICE_MONUMENTS[(hash(lx, lz, seed.wrapping_add(913)) * 9.0) as usize];
    let base = pad_radius(kind)?;
    let ax =
        lx as f64 * PAD_LATTICE + (hash(lx, lz, seed.wrapping_add(911)) - 0.5) * 2.0 * PAD_JITTER;
    let az =
        lz as f64 * PAD_LATTICE + (hash(lx, lz, seed.wrapping_add(912)) - 0.5) * 2.0 * PAD_JITTER;
    let scale = pad_scale(kind) + hash(lx, lz, seed.wrapping_add(915)) * 1.2;

    Some(PadParams {
        x: ax,
        z: az,
        kind,
        radius: base * scale + 8.0 * std::f64::consts::SQRT_2,
    })
}

#[inline]
fn pad_fits(kind: i32, height: f64, slope: f64) -> bool {
    if height < SEA_LEVEL + 8.0 {
        return false;
    }
    if kind != 8 && kind != 20 && height > 2050.0 {
        return false;
    }
    if height > 2360.0 {
        return false;
    }
    slope >= pad_slope(kind)
}

pub(crate) fn terrain_sample_full(x: f64, z: f64, seed: u32, out: &mut [f64; 9]) {
    let rock = bedrock_height(x, z, seed);
    let pack = snowpack_at(x, z, seed, rock);
    let raw = if rock >= 1000.0 {
        rock + pack.depth
    } else if pack.ice_depth > 0.0 {
        rock + pack.ice_depth
    } else {
        rock
    };
    let mut height = raw;
    if let Some(p) = pad_params(
        js_round(x / PAD_LATTICE) as i32,
        js_round(z / PAD_LATTICE) as i32,
        seed,
    ) {
        let d = hypot2(x - p.x, z - p.z);
        let outer = p.radius * PAD_BLEND;
        if d < outer {
            let gh = raw_height(p.x, p.z, seed);
            let gx = (raw_height(p.x + 12.0, p.z, seed) - raw_height(p.x - 12.0, p.z, seed)) / 24.0;
            let gz = (raw_height(p.x, p.z + 12.0, seed) - raw_height(p.x, p.z - 12.0, seed)) / 24.0;
            if pad_fits(p.kind, gh, 1.0 / hypot3(gx, 1.0, gz)) {
                let t = smooth01((d - p.radius) / (outer - p.radius));
                height = gh + (raw - gh) * t;
            }
        }
    }
    out[0] = height;
    out[1] = pack.slope;
    out[2] = pack.snow;
    out[3] = pack.ice;
    out[4] = pack.ice_depth;
    out[5] = pack.ice_along;
    out[6] = pack.ice_across;
    out[7] = pack.depth;
    out[8] = rock;
}
