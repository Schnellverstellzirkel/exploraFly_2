//! Build-time procedural source geometry for the aircraft.
//!
//! Runtime representation, packing, LODs and meshlets belong to
//! `airframe-baker`. This crate stays a small offline generator:
//! no SIMD, threading, or format-specific encoding here.

mod airframe;
mod kinematics;
mod util;

pub use airframe::build_airframe;
pub use kinematics::{
    flap_limit, flap_pivot, node_matrix, wing_point, BEND_LIMITS, ELEVATOR_LIMIT,
    FLEX_GUST_AMPLITUDE, PETAL_LIMITS,
};
pub use util::{Importance, LodPolicy, MatId, Node, PartFlags, RawPart, RawVert};
