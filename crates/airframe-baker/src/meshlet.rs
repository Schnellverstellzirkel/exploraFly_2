//! Offline meshlet partitioning: split one LOD index buffer into GPU
//! clusters of at most 64 vertices and 126 triangles, each with bounds and
//! a normal cone for task-shader frustum and backface-cone rejection.

use airframe_format::{MeshletDesc, MESHLET_MAX_TRIANGLES, MESHLET_MAX_VERTICES};
use meshopt_rs::cluster::{build_meshlets, build_meshlets_bound, compute_meshlet_bounds, Meshlet};

use crate::lod::Pos;

/// Accumulated meshlet payload for the whole airframe.
#[derive(Default)]
pub(crate) struct MeshletBuild {
    pub meshlets: Vec<MeshletDesc>,
    pub vertices: Vec<u32>,
    pub triangles: Vec<u8>,
}

impl MeshletBuild {
    /// Partition one LOD. `indices` are global vertex indices into the packed
    /// stream; `positions` is indexed the same way for bounds and cones.
    pub fn push_lod(
        &mut self,
        indices: &[u32],
        positions: &[[f32; 3]],
        part: u16,
        level: u16,
    ) {
        if indices.is_empty() {
            return;
        }
        let vertex_count = positions.len();
        let bound = build_meshlets_bound(indices.len(), MESHLET_MAX_VERTICES, MESHLET_MAX_TRIANGLES);
        let mut meshlets = vec![Meshlet::default(); bound];
        let count = build_meshlets(
            &mut meshlets,
            indices,
            vertex_count,
            MESHLET_MAX_VERTICES,
            MESHLET_MAX_TRIANGLES,
        );
        let position_views: Vec<Pos> = positions.iter().map(|p| Pos(*p)).collect();
        for meshlet in meshlets[..count].iter() {
            let bounds = compute_meshlet_bounds(meshlet, &position_views);
            let vertex_offset = self.vertices.len() as u32;
            let triangle_offset = self.triangles.len() as u32;
            for i in 0..meshlet.vertex_count as usize {
                self.vertices.push(meshlet.vertices[i] as u32);
            }
            for triangle in meshlet.indices[..meshlet.triangle_count as usize].iter() {
                self.triangles.extend_from_slice(triangle);
            }
            self.meshlets.push(MeshletDesc {
                bounds: [bounds.center[0], bounds.center[1], bounds.center[2], bounds.radius],
                cone_axis: bounds.cone_axis,
                cone_cutoff: bounds.cone_cutoff,
                vertex_offset,
                triangle_offset,
                vertex_count: meshlet.vertex_count as u16,
                triangle_count: meshlet.triangle_count as u16,
                part,
                level,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use airframe_format::BakedAirframe;

    #[test]
    fn partitions_a_strip_into_valid_meshlets() {
        let positions: Vec<[f32; 3]> = (0..16u32)
            .map(|i| [i as f32, (i % 4) as f32, 0.0])
            .collect();
        let mut indices = Vec::new();
        for row in 0..3u32 {
            for col in 0..3u32 {
                let a = row * 4 + col;
                let b = a + 1;
                let c = a + 4;
                let d = c + 1;
                indices.extend_from_slice(&[a, b, d, a, d, c]);
            }
        }
        let mut build = MeshletBuild::default();
        build.push_lod(&indices, &positions, 0, 0);
        assert!(!build.meshlets.is_empty());
        let total_triangles: u32 = build
            .meshlets
            .iter()
            .map(|m| m.triangle_count as u32)
            .sum();
        assert_eq!(total_triangles, 18);
        for meshlet in &build.meshlets {
            assert!(meshlet.vertex_count as usize <= MESHLET_MAX_VERTICES);
            assert!(meshlet.triangle_count as usize <= MESHLET_MAX_TRIANGLES);
            let tri_end = meshlet.triangle_offset as usize
                + meshlet.triangle_count as usize * 3;
            let local = &build.triangles[meshlet.triangle_offset as usize..tri_end];
            assert!(local.iter().all(|&i| u16::from(i) < meshlet.vertex_count));
            let vert_end = meshlet.vertex_offset as usize + meshlet.vertex_count as usize;
            assert!(build.vertices[meshlet.vertex_offset as usize..vert_end]
                .iter()
                .all(|&v| (v as usize) < positions.len()));
        }
        // Hierarchy must satisfy the format validator when wired into an asset.
        let _ = BakedAirframe {
            stream: vec![0; airframe_format::VERTEX_BYTES * positions.len()],
            raster: Vec::new(),
            opaque_count: 0,
            glass_count: 0,
            rt_idx: Vec::new(),
            rt_geom_nodes: Vec::new(),
            rt_node_ranges: Vec::new(),
            parts: Vec::new(),
            lods: Vec::new(),
            meshlets: Vec::new(),
            meshlet_vertices: Vec::new(),
            meshlet_triangles: Vec::new(),
        };
    }
}
