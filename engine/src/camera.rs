// Chase camera. Fixed offset behind the nose.
// Vulkan clip: Y down, depth zero to one.

use crate::flight::Pose;
use glam::{Mat4, Vec3};

pub const FOV_Y: f32 = 60.0_f32.to_radians();
pub const NEAR: f32 = 2.0;
pub const FAR: f32 = 30000.0;

pub fn view_proj(pose: &Pose, aspect: f32) -> Mat4 {
    let back = 14.0;
    let up = 4.0;
    let ch = pose.heading.cos();
    let sh = pose.heading.sin();
    let eye = Vec3::new(pose.x - sh * back, pose.y + up, pose.z - ch * back);
    let target = Vec3::new(pose.x + sh * 40.0, pose.y - 1.0, pose.z + ch * 40.0);
    let view = Mat4::look_at_rh(eye, target, Vec3::Y);
    let proj = Mat4::perspective_rh(FOV_Y, aspect, NEAR, FAR);
    // No Y flip: glam's matrix already matches Vulkan clip here,
    // proven by screenshot. The flip rendered everything mirrored.
    proj * view
}
