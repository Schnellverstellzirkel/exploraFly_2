// Chase camera. Fixed offset behind the nose.
// Vulkan clip: Y down, depth zero to one.

use crate::flight::Pose;
use glam::{Mat4, Vec3};

/// Vertical field of view in radians (60 degrees).
pub const FOV_Y: f32 = 60.0_f32.to_radians();
/// Near clipping plane distance in meters.
pub const NEAR: f32 = 2.0;
/// Far clipping plane distance in meters (30 km for long-range horizon).
pub const FAR: f32 = 30000.0;

/// Compute the combined view-projection matrix and relative eye position
/// using floating-origin camera-relative coordinates.
///
/// Returns `(view_proj, eye_rel)` where `eye_rel` is the camera position relative to `origin`.
pub fn view_proj(pose: &Pose, aspect: f32, origin: Vec3) -> (Mat4, Vec3) {
    let back = 14.0;
    let up = 4.0;
    let ch = pose.heading.cos();
    let sh = pose.heading.sin();
    // Camera-relative: eye and target minus a near origin keeps
    // every matrix entry small. World-scale coordinates cancel
    // catastrophically in float32 and shimmer whole frames.
    // The eye returns relative too: lighting anchors must ride the
    // same origin, never absolute world coordinates.
    let eye = Vec3::new(pose.x - sh * back, pose.y + up, pose.z - ch * back) - origin;
    let target = Vec3::new(pose.x + sh * 40.0, pose.y - 1.0, pose.z + ch * 40.0) - origin;
    let view = Mat4::look_at_rh(eye, target, Vec3::Y);
    let mut proj = Mat4::perspective_rh(FOV_Y, aspect, NEAR, FAR);
    // Positive-height Vulkan viewports map NDC -Y to the top of the image.
    proj.y_axis.y = -proj.y_axis.y;
    (proj * view, eye)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_up_projects_toward_top_of_vulkan_image() {
        let pose = Pose::start();
        let origin = Vec3::new(pose.x, pose.y, pose.z);
        let (vp, _) = view_proj(&pose, 1.6, origin);
        let center = vp.project_point3(Vec3::ZERO);
        let above = vp.project_point3(Vec3::Y);
        assert!(above.y < center.y);
        let p = Vec3::new(1.0, 2.0, 8.0);
        assert!((vp.inverse().project_point3(vp.project_point3(p)) - p).length() < 0.001);
    }
}
