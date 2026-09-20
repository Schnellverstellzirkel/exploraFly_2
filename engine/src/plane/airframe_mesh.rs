//! CPU-side airframe merge: one 28-byte vertex stream, opaque/glass index
//! ranges, and contiguous per-node index ranges for the ray-traced shadow
//! casters. Pure geometry packing; no GPU handles here.

use airframe::{build_airframe, MatId};
use airframe::{f32_to_f16, oct_encode};
use crate::anim::{mat_index, node_index};
use crate::ubo::NODE_COUNT;
use super::VERTEX_BYTES;
use glam::Vec3;

/// Merged airframe geometry ready for device upload.
pub(super) struct AirframeMesh {
    pub(super) stream: Vec<u8>,
    pub(super) opaque: Vec<u16>,
    pub(super) glass: Vec<u16>,
    pub(super) rt_idx: Vec<u16>,
    pub(super) rt_geom_nodes: Vec<u32>,
    pub(super) rt_node_ranges: Vec<(u32, u32)>,
}

pub(super) fn airframe_mesh() -> AirframeMesh {
    let _raw = build_airframe();
    let raw = build_airframe();
    // One 28 byte stream: pos12 + oct4 + uvHalf4 + flex4 + ids4.
    let mut stream: Vec<u8> = Vec::new();
    let mut opaque: Vec<u16> = Vec::new();
    let mut glass: Vec<u16> = Vec::new();
    // Ray-traced shadow casters: raw absolute indices grouped per node so
    // each BLAS gets a contiguous index range. Glass panels never cast a
    // hard sun shadow (their shade pass ignores direct light).
    let mut rt_nodes: Vec<Vec<u16>> = vec![Vec::new(); NODE_COUNT];
    let mut tri_total = 0u32;
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
            let n = (b - a).cross(c - a);
            normals[tri[0] as usize] += n;
            normals[tri[1] as usize] += n;
            normals[tri[2] as usize] += n;
        }
        let reordered = airframe::forsyth::reorder(&part.idx);
        let node = node_index(part.node) as u16;
        let mat = mat_index(part.mat);
        for (v, n) in part.verts.iter().zip(normals.iter()) {
            let n = n.normalize_or_zero();
            let oct = oct_encode(n);
            stream.extend_from_slice(&v.pos[0].to_le_bytes());
            stream.extend_from_slice(&v.pos[1].to_le_bytes());
            stream.extend_from_slice(&v.pos[2].to_le_bytes());
            stream.extend_from_slice(&oct[0].to_le_bytes());
            stream.extend_from_slice(&oct[1].to_le_bytes());
            stream.extend_from_slice(&f32_to_f16(v.uv[0]).to_le_bytes());
            stream.extend_from_slice(&f32_to_f16(v.uv[1]).to_le_bytes());
            stream.extend_from_slice(&v.flex.to_le_bytes());
            stream.extend_from_slice(&node.to_le_bytes());
            stream.extend_from_slice(&mat.to_le_bytes());
        }
        let target = if part.mat == MatId::Glass {
            &mut glass
        } else {
            &mut opaque
        };
        for i in &reordered {
            target.push((base + i) as u16);
        }
        if part.mat != MatId::Glass {
            rt_nodes[node_index(part.node)].extend((&reordered).iter().map(|i| (base + i) as u16));
        }
        tri_total += part.idx.len() as u32 / 3;
    }
    // Contiguous per-node ranges; nodes without casters are dropped.
    let mut rt_idx: Vec<u16> = Vec::new();
    let mut rt_node_ranges: Vec<(u32, u32)> = Vec::new();
    let mut rt_geom_nodes: Vec<u32> = Vec::new();
    for (n, idx) in rt_nodes.iter().enumerate() {
        if idx.is_empty() {
            continue;
        }
        rt_node_ranges.push((rt_idx.len() as u32, idx.len() as u32));
        rt_geom_nodes.push(n as u32);
        rt_idx.extend_from_slice(idx);
    }
    for part in &raw {
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for v in &part.verts {
            for k in 0..3 {
                min[k] = min[k].min(v.pos[k]);
                max[k] = max[k].max(v.pos[k]);
            }
        }
        println!(
            "part {:?}/{:?} v{} t{} bbox [{:.2},{:.2},{:.2}]-[{:.2},{:.2},{:.2}]",
            part.node,
            part.mat,
            part.verts.len(),
            part.idx.len() / 3,
            min[0],
            min[1],
            min[2],
            max[0],
            max[1],
            max[2],
        );
    }
    println!(
        "airframe: {} tris merged, {:.1} KiB verts, {:.1} KiB indices, 2 draws",
        tri_total,
        stream.len() as f32 / 1024.0,
        (opaque.len() + glass.len()) as f32 * 2.0 / 1024.0,
    );
    AirframeMesh {
        stream,
        opaque,
        glass,
        rt_idx,
        rt_geom_nodes,
        rt_node_ranges,
    }
}
