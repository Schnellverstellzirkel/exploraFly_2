//! Build-time airframe baker.
//!
//! This crate owns the immutable conversion from procedural CAD parts to the
//! runtime's packed 28-byte vertex/index representation. The engine links the
//! resulting asset, not the procedural generator, so geometry construction,
//! normal accumulation, cache reordering, and RT range creation leave the
//! launch path.

use airframe::{build_airframe, f32_to_f16, oct_encode, MatId, Node};
use airframe_format::{BakedAirframe, NODE_COUNT, VERTEX_BYTES};
use glam::Vec3;

/// Summary emitted by build tooling and used by bake tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BakeStats {
    pub triangles: u32,
    pub vertices: usize,
    pub opaque_indices: usize,
    pub glass_indices: usize,
    pub rt_indices: usize,
    pub rt_nodes: usize,
}

/// Bake the current procedural airframe into the versioned runtime asset.
pub fn bake() -> BakedAirframe {
    bake_with_stats().0
}

/// Bake and return immutable geometry plus counts for build diagnostics.
pub fn bake_with_stats() -> (BakedAirframe, BakeStats) {
    let raw = build_airframe();
    let vertex_capacity: usize = raw.iter().map(|part| part.verts.len()).sum();
    let index_capacity: usize = raw.iter().map(|part| part.idx.len()).sum();
    let mut stream = Vec::with_capacity(vertex_capacity * VERTEX_BYTES);
    let mut opaque = Vec::with_capacity(index_capacity);
    let mut glass = Vec::new();
    let mut rt_nodes: Vec<Vec<u16>> = vec![Vec::new(); NODE_COUNT];
    let mut triangles = 0u32;

    for part in &raw {
        let base = (stream.len() / VERTEX_BYTES) as u32;
        assert!(
            base + part.verts.len() as u32 <= 65536,
            "merged verts exceed u16"
        );
        let mut normals = vec![Vec3::ZERO; part.verts.len()];
        for tri in part.idx.chunks_exact(3) {
            let a = Vec3::from_array(part.verts[tri[0] as usize].pos);
            let b = Vec3::from_array(part.verts[tri[1] as usize].pos);
            let c = Vec3::from_array(part.verts[tri[2] as usize].pos);
            let normal = (b - a).cross(c - a);
            normals[tri[0] as usize] += normal;
            normals[tri[1] as usize] += normal;
            normals[tri[2] as usize] += normal;
        }
        let reordered = airframe::forsyth::reorder(&part.idx);
        let node = node_index(part.node) as u16;
        let material = mat_index(part.mat);
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
        let target = if part.mat == MatId::Glass {
            &mut glass
        } else {
            &mut opaque
        };
        for &index in &reordered {
            target.push((base + index) as u16);
        }
        if part.mat != MatId::Glass {
            rt_nodes[node_index(part.node)]
                .extend(reordered.iter().map(|&index| (base + index) as u16));
        }
        triangles += part.idx.len() as u32 / 3;
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
        opaque,
        glass,
        rt_idx,
        rt_geom_nodes,
        rt_node_ranges,
    };
    asset
        .validate()
        .expect("airframe baker produced invalid asset");
    let stats = BakeStats {
        triangles,
        vertices: asset.stream.len() / VERTEX_BYTES,
        opaque_indices: asset.opaque.len(),
        glass_indices: asset.glass.len(),
        rt_indices: asset.rt_idx.len(),
        rt_nodes: asset.rt_geom_nodes.len(),
    };
    (asset, stats)
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
        assert_eq!(stats.triangles, 46_752);
        assert_eq!(stats.vertices, 28_162);
        assert_eq!(stats.rt_nodes, 23);
        assert_eq!(airframe_format::decode(&asset.encode()).unwrap(), asset);
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
}
