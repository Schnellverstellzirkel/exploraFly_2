//! Runtime loader for the immutable airframe asset produced by the
//! `airframe-baker` build dependency. Geometry generation, normal accumulation,
//! index reordering, and RT range construction are intentionally absent from
//! the launch path.

use airframe_format::{decode, BakedAirframe};

pub(super) type AirframeMesh = BakedAirframe;

pub(super) fn airframe_mesh() -> AirframeMesh {
    let bytes = include_bytes!(concat!(env!("OUT_DIR"), "/airframe.bin"));
    decode(bytes).unwrap_or_else(|error| panic!("invalid baked airframe: {error}"))
}
