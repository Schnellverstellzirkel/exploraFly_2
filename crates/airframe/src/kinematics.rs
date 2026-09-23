//! Shared aircraft kinematics and animation limits used by runtime animation
//! and the offline conservative camera-bound bake.

use glam::{Mat4, Vec3};

/// Inclusive physical range of the flex oscillator used by the vertex shader.
pub const BEND_LIMITS: [f32; 2] = [-0.4, 1.1];
/// Maximum gust-driven cubic wing-flex displacement at unit span and pressure.
pub const FLEX_GUST_AMPLITUDE: f32 = 0.022;
/// Inclusive range of each exhaust petal angle in radians.
pub const PETAL_LIMITS: [f32; 2] = [0.12, 0.42];
/// Maximum absolute tail control-surface angle in radians.
pub const ELEVATOR_LIMIT: f32 = 0.39;

/// Maximum absolute flap angle for spanwise flap index `0..3`.
#[inline]
pub fn flap_limit(index: usize) -> f32 {
    0.29 + index.min(2) as f32 * 0.04
}

/// Evaluate a point on the procedural wing planform.
pub fn wing_point(side: f32, t: f32, chord: f32) -> Vec3 {
    let x = 0.42 + 10.4 * t;
    let leading = -1.4 + 0.9 * t + 2.7 * t * t;
    let width = (2.35 - 1.65 * t) * (1.0 - t.powi(12) * 0.87);
    let y = 0.08
        + 0.22 * t
        + 0.65 * t.powi(5)
        + (chord * std::f32::consts::PI).sin() * 0.14 * (1.0 - t);
    Vec3::new(side * (x - 1.2), y, leading + width * chord)
}

/// Evaluate the hinge pivot point for trailing-edge flap `index` on one wing.
pub fn flap_pivot(side: f32, index: usize) -> Vec3 {
    let start = 0.425 + index as f32 * 0.155;
    let end = start + 0.15;
    wing_point(side, (start + end) / 2.0, 0.77)
}

/// Build the local matrix for one of the 23 packed animation nodes.
/// Animation values are supplied by the runtime; the baker calls this same
/// function at their declared limits when generating the camera envelope.
pub fn node_matrix(
    node: usize,
    flaps: &[f32; 6],
    elevators: &[f32; 2],
    rotor: f32,
    petals: &[f32; 10],
) -> Mat4 {
    match node {
        0 => Mat4::IDENTITY,
        1 => Mat4::from_translation(Vec3::new(0.0, 0.37, 1.25)),
        2 => Mat4::from_translation(Vec3::new(-1.2, 0.15, 0.0)),
        3 => Mat4::from_translation(Vec3::new(1.2, 0.15, 0.0)),
        4..=9 => {
            let id = node - 4;
            let side = if id < 3 { -1.0 } else { 1.0 };
            let pivot = flap_pivot(side, id % 3);
            let comp = Vec3::new(side * 1.2, 0.15, 0.0);
            let p = Vec3::new(pivot.x + comp.x, pivot.y + comp.y, -(pivot.z + comp.z));
            Mat4::from_translation(p) * Mat4::from_rotation_x(flaps[id])
        }
        10 => Mat4::from_translation(Vec3::new(0.0, 0.34, -2.63)) * Mat4::from_rotation_z(rotor),
        11..=20 => {
            let i = node - 11;
            let a = i as f32 / 10.0 * std::f32::consts::TAU;
            let hinge = Vec3::new(-a.sin() * 0.46, a.cos() * 0.46 + 0.34, -(1.3 + 1.35));
            Mat4::from_translation(hinge)
                * Mat4::from_rotation_z(a)
                * Mat4::from_rotation_x(petals[i])
        }
        21..=22 => {
            let side = if node == 21 { -1.0 } else { 1.0 };
            let p = Vec3::new(side * 0.65, 0.65 + 0.2, -(2.0 + 2.5));
            Mat4::from_translation(p)
                * Mat4::from_rotation_z(side * 0.5)
                * Mat4::from_rotation_x(elevators[node - 21])
        }
        _ => Mat4::IDENTITY,
    }
}
