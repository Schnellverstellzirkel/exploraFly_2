// Chase camera: 2nd-order critically damped spring dynamics, flight-path slipstream
// tracking, turn anticipation look-ahead, aerodynamic dynamic-pressure buffet shake,
// dynamic velocity FOV expansion, and solar exposure adaptation.
// Vulkan clip: Y down, depth zero to one.

use crate::flight::{Controls, Pose};
use glam::{Mat4, Quat, Vec3};

/// Base vertical field of view in radians (72 degrees).
/// Provides an expansive, immersive action-cam sightline (~104 degrees horizontal on 16:9).
pub const BASE_FOV_Y: f32 = 72.0_f32.to_radians();
/// Maximum vertical field of view under maximum Mach and afterburner boost (86 degrees, ~119 horizontal on 16:9).
pub const MAX_FOV_Y: f32 = 86.0_f32.to_radians();
/// Legacy alias for BASE_FOV_Y.
pub const FOV_Y: f32 = BASE_FOV_Y;
/// Near clipping plane distance in meters.
pub const NEAR: f32 = 2.0;
/// Far clipping plane distance in meters (30 km for long-range horizon).
pub const FAR: f32 = 30000.0;

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

/// State-of-the-art chase camera with physical mass, spring-damper inertia,
/// aerodynamic slipstream look-ahead, and dynamic optics.
#[derive(Clone, Copy, Debug)]
pub struct ChaseCamera {
    eye_world: Vec3,
    eye_vel: Vec3,
    target_world: Vec3,
    target_vel: Vec3,
    camera_up: Vec3,
    fov_y: f32,
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
            eye_world: Vec3::ZERO,
            eye_vel: Vec3::ZERO,
            target_world: Vec3::ZERO,
            target_vel: Vec3::ZERO,
            camera_up: Vec3::Y,
            fov_y: BASE_FOV_Y,
            exposure: 1.0,
            time: 0.0,
            initialized: false,
        }
    }

    /// Snap camera state to immediately match the given pose without interpolation lag.
    pub fn snap(&mut self, pose: &Pose) {
        let forward = pose.orientation * Vec3::Z;
        let body_up = pose.orientation * Vec3::Y;
        let horizon_weight = 0.75 * body_up.y.max(0.0) * (1.0 - forward.y.abs());
        let blended_up = body_up.lerp(Vec3::Y, horizon_weight);
        let cam_up = (blended_up - forward * blended_up.dot(forward)).normalize();
        let anchor = Vec3::new(pose.x, pose.y, pose.z);

        self.camera_up = cam_up;
        self.eye_world = anchor - forward * 14.5 + cam_up * 4.2;
        self.eye_vel = pose.velocity;
        self.target_world = anchor + forward * 42.0 - cam_up * 0.8;
        self.target_vel = pose.velocity;
        self.fov_y = BASE_FOV_Y;
        self.exposure = 1.0;
        self.initialized = true;
    }

    /// Advance camera simulation by `dt` seconds and calculate camera view-projection.
    pub fn step(
        &mut self,
        pose: &Pose,
        controls: &Controls,
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
        let right = pose.orientation * -Vec3::X;
        let anchor = Vec3::new(pose.x, pose.y, pose.z);

        // 1. Slipstream & velocity-vector blend:
        // High-speed flight aligns sightline slightly with the aircraft's actual velocity vector.
        let vel_dir = if speed > 5.0 {
            pose.velocity / speed
        } else {
            forward
        };
        let slip_weight = (speed / 300.0).clamp(0.0, 0.35);
        let flight_dir = forward.lerp(vel_dir, slip_weight).normalize();

        // 2. Dynamic horizon-biased camera up vector:
        let horizon_weight = 0.75 * body_up.y.max(0.0) * (1.0 - forward.y.abs());
        let blended_up = body_up.lerp(Vec3::Y, horizon_weight);
        let desired_up = (blended_up - flight_dir * blended_up.dot(flight_dir)).normalize();
        self.camera_up = self
            .camera_up
            .lerp(desired_up, 1.0 - (-14.0 * dt).exp())
            .normalize();

        // 3. Dynamic distance & G-load / acceleration lag:
        let speed_ratio = ((speed - 70.0) / 750.0).clamp(0.0, 1.0);
        let back_dist = 14.5 + speed_ratio * 2.5 + pose.boost * 1.0;
        let load_offset = (pose.load - 1.0).clamp(-2.0, 5.0);
        let up_dist = 4.2 - load_offset * 0.22; // High G pulls camera slightly down in cockpit seat

        // Inertial centrifugal displacement:
        let centripetal = right * pose.rates.y * 3.0 - body_up * (load_offset * 0.4);
        let ideal_eye = anchor - flight_dir * back_dist + self.camera_up * up_dist + centripetal;

        // 4. Turn anticipation & look-ahead:
        // Offset look target into turns based on angular rates and roll command.
        let lead_yaw = right * (-pose.rates.y * 14.0 + controls.bank * 3.5);
        let lead_pitch = body_up * (pose.rates.x * 9.0);
        let target_dist = 42.0 + speed_ratio * 16.0;
        let ideal_target = anchor + flight_dir * target_dist - self.camera_up * 0.8 + lead_yaw + lead_pitch;

        // 5. 2nd-order critically damped spring updates:
        // Position spring (omega_n = 16.0 rad/s) has weighted follow latency.
        // Target spring (omega_n = 22.0 rad/s) responds quickly to track direction changes.
        self.eye_world = critically_damped_spring(
            self.eye_world,
            &mut self.eye_vel,
            ideal_eye,
            16.0,
            dt,
        );
        self.target_world = critically_damped_spring(
            self.target_world,
            &mut self.target_vel,
            ideal_target,
            22.0,
            dt,
        );

        // 6. Aerodynamic dynamic-pressure buffet & high-G micro-shake:
        let sound_speed = 340.29; // Sea-level sound speed nominal
        let mach = speed / sound_speed;
        let dynamic_pressure = 0.5 * 1.225 * speed * speed;
        let q_norm = (dynamic_pressure / 45000.0).clamp(0.0, 1.0);
        let g_stress = load_offset.abs().clamp(0.0, 6.0) / 4.0;
        let transonic_flutter = (-((mach - 1.0) / 0.12).powi(2)).exp() * 0.6;
        let shake_intensity = (0.15 * q_norm + 0.55 * g_stress + 0.30 * transonic_flutter).clamp(0.0, 1.0);

        let t = self.time;
        // Harmonic vibration: 23.4 Hz airframe hum + 4.8 Hz aerodynamic buffet + 12.1 Hz engine acoustic
        let shake_pitch = ((t * 23.4).sin() * 0.55 + (t * 4.8).sin() * 0.45) * 0.0032 * shake_intensity;
        let shake_yaw = ((t * 19.1 + 1.2).sin() * 0.55 + (t * 5.2 + 0.8).sin() * 0.45) * 0.0030 * shake_intensity;
        let shake_roll = ((t * 27.3 + 2.4).sin()) * 0.0042 * shake_intensity;
        let shake_rot = Quat::from_euler(glam::EulerRot::YXZ, shake_yaw, shake_pitch, shake_roll);

        let shake_pos = Vec3::new(
            (t * 25.1).sin() * 0.024,
            (t * 20.3 + 1.1).sin() * 0.020,
            (t * 29.7 + 2.2).sin() * 0.016,
        ) * shake_intensity;

        let shaken_eye = self.eye_world + shake_pos;
        let shaken_target = self.target_world + shake_pos;
        let shaken_up = (shake_rot * self.camera_up).normalize();

        // 7. Dynamic speed FOV expansion & G-force visual tunneling:
        let target_fov = BASE_FOV_Y
            + (MAX_FOV_Y - BASE_FOV_Y) * (speed_ratio * 0.7 + pose.boost * 0.3)
            - (g_stress * 0.035).clamp(0.0, 0.04);
        self.fov_y += (target_fov - self.fov_y) * (1.0 - (-6.0 * dt).exp());

        // 8. Dynamic photometric auto-exposure:
        let look_dir = (shaken_target - shaken_eye).normalize();
        let sun_dot = look_dir.dot(SUN_DIR).clamp(-1.0, 1.0);
        let target_exposure = if sun_dot > 0.0 {
            1.0 - sun_dot.powf(1.8) * 0.28
        } else {
            1.0 + (-sun_dot).powf(1.2) * 0.08
        };
        self.exposure += (target_exposure - self.exposure) * (1.0 - (-4.0 * dt).exp());

        // 9. Floating-origin view and Vulkan projection matrix:
        let eye_rel = shaken_eye - origin;
        let target_rel = shaken_target - origin;
        let view = Mat4::look_at_rh(eye_rel, target_rel, shaken_up);
        let mut proj = Mat4::perspective_rh(self.fov_y, aspect, NEAR, FAR);
        // Vulkan viewport NDC: Y points down
        proj.y_axis.y = -proj.y_axis.y;

        CameraFrame {
            view_proj: proj * view,
            eye_rel,
            eye_world: shaken_eye,
            target_rel,
            camera_up: shaken_up,
            fov_y: self.fov_y,
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
    let back = 14.5;
    let up = 4.2;
    let forward = pose.orientation * Vec3::Z;
    let body_up = pose.orientation * Vec3::Y;
    // Keep a readable horizon in gentle banks, but follow loops and inverted flight.
    let horizon_weight = 0.75 * body_up.y.max(0.0) * (1.0 - forward.y.abs());
    let blended_up = body_up.lerp(Vec3::Y, horizon_weight);
    let camera_up = (blended_up - forward * blended_up.dot(forward)).normalize();
    let anchor = Vec3::new(pose.x, pose.y, pose.z) - origin;
    let eye = anchor - forward * back + camera_up * up;
    let target = anchor + forward * 42.0 - camera_up * 0.8;
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
            assert!((eye.dot(forward) + 14.5).abs() < 0.001);
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

        // Step through multiple iterations at 144 Hz
        for _ in 0..300 {
            pose.step(&controls, 1.0 / 144.0);
            let frame = cam.step(&pose, &controls, 1.0 / 144.0, aspect, origin);
            assert!(frame.view_proj.is_finite());
            assert!(frame.eye_rel.is_finite());
            assert!(frame.fov_y >= BASE_FOV_Y && frame.fov_y <= MAX_FOV_Y + 0.05);
            assert!(frame.exposure >= 0.5 && frame.exposure <= 1.5);
        }
    }

    #[test]
    fn dynamic_fov_expands_with_airspeed() {
        let mut cam = ChaseCamera::new();
        let mut pose = Pose::start();
        let controls = Controls::neutral();
        let aspect = 1.77;
        let origin = Vec3::ZERO;

        let frame_low = cam.step(&pose, &controls, 0.016, aspect, origin);

        // Accelerate to Mach 2.5
        pose.speed = 850.0;
        pose.velocity = Vec3::Z * 850.0;
        pose.boost = 1.0;

        let mut frame_high = frame_low;
        for _ in 0..100 {
            frame_high = cam.step(&pose, &controls, 0.016, aspect, origin);
        }

        assert!(frame_high.fov_y > frame_low.fov_y);
        assert!(frame_high.shake_intensity > frame_low.shake_intensity);
    }
}
