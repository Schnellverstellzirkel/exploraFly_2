#[inline]
pub(crate) fn hash(x: i32, z: i32, seed: u32) -> f64 {
    let n = x.wrapping_mul(374761393) ^ z.wrapping_mul(668265263) ^ seed as i32;
    let n = (n ^ ((n as u32 >> 13) as i32)).wrapping_mul(1274126177);
    let n = n as u32;
    ((n ^ (n >> 16)) as f64) / 4294967296.0
}

#[inline]
pub(crate) fn quintic(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
pub(crate) fn noise(x: f64, z: f64, seed: u32) -> f64 {
    let ix = x.floor();
    let iz = z.floor();
    let u = quintic(x - ix);
    let v = quintic(z - iz);
    let a = hash(ix as i32, iz as i32, seed);
    let b = hash(ix as i32 + 1, iz as i32, seed);
    let c = hash(ix as i32, iz as i32 + 1, seed);
    let d = hash(ix as i32 + 1, iz as i32 + 1, seed);
    (a + (b - a) * u) * (1.0 - v) + (c + (d - c) * u) * v
}

#[inline]
pub(crate) fn smooth(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
pub(crate) fn valley_center(z: f64) -> f64 {
    (z / 1900.0).sin() * 460.0 + (z / 4700.0).sin() * 650.0
}

#[inline]
pub(crate) fn hypot2(a: f64, b: f64) -> f64 {
    let ax = a.abs();
    let ab = b.abs();
    let m = ax.max(ab);
    if m == 0.0 {
        return 0.0;
    }
    let sa = a / m;
    let sb = b / m;
    (sa * sa + sb * sb).sqrt() * m
}

#[inline]
pub(crate) fn hypot3(a: f64, b: f64, c: f64) -> f64 {
    let a = a.abs();
    let b = b.abs();
    let c = c.abs();

    let max = a.max(b).max(c);
    if max == 0.0 {
        return 0.0;
    }
    let power_a = (a / max) * (a / max);
    let power_b = (b / max) * (b / max);
    let compensation = (power_a + power_b) - power_a - power_b;
    let power_c = (c / max) * (c / max) - compensation;
    (power_a + power_b + power_c).sqrt() * max
}

#[inline]
pub(crate) fn js_round(x: f64) -> f64 {
    let r = (x + 0.5).floor();
    if r == 0.0 && x < 0.0 {
        -0.0
    } else {
        r
    }
}

#[inline]
pub(crate) fn smooth01(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

const DIRECTIONS: [(f64, f64); 8] = {
    const D: f64 = std::f64::consts::FRAC_1_SQRT_2;
    [
        (1.0, 0.0),
        (-1.0, 0.0),
        (0.0, 1.0),
        (0.0, -1.0),
        (D, D),
        (-D, D),
        (D, -D),
        (-D, -D),
    ]
};

#[inline]
fn gradient_dot(x: f64, z: f64, ix: f64, iz: f64, dx: f64, dz: f64, seed: u32) -> f64 {
    let g = DIRECTIONS[(hash(ix as i32 + dx as i32, iz as i32 + dz as i32, seed) * 8.0) as usize];
    g.0 * (x - ix - dx) + g.1 * (z - iz - dz)
}

#[inline]
pub(crate) fn gradient_noise(x: f64, z: f64, seed: u32) -> f64 {
    let ix = x.floor();
    let iz = z.floor();
    let u = x - ix;
    let v = z - iz;
    let a = gradient_dot(x, z, ix, iz, 0.0, 0.0, seed);
    let b = gradient_dot(x, z, ix, iz, 1.0, 0.0, seed);
    let c = gradient_dot(x, z, ix, iz, 0.0, 1.0, seed);
    let d = gradient_dot(x, z, ix, iz, 1.0, 1.0, seed);
    let fu = quintic(u);
    let fv = quintic(v);
    ((a + (b - a) * fu) * (1.0 - fv) + (c + (d - c) * fu) * fv) * 1.42
}
