// GPU resource builders for FX passes. CPU parts tested; Vulkan upload
// helpers mirror the weave/LUT path in plane.rs so review stays local.
// Layout plan (no change to airframe set 0..4):
// - fx set layout: 0 UBO (same buffer), 1 base_vol sampled, 2 base sampler,
//   3 detail_vol sampled, 4 detail sampler, 5 scene HDR sampled, 6 scene sampler.
// - plume pipeline: cone proxy verts (pos12 + axial4 + radial4 = 20 B).
// - trail pipeline: ribbon verts (center12 + side12 + age4 + density4 +
//   flow_uv8 + seed4 + radius4 + ice4 = 52 B).
// - composite pipeline: fullscreen triangle, no vertex input.

use glam::Vec3;

pub const PLUME_VERT_BYTES: usize = 20;
pub const TRAIL_VERT_BYTES: usize = 52;
// Fixed per-frame ribbon budget. fill_trail packs newest-first up to this
// many quads per emitter; record() draws the matching static index pattern.
pub const TRAIL_MAX_QUADS_PER_EMITTER: usize = 256;
// A closed box only bounds fragment work; the shader integrates a smooth volume.
pub const CONE_INDEX_COUNT: u32 = 36;

#[derive(Clone, Copy)]
pub struct PlumeVert {
    pub pos: [f32; 3],
    pub axial: f32,
    pub radial: f32,
}

pub fn build_plume_cone(length: f32, radius: f32) -> (Vec<PlumeVert>, Vec<u16>) {
    let verts = [
        [-radius, -radius, 0.0], [radius, -radius, 0.0],
        [radius, radius, 0.0], [-radius, radius, 0.0],
        [-radius, -radius, -length], [radius, -radius, -length],
        [radius, radius, -length], [-radius, radius, -length],
    ].into_iter().map(|pos| PlumeVert {
        axial: -pos[2] / length, radial: 1.0, pos,
    }).collect();
    let idx = vec![
        0, 1, 2, 0, 2, 3, 4, 6, 5, 4, 7, 6,
        0, 4, 5, 0, 5, 1, 3, 2, 6, 3, 6, 7,
        0, 3, 7, 0, 7, 4, 1, 5, 6, 1, 6, 2,
    ];
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
        assert_eq!(v.len(), 8);
        assert_eq!(idx.len(), CONE_INDEX_COUNT as usize);
        assert_eq!(idx.len() % 3, 0);
        assert!(idx.iter().all(|&i| (i as usize) < v.len()));
        // Lip ring at x=0, tip at x=-length.
        assert!(v[0].pos[2].abs() < 1e-5);
        assert!((v.last().unwrap().pos[2] + 8.0).abs() < 1e-4);
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
        assert_eq!(idx.len() % 3, 0);
        for tri in idx.chunks_exact(3) {
            let emitter = tri[0] as usize / (TRAIL_MAX_QUADS_PER_EMITTER * 2);
            assert!(tri.iter().all(|&v| v as usize / (TRAIL_MAX_QUADS_PER_EMITTER * 2) == emitter));
        }
    }
}
