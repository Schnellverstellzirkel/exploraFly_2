// War Thunder style chase camera: constant boom distance, horizon-stabilized
// spherical tracking, smooth rotational spring lag, and aerodynamic buffet flutter.
// Vulkan clip: Y down, depth zero to one.

use crate::flight::{Controls, Pose};
use glam::{Mat4, Quat, Vec3};

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
    /// Normalized airframe aerodynamic buffet & vibration intensity [0.0, 1.0].
    pub shake_intensity: f32,
    /// Photometric auto-exposure multiplier.
    pub exposure: f32,
    /// Current Mach number.
    pub mach: f32,
}

/// Critically damped spring-mass-damper system (zeta = 1.0).
/// Exact analytical formulation guarantees unconditional stability without overshoot or ringing.
#[inline]
fn critically_damped_spring(
    current: Vec3,
    velocity: &mut Vec3,
    target: Vec3,
    omega_n: f32,
    dt: f32,
) -> Vec3 {
    let delta = current - target;
    let exp_term = (-omega_n * dt).exp();
    let c2 = *velocity + delta * omega_n;
    let new_pos = target + (delta + c2 * dt) * exp_term;
    *velocity = (*velocity - c2 * (omega_n * dt)) * exp_term;
    new_pos
}

/// War Thunder style chase camera.
///
/// Features:
/// - Strictly invariant boom distance to the aircraft: the plane never shrinks or pulls away with speed/boost.
/// - Horizon-stabilized attitude: the horizon stays stable during turns, while the aircraft banks inside the screen.
/// - Spherical boom rotation: smoothly lags behind aircraft turns with critically damped angular springs.
/// - Seamless vertical loop tracking: passes through vertical climbs and inverted flight without gimbal flipping.
#[derive(Clone, Copy, Debug)]
pub struct ChaseCamera {
    boom_forward: Vec3,
    boom_forward_vel: Vec3,
    boom_up: Vec3,
    boom_up_vel: Vec3,
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
            boom_forward: Vec3::Z,
            boom_forward_vel: Vec3::ZERO,
            boom_up: Vec3::Y,
            boom_up_vel: Vec3::ZERO,
            exposure: 1.0,
            time: 0.0,
            initialized: false,
        }
    }

    /// Snap camera state to immediately match the given pose without interpolation lag.
    pub fn snap(&mut self, pose: &Pose) {
        let forward = pose.orientation * Vec3::Z;
        let body_up = pose.orientation * Vec3::Y;
        let world_up = Vec3::Y;

        let verticality = forward.y.abs();
        let horizon_blend = (1.0 - verticality * verticality).max(0.0) * 0.80;
        let perp_world_up = (world_up - forward * forward.dot(world_up)).normalize_or_zero();
        let perp_body_up = (body_up - forward * forward.dot(body_up)).normalize_or_zero();
        let init_up = perp_body_up.lerp(perp_world_up, horizon_blend).normalize();

        self.boom_forward = forward;
        self.boom_forward_vel = Vec3::ZERO;
        self.boom_up = (init_up - forward * init_up.dot(forward)).normalize();
        self.boom_up_vel = Vec3::ZERO;
        self.exposure = 1.0;
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

        let speed = pose.speed.max(1.0);
        let forward = pose.orientation * Vec3::Z;
        let body_up = pose.orientation * Vec3::Y;
        let anchor = Vec3::new(pose.x, pose.y, pose.z);

        // 1. War Thunder Flight Path & Heading Tracking:
        // The boom tracks the aircraft flight vector with gentle nose alignment.
        let vel_dir = if speed > 10.0 {
            pose.velocity / speed
        } else {
            forward
        };
        let desired_forward = forward.lerp(vel_dir, 0.25).normalize();

        // 2. War Thunder Horizon-Stabilized Attitude:
        // The camera maintains a stable horizon during banks, allowing the aircraft to bank
        // visibly inside the screen, while smoothly following vertical loops and inverted flight.
        let world_up = Vec3::Y;
        let verticality = desired_forward.y.abs();
        let horizon_blend = (1.0 - verticality * verticality).max(0.0) * 0.80;
        let perp_world_up = (world_up - desired_forward * desired_forward.dot(world_up)).normalize_or_zero();
        let perp_body_up = (body_up - desired_forward * desired_forward.dot(body_up)).normalize_or_zero();
        let target_up = perp_body_up.lerp(perp_world_up, horizon_blend).normalize();
        let ortho_target_up = (target_up - desired_forward * target_up.dot(desired_forward)).normalize();

        // 3. Critically Damped Rotational Lag (War Thunder Boom Spring):
        // Follows heading and pitch with smooth, weighted latency.
        let new_forward = critically_damped_spring(
            self.boom_forward,
            &mut self.boom_forward_vel,
            desired_forward,
            13.0,
            dt,
        )
        .normalize();
        self.boom_forward = new_forward;

        let raw_up = critically_damped_spring(
            self.boom_up,
            &mut self.boom_up_vel,
            ortho_target_up,
            15.0,
            dt,
        );
        self.boom_up = (raw_up - self.boom_forward * raw_up.dot(self.boom_forward)).normalize();

        // 4. Strict Constant Camera-to-Plane Distance (War Thunder Invariant Boom):
        // Mathematically constrained to constant radius sqrt(BOOM_BACK^2 + BOOM_UP^2) around anchor.
        // The plane never pulls away or shrinks when speeding up or using burner boost.
        let base_eye = anchor - self.boom_forward * BOOM_BACK + self.boom_up * BOOM_UP;
        let base_target = anchor + self.boom_forward * TARGET_DIST + self.boom_up * TARGET_UP;

        // 5. Aerodynamic Dynamic-Pressure Buffet & High-G Micro-Flutter:
        let sound_speed = 340.29;
        let mach = speed / sound_speed;
        let dynamic_pressure = 0.5 * 1.225 * speed * speed;
        let q_norm = (dynamic_pressure / 45000.0).clamp(0.0, 1.0);
        let g_stress = (pose.load - 1.0).abs().clamp(0.0, 6.0) / 4.0;
        let transonic_flutter = (-((mach - 1.0) / 0.12).powi(2)).exp() * 0.6;
        let shake_intensity = (0.15 * q_norm + 0.55 * g_stress + 0.30 * transonic_flutter).clamp(0.0, 1.0);

        let t = self.time;
        // Subtle rotational micro-flutter:
        let shake_pitch = ((t * 23.4).sin() * 0.55 + (t * 4.8).sin() * 0.45) * 0.0022 * shake_intensity;
        let shake_yaw = ((t * 19.1 + 1.2).sin() * 0.55 + (t * 5.2 + 0.8).sin() * 0.45) * 0.0020 * shake_intensity;
        let shake_roll = ((t * 27.3 + 2.4).sin()) * 0.0028 * shake_intensity;
        let shake_rot = Quat::from_euler(glam::EulerRot::YXZ, shake_yaw, shake_pitch, shake_roll);

        // Sub-centimeter positional micro-jitter:
        let shake_pos = Vec3::new(
            (t * 25.1).sin() * 0.012,
            (t * 20.3 + 1.1).sin() * 0.010,
            (t * 29.7 + 2.2).sin() * 0.008,
        ) * shake_intensity;

        let shaken_eye = base_eye + shake_pos;
        let shaken_target = base_target + shake_pos;
        let shaken_up = (shake_rot * self.boom_up).normalize();

        // 6. Invariant Field of View (Constant Plane Screen Scale):
        let fov_y = BASE_FOV_Y;

        // 7. Dynamic Photometric Auto-Exposure:
        let look_dir = (shaken_target - shaken_eye).normalize();
        let sun_dot = look_dir.dot(SUN_DIR).clamp(-1.0, 1.0);
        let target_exposure = if sun_dot > 0.0 {
            1.0 - sun_dot.powf(1.8) * 0.28
        } else {
            1.0 + (-sun_dot).powf(1.2) * 0.08
        };
        self.exposure += (target_exposure - self.exposure) * (1.0 - (-4.0 * dt).exp());

        // 8. Floating-Origin View and Vulkan Projection:
        let eye_rel = shaken_eye - origin;
        let target_rel = shaken_target - origin;
        let view = Mat4::look_at_rh(eye_rel, target_rel, shaken_up);
        let mut proj = Mat4::perspective_rh(fov_y, aspect, NEAR, FAR);
        // Positive-height Vulkan viewports map NDC -Y to the top of the image.
        proj.y_axis.y = -proj.y_axis.y;

        CameraFrame {
            view_proj: proj * view,
            eye_rel,
            eye_world: shaken_eye,
            target_rel,
            camera_up: shaken_up,
            fov_y,
            aspect,
            speed,
            load: pose.load,
            shake_intensity,
            exposure: self.exposure,
            mach,
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
    let horizon_blend = (1.0 - verticality * verticality).max(0.0) * 0.80;
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
    fn chase_camera_spring_damper_converges_without_divergence() {
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

        // Distance must remain invariant (within sub-centimeter airframe micro-jitter tolerance)
        assert!((dist_supersonic - dist_cruise).abs() < 0.05);
        // FOV must stay constant (no zoom-out pulling plane away)
        assert_eq!(frame_supersonic.fov_y, BASE_FOV_Y);
    }
}
