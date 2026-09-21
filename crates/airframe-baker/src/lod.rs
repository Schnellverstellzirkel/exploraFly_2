//! Offline LOD generation: QEM edge-collapse simplification per procedural
//! part. Indices stay in the part-local vertex space so every level still
//! addresses the original packed stream; only triangles are removed.
//!
//! Budgets and target ratios are selected from the part's semantic
//! `Importance` so silhouette geometry holds shape while interior and
//! hardware parts collapse early. Error growth and RT density come from
//! `Importance::policy()`; triangle ratios stay per-class tables.

use airframe::{Importance, LodPolicy};
use meshopt_rs::vertex::Position;

use crate::cache_order::CacheOrder;

/// Number of baked LOD levels per part (0 is full resolution).
pub const LOD_COUNT: usize = 5;

/// Base geometric error budget per level as a fraction of the part's
/// bounding-box max extent for the structural class (`lod_error_scale = 1.0`).
/// Level 0 is exact. Per-class rows are `BASE_LOD_ERROR_BUDGET *
/// Importance::policy().lod_error_scale`. These become the object-space
/// errors the task shader projects to pixels for screen-space LOD selection.
pub(crate) const BASE_LOD_ERROR_BUDGET: [f32; LOD_COUNT] =
    [0.0, 0.001, 0.004, 0.016, 0.064];

/// Target triangle count per level as a fraction of level 0, indexed by
/// importance. Ratios must stay non-increasing per row.
pub(crate) const LOD_TRIANGLE_RATIO: [[f32; LOD_COUNT]; airframe_format::IMPORTANCE_COUNT as usize] =
    [
        [1.0, 0.6, 0.35, 0.2, 0.1],
        [1.0, 0.5, 0.25, 0.125, 0.0625],
        [1.0, 0.4, 0.15, 0.05, 0.02],
        [1.0, 0.3, 0.1, 0.03, 0.01],
        [1.0, 0.5, 0.2, 0.05, 0.02],
    ];

/// RT shadow-proxy geometric error budget as a fraction of the part extent.
pub(crate) const RT_ERROR_BUDGET: f32 = 0.04;

/// Parts with fewer triangles than this contribute no RT geometry at all.
pub(crate) const RT_MIN_SOURCE_TRIANGLES: usize = 4;

/// Rendering policy for a packed importance nibble, clamped to a defined class.
pub(crate) fn importance_policy(importance: u32) -> LodPolicy {
    let id = importance.min(airframe_format::IMPORTANCE_COUNT - 1) as u8;
    match Importance::from_packed(id) {
        Some(class) => class.policy(),
        None => Importance::Structural.policy(),
    }
}

fn clamp_importance(importance: u32) -> usize {
    (importance as usize).min(airframe_format::IMPORTANCE_COUNT as usize - 1)
}

/// Per-level error budget row for one importance class.
pub(crate) fn lod_error_budget(importance: u32) -> [f32; LOD_COUNT] {
    let scale = importance_policy(importance).lod_error_scale;
    BASE_LOD_ERROR_BUDGET.map(|budget| budget * scale)
}

/// Per-level triangle-ratio row for one importance class.
pub(crate) fn lod_triangle_ratio(importance: u32) -> &'static [f32; LOD_COUNT] {
    &LOD_TRIANGLE_RATIO[clamp_importance(importance)]
}

/// One simplified index buffer for a part at a single LOD level.
pub(crate) struct PartLod {
    /// Part-local triangle indices, cache-reordered.
    pub indices: Vec<u32>,
    /// Object-space geometric error upper bound in meters.
    pub error: f32,
    /// Triangle count of this level.
    pub triangles: u32,
}

/// One simplified index buffer before any cache reorder.
pub(crate) struct RawPartLod {
    /// Part-local triangle indices in simplifier output order.
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

/// Build the simplified index buffer for every LOD of one part, before any
/// cache reorder. Level 0 is the full-resolution indices; higher levels
/// edge-collapse toward the importance-specific triangle ratios while
/// staying under the per-level error budget. Animated silhouettes survive
/// because simplification runs inside the part and never collapses across
/// node or material boundaries. `importance` selects how aggressively each
/// level may give up triangles.
pub(crate) fn build_part_lod_indices(
    full_indices: &[u32],
    positions: &[[f32; 3]],
    importance: u32,
) -> Vec<RawPartLod> {
    let budgets = lod_error_budget(importance);
    let ratios = lod_triangle_ratio(importance);
    let diag = extent(positions);
    let full_tris = full_indices.len() / 3;
    let mut lods = Vec::with_capacity(LOD_COUNT);
    lods.push(RawPartLod {
        indices: full_indices.to_vec(),
        error: 0.0,
        triangles: full_tris as u32,
    });
    if full_tris == 0 {
        for &budget in budgets.iter().take(LOD_COUNT).skip(1) {
            lods.push(RawPartLod {
                indices: Vec::new(),
                error: budget * diag,
                triangles: 0,
            });
        }
        return lods;
    }
    let mut current = full_indices.to_vec();
    for level in 1..LOD_COUNT {
        let target_tris = ((full_tris as f32 * ratios[level]).round() as usize)
            .max(1)
            .min(full_tris);
        let target_indices = (target_tris * 3).min(current.len());
        let budget = budgets[level];
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
        lods.push(RawPartLod {
            indices: destination[..count].to_vec(),
            error: budget * diag,
            triangles: (count / 3) as u32,
        });
        current = destination[..count].to_vec();
    }
    lods
}

/// Build every LOD of one part and reorder each level with `order`.
/// Triangle counts and errors come from [`build_part_lod_indices`]; only
/// the index order differs between strategies.
pub(crate) fn build_part_lods(
    full_indices: &[u32],
    positions: &[[f32; 3]],
    importance: u32,
    order: CacheOrder,
) -> Vec<PartLod> {
    build_part_lod_indices(full_indices, positions, importance)
        .into_iter()
        .map(|raw| PartLod {
            indices: order.reorder(&raw.indices, positions.len()),
            error: raw.error,
            triangles: raw.triangles,
        })
        .collect()
}

/// Position adapter for the meshopt `Position` trait.
pub(crate) struct Pos(pub [f32; 3]);

impl Position for Pos {
    fn pos(&self) -> [f32; 3] {
        self.0
    }
}

/// Build the dedicated ray-tracing proxy for one opaque part. Aggressively
/// simplified and independent of the visual LOD chain so tiny detail never
/// reaches the shadow BLAS. `rt_density` is `Importance::policy().rt_density`:
/// `0.0` contributes nothing (emitters cast no meaningful shadow) as long as
/// every animation node keeps some other RT geometry. Returns part-local indices.
pub(crate) fn build_rt_proxy(
    full_indices: &[u32],
    positions: &[[f32; 3]],
    rt_density: f32,
    order: CacheOrder,
) -> Vec<u32> {
    if rt_density <= 0.0 {
        return Vec::new();
    }
    let source_tris = full_indices.len() / 3;
    if source_tris < RT_MIN_SOURCE_TRIANGLES {
        return Vec::new();
    }
    let target_tris = ((source_tris as f32 * rt_density).round() as usize)
        .max(1)
        .min(source_tris);
    let target_indices = target_tris * 3;
    if target_indices >= full_indices.len() {
        return order.reorder(full_indices, positions.len());
    }
    let mut destination = vec![0u32; full_indices.len()];
    let count = meshopt_rs::simplify::simplify(
        &mut destination,
        full_indices,
        positions,
        target_indices,
        RT_ERROR_BUDGET,
    );
    order.reorder(&destination[..count], positions.len())
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
        for importance in 0..airframe_format::IMPORTANCE_COUNT {
            let lods = build_part_lods(&indices, &positions, importance, CacheOrder::Meshopt);
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
    }

    #[test]
    fn interior_budgets_are_looser_than_silhouette_budgets() {
        for level in 1..LOD_COUNT {
            assert!(
                lod_error_budget(airframe_format::IMPORTANCE_INTERIOR)[level]
                    > lod_error_budget(airframe_format::IMPORTANCE_SILHOUETTE)[level]
            );
            assert!(
                lod_triangle_ratio(airframe_format::IMPORTANCE_INTERIOR)[level]
                    < lod_triangle_ratio(airframe_format::IMPORTANCE_SILHOUETTE)[level]
            );
        }
    }

    #[test]
    fn empty_part_still_produces_every_level() {
        let lods = build_part_lods(
            &[],
            &[],
            airframe_format::IMPORTANCE_STRUCTURAL,
            CacheOrder::Meshopt,
        );
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

    #[test]
    fn rt_proxy_drops_sub_detail_parts_and_shrinks_large_ones() {
        assert!(
            build_rt_proxy(
                &[0, 1, 2],
                &[[0.0; 3]; 3],
                importance_policy(airframe_format::IMPORTANCE_STRUCTURAL).rt_density,
                CacheOrder::Meshopt,
            )
            .is_empty()
        );
        assert!(
            build_rt_proxy(
                &[0, 1, 2, 3, 4, 5],
                &[[0.0; 3]; 6],
                importance_policy(airframe_format::IMPORTANCE_EMITTER).rt_density,
                CacheOrder::Meshopt,
            )
            .is_empty()
        );
        // 32x32 grid: QEM collapses far below the source triangle count.
        let side = 33u32;
        let mut positions = Vec::new();
        for z in 0..side {
            for x in 0..side {
                positions.push([x as f32 * 0.1, 0.0, z as f32 * 0.1]);
            }
        }
        let mut indices = Vec::new();
        for z in 0..side - 1 {
            for x in 0..side - 1 {
                let a = z * side + x;
                let b = a + 1;
                let c = a + side;
                let d = c + 1;
                indices.extend_from_slice(&[a, b, d, a, d, c]);
            }
        }
        let proxy = build_rt_proxy(
            &indices,
            &positions,
            importance_policy(airframe_format::IMPORTANCE_STRUCTURAL).rt_density,
            CacheOrder::Meshopt,
        );
        assert!(!proxy.is_empty());
        assert!(proxy.len() < indices.len());
        assert_eq!(proxy.len() % 3, 0);
        assert!(proxy.iter().all(|&i| (i as usize) < positions.len()));
    }
}
