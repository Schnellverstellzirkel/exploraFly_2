//! Offline LOD generation: QEM edge-collapse simplification per procedural
//! part. Indices stay in the part-local vertex space so every level still
//! addresses the original packed stream; only triangles are removed.

use meshopt_rs::vertex::Position;

/// Number of baked LOD levels per part (0 is full resolution).
pub const LOD_COUNT: usize = 5;

/// Geometric error budget per level as a fraction of the part's bounding-box
/// max extent. Level 0 is exact. These become the object-space errors the
/// task shader projects to pixels for screen-space LOD selection.
pub(crate) const LOD_ERROR_BUDGET: [f32; LOD_COUNT] = [0.0, 0.001, 0.004, 0.016, 0.064];

/// Target triangle count per level as a fraction of level 0.
pub(crate) const LOD_TRIANGLE_RATIO: [f32; LOD_COUNT] = [1.0, 0.5, 0.25, 0.125, 0.0625];

/// One simplified index buffer for a part at a single LOD level.
pub(crate) struct PartLod {
    /// Part-local triangle indices, cache-reordered.
    pub indices: Vec<u32>,
    /// Object-space geometric error upper bound in meters.
    pub error: f32,
    /// Triangle count of this level.
    pub triangles: u32,
}

/// Max bounding-box dimension of a position list (meters).
pub(crate) fn extent(positions: &[[f32; 3]]) -> f32 {
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for p in positions {
        for axis in 0..3 {
            lo[axis] = lo[axis].min(p[axis]);
            hi[axis] = hi[axis].max(p[axis]);
        }
    }
    if !lo[0].is_finite() {
        return 0.0;
    }
    (0..3)
        .map(|axis| hi[axis] - lo[axis])
        .fold(0.0f32, f32::max)
}

/// Axis-aligned bounding sphere of a position list: center + radius.
pub(crate) fn bounds(positions: &[[f32; 3]]) -> [f32; 4] {
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for p in positions {
        for axis in 0..3 {
            lo[axis] = lo[axis].min(p[axis]);
            hi[axis] = hi[axis].max(p[axis]);
        }
    }
    if !lo[0].is_finite() {
        return [0.0, 0.0, 0.0, 0.0];
    }
    let center = [
        0.5 * (lo[0] + hi[0]),
        0.5 * (lo[1] + hi[1]),
        0.5 * (lo[2] + hi[2]),
    ];
    let mut radius = 0.0f32;
    for p in positions {
        let d = (p[0] - center[0]).powi(2)
            + (p[1] - center[1]).powi(2)
            + (p[2] - center[2]).powi(2);
        radius = radius.max(d);
    }
    [center[0], center[1], center[2], radius.sqrt()]
}

/// Build every LOD of one part. Level 0 is the reordered full-resolution
/// index buffer; higher levels edge-collapse toward the triangle ratios while
/// staying under the per-level error budget. Animated silhouettes survive
/// because simplification runs inside the part and never collapses across
/// node or material boundaries.
pub(crate) fn build_part_lods(
    full_indices: &[u32],
    positions: &[[f32; 3]],
) -> Vec<PartLod> {
    let diag = extent(positions);
    let full_tris = full_indices.len() / 3;
    let mut lods = Vec::with_capacity(LOD_COUNT);
    let reordered = airframe::forsyth::reorder(full_indices);
    lods.push(PartLod {
        indices: reordered,
        error: 0.0,
        triangles: full_tris as u32,
    });
    if full_tris == 0 {
        for level in 1..LOD_COUNT {
            lods.push(PartLod {
                indices: Vec::new(),
                error: LOD_ERROR_BUDGET[level] * diag,
                triangles: 0,
            });
        }
        return lods;
    }
    let mut current = full_indices.to_vec();
    for level in 1..LOD_COUNT {
        let target_tris = ((full_tris as f32 * LOD_TRIANGLE_RATIO[level]).round() as usize)
            .max(1)
            .min(full_tris);
        let target_indices = (target_tris * 3).min(current.len());
        let budget = LOD_ERROR_BUDGET[level];
        let mut destination = vec![0u32; current.len()];
        let count = if target_indices < current.len() {
            meshopt_rs::simplify::simplify(
                &mut destination,
                &current,
                positions,
                target_indices,
                budget,
            )
        } else {
            current.len()
        };
        let simplified = &destination[..count];
        let reordered = airframe::forsyth::reorder(simplified);
        lods.push(PartLod {
            indices: reordered,
            error: budget * diag,
            triangles: (count / 3) as u32,
        });
        current = destination[..count].to_vec();
    }
    lods
}

/// Position adapter for the meshopt `Position` trait.
pub(crate) struct Pos(pub [f32; 3]);

impl Position for Pos {
    fn pos(&self) -> [f32; 3] {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad() -> (Vec<u32>, Vec<[f32; 3]>) {
        let positions = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        let indices = vec![0, 1, 2, 0, 2, 3];
        (indices, positions)
    }

    #[test]
    fn lod_chain_is_monotone_in_triangles_and_error() {
        let (indices, positions) = quad();
        let lods = build_part_lods(&indices, &positions);
        assert_eq!(lods.len(), LOD_COUNT);
        assert_eq!(lods[0].error, 0.0);
        assert_eq!(lods[0].triangles, 2);
        for window in lods.windows(2) {
            assert!(window[1].triangles <= window[0].triangles);
            assert!(window[1].error >= window[0].error);
        }
        for lod in &lods {
            assert_eq!(lod.indices.len(), lod.triangles as usize * 3);
        }
    }

    #[test]
    fn empty_part_still_produces_every_level() {
        let lods = build_part_lods(&[], &[]);
        assert_eq!(lods.len(), LOD_COUNT);
        assert!(lods.iter().all(|l| l.triangles == 0));
    }

    #[test]
    fn extent_and_bounds_handle_empty_input() {
        assert_eq!(extent(&[]), 0.0);
        assert_eq!(bounds(&[]), [0.0, 0.0, 0.0, 0.0]);
        let b = bounds(&[[0.0, 0.0, 0.0], [2.0, 0.0, 0.0]]);
        assert!((b[0] - 1.0).abs() < 1e-6);
        assert!((b[3] - 1.0).abs() < 1e-6);
    }
}
