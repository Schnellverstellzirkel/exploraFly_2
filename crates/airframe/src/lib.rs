//! Build-time procedural source geometry for the aircraft.
//!
//! Runtime representation, packing, LODs and meshlets belong to
//! `airframe-baker`. This crate stays a small offline generator:
//! no SIMD, threading, or format-specific encoding here.

mod airframe;
mod util;

pub use airframe::build_airframe;
pub use util::{Importance, LodPolicy, MatId, Node, PartFlags, RawPart, RawVert};
