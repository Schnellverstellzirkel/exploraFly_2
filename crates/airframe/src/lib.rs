//! Procedural glider airframe. Planform generator plus index
//! optimization plus packed-vertex helpers. Changes rarely, so it
//! lives outside the hot engine crate to keep iteration builds small.

mod airframe;
pub mod forsyth;
pub mod util;

pub use airframe::{build_airframe, MatId, Node};
pub use util::{f32_to_f16, oct_encode, Importance, RawPart, RawVert};
