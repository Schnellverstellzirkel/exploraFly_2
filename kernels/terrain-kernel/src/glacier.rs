use std::collections::HashMap;

use crate::geology::{bedrock_height, region_at, tributary_at};
use crate::noise::{hash, js_round, noise, smooth, valley_center};

const GLACIER_START: f64 = 2300.0;
const GLACIER_STEP: f64 = 80.0;
const GLACIER_COUNT: usize = 33;

std::thread_local! {



    static SPINES: std::cell::RefCell<(u32, HashMap<(i32, i32), [f64; GLACIER_COUNT]>)> =
        std::cell::RefCell::new((u32::MAX, HashMap::new()));
}

fn glacier_spine(cell: i32, side: i32, seed: u32) -> [f64; GLACIER_COUNT] {
    if let Some(hit) = SPINES.with(|s| {
        let s = s.borrow();
        if s.0 == seed {
            s.1.get(&(cell, side)).copied()
        } else {
            None
        }
    }) {
        return hit;
    }
    let mut bed = [0.0f64; GLACIER_COUNT + 6];
    for (k, slot) in bed.iter_mut().enumerate() {
        let distance = GLACIER_START + (k as i32 - 3) as f64 * GLACIER_STEP;
        let z = tributary_at(cell, side, distance, seed);
        let x = valley_center(z) + side as f64 * distance;
        *slot = bedrock_height(x, z, seed);
    }
    let mut result = [0.0f64; GLACIER_COUNT];
    for i in 0..GLACIER_COUNT {
        let mut average = 0.0;
        let mut weight = 0.0;
        for k in -3i32..=3 {
            let w = (4 - k.abs()) as f64;
            average += bed[i + (k + 3) as usize] * w;
            weight += w;
        }
        let distance = GLACIER_START + i as f64 * GLACIER_STEP;
        let thickness = 45.0 + smooth(GLACIER_START, 4200.0, distance) * 150.0;
        let prev = if i > 0 {
            result[i - 1] + GLACIER_STEP * 0.12
        } else {
            0.0
        };
        result[i] = prev.max(average / weight + thickness);
    }
    SPINES.with(|s| {
        let mut s = s.borrow_mut();
        if s.0 != seed {
            s.1.clear();
            s.0 = seed;
        }
        if s.1.len() >= 512 {
            s.1.clear();
        }
        s.1.insert((cell, side), result);
    });
    result
}

pub(crate) struct GlacierSample {
    pub(crate) depth: f64,
    pub(crate) cover: f64,
    pub(crate) along: f64,
    pub(crate) across: f64,
}

#[inline]
pub(crate) fn glacier_at(x: f64, z: f64, seed: u32, rock: f64) -> GlacierSample {
    let center = valley_center(z);
    let distance = (x - center).abs();
    let side: i32 = if x < center { -1 } else { 1 };
    let mut best = GlacierSample {
        depth: 0.0,
        cover: 0.0,
        along: 0.0,
        across: 0.0,
    };
    if distance >= GLACIER_START && distance <= 4800.0 && region_at(x, z, seed).mountains >= 0.65 {
        let cell = js_round(z / 2700.0) as i32;
        for i in -1i32..=1 {
            let branch = cell + i;
            if hash(branch, side, seed.wrapping_add(604)) > 0.8 {
                continue;
            }
            let axis = tributary_at(branch, side, distance, seed);
            let length = smooth(GLACIER_START, GLACIER_START + 420.0, distance)
                * (1.0 - smooth(4510.0, 4800.0, distance));
            let width = (170.0 + smooth(2600.0, 4450.0, distance) * 370.0) * length.sqrt();
            if width < 1.0 {
                continue;
            }
            let across = (z - axis) / width;
            if across.abs() > 1.12 {
                continue;
            }
            let profile = glacier_spine(branch, side, seed);
            let t = (distance - GLACIER_START) / GLACIER_STEP;
            let index = (GLACIER_COUNT as f64 - 2.0).min(t.floor());
            let i = index as usize;
            let level =
                profile[i] + (profile[i + 1] - profile[i]) * (t - index) - across * across * 42.0;
            let edge = 1.0 - smooth(0.82, 1.12, across.abs());
            let fill = (level - rock).max(0.0) * length * edge;

            let ogive_arc = distance - (1.0 - across * across * 0.82) * 115.0;
            let ogive_phase = ogive_arc / 125.0
                + noise(across * 2.5, distance / 450.0, seed.wrapping_add(605)) * 0.3;
            let ogive_wave = (ogive_phase * std::f64::consts::PI * 2.0).sin()
                * 1.4
                * smooth(GLACIER_START + 300.0, GLACIER_START + 800.0, distance)
                * (1.0 - smooth(4200.0, 4650.0, distance));

            let shear_angle = across.abs() * 38.0;
            let u_crack = distance
                + shear_angle
                + (noise(across * 4.0, distance / 120.0, seed.wrapping_add(606)) - 0.5) * 8.0;
            let cross_meters = across * 220.0;
            let cell_w = ((cross_meters + 300.0) / 48.0).floor() as i32;
            let row_u = (u_crack / 55.0).floor() as i32;
            let cell_hash = hash(cell_w, row_u, seed.wrapping_add(607));
            let cell_hash2 = hash(cell_w * 3 + 7, row_u * 2 + 13, seed.wrapping_add(608));
            let crack_u = (((u_crack + cell_hash * 55.0) % 55.0) + 55.0) % 55.0 - 27.5;
            let crack_w = (((cross_meters + 300.0) % 48.0) + 48.0) % 48.0 - 24.0;
            let crack_taper = 1.0 - smooth(15.0, 23.0, crack_w.abs());
            let crack_width = 1.6 + cell_hash2 * 1.2;
            let crack_relief = (1.0 - smooth(0.4, crack_width, crack_u.abs()))
                * crack_taper
                * 1.6
                * smooth(GLACIER_START + 380.0, GLACIER_START + 950.0, distance)
                * (1.0 - smooth(4000.0, 4650.0, distance))
                * (1.0 - smooth(0.72, 1.02, across.abs()));

            let depth = (fill + ogive_wave * (fill / 15.0).min(1.0)
                - crack_relief * (fill / 15.0).min(1.0))
            .max(0.0);
            if depth > best.depth {
                best.depth = depth;
                best.cover = (depth / 12.0).min(1.0);
                best.along = distance - GLACIER_START;
                best.across = across;
            }
        }
    }
    best
}
