//! Versioned packed airframe asset format shared by the build-time baker and
//! the runtime Vulkan loader. The format is deliberately little-endian and
//! self-describing so generated geometry can be validated before GPU upload.
//!
//! Version 2 keeps the version-1 vertex stream and flat index sections for the
//! legacy two-draw path and the RT builder, and appends a part → LOD → meshlet
//! hierarchy for the mesh-shader path: per-part bounds, per-LOD geometric
//! screen-space error, per-meshlet bounds/normal cone/local index ranges.

/// Eight-byte file signature.
pub const MAGIC: [u8; 8] = *b"EXAFRM01";
/// Packed asset format version.
pub const VERSION: u32 = 2;
/// Runtime vertex stride: position, oct normal, half UV, flex, node/material IDs.
pub const VERTEX_BYTES: usize = 28;
/// Number of animation-node slots in the shared UBO.
pub const NODE_COUNT: usize = 23;
/// Size of one packed `PartDesc`.
pub const PART_BYTES: usize = 32;
/// Size of one packed `LodDesc`.
pub const LOD_BYTES: usize = 48;
/// Size of one packed `MeshletDesc` (matches the task-shader std430 stride).
pub const MESHLET_BYTES: usize = 48;
/// Upper bound on vertices a single meshlet may reference.
pub const MESHLET_MAX_VERTICES: usize = 64;
/// Upper bound on triangles a single meshlet may contain.
pub const MESHLET_MAX_TRIANGLES: usize = 126;
const HEADER_BYTES: usize = 68;

/// Fully packed immutable airframe data.
#[derive(Clone, Debug, PartialEq)]
pub struct BakedAirframe {
    pub stream: Vec<u8>,
    pub opaque: Vec<u16>,
    pub glass: Vec<u16>,
    pub rt_idx: Vec<u16>,
    pub rt_geom_nodes: Vec<u32>,
    pub rt_node_ranges: Vec<(u32, u32)>,
    pub parts: Vec<PartDesc>,
    pub lods: Vec<LodDesc>,
    pub meshlets: Vec<MeshletDesc>,
    /// Global vertex indices, meshlet-local runs referenced by `MeshletDesc`.
    pub meshlet_vertices: Vec<u32>,
    /// Packed local triangle indices (3 bytes per triangle, values < vertex_count).
    pub meshlet_triangles: Vec<u8>,
}

/// One aircraft component: a node/material pair with its LOD chain.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct PartDesc {
    /// Bounding sphere in part-local object space: center xyz, radius w.
    pub bounds: [f32; 4],
    /// Animation node slot (0..NODE_COUNT).
    pub node: u16,
    /// Material index matching the per-vertex material field.
    pub material: u16,
    /// First index into `BakedAirframe::lods`.
    pub lod_first: u32,
    /// Number of contiguous LOD records for this part.
    pub lod_count: u32,
    /// Bit 0: glass part (alpha pass, excluded from RT).
    pub flags: u32,
}

/// Bit 0 of `PartDesc::flags`: transparent canopy glass.
pub const PART_FLAG_GLASS: u32 = 1;

/// One simplified level of detail for a part.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct LodDesc {
    /// LOD bounding sphere, part-local: center xyz, radius w.
    pub bounds: [f32; 4],
    /// Object-space geometric error upper bound in meters. Level 0 is 0.
    pub error: f32,
    /// Owning part index.
    pub part: u32,
    /// First meshlet of this LOD.
    pub meshlet_first: u32,
    /// Meshlet count of this LOD.
    pub meshlet_count: u32,
    /// Triangle count of this LOD.
    pub triangle_count: u32,
    /// 0 is full resolution; higher is coarser.
    pub level: u32,
    /// Flat index count (3 × triangles) matching the legacy arrays for level 0.
    pub index_count: u32,
    /// Reserved for future payload flags (compression, etc).
    pub flags: u32,
}

/// One GPU cluster: local vertex/index ranges plus culling bounds.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct MeshletDesc {
    /// Meshlet bounding sphere, part-local: center xyz, radius w.
    pub bounds: [f32; 4],
    /// Backface normal-cone axis in part-local space.
    pub cone_axis: [f32; 3],
    /// Cosine of the cone half-angle; reject when axis·view < cutoff.
    pub cone_cutoff: f32,
    /// First entry in `meshlet_vertices`.
    pub vertex_offset: u32,
    /// Byte offset into `meshlet_triangles`.
    pub triangle_offset: u32,
    /// Vertex count (1..=MESHLET_MAX_VERTICES).
    pub vertex_count: u16,
    /// Triangle count (0..=MESHLET_MAX_TRIANGLES).
    pub triangle_count: u16,
    /// Owning part index.
    pub part: u16,
    /// LOD level of the parent `LodDesc`.
    pub level: u16,
}

/// Errors raised while loading a generated asset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeError {
    Truncated { needed: usize, available: usize },
    Invalid(&'static str),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated { needed, available } => {
                write!(
                    f,
                    "airframe asset truncated: need {needed} bytes, have {available}"
                )
            }
            Self::Invalid(reason) => write!(f, "invalid airframe asset: {reason}"),
        }
    }
}

impl std::error::Error for DecodeError {}

impl BakedAirframe {
    /// Validate all bounds that later Vulkan upload, mesh culling, and RT build
    /// code relies on.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.stream.len() % VERTEX_BYTES != 0 {
            return Err("vertex stream is not stride aligned");
        }
        let vertices = self.stream.len() / VERTEX_BYTES;
        if vertices > u16::MAX as usize + 1 {
            return Err("vertex stream exceeds UINT16 index space");
        }
        let valid_index = |&index: &u16| (index as usize) < vertices;
        if !self.opaque.iter().all(valid_index)
            || !self.glass.iter().all(valid_index)
            || !self.rt_idx.iter().all(valid_index)
        {
            return Err("index references a vertex outside the packed stream");
        }
        if self.rt_geom_nodes.len() != self.rt_node_ranges.len() {
            return Err("RT node and range counts differ");
        }
        if self
            .rt_geom_nodes
            .iter()
            .any(|&node| node as usize >= NODE_COUNT)
        {
            return Err("RT node is outside the animation-node table");
        }
        for &(offset, count) in &self.rt_node_ranges {
            let end = offset.checked_add(count).ok_or("RT node range overflows")?;
            if end as usize > self.rt_idx.len() {
                return Err("RT node range exceeds RT index data");
            }
            if count % 3 != 0 {
                return Err("RT node range is not triangle aligned");
            }
        }
        self.validate_hierarchy(vertices)?;
        Ok(())
    }

    fn validate_hierarchy(&self, vertices: usize) -> Result<(), &'static str> {
        if self.meshlet_triangles.len() % 3 != 0 {
            return Err("meshlet triangle stream is not triangle aligned");
        }
        for (index, part) in self.parts.iter().enumerate() {
            if part.node as usize >= NODE_COUNT {
                return Err("part node is outside the animation-node table");
            }
            let end = part
                .lod_first
                .checked_add(part.lod_count)
                .ok_or("part LOD range overflows")?;
            if end as usize > self.lods.len() {
                return Err("part LOD range exceeds LOD table");
            }
            if part.lod_count == 0 {
                return Err("part has no LOD levels");
            }
            for (level, lod) in self.lods[part.lod_first as usize..end as usize]
                .iter()
                .enumerate()
            {
                if lod.part as usize != index {
                    return Err("LOD back-reference does not match its part");
                }
                if lod.level as usize != level {
                    return Err("LOD levels are not dense from 0");
                }
                let mesh_end = lod
                    .meshlet_first
                    .checked_add(lod.meshlet_count)
                    .ok_or("LOD meshlet range overflows")?;
                if mesh_end as usize > self.meshlets.len() {
                    return Err("LOD meshlet range exceeds meshlet table");
                }
                if lod.triangle_count as usize * 3 != lod.index_count as usize {
                    return Err("LOD triangle count does not match index count");
                }
            }
        }
        for meshlet in &self.meshlets {
            if meshlet.part as usize >= self.parts.len() {
                return Err("meshlet part is outside the part table");
            }
            if meshlet.vertex_count == 0
                || meshlet.vertex_count as usize > MESHLET_MAX_VERTICES
            {
                return Err("meshlet vertex count is out of range");
            }
            if meshlet.triangle_count as usize > MESHLET_MAX_TRIANGLES {
                return Err("meshlet triangle count is out of range");
            }
            if !meshlet.cone_cutoff.is_finite()
                || !meshlet.cone_axis.iter().all(|v| v.is_finite())
                || !meshlet.bounds.iter().all(|v| v.is_finite())
                || meshlet.bounds[3] < 0.0
            {
                return Err("meshlet bounds or cone are not finite");
            }
            let vertex_end = meshlet
                .vertex_offset
                .checked_add(meshlet.vertex_count as u32)
                .ok_or("meshlet vertex range overflows")?;
            if vertex_end as usize > self.meshlet_vertices.len() {
                return Err("meshlet vertex range exceeds meshlet vertex stream");
            }
            if self.meshlet_vertices[meshlet.vertex_offset as usize..vertex_end as usize]
                .iter()
                .any(|&v| v as usize >= vertices)
            {
                return Err("meshlet vertex references a vertex outside the packed stream");
            }
            let tri_bytes = (meshlet.triangle_count as u32)
                .checked_mul(3)
                .ok_or("meshlet triangle size overflows")?;
            let tri_end = meshlet
                .triangle_offset
                .checked_add(tri_bytes)
                .ok_or("meshlet triangle range overflows")?;
            if tri_end as usize > self.meshlet_triangles.len() {
                return Err("meshlet triangle range exceeds triangle stream");
            }
            let local = &self.meshlet_triangles
                [meshlet.triangle_offset as usize..tri_end as usize];
            if local.iter().any(|&i| i as u16 >= meshlet.vertex_count) {
                return Err("meshlet local index is outside its vertex range");
            }
        }
        if let Some(first) = self.lods.first() {
            if first.level != 0 {
                return Err("first LOD is not full resolution");
            }
        }
        Ok(())
    }

    /// Serialize the validated asset into the stable little-endian format.
    pub fn encode(&self) -> Vec<u8> {
        self.validate().expect("invalid baked airframe");
        let index_bytes = self
            .opaque
            .len()
            .checked_add(self.glass.len())
            .and_then(|n| n.checked_add(self.rt_idx.len()))
            .and_then(|n| n.checked_mul(2))
            .expect("airframe index size overflow");
        let node_bytes = self
            .rt_geom_nodes
            .len()
            .checked_mul(4)
            .expect("airframe node size overflow");
        let range_bytes = self
            .rt_node_ranges
            .len()
            .checked_mul(8)
            .expect("airframe range size overflow");
        let part_bytes = self
            .parts
            .len()
            .checked_mul(PART_BYTES)
            .expect("airframe part size overflow");
        let lod_bytes = self
            .lods
            .len()
            .checked_mul(LOD_BYTES)
            .expect("airframe LOD size overflow");
        let meshlet_bytes = self
            .meshlets
            .len()
            .checked_mul(MESHLET_BYTES)
            .expect("airframe meshlet size overflow");
        let meshlet_vertex_bytes = self
            .meshlet_vertices
            .len()
            .checked_mul(4)
            .expect("airframe meshlet vertex size overflow");
        let capacity = HEADER_BYTES
            .checked_add(self.stream.len())
            .and_then(|n| n.checked_add(index_bytes))
            .and_then(|n| n.checked_add(node_bytes))
            .and_then(|n| n.checked_add(range_bytes))
            .and_then(|n| n.checked_add(part_bytes))
            .and_then(|n| n.checked_add(lod_bytes))
            .and_then(|n| n.checked_add(meshlet_bytes))
            .and_then(|n| n.checked_add(meshlet_vertex_bytes))
            .and_then(|n| n.checked_add(self.meshlet_triangles.len()))
            .expect("airframe asset size overflow");
        let mut out = Vec::with_capacity(capacity);
        out.extend_from_slice(&MAGIC);
        push_u32(&mut out, VERSION);
        push_u32(&mut out, HEADER_BYTES as u32);
        push_u32(&mut out, VERTEX_BYTES as u32);
        push_u32(&mut out, self.stream.len() as u32);
        push_u32(&mut out, self.opaque.len() as u32);
        push_u32(&mut out, self.glass.len() as u32);
        push_u32(&mut out, self.rt_idx.len() as u32);
        push_u32(&mut out, self.rt_geom_nodes.len() as u32);
        push_u32(&mut out, self.rt_node_ranges.len() as u32);
        push_u32(&mut out, self.parts.len() as u32);
        push_u32(&mut out, self.lods.len() as u32);
        push_u32(&mut out, self.meshlets.len() as u32);
        push_u32(&mut out, self.meshlet_vertices.len() as u32);
        push_u32(&mut out, self.meshlet_triangles.len() as u32);
        push_u32(&mut out, 0);
        out.extend_from_slice(&self.stream);
        for &index in &self.opaque {
            out.extend_from_slice(&index.to_le_bytes());
        }
        for &index in &self.glass {
            out.extend_from_slice(&index.to_le_bytes());
        }
        for &index in &self.rt_idx {
            out.extend_from_slice(&index.to_le_bytes());
        }
        for &node in &self.rt_geom_nodes {
            out.extend_from_slice(&node.to_le_bytes());
        }
        for &(offset, count) in &self.rt_node_ranges {
            out.extend_from_slice(&offset.to_le_bytes());
            out.extend_from_slice(&count.to_le_bytes());
        }
        for part in &self.parts {
            push_f32x4(&mut out, part.bounds);
            out.extend_from_slice(&part.node.to_le_bytes());
            out.extend_from_slice(&part.material.to_le_bytes());
            push_u32(&mut out, part.lod_first);
            push_u32(&mut out, part.lod_count);
            push_u32(&mut out, part.flags);
        }
        for lod in &self.lods {
            push_f32x4(&mut out, lod.bounds);
            push_f32(&mut out, lod.error);
            push_u32(&mut out, lod.part);
            push_u32(&mut out, lod.meshlet_first);
            push_u32(&mut out, lod.meshlet_count);
            push_u32(&mut out, lod.triangle_count);
            push_u32(&mut out, lod.level);
            push_u32(&mut out, lod.index_count);
            push_u32(&mut out, lod.flags);
        }
        for meshlet in &self.meshlets {
            push_f32x4(&mut out, meshlet.bounds);
            for axis in meshlet.cone_axis {
                push_f32(&mut out, axis);
            }
            push_f32(&mut out, meshlet.cone_cutoff);
            push_u32(&mut out, meshlet.vertex_offset);
            push_u32(&mut out, meshlet.triangle_offset);
            out.extend_from_slice(&meshlet.vertex_count.to_le_bytes());
            out.extend_from_slice(&meshlet.triangle_count.to_le_bytes());
            out.extend_from_slice(&meshlet.part.to_le_bytes());
            out.extend_from_slice(&meshlet.level.to_le_bytes());
        }
        for &index in &self.meshlet_vertices {
            out.extend_from_slice(&index.to_le_bytes());
        }
        out.extend_from_slice(&self.meshlet_triangles);
        debug_assert_eq!(out.len(), capacity);
        out
    }
}

/// Decode and validate a generated asset before handing it to the renderer.
pub fn decode(bytes: &[u8]) -> Result<BakedAirframe, DecodeError> {
    if bytes.len() < HEADER_BYTES {
        return Err(DecodeError::Truncated {
            needed: HEADER_BYTES,
            available: bytes.len(),
        });
    }
    if bytes[..MAGIC.len()] != MAGIC {
        return Err(DecodeError::Invalid("bad magic"));
    }
    let version = read_u32(bytes, 8)?;
    if version != VERSION {
        return Err(DecodeError::Invalid("unsupported version"));
    }
    if read_u32(bytes, 12)? as usize != HEADER_BYTES {
        return Err(DecodeError::Invalid("unexpected header size"));
    }
    if read_u32(bytes, 16)? as usize != VERTEX_BYTES {
        return Err(DecodeError::Invalid("unexpected vertex stride"));
    }
    let stream_bytes = read_u32(bytes, 20)? as usize;
    let opaque_count = read_u32(bytes, 24)? as usize;
    let glass_count = read_u32(bytes, 28)? as usize;
    let rt_index_count = read_u32(bytes, 32)? as usize;
    let node_count = read_u32(bytes, 36)? as usize;
    let range_count = read_u32(bytes, 40)? as usize;
    let part_count = read_u32(bytes, 44)? as usize;
    let lod_count = read_u32(bytes, 48)? as usize;
    let meshlet_count = read_u32(bytes, 52)? as usize;
    let meshlet_vertex_count = read_u32(bytes, 56)? as usize;
    let meshlet_triangle_bytes = read_u32(bytes, 60)? as usize;
    let mut cursor = HEADER_BYTES;
    let stream = take(bytes, &mut cursor, stream_bytes)?.to_vec();
    let opaque = read_u16s(take(bytes, &mut cursor, bytes_for(opaque_count, 2)?)?);
    let glass = read_u16s(take(bytes, &mut cursor, bytes_for(glass_count, 2)?)?);
    let rt_idx = read_u16s(take(bytes, &mut cursor, bytes_for(rt_index_count, 2)?)?);
    let rt_geom_nodes = read_u32s(take(bytes, &mut cursor, bytes_for(node_count, 4)?)?);
    let range_bytes = bytes_for(range_count, 8)?;
    let range_data = take(bytes, &mut cursor, range_bytes)?;
    let rt_node_ranges = range_data
        .chunks_exact(8)
        .map(|chunk| {
            (
                u32::from_le_bytes(chunk[0..4].try_into().unwrap()),
                u32::from_le_bytes(chunk[4..8].try_into().unwrap()),
            )
        })
        .collect();
    let parts = read_parts(take(bytes, &mut cursor, bytes_for(part_count, PART_BYTES)?)?)?;
    let lods = read_lods(take(bytes, &mut cursor, bytes_for(lod_count, LOD_BYTES)?)?)?;
    let meshlets =
        read_meshlets(take(bytes, &mut cursor, bytes_for(meshlet_count, MESHLET_BYTES)?)?)?;
    let meshlet_vertices = read_u32s(take(
        bytes,
        &mut cursor,
        bytes_for(meshlet_vertex_count, 4)?,
    )?);
    let meshlet_triangles =
        take(bytes, &mut cursor, meshlet_triangle_bytes)?.to_vec();
    if cursor != bytes.len() {
        return Err(DecodeError::Invalid("trailing bytes"));
    }
    let asset = BakedAirframe {
        stream,
        opaque,
        glass,
        rt_idx,
        rt_geom_nodes,
        rt_node_ranges,
        parts,
        lods,
        meshlets,
        meshlet_vertices,
        meshlet_triangles,
    };
    asset.validate().map_err(DecodeError::Invalid)?;
    Ok(asset)
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_f32(out: &mut Vec<u8>, value: f32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_f32x4(out: &mut Vec<u8>, values: [f32; 4]) {
    for value in values {
        push_f32(out, value);
    }
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, DecodeError> {
    let end = offset
        .checked_add(4)
        .ok_or(DecodeError::Invalid("header offset overflow"))?;
    if end > bytes.len() {
        return Err(DecodeError::Truncated {
            needed: end,
            available: bytes.len(),
        });
    }
    Ok(u32::from_le_bytes(bytes[offset..end].try_into().unwrap()))
}

fn read_f32(bytes: &[u8], offset: usize) -> Result<f32, DecodeError> {
    Ok(f32::from_bits(read_u32(bytes, offset)?))
}

fn bytes_for(count: usize, width: usize) -> Result<usize, DecodeError> {
    count
        .checked_mul(width)
        .ok_or(DecodeError::Invalid("section size overflow"))
}

fn take<'a>(bytes: &'a [u8], cursor: &mut usize, len: usize) -> Result<&'a [u8], DecodeError> {
    let end = cursor
        .checked_add(len)
        .ok_or(DecodeError::Invalid("section offset overflow"))?;
    if end > bytes.len() {
        return Err(DecodeError::Truncated {
            needed: end,
            available: bytes.len(),
        });
    }
    let section = &bytes[*cursor..end];
    *cursor = end;
    Ok(section)
}

fn read_u16s(bytes: &[u8]) -> Vec<u16> {
    bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes(chunk.try_into().unwrap()))
        .collect()
}

fn read_u32s(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes(chunk.try_into().unwrap()))
        .collect()
}

fn read_f32x4(bytes: &[u8], offset: usize) -> Result<[f32; 4], DecodeError> {
    Ok([
        read_f32(bytes, offset)?,
        read_f32(bytes, offset + 4)?,
        read_f32(bytes, offset + 8)?,
        read_f32(bytes, offset + 12)?,
    ])
}

fn read_parts(bytes: &[u8]) -> Result<Vec<PartDesc>, DecodeError> {
    bytes
        .chunks_exact(PART_BYTES)
        .map(|chunk| {
            Ok(PartDesc {
                bounds: read_f32x4(chunk, 0)?,
                node: u16::from_le_bytes(chunk[16..18].try_into().unwrap()),
                material: u16::from_le_bytes(chunk[18..20].try_into().unwrap()),
                lod_first: u32::from_le_bytes(chunk[20..24].try_into().unwrap()),
                lod_count: u32::from_le_bytes(chunk[24..28].try_into().unwrap()),
                flags: u32::from_le_bytes(chunk[28..32].try_into().unwrap()),
            })
        })
        .collect()
}

fn read_lods(bytes: &[u8]) -> Result<Vec<LodDesc>, DecodeError> {
    bytes
        .chunks_exact(LOD_BYTES)
        .map(|chunk| {
            Ok(LodDesc {
                bounds: read_f32x4(chunk, 0)?,
                error: read_f32(chunk, 16)?,
                part: u32::from_le_bytes(chunk[20..24].try_into().unwrap()),
                meshlet_first: u32::from_le_bytes(chunk[24..28].try_into().unwrap()),
                meshlet_count: u32::from_le_bytes(chunk[28..32].try_into().unwrap()),
                triangle_count: u32::from_le_bytes(chunk[32..36].try_into().unwrap()),
                level: u32::from_le_bytes(chunk[36..40].try_into().unwrap()),
                index_count: u32::from_le_bytes(chunk[40..44].try_into().unwrap()),
                flags: u32::from_le_bytes(chunk[44..48].try_into().unwrap()),
            })
        })
        .collect()
}

fn read_meshlets(bytes: &[u8]) -> Result<Vec<MeshletDesc>, DecodeError> {
    bytes
        .chunks_exact(MESHLET_BYTES)
        .map(|chunk| {
            Ok(MeshletDesc {
                bounds: read_f32x4(chunk, 0)?,
                cone_axis: [
                    read_f32(chunk, 16)?,
                    read_f32(chunk, 20)?,
                    read_f32(chunk, 24)?,
                ],
                cone_cutoff: read_f32(chunk, 28)?,
                vertex_offset: u32::from_le_bytes(chunk[32..36].try_into().unwrap()),
                triangle_offset: u32::from_le_bytes(chunk[36..40].try_into().unwrap()),
                vertex_count: u16::from_le_bytes(chunk[40..42].try_into().unwrap()),
                triangle_count: u16::from_le_bytes(chunk[42..44].try_into().unwrap()),
                part: u16::from_le_bytes(chunk[44..46].try_into().unwrap()),
                level: u16::from_le_bytes(chunk[46..48].try_into().unwrap()),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_airframe(vertex_count: usize) -> BakedAirframe {
        BakedAirframe {
            stream: vec![0; VERTEX_BYTES * vertex_count],
            opaque: Vec::new(),
            glass: Vec::new(),
            rt_idx: Vec::new(),
            rt_geom_nodes: Vec::new(),
            rt_node_ranges: Vec::new(),
            parts: Vec::new(),
            lods: Vec::new(),
            meshlets: Vec::new(),
            meshlet_vertices: Vec::new(),
            meshlet_triangles: Vec::new(),
        }
    }

    #[test]
    fn round_trip_preserves_all_sections() {
        let asset = BakedAirframe {
            stream: vec![0; VERTEX_BYTES * 3],
            opaque: vec![0, 1, 2],
            glass: vec![1],
            rt_idx: vec![0, 1, 2],
            rt_geom_nodes: vec![2],
            rt_node_ranges: vec![(0, 3)],
            parts: vec![PartDesc {
                bounds: [0.0, 0.0, 0.0, 1.0],
                node: 2,
                material: 0,
                lod_first: 0,
                lod_count: 2,
                flags: 0,
            }],
            lods: vec![
                LodDesc {
                    bounds: [0.0, 0.0, 0.0, 1.0],
                    error: 0.0,
                    part: 0,
                    meshlet_first: 0,
                    meshlet_count: 1,
                    triangle_count: 1,
                    level: 0,
                    index_count: 3,
                    flags: 0,
                },
                LodDesc {
                    bounds: [0.0, 0.0, 0.0, 1.0],
                    error: 0.05,
                    part: 0,
                    meshlet_first: 1,
                    meshlet_count: 1,
                    triangle_count: 1,
                    level: 1,
                    index_count: 3,
                    flags: 0,
                },
            ],
            meshlets: vec![
                MeshletDesc {
                    bounds: [0.0, 0.0, 0.0, 1.0],
                    cone_axis: [0.0, 0.0, 1.0],
                    cone_cutoff: 0.1,
                    vertex_offset: 0,
                    triangle_offset: 0,
                    vertex_count: 3,
                    triangle_count: 1,
                    part: 0,
                    level: 0,
                },
                MeshletDesc {
                    bounds: [0.0, 0.0, 0.0, 1.0],
                    cone_axis: [0.0, 0.0, 1.0],
                    cone_cutoff: 0.1,
                    vertex_offset: 3,
                    triangle_offset: 3,
                    vertex_count: 3,
                    triangle_count: 1,
                    part: 0,
                    level: 1,
                },
            ],
            meshlet_vertices: vec![0, 1, 2, 0, 1, 2],
            meshlet_triangles: vec![0, 1, 2, 0, 1, 2],
        };
        let encoded = asset.encode();
        assert_eq!(decode(&encoded).unwrap(), asset);
    }

    #[test]
    fn round_trip_accepts_an_asset_without_hierarchy() {
        let asset = empty_airframe(2);
        let encoded = asset.encode();
        assert_eq!(decode(&encoded).unwrap(), asset);
    }

    #[test]
    fn rejects_trailing_data() {
        let mut encoded = empty_airframe(1).encode();
        encoded.push(0);
        assert_eq!(
            decode(&encoded),
            Err(DecodeError::Invalid("trailing bytes"))
        );
    }

    #[test]
    fn rejects_version_one_payloads() {
        let mut encoded = empty_airframe(1).encode();
        encoded[8..12].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(
            decode(&encoded),
            Err(DecodeError::Invalid("unsupported version"))
        );
    }

    #[test]
    fn rejects_local_indices_outside_a_meshlet() {
        let mut asset = empty_airframe(3);
        asset.parts.push(PartDesc {
            bounds: [0.0; 4],
            node: 0,
            material: 0,
            lod_first: 0,
            lod_count: 1,
            flags: 0,
        });
        asset.lods.push(LodDesc {
            bounds: [0.0; 4],
            error: 0.0,
            part: 0,
            meshlet_first: 0,
            meshlet_count: 1,
            triangle_count: 1,
            level: 0,
            index_count: 3,
            flags: 0,
        });
        asset.meshlets.push(MeshletDesc {
            bounds: [0.0, 0.0, 0.0, 1.0],
            cone_axis: [0.0, 0.0, 1.0],
            cone_cutoff: 0.0,
            vertex_offset: 0,
            triangle_offset: 0,
            vertex_count: 3,
            triangle_count: 1,
            part: 0,
            level: 0,
        });
        asset.meshlet_vertices = vec![0, 1, 2];
        asset.meshlet_triangles = vec![0, 1, 3];
        assert_eq!(
            asset.validate(),
            Err("meshlet local index is outside its vertex range")
        );
    }

    #[test]
    fn struct_sizes_match_the_packed_strides() {
        assert_eq!(std::mem::size_of::<PartDesc>(), PART_BYTES);
        assert_eq!(std::mem::size_of::<LodDesc>(), LOD_BYTES);
        assert_eq!(std::mem::size_of::<MeshletDesc>(), MESHLET_BYTES);
    }
}
