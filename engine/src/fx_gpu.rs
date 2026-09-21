// GPU resource builders for FX passes. CPU parts tested; Vulkan upload
// helpers mirror the weave/LUT path in plane.rs so review stays local.
// Layout plan (no change to airframe set 0..4):
// - fx set layout: 0 UBO (same buffer), 1 base_vol sampled, 2 base sampler,
//   3 detail_vol sampled, 4 detail sampler, 5 scene HDR sampled, 6 scene sampler.
// - plume pipeline: tapered-cylinder proxy verts (pos12 + axial4 + radial4 = 20 B).
// - trail pipeline: ribbon verts (center12 + side12 + age4 + density4 +
//   flow_uv8 + seed4 + radius4 + ice4 = 52 B).
// - composite pipeline: fullscreen triangle, no vertex input.

use glam::Vec3;

pub const PLUME_VERT_BYTES: usize = 20;
pub const TRAIL_VERT_BYTES: usize = 52;
// Fixed per-frame ribbon budget. fill_trail packs newest-first up to this
// many quads per emitter; record() draws the matching static index pattern.
pub const TRAIL_MAX_QUADS_PER_EMITTER: usize = 256;
pub const CONE_SEGMENTS: usize = 32;
// Two side triangles and two cap triangles per segment.
pub const CONE_INDEX_COUNT: u32 = (CONE_SEGMENTS * 12) as u32;

#[derive(Clone, Copy)]
pub struct PlumeVert {
    pub pos: [f32; 3],
    pub axial: f32,
    pub radial: f32,
}

pub fn build_plume_cone(length: f32, radius: f32) -> (Vec<PlumeVert>, Vec<u16>) {
    let mut verts = Vec::with_capacity(CONE_SEGMENTS * 2 + 2);
    for axial in [0.0, 1.0] {
        for i in 0..CONE_SEGMENTS {
            let angle = std::f32::consts::TAU * i as f32 / CONE_SEGMENTS as f32;
            verts.push(PlumeVert {
                pos: [radius * angle.cos(), radius * angle.sin(), -length * axial],
                axial,
                radial: 1.0,
            });
        }
    }
    let start_center = (CONE_SEGMENTS * 2) as u16;
    verts.push(PlumeVert {
        pos: [0.0, 0.0, 0.0],
        axial: 0.0,
        radial: 0.0,
    });
    let end_center = start_center + 1;
    verts.push(PlumeVert {
        pos: [0.0, 0.0, -length],
        axial: 1.0,
        radial: 0.0,
    });

    let mut idx = Vec::with_capacity(CONE_INDEX_COUNT as usize);
    for i in 0..CONE_SEGMENTS {
        let next = (i + 1) % CONE_SEGMENTS;
        let start = i as u16;
        let start_next = next as u16;
        let end = start + CONE_SEGMENTS as u16;
        let end_next = start_next + CONE_SEGMENTS as u16;
        // Side, start cap, and end cap close the conservative volume proxy.
        idx.extend_from_slice(&[start, end, start_next, start_next, end, end_next]);
        idx.extend_from_slice(&[start_center, start, start_next]);
        idx.extend_from_slice(&[end_center, end_next, end]);
    }
    (verts, idx)
}

/// Independent triangle lists. Each emitter owns a fixed vertex range.
pub fn build_trail_indices() -> Vec<u16> {
    let q = TRAIL_MAX_QUADS_PER_EMITTER;
    let mut idx = Vec::with_capacity(5 * (q - 1) * 6);
    for e in 0..5usize {
        let base = (e * q * 2) as u16;
        for k in 0..(q - 1) {
            let a = base + (k * 2) as u16;
            idx.extend_from_slice(&[a, a + 1, a + 2, a + 1, a + 3, a + 2]);
        }
    }
    idx
}
/// Pack one trail segment pair into two ribbon verts (CPU side).
/// center/prev define tangent; side is camera-facing offset dir * width.
#[allow(clippy::too_many_arguments)]
pub fn ribbon_quad(
    center: Vec3,
    prev: Vec3,
    cam_dir: Vec3,
    radius: f32,
    age: f32,
    density: f32,
    flow_uv: [f32; 2],
    seed: f32,
    ice: f32,
) -> [[f32; 13]; 2] {
    let tangent = (center - prev).normalize_or_zero();
    let mut side = tangent.cross(cam_dir).normalize_or_zero() * radius;
    if side.length_squared() < 1e-8 {
        side = Vec3::new(radius, 0.0, 0.0);
    }
    let c = [center.x, center.y, center.z];
    let s = [side.x, side.y, side.z];
    [
        [c[0], c[1], c[2], s[0], s[1], s[2], age, density, flow_uv[0], flow_uv[1], seed, radius, ice],
        [c[0], c[1], c[2], -s[0], -s[1], -s[2], age, density, flow_uv[0], flow_uv[1] + 1.0, seed, radius, ice],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cone_counts_hold() {
        let (v, idx) = build_plume_cone(8.0, 0.5);
        assert_eq!(v.len(), CONE_SEGMENTS * 2 + 2);
        assert_eq!(idx.len(), CONE_INDEX_COUNT as usize);
        assert_eq!(idx.len() % 3, 0);
        assert!(idx.iter().all(|&i| (i as usize) < v.len()));
        assert!(v[..CONE_SEGMENTS].iter().all(|p| p.pos[2].abs() < 1e-5));
        assert!(v[CONE_SEGMENTS..CONE_SEGMENTS * 2]
            .iter()
            .all(|p| (p.pos[2] + 8.0).abs() < 1e-4));
        assert!(v
            .iter()
            .take(CONE_SEGMENTS)
            .all(|p| { (p.pos[0].hypot(p.pos[1]) - 0.5).abs() < 1e-5 }));
    }

    #[test]
    fn ribbon_degenerate_tangent_safe() {
        let q = ribbon_quad(Vec3::ZERO, Vec3::ZERO, Vec3::Z, 0.3, 1.0, 0.8, [0.0, 0.0], 0.5, 0.2);
        assert!(q[0][3].is_finite() && q[1][3].is_finite());
    }

    #[test]
    fn trail_indices_stay_in_range() {
        let idx = build_trail_indices();
        let max_vert = 5 * TRAIL_MAX_QUADS_PER_EMITTER * 2;
        assert!(idx.iter().all(|&i| (i as usize) < max_vert));
        assert!(idx.len().is_multiple_of(3));
        for tri in idx.as_chunks::<3>().0.iter() {
            let emitter = tri[0] as usize / (TRAIL_MAX_QUADS_PER_EMITTER * 2);
            assert!(tri.iter().all(|&v| v as usize / (TRAIL_MAX_QUADS_PER_EMITTER * 2) == emitter));
        }
    }
}
