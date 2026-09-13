// Procedural glider airframe. Ported from the web prototype.
// Same planform, same parts, new storage: indexed triangles,
// one interleaved stream, baked flex weights, merged batches.
// Forward is +z here, so source z is negated and triangles flip.

use glam::Vec3;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MatId {
    Sail,
    Composite,
    Graphite,
    Titanium,
    Dark,
    Seat,
    Glass,
    Glow,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Node {
    Hull,
    Canopy,
    WingL,
    WingR,
    Flap(u8),
    Rotor,
    Petal(u8),
    Fin(u8),
}

pub struct RawVert {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub flex: f32,
}

pub struct RawPart {
    pub node: Node,
    pub mat: MatId,
    pub verts: Vec<RawVert>,
    pub idx: Vec<u32>,
}

pub fn f32_to_f16(v: f32) -> u16 {
    // Round-to-nearest-even fp32 to fp16 bits. No NaN payload care.
    let bits = v.to_bits();
    let sign = (bits >> 16) & 0x8000;
    let exp = ((bits >> 23) & 0xff) as i32 - 112;
    let mant = bits & 0x7fffff;
    if exp >= 31 {
        return (sign | 0x7bff) as u16;
    }
    if exp <= 0 {
        if exp < -10 {
            return sign as u16;
        }
        let shifted = (mant | 0x800000) >> (1 - exp) as u32;
        let half = (shifted + 0xfff + ((shifted >> 13) & 1)) >> 13;
        return (sign | half) as u16;
    }
    let half = ((mant + 0xfff + ((mant >> 13) & 1)) >> 13) as u32;
    (sign | ((exp as u32) << 10) | half.min(0x3ff)) as u16
}

pub fn oct_encode(n: Vec3) -> [u16; 2] {
    // Octahedral normal map to SNORM16 pair.
    let l = n.x.abs() + n.y.abs() + n.z.abs();
    let mut x = n.x / l;
    let mut y = n.y / l;
    if n.z < 0.0 {
        let ox = x;
        x = (1.0 - y.abs()) * ox.signum();
        y = (1.0 - ox.abs()) * y.signum();
    }
    let enc = |v: f32| ((v.clamp(-1.0, 1.0) * 32767.0).round() as i32) as u16;
    [enc(x), enc(y)]
}

#[allow(dead_code)]
/// CPU-side twin of the shader decode. Kept for future picking.
pub fn oct_decode(pair: [u16; 2]) -> Vec3 {
    let dec = |v: u16| (v as i16) as f32 / 32767.0;
    let (x, y) = (dec(pair[0]), dec(pair[1]));
    let mut z = 1.0 - x.abs() - y.abs();
    let (mut nx, mut ny) = (x, y);
    if z < 0.0 {
        nx = (1.0 - y.abs()) * x.signum();
        ny = (1.0 - x.abs()) * y.signum();
        z = 1.0 - nx.abs() - ny.abs();
    }
    Vec3::new(nx, ny, z).normalize_or_zero()
}
