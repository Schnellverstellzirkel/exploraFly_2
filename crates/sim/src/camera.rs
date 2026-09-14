// War Thunder style chase camera: constant boom distance, horizon-stabilized
// quaternion attitude tracking, and smooth, jitter-free cinematic follow.
// Vulkan clip: Y down, depth zero to one.

use crate::flight::{Controls, Pose};
use glam::{Mat3, Mat4, Quat, Vec3};

/// Base vertical field of view in radians (72 degrees).
/// Provides an expansive, immersive sightline (~104 degrees horizontal on 16:9)
/// while keeping the aircraft solidly sized and framed.
pub const BASE_FOV_Y: f32 = 72.0_f32.to_radians();
/// Legacy alias for BASE_FOV_Y.
pub const FOV_Y: f32 = BASE_FOV_Y;
/// Near clipping plane distance in meters.
pub const NEAR: f32 = 2.0;
/// Far clipping plane distance in meters (30 km for long-range horizon).
pub const FAR: f32 = 30000.0;

/// War Thunder fixed boom distance behind the aircraft anchor (meters).
pub const BOOM_BACK: f32 = 14.5;
/// War Thunder fixed boom elevation above the aircraft anchor (meters).
pub const BOOM_UP: f32 = 3.6;
/// Target look-at distance forward along the sightline (meters).
pub const TARGET_DIST: f32 = 45.0;
/// Target look-at elevation offset (meters), positioning aircraft in the lower-middle viewport.
pub const TARGET_UP: f32 = 0.5;

/// Fixed astronomical solar direction vector matching sky & atmospheric shaders.
const SUN_DIR: Vec3 = Vec3::new(
    0.5226423, // sin(0.55) * cos(0.38)
    0.3709200, // sin(0.38)
    0.7677840, // cos(0.55) * cos(0.38)
);

/// Comprehensive camera state output per frame.
#[derive(Clone, Copy, Debug)]
pub struct CameraFrame {
    /// Combined view-projection matrix with Vulkan Y-inverted viewport projection.
    pub view_proj: Mat4,
    /// Eye position relative to floating origin (meters).
    pub eye_rel: Vec3,
    /// Absolute world-space eye position (meters).
    pub eye_world: Vec3,
    /// Look target relative to floating origin (meters).
    pub target_rel: Vec3,
    /// Camera up vector in world space.
    pub camera_up: Vec3,
    /// Current vertical FOV in radians.
    pub fov_y: f32,
    /// Aspect ratio (width / height).
    pub aspect: f32,
    /// Airspeed in m/s.
    pub speed: f32,
    /// Wing load factor in Gs.
    pub load: f32,
    /// Normalized airframe vibration intensity [0.0, 1.0].
    pub shake_intensity: f32,
    /// Photometric auto-exposure multiplier.
    pub exposure: f32,
    /// Current Mach number.
    pub mach: f32,
}

/// War Thunder style chase camera.
///
/// Features:
/// - Strictly invariant boom distance: the plane never shrinks or pulls away with speed/boost.
/// - Horizon-stabilized attitude: the horizon stays stable during turns, while the aircraft banks inside the screen.
/// - Smooth quaternion slerp: tracks pitch and heading with fluid, damped angular latency, completely free of jitter.
/// - Singularity-free loop tracking: passes through vertical climbs and inverted flight without gimbal flips.
#[derive(Clone, Copy, Debug)]
pub struct ChaseCamera {
    /// Smoothed camera orientation quaternion in world space.
    orientation: Quat,
    exposure: f32,
    time: f32,
    initialized: bool,
}

impl Default for ChaseCamera {
    fn default() -> Self {
        Self::new()
    }
}

impl ChaseCamera {
    /// Initialize camera in default unattached state.
    pub fn new() -> Self {
        Self {
            orientation: Quat::IDENTITY,
            exposure: 1.0,
            time: 0.0,
            initialized: false,
        }
    }

    /// Compute the desired horizon-stabilized camera orientation for a given aircraft pose.
    fn compute_target_orientation(pose: &Pose) -> Quat {
        let forward = pose.orientation * Vec3::Z;
        let body_up = pose.orientation * Vec3::Y;
        let world_up = Vec3::Y;

        // Decouple rapid aircraft roll from the camera: keep the horizon stable (82% horizon bias),
        // but smoothly yield to the aircraft's body orientation as pitch approaches purely vertical.
        let verticality = forward.y.abs();
        let horizon_blend = (1.0 - verticality * verticality).max(0.0) * 0.82;
        let perp_world_up = (world_up - forward * forward.dot(world_up)).normalize_or_zero();
        let perp_body_up = (body_up - forward * forward.dot(body_up)).normalize_or_zero();
        let target_up = perp_body_up.lerp(perp_world_up, horizon_blend).normalize();

        // Right vector: in native axes, -X is right.
        let target_right = forward.cross(target_up).normalize();
        let ortho_up = target_right.cross(forward).normalize();

        // Construct orthonormal basis matrix [-right, up, forward]
        Quat::from_mat3(&Mat3::from_cols(
            -target_right,
            ortho_up,
            forward,
        )).normalize()
    }

    /// Snap camera state to immediately match the given pose without interpolation lag.
    pub fn snap(&mut self, pose: &Pose) {
        self.orientation = Self::compute_target_orientation(pose);
        self.exposure = 1.0;
        self.time = 0.0;
        self.initialized = true;
    }

    /// Advance camera simulation by `dt` seconds and calculate camera view-projection.
    pub fn step(
        &mut self,
        pose: &Pose,
        _controls: &Controls,
        dt: f32,
        aspect: f32,
        origin: Vec3,
    ) -> CameraFrame {
        if !self.initialized {
            self.snap(pose);
        }
        let dt = dt.clamp(0.0001, 0.1);
        self.time += dt;

        let anchor = Vec3::new(pose.x, pose.y, pose.z);
        let target_quat = Self::compute_target_orientation(pose);

        // 1. Smooth, Damped Quaternion Slerp (War Thunder Chase Follow):
        // 10.5 s^-1 gives the exact War Thunder weighted, fluid, authoritative follow rate.
        // Operating purely on SO(3) quaternions guarantees zero cross-axis jitter, zero shear,
        // and zero geometric wobble during combined pitch and bank maneuvers.
        let slerp_factor = 1.0 - (-10.5 * dt).exp();
        self.orientation = self.orientation.slerp(target_quat, slerp_factor).normalize();

        // 2. Extract strictly orthonormal camera axes:
        let cam_forward = self.orientation * Vec3::Z;
        let cam_up = self.orientation * Vec3::Y;

        // 3. Strict Invariant Camera Boom Distance:
        // Eye distance to aircraft anchor is mathematically constant: sqrt(14.5^2 + 3.6^2) = 14.94m.
        // Aircraft stays rock-solid in screen coordinates, perfectly framed in lower-middle view.
        let eye = anchor - cam_forward * BOOM_BACK + cam_up * BOOM_UP;
        let target = anchor + cam_forward * TARGET_DIST + cam_up * TARGET_UP;

        // 4. Invariant Field of View:
        let fov_y = BASE_FOV_Y;

        // 5. Dynamic Photometric Auto-Exposure:
        let sun_dot = cam_forward.dot(SUN_DIR).clamp(-1.0, 1.0);
        let target_exposure = if sun_dot > 0.0 {
            1.0 - sun_dot.powf(1.8) * 0.28
        } else {
            1.0 + (-sun_dot).powf(1.2) * 0.08
        };
        self.exposure += (target_exposure - self.exposure) * (1.0 - (-4.0 * dt).exp());

        // 6. Floating-Origin View and Vulkan Projection:
        let eye_rel = eye - origin;
        let target_rel = target - origin;
        let view = Mat4::look_at_rh(eye_rel, target_rel, cam_up);
        let mut proj = Mat4::perspective_rh(fov_y, aspect, NEAR, FAR);
        // Positive-height Vulkan viewports map NDC -Y to the top of the image.
        proj.y_axis.y = -proj.y_axis.y;

        CameraFrame {
            view_proj: proj * view,
            eye_rel,
            eye_world: eye,
            target_rel,
            camera_up: cam_up,
            fov_y,
            aspect,
            speed: pose.speed,
            load: pose.load,
            shake_intensity: 0.0,
            exposure: self.exposure,
            mach: pose.speed / 340.29,
        }
    }
}

/// Compute the combined view-projection matrix and relative eye position
/// using floating-origin camera-relative coordinates.
///
/// Returns `(view_proj, eye_rel)` where `eye_rel` is the camera position relative to `origin`.
pub fn view_proj(pose: &Pose, aspect: f32, origin: Vec3) -> (Mat4, Vec3) {
    let forward = pose.orientation * Vec3::Z;
    let body_up = pose.orientation * Vec3::Y;
    let world_up = Vec3::Y;

    let verticality = forward.y.abs();
    let horizon_blend = (1.0 - verticality * verticality).max(0.0) * 0.82;
    let perp_world_up = (world_up - forward * forward.dot(world_up)).normalize_or_zero();
    let perp_body_up = (body_up - forward * forward.dot(body_up)).normalize_or_zero();
    let blended_up = perp_body_up.lerp(perp_world_up, horizon_blend).normalize();
    let camera_up = (blended_up - forward * blended_up.dot(forward)).normalize();

    let anchor = Vec3::new(pose.x, pose.y, pose.z) - origin;
    let eye = anchor - forward * BOOM_BACK + camera_up * BOOM_UP;
    let target = anchor + forward * TARGET_DIST + camera_up * TARGET_UP;
    let view = Mat4::look_at_rh(eye, target, camera_up);
    let mut proj = Mat4::perspective_rh(BASE_FOV_Y, aspect, NEAR, FAR);
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
            assert!((eye.dot(forward) + BOOM_BACK).abs() < 0.001);
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

    #[test]
    fn chase_camera_slerp_converges_smoothly_without_divergence() {
        let mut cam = ChaseCamera::new();
        let mut pose = Pose::start();
        let controls = Controls::neutral();
        let aspect = 16.0 / 9.0;
        let origin = Vec3::new(pose.x, pose.y, pose.z);

        for _ in 0..300 {
            pose.step(&controls, 1.0 / 144.0);
            let frame = cam.step(&pose, &controls, 1.0 / 144.0, aspect, origin);
            assert!(frame.view_proj.is_finite());
            assert!(frame.eye_rel.is_finite());
            assert_eq!(frame.fov_y, BASE_FOV_Y);
            assert!(frame.exposure >= 0.5 && frame.exposure <= 1.5);
            assert_eq!(frame.shake_intensity, 0.0);
        }
    }

    #[test]
    fn camera_distance_to_plane_stays_strictly_constant_at_any_speed_and_boost() {
        let mut cam = ChaseCamera::new();
        let mut pose = Pose::start();
        let controls = Controls::neutral();
        let aspect = 16.0 / 9.0;
        let origin = Vec3::ZERO;

        let frame_cruise = cam.step(&pose, &controls, 0.016, aspect, origin);
        let dist_cruise = (frame_cruise.eye_world - Vec3::new(pose.x, pose.y, pose.z)).length();

        // High speed (Mach 2.5) with burner boost
        pose.speed = 850.0;
        pose.velocity = Vec3::Z * 850.0;
        pose.boost = 1.0;

        let mut frame_supersonic = frame_cruise;
        for _ in 0..100 {
            frame_supersonic = cam.step(&pose, &controls, 0.016, aspect, origin);
        }
        let dist_supersonic = (frame_supersonic.eye_world - Vec3::new(pose.x, pose.y, pose.z)).length();

        // Distance must remain strictly invariant to floating point precision
        assert!((dist_supersonic - dist_cruise).abs() < 0.001);
        assert_eq!(frame_supersonic.fov_y, BASE_FOV_Y);
    }
}
