//! Flight dynamics, atmospheric physics, 3D noise generation, and chase camera.
//! Decoupled from Vulkan and windowing to enable fast multi-core compilation
//! and zero-recompilation touches during engine iteration.

pub mod camera;
pub mod effects;
pub mod flight;
pub mod noise;
