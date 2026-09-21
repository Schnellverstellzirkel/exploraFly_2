//! Procedural glider airframe. Planform generator plus packed-vertex
//! helpers. Changes rarely, so it lives outside the hot engine crate to
//! keep iteration builds small. Index ordering and baking belong to
//! `airframe-baker`.

mod airframe;
pub mod util;

pub use airframe::{build_airframe, MatId, Node};
pub use util::{f32_to_f16, oct_encode, Importance, RawPart, RawVert};
