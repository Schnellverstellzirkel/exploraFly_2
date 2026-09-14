// Chase camera follows the full aircraft attitude, with a gentle horizon bias.
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
    let forward = pose.orientation * Vec3::Z;
    let body_up = pose.orientation * Vec3::Y;
    // Keep a readable horizon in gentle banks, but follow loops and inverted flight.
    let horizon_weight = 0.8 * body_up.y.max(0.0) * (1.0 - forward.y.abs());
    let blended_up = body_up.lerp(Vec3::Y, horizon_weight);
    let camera_up = (blended_up - forward * blended_up.dot(forward)).normalize();
    let anchor = Vec3::new(pose.x, pose.y, pose.z) - origin;
    let eye = anchor - forward * back + camera_up * up;
    let target = anchor + forward * 40.0 - camera_up;
    let view = Mat4::look_at_rh(eye, target, camera_up);
    let mut proj = Mat4::perspective_rh(FOV_Y, aspect, NEAR, FAR);
    // Positive-height Vulkan viewports map NDC -Y to the top of the image.
    proj.y_axis.y = -proj.y_axis.y;
    (proj * view, eye)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_follows_pitch_and_survives_vertical_and_inverted_flight() {
        for pitch in [0.7, std::f32::consts::FRAC_PI_2, std::f32::consts::PI] {
            let mut pose = Pose::start();
            pose.orientation = glam::Quat::from_rotation_x(-pitch);
            let origin = Vec3::new(pose.x, pose.y, pose.z);
            let (vp, eye) = view_proj(&pose, 1.6, origin);
            let forward = pose.orientation * Vec3::Z;
            assert!(vp.is_finite());
            assert!((eye.dot(forward) + 14.0).abs() < 0.001);
            assert!(vp.project_point3(Vec3::ZERO).is_finite());
            if pitch == 0.7 {
                assert!(eye.y < 0.0);
            }
        }
    }

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
