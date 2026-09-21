//! Packed GPU vertex helpers: half-float UVs and octahedral normals.
//!
//! These encode procedural geometry into `airframe_format::VERTEX_BYTES`.
//! The `airframe` crate stays free of format-specific packing.

use glam::Vec3;

/// Convert IEEE 754 float32 to half-precision float16 bits with round-to-nearest-even.
pub(crate) fn f32_to_f16(v: f32) -> u16 {
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
    let half = (mant + 0xfff + ((mant >> 13) & 1)) >> 13;
    (sign | ((exp as u32) << 10) | half.min(0x3ff)) as u16
}

/// Encode a unit normal vector onto an octahedron mapped to 2x 16-bit signed normalized integers (SNORM16).
pub(crate) fn oct_encode(n: Vec3) -> [u16; 2] {
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

/// CPU-side twin of the shader decode. Kept for future picking.
#[allow(dead_code)]
pub(crate) fn oct_decode(pair: [u16; 2]) -> Vec3 {
    let dec = |v: u16| (v as i16) as f32 / 32767.0;
    let (x, y) = (dec(pair[0]), dec(pair[1]));
    let z = 1.0 - x.abs() - y.abs();
    let (mut nx, mut ny) = (x, y);
    if z < 0.0 {
        nx = (1.0 - y.abs()) * x.signum();
        ny = (1.0 - x.abs()) * y.signum();
    }
    Vec3::new(nx, ny, z).normalize_or_zero()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn octahedral_roundtrip_preserves_both_hemispheres() {
        for x in -8..=8 {
            for y in -8..=8 {
                for z in -8..=8 {
                    let v = Vec3::new(x as f32, y as f32, z as f32);
                    if v == Vec3::ZERO {
                        continue;
                    }
                    let n = v.normalize();
                    let decoded = oct_decode(oct_encode(n));
                    assert!((decoded - n).length() < 0.00015, "{n:?} -> {decoded:?}");
                }
            }
        }
    }

    #[test]
    fn f16_rounds_representable_floats_exactly() {
        assert_eq!(f32_to_f16(0.0), 0);
        assert_eq!(f32_to_f16(-0.0) & 0x7fff, 0);
        assert_eq!(f32_to_f16(1.0), 0x3c00);
        assert_eq!(f32_to_f16(2.0), 0x4000);
        assert_ne!(f32_to_f16(f32::INFINITY) & 0x7fff, 0);
    }
}
