//! Bake-time vertex-cache and meshlet analysis.
//!
//! Records ACMR, ATVR, triangle count, and downstream meshlet count/fill
//! for every part at all five LODs under each [`CacheOrder`]. ACMR and
//! ATVR come from meshoptimizer's FIFO cache model; meshlet metrics come
//! from the same `build_meshlets(64v, 126t)` call the bake pipeline uses,
//! because triangle order changes meshlet composition. The retired Forsyth
//! order was measured with this harness on 2026-09-22; its numbers live in
//! `docs/optimization_techniques/airframe-vertex-cache-order.md`.

use meshopt_rs::cluster::{build_meshlets, build_meshlets_bound, compute_meshlet_bounds, Meshlet};
use meshopt_rs::vertex::cache::analyze_vertex_cache;

use airframe::build_airframe;
use airframe_format::{MESHLET_MAX_TRIANGLES, MESHLET_MAX_VERTICES};

use crate::cache_order::{CacheOrder, ALL};
use crate::lod::{build_part_lod_indices, LOD_COUNT};

/// FIFO cache size for `analyze_vertex_cache`. Matches meshoptimizer's
/// adaptive optimizer design point; the model is an approximation of real
/// GPU post-transform caches.
pub const ANALYSIS_CACHE_SIZE: usize = 16;
/// Warp size for the cache model. 0 disables warp accounting so ACMR and
/// ATVR count pure cache misses.
pub const ANALYSIS_WARP_SIZE: usize = 0;
/// Primitive-group size for the cache model; 0 disables group flushes.
pub const ANALYSIS_PRIMGROUP_SIZE: usize = 0;

/// One part at one LOD under one order.
#[derive(Clone, Debug, PartialEq)]
pub struct LodCacheSample {
    pub order: CacheOrder,
    pub part: usize,
    pub level: usize,
    pub triangles: u32,
    /// Vertex invocations / triangle count under the FIFO cache model.
    pub acmr: f32,
    /// Vertex invocations / unique referenced vertices.
    pub atvr: f32,
    pub meshlets: u32,
    /// Sum of per-meshlet vertex counts.
    pub meshlet_vertices: u32,
    /// Meshlets whose normal cone is usable for backface rejection
    /// (`cone_cutoff < 1.0`).
    pub cone_cullable: u32,
    /// Sum of `cone_cutoff` across meshlets.
    pub cone_cutoff_sum: f32,
}

impl LodCacheSample {
    /// Triangles packed against the 126-triangle meshlet capacity.
    pub fn meshlet_fill(&self) -> f32 {
        if self.meshlets == 0 {
            return 0.0;
        }
        self.triangles as f32 / (self.meshlets * MESHLET_MAX_TRIANGLES as u32) as f32
    }

    /// Mean triangles referenced per unique vertex inside meshlets; higher
    /// means better local reuse after partitioning.
    pub fn meshlet_tris_per_vert(&self) -> f32 {
        if self.meshlet_vertices == 0 {
            return 0.0;
        }
        (self.triangles * 3) as f32 / self.meshlet_vertices as f32
    }

    pub fn cone_cullable_frac(&self) -> f32 {
        if self.meshlets == 0 {
            return 0.0;
        }
        self.cone_cullable as f32 / self.meshlets as f32
    }

    pub fn cone_mean_cutoff(&self) -> f32 {
        if self.meshlets == 0 {
            return 0.0;
        }
        self.cone_cutoff_sum / self.meshlets as f32
    }
}

/// Per-LOD aggregate over many parts for one order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LevelAggregate {
    pub parts: u32,
    pub triangles: u32,
    pub vertices_transformed: f32,
    pub unique_vertices: f32,
    pub meshlets: u32,
    pub meshlet_vertices: u32,
    pub cone_cullable: u32,
    pub cone_cutoff_sum: f32,
}

impl LevelAggregate {
    pub fn acmr(&self) -> f32 {
        if self.triangles == 0 {
            0.0
        } else {
            self.vertices_transformed / self.triangles as f32
        }
    }

    pub fn atvr(&self) -> f32 {
        if self.unique_vertices == 0.0 {
            0.0
        } else {
            self.vertices_transformed / self.unique_vertices
        }
    }

    pub fn meshlet_fill(&self) -> f32 {
        if self.meshlets == 0 {
            return 0.0;
        }
        self.triangles as f32 / (self.meshlets * MESHLET_MAX_TRIANGLES as u32) as f32
    }

    pub fn meshlet_tris_per_vert(&self) -> f32 {
        if self.meshlet_vertices == 0 {
            return 0.0;
        }
        (self.triangles * 3) as f32 / self.meshlet_vertices as f32
    }

    pub fn cone_cullable_frac(&self) -> f32 {
        if self.meshlets == 0 {
            return 0.0;
        }
        self.cone_cullable as f32 / self.meshlets as f32
    }

    pub fn cone_mean_cutoff(&self) -> f32 {
        if self.meshlets == 0 {
            return 0.0;
        }
        self.cone_cutoff_sum / self.meshlets as f32
    }

    pub fn push(&mut self, sample: &LodCacheSample) {
        self.parts += 1;
        self.triangles += sample.triangles;
        self.vertices_transformed += sample.acmr * sample.triangles as f32;
        if sample.atvr > 0.0 {
            self.unique_vertices += sample.acmr * sample.triangles as f32 / sample.atvr;
        }
        self.meshlets += sample.meshlets;
        self.meshlet_vertices += sample.meshlet_vertices;
        self.cone_cullable += sample.cone_cullable;
        self.cone_cutoff_sum += sample.cone_cutoff_sum;
    }
}

/// Full comparison output: one sample per part, level, and order.
#[derive(Debug, Default)]
pub struct CacheOrderReport {
    pub samples: Vec<LodCacheSample>,
}

impl CacheOrderReport {
    pub fn for_order(&self, order: CacheOrder) -> Vec<&LodCacheSample> {
        self.samples
            .iter()
            .filter(|s| s.order == order)
            .collect()
    }

    /// Aggregate one order across every part at one LOD level.
    pub fn level_aggregate(&self, order: CacheOrder, level: usize) -> LevelAggregate {
        let mut aggregate = LevelAggregate::default();
        for sample in &self.samples {
            if sample.order == order && sample.level == level {
                aggregate.push(sample);
            }
        }
        aggregate
    }

    /// Aggregate one order across every part and every LOD level.
    pub fn total_aggregate(&self, order: CacheOrder) -> LevelAggregate {
        let mut aggregate = LevelAggregate::default();
        for sample in &self.samples {
            if sample.order == order {
                aggregate.push(sample);
            }
        }
        aggregate
    }
}

/// Measure one reordered part-local LOD: cache statistics plus the
/// meshlets `build_meshlets` produces from that exact triangle order.
pub fn measure_lod(
    order: CacheOrder,
    part: usize,
    level: usize,
    indices: &[u32],
    positions: &[[f32; 3]],
) -> LodCacheSample {
    let triangles = (indices.len() / 3) as u32;
    let stats = analyze_vertex_cache(
        indices,
        positions.len(),
        ANALYSIS_CACHE_SIZE,
        ANALYSIS_WARP_SIZE,
        ANALYSIS_PRIMGROUP_SIZE,
    );

    let mut meshlets = 0u32;
    let mut meshlet_vertices = 0u32;
    let mut cone_cullable = 0u32;
    let mut cone_cutoff_sum = 0.0f32;
    if !indices.is_empty() {
        let bound = build_meshlets_bound(indices.len(), MESHLET_MAX_VERTICES, MESHLET_MAX_TRIANGLES);
        let mut built = vec![Meshlet::default(); bound];
        let count = build_meshlets(
            &mut built,
            indices,
            positions.len(),
            MESHLET_MAX_VERTICES,
            MESHLET_MAX_TRIANGLES,
        );
        meshlets = count as u32;
        for meshlet in built[..count].iter() {
            meshlet_vertices += meshlet.vertex_count as u32;
            let bounds = compute_meshlet_bounds(meshlet, positions);
            if bounds.cone_cutoff < 1.0 {
                cone_cullable += 1;
            }
            cone_cutoff_sum += bounds.cone_cutoff;
        }
    }

    LodCacheSample {
        order,
        part,
        level,
        triangles,
        acmr: stats.acmr,
        atvr: stats.atvr,
        meshlets,
        meshlet_vertices,
        cone_cullable,
        cone_cutoff_sum,
    }
}

/// Build every LOD of every procedural part once, then reorder and measure
/// that shared chain under each [`CacheOrder`].
pub fn compare_cache_orders() -> CacheOrderReport {
    let raw = build_airframe();
    let mut report = CacheOrderReport::default();
    for (part_index, part) in raw.iter().enumerate() {
        let positions: Vec<[f32; 3]> = part.verts.iter().map(|v| v.pos).collect();
        let raw_lods = build_part_lod_indices(&part.idx, &positions, part.importance.packed_id() as u32);
        for order in ALL {
            for (level, lod) in raw_lods.iter().enumerate() {
                let reordered = order.reorder(&lod.indices, positions.len());
                report
                    .samples
                    .push(measure_lod(order, part_index, level, &reordered, &positions));
            }
        }
    }
    report
}

/// Print the aggregate table, then every part x level sample.
pub fn print_comparison(report: &CacheOrderReport) {
    println!(
        "cache order analysis: analyze_vertex_cache(cache={}, warp={}, primgroup={}); meshlets <= {}v/{}t",
        ANALYSIS_CACHE_SIZE,
        ANALYSIS_WARP_SIZE,
        ANALYSIS_PRIMGROUP_SIZE,
        MESHLET_MAX_VERTICES,
        MESHLET_MAX_TRIANGLES,
    );
    println!(
        "{:<8} {:>4} {:>6} {:>8} {:>7} {:>7} {:>9} {:>7} {:>7} {:>7} {:>8}",
        "order", "lod", "parts", "tris", "acmr", "atvr", "meshlets", "fill%", "t/vert", "cone%", "cutoff"
    );
    for order in ALL {
        for level in 0..LOD_COUNT {
            let a = report.level_aggregate(order, level);
            if a.parts == 0 {
                continue;
            }
            print_aggregate(order.as_str(), level, &a);
        }
        let total = report.total_aggregate(order);
        print_aggregate(order.as_str(), "all", &total);
    }
    println!();
    println!(
        "{:<8} {:>5} {:>4} {:>8} {:>7} {:>7} {:>9} {:>7} {:>7} {:>7} {:>8}",
        "order", "part", "lod", "tris", "acmr", "atvr", "meshlets", "fill%", "t/vert", "cone%", "cutoff"
    );
    for sample in &report.samples {
        println!(
            "{:<8} {:>5} {:>4} {:>8} {:>7.4} {:>7.4} {:>9} {:>7.1} {:>7.3} {:>7.1} {:>8.4}",
            sample.order.as_str(),
            sample.part,
            sample.level,
            sample.triangles,
            sample.acmr,
            sample.atvr,
            sample.meshlets,
            sample.meshlet_fill() * 100.0,
            sample.meshlet_tris_per_vert(),
            sample.cone_cullable_frac() * 100.0,
            sample.cone_mean_cutoff(),
        );
    }
}

fn print_aggregate(order: &str, level: impl std::fmt::Display, a: &LevelAggregate) {
    println!(
        "{:<8} {:>4} {:>6} {:>8} {:>7.4} {:>7.4} {:>9} {:>7.1} {:>7.3} {:>7.1} {:>8.4}",
        order,
        level,
        a.parts,
        a.triangles,
        a.acmr(),
        a.atvr(),
        a.meshlets,
        a.meshlet_fill() * 100.0,
        a.meshlet_tris_per_vert(),
        a.cone_cullable_frac() * 100.0,
        a.cone_mean_cutoff(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(side: u32) -> (Vec<u32>, Vec<[f32; 3]>) {
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
        (indices, positions)
    }

    #[test]
    fn meshlet_triangles_match_the_lod_triangle_count() {
        let (indices, positions) = grid(17);
        for order in crate::cache_order::ALL {
            let reordered = order.reorder(&indices, positions.len());
            let sample = measure_lod(order, 0, 0, &reordered, &positions);
            assert!(sample.meshlets > 0, "{order:?}");
            assert_eq!(
                sample.meshlet_fill(),
                sample.triangles as f32 / (sample.meshlets * MESHLET_MAX_TRIANGLES as u32) as f32
            );
            // Every meshlet triangle is full; total packed triangles equal the LOD.
            let mut packed = 0u32;
            let bound = build_meshlets_bound(
                reordered.len(),
                MESHLET_MAX_VERTICES,
                MESHLET_MAX_TRIANGLES,
            );
            let mut built = vec![Meshlet::default(); bound];
            let count = build_meshlets(
                &mut built,
                &reordered,
                positions.len(),
                MESHLET_MAX_VERTICES,
                MESHLET_MAX_TRIANGLES,
            );
            for meshlet in built[..count].iter() {
                packed += meshlet.triangle_count as u32;
            }
            assert_eq!(packed, sample.triangles, "{order:?}");
        }
    }

    #[test]
    fn adaptive_meshopt_beats_the_unordered_grid_on_acmr() {
        let (indices, positions) = grid(17);
        let vertex_count = positions.len();
        let before = analyze_vertex_cache(
            &indices,
            vertex_count,
            ANALYSIS_CACHE_SIZE,
            ANALYSIS_WARP_SIZE,
            ANALYSIS_PRIMGROUP_SIZE,
        );
        let reordered = CacheOrder::Meshopt.reorder(&indices, vertex_count);
        let after = analyze_vertex_cache(
            &reordered,
            vertex_count,
            ANALYSIS_CACHE_SIZE,
            ANALYSIS_WARP_SIZE,
            ANALYSIS_PRIMGROUP_SIZE,
        );
        assert!(after.acmr <= before.acmr, "{} -> {}", before.acmr, after.acmr);
        assert!(after.acmr >= 0.5 && after.acmr <= 3.0);
        assert!(after.atvr >= 1.0);
    }

    #[test]
    fn comparison_covers_every_part_level_and_order() {
        let report = compare_cache_orders();
        let parts = build_airframe().len();
        assert_eq!(
            report.samples.len(),
            parts * LOD_COUNT * crate::cache_order::ALL.len()
        );
        for order in crate::cache_order::ALL {
            for level in 0..LOD_COUNT {
                let aggregate = report.level_aggregate(order, level);
                assert_eq!(aggregate.parts as usize, parts, "{order:?} lod{level}");
                assert!(aggregate.triangles > 0, "{order:?} lod{level}");
            }
        }
        // Reordering must not change triangle counts across orders at any level.
        for level in 0..LOD_COUNT {
            let counts: Vec<u32> = crate::cache_order::ALL
                .iter()
                .map(|&order| report.level_aggregate(order, level).triangles)
                .collect();
            assert!(counts.windows(2).all(|w| w[0] == w[1]), "lod{level}: {counts:?}");
        }
    }
}
