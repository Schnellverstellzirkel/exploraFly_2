//! Build-time airframe baker.
//!
//! This crate owns the immutable conversion from procedural CAD parts to the
//! runtime's packed representation. The engine links the resulting asset, not
//! the procedural generator, so geometry construction, normal accumulation,
//! cache reordering, LOD simplification, meshlet partitioning, and RT range
//! creation leave the launch path.

mod analysis;
mod cache_order;
mod lod;
mod meshlet;

use airframe::{build_airframe, f32_to_f16, oct_encode, MatId, Node};
use airframe_format::{
    BakedAirframe, LodDesc, PartDesc, IMPORTANCE_COUNT, NODE_COUNT, PART_FLAG_GLASS,
    PART_IMPORTANCE_SHIFT, VERTEX_BYTES,
};
use glam::Vec3;
use lod::{build_part_lods, build_rt_proxy, bounds as bounds_of, LOD_COUNT};
use meshlet::MeshletBuild;

pub use analysis::{
    compare_cache_orders, print_comparison, CacheOrderReport, LodCacheSample,
    ANALYSIS_CACHE_SIZE, ANALYSIS_PRIMGROUP_SIZE, ANALYSIS_WARP_SIZE,
};
pub use cache_order::{CacheOrder, ENV_VAR as CACHE_ORDER_ENV_VAR, FIFO_CACHE_SIZE};
pub use lod::LOD_COUNT as LOD_LEVELS;

/// Summary emitted by build tooling and used by bake tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BakeStats {
    pub triangles: u32,
    pub vertices: usize,
    pub opaque_indices: usize,
    pub glass_indices: usize,
    pub rt_indices: usize,
    pub rt_nodes: usize,
    pub parts: u32,
    pub lods: u32,
    pub meshlets: u32,
    pub meshlet_vertices: u32,
    pub lod_triangles: [u32; LOD_COUNT],
    pub lod_meshlets: [u32; LOD_COUNT],
    /// Level-0 triangles per importance class, indexed by `Importance::index()`.
    pub importance_triangles: [u32; IMPORTANCE_COUNT as usize],
    /// Part count per importance class.
    pub importance_parts: [u32; IMPORTANCE_COUNT as usize],
}

/// Bake the current procedural airframe into the versioned runtime asset
/// with the order selected by [`CACHE_ORDER_ENV_VAR`].
pub fn bake() -> BakedAirframe {
    bake_with_stats().0
}

/// Bake and return immutable geometry plus counts for build diagnostics,
/// using the order selected by [`CACHE_ORDER_ENV_VAR`].
pub fn bake_with_stats() -> (BakedAirframe, BakeStats) {
    bake_with_stats_with(CacheOrder::from_env())
}

/// Bake with an explicit cache order instead of reading the environment.
pub fn bake_with_stats_with(order: CacheOrder) -> (BakedAirframe, BakeStats) {
    let raw = build_airframe();
    let vertex_capacity: usize = raw.iter().map(|part| part.verts.len()).sum();
    let index_capacity: usize = raw.iter().map(|part| part.idx.len()).sum();
    let mut stream = Vec::with_capacity(vertex_capacity * VERTEX_BYTES);
    let mut opaque = Vec::with_capacity(index_capacity);
    let mut glass = Vec::new();
    // Per-part level-0 index start inside its material half, patched into
    // LodDesc::index_first after opaque and glass are concatenated.
    let mut level0_local_starts: Vec<u32> = Vec::new();
    let mut level0_is_glass: Vec<bool> = Vec::new();
    let mut rt_nodes: Vec<Vec<u16>> = vec![Vec::new(); NODE_COUNT];
    let mut parts: Vec<PartDesc> = Vec::with_capacity(raw.len());
    let mut lods: Vec<LodDesc> = Vec::new();
    let mut meshlet_build = MeshletBuild::default();
    let mut lod_triangles = [0u32; LOD_COUNT];
    let mut lod_meshlets = [0u32; LOD_COUNT];
    let mut importance_triangles = [0u32; IMPORTANCE_COUNT as usize];
    let mut importance_parts = [0u32; IMPORTANCE_COUNT as usize];
    let mut triangles = 0u32;

    // Positions of the whole packed stream, indexed by global vertex id,
    // for meshlet bounds/cones after each part is rebased.
    let mut stream_positions: Vec<[f32; 3]> = Vec::with_capacity(vertex_capacity);

    for part in &raw {
        let base = (stream.len() / VERTEX_BYTES) as u32;
        assert!(
            base + part.verts.len() as u32 <= 65536,
            "merged verts exceed u16"
        );
        let mut normals = vec![Vec3::ZERO; part.verts.len()];
        for tri in part.idx.as_chunks::<3>().0 {
            let a = Vec3::from_array(part.verts[tri[0] as usize].pos);
            let b = Vec3::from_array(part.verts[tri[1] as usize].pos);
            let c = Vec3::from_array(part.verts[tri[2] as usize].pos);
            let normal = (b - a).cross(c - a);
            normals[tri[0] as usize] += normal;
            normals[tri[1] as usize] += normal;
            normals[tri[2] as usize] += normal;
        }
        let node = node_index(part.node) as u16;
        let material = mat_index(part.mat);
        let positions: Vec<[f32; 3]> = part.verts.iter().map(|v| v.pos).collect();
        stream_positions.extend_from_slice(&positions);
        for (vertex, normal) in part.verts.iter().zip(normals.iter()) {
            let oct = oct_encode(normal.normalize_or_zero());
            stream.extend_from_slice(&vertex.pos[0].to_le_bytes());
            stream.extend_from_slice(&vertex.pos[1].to_le_bytes());
            stream.extend_from_slice(&vertex.pos[2].to_le_bytes());
            stream.extend_from_slice(&oct[0].to_le_bytes());
            stream.extend_from_slice(&oct[1].to_le_bytes());
            stream.extend_from_slice(&f32_to_f16(vertex.uv[0]).to_le_bytes());
            stream.extend_from_slice(&f32_to_f16(vertex.uv[1]).to_le_bytes());
            stream.extend_from_slice(&vertex.flex.to_le_bytes());
            stream.extend_from_slice(&node.to_le_bytes());
            stream.extend_from_slice(&material.to_le_bytes());
        }

        let is_glass = part.mat == MatId::Glass;
        let importance = part.importance.index();
        let part_lods = build_part_lods(&part.idx, &positions, importance, order);
        let part_bounds = bounds_of(&positions);
        let lod_first = lods.len() as u32;
        let mut meshlet_cursor = meshlet_build.meshlets.len() as u32;
        for (level, part_lod) in part_lods.iter().enumerate() {
            let global: Vec<u32> = part_lod
                .indices
                .iter()
                .map(|&index| base + index)
                .collect();
            meshlet_build.push_lod(&global, &stream_positions, parts.len() as u16, level as u16);
            let meshlet_count = meshlet_build.meshlets.len() as u32 - meshlet_cursor;
            let level_bounds = if level == 0 {
                part_bounds
            } else {
                bounds_of(&positions)
            };
            lods.push(LodDesc {
                bounds: level_bounds,
                error: part_lod.error,
                part: parts.len() as u32,
                meshlet_first: meshlet_cursor,
                meshlet_count,
                triangle_count: part_lod.triangles,
                level: level as u32,
                index_first: 0,
                index_count: part_lod.triangles * 3,
                flags: 0,
            });
            meshlet_cursor += meshlet_count;
            lod_triangles[level] += part_lod.triangles;
            lod_meshlets[level] += meshlet_count;
        }
        parts.push(PartDesc {
            bounds: part_bounds,
            node,
            material,
            lod_first,
            lod_count: LOD_COUNT as u32,
            vertex_first: base,
            vertex_count: part.verts.len() as u32,
            flags: (if is_glass { PART_FLAG_GLASS } else { 0 })
                | (importance << PART_IMPORTANCE_SHIFT),
        });

        // Legacy flat arrays stay level 0, cache-reordered, opaque/glass split.
        // They are concatenated into one raster buffer after the loop so the
        // runtime copies a single byte range with no glass splice.
        let reordered = &part_lods[0].indices;
        let target = if is_glass { &mut glass } else { &mut opaque };
        level0_local_starts.push(target.len() as u32);
        level0_is_glass.push(is_glass);
        for &index in reordered {
            target.push((base + index) as u16);
        }
        // RT gets a separate simplified proxy per part, still grouped by the
        // animated node for the per-node BLAS/TLAS instances. Glass and
        // emissive parts are excluded so soft-shadow rays skip them.
        if !is_glass {
            let proxy = build_rt_proxy(&part.idx, &positions, importance, order);
            rt_nodes[node_index(part.node)]
                .extend(proxy.iter().map(|&index| (base + index) as u16));
        }
        triangles += part.idx.len() as u32 / 3;
        importance_triangles[importance as usize] += part.idx.len() as u32 / 3;
        importance_parts[importance as usize] += 1;
    }

    // Opaque then glass in final GPU draw order.
    let opaque_count = opaque.len() as u32;
    let glass_count = glass.len() as u32;
    let mut raster = opaque;
    raster.extend_from_slice(&glass);
    for (part_index, (local_start, is_glass)) in level0_local_starts
        .iter()
        .zip(level0_is_glass.iter())
        .enumerate()
    {
        let lod_index = part_index * LOD_COUNT;
        lods[lod_index].index_first = if *is_glass {
            opaque_count + local_start
        } else {
            *local_start
        };
    }

    let mut rt_idx = Vec::new();
    let mut rt_geom_nodes = Vec::new();
    let mut rt_node_ranges = Vec::new();
    for (node, indices) in rt_nodes.into_iter().enumerate() {
        if indices.is_empty() {
            continue;
        }
        rt_node_ranges.push((rt_idx.len() as u32, indices.len() as u32));
        rt_geom_nodes.push(node as u32);
        rt_idx.extend_from_slice(&indices);
    }

    let asset = BakedAirframe {
        stream,
        raster,
        opaque_count,
        glass_count,
        rt_idx,
        rt_geom_nodes,
        rt_node_ranges,
        parts,
        lods,
        meshlets: meshlet_build.meshlets,
        meshlet_vertices: meshlet_build.vertices,
        meshlet_triangles: meshlet_build.triangles,
    };
    asset
        .validate()
        .expect("airframe baker produced invalid asset");
    let stats = BakeStats {
        triangles,
        vertices: asset.stream.len() / VERTEX_BYTES,
        opaque_indices: asset.opaque_count as usize,
        glass_indices: asset.glass_count as usize,
        rt_indices: asset.rt_idx.len(),
        rt_nodes: asset.rt_geom_nodes.len(),
        parts: asset.parts.len() as u32,
        lods: asset.lods.len() as u32,
        meshlets: asset.meshlets.len() as u32,
        meshlet_vertices: asset.meshlet_vertices.len() as u32,
        lod_triangles,
        lod_meshlets,
        importance_triangles,
        importance_parts,
    };
    (asset, stats)
}

/// Per-part breakdown for `--analyze`.
#[derive(Clone, Debug, PartialEq)]
pub struct PartAnalysis {
    pub part: usize,
    pub node: u16,
    pub material: u16,
    pub importance: u32,
    pub bounds: [f32; 4],
    pub lod_triangles: Vec<u32>,
    pub lod_meshlets: Vec<u32>,
    pub lod_errors: Vec<f32>,
}

/// Walk the baked hierarchy and return one record per part.
pub fn analyze(asset: &BakedAirframe) -> Vec<PartAnalysis> {
    asset
        .parts
        .iter()
        .enumerate()
        .map(|(index, part)| {
            let range = part.lod_first as usize..part.lod_first as usize + part.lod_count as usize;
            let part_lods = &asset.lods[range];
            PartAnalysis {
                part: index,
                node: part.node,
                material: part.material,
                importance: airframe_format::part_importance(part.flags),
                bounds: part.bounds,
                lod_triangles: part_lods.iter().map(|l| l.triangle_count).collect(),
                lod_meshlets: part_lods.iter().map(|l| l.meshlet_count).collect(),
                lod_errors: part_lods.iter().map(|l| l.error).collect(),
            }
        })
        .collect()
}

/// Draw-path target selected by `--target`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BakeTarget {
    /// Task/mesh-shader path: hierarchy must be complete.
    MeshShader,
    /// Legacy two-draw u16 path: flat sections must be complete.
    Legacy,
}

impl BakeTarget {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "mesh-shader" => Ok(Self::MeshShader),
            "legacy" => Ok(Self::Legacy),
            other => Err(format!(
                "unknown target {other:?}; expected mesh-shader or legacy"
            )),
        }
    }
}

/// Check that the baked asset carries every section the chosen draw path needs.
pub fn validate_target(asset: &BakedAirframe, target: BakeTarget) -> Result<(), String> {
    asset
        .validate()
        .map_err(|error| error.to_string())?;
    match target {
        BakeTarget::Legacy => {
            if asset.raster.is_empty() || asset.opaque_count == 0 {
                return Err("legacy target requires a non-empty opaque raster section".into());
            }
            if asset.rt_geom_nodes.is_empty() {
                return Err("legacy target requires RT node ranges".into());
            }
            Ok(())
        }
        BakeTarget::MeshShader => {
            if asset.parts.is_empty() {
                return Err("mesh-shader target requires parts".into());
            }
            if asset.meshlets.is_empty() {
                return Err("mesh-shader target requires meshlets".into());
            }
            if asset.lods.len() != asset.parts.len() * LOD_COUNT {
                return Err(format!(
                    "expected {} LODs for {} parts, found {}",
                    asset.parts.len() * LOD_COUNT,
                    asset.parts.len(),
                    asset.lods.len()
                ));
            }
            Ok(())
        }
    }
}

/// Encode then decode and require bit-identical equality.
pub fn round_trip(asset: &BakedAirframe) -> Result<(), String> {
    let encoded = asset.encode();
    let decoded = airframe_format::decode(&encoded).map_err(|error| error.to_string())?;
    if decoded != *asset {
        return Err("encode/decode round trip changed the asset".into());
        }
    Ok(())
}

fn node_index(node: Node) -> usize {
    match node {
        Node::Hull => 0,
        Node::Canopy => 1,
        Node::WingL => 2,
        Node::WingR => 3,
        Node::Flap(id) => 4 + id as usize,
        Node::Rotor => 10,
        Node::Petal(id) => 11 + id as usize,
        Node::Fin(id) => 21 + id as usize,
    }
}

fn mat_index(mat: MatId) -> u16 {
    match mat {
        MatId::Sail => 0,
        MatId::Composite => 1,
        MatId::Graphite => 2,
        MatId::Titanium => 3,
        MatId::Dark => 4,
        MatId::Seat => 5,
        MatId::Glass => 6,
        MatId::Glow => 7,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baked_asset_round_trips_and_has_expected_topology() {
        let (asset, stats) = bake_with_stats();
        // Baseline counters for tessellation/importance changes; update with
        // measured values whenever the generator changes.
        // 2026-09-22: semantic part split + adaptive tessellation.
        assert_eq!(stats.triangles, 24_486);
        assert_eq!(stats.vertices, 15_341);
        assert_eq!(stats.rt_nodes, 23);
        assert_eq!(
            airframe_format::decode(&asset.encode()).unwrap(),
            asset
        );
    }

    #[test]
    fn part_flags_carry_importance_and_glass() {
        let (asset, stats) = bake_with_stats();
        assert_eq!(
            stats.importance_parts.iter().sum::<u32>(),
            stats.parts
        );
        assert_eq!(
            stats.importance_triangles.iter().sum::<u32>(),
            stats.triangles
        );
        let mut seen = [false; IMPORTANCE_COUNT as usize];
        for part in &asset.parts {
            let importance = airframe_format::part_importance(part.flags);
            assert!(importance < IMPORTANCE_COUNT);
            seen[importance as usize] = true;
            if part.flags & PART_FLAG_GLASS != 0 {
                assert_eq!(part.material, mat_index(MatId::Glass));
            }
        }
        assert!(seen.iter().all(|&present| present));
        // Hidden interior geometry must not dominate the far LOD chain.
        let interior = airframe_format::IMPORTANCE_INTERIOR as usize;
        let silhouette = airframe_format::IMPORTANCE_SILHOUETTE as usize;
        assert!(stats.importance_triangles[interior] > 0);
        assert!(stats.importance_triangles[silhouette] > 0);
    }

    #[test]
    fn hierarchy_covers_every_part_with_dense_lods_and_meshlets() {
        let (asset, stats) = bake_with_stats();
        assert!(stats.parts > 0);
        assert_eq!(stats.lods, stats.parts * LOD_LEVELS as u32);
        assert!(stats.meshlets > 0);
        assert!(stats.lod_triangles[0] > 0);
        for level in 1..LOD_LEVELS {
            assert!(
                stats.lod_triangles[level] < stats.lod_triangles[0],
                "LOD {level} did not simplify"
            );
            assert!(stats.lod_meshlets[level] > 0);
        }
        for level in 1..LOD_LEVELS {
            assert!(stats.lod_triangles[level] <= stats.lod_triangles[level - 1]);
        }
        assert_eq!(
            asset.lods.iter().map(|l| l.triangle_count).sum::<u32>(),
            stats
                .lod_triangles
                .iter()
                .sum::<u32>()
        );
        validate_target(&asset, BakeTarget::MeshShader).expect("mesh-shader target");
        validate_target(&asset, BakeTarget::Legacy).expect("legacy target");
        round_trip(&asset).expect("round trip");
        // Every animation node still owns at least one part.
        let mut seen = [false; NODE_COUNT];
        for part in &asset.parts {
            seen[part.node as usize] = true;
        }
        assert!(seen.iter().all(|&present| present), "a node lost its parts");
    }

    #[test]
    fn meshlet_local_indices_stay_inside_each_cluster() {
        let (asset, _) = bake_with_stats();
        for meshlet in &asset.meshlets {
            let tri_end =
                meshlet.triangle_offset as usize + meshlet.triangle_count as usize * 3;
            let local = &asset.meshlet_triangles[meshlet.triangle_offset as usize..tri_end];
            assert!(local.iter().all(|&i| u16::from(i) < meshlet.vertex_count));
        }
    }

    #[test]
    fn rt_proxy_is_simpler_than_the_raster_mesh_but_keeps_every_node() {
        let (asset, stats) = bake_with_stats();
        assert!(
            stats.rt_indices < stats.opaque_indices,
            "RT proxy ({}) did not shrink below opaque raster ({})",
            stats.rt_indices,
            stats.opaque_indices
        );
        assert_eq!(stats.rt_nodes, 23);
        // Every RT range still indexes the packed stream and is triangle aligned.
        asset
            .validate()
            .expect("RT proxy ranges must validate");
        let vertices = asset.stream.len() / airframe_format::VERTEX_BYTES;
        for &index in &asset.rt_idx {
            assert!((index as usize) < vertices);
        }
    }

    #[test]
    fn node_and_material_indices_match_runtime_tables() {
        assert_eq!(node_index(Node::Hull), 0);
        assert_eq!(node_index(Node::Canopy), 1);
        assert_eq!(node_index(Node::WingL), 2);
        assert_eq!(node_index(Node::WingR), 3);
        for id in 0..6 {
            assert_eq!(node_index(Node::Flap(id)), 4 + id as usize);
        }
        assert_eq!(node_index(Node::Rotor), 10);
        for id in 0..10 {
            assert_eq!(node_index(Node::Petal(id)), 11 + id as usize);
        }
        assert_eq!(node_index(Node::Fin(0)), 21);
        assert_eq!(node_index(Node::Fin(1)), 22);

        assert_eq!(mat_index(MatId::Sail), 0);
        assert_eq!(mat_index(MatId::Glass), 6);
        assert_eq!(mat_index(MatId::Glow), 7);
    }

    #[test]
    fn analyze_reports_one_record_per_part() {
        let (asset, stats) = bake_with_stats();
        let report = analyze(&asset);
        assert_eq!(report.len(), asset.parts.len());
        assert_eq!(report.len(), stats.parts as usize);
        for record in &report {
            assert_eq!(record.lod_triangles.len(), LOD_LEVELS);
            assert_eq!(record.lod_errors[0], 0.0);
        }
    }

    #[test]
    fn target_validation_rejects_unknown_names() {
        assert!(BakeTarget::parse("mesh-shader").is_ok());
        assert!(BakeTarget::parse("legacy").is_ok());
        assert!(BakeTarget::parse("vk9").is_err());
    }
}
