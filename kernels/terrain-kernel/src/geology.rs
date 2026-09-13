use std::f64::consts::PI;

use crate::hydro::river_bed;
use crate::noise::{gradient_noise, hash, hypot2, js_round, noise, smooth, valley_center};

pub(crate) struct Region {
    pub(crate) mountains: f64,
    pub(crate) upland: f64,
    pub(crate) lakes: f64,
}

#[inline]
pub(crate) fn region_at(x: f64, z: f64, seed: u32) -> Region {
    let province = noise(
        x / 10500.0 + 0.37,
        z / 10500.0 - 0.21,
        seed.wrapping_add(401),
    );
    let hr_x = x / 8500.0;
    let hr_z = z / 11000.0;
    let home_range = (-(hr_x * hr_x) - hr_z * hr_z).exp();
    let mountains = home_range.max(smooth(0.48, 0.73, province));
    let upland = smooth(0.3, 0.52, province) * (1.0 - mountains);
    let lakes = (1.0 - smooth(0.22, 0.38, province)) * (1.0 - mountains);
    Region {
        mountains,
        upland,
        lakes,
    }
}

struct Catchment {
    x: f64,
    z: f64,
    floor: f64,
    stretch: f64,
}

#[inline]
fn catchment(cx: i32, cz: i32, seed: u32) -> Catchment {
    let x = (cx as f64 + (hash(cx, cz, seed.wrapping_add(410)) - 0.5) * 0.75) * 2400.0;
    let z = (cz as f64 + (hash(cx, cz, seed.wrapping_add(411)) - 0.5) * 0.75) * 2400.0;
    let floor = 1250.0 + hash(cx, cz, seed.wrapping_add(412)) * 650.0;
    let stretch = 0.75 + hash(cx, cz, seed.wrapping_add(413)) * 0.5;
    Catchment {
        x,
        z,
        floor,
        stretch,
    }
}

#[inline]
pub(crate) fn tributary_at(cell: i32, side: i32, distance: f64, seed: u32) -> f64 {
    let phase = hash(cell, side, seed.wrapping_add(430)) * PI * 2.0;
    cell as f64 * 2700.0
        + (hash(cell, side, seed.wrapping_add(431)) - 0.5) * 1000.0
        + (distance / 1500.0 + phase).sin() * 230.0
}

pub(crate) fn eroded_bedrock(x: f64, z: f64, seed: u32) -> f64 {
    let region = region_at(x, z, seed);
    let center = valley_center(z);
    let distance = (x - center).abs();
    let floor = 112.0 + noise(x / 1400.0, z / 1400.0, seed.wrapping_add(7)) * 70.0;
    let lz = (z + 900.0) / 1200.0;
    let lx = (x - center) / 430.0;
    let lake = (-(lz * lz) - lx * lx).exp() * 62.0;
    let lowland = 105.0 + noise(x / 2600.0, z / 2600.0, seed.wrapping_add(420)) * 140.0
        - region.lakes * 145.0;
    let hills = gradient_noise(x / 1100.0, z / 950.0, seed.wrapping_add(421)) * 150.0;
    let plateau = 430.0 + noise(x / 3400.0, z / 3400.0, seed.wrapping_add(422)) * 450.0 + hills;
    let countryside = lowland + hills * (1.0 - region.lakes) * 0.35 + region.upland * plateau;
    if region.mountains < 0.001 {
        return countryside;
    }

    let bx = js_round(x / 2400.0) as i32;
    let bz = js_round(z / 2400.0) as i32;
    let mut eroded = f64::INFINITY;
    for dz in -1i32..=1 {
        for dx in -1i32..=1 {
            let basin = catchment(bx + dx, bz + dz, seed);
            let rx = (x - basin.x) * basin.stretch;
            let rz = (z - basin.z) / basin.stretch;
            let radius = hypot2(rx, rz);
            let bowl = basin.floor + 0.1 * radius + (radius - 380.0).max(0.0).powf(1.22) * 0.48;
            eroded = eroded.min(bowl);
        }
    }
    let uplift = 3000.0 + gradient_noise(x / 3200.0, z / 2800.0, seed.wrapping_add(425)) * 620.0;
    let mut massif = uplift.min(eroded);
    let side: f64 = if x < center { -1.0 } else { 1.0 };
    let branch_cell = js_round(z / 2700.0) as i32;
    for i in -1i32..=1 {
        let cell = branch_cell + i;
        let branch = tributary_at(cell, side as i32, distance, seed);
        let cross = (z - branch).abs();
        let width = 150.0 + smooth(600.0, 3200.0, distance) * 460.0;
        let bed = floor + 90.0 + (distance - 500.0).max(0.0) * 0.43;
        let trough = bed + (cross / width).powf(2.4) * 420.0;
        let reach = 1.0 - smooth(3500.0, 5100.0, distance);
        massif -= (massif - trough).max(0.0) * reach;
    }
    let wall = smooth(320.0, 2450.0, distance);
    let valley_surface = floor - lake + wall * (massif - floor).max(0.0);
    let rock = wall * smooth(220.0, 1050.0, valley_surface);
    let strike = x * 0.88 + z * 0.47;
    let dip = z * 0.88 - x * 0.47;
    let gullies =
        (gradient_noise(strike / 110.0, dip / 430.0, seed.wrapping_add(440)).abs() - 0.35) * 210.0;
    let ribs =
        (gradient_noise(strike / 38.0, dip / 150.0, seed.wrapping_add(441)).abs() - 0.32) * 58.0;
    let scree = gradient_noise(x / 27.0, z / 31.0, seed.wrapping_add(442)) * 4.0;
    let alpine = valley_surface
        + rock * (gullies + ribs + scree)
        + gradient_noise(x / 160.0, z / 160.0, seed.wrapping_add(443)) * (1.5 + wall * 5.0);
    countryside + (alpine - countryside) * region.mountains
}

pub(crate) fn bedrock_height(x: f64, z: f64, seed: u32) -> f64 {
    river_bed(x, z, seed, eroded_bedrock(x, z, seed))
}
