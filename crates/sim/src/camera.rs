// War Thunder style chase camera: constant boom distance, horizon-stabilized
// quaternion attitude tracking, and smooth, jitter-free cinematic follow.
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
    /// Normalized airframe vibration intensity [0.0, 1.0].
    pub shake_intensity: f32,
    /// Photometric auto-exposure multiplier.
    pub exposure: f32,
    /// Current Mach number.
    pub mach: f32,
}

/// 1D continuous C2 smooth gradient noise generator using Ken Perlin's quintic polynomial:
/// s(t) = 6t^5 - 15t^4 + 10t^3.
/// First and second derivatives are zero at cell boundaries, guaranteeing continuous acceleration
/// without infinite-jerk spikes or discrete square-wave steps.
fn quintic_noise_1d(t: f32, seed: u32) -> f32 {
    let t_floor = t.floor();
    let i = t_floor as i32;
    let f = t - t_floor;

    let grad = |idx: i32| -> f32 {
        let mut h = (idx as u32).wrapping_mul(0x45d9f3b).wrapping_add(seed);
        h = ((h >> 16) ^ h).wrapping_mul(0x45d9f3b);
        h = ((h >> 16) ^ h).wrapping_mul(0x45d9f3b);
        h = (h >> 16) ^ h;
        ((h & 0xFFFF) as f32 / 32767.5) - 1.0
    };

    let g0 = grad(i);
    let g1 = grad(i + 1);
    let d0 = g0 * f;
    let d1 = g1 * (f - 1.0);

    let s = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    d0 + s * (d1 - d0)
}

/// Multi-octave continuous airframe structural buffet vibration.
/// Combines fundamental structural mode (3.6 Hz, primary airframe buffet),
/// harmonic empennage mode (7.2 Hz, fin buffeting), and low-frequency turbulence (1.6 Hz).
/// All frequencies are well below the Nyquist limit at 60-144 Hz, preventing aliasing and strobing.
fn structural_rumble_octaves(time: f32, seed: u32) -> f32 {
    let f0 = 3.6;
    let oct0 = quintic_noise_1d(time * f0, seed);
    let oct1 = quintic_noise_1d(time * f0 * 2.0, seed ^ 0x9e3779b9) * 0.45;
    let oct2 = quintic_noise_1d(time * 1.6, seed ^ 0x51f33a61) * 0.30;
    (oct0 + oct1 + oct2) * 1.2
}

/// War Thunder style chase camera.
///
/// Features:
/// - Strictly invariant boom distance: the plane never shrinks or pulls away with speed/boost.
/// - Horizon-stabilized attitude: the horizon stays stable during turns, while the aircraft banks inside the screen.
/// - Smooth quaternion slerp: tracks pitch and heading with fluid, damped angular latency, completely free of jitter.
/// - SOTA low-frequency C2 airframe structural rumble: Squirrel Eiserloh trauma model driven by aerodynamic G-load,
///   transonic buffet, and AoA stall separation, pivoted around the aircraft anchor so the aircraft tail remains rock-solid.
/// - Singularity-free loop tracking: passes through vertical climbs and inverted flight without gimbal flips.
#[derive(Clone, Copy, Debug)]
pub struct ChaseCamera {
    /// Smoothed camera orientation quaternion in world space.
    orientation: Quat,
    /// Persistent trauma level in [0.0, 1.0] representing airframe structural excitation.
    trauma: f32,
    /// Smoothed shake intensity in [0.0, 1.0] (trauma^2).
    shake_intensity: f32,
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
            trauma: 0.0,
            shake_intensity: 0.0,
            exposure: 1.0,
            time: 0.0,
            initialized: false,
        }
    }

    /// Compute the desired horizon-stabilized camera orientation for a given aircraft pose.
    /// Uses continuous quaternion sightline decomposition (yaw * pitch * fractional roll),
    /// eliminating vector cancellation singularities, 180-degree flip snaps, and gimbal lock.
    pub fn compute_target_orientation(pose: &Pose) -> Quat {
        let forward = pose.orientation * Vec3::Z;
        let verticality = forward.y.abs();

        // 1. Horizon-stabilized level reference (zero roll) pointing along `forward`:
        let heading = forward.x.atan2(forward.z);
        let pitch = forward.y.clamp(-1.0, 1.0).asin();
        let q_yaw = Quat::from_rotation_y(heading);
        let q_pitch = Quat::from_rotation_x(-pitch);
        let q_level = q_yaw * q_pitch;

        // 2. Relative roll of the aircraft relative to level reference:
        // Quaternions have double cover (q and -q are identical rotations).
        // Align hemisphere before relative product so w > 0 and atan2 never jumps by 2*PI.
        let mut plane_q = pose.orientation;
        if q_level.dot(plane_q) < 0.0 {
            plane_q = -plane_q;
        }
        let q_rel = q_level.inverse() * plane_q;
        let roll_angle = 2.0 * q_rel.z.atan2(q_rel.w);

        // 3. Horizon stabilization with continuous, singularity-free roll follow:
        // In shallow flight and gentle turns, keep horizon mostly stable (~28% roll follow),
        // but smoothly increase bank authority as roll steepens, so the camera smoothly
        // tracks loops and inverted flight without gimbal snaps or 180-degree flips.
        let bank_factor = 0.28 + 0.72 * (1.0 - roll_angle.cos().max(0.0));
        let cam_roll = roll_angle * bank_factor;
        let target_quat = (q_level * Quat::from_axis_angle(Vec3::Z, cam_roll)).normalize();

        // 4. Smoothly yield to aircraft body orientation in extreme vertical climbs/dives:
        if verticality > 0.85 {
            let blend = ((verticality - 0.85) / 0.14).clamp(0.0, 1.0);
            let blend_smooth = blend * blend * (3.0 - 2.0 * blend);
            target_quat.slerp(pose.orientation, blend_smooth).normalize()
        } else {
            target_quat
        }
    }

    /// Snap camera state to immediately match the given pose without interpolation lag.
    pub fn snap(&mut self, pose: &Pose) {
        self.orientation = Self::compute_target_orientation(pose);
        self.trauma = 0.0;
        self.shake_intensity = 0.0;
        self.exposure = 1.0;
        self.time = 0.0;
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
        self.step_with_wind(pose, controls, dt, aspect, origin, Vec3::ZERO)
    }

    /// Wind-aware chase camera: buffet responds to air-relative angle of attack.
    pub fn step_with_wind(
        &mut self,
        pose: &Pose,
        _controls: &Controls,
        dt: f32,
        aspect: f32,
        origin: Vec3,
        wind_velocity: Vec3,
    ) -> CameraFrame {
        if !self.initialized {
            self.snap(pose);
        }
        // A paused frame still rebuilds projection for resize, but advances no
        // smoothing, exposure or rumble state.
        let dt = dt.clamp(0.0, 0.1);
        self.time += dt;

        let anchor = Vec3::new(pose.x, pose.y, pose.z);
        let target_quat = Self::compute_target_orientation(pose);

        // 1. Smooth, Damped Quaternion Slerp (War Thunder Chase Follow):
        // 10.5 s^-1 gives the exact War Thunder weighted, fluid, authoritative follow rate.
        // Operating purely on SO(3) quaternions guarantees zero cross-axis jitter, zero shear,
        // and zero geometric wobble during combined pitch and bank maneuvers.
        let slerp_factor = 1.0 - (-10.5 * dt).exp();
        if dt > 0.0 {
            self.orientation = self.orientation.slerp(target_quat, slerp_factor).normalize();
        }

        // 2. Aerodynamic Airframe Trauma Calculation (Squirrel Eiserloh Model):
        // Trauma is driven by real aerodynamic stress: G-load factor, transonic shock buffet,
        // high angle-of-attack stall flow separation, dynamic pressure, and engine spool.
        let g_delta = (pose.load - 1.0).abs();
        let g_trauma = ((g_delta - 0.5) / 4.0).clamp(0.0, 0.70);

        let sound_speed = (1.4 * 287.05 * crate::effects::isa_temperature(pose.y)).sqrt();
        let mach = pose.speed / sound_speed;
        let transonic_trauma = if (0.85..1.22).contains(&mach) {
            let m = (mach - 0.98) / 0.12;
            (-m * m).exp() * 0.60
        } else {
            0.0
        };

        let plane_forward = pose.orientation * Vec3::Z;
        let plane_up = pose.orientation * Vec3::Y;
        let air_velocity = pose.velocity - if wind_velocity.is_finite() {
            wind_velocity
        } else {
            Vec3::ZERO
        };
        let alpha = (-air_velocity.dot(plane_up)).atan2(air_velocity.dot(plane_forward));
        let aoa_trauma = ((alpha.abs() - 0.20) / 0.16).clamp(0.0, 0.65);

        let speed_trauma = ((pose.speed - 350.0) / 550.0).clamp(0.0, 0.35);
        let boost_trauma = pose.boost * 0.25;

        let target_trauma = g_trauma
            .max(transonic_trauma)
            .max(aoa_trauma)
            .max(speed_trauma)
            .max(boost_trauma);

        // Fast attack (8.0 s^-1), smooth gradual decay (2.5 s^-1):
        if target_trauma > self.trauma {
            self.trauma += (target_trauma - self.trauma) * (1.0 - (-8.0 * dt).exp());
        } else {
            self.trauma += (target_trauma - self.trauma) * (1.0 - (-2.5 * dt).exp());
        }
        self.trauma = self.trauma.clamp(0.0, 1.0);
        self.shake_intensity = self.trauma * self.trauma;

        // 3. Sightline-Aligned Airframe Structural Rumble:
        // In high-G flight and transonic buffet, vibration is applied as a subtle roll oscillation
        // along the camera sightline (Vec3::Z). Because the line of sight passes directly through
        // the aircraft, roll vibration tilts the distant horizon and clouds without displacing the
        // aircraft anchor or tail vertically or horizontally, completely eliminating tail jitter
        // while preserving visceral airframe buffeting (matching War Thunder chase mechanics).
        let roll_rumble = structural_rumble_octaves(self.time, 101) * self.shake_intensity * 0.008;
        let q_roll = Quat::from_axis_angle(Vec3::Z, roll_rumble);
        let shaken_orientation = (self.orientation * q_roll).normalize();

        // 4. Extract strictly orthonormal camera axes:
        let cam_forward = shaken_orientation * Vec3::Z;
        let cam_up = shaken_orientation * Vec3::Y;

        // 5. Strict Invariant Camera Boom Distance:
        // Eye distance to aircraft anchor is mathematically constant: sqrt(14.5^2 + 3.6^2) = 14.94m.
        // Aircraft stays rock-solid in screen coordinates, perfectly framed in lower-middle view.
        let eye = anchor - cam_forward * BOOM_BACK + cam_up * BOOM_UP;
        let target = anchor + cam_forward * TARGET_DIST + cam_up * TARGET_UP;

        // 6. Invariant Field of View:
        let fov_y = BASE_FOV_Y;

        // 7. Dynamic Photometric Auto-Exposure:
        let sun_dot = cam_forward.dot(SUN_DIR).clamp(-1.0, 1.0);
        let target_exposure = if sun_dot > 0.0 {
            1.0 - sun_dot.powf(1.8) * 0.28
        } else {
            1.0 + (-sun_dot).powf(1.2) * 0.08
        };
        self.exposure += (target_exposure - self.exposure) * (1.0 - (-4.0 * dt).exp());

        // 8. Floating-Origin View and Vulkan Projection:
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
            shake_intensity: self.shake_intensity,
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
    let target_quat = ChaseCamera::compute_target_orientation(pose);
    let cam_forward = target_quat * Vec3::Z;
    let camera_up = target_quat * Vec3::Y;

    let anchor = Vec3::new(pose.x, pose.y, pose.z) - origin;
    let eye = anchor - cam_forward * BOOM_BACK + camera_up * BOOM_UP;
    let target = anchor + cam_forward * TARGET_DIST + camera_up * TARGET_UP;
    let view = Mat4::look_at_rh(eye, target, camera_up);
    let mut proj = Mat4::perspective_rh(BASE_FOV_Y, aspect, NEAR, FAR);
    // Positive-height Vulkan viewports map NDC -Y to the top of the image.
    proj.y_axis.y = -proj.y_axis.y;
    (proj * view, eye)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flight::SIM_STEP;
    use glam::Vec2;

    #[test]
    fn zero_timestep_holds_active_camera_motion_while_allowing_resize() {
        let controls = Controls::neutral();
        let mut pose = Pose::start();
        let origin = Vec3::new(pose.x, pose.y, pose.z);
        let mut camera = ChaseCamera::new();
        camera.step(&pose, &controls, SIM_STEP, 1.6, origin);
        pose.orientation = Quat::from_rotation_y(0.9) * Quat::from_rotation_x(-0.4);
        pose.load = 6.0;
        pose.speed = 340.0;
        let moving = camera.step(&pose, &controls, SIM_STEP, 1.6, origin);
        assert!(moving.shake_intensity > 0.0);

        for _ in 0..240 {
            let paused = camera.step(&pose, &controls, 0.0, 1.6, origin);
            assert_eq!(paused.view_proj, moving.view_proj);
            assert_eq!(paused.eye_world, moving.eye_world);
            assert_eq!(paused.shake_intensity, moving.shake_intensity);
            assert_eq!(paused.exposure, moving.exposure);
        }
        let resized = camera.step(&pose, &controls, 0.0, 2.0, origin);
        assert_eq!(resized.eye_world, moving.eye_world);
        assert_ne!(resized.view_proj, moving.view_proj);
        assert!(resized.view_proj.is_finite());

        let resumed = camera.step(&pose, &controls, SIM_STEP, 1.6, origin);
        assert_ne!(resumed.view_proj, moving.view_proj);
        assert!(resumed.shake_intensity > moving.shake_intensity);
    }

    #[test]
    fn wind_drift_does_not_create_false_angle_of_attack_buffet() {
        let mut calm_pose = Pose::start();
        calm_pose.orientation = Quat::from_rotation_x(-0.32);
        let wind = Vec3::new(15.0, 30.0, -10.0);
        let mut drifting_pose = calm_pose;
        drifting_pose.velocity += wind;
        let mut calm_camera = ChaseCamera::new();
        let mut drifting_camera = ChaseCamera::new();
        for _ in 0..120 {
            let calm = calm_camera.step(&calm_pose, &Controls::neutral(), SIM_STEP, 1.6, Vec3::ZERO);
            let drifting = drifting_camera.step_with_wind(
                &drifting_pose,
                &Controls::neutral(),
                SIM_STEP,
                1.6,
                Vec3::ZERO,
                wind,
            );
            assert_eq!(drifting.shake_intensity, calm.shake_intensity);
            assert_eq!(drifting.mach, calm.mach);
            assert_eq!(drifting.view_proj, calm.view_proj);
        }
        assert!(calm_camera.shake_intensity > 0.1, "AoA buffet should be active");
    }

    #[test]
    fn mach_number_tracks_altitude_dependent_speed_of_sound() {
        let mut pose = Pose::start();
        pose.speed = 300.0;
        pose.y = 0.0;
        let sea_level = ChaseCamera::new().step(
            &pose, &Controls::neutral(), SIM_STEP, 1.6, Vec3::ZERO,
        );
        pose.y = 11000.0;
        let altitude = ChaseCamera::new().step(
            &pose, &Controls::neutral(), SIM_STEP, 1.6, Vec3::ZERO,
        );
        assert!(sea_level.mach < 0.9);
        assert!(altitude.mach > 1.0);
        assert!(altitude.mach > sea_level.mach * 1.1);
    }

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
    fn ground_envelope_quad_survives_pitch_down_and_all_attitudes() {
        let envelope_quad_area = |vp: Mat4, eye: Vec3, ground_y: f32| -> f32 {
            let inv_vp = vp.inverse();
            let ray_at = |ndc: Vec2| -> Vec3 {
                inv_vp.project_point3(Vec3::new(ndc.x, ndc.y, 1.0)) - eye
            };
            let ground_sign = (ground_y - eye.y).signum();
            let is_ground = |ndc: Vec2| -> bool {
                ray_at(ndc).y * ground_sign > 0.0
            };
            let horizon_at = |x: f32| -> f32 {
                let ray_top = ray_at(Vec2::new(x, -1.0));
                let ray_bottom = ray_at(Vec2::new(x, 1.0));
                let delta = ray_bottom.y - ray_top.y;
                if delta.abs() <= 1e-5 {
                    0.0
                } else {
                    let t = (-ray_top.y / delta).clamp(0.0, 1.0);
                    -1.0 + 2.0 * t
                }
            };

            let tl_ground = is_ground(Vec2::new(-1.0, -1.0));
            let tr_ground = is_ground(Vec2::new(1.0, -1.0));
            let bl_ground = is_ground(Vec2::new(-1.0, 1.0));
            let br_ground = is_ground(Vec2::new(1.0, 1.0));

            let mut p = [Vec2::ZERO; 6];
            if !tl_ground && !tr_ground && !bl_ground && !br_ground {
                for v in &mut p {
                    *v = Vec2::new(-1.0, -1.0);
                }
            } else if !tl_ground && !tr_ground && bl_ground && br_ground {
                let left_horizon = (horizon_at(-1.0) - 0.01).max(-1.0);
                let right_horizon = (horizon_at(1.0) - 0.01).max(-1.0);
                p[0] = Vec2::new(-1.0, left_horizon);
                p[1] = Vec2::new(1.0, right_horizon);
                p[2] = Vec2::new(-1.0, 1.0);
                p[3] = Vec2::new(-1.0, 1.0);
                p[4] = Vec2::new(1.0, right_horizon);
                p[5] = Vec2::new(1.0, 1.0);
            } else if tl_ground && tr_ground && !bl_ground && !br_ground {
                let left_horizon = (horizon_at(-1.0) + 0.01).min(1.0);
                let right_horizon = (horizon_at(1.0) + 0.01).min(1.0);
                p[0] = Vec2::new(-1.0, -1.0);
                p[1] = Vec2::new(1.0, -1.0);
                p[2] = Vec2::new(-1.0, left_horizon);
                p[3] = Vec2::new(-1.0, left_horizon);
                p[4] = Vec2::new(1.0, -1.0);
                p[5] = Vec2::new(1.0, right_horizon);
            } else {
                p[0] = Vec2::new(-1.0, -1.0);
                p[1] = Vec2::new(1.0, -1.0);
                p[2] = Vec2::new(-1.0, 1.0);
                p[3] = Vec2::new(-1.0, 1.0);
                p[4] = Vec2::new(1.0, -1.0);
                p[5] = Vec2::new(1.0, 1.0);
            }

            let tri_area = |a: Vec2, b: Vec2, c: Vec2| -> f32 {
                0.5 * ((b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y)).abs()
            };
            tri_area(p[0], p[1], p[2]) + tri_area(p[3], p[4], p[5])
        };

        // 1. Level cruise: ground covers roughly the bottom half of the screen.
        let mut pose = Pose::start();
        pose.y = 500.0;
        let origin = Vec3::new(pose.x, pose.y, pose.z);
        let rel_ground = -pose.y;
        let (vp_level, eye_level) = view_proj(&pose, 1.6, origin);
        let area_level = envelope_quad_area(vp_level, eye_level, rel_ground);
        assert!(area_level > 1.0 && area_level <= 4.0, "level flight must cover ground region (got {area_level})");

        // 2. Pitching down at various angles: MUST NOT COLLAPSE (must stay > 0 and reach 4.0).
        for pitch in [0.2, 0.4, 0.6, 0.8, 1.2, std::f32::consts::FRAC_PI_2] {
            pose.orientation = glam::Quat::from_rotation_x(pitch);
            let (vp, eye) = view_proj(&pose, 1.6, origin);
            let area = envelope_quad_area(vp, eye, rel_ground);
            assert!(
                area > 1.0,
                "when pitching down (pitch={pitch}), ground envelope must not collapse (got area={area})"
            );
        }

        // 3. Steep dive (looking directly down at ground): must be fullscreen quad (area = 4.0).
        pose.orientation = glam::Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
        let (vp_dive, eye_dive) = view_proj(&pose, 1.6, origin);
        let area_dive = envelope_quad_area(vp_dive, eye_dive, rel_ground);
        assert_eq!(area_dive, 4.0, "steep dive looking at ground must be fullscreen quad");

        // 4. Steep climb (looking directly up at sky): must cull ground quad (area = 0.0).
        pose.orientation = glam::Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
        let (vp_sky, eye_sky) = view_proj(&pose, 1.6, origin);
        let area_sky = envelope_quad_area(vp_sky, eye_sky, rel_ground);
        assert_eq!(area_sky, 0.0, "steep climb looking at sky must cull ground quad");
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

    #[test]
    fn trauma_rises_under_high_g_and_transonic_buffet_and_decays_smoothly() {
        let mut cam = ChaseCamera::new();
        let mut pose = Pose::start();
        let controls = Controls::neutral();
        let aspect = 16.0 / 9.0;
        let origin = Vec3::ZERO;

        // Baseline calm cruise
        let frame0 = cam.step(&pose, &controls, 0.016, aspect, origin);
        assert_eq!(frame0.shake_intensity, 0.0);

        // Apply high G pull (5.5 Gs)
        pose.load = 5.5;
        let mut max_shake = 0.0f32;
        for _ in 0..60 {
            let f = cam.step(&pose, &controls, 0.016, aspect, origin);
            if f.shake_intensity > max_shake {
                max_shake = f.shake_intensity;
            }
        }
        assert!(max_shake > 0.35, "high G load must excite airframe buffet shake");

        // Release G pull: airframe structural shake must decay smoothly without discontinuous step
        pose.load = 1.0;
        let mut prev_shake = max_shake;
        let mut negative_steps = 0;
        for _ in 0..120 {
            let f = cam.step(&pose, &controls, 0.016, aspect, origin);
            if f.shake_intensity < prev_shake {
                negative_steps += 1;
            }
            // Must never jump abruptly by more than 10% in a single 16ms frame
            assert!(
                (f.shake_intensity - prev_shake).abs() < 0.10,
                "decay step was discontinuous: {} to {}",
                prev_shake,
                f.shake_intensity
            );
            prev_shake = f.shake_intensity;
        }
        assert!(negative_steps > 80, "shake must decay monotonically towards calm");
        assert!(prev_shake < 0.05, "shake must settle back near zero after relaxation");
    }

    #[test]
    fn camera_to_anchor_distance_is_invariant_under_active_rumble() {
        let mut cam = ChaseCamera::new();
        let mut pose = Pose::start();
        let controls = Controls::neutral();
        let aspect = 16.0 / 9.0;
        let origin = Vec3::ZERO;

        // Force maximum structural trauma buffet
        pose.load = 9.0;
        pose.speed = 340.0; // Mach 1.0 transonic buffet peak

        let baseline_dist = (BOOM_BACK * BOOM_BACK + BOOM_UP * BOOM_UP).sqrt();

        for step in 0..200 {
            let dt = 0.007; // ~144 Hz
            let frame = cam.step(&pose, &controls, dt, aspect, origin);
            if step > 60 {
                assert!(frame.shake_intensity > 0.30, "shake should be active, got {}", frame.shake_intensity);
                let dist = (frame.eye_world - Vec3::new(pose.x, pose.y, pose.z)).length();
                assert!(
                    (dist - baseline_dist).abs() < 0.001,
                    "boom distance varied under rumble! Expected {}, got {}",
                    baseline_dist,
                    dist
                );
            }
        }
    }

    #[test]
    fn quintic_noise_has_continuous_first_and_second_derivatives() {
        // Sample quintic noise at high resolution and verify no step discontinuities in value or acceleration
        let seed = 42;
        let dt = 0.001f32;
        let mut prev_v = quintic_noise_1d(0.0, seed);
        let mut prev_vel = (quintic_noise_1d(dt, seed) - prev_v) / dt;

        for step in 1..2000 {
            let t = step as f32 * dt;
            let v = quintic_noise_1d(t, seed);
            let vel = (v - prev_v) / dt;
            let accel = (vel - prev_vel) / dt;

            // Value must be bounded
            assert!(v.abs() <= 1.0, "value out of bounds: {}", v);
            // Velocity must be bounded and continuous
            assert!(vel.abs() < 10.0, "excessive velocity spike: {}", vel);
            // Acceleration (2nd derivative) must never exhibit infinite jerk / square-wave step
            assert!(accel.abs() < 250.0, "excessive acceleration spike: {}", accel);

            prev_v = v;
            prev_vel = vel;
        }
    }

    #[test]
    fn sub_step_interpolated_pose_eliminates_tail_motion_jitter() {
        let mut cam = ChaseCamera::new();
        let mut pose = Pose::start();
        let controls = Controls {
            pitch: 1.0,
            bank: 0.0,
            yaw: 0.0,
            boost: false,
        };
        let aspect = 16.0 / 9.0;
        let tail_local = Vec3::new(0.65, 0.85, -4.5);

        let dt = 1.0 / 60.0; // 60 Hz render loop
        let mut accumulator = 0.0f32;
        let mut prev_ndc_y = 0.0f32;

        let mut prev_pose = pose;

        for frame_idx in 0..30 {
            accumulator += dt;
            let mut steps = 0;
            while accumulator >= SIM_STEP && steps < 5 {
                prev_pose = pose;
                pose.step(&controls, SIM_STEP);
                accumulator -= SIM_STEP;
                steps += 1;
            }

            let alpha = (accumulator / SIM_STEP).clamp(0.0, 1.0);
            let render_pose = prev_pose.interpolate(&pose, alpha);

            let origin = Vec3::new(render_pose.x, render_pose.y, render_pose.z);
            let frame = cam.step(&render_pose, &controls, dt, aspect, origin);

            let tail_world = render_pose.orientation * tail_local + origin;
            let tail_clip = frame.view_proj * tail_world.extend(1.0);
            let ndc_y = tail_clip.y / tail_clip.w;
            let delta = ndc_y - prev_ndc_y;

            if frame_idx > 2 {
                // Motion must remain strictly continuous without jitter sign-reversals
                assert!(delta > 0.0, "tail motion reversed unexpectedly: {}", delta);
            }
            prev_ndc_y = ndc_y;
        }
    }

    #[test]
    fn test_maneuver_smoothness_and_continuity() {
        let mut cam = ChaseCamera::new();
        let mut pose = Pose::start();
        let controls_bank = Controls {
            pitch: 0.2,
            bank: 1.0,
            yaw: 0.1,
            boost: true,
        };
        let aspect = 16.0 / 9.0;
        let dt = 1.0 / 144.0;
        let mut prev_eye_rel_anchor = Vec3::ZERO;
        let mut max_rel_jump = 0.0f32;

        for step in 0..1000 {
            pose.step(&controls_bank, SIM_STEP);
            let origin = Vec3::new(pose.x, pose.y, pose.z);
            let frame = cam.step(&pose, &controls_bank, dt, aspect, origin);
            assert!(frame.view_proj.is_finite());
            let eye_rel_anchor = frame.eye_world - Vec3::new(pose.x, pose.y, pose.z);
            if step > 10 {
                let jump = (eye_rel_anchor - prev_eye_rel_anchor).length();
                if jump > max_rel_jump {
                    max_rel_jump = jump;
                }
            }
            prev_eye_rel_anchor = eye_rel_anchor;
        }
        // Sub-step relative motion must remain strictly continuous under high-rate rolls and loops (< 0.20m per frame)
        assert!(max_rel_jump < 0.20, "excessive camera relative jump: {}", max_rel_jump);
    }
}

