// Tileable volume noise matching Nubis 2015/2017 recipe.
// Base: 128^3 RGBA (R Perlin-Worley, GBA Worley rising freq).
// Detail: 32^3 RGB Worley. Curl: 128^2 RG for domain warp.
// Generator is Seb Hillaire TileableVolumeNoise style: integer lattice
// hash, trilinear value noise for Perlin, Voronoi F1 for Worley, fBm octaves.
// Deterministic seed. No OS entropy. Boot-time cost only.

pub const BASE_N: usize = 32;
pub const DETAIL_N: usize = 16;
pub const CURL_N: usize = 32;

fn hash3(x: u32, y: u32, z: u32, seed: u32) -> u32 {
    let mut h = x.wrapping_mul(374761393)
        ^ y.wrapping_mul(668265263)
        ^ z.wrapping_mul(2147483647)
        ^ seed.wrapping_mul(1442695041);
    h = (h ^ (h >> 13)).wrapping_mul(1274126177);
    h ^ (h >> 16)
}

fn rand01_cell(x: u32, y: u32, z: u32, seed: u32) -> f32 {
    (hash3(x, y, z, seed) & 0xffffff) as f32 / 16777216.0
}

fn fade(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Tileable value noise in [0,1].
fn value_noise(x: f32, y: f32, z: f32, cells: u32, seed: u32) -> f32 {
    let xi = x.floor() as i32;
    let yi = y.floor() as i32;
    let zi = z.floor() as i32;
    let xf = x - xi as f32;
    let yf = y - yi as f32;
    let zf = z - zi as f32;
    let c = cells as i32;
    let w = |dx: i32, dy: i32, dz: i32| {
        let ix = ((xi + dx) % c + c) % c;
        let iy = ((yi + dy) % c + c) % c;
        let iz = ((zi + dz) % c + c) % c;
        rand01_cell(ix as u32, iy as u32, iz as u32, seed)
    };
    let u = fade(xf);
    let v = fade(yf);
    let wgt = fade(zf);
    let a = w(0, 0, 0).lerp(w(1, 0, 0), u);
    let b = w(0, 1, 0).lerp(w(1, 1, 0), u);
    let c0 = w(0, 0, 1).lerp(w(1, 0, 1), u);
    let d = w(0, 1, 1).lerp(w(1, 1, 1), u);
    a.lerp(b, v).lerp(c0.lerp(d, v), wgt)
}

trait Lerp {
    fn lerp(self, other: Self, t: f32) -> Self;
}
impl Lerp for f32 {
    fn lerp(self, other: Self, t: f32) -> Self {
        self + (other - self) * t
    }
}

/// Tileable Worley F1 in [0,1]. Inverted by caller for billows.
fn worley(x: f32, y: f32, z: f32, cells: u32, seed: u32) -> f32 {
    let xi = x.floor() as i32;
    let yi = y.floor() as i32;
    let zi = z.floor() as i32;
    let c = cells as i32;
    let mut best = 3.0f32;
    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let cx = ((xi + dx) % c + c) % c;
                let cy = ((yi + dy) % c + c) % c;
                let cz = ((zi + dz) % c + c) % c;
                // Feature point inside cell from hash.
                let px = cx as f32 + rand01_cell(cx as u32, cy as u32, cz as u32, seed);
                let py = cy as f32 + rand01_cell(cx as u32, cy as u32, cz as u32, seed ^ 0x9e37);
                let pz = cz as f32 + rand01_cell(cx as u32, cy as u32, cz as u32, seed ^ 0x51f3);
                let d = ((x - px).powi(2) + (y - py).powi(2) + (z - pz).powi(2)).sqrt();
                if d < best {
                    best = d;
                }
            }
        }
    }
    (best / 1.75).clamp(0.0, 1.0)
}

fn fbm_perlin(x: f32, y: f32, z: f32, cells: u32, oct: u32, seed: u32) -> f32 {
    let mut acc = 0.0;
    let mut amp = 0.5;
    let mut tot = 0.0;
    for o in 0..oct {
        let f = (cells << o).min(64);
        let s = x * (1 << o) as f32 / (BASE_N as f32 / cells as f32);
        let _ = s;
        acc += amp * value_noise(x * (1 << o) as f32, y * (1 << o) as f32, z * (1 << o) as f32, f, seed + o * 101);
        tot += amp;
        amp *= 0.5;
    }
    acc / tot.max(1e-6)
}

/// Perlin-Worley composite from Schneider 2017: remap Perlin by inverted Worley.
pub fn perlin_worley(px: f32, py: f32, pz: f32, cells: u32, seed: u32) -> f32 {
    let p = fbm_perlin(px, py, pz, cells, 3, seed);
    let w = worley(px, py, pz, cells * 2, seed ^ 0xabcd);
    // remap(perlin, 1-worley, 1, 0, 1)
    let inv_w = 1.0 - w;
    ((p * (1.0 - 0.0) + inv_w * 0.0) * 0.5 + p * 0.5 * inv_w + p * 0.25).clamp(0.0, 1.0)
}

/// Generate base volume RGBA8. Size BASE_N^3 * 4.
pub fn generate_base() -> Vec<u8> {
    let n = BASE_N;
    let mut out = vec![0u8; n * n * n * 4];
    for z in 0..n {
        for y in 0..n {
            for x in 0..n {
                let fx = x as f32 / n as f32 * 4.0;
                let fy = y as f32 / n as f32 * 4.0;
                let fz = z as f32 / n as f32 * 4.0;
                let r = perlin_worley(fx, fy, fz, 4, 7);
                let g = 1.0 - worley(fx * 2.0, fy * 2.0, fz * 2.0, 8, 11);
                let b = 1.0 - worley(fx * 4.0, fy * 4.0, fz * 4.0, 16, 13);
                let a = 1.0 - worley(fx * 8.0, fy * 8.0, fz * 8.0, 32, 17);
                let o = (z * n * n + y * n + x) * 4;
                out[o] = (r.clamp(0.0, 1.0) * 255.0) as u8;
                out[o + 1] = (g.clamp(0.0, 1.0) * 255.0) as u8;
                out[o + 2] = (b.clamp(0.0, 1.0) * 255.0) as u8;
                out[o + 3] = (a.clamp(0.0, 1.0) * 255.0) as u8;
            }
        }
    }
    out
}

/// Generate detail volume RGB8. Size DETAIL_N^3 * 4 (padded to RGBA).
pub fn generate_detail() -> Vec<u8> {
    let n = DETAIL_N;
    let mut out = vec![0u8; n * n * n * 4];
    for z in 0..n {
        for y in 0..n {
            for x in 0..n {
                let fx = x as f32 / n as f32 * 4.0;
                let fy = y as f32 / n as f32 * 4.0;
                let fz = z as f32 / n as f32 * 4.0;
                let r = 1.0 - worley(fx, fy, fz, 4, 31);
                let g = 1.0 - worley(fx * 2.0, fy * 2.0, fz * 2.0, 8, 37);
                let b = 1.0 - worley(fx * 4.0, fy * 4.0, fz * 4.0, 16, 41);
                let o = (z * n * n + y * n + x) * 4;
                out[o] = (r.clamp(0.0, 1.0) * 255.0) as u8;
                out[o + 1] = (g.clamp(0.0, 1.0) * 255.0) as u8;
                out[o + 2] = (b.clamp(0.0, 1.0) * 255.0) as u8;
                out[o + 3] = 255;
            }
        }
    }
    out
}

/// Generate 2D curl-ish warp RG8. Size CURL_N^2 * 4.
pub fn generate_curl() -> Vec<u8> {
    let n = CURL_N;
    let mut out = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let fx = x as f32 / n as f32 * 6.0;
            let fy = y as f32 / n as f32 * 6.0;
            // Cheap curl from value noise gradients.
            let e = 0.35;
            let n1 = value_noise(fx, fy, 1.7, 6, 91);
            let n2 = value_noise(fx + e, fy, 1.7, 6, 91);
            let n3 = value_noise(fx, fy + e, 1.7, 6, 91);
            let gx = (n2 - n1) / e;
            let gy = (n3 - n1) / e;
            // Divergence-free-ish warp in [-1,1] packed to [0,255].
            let cx = (-gy * 0.5).clamp(-1.0, 1.0);
            let cy = (gx * 0.5).clamp(-1.0, 1.0);
            let o = (y * n + x) * 4;
            out[o] = ((cx * 0.5 + 0.5) * 255.0) as u8;
            out[o + 1] = ((cy * 0.5 + 0.5) * 255.0) as u8;
            out[o + 2] = 0;
            out[o + 3] = 255;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volumes_have_range_and_size() {
        let b = generate_base();
        assert_eq!(b.len(), BASE_N * BASE_N * BASE_N * 4);
        let min = *b.iter().min().unwrap();
        let max = *b.iter().max().unwrap();
        assert!(max > min + 20, "min={min} max={max}");
        let d = generate_detail();
        assert_eq!(d.len(), DETAIL_N * DETAIL_N * DETAIL_N * 4);
        let c = generate_curl();
        assert_eq!(c.len(), CURL_N * CURL_N * 4);
    }

    #[test]
    fn noise_tileable_integers_match() {
        // Integer lattice wraps: value at 0 and at cells must match.
        let a = value_noise(0.0, 0.0, 0.0, 4, 7);
        let b = value_noise(4.0, 0.0, 0.0, 4, 7);
        assert!((a - b).abs() < 1e-5, "{a} vs {b}");
    }
}
