use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use crate::geology::region_at;
use crate::hydro::{river_at, water_level_at};
use crate::noise::{hypot3, noise, smooth, valley_center};
use crate::surface::terrain_sample_full;

#[derive(Default)]
struct FxHasher {
    hash: u64,
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.hash = (self.hash.rotate_left(5) ^ b as u64).wrapping_mul(0x517c_c1b7_2722_0a95);
        }
    }

    #[inline]
    fn write_u64(&mut self, v: u64) {
        self.hash = (self.hash.rotate_left(5) ^ v).wrapping_mul(0x517c_c1b7_2722_0a95);
        self.hash =
            (self.hash.rotate_left(5) ^ v.rotate_right(32)).wrapping_mul(0x517c_c1b7_2722_0a95);
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}

type FxMap<K, V> = HashMap<K, V, BuildHasherDefault<FxHasher>>;

#[inline]
fn fx_map<K: std::hash::Hash + Eq, V>(capacity: usize) -> FxMap<K, V> {
    HashMap::with_capacity_and_hasher(capacity, BuildHasherDefault::default())
}

pub(crate) const TILE_F: f64 = 1024.0;
const LOD_STEPS: [f64; 4] = [16.0, 32.0, 64.0, 128.0];
const HORIZON_RADIUS: i32 = 24;
const HORIZON_INNER: i32 = 8;
const HORIZON_MID: i32 = 16;
const INNER_BASE: f64 = 64.0;
const MID_BASE: f64 = 128.0;
const OUTER_BASE: f64 = 256.0;
const REFINED: f64 = 32.0;

type Sample = [f64; 5];

struct WalkState {
    seed: u32,

    ox: f64,
    oz: f64,

    memo_seed: Option<u32>,
    samples: FxMap<(u64, u64), Sample>,
    lod: [FxMap<(u64, u64), [f64; 4]>; 4],
    decisions: FxMap<i64, f64>,
    dedup: FxMap<(u64, u64), u32>,

    evals: u64,
    hits: u64,
    pos: Vec<f32>,
    idx: Vec<u32>,
    nrm: Vec<f32>,
    snow: Vec<f32>,
    glc: Vec<f32>,
    bio: Vec<f32>,
    lod_bufs: [Vec<f32>; 4],
    for_: Vec<f32>,
    col: Vec<f32>,
}

impl WalkState {
    fn new() -> Self {
        WalkState {
            seed: 0,
            ox: 0.0,
            oz: 0.0,
            memo_seed: None,

            samples: fx_map(1 << 19),
            lod: [
                fx_map(1 << 16),
                fx_map(1 << 16),
                fx_map(1 << 16),
                fx_map(1 << 16),
            ],
            decisions: fx_map(1 << 16),
            dedup: fx_map(1 << 18),
            evals: 0,
            hits: 0,
            pos: Vec::new(),
            idx: Vec::new(),
            nrm: Vec::new(),
            snow: Vec::new(),
            glc: Vec::new(),
            bio: Vec::new(),
            lod_bufs: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
            for_: Vec::new(),
            col: Vec::new(),
        }
    }

    fn reset(&mut self, cx: i32, cz: i32, seed: u32) {
        self.seed = seed;
        self.ox = cx as f64 * TILE_F;
        self.oz = cz as f64 * TILE_F;

        if self.memo_seed != Some(seed) {
            self.memo_seed = Some(seed);
            self.samples.clear();
            for memo in self.lod.iter_mut() {
                memo.clear();
            }
        } else {
            if self.samples.len() > 1 << 20 {
                self.samples.clear();
            }
            for memo in self.lod.iter_mut() {
                if memo.len() > 1 << 17 {
                    memo.clear();
                }
            }
        }
        self.decisions.clear();
        self.dedup.clear();
        self.evals = 0;
        self.hits = 0;
        self.pos.clear();
        self.idx.clear();
        self.nrm.clear();
        self.snow.clear();
        self.glc.clear();
        self.bio.clear();
        for buf in self.lod_bufs.iter_mut() {
            buf.clear();
        }
        self.for_.clear();
        self.col.clear();
    }

    #[inline]
    fn sample(&mut self, wx: f64, wz: f64) -> Sample {
        let key = (wx.to_bits(), wz.to_bits());
        if let Some(&hit) = self.samples.get(&key) {
            self.hits += 1;
            return hit;
        }
        self.evals += 1;
        let mut out = [0.0f64; 9];
        terrain_sample_full(wx, wz, self.seed, &mut out);
        let sample = [out[0], out[2], out[3], out[5], out[6]];
        self.samples.insert(key, sample);
        sample
    }

    fn sample_into(&mut self, lx: f64, lz: f64, spacing: f64, out: &mut [f64; 6]) {
        let wx = self.ox + lx;
        let wz = self.oz + lz;
        let seed = self.seed;
        let h = self.sample(wx, wz)[0];
        let nx = self.sample(wx - spacing, wz)[0] - self.sample(wx + spacing, wz)[0];
        let ny = spacing * 2.0;
        let nz = self.sample(wx, wz - spacing)[0] - self.sample(wx, wz + spacing)[0];
        let length = hypot3(nx, ny, nz);
        let slope = ny / length;
        let ao = 1.0f64.min(0.68f64.max(0.74 + slope * 0.26));
        let macro_var = 0.94 + noise(wx / 500.0, wz / 500.0, seed.wrapping_add(77)) * 0.12;
        let snow = self.sample(wx, wz)[1];
        out[0] = h;
        out[1] = nx / length;
        out[2] = ny / length;
        out[3] = nz / length;
        out[4] = snow;
        out[5] = ao * macro_var;
    }

    fn lod_probe(&mut self, lx: f64, lz: f64, spacing: f64, level: usize, out: &mut [f64]) {
        let key = ((self.ox + lx).to_bits(), (self.oz + lz).to_bits());
        if let Some(hit) = self.lod[level].get(&key) {
            out[0] = hit[0];
            out[1] = hit[1];
            out[2] = hit[2];
            out[3] = hit[3];
            return;
        }
        let mut s = [0.0f64; 6];
        self.sample_into(lx, lz, spacing, &mut s);
        let stored = [s[0], s[1], s[3], s[4]];
        self.lod[level].insert(key, stored);
        out[0] = stored[0];
        out[1] = stored[1];
        out[2] = stored[2];
        out[3] = stored[3];
    }

    fn triangle_sample(&mut self, lx: f64, lz: f64, step: f64, level: usize, out: &mut [f64; 4]) {
        let gx = (lx / step).floor() * step;
        let gz = (lz / step).floor() * step;
        let u = (lx - gx) / step;
        let v = (lz - gz) / step;
        let mut corners = [0.0f64; 16];
        for (k, (dx, dz)) in [(0.0, 0.0), (step, 0.0), (0.0, step), (step, step)]
            .iter()
            .enumerate()
        {
            let mut probe = [0.0f64; 4];
            self.lod_probe(gx + dx, gz + dz, step, level, &mut probe);
            corners[k * 4..k * 4 + 4].copy_from_slice(&probe);
        }
        if u + v <= 1.0 {
            for i in 0..4 {
                let value = corners[i];
                let b = corners[4 + i];
                let c = corners[8 + i];
                out[i] = value + (b - value) * u + (c - value) * v;
            }
        } else {
            for i in 0..4 {
                let d = corners[12 + i];
                let c = corners[8 + i];
                let b = corners[4 + i];
                out[i] = d + (c - d) * (1.0 - u) + (b - d) * (1.0 - v);
            }
        }
    }

    fn biome_4(&self, wx: f64, wz: f64, h: f64) -> [f64; 4] {
        let seed = self.seed;
        let region = region_at(wx, wz, seed);
        let warp = (noise(wx / 2300.0, wz / 2300.0, seed.wrapping_add(901)) - 0.5) * 900.0;
        let dry = noise(
            (wx + warp) / 1750.0,
            (wz - warp) / 1750.0,
            seed.wrapping_add(902),
        );
        let lush = noise(wx / 1450.0, wz / 1450.0, seed.wrapping_add(903));

        let golden =
            smooth(0.55, 0.74, dry) * smooth(420.0, 700.0, h) * (1.0 - smooth(1700.0, 2100.0, h));
        let green =
            smooth(0.57, 0.78, lush) * smooth(160.0, 320.0, h) * (1.0 - smooth(1050.0, 1400.0, h));
        let lz = (wz + 900.0) / 1400.0;
        let lx = (wx - valley_center(wz)) / 800.0;
        let shore = region.lakes.max((-(lz * lz) - lx * lx).exp());
        let wet = (1.0 - smooth(141.0, 189.0, h)) * smooth(0.3, 0.62, shore);
        let moor = smooth(0.42, 0.66, region.upland)
            * smooth(800.0, 1020.0, h)
            * (1.0 - smooth(1560.0, 1900.0, h))
            * (1.0 - smooth(0.55, 0.8, region.mountains));
        [golden, green, wet, moor]
    }

    fn stand_mask(&self, wx: f64, wz: f64) -> f64 {
        let seed = self.seed;
        let u = wx * 0.81 + wz * 0.59;
        let v = wz * 0.81 - wx * 0.59;
        let warp_x = (noise(u / 330.0, v / 330.0, seed.wrapping_add(64)) - 0.5) * 420.0;
        let warp_z = (noise(u / 410.0, v / 410.0, seed.wrapping_add(65)) - 0.5) * 420.0;
        let grove = noise(
            (u + warp_x) / 620.0,
            (v + warp_z) / 760.0,
            seed.wrapping_add(61),
        );
        let fringe = (noise(wx / 85.0, wz / 85.0, seed.wrapping_add(62)) - 0.5) * 0.1;
        smooth(0.43, 0.58, grove + fringe)
    }

    fn forest_cover(&self, wx: f64, wz: f64, h: f64, slope: f64, snow: f64, ice: f64) -> f64 {
        let seed = self.seed;
        let pasture = noise(wx / 850.0, wz / 850.0, seed.wrapping_add(19)) > 0.57
            && slope > 0.91
            && h < 850.0;
        if pasture || ice > 0.05 || h < water_level_at(wx, wz, seed) + 7.0 || h > 2280.0 {
            return 0.0;
        }
        self.stand_mask(wx, wz)
            * smooth(0.68, 0.82, slope)
            * (1.0 - smooth(1550.0, 2050.0, h))
            * (1.0 - smooth(0.25, 0.85, snow))
    }

    fn emit(&mut self, lx: f64, lz: f64, spacing: f64) {
        let key = (lx.to_bits(), lz.to_bits());
        if let Some(&existing) = self.dedup.get(&key) {
            self.idx.push(existing);
            return;
        }
        let mut s = [0.0f64; 6];
        self.sample_into(lx, lz, spacing, &mut s);
        let index = (self.pos.len() / 3) as u32;
        self.dedup.insert(key, index);
        self.idx.push(index);
        self.pos.push(lx as f32);
        self.pos.push(s[0] as f32);
        self.pos.push(lz as f32);
        self.nrm.push(s[1] as f32);
        self.nrm.push(s[2] as f32);
        self.nrm.push(s[3] as f32);
        self.snow.push(s[4] as f32);
        let wx = self.ox + lx;
        let wz = self.oz + lz;
        let ice = self.sample(wx, wz);
        self.glc.push(ice[2] as f32);
        self.glc.push(ice[3] as f32);
        self.glc.push(ice[4] as f32);
        let biome = self.biome_4(wx, wz, s[0]);
        self.bio.push(biome[0] as f32);
        self.bio.push(biome[1] as f32);
        self.bio.push(biome[2] as f32);
        self.bio.push(biome[3] as f32);
        self.for_
            .push(self.forest_cover(wx, wz, s[0], s[2], s[4], ice[2]) as f32);
        for level in 0..4 {
            if LOD_STEPS[level] <= spacing {
                let buf = &mut self.lod_bufs[level];
                buf.push(s[0] as f32);
                buf.push(s[1] as f32);
                buf.push(s[3] as f32);
                buf.push(s[4] as f32);
            } else {
                let mut c = [0.0f64; 4];
                self.triangle_sample(lx, lz, LOD_STEPS[level], level, &mut c);
                let buf = &mut self.lod_bufs[level];
                buf.push(c[0] as f32);
                buf.push(c[1] as f32);
                buf.push(c[2] as f32);
                buf.push(c[3] as f32);
            }
        }
        let gray = s[5] as f32;
        self.col.push(gray);
        self.col.push(gray);
        self.col.push(gray);
    }

    fn base_at(lx: f64, lz: f64) -> f64 {
        let ring = ((lx + TILE_F / 2.0) / TILE_F)
            .floor()
            .abs()
            .max(((lz + TILE_F / 2.0) / TILE_F).floor().abs());
        if ring <= HORIZON_INNER as f64 {
            INNER_BASE
        } else if ring <= HORIZON_MID as f64 {
            MID_BASE
        } else {
            OUTER_BASE
        }
    }

    fn spacing_of(&mut self, bx: f64, bz: f64, base: f64) -> f64 {
        let key = bx as i64 * 16777216
            + bz as i64
            + if base == INNER_BASE {
                281474976710656
            } else if base == MID_BASE {
                562949953421312
            } else {
                0
            };
        if let Some(&hit) = self.decisions.get(&key) {
            return hit;
        }
        if base != INNER_BASE {
            self.decisions.insert(key, base);
            return base;
        }
        let seed = self.seed;
        let wx = self.ox + bx;
        let wz = self.oz + bz;
        let (rx, width, _, _) = river_at(wz + base / 2.0, seed);
        let mut feature = (wx + base / 2.0 - rx).abs() < width * 2.0 + base;
        if !feature {
            let mut corners = [0.0f64; 9];
            corners[4] = self.sample(wx + base / 2.0, wz + base / 2.0)[0];
            if corners[4] < 600.0 {
                corners[0] = self.sample(wx, wz)[0];
                corners[1] = self.sample(wx + base, wz)[0];
                corners[2] = self.sample(wx, wz + base)[0];
                corners[3] = self.sample(wx + base, wz + base)[0];
                corners[5] = self.sample(wx + base / 2.0, wz)[0];
                corners[6] = self.sample(wx + base / 2.0, wz + base)[0];
                corners[7] = self.sample(wx, wz + base / 2.0)[0];
                corners[8] = self.sample(wx + base, wz + base / 2.0)[0];
                let mut min = corners[0];
                let mut max = corners[0];
                for i in 1..9 {
                    let v = corners[i];
                    if v < min {
                        min = v;
                    }
                    if v > max {
                        max = v;
                    }
                }
                feature = min < 128.0 && max > 122.0;
            }
        }
        let spacing = if feature { REFINED } else { base };
        self.decisions.insert(key, spacing);
        spacing
    }

    fn spacing_at(&mut self, lx: f64, lz: f64) -> f64 {
        let base = Self::base_at(lx, lz);
        self.spacing_of((lx / base).floor() * base, (lz / base).floor() * base, base)
    }

    fn subdivide(
        &mut self,
        x: f64,
        z: f64,
        spacing: f64,
        ax: f64,
        az: f64,
        bx: f64,
        bz: f64,
        neighbour: f64,
    ) {
        let segments = 1.0f64.max(spacing / neighbour);
        let count = segments as i32;
        for k in 0..count {
            let kf = k as f64;
            self.emit(x + spacing / 2.0, z + spacing / 2.0, spacing);
            self.emit(
                ax + ((bx - ax) * kf) / segments,
                az + ((bz - az) * kf) / segments,
                spacing.min(neighbour),
            );
            self.emit(
                ax + ((bx - ax) * (kf + 1.0)) / segments,
                az + ((bz - az) * (kf + 1.0)) / segments,
                spacing.min(neighbour),
            );
        }
    }

    fn patch(&mut self, x: f64, z: f64, spacing: f64) {
        let n0 = self.spacing_at(x - 1.0, z + spacing / 2.0);
        let n1 = self.spacing_at(x + spacing / 2.0, z + spacing + 1.0);
        let n2 = self.spacing_at(x + spacing + 1.0, z + spacing / 2.0);
        let n3 = self.spacing_at(x + spacing / 2.0, z - 1.0);
        if n0 >= spacing && n1 >= spacing && n2 >= spacing && n3 >= spacing {
            self.emit(x, z, spacing);
            self.emit(x, z + spacing, spacing);
            self.emit(x + spacing, z, spacing);
            self.emit(x + spacing, z, spacing);
            self.emit(x, z + spacing, spacing);
            self.emit(x + spacing, z + spacing, spacing);
            return;
        }
        self.subdivide(x, z, spacing, x, z, x, z + spacing, n0);
        self.subdivide(x, z, spacing, x, z + spacing, x + spacing, z + spacing, n1);
        self.subdivide(x, z, spacing, x + spacing, z + spacing, x + spacing, z, n2);
        self.subdivide(x, z, spacing, x + spacing, z, x, z, n3);
    }

    fn walk(&mut self) {
        for tz in -HORIZON_RADIUS..=HORIZON_RADIUS {
            for tx in -HORIZON_RADIUS..=HORIZON_RADIUS {
                let base = Self::base_at(tx as f64 * TILE_F, tz as f64 * TILE_F);
                let left = tx as f64 * TILE_F - TILE_F / 2.0;
                let top = tz as f64 * TILE_F - TILE_F / 2.0;
                let cells = (TILE_F / base) as i32;
                for j in 0..cells {
                    for i in 0..cells {
                        let x = left + i as f64 * base;
                        let z = top + j as f64 * base;
                        let spacing = self.spacing_of(x, z, base);
                        let steps = (base / spacing) as i32;
                        for dj in 0..steps {
                            for di in 0..steps {
                                self.patch(
                                    x + di as f64 * spacing,
                                    z + dj as f64 * spacing,
                                    spacing,
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

std::thread_local! {
    static WALK: std::cell::RefCell<WalkState> = std::cell::RefCell::new(WalkState::new());
}

pub(crate) fn far_tile_build(cx: i32, cz: i32, seed: u32) {
    WALK.with(|w| {
        let mut w = w.borrow_mut();
        w.reset(cx, cz, seed);
        w.walk();
    });
}

pub(crate) fn far_tile_stats(out: &mut [usize; 9]) {
    WALK.with(|w| {
        let w = w.borrow();
        out[0] = w.evals as usize;
        out[1] = w.hits as usize;
        out[2] = w.samples.len();
        out[3] = w.lod[0].len();
        out[4] = w.lod[1].len();
        out[5] = w.lod[2].len();
        out[6] = w.lod[3].len();
        out[7] = w.decisions.len();
        out[8] = w.dedup.len();
    });
}

pub(crate) fn far_tile_layout(out: &mut [usize; 24]) {
    WALK.with(|w| {
        let w = w.borrow();
        out[0] = w.pos.as_ptr() as usize;
        out[1] = w.pos.len();
        out[2] = w.idx.as_ptr() as usize;
        out[3] = w.idx.len();
        out[4] = w.nrm.as_ptr() as usize;
        out[5] = w.nrm.len();
        out[6] = w.snow.as_ptr() as usize;
        out[7] = w.snow.len();
        out[8] = w.glc.as_ptr() as usize;
        out[9] = w.glc.len();
        out[10] = w.bio.as_ptr() as usize;
        out[11] = w.bio.len();
        out[12] = w.lod_bufs[0].as_ptr() as usize;
        out[13] = w.lod_bufs[0].len();
        out[14] = w.lod_bufs[1].as_ptr() as usize;
        out[15] = w.lod_bufs[1].len();
        out[16] = w.lod_bufs[2].as_ptr() as usize;
        out[17] = w.lod_bufs[2].len();
        out[18] = w.lod_bufs[3].as_ptr() as usize;
        out[19] = w.lod_bufs[3].len();
        out[20] = w.for_.as_ptr() as usize;
        out[21] = w.for_.len();
        out[22] = w.col.as_ptr() as usize;
        out[23] = w.col.len();
    });
}
