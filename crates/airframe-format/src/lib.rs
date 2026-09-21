//! Versioned packed airframe asset format shared by the build-time baker and
//! the runtime Vulkan loader. The format is deliberately little-endian and
//! self-describing so generated geometry can be validated before GPU upload.

/// Eight-byte file signature.
pub const MAGIC: [u8; 8] = *b"EXAFRM01";
/// Packed asset format version.
pub const VERSION: u32 = 1;
/// Runtime vertex stride: position, oct normal, half UV, flex, node/material IDs.
pub const VERTEX_BYTES: usize = 28;
/// Number of animation-node slots in the shared UBO.
pub const NODE_COUNT: usize = 23;
const HEADER_BYTES: usize = 48;

/// Fully packed immutable airframe data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BakedAirframe {
    pub stream: Vec<u8>,
    pub opaque: Vec<u16>,
    pub glass: Vec<u16>,
    pub rt_idx: Vec<u16>,
    pub rt_geom_nodes: Vec<u32>,
    pub rt_node_ranges: Vec<(u32, u32)>,
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
    /// Validate all bounds that later Vulkan upload and RT build code relies on.
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
        let capacity = HEADER_BYTES
            .checked_add(self.stream.len())
            .and_then(|n| n.checked_add(index_bytes))
            .and_then(|n| n.checked_add(node_bytes))
            .and_then(|n| n.checked_add(range_bytes))
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
    };
    asset.validate().map_err(DecodeError::Invalid)?;
    Ok(asset)
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_preserves_all_sections() {
        let asset = BakedAirframe {
            stream: vec![0; VERTEX_BYTES * 2],
            opaque: vec![0, 1],
            glass: vec![1],
            rt_idx: vec![0, 1, 0],
            rt_geom_nodes: vec![2],
            rt_node_ranges: vec![(0, 3)],
        };
        let encoded = asset.encode();
        assert_eq!(decode(&encoded).unwrap(), asset);
    }

    #[test]
    fn rejects_trailing_data() {
        let asset = BakedAirframe {
            stream: vec![0; VERTEX_BYTES],
            opaque: vec![0],
            glass: Vec::new(),
            rt_idx: Vec::new(),
            rt_geom_nodes: Vec::new(),
            rt_node_ranges: Vec::new(),
        };
        let mut encoded = asset.encode();
        encoded.push(0);
        assert_eq!(
            decode(&encoded),
            Err(DecodeError::Invalid("trailing bytes"))
        );
    }
}
