//! Versioned packed airframe asset format shared by the build-time baker and
//! the runtime Vulkan loader. The format is deliberately little-endian and
//! self-describing so generated geometry can be validated before GPU upload.
//!
//! Format 2 replaces the version-1 count header with a fixed header and a
//! section table. Each `SectionDesc` stores an explicit offset, length,
//! alignment, element count, and element stride. The baker writes opaque and
//! glass indices into one contiguous raster section in final GPU draw order and
//! records `opaque_count`, `glass_first`, and `glass_count` in the header, so
//! the runtime path is validate the header, copy byte ranges to a staging
//! buffer, then hand the ranges to the GPU.
//!
//! Version 1 payloads still decode. They synthesize a section table from the
//! sequential layout and expose the same borrowed `AirframeView`.
//!
//! The version-2 hierarchy keeps the part, LOD, and meshlet tables behind
//! feature flags: `FEATURE_MESH_LODS`, `FEATURE_MESHLETS`, and the reserved
//! `FEATURE_IMPOSTOR_LODS`. Selection is per aircraft component by projected
//! screen-space geometric error, not by fixed metres of camera distance.

/// Eight-byte file signature.
pub const MAGIC: [u8; 8] = *b"EXAFRM01";
/// Current packed asset format version (fixed header plus section table).
pub const VERSION: u32 = 2;
/// Oldest payload version `decode` still accepts.
pub const VERSION_1: u32 = 1;
/// Runtime vertex stride: position, oct normal, half UV, flex, node/material IDs.
pub const VERTEX_BYTES: usize = 28;
/// Number of animation-node slots in the shared UBO.
pub const NODE_COUNT: usize = 23;
/// Size of one packed `PartDesc`.
pub const PART_BYTES: usize = 40;
/// Size of one packed `LodDesc`.
pub const LOD_BYTES: usize = 52;
/// Size of one packed `MeshletDesc` (matches the task-shader std430 stride).
pub const MESHLET_BYTES: usize = 48;
/// Size of one packed `SectionDesc` in the version-2 table.
pub const SECTION_BYTES: usize = 24;
/// Fixed header bytes before the section table.
pub const HEADER_BYTES: usize = 48;
/// Upper bound on vertices a single meshlet may reference.
pub const MESHLET_MAX_VERTICES: usize = 64;
/// Upper bound on triangles a single meshlet may contain.
pub const MESHLET_MAX_TRIANGLES: usize = 126;

/// Level-0 raster indices are present for the legacy two-draw path.
pub const FEATURE_RASTER: u32 = 1 << 0;
/// Per-component part and LOD tables are present.
pub const FEATURE_MESH_LODS: u32 = 1 << 1;
/// Baked meshlet clusters and payloads are present.
pub const FEATURE_MESHLETS: u32 = 1 << 2;
/// Reserved for reshadeable far-field impostor chains. Never set by the baker.
pub const FEATURE_IMPOSTOR_LODS: u32 = 1 << 3;
/// Ray-tracing proxy indices and per-node ranges are present.
pub const FEATURE_RT: u32 = 1 << 4;
/// Every feature bit this decoder understands.
pub const FEATURES_KNOWN: u32 = FEATURE_RASTER
    | FEATURE_MESH_LODS
    | FEATURE_MESHLETS
    | FEATURE_IMPOSTOR_LODS
    | FEATURE_RT;

/// Bit 0 of `PartDesc::flags`: transparent canopy glass.
pub const PART_FLAG_GLASS: u32 = 1;

/// Section kinds in the version-2 table.
pub mod section_kind {
    /// Packed vertex stream, `VERTEX_BYTES` stride.
    pub const VERTEX_DATA: u32 = 0;
    /// u16 raster indices in GPU draw order: opaque, then glass.
    pub const RASTER_INDICES: u32 = 1;
    /// u16 indices for the ray-tracing proxy.
    pub const RT_INDICES: u32 = 2;
    /// u32 animation-node IDs parallel to `RT_RANGES`.
    pub const RT_NODES: u32 = 3;
    /// Packed `(u32 index_offset, u32 index_count)` pairs.
    pub const RT_RANGES: u32 = 4;
    /// `PartDesc` records.
    pub const PARTS: u32 = 5;
    /// `LodDesc` records.
    pub const LODS: u32 = 6;
    /// `MeshletDesc` records.
    pub const MESHLETS: u32 = 7;
    /// u32 global vertex indices referenced by meshlets.
    pub const MESHLET_VERTICES: u32 = 8;
    /// Meshlet-local u8 triangle indices, three bytes per triangle.
    pub const MESHLET_TRIANGLES: u32 = 9;
}

/// Number of section kinds defined by format 2.
pub const SECTION_KIND_COUNT: u32 = 10;

/// One section-table entry: where a payload lives and how to address it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct SectionDesc {
    /// One of the `section_kind` constants.
    pub kind: u32,
    /// Absolute byte offset from the start of the asset.
    pub offset: u32,
    /// Payload length in bytes.
    pub length: u32,
    /// Required alignment of `offset` as a power-of-two byte count.
    pub alignment: u32,
    /// Number of addressable elements in the section.
    pub element_count: u32,
    /// Bytes per element. Zero means raw byte payload.
    pub element_stride: u32,
}

/// Fully packed immutable airframe data owned by the baker.
///
/// `raster` holds opaque indices followed by glass indices in final GPU draw
/// order. The runtime never concatenates or clones these ranges.
#[derive(Clone, Debug, PartialEq)]
pub struct BakedAirframe {
    pub stream: Vec<u8>,
    /// Opaque indices followed by glass indices.
    pub raster: Vec<u16>,
    /// Count of leading opaque indices in `raster`.
    pub opaque_count: u32,
    /// Count of trailing glass indices in `raster`.
    pub glass_count: u32,
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
    /// First vertex of this part in the packed stream.
    pub vertex_first: u32,
    /// Vertex count of this part in the packed stream.
    pub vertex_count: u32,
    /// Bit 0: glass part (alpha pass, excluded from RT).
    pub flags: u32,
}

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
    /// Level 0: first u16 in the raster section for this part. Higher levels
    /// store meshlets only and leave this at 0.
    pub index_first: u32,
    /// Flat index count (3 × triangles). Level 0 ranges address `raster`.
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
    /// First glass index in `raster`. Always equal to `opaque_count`.
    pub fn glass_first(&self) -> u32 {
        self.opaque_count
    }

    /// Feature bits this asset exposes to the runtime.
    pub fn features(&self) -> u32 {
        let mut features = 0;
        if !self.raster.is_empty() {
            features |= FEATURE_RASTER;
        }
        if !self.rt_idx.is_empty() && !self.rt_node_ranges.is_empty() {
            features |= FEATURE_RT;
        }
        if !self.parts.is_empty() && !self.lods.is_empty() {
            features |= FEATURE_MESH_LODS;
        }
        if !self.meshlets.is_empty() {
            features |= FEATURE_MESHLETS;
        }
        features
    }

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
        if self.glass_first() != self.opaque_count {
            return Err("glass first does not equal opaque count");
        }
        let raster_total = (self.opaque_count as usize).checked_add(self.glass_count as usize);
        if raster_total != Some(self.raster.len()) {
            return Err("raster length does not match opaque plus glass counts");
        }
        let valid_index = |&index: &u16| (index as usize) < vertices;
        if !self.raster.iter().all(valid_index) || !self.rt_idx.iter().all(valid_index) {
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
            let vertex_end = part
                .vertex_first
                .checked_add(part.vertex_count)
                .ok_or("part vertex range overflows")?;
            if vertex_end as usize > vertices {
                return Err("part vertex range exceeds the packed stream");
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
                if level == 0 {
                    let index_end = lod
                        .index_first
                        .checked_add(lod.index_count)
                        .ok_or("LOD raster range overflows")?;
                    if index_end as usize > self.raster.len() {
                        return Err("LOD raster range exceeds raster indices");
                    }
                    let start = lod.index_first as usize;
                    let stop = index_end as usize;
                    if stop <= self.opaque_count as usize {
                        // Fully inside the opaque half.
                    } else if start >= self.opaque_count as usize {
                        if (part.flags & PART_FLAG_GLASS) == 0 {
                            return Err("opaque part level-0 indices live in the glass half");
                        }
                    } else {
                        return Err("LOD raster range crosses the opaque and glass halves");
                    }
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
        let features = self.features();
        let vertex_count = (self.stream.len() / VERTEX_BYTES) as u32;
        let mut sections: Vec<(SectionDesc, Vec<u8>)> = Vec::new();

        let mut push = |kind: u32,
                        alignment: u32,
                        element_stride: u32,
                        element_count: u32,
                        data: Vec<u8>| {
            sections.push((
                SectionDesc {
                    kind,
                    offset: 0,
                    length: data.len() as u32,
                    alignment,
                    element_count,
                    element_stride,
                },
                data,
            ));
        };

        push(
            section_kind::VERTEX_DATA,
            16,
            VERTEX_BYTES as u32,
            vertex_count,
            self.stream.clone(),
        );
        push(
            section_kind::RASTER_INDICES,
            4,
            2,
            self.raster.len() as u32,
            u16_bytes(&self.raster),
        );
        push(
            section_kind::RT_INDICES,
            4,
            2,
            self.rt_idx.len() as u32,
            u16_bytes(&self.rt_idx),
        );
        push(
            section_kind::RT_NODES,
            4,
            4,
            self.rt_geom_nodes.len() as u32,
            u32_bytes(&self.rt_geom_nodes),
        );
        let mut range_data = Vec::with_capacity(self.rt_node_ranges.len() * 8);
        for &(offset, count) in &self.rt_node_ranges {
            range_data.extend_from_slice(&offset.to_le_bytes());
            range_data.extend_from_slice(&count.to_le_bytes());
        }
        push(
            section_kind::RT_RANGES,
            4,
            8,
            self.rt_node_ranges.len() as u32,
            range_data,
        );
        let mut part_data = Vec::with_capacity(self.parts.len() * PART_BYTES);
        for part in &self.parts {
            push_f32x4(&mut part_data, part.bounds);
            part_data.extend_from_slice(&part.node.to_le_bytes());
            part_data.extend_from_slice(&part.material.to_le_bytes());
            push_u32(&mut part_data, part.lod_first);
            push_u32(&mut part_data, part.lod_count);
            push_u32(&mut part_data, part.vertex_first);
            push_u32(&mut part_data, part.vertex_count);
            push_u32(&mut part_data, part.flags);
        }
        push(
            section_kind::PARTS,
            4,
            PART_BYTES as u32,
            self.parts.len() as u32,
            part_data,
        );
        let mut lod_data = Vec::with_capacity(self.lods.len() * LOD_BYTES);
        for lod in &self.lods {
            push_f32x4(&mut lod_data, lod.bounds);
            push_f32(&mut lod_data, lod.error);
            push_u32(&mut lod_data, lod.part);
            push_u32(&mut lod_data, lod.meshlet_first);
            push_u32(&mut lod_data, lod.meshlet_count);
            push_u32(&mut lod_data, lod.triangle_count);
            push_u32(&mut lod_data, lod.level);
            push_u32(&mut lod_data, lod.index_first);
            push_u32(&mut lod_data, lod.index_count);
            push_u32(&mut lod_data, lod.flags);
        }
        push(
            section_kind::LODS,
            4,
            LOD_BYTES as u32,
            self.lods.len() as u32,
            lod_data,
        );
        let mut meshlet_data = Vec::with_capacity(self.meshlets.len() * MESHLET_BYTES);
        for meshlet in &self.meshlets {
            push_f32x4(&mut meshlet_data, meshlet.bounds);
            for axis in meshlet.cone_axis {
                push_f32(&mut meshlet_data, axis);
            }
            push_f32(&mut meshlet_data, meshlet.cone_cutoff);
            push_u32(&mut meshlet_data, meshlet.vertex_offset);
            push_u32(&mut meshlet_data, meshlet.triangle_offset);
            meshlet_data.extend_from_slice(&meshlet.vertex_count.to_le_bytes());
            meshlet_data.extend_from_slice(&meshlet.triangle_count.to_le_bytes());
            meshlet_data.extend_from_slice(&meshlet.part.to_le_bytes());
            meshlet_data.extend_from_slice(&meshlet.level.to_le_bytes());
        }
        push(
            section_kind::MESHLETS,
            4,
            MESHLET_BYTES as u32,
            self.meshlets.len() as u32,
            meshlet_data,
        );
        push(
            section_kind::MESHLET_VERTICES,
            4,
            4,
            self.meshlet_vertices.len() as u32,
            u32_bytes(&self.meshlet_vertices),
        );
        push(
            section_kind::MESHLET_TRIANGLES,
            1,
            1,
            self.meshlet_triangles.len() as u32,
            self.meshlet_triangles.clone(),
        );

        let section_count = sections.len() as u32;
        let table_bytes = section_count as usize * SECTION_BYTES;
        let mut header_and_table = Vec::with_capacity(HEADER_BYTES + table_bytes);
        header_and_table.extend_from_slice(&MAGIC);
        push_u32(&mut header_and_table, VERSION);
        push_u32(
            &mut header_and_table,
            (HEADER_BYTES + table_bytes) as u32,
        );
        push_u32(&mut header_and_table, VERTEX_BYTES as u32);
        push_u32(&mut header_and_table, features);
        push_u32(&mut header_and_table, section_count);
        push_u32(&mut header_and_table, vertex_count);
        push_u32(&mut header_and_table, self.opaque_count);
        push_u32(&mut header_and_table, self.glass_first());
        push_u32(&mut header_and_table, self.glass_count);
        push_u32(&mut header_and_table, 0);

        let mut cursor = (HEADER_BYTES + table_bytes) as u32;
        for (desc, _) in sections.iter_mut() {
            cursor = align_up(cursor, desc.alignment);
            desc.offset = cursor;
            cursor = cursor
                .checked_add(desc.length)
                .expect("airframe asset size overflow");
        }

        for (desc, _) in sections.iter() {
            push_u32(&mut header_and_table, desc.kind);
            push_u32(&mut header_and_table, desc.offset);
            push_u32(&mut header_and_table, desc.length);
            push_u32(&mut header_and_table, desc.alignment);
            push_u32(&mut header_and_table, desc.element_count);
            push_u32(&mut header_and_table, desc.element_stride);
        }

        let capacity = cursor as usize;
        let mut out = Vec::with_capacity(capacity);
        out.extend_from_slice(&header_and_table);
        for (desc, data) in sections.iter() {
            let start = desc.offset as usize;
            if out.len() < start + data.len() {
                out.resize(start + data.len(), 0);
            }
            out[start..start + data.len()].copy_from_slice(data);
        }
        debug_assert_eq!(out.len(), capacity);
        out
    }
}

/// Borrowed, validated view over a packed airframe asset.
///
/// The runtime keeps this over `include_bytes!` data and copies section byte
/// ranges straight to GPU staging. Structured tables are read through
/// iterators instead of being decoded into owned vectors.
#[derive(Clone, Copy, Debug)]
pub struct AirframeView<'a> {
    /// Feature bits set in the header.
    pub features: u32,
    /// Vertex stride in bytes.
    pub vertex_stride: u32,
    /// Number of vertices in `vertex_data`.
    pub vertex_count: u32,
    /// Leading opaque index count in `raster_indices`.
    pub opaque_count: u32,
    /// First glass index in `raster_indices`.
    pub glass_first: u32,
    /// Trailing glass index count in `raster_indices`.
    pub glass_count: u32,
    /// Packed vertex stream.
    pub vertex_data: &'a [u8],
    /// u16 raster indices, opaque then glass, in GPU draw order.
    pub raster_indices: &'a [u8],
    /// u16 ray-tracing proxy indices.
    pub rt_indices: &'a [u8],
    /// u32 animation-node IDs for the RT ranges.
    pub rt_nodes: &'a [u8],
    /// Packed RT `(offset, count)` pairs.
    pub rt_ranges: &'a [u8],
    /// Packed `PartDesc` records.
    pub parts: &'a [u8],
    /// Packed `LodDesc` records.
    pub lods: &'a [u8],
    /// Packed `MeshletDesc` records.
    pub meshlets: &'a [u8],
    /// u32 meshlet vertex indices.
    pub meshlet_vertices: &'a [u8],
    /// Meshlet-local u8 triangle indices.
    pub meshlet_triangles: &'a [u8],
}

impl<'a> AirframeView<'a> {
    /// Whether every bit in `mask` is set.
    pub fn has_features(&self, mask: u32) -> bool {
        self.features & mask == mask
    }

    /// Number of packed parts.
    pub fn part_count(&self) -> usize {
        self.parts.len() / PART_BYTES
    }

    /// Number of packed LOD records.
    pub fn lod_count(&self) -> usize {
        self.lods.len() / LOD_BYTES
    }

    /// Number of packed meshlets.
    pub fn meshlet_count(&self) -> usize {
        self.meshlets.len() / MESHLET_BYTES
    }

    /// Iterate the part table.
    pub fn part_iter(&self) -> impl Iterator<Item = PartDesc> + 'a {
        read_parts(self.parts).into_iter()
    }

    /// Iterate the LOD table.
    pub fn lod_iter(&self) -> impl Iterator<Item = LodDesc> + 'a {
        read_lods(self.lods).into_iter()
    }

    /// Iterate the meshlet table.
    pub fn meshlet_iter(&self) -> impl Iterator<Item = MeshletDesc> + 'a {
        read_meshlets(self.meshlets).into_iter()
    }

    /// Iterate RT node IDs.
    pub fn rt_node_iter(&self) -> impl Iterator<Item = u32> + 'a {
        read_u32s(self.rt_nodes).into_iter()
    }

    /// Iterate RT `(index_offset, index_count)` pairs.
    pub fn rt_range_iter(&self) -> impl Iterator<Item = (u32, u32)> + 'a {
        self.rt_ranges.chunks_exact(8).map(|chunk| {
            (
                u32::from_le_bytes(chunk[0..4].try_into().unwrap()),
                u32::from_le_bytes(chunk[4..8].try_into().unwrap()),
            )
        })
    }

    /// Iterate raster u16 indices in GPU draw order.
    pub fn raster_iter(&self) -> impl Iterator<Item = u16> + 'a {
        read_u16s(self.raster_indices).into_iter()
    }

    /// Ray-tracing index count.
    pub fn rt_index_count(&self) -> usize {
        self.rt_indices.len() / 2
    }

    /// Select the LOD level for one part from projected geometric error.
    ///
    /// `error_to_pixels` converts object-space metres to pixels for the
    /// current camera, for example
    /// `0.5 * viewport_height / (tan(fov_y / 2) * distance_to_part)`.
    /// Returns the coarsest level whose projected error stays within
    /// `max_error_px`. Levels are finest first with non-decreasing `error`.
    pub fn select_lod(
        &self,
        part_index: usize,
        error_to_pixels: f32,
        max_error_px: f32,
    ) -> usize {
        let part = match self.part_iter().nth(part_index) {
            Some(part) => part,
            None => return 0,
        };
        let start = part.lod_first as usize;
        let end = start + part.lod_count as usize;
        if end > self.lod_count() {
            return 0;
        }
        select_lod_range(&self.lods[start * LOD_BYTES..end * LOD_BYTES], error_to_pixels, max_error_px)
    }

    /// Compare a decoded view against the baker-owned asset it came from.
    pub fn matches(&self, asset: &BakedAirframe) -> bool {
        self.features == asset.features()
            && self.vertex_stride as usize == VERTEX_BYTES
            && self.vertex_count as usize * VERTEX_BYTES == asset.stream.len()
            && self.opaque_count == asset.opaque_count
            && self.glass_first == asset.glass_first()
            && self.glass_count == asset.glass_count
            && self.vertex_data == asset.stream.as_slice()
            && read_u16s(self.raster_indices) == asset.raster
            && read_u16s(self.rt_indices) == asset.rt_idx
            && read_u32s(self.rt_nodes) == asset.rt_geom_nodes
            && self.rt_range_iter().eq(asset.rt_node_ranges.iter().copied())
            && read_parts(self.parts) == asset.parts
            && read_lods(self.lods) == asset.lods
            && read_meshlets(self.meshlets) == asset.meshlets
            && read_u32s(self.meshlet_vertices) == asset.meshlet_vertices
            && self.meshlet_triangles == asset.meshlet_triangles.as_slice()
    }

    /// Re-run the semantic checks the decoder already performed.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.vertex_stride as usize != VERTEX_BYTES {
            return Err("unexpected vertex stride");
        }
        if self.vertex_data.len() != self.vertex_count as usize * VERTEX_BYTES {
            return Err("vertex section length does not match vertex count");
        }
        if self.vertex_count > u16::MAX as u32 + 1 {
            return Err("vertex stream exceeds UINT16 index space");
        }
        if self.glass_first != self.opaque_count {
            return Err("glass first does not equal opaque count");
        }
        let raster_total = (self.opaque_count as usize).checked_add(self.glass_count as usize);
        if raster_total.map(|total| total * 2) != Some(self.raster_indices.len()) {
            return Err("raster length does not match opaque plus glass counts");
        }
        if self.raster_indices.len() % 2 != 0
            || self.rt_indices.len() % 2 != 0
            || self.rt_nodes.len() % 4 != 0
            || self.rt_ranges.len() % 8 != 0
            || self.parts.len() % PART_BYTES != 0
            || self.lods.len() % LOD_BYTES != 0
            || self.meshlets.len() % MESHLET_BYTES != 0
            || self.meshlet_vertices.len() % 4 != 0
        {
            return Err("section length is not element aligned");
        }
        if self.meshlet_triangles.len() % 3 != 0 {
            return Err("meshlet triangle stream is not triangle aligned");
        }
        if (self.features & !FEATURES_KNOWN) != 0 {
            return Err("unknown feature bits");
        }
        if self.has_features(FEATURE_MESH_LODS)
            && (self.parts.is_empty() || self.lods.is_empty())
        {
            return Err("mesh LOD feature set without part or LOD data");
        }
        if self.has_features(FEATURE_MESHLETS) && self.meshlets.is_empty() {
            return Err("meshlet feature set without meshlet data");
        }
        if self.has_features(FEATURE_IMPOSTOR_LODS) {
            return Err("impostor LOD feature is reserved and not yet implemented");
        }
        if self.has_features(FEATURE_RASTER) && self.raster_indices.is_empty() {
            return Err("raster feature set without raster indices");
        }
        if self.has_features(FEATURE_RT)
            && (self.rt_indices.is_empty() || self.rt_ranges.is_empty())
        {
            return Err("RT feature set without RT index or range data");
        }

        let vertices = self.vertex_count as usize;
        let valid_index = |index: u16| (index as usize) < vertices;
        if self.raster_iter().any(|index| !valid_index(index))
            || read_u16s(self.rt_indices).iter().any(|&index| !valid_index(index))
        {
            return Err("index references a vertex outside the packed stream");
        }
        let rt_nodes = read_u32s(self.rt_nodes);
        let rt_ranges = self.rt_range_iter().collect::<Vec<_>>();
        if rt_nodes.len() != rt_ranges.len() {
            return Err("RT node and range counts differ");
        }
        if rt_nodes.iter().any(|&node| node as usize >= NODE_COUNT) {
            return Err("RT node is outside the animation-node table");
        }
        for &(offset, count) in &rt_ranges {
            let end = offset.checked_add(count).ok_or("RT node range overflows")?;
            if end as usize > self.rt_index_count() {
                return Err("RT node range exceeds RT index data");
            }
            if count % 3 != 0 {
                return Err("RT node range is not triangle aligned");
            }
        }

        let parts = read_parts(self.parts);
        let lods = read_lods(self.lods);
        let meshlets = read_meshlets(self.meshlets);
        let meshlet_vertices = read_u32s(self.meshlet_vertices);
        let raster_count = self.raster_indices.len() / 2;
        for (index, part) in parts.iter().enumerate() {
            if part.node as usize >= NODE_COUNT {
                return Err("part node is outside the animation-node table");
            }
            let vertex_end = part
                .vertex_first
                .checked_add(part.vertex_count)
                .ok_or("part vertex range overflows")?;
            if vertex_end as usize > vertices {
                return Err("part vertex range exceeds the packed stream");
            }
            let end = part
                .lod_first
                .checked_add(part.lod_count)
                .ok_or("part LOD range overflows")?;
            if end as usize > lods.len() {
                return Err("part LOD range exceeds LOD table");
            }
            if part.lod_count == 0 {
                return Err("part has no LOD levels");
            }
            for (level, lod) in lods[part.lod_first as usize..end as usize]
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
                if mesh_end as usize > meshlets.len() {
                    return Err("LOD meshlet range exceeds meshlet table");
                }
                if lod.triangle_count as usize * 3 != lod.index_count as usize {
                    return Err("LOD triangle count does not match index count");
                }
                if level == 0 {
                    let index_end = lod
                        .index_first
                        .checked_add(lod.index_count)
                        .ok_or("LOD raster range overflows")?;
                    if index_end as usize > raster_count {
                        return Err("LOD raster range exceeds raster indices");
                    }
                    let start = lod.index_first as usize;
                    if index_end as usize <= self.opaque_count as usize {
                        if (part.flags & PART_FLAG_GLASS) != 0 {
                            return Err("glass part level-0 indices live in the opaque half");
                        }
                    } else if start >= self.opaque_count as usize {
                        if (part.flags & PART_FLAG_GLASS) == 0 {
                            return Err("opaque part level-0 indices live in the glass half");
                        }
                    } else {
                        return Err("LOD raster range crosses the opaque and glass halves");
                    }
                }
            }
        }
        for meshlet in &meshlets {
            if meshlet.part as usize >= parts.len() {
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
            if vertex_end as usize > meshlet_vertices.len() {
                return Err("meshlet vertex range exceeds meshlet vertex stream");
            }
            if meshlet_vertices[meshlet.vertex_offset as usize..vertex_end as usize]
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
            if local.iter().any(|&i| i >= meshlet.vertex_count as u8) {
                return Err("meshlet local index is outside its vertex range");
            }
        }
        if let Some(first) = lods.first() {
            if first.level != 0 {
                return Err("first LOD is not full resolution");
            }
        }
        Ok(())
    }
}

impl PartialEq for AirframeView<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.features == other.features
            && self.vertex_stride == other.vertex_stride
            && self.vertex_count == other.vertex_count
            && self.opaque_count == other.opaque_count
            && self.glass_first == other.glass_first
            && self.glass_count == other.glass_count
            && self.vertex_data == other.vertex_data
            && self.raster_indices == other.raster_indices
            && self.rt_indices == other.rt_indices
            && self.rt_nodes == other.rt_nodes
            && self.rt_ranges == other.rt_ranges
            && self.parts == other.parts
            && self.lods == other.lods
            && self.meshlets == other.meshlets
            && self.meshlet_vertices == other.meshlet_vertices
            && self.meshlet_triangles == other.meshlet_triangles
    }
}

impl PartialEq<BakedAirframe> for AirframeView<'_> {
    fn eq(&self, other: &BakedAirframe) -> bool {
        self.matches(other)
    }
}

impl PartialEq<AirframeView<'_>> for BakedAirframe {
    fn eq(&self, other: &AirframeView<'_>) -> bool {
        other.matches(self)
    }
}

/// Vertical screen-space size of a projected bounding sphere.
///
/// Returns the sphere diameter as a fraction of viewport height for a
/// perspective camera. This is the screen-ratio selector rather than a
/// distance threshold.
pub fn screen_size_ratio(bounds_radius: f32, distance: f32, fov_y_radians: f32) -> f32 {
    let d = distance.max(1e-6);
    let half_tan = (0.5 * fov_y_radians.max(1e-6)).tan();
    bounds_radius.max(0.0) / (d * half_tan)
}

/// Pick the coarsest LOD whose projected geometric error fits the budget.
///
/// `lods` is the packed byte range of one part's `LodDesc` chain, finest
/// first. Returns 0 when the chain is empty.
pub fn select_lod_range(lods: &[u8], error_to_pixels: f32, max_error_px: f32) -> usize {
    let records = lods.len() / LOD_BYTES;
    let mut chosen = 0;
    for level in 0..records {
        let lod = read_lod(&lods[level * LOD_BYTES..(level + 1) * LOD_BYTES]);
        if lod.error * error_to_pixels <= max_error_px {
            chosen = level;
        } else {
            break;
        }
    }
    chosen
}

/// Decode and validate a generated asset before handing it to the renderer.
///
/// Accepts format 1 (sequential count header) and format 2 (section table).
/// The returned view borrows `bytes`.
pub fn decode(bytes: &[u8]) -> Result<AirframeView<'_>, DecodeError> {
    if bytes.len() < 12 {
        return Err(DecodeError::Truncated {
            needed: 12,
            available: bytes.len(),
        });
    }
    if bytes[..MAGIC.len()] != MAGIC {
        return Err(DecodeError::Invalid("bad magic"));
    }
    let version = read_u32(bytes, 8)?;
    match version {
        VERSION_1 => decode_v1(bytes),
        VERSION => decode_v2(bytes),
        _ => Err(DecodeError::Invalid("unsupported version")),
    }
}

fn decode_v2<'a>(bytes: &'a [u8]) -> Result<AirframeView<'a>, DecodeError> {
    if bytes.len() < HEADER_BYTES {
        return Err(DecodeError::Truncated {
            needed: HEADER_BYTES,
            available: bytes.len(),
        });
    }
    let header_bytes = read_u32(bytes, 12)? as usize;
    let vertex_stride = read_u32(bytes, 16)?;
    let features = read_u32(bytes, 20)?;
    let section_count = read_u32(bytes, 24)? as usize;
    let vertex_count = read_u32(bytes, 28)?;
    let opaque_count = read_u32(bytes, 32)?;
    let glass_first = read_u32(bytes, 36)?;
    let glass_count = read_u32(bytes, 40)?;
    if vertex_stride != VERTEX_BYTES as u32 {
        return Err(DecodeError::Invalid("unexpected vertex stride"));
    }
    if section_count == 0 || section_count > SECTION_KIND_COUNT as usize {
        return Err(DecodeError::Invalid("section count out of range"));
    }
    let expected_header = HEADER_BYTES
        .checked_add(section_count.checked_mul(SECTION_BYTES).ok_or(
            DecodeError::Invalid("section table size overflow"),
        )?)
        .ok_or(DecodeError::Invalid("header size overflow"))?;
    if header_bytes != expected_header {
        return Err(DecodeError::Invalid("unexpected header size"));
    }
    if (features & !FEATURES_KNOWN) != 0 {
        return Err(DecodeError::Invalid("unknown feature bits"));
    }
    if glass_first != opaque_count {
        return Err(DecodeError::Invalid("glass first does not equal opaque count"));
    }

    let table_end = header_bytes;
    if bytes.len() < table_end {
        return Err(DecodeError::Truncated {
            needed: table_end,
            available: bytes.len(),
        });
    }

    let mut slots: [Option<&'a [u8]>; SECTION_KIND_COUNT as usize] =
        [None; SECTION_KIND_COUNT as usize];
    let mut ranges: Vec<(u32, u32, u32)> = Vec::with_capacity(section_count);
    for index in 0..section_count {
        let base = HEADER_BYTES + index * SECTION_BYTES;
        let kind = read_u32(bytes, base)?;
        let offset = read_u32(bytes, base + 4)?;
        let length = read_u32(bytes, base + 8)?;
        let alignment = read_u32(bytes, base + 12)?;
        let element_count = read_u32(bytes, base + 16)?;
        let element_stride = read_u32(bytes, base + 20)?;
        if kind >= SECTION_KIND_COUNT {
            return Err(DecodeError::Invalid("unknown section kind"));
        }
        if alignment == 0 || !alignment.is_power_of_two() {
            return Err(DecodeError::Invalid("section alignment is not a power of two"));
        }
        if offset as usize % alignment as usize != 0 {
            return Err(DecodeError::Invalid("section offset violates alignment"));
        }
        let end = (offset as usize)
            .checked_add(length as usize)
            .ok_or(DecodeError::Invalid("section offset overflow"))?;
        if end > bytes.len() {
            return Err(DecodeError::Truncated {
                needed: end,
                available: bytes.len(),
            });
        }
        if end < table_end && length > 0 {
            return Err(DecodeError::Invalid("section overlaps the header"));
        }
        if slots[kind as usize].is_some() {
            return Err(DecodeError::Invalid("duplicate section kind"));
        }
        if length > 0 && element_stride > 0 {
            let expect = (element_count as u64)
                .checked_mul(element_stride as u64)
                .ok_or(DecodeError::Invalid("section size overflow"))?;
            if expect != length as u64 {
                return Err(DecodeError::Invalid(
                    "section length does not match element count and stride",
                ));
            }
        }
        slots[kind as usize] = Some(&bytes[offset as usize..end]);
        ranges.push((offset, end as u32, kind));
    }

    // Non-empty sections must not overlap each other.
    ranges.retain(|(start, end, _)| end > start);
    ranges.sort_by_key(|(start, _, _)| *start);
    for window in ranges.windows(2) {
        if window[0].1 > window[1].0 {
            return Err(DecodeError::Invalid("sections overlap"));
        }
    }

    let empty: &[u8] = &[];
    let take = |kind: u32| slots[kind as usize].unwrap_or(empty);
    let vertex_data = take(section_kind::VERTEX_DATA);
    let raster_indices = take(section_kind::RASTER_INDICES);
    let rt_indices = take(section_kind::RT_INDICES);
    let rt_nodes = take(section_kind::RT_NODES);
    let rt_ranges = take(section_kind::RT_RANGES);
    let parts = take(section_kind::PARTS);
    let lods = take(section_kind::LODS);
    let meshlets = take(section_kind::MESHLETS);
    let meshlet_vertices = take(section_kind::MESHLET_VERTICES);
    let meshlet_triangles = take(section_kind::MESHLET_TRIANGLES);

    for (kind, required) in [
        (section_kind::VERTEX_DATA, true),
        (section_kind::RASTER_INDICES, features & FEATURE_RASTER != 0),
        (section_kind::RT_INDICES, features & FEATURE_RT != 0),
        (section_kind::RT_NODES, features & FEATURE_RT != 0),
        (section_kind::RT_RANGES, features & FEATURE_RT != 0),
        (section_kind::PARTS, features & FEATURE_MESH_LODS != 0),
        (section_kind::LODS, features & FEATURE_MESH_LODS != 0),
        (section_kind::MESHLETS, features & FEATURE_MESHLETS != 0),
        (section_kind::MESHLET_VERTICES, features & FEATURE_MESHLETS != 0),
        (section_kind::MESHLET_TRIANGLES, features & FEATURE_MESHLETS != 0),
    ] {
        if required && slots[kind as usize].is_none() {
            return Err(DecodeError::Invalid("required section is missing"));
        }
    }

    let view = AirframeView {
        features,
        vertex_stride,
        vertex_count,
        opaque_count,
        glass_first,
        glass_count,
        vertex_data,
        raster_indices,
        rt_indices,
        rt_nodes,
        rt_ranges,
        parts,
        lods,
        meshlets,
        meshlet_vertices,
        meshlet_triangles,
    };
    if view.vertex_data.len() != view.vertex_count as usize * VERTEX_BYTES {
        return Err(DecodeError::Invalid(
            "vertex section length does not match vertex count",
        ));
    }
    view.validate().map_err(DecodeError::Invalid)?;
    Ok(view)
}

fn decode_v1(bytes: &[u8]) -> Result<AirframeView<'_>, DecodeError> {
    const V1_HEADER: usize = 48;
    if bytes.len() < V1_HEADER {
        return Err(DecodeError::Truncated {
            needed: V1_HEADER,
            available: bytes.len(),
        });
    }
    if read_u32(bytes, 12)? as usize != V1_HEADER {
        return Err(DecodeError::Invalid("unexpected header size"));
    }
    let vertex_stride = read_u32(bytes, 16)?;
    if vertex_stride != VERTEX_BYTES as u32 {
        return Err(DecodeError::Invalid("unexpected vertex stride"));
    }
    let stream_bytes = read_u32(bytes, 20)? as usize;
    let opaque_count = read_u32(bytes, 24)?;
    let glass_count = read_u32(bytes, 28)?;
    let rt_index_count = read_u32(bytes, 32)? as usize;
    let node_count = read_u32(bytes, 36)? as usize;
    let range_count = read_u32(bytes, 40)? as usize;

    let mut cursor = V1_HEADER;
    let vertex_data = take(bytes, &mut cursor, stream_bytes)?;
    let raster_bytes = bytes_for(
        (opaque_count as usize)
            .checked_add(glass_count as usize)
            .ok_or(DecodeError::Invalid("section size overflow"))?,
        2,
    )?;
    let raster_indices = take(bytes, &mut cursor, raster_bytes)?;
    let rt_indices = take(bytes, &mut cursor, bytes_for(rt_index_count, 2)?)?;
    let rt_nodes = take(bytes, &mut cursor, bytes_for(node_count, 4)?)?;
    let rt_ranges = take(bytes, &mut cursor, bytes_for(range_count, 8)?)?;
    if cursor != bytes.len() {
        return Err(DecodeError::Invalid("trailing bytes"));
    }

    let features = if raster_indices.is_empty() {
        0
    } else {
        FEATURE_RASTER
    } | if rt_indices.is_empty() || rt_ranges.is_empty() {
        0
    } else {
        FEATURE_RT
    };

    let view = AirframeView {
        features,
        vertex_stride,
        vertex_count: (stream_bytes / VERTEX_BYTES) as u32,
        opaque_count,
        glass_first: opaque_count,
        glass_count,
        vertex_data,
        raster_indices,
        rt_indices,
        rt_nodes,
        rt_ranges,
        parts: &[],
        lods: &[],
        meshlets: &[],
        meshlet_vertices: &[],
        meshlet_triangles: &[],
    };
    view.validate().map_err(DecodeError::Invalid)?;
    Ok(view)
}

fn align_up(offset: u32, alignment: u32) -> u32 {
    debug_assert!(alignment.is_power_of_two());
    (offset.wrapping_add(alignment - 1)) & !(alignment - 1)
}

fn u16_bytes(values: &[u16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 2);
    for value in values {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

fn u32_bytes(values: &[u32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 4);
    for value in values {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
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

fn read_f32(bytes: &[u8], offset: usize) -> f32 {
    f32::from_bits(read_u32(bytes, offset).expect("f32 read within slice"))
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

fn read_f32x4(bytes: &[u8], offset: usize) -> [f32; 4] {
    [
        read_f32(bytes, offset),
        read_f32(bytes, offset + 4),
        read_f32(bytes, offset + 8),
        read_f32(bytes, offset + 12),
    ]
}

fn read_parts(bytes: &[u8]) -> Vec<PartDesc> {
    bytes
        .chunks_exact(PART_BYTES)
        .map(|chunk| PartDesc {
            bounds: read_f32x4(chunk, 0),
            node: u16::from_le_bytes(chunk[16..18].try_into().unwrap()),
            material: u16::from_le_bytes(chunk[18..20].try_into().unwrap()),
            lod_first: u32::from_le_bytes(chunk[20..24].try_into().unwrap()),
            lod_count: u32::from_le_bytes(chunk[24..28].try_into().unwrap()),
            vertex_first: u32::from_le_bytes(chunk[28..32].try_into().unwrap()),
            vertex_count: u32::from_le_bytes(chunk[32..36].try_into().unwrap()),
            flags: u32::from_le_bytes(chunk[36..40].try_into().unwrap()),
        })
        .collect()
}

fn read_lod(chunk: &[u8]) -> LodDesc {
    LodDesc {
        bounds: [
            f32::from_bits(u32::from_le_bytes(chunk[0..4].try_into().unwrap())),
            f32::from_bits(u32::from_le_bytes(chunk[4..8].try_into().unwrap())),
            f32::from_bits(u32::from_le_bytes(chunk[8..12].try_into().unwrap())),
            f32::from_bits(u32::from_le_bytes(chunk[12..16].try_into().unwrap())),
        ],
        error: f32::from_bits(u32::from_le_bytes(chunk[16..20].try_into().unwrap())),
        part: u32::from_le_bytes(chunk[20..24].try_into().unwrap()),
        meshlet_first: u32::from_le_bytes(chunk[24..28].try_into().unwrap()),
        meshlet_count: u32::from_le_bytes(chunk[28..32].try_into().unwrap()),
        triangle_count: u32::from_le_bytes(chunk[32..36].try_into().unwrap()),
        level: u32::from_le_bytes(chunk[36..40].try_into().unwrap()),
        index_first: u32::from_le_bytes(chunk[40..44].try_into().unwrap()),
        index_count: u32::from_le_bytes(chunk[44..48].try_into().unwrap()),
        flags: u32::from_le_bytes(chunk[48..52].try_into().unwrap()),
    }
}

fn read_lods(bytes: &[u8]) -> Vec<LodDesc> {
    bytes.chunks_exact(LOD_BYTES).map(read_lod).collect()
}

fn read_meshlets(bytes: &[u8]) -> Vec<MeshletDesc> {
    bytes
        .chunks_exact(MESHLET_BYTES)
        .map(|chunk| MeshletDesc {
            bounds: read_f32x4(chunk, 0),
            cone_axis: [
                read_f32(chunk, 16),
                read_f32(chunk, 20),
                read_f32(chunk, 24),
            ],
            cone_cutoff: read_f32(chunk, 28),
            vertex_offset: u32::from_le_bytes(chunk[32..36].try_into().unwrap()),
            triangle_offset: u32::from_le_bytes(chunk[36..40].try_into().unwrap()),
            vertex_count: u16::from_le_bytes(chunk[40..42].try_into().unwrap()),
            triangle_count: u16::from_le_bytes(chunk[42..44].try_into().unwrap()),
            part: u16::from_le_bytes(chunk[44..46].try_into().unwrap()),
            level: u16::from_le_bytes(chunk[46..48].try_into().unwrap()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_airframe(vertex_count: usize) -> BakedAirframe {
        BakedAirframe {
            stream: vec![0; VERTEX_BYTES * vertex_count],
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
        }
    }

    #[test]
    fn round_trip_preserves_all_sections() {
        let asset = BakedAirframe {
            stream: vec![0; VERTEX_BYTES * 3],
            raster: vec![0, 1, 2, 1, 2, 0],
            opaque_count: 3,
            glass_count: 3,
            rt_idx: vec![0, 1, 2],
            rt_geom_nodes: vec![2],
            rt_node_ranges: vec![(0, 3)],
            parts: vec![
                PartDesc {
                    bounds: [0.0, 0.0, 0.0, 1.0],
                    node: 2,
                    material: 0,
                    lod_first: 0,
                    lod_count: 2,
                    vertex_first: 0,
                    vertex_count: 3,
                    flags: 0,
                },
                PartDesc {
                    bounds: [0.0, 0.0, 0.0, 1.0],
                    node: 1,
                    material: 6,
                    lod_first: 2,
                    lod_count: 1,
                    vertex_first: 0,
                    vertex_count: 3,
                    flags: PART_FLAG_GLASS,
                },
            ],
            lods: vec![
                LodDesc {
                    bounds: [0.0, 0.0, 0.0, 1.0],
                    error: 0.0,
                    part: 0,
                    meshlet_first: 0,
                    meshlet_count: 1,
                    triangle_count: 1,
                    level: 0,
                    index_first: 0,
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
                    index_first: 0,
                    index_count: 3,
                    flags: 0,
                },
                LodDesc {
                    bounds: [0.0, 0.0, 0.0, 1.0],
                    error: 0.0,
                    part: 1,
                    meshlet_first: 2,
                    meshlet_count: 1,
                    triangle_count: 1,
                    level: 0,
                    index_first: 3,
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
                MeshletDesc {
                    bounds: [0.0, 0.0, 0.0, 1.0],
                    cone_axis: [0.0, 0.0, 1.0],
                    cone_cutoff: 0.1,
                    vertex_offset: 6,
                    triangle_offset: 6,
                    vertex_count: 3,
                    triangle_count: 1,
                    part: 1,
                    level: 0,
                },
            ],
            meshlet_vertices: vec![0, 1, 2, 0, 1, 2, 0, 1, 2],
            meshlet_triangles: vec![0, 1, 2, 0, 1, 2, 0, 1, 2],
        };
        asset.validate().expect("asset validates");
        let encoded = asset.encode();
        let view = decode(&encoded).unwrap();
        assert_eq!(view, asset);
        assert!(view.has_features(
            FEATURE_RASTER | FEATURE_RT | FEATURE_MESH_LODS | FEATURE_MESHLETS
        ));
        assert_eq!(view.part_count(), 2);
        assert_eq!(view.lod_count(), 3);
        assert_eq!(view.meshlet_count(), 3);
        assert_eq!(view.raster_iter().collect::<Vec<_>>(), asset.raster);
    }

    #[test]
    fn round_trip_accepts_an_asset_without_hierarchy() {
        let asset = empty_airframe(2);
        let encoded = asset.encode();
        let view = decode(&encoded).unwrap();
        assert_eq!(view, asset);
        assert_eq!(view.features, 0);
    }

    #[test]
    fn rejects_trailing_data() {
        let clean = empty_airframe(1).encode();
        assert!(decode(&clean).is_ok());
        // A truncated view of the asset must fail cleanly.
        assert!(decode(&clean[..clean.len() - 1]).is_err());
        // A future version number is rejected.
        let mut future = clean.clone();
        future[8..12].copy_from_slice(&9u32.to_le_bytes());
        assert_eq!(
            decode(&future),
            Err(DecodeError::Invalid("unsupported version"))
        );
    }

    #[test]
    fn rejects_future_versions() {
        let mut encoded = empty_airframe(1).encode();
        encoded[8..12].copy_from_slice(&9u32.to_le_bytes());
        assert_eq!(
            decode(&encoded),
            Err(DecodeError::Invalid("unsupported version"))
        );
    }

    #[test]
    fn decodes_version_one_payloads() {
        let stream = vec![0u8; VERTEX_BYTES * 3];
        let opaque = [0u16, 1, 2];
        let glass = [1u16];
        let rt_idx = [0u16, 1, 2];
        let rt_nodes = [2u32];
        let rt_ranges = [(0u32, 3u32)];
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&MAGIC);
        push_u32(&mut bytes, VERSION_1);
        push_u32(&mut bytes, 48);
        push_u32(&mut bytes, VERTEX_BYTES as u32);
        push_u32(&mut bytes, stream.len() as u32);
        push_u32(&mut bytes, opaque.len() as u32);
        push_u32(&mut bytes, glass.len() as u32);
        push_u32(&mut bytes, rt_idx.len() as u32);
        push_u32(&mut bytes, rt_nodes.len() as u32);
        push_u32(&mut bytes, rt_ranges.len() as u32);
        push_u32(&mut bytes, 0);
        bytes.extend_from_slice(&stream);
        bytes.extend_from_slice(&u16_bytes(&opaque));
        bytes.extend_from_slice(&u16_bytes(&glass));
        bytes.extend_from_slice(&u16_bytes(&rt_idx));
        bytes.extend_from_slice(&u32_bytes(&rt_nodes));
        for &(offset, count) in &rt_ranges {
            push_u32(&mut bytes, offset);
            push_u32(&mut bytes, count);
        }
        let view = decode(&bytes).expect("v1 decodes");
        assert_eq!(view.vertex_count, 3);
        assert_eq!(view.opaque_count, 3);
        assert_eq!(view.glass_count, 1);
        assert_eq!(view.glass_first, 3);
        assert_eq!(view.raster_iter().collect::<Vec<_>>(), [0, 1, 2, 1]);
        assert_eq!(view.part_count(), 0);
        assert_eq!(view.rt_node_iter().collect::<Vec<_>>(), [2]);
        assert_eq!(view.rt_range_iter().collect::<Vec<_>>(), [(0, 3)]);
        assert!(view.has_features(FEATURE_RASTER | FEATURE_RT));
        assert!(!view.has_features(FEATURE_MESH_LODS));
    }

    #[test]
    fn rejects_local_indices_outside_a_meshlet() {
        let mut asset = empty_airframe(3);
        asset.raster = vec![0, 1, 2];
        asset.opaque_count = 3;
        asset.parts.push(PartDesc {
            bounds: [0.0; 4],
            node: 0,
            material: 0,
            lod_first: 0,
            lod_count: 1,
            vertex_first: 0,
            vertex_count: 3,
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
            index_first: 0,
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
    fn rejects_level_zero_ranges_that_cross_the_glass_split() {
        let mut asset = empty_airframe(3);
        asset.raster = vec![0, 1, 2, 1];
        asset.opaque_count = 3;
        asset.glass_count = 1;
        asset.parts.push(PartDesc {
            bounds: [0.0; 4],
            node: 0,
            material: 0,
            lod_first: 0,
            lod_count: 1,
            vertex_first: 0,
            vertex_count: 3,
            flags: 0,
        });
        asset.lods.push(LodDesc {
            bounds: [0.0; 4],
            error: 0.0,
            part: 0,
            meshlet_first: 0,
            meshlet_count: 0,
            triangle_count: 1,
            level: 0,
            index_first: 1,
            index_count: 3,
            flags: 0,
        });
        assert_eq!(
            asset.validate(),
            Err("LOD raster range crosses the opaque and glass halves")
        );
    }

    #[test]
    fn struct_sizes_match_the_packed_strides() {
        assert_eq!(std::mem::size_of::<PartDesc>(), PART_BYTES);
        assert_eq!(std::mem::size_of::<LodDesc>(), LOD_BYTES);
        assert_eq!(std::mem::size_of::<MeshletDesc>(), MESHLET_BYTES);
        assert_eq!(std::mem::size_of::<SectionDesc>(), SECTION_BYTES);
    }

    #[test]
    fn section_offsets_honor_declared_alignment() {
        let mut asset = empty_airframe(3);
        asset.raster = vec![0, 1, 2];
        asset.opaque_count = 3;
        let encoded = asset.encode();
        let view = decode(&encoded).unwrap();
        assert_eq!(view.vertex_stride as usize, VERTEX_BYTES);
        let header_bytes = u32::from_le_bytes(encoded[12..16].try_into().unwrap()) as usize;
        let section_count = u32::from_le_bytes(encoded[24..28].try_into().unwrap()) as usize;
        assert_eq!(section_count, SECTION_KIND_COUNT as usize);
        for index in 0..section_count {
            let base = HEADER_BYTES + index * SECTION_BYTES;
            let offset = u32::from_le_bytes(encoded[base + 4..base + 8].try_into().unwrap());
            let alignment = u32::from_le_bytes(encoded[base + 12..base + 16].try_into().unwrap());
            assert!(offset as usize >= header_bytes);
            assert_eq!(offset as usize % alignment as usize, 0);
        }
    }

    #[test]
    fn select_lod_picks_the_coarsest_level_under_budget() {
        let lods = [
            LodDesc {
                bounds: [0.0; 4],
                error: 0.0,
                part: 0,
                meshlet_first: 0,
                meshlet_count: 0,
                triangle_count: 1,
                level: 0,
                index_first: 0,
                index_count: 3,
                flags: 0,
            },
            LodDesc {
                bounds: [0.0; 4],
                error: 0.1,
                part: 0,
                meshlet_first: 0,
                meshlet_count: 0,
                triangle_count: 1,
                level: 1,
                index_first: 0,
                index_count: 3,
                flags: 0,
            },
            LodDesc {
                bounds: [0.0; 4],
                error: 0.5,
                part: 0,
                meshlet_first: 0,
                meshlet_count: 0,
                triangle_count: 1,
                level: 2,
                index_first: 0,
                index_count: 3,
                flags: 0,
            },
        ];
        let mut packed = Vec::new();
        for lod in &lods {
            push_f32x4(&mut packed, lod.bounds);
            push_f32(&mut packed, lod.error);
            push_u32(&mut packed, lod.part);
            push_u32(&mut packed, lod.meshlet_first);
            push_u32(&mut packed, lod.meshlet_count);
            push_u32(&mut packed, lod.triangle_count);
            push_u32(&mut packed, lod.level);
            push_u32(&mut packed, lod.index_first);
            push_u32(&mut packed, lod.index_count);
            push_u32(&mut packed, lod.flags);
        }
        assert_eq!(select_lod_range(&packed, 10.0, 5.0), 2);
        assert_eq!(select_lod_range(&packed, 10.0, 1.0), 1);
        assert_eq!(select_lod_range(&packed, 10.0, 0.01), 0);
        assert_eq!(select_lod_range(&[], 10.0, 1.0), 0);
    }

    #[test]
    fn screen_size_ratio_grows_with_proximity() {
        let near = screen_size_ratio(5.0, 50.0, 1.0);
        let far = screen_size_ratio(5.0, 500.0, 1.0);
        assert!(near > far);
        assert!(far > 0.0);
    }

    #[test]
    fn rejects_reserved_impostor_feature() {
        let mut encoded = empty_airframe(1).encode();
        let features_offset = 20;
        let features = u32::from_le_bytes(
            encoded[features_offset..features_offset + 4].try_into().unwrap(),
        );
        let patched = features | FEATURE_IMPOSTOR_LODS;
        encoded[features_offset..features_offset + 4].copy_from_slice(&patched.to_le_bytes());
        assert_eq!(
            decode(&encoded),
            Err(DecodeError::Invalid(
                "impostor LOD feature is reserved and not yet implemented"
            ))
        );
    }
}
