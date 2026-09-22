// Hybrid chase camera: horizon-stabilized quaternion tracking, airflow reference
// blending, axis-specific inertia, and an obstacle-aware, floating-origin boom.
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

/// Nominal boom distance behind the aircraft anchor (meters).
pub const BOOM_BACK: f32 = 14.5;
/// Nominal boom elevation above the aircraft anchor (meters).
pub const BOOM_UP: f32 = 3.6;
/// Target look-at distance forward along the sightline (meters).
pub const TARGET_DIST: f32 = 45.0;
/// Target look-at elevation offset (meters), positioning aircraft in the lower-middle viewport.
pub const TARGET_UP: f32 = 0.5;
/// Maximum sweep radius around the camera center (meters).
const CAMERA_COLLISION_RADIUS: f32 = 0.55;
/// Margin between the aircraft envelope and the near clipping plane (meters).
const AIRFRAME_NEAR_CLEARANCE: f32 = 0.25;
/// Conservative aircraft-local bounds, including node offsets and expected
/// flex/control motion, expanded by the camera sweep radius. The generated
/// airframe reaches x=±10.82, y=-0.53..2.25, z=-5.32..4.30 m. Keep synchronized
/// with airframe.rs, anim.rs, and plane.mesh.
const AIRFRAME_BOUNDS_CENTER: Vec3 = Vec3::new(0.0, 0.65, -0.45);
const AIRFRAME_BOUNDS_HALF_EXTENTS: Vec3 = Vec3::new(11.4, 3.1, 5.8);
/// Bounded spacing between boom collision samples (meters).
const BOOM_SWEEP_STEP: f32 = 1.0;
/// Bisection iterations after the first coarse boom obstruction is found.
const BOOM_SWEEP_REFINEMENT_STEPS: u32 = 5;
/// Small retreat from the last clear binary-search position (meters).
const BOOM_COLLISION_CLEARANCE: f32 = 0.05;
/// Hard bounds for inertial camera-local translation, in metres.
const CAMERA_INERTIA_LIMIT: Vec3 = Vec3::new(0.22, 0.18, 0.12);
/// Cutoff for smoothing sampled or differentiated acceleration before it drives inertia.
const CAMERA_ACCELERATION_CUTOFF_HZ: f32 = 6.0;

/// Fixed astronomical solar direction vector matching sky & atmospheric shaders.
const SUN_DIR: Vec3 = Vec3::new(
    0.5226423, // sin(0.55) * cos(0.38)
    0.37092,   // sin(0.38)
    0.767784,  // cos(0.55) * cos(0.38)
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
    /// Airflow contribution to the camera forward reference, in 0..0.72.
    pub airflow_blend: f32,
    /// Current local rotation-vector error (X=pitch, Y=yaw, Z=roll), in radians.
    pub angular_error: Vec3,
    /// Bounded camera-local translational offset (right, up, forward), in meters.
    pub position_offset: Vec3,
    /// Fraction of the available boom after near-plane and obstacle constraints.
    pub boom_fraction: f32,
    /// Effective axial setback after contraction; may extend beyond the nominal boom.
    pub boom_back_m: f32,
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

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn air_relative_velocity(pose: &Pose, wind_velocity: Vec3) -> Vec3 {
    let body_forward = (pose.orientation * Vec3::Z).normalize_or_zero();
    let speed = if pose.speed.is_finite() {
        pose.speed.clamp(0.0, 2_000.0)
    } else {
        0.0
    };
    let fallback = body_forward * speed;
    let wind_velocity = if wind_velocity.is_finite() {
        wind_velocity
    } else {
        Vec3::ZERO
    };
    let velocity = if pose.velocity.is_finite() {
        pose.velocity - wind_velocity
    } else {
        fallback
    };
    if velocity.is_finite() {
        velocity.clamp_length_max(2_000.0)
    } else {
        fallback
    }
}

/// Exact critically-damped spring step for a scalar state and a fixed target.
/// `velocity` is the derivative of `position` in the same coordinate frame.
fn critically_damped_step(
    position: f32,
    velocity: f32,
    target: f32,
    omega: f32,
    dt: f32,
) -> (f32, f32) {
    if dt <= 0.0 {
        return (position, velocity);
    }
    let error = position - target;
    let c = velocity + omega * error;
    let decay = (-omega * dt).exp();
    let next_error = (error + c * dt) * decay;
    let next_velocity = (velocity - omega * c * dt) * decay;
    (target + next_error, next_velocity)
}

fn critically_damped_vec3(
    position: Vec3,
    velocity: Vec3,
    target: Vec3,
    omega: Vec3,
    dt: f32,
) -> (Vec3, Vec3) {
    let (x, vx) = critically_damped_step(position.x, velocity.x, target.x, omega.x, dt);
    let (y, vy) = critically_damped_step(position.y, velocity.y, target.y, omega.y, dt);
    let (z, vz) = critically_damped_step(position.z, velocity.z, target.z, omega.z, dt);
    (Vec3::new(x, y, z), Vec3::new(vx, vy, vz))
}

fn low_pass_vec3(current: Vec3, target: Vec3, dt: f32) -> Vec3 {
    if dt <= 0.0 {
        return current;
    }
    let alpha = 1.0 - (-std::f32::consts::TAU * CAMERA_ACCELERATION_CUTOFF_HZ * dt).exp();
    current.lerp(target, alpha)
}

fn clamp_position_velocity(position: f32, velocity: f32, limit: f32) -> (f32, f32) {
    if position > limit {
        (limit, velocity.min(0.0))
    } else if position < -limit {
        (-limit, velocity.max(0.0))
    } else {
        (position, velocity)
    }
}

fn boom_position_obstructed<F>(
    anchor: Vec3,
    boom: Vec3,
    fraction: f32,
    collision_height_at: &mut F,
) -> bool
where
    F: FnMut(f64, f64) -> f32 + ?Sized,
{
    const RING: [(f32, f32); 9] = [
        (0.0, 0.0),
        (1.0, 0.0),
        (0.70710677, 0.70710677),
        (0.0, 1.0),
        (-0.70710677, 0.70710677),
        (-1.0, 0.0),
        (-0.70710677, -0.70710677),
        (0.0, -1.0),
        (0.70710677, -0.70710677),
    ];

    let center = anchor + boom * fraction;
    for (dx, dz) in RING {
        let x = center.x + dx * CAMERA_COLLISION_RADIUS;
        let z = center.z + dz * CAMERA_COLLISION_RADIUS;
        let floor = collision_height_at(x as f64, z as f64);
        if floor.is_finite() && center.y < floor + CAMERA_COLLISION_RADIUS {
            return true;
        }
    }
    false
}

fn nearest_airframe_view_depth(pose: &Pose, eye_from_anchor: Vec3, view_forward: Vec3) -> f32 {
    let view_forward = view_forward.normalize_or_zero();
    if view_forward == Vec3::ZERO {
        return f32::NEG_INFINITY;
    }
    let local_forward = (pose.orientation.conjugate() * view_forward).normalize_or_zero();
    let nearest_airframe_projection = AIRFRAME_BOUNDS_CENTER.dot(local_forward)
        - AIRFRAME_BOUNDS_HALF_EXTENTS.dot(local_forward.abs());
    nearest_airframe_projection - eye_from_anchor.dot(view_forward)
}

fn safe_boom_limits(
    pose: &Pose,
    cam_forward: Vec3,
    cam_up: Vec3,
    lookahead: Vec3,
    inertial_world: Vec3,
) -> (f32, f32) {
    let target_from_anchor =
        inertial_world + cam_forward * TARGET_DIST + cam_up * TARGET_UP + lookahead;
    let depth_at_fraction = |boom_back: f32, fraction: f32| {
        let boom = -cam_forward * boom_back + cam_up * BOOM_UP;
        let eye_from_anchor = inertial_world + boom * fraction;
        let view_forward = (target_from_anchor - eye_from_anchor).normalize_or_zero();
        nearest_airframe_view_depth(pose, eye_from_anchor, view_forward)
    };

    // Extend the aft leg only when even its uncontracted position would leave
    // the aircraft envelope inside the near plane. The target lead and raised
    // boom are included when measuring depth along the actual look direction.
    let minimum_depth = NEAR + AIRFRAME_NEAR_CLEARANCE;
    let mut too_short_back = BOOM_BACK;
    let mut clear_back = BOOM_BACK;
    let mut needs_extension = depth_at_fraction(BOOM_BACK, 1.0) < minimum_depth;
    for _ in 0..8 {
        if !needs_extension {
            break;
        }
        too_short_back = clear_back;
        clear_back *= 1.5;
        needs_extension = depth_at_fraction(clear_back, 1.0) < minimum_depth;
    }

    if needs_extension {
        return (clear_back, 1.0);
    }
    if clear_back > BOOM_BACK {
        for _ in 0..10 {
            let midpoint = (too_short_back + clear_back) * 0.5;
            if depth_at_fraction(midpoint, 1.0) >= minimum_depth {
                clear_back = midpoint;
            } else {
                too_short_back = midpoint;
            }
        }
    }
    let boom_back = clear_back;

    if depth_at_fraction(boom_back, 0.0) >= minimum_depth {
        return (boom_back, 0.0);
    }

    // Find the closest boom position whose projected aircraft support point
    // stays beyond the near plane. Ten iterations place the floor within a few
    // centimetres while keeping this per-frame calculation bounded.
    let mut too_close = 0.0;
    let mut clear = 1.0;
    for _ in 0..10 {
        let midpoint = (too_close + clear) * 0.5;
        if depth_at_fraction(boom_back, midpoint) >= minimum_depth {
            clear = midpoint;
        } else {
            too_close = midpoint;
        }
    }
    (boom_back, clear)
}

fn sweep_boom_fraction<F>(
    anchor: Vec3,
    boom: Vec3,
    minimum_fraction: f32,
    collision_height_at: &mut F,
) -> f32
where
    F: FnMut(f64, f64) -> f32 + ?Sized,
{
    let distance = boom.length();
    if distance <= 1e-4 {
        return 1.0;
    }
    let steps = (distance / BOOM_SWEEP_STEP).ceil().clamp(1.0, 32.0) as u32;
    let mut last_clear_fraction = 0.0;
    for step in 1..=steps {
        let sample_fraction = step as f32 / steps as f32;
        if boom_position_obstructed(anchor, boom, sample_fraction, collision_height_at) {
            let mut clear_fraction = last_clear_fraction;
            let mut blocked_fraction = sample_fraction;
            for _ in 0..BOOM_SWEEP_REFINEMENT_STEPS {
                let midpoint = (clear_fraction + blocked_fraction) * 0.5;
                if boom_position_obstructed(anchor, boom, midpoint, collision_height_at) {
                    blocked_fraction = midpoint;
                } else {
                    clear_fraction = midpoint;
                }
            }
            return (clear_fraction - BOOM_COLLISION_CLEARANCE / distance)
                .clamp(minimum_fraction, 1.0);
        }
        last_clear_fraction = sample_fraction;
    }
    1.0
}

/// Horizon-stabilized chase camera with air-relative framing and bounded inertia.
///
/// Features:
/// - Body-forward tracking at low speed; air-relative velocity contributes more when flow diverges at speed.
/// - Horizon-stabilized fractional roll with the existing vertical-flight body-orientation fallback.
/// - Independent critically-damped angular and camera-local translational response.
/// - A capped predictive lookahead, two-degree speed FOV range, and swept sphere boom contraction.
/// - Sightline-aligned structural rumble remains the final orientation effect layer.
#[derive(Clone, Copy, Debug)]
pub struct ChaseCamera {
    /// Smoothed camera orientation quaternion in world space.
    orientation: Quat,
    /// Camera-local angular velocity used by independent pitch/yaw/roll springs.
    angular_velocity: Vec3,
    /// Small camera-local inertial offset and velocity (right, up, forward).
    position_offset: Vec3,
    position_velocity: Vec3,
    /// Last air-relative velocity sample for translational acceleration response.
    previous_air_velocity: Vec3,
    /// Low-passed air-relative acceleration used by the inertial offset.
    filtered_air_acceleration: Vec3,
    has_air_velocity: bool,
    /// Current boom fraction. Obstructions contract immediately; clearance returns slowly.
    boom_fraction: f32,
    /// Critically-damped, speed-dependent vertical field of view.
    fov_y: f32,
    fov_velocity: f32,
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
            angular_velocity: Vec3::ZERO,
            position_offset: Vec3::ZERO,
            position_velocity: Vec3::ZERO,
            previous_air_velocity: Vec3::ZERO,
            filtered_air_acceleration: Vec3::ZERO,
            has_air_velocity: false,
            boom_fraction: 1.0,
            fov_y: BASE_FOV_Y,
            fov_velocity: 0.0,
            trauma: 0.0,
            shake_intensity: 0.0,
            exposure: 1.0,
            time: 0.0,
            initialized: false,
        }
    }

    pub fn orientation(&self) -> Quat {
        self.orientation
    }

    /// Compute the desired horizon-stabilized camera orientation for a given aircraft pose.
    /// Uses continuous quaternion sightline decomposition (yaw * pitch * fractional roll),
    /// eliminating vector cancellation singularities, 180-degree flip snaps, and gimbal lock.
    pub fn compute_target_orientation(pose: &Pose) -> Quat {
        Self::compute_target_orientation_with_air_velocity(pose, pose.orientation * Vec3::Z)
    }

    fn compute_target_orientation_with_air_velocity(pose: &Pose, air_velocity: Vec3) -> Quat {
        Self::compute_target_orientation_and_blend(pose, air_velocity).0
    }

    fn compute_target_orientation_and_blend(pose: &Pose, air_velocity: Vec3) -> (Quat, f32) {
        let body_forward = (pose.orientation * Vec3::Z).normalize_or_zero();
        let body_verticality = body_forward.y.abs();
        let air_speed = air_velocity.length();
        let air_forward = if air_velocity.is_finite() && air_speed > 2.0 {
            air_velocity / air_speed
        } else {
            body_forward
        };

        // Low-speed tracking follows the nose. Above 25 m/s, a larger share of
        // the reference follows air-relative travel direction when AoA or
        // sideslip separates it from body-forward. Near vertical attitudes
        // retain the aircraft reference to preserve loop and roll continuity.
        let speed_authority = smoothstep(10.0, 25.0, air_speed);
        let flow_divergence = body_forward.dot(air_forward).clamp(-1.0, 1.0).acos();
        let flow_authority = smoothstep(0.08, 0.65, flow_divergence) * 0.72;
        let vertical_yield = smoothstep(0.72, 0.96, body_verticality);
        let velocity_blend = speed_authority * flow_authority * (1.0 - vertical_yield);
        let forward = (body_forward * (1.0 - velocity_blend) + air_forward * velocity_blend)
            .try_normalize()
            .unwrap_or(body_forward);
        let verticality = body_verticality;

        // 1. Horizon-stabilized level reference (zero roll) pointing along `forward`:
        let heading = forward.x.atan2(forward.z);
        let pitch = forward.y.clamp(-1.0, 1.0).asin();
        let q_yaw = Quat::from_rotation_y(heading);
        let q_pitch = Quat::from_rotation_x(-pitch);
        let q_level = q_yaw * q_pitch;

        let body_heading = body_forward.x.atan2(body_forward.z);
        let body_pitch = body_forward.y.clamp(-1.0, 1.0).asin();
        let q_body_level =
            Quat::from_rotation_y(body_heading) * Quat::from_rotation_x(-body_pitch);

        // 2. Relative roll of the aircraft relative to level reference:
        // Quaternions have double cover (q and -q are identical rotations).
        // Align hemisphere before relative product so w > 0 and atan2 never jumps by 2*PI.
        let mut plane_q = pose.orientation;
        if q_body_level.dot(plane_q) < 0.0 {
            plane_q = -plane_q;
        }
        let q_rel = q_body_level.inverse() * plane_q;
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
            (
                target_quat.slerp(pose.orientation, blend_smooth).normalize(),
                velocity_blend,
            )
        } else {
            (target_quat, velocity_blend)
        }
    }

    /// Snap camera state to immediately match the given pose without interpolation lag.
    pub fn snap(&mut self, pose: &Pose) {
        self.orientation = Self::compute_target_orientation(pose);
        self.angular_velocity = Vec3::ZERO;
        self.position_offset = Vec3::ZERO;
        self.position_velocity = Vec3::ZERO;
        self.previous_air_velocity = pose.velocity;
        self.filtered_air_acceleration = Vec3::ZERO;
        self.has_air_velocity = false;
        self.boom_fraction = 1.0;
        self.fov_y = Self::target_fov(pose.speed);
        self.fov_velocity = 0.0;
        self.trauma = 0.0;
        self.shake_intensity = 0.0;
        self.exposure = 1.0;
        self.time = 0.0;
        self.initialized = true;
    }

    fn target_fov(speed: f32) -> f32 {
        let speed = if speed.is_finite() { speed.max(0.0) } else { 0.0 };
        BASE_FOV_Y + 2.0_f32.to_radians() * smoothstep(120.0, 850.0, speed)
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

    /// Wind-aware chase camera with airflow-based framing and aerodynamic buffet.
    pub fn step_with_wind(
        &mut self,
        pose: &Pose,
        controls: &Controls,
        dt: f32,
        aspect: f32,
        origin: Vec3,
        wind_velocity: Vec3,
    ) -> CameraFrame {
        self.step_internal(
            pose,
            controls,
            dt,
            aspect,
            origin,
            wind_velocity,
            None,
            None,
        )
    }

    /// Wind-aware chase camera with a bounded sphere sweep along its boom.
    /// The callback returns the conservative world-space collision height at X/Z.
    pub fn step_with_wind_and_collision<F>(
        &mut self,
        pose: &Pose,
        controls: &Controls,
        dt: f32,
        aspect: f32,
        origin: Vec3,
        wind_velocity: Vec3,
        mut collision_height_at: F,
    ) -> CameraFrame
    where
        F: FnMut(f64, f64) -> f32,
    {
        self.step_internal(
            pose,
            controls,
            dt,
            aspect,
            origin,
            wind_velocity,
            Some(&mut collision_height_at),
            None,
        )
    }

    /// Wind-aware camera using acceleration sampled by the fixed-step flight simulation.
    /// The acceleration is world-space and relative to the local air mass.
    pub fn step_with_wind_collision_and_acceleration<F>(
        &mut self,
        pose: &Pose,
        controls: &Controls,
        dt: f32,
        aspect: f32,
        origin: Vec3,
        wind_velocity: Vec3,
        air_acceleration: Vec3,
        mut collision_height_at: F,
    ) -> CameraFrame
    where
        F: FnMut(f64, f64) -> f32,
    {
        self.step_internal(
            pose,
            controls,
            dt,
            aspect,
            origin,
            wind_velocity,
            Some(&mut collision_height_at),
            Some(air_acceleration),
        )
    }

    fn step_internal(
        &mut self,
        pose: &Pose,
        _controls: &Controls,
        dt: f32,
        aspect: f32,
        origin: Vec3,
        wind_velocity: Vec3,
        mut collision_height_at: Option<&mut dyn FnMut(f64, f64) -> f32>,
        fixed_air_acceleration: Option<Vec3>,
    ) -> CameraFrame {
        let wind_velocity = if wind_velocity.is_finite() {
            wind_velocity
        } else {
            Vec3::ZERO
        };
        let air_velocity = air_relative_velocity(pose, wind_velocity);
        if !self.initialized {
            self.snap(pose);
            self.orientation = Self::compute_target_orientation_with_air_velocity(pose, air_velocity);
            self.previous_air_velocity = air_velocity;
            self.has_air_velocity = true;
        }
        // A paused frame still rebuilds projection for resize, but advances no
        // smoothing, exposure or rumble state.
        let dt = dt.clamp(0.0, 0.1);
        self.time += dt;

        let anchor = Vec3::new(pose.x, pose.y, pose.z);
        let (target_quat, airflow_blend) =
            Self::compute_target_orientation_and_blend(pose, air_velocity);

        // Critically damp independent camera-local pitch, yaw, and roll error
        // components, then reassemble one unit quaternion. The resulting axes
        // stay orthonormal without imposing one follow rate on every axis.
        let mut error_quat = self.orientation.conjugate() * target_quat;
        if error_quat.w < 0.0 {
            error_quat = -error_quat;
        }
        let (error_axis, error_angle) = error_quat.to_axis_angle();
        let angular_error = error_axis * error_angle;
        if dt > 0.0 {
            let error_velocity = -self.angular_velocity;
            let (remaining, next_error_velocity) = critically_damped_vec3(
                angular_error,
                error_velocity,
                Vec3::ZERO,
                Vec3::new(38.0, 28.0, 22.0),
                dt,
            );
            let turn = angular_error - remaining;
            let turn_angle = turn.length();
            if turn_angle > 1e-7 {
                self.orientation = (self.orientation
                    * Quat::from_axis_angle(turn / turn_angle, turn_angle))
                .normalize();
            }
            self.angular_velocity = -next_error_velocity;
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

        let differentiated_acceleration = if dt > 0.0 && self.has_air_velocity {
            ((air_velocity - self.previous_air_velocity) / dt).clamp_length_max(120.0)
        } else {
            Vec3::ZERO
        };
        if dt > 0.0 {
            self.previous_air_velocity = air_velocity;
            self.has_air_velocity = true;
            let acceleration_sample = fixed_air_acceleration
                .filter(|acceleration| acceleration.is_finite())
                .unwrap_or(differentiated_acceleration)
                .clamp_length_max(120.0);
            self.filtered_air_acceleration = low_pass_vec3(
                self.filtered_air_acceleration,
                acceleration_sample,
                dt,
            );
        }
        let air_acceleration = self.filtered_air_acceleration;

        // 3. Sightline-Aligned Airframe Structural Rumble:
        // In high-G flight and transonic buffet, vibration is applied as a subtle roll oscillation
        // along the camera sightline (Vec3::Z). Because the line of sight passes directly through
        // the aircraft, roll vibration tilts the distant horizon and clouds without displacing the
        // aircraft anchor or tail vertically or horizontally, completely eliminating tail jitter
        // while preserving the base chase framing during airframe buffeting.
        let roll_rumble = structural_rumble_octaves(self.time, 101) * self.shake_intensity * 0.008;
        let q_roll = Quat::from_axis_angle(Vec3::Z, roll_rumble);
        let shaken_orientation = (self.orientation * q_roll).normalize();

        // 4. Extract strictly orthonormal camera axes:
        let cam_forward = shaken_orientation * Vec3::Z;
        let cam_up = shaken_orientation * Vec3::Y;
        let cam_right = shaken_orientation * Vec3::X;

        // Small, axis-specific translational response to air-relative
        // acceleration. The bounded spring target never changes the nominal
        // boom by more than a few tenths of a metre.
        let local_acceleration = Vec3::new(
            air_acceleration.dot(cam_right),
            air_acceleration.dot(cam_up),
            air_acceleration.dot(cam_forward),
        );
        let inertia_target = Vec3::new(
            (-local_acceleration.x * 0.007).clamp(-0.22, 0.22),
            (-local_acceleration.y * 0.007).clamp(-0.18, 0.18),
            (-local_acceleration.z * 0.005).clamp(-0.12, 0.12),
        );
        (self.position_offset, self.position_velocity) = critically_damped_vec3(
            self.position_offset,
            self.position_velocity,
            inertia_target,
            Vec3::new(8.0, 6.5, 4.5),
            dt,
        );
        (self.position_offset.x, self.position_velocity.x) = clamp_position_velocity(
            self.position_offset.x,
            self.position_velocity.x,
            CAMERA_INERTIA_LIMIT.x,
        );
        (self.position_offset.y, self.position_velocity.y) = clamp_position_velocity(
            self.position_offset.y,
            self.position_velocity.y,
            CAMERA_INERTIA_LIMIT.y,
        );
        (self.position_offset.z, self.position_velocity.z) = clamp_position_velocity(
            self.position_offset.z,
            self.position_velocity.z,
            CAMERA_INERTIA_LIMIT.z,
        );
        let inertial_world = cam_right * self.position_offset.x
            + cam_up * self.position_offset.y
            + cam_forward * self.position_offset.z;

        // 5. Predictive framing follows a short, capped slice of air-relative
        // travel. Projective lookahead stays subtle and does not zoom the rig.
        let lookahead = (air_velocity * 0.035).clamp_length_max(3.0);
        let target = anchor
            + cam_forward * TARGET_DIST
            + cam_up * TARGET_UP
            + inertial_world
            + lookahead;

        // 6. Sweep a small camera sphere along the nominal boom. The first
        // obstruction contracts immediately; clearance restores distance slowly.
        let (boom_back, minimum_fraction) =
            safe_boom_limits(pose, cam_forward, cam_up, lookahead, inertial_world);
        let boom = -cam_forward * boom_back + cam_up * BOOM_UP;
        let safe_fraction = if let Some(collision_height_at) = collision_height_at.as_deref_mut() {
            sweep_boom_fraction(
                anchor + inertial_world,
                boom,
                minimum_fraction,
                collision_height_at,
            )
        } else {
            1.0
        };
        if safe_fraction < self.boom_fraction {
            self.boom_fraction = safe_fraction;
        } else if dt > 0.0 {
            self.boom_fraction +=
                (safe_fraction - self.boom_fraction) * (1.0 - (-3.0 * dt).exp());
        }
        self.boom_fraction = self.boom_fraction.clamp(minimum_fraction, 1.0);
        let eye = anchor + inertial_world + boom * self.boom_fraction;

        // 7. Keep the aircraft's framing stable with only a two-degree FOV
        // increase across the full speed range; smooth it independently.
        let target_fov = Self::target_fov(pose.speed);
        (self.fov_y, self.fov_velocity) = critically_damped_step(
            self.fov_y,
            self.fov_velocity,
            target_fov,
            4.0,
            dt,
        );
        let max_fov = BASE_FOV_Y + 2.0_f32.to_radians();
        if self.fov_y > max_fov {
            self.fov_y = max_fov;
            self.fov_velocity = self.fov_velocity.min(0.0);
        } else if self.fov_y < BASE_FOV_Y {
            self.fov_y = BASE_FOV_Y;
            self.fov_velocity = self.fov_velocity.max(0.0);
        }
        let fov_y = self.fov_y;

        // 8. Dynamic Photometric Auto-Exposure:
        let sun_dot = cam_forward.dot(SUN_DIR).clamp(-1.0, 1.0);
        let target_exposure = if sun_dot > 0.0 {
            1.0 - sun_dot.powf(1.8) * 0.28
        } else {
            1.0 + (-sun_dot).powf(1.2) * 0.08
        };
        self.exposure += (target_exposure - self.exposure) * (1.0 - (-4.0 * dt).exp());

        // 9. Floating-Origin View and Vulkan Projection:
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
            airflow_blend,
            angular_error,
            position_offset: self.position_offset,
            boom_fraction: self.boom_fraction,
            boom_back_m: boom_back * self.boom_fraction,
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
    let air_velocity = air_relative_velocity(pose, Vec3::ZERO);
    let target_quat =
        ChaseCamera::compute_target_orientation_with_air_velocity(pose, air_velocity);
    let cam_forward = target_quat * Vec3::Z;
    let camera_up = target_quat * Vec3::Y;
    let lookahead = (air_velocity * 0.035).clamp_length_max(3.0);
    let (boom_back, _) = safe_boom_limits(pose, cam_forward, camera_up, lookahead, Vec3::ZERO);

    let anchor = Vec3::new(pose.x, pose.y, pose.z) - origin;
    let eye = anchor - cam_forward * boom_back + camera_up * BOOM_UP;
    let target = anchor
        + cam_forward * TARGET_DIST
        + camera_up * TARGET_UP
        + lookahead;
    let view = Mat4::look_at_rh(eye, target, camera_up);
    let mut proj = Mat4::perspective_rh(ChaseCamera::target_fov(pose.speed), aspect, NEAR, FAR);
    // Positive-height Vulkan viewports map NDC -Y to the top of the image.
    proj.y_axis.y = -proj.y_axis.y;
    (proj * view, eye)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flight::SIM_STEP;
    use glam::Vec2;

    fn frame_airframe_view_depth(pose: &Pose, frame: &CameraFrame) -> f32 {
        let anchor = Vec3::new(pose.x, pose.y, pose.z);
        let view_forward = (frame.target_rel - frame.eye_rel).normalize_or_zero();
        nearest_airframe_view_depth(pose, frame.eye_world - anchor, view_forward)
    }

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
    fn high_speed_reference_blends_toward_airflow_without_losing_vertical_body_follow() {
        let mut pose = Pose::start();
        pose.orientation = Quat::IDENTITY;
        pose.speed = 100.0;
        let body_forward = pose.orientation * Vec3::Z;
        let airflow = Vec3::new(0.0, 38.0, 92.0).normalize() * pose.speed;
        let body_target = ChaseCamera::compute_target_orientation(&pose);
        let mut low_pose = pose;
        low_pose.speed = 5.0;
        let low_speed = ChaseCamera::compute_target_orientation_with_air_velocity(
            &low_pose,
            airflow.normalize() * 5.0,
        );
        let high_speed =
            ChaseCamera::compute_target_orientation_with_air_velocity(&pose, airflow);
        let target_forward = high_speed * Vec3::Z;

        assert!((low_speed.dot(body_target).abs() - 1.0).abs() < 1e-5);
        assert!(target_forward.dot(airflow.normalize()) > body_forward.dot(airflow.normalize()));
        assert!(target_forward.dot(body_forward) > 0.9);

        let mut vertical = pose;
        vertical.orientation = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
        let vertical_target = ChaseCamera::compute_target_orientation_with_air_velocity(
            &vertical,
            Vec3::Z * vertical.speed,
        );
        let vertical_body_target = ChaseCamera::compute_target_orientation(&vertical);
        assert!(vertical_target.dot(vertical_body_target).abs() > 0.9999);
    }

    #[test]
    fn translational_response_is_bounded_and_opposes_lateral_acceleration() {
        let mut pose = Pose::start();
        pose.orientation = Quat::IDENTITY;
        pose.speed = 100.0;
        pose.velocity = Vec3::Z * pose.speed;
        let mut camera = ChaseCamera::new();
        let controls = Controls::neutral();
        camera.step(&pose, &controls, SIM_STEP, 1.6, Vec3::ZERO);

        for _ in 0..100 {
            pose.velocity.x += 1.0;
            camera.step(&pose, &controls, SIM_STEP, 1.6, Vec3::ZERO);
        }

        assert!(camera.position_offset.x < -0.08);
        assert!(camera.position_offset.x >= -0.221);
        assert!(camera.position_offset.y.abs() <= 0.181);
        assert!(camera.position_offset.z.abs() <= 0.121);

        for _ in 0..100 {
            pose.velocity.x -= 1.0;
            camera.step(&pose, &controls, SIM_STEP, 1.6, Vec3::ZERO);
            assert!(camera.position_offset.x.abs() <= CAMERA_INERTIA_LIMIT.x);
            assert!(camera.position_offset.y.abs() <= CAMERA_INERTIA_LIMIT.y);
            assert!(camera.position_offset.z.abs() <= CAMERA_INERTIA_LIMIT.z);
        }
    }

    #[test]
    fn velocity_derivative_noise_is_low_passed_across_variable_render_steps() {
        let mut pose = Pose::start();
        pose.orientation = Quat::IDENTITY;
        pose.speed = 100.0;
        pose.velocity = Vec3::Z * pose.speed;
        let controls = Controls::neutral();
        let mut camera = ChaseCamera::new();
        camera.step(&pose, &controls, SIM_STEP, 1.6, Vec3::ZERO);

        let mut peak_filtered_acceleration = 0.0f32;
        for frame in 0..400 {
            let dt = if frame % 3 == 0 { 1.0 / 240.0 } else { 1.0 / 90.0 };
            pose.velocity.x += if frame % 2 == 0 { 0.08 } else { -0.08 };
            camera.step(&pose, &controls, dt, 1.6, Vec3::ZERO);
            peak_filtered_acceleration = peak_filtered_acceleration
                .max(camera.filtered_air_acceleration.length());
        }

        assert!(peak_filtered_acceleration < 20.0);
        assert!(camera.position_offset.is_finite());
        assert!(camera.position_offset.length() <= CAMERA_INERTIA_LIMIT.length());
    }

    #[test]
    fn fixed_step_acceleration_takes_priority_over_render_velocity_differences() {
        let mut pose = Pose::start();
        pose.orientation = Quat::IDENTITY;
        pose.speed = 100.0;
        pose.velocity = Vec3::Z * pose.speed;
        let controls = Controls::neutral();
        let clear = |_: f64, _: f64| 0.0;
        let mut camera = ChaseCamera::new();

        for frame in 0..144 {
            pose.velocity.x = if frame % 2 == 0 { 0.5 } else { 0.0 };
            camera.step_with_wind_collision_and_acceleration(
                &pose,
                &controls,
                SIM_STEP,
                1.6,
                Vec3::ZERO,
                Vec3::ZERO,
                Vec3::X * 8.0,
                clear,
            );
        }

        assert!((camera.filtered_air_acceleration.x - 8.0).abs() < 0.01);
        assert!(camera.position_offset.x < -0.02);
    }

    #[test]
    fn swept_camera_boom_contracts_at_a_ridge_and_returns_slowly() {
        let mut pose = Pose::start();
        pose.orientation = Quat::IDENTITY;
        pose.velocity = Vec3::Z * pose.speed;
        pose.x = 0.0;
        pose.z = 0.0;
        pose.y = 40.0;
        let controls = Controls::neutral();
        let mut camera = ChaseCamera::new();
        let obstructed = |_: f64, z: f64| {
            if (-8.0..=-6.0).contains(&z) { 42.0 } else { 0.0 }
        };
        let frame = camera.step_with_wind_and_collision(
            &pose,
            &controls,
            SIM_STEP,
            1.6,
            Vec3::ZERO,
            Vec3::ZERO,
            obstructed,
        );
        let anchor = Vec3::new(pose.x, pose.y, pose.z);
        let contracted = (frame.eye_world - anchor).length();
        assert!(contracted < (BOOM_BACK * BOOM_BACK + BOOM_UP * BOOM_UP).sqrt() - 3.0);

        let clear = |_: f64, _: f64| 0.0;
        let mut released = contracted;
        for _ in 0..12 {
            let frame = camera.step_with_wind_and_collision(
                &pose,
                &controls,
                SIM_STEP,
                1.6,
                Vec3::ZERO,
                Vec3::ZERO,
                clear,
            );
            released = (frame.eye_world - anchor).length();
        }
        assert!(released > contracted + 0.2);
        assert!(released < (BOOM_BACK * BOOM_BACK + BOOM_UP * BOOM_UP).sqrt());
    }

    #[test]
    fn swept_camera_boom_refines_contact_below_the_coarse_sample_spacing() {
        let mut pose = Pose::start();
        pose.orientation = Quat::IDENTITY;
        pose.speed = 100.0;
        pose.velocity = Vec3::Z * pose.speed;
        pose.x = 0.0;
        pose.z = 0.0;
        pose.y = 40.0;
        let obstacle_edge_z = -9.5f32;
        let collision_height = |_: f64, z: f64| {
            if z as f32 <= obstacle_edge_z { 50.0 } else { 0.0 }
        };
        let mut camera = ChaseCamera::new();
        let frame = camera.step_with_wind_and_collision(
            &pose,
            &Controls::neutral(),
            SIM_STEP,
            1.6,
            Vec3::ZERO,
            Vec3::ZERO,
            collision_height,
        );

        // The negative-Z ring point is the first to meet this vertical edge.
        let contact_center_z = obstacle_edge_z + CAMERA_COLLISION_RADIUS;
        let expected_contact_distance =
            (BOOM_BACK * BOOM_BACK + BOOM_UP * BOOM_UP).sqrt()
                * (-contact_center_z / BOOM_BACK);
        let actual_distance = (frame.eye_world - Vec3::new(pose.x, pose.y, pose.z)).length();
        assert!(actual_distance < expected_contact_distance);
        assert!(expected_contact_distance - actual_distance < 0.15);
    }

    #[test]
    fn near_plane_airframe_bounds_cover_current_generated_geometry() {
        let bounds_min = AIRFRAME_BOUNDS_CENTER - AIRFRAME_BOUNDS_HALF_EXTENTS;
        let bounds_max = AIRFRAME_BOUNDS_CENTER + AIRFRAME_BOUNDS_HALF_EXTENTS;
        assert!(bounds_min.x <= -10.82 && bounds_max.x >= 10.82);
        assert!(bounds_min.y <= -0.53 && bounds_max.y >= 2.25);
        assert!(bounds_min.z <= -5.32 && bounds_max.z >= 4.30);
    }

    #[test]
    fn stateless_view_proj_keeps_an_oblique_airframe_clear_of_the_near_plane() {
        let mut pose = Pose::start();
        let direction = Vec3::new(11.4, -3.75, 6.25).normalize();
        pose.orientation = Quat::from_rotation_arc(Vec3::Z, direction);
        pose.speed = 100.0;
        pose.velocity = direction * pose.speed;
        let (view_proj, eye) = view_proj(&pose, 1.6, Vec3::ZERO);

        let air_velocity = air_relative_velocity(&pose, Vec3::ZERO);
        let target_orientation =
            ChaseCamera::compute_target_orientation_with_air_velocity(&pose, air_velocity);
        let cam_forward = target_orientation * Vec3::Z;
        let cam_up = target_orientation * Vec3::Y;
        let lookahead = (air_velocity * 0.035).clamp_length_max(3.0);
        let anchor = Vec3::new(pose.x, pose.y, pose.z);
        let target = anchor + cam_forward * TARGET_DIST + cam_up * TARGET_UP + lookahead;
        let view_forward = (target - eye).normalize_or_zero();

        assert!(view_proj.is_finite());
        assert!(nearest_airframe_view_depth(&pose, eye - anchor, view_forward)
            >= NEAR + AIRFRAME_NEAR_CLEARANCE - 0.01);
    }

    #[test]
    fn swept_boom_keeps_the_airframe_bounds_beyond_the_near_plane() {
        let mut pose = Pose::start();
        pose.orientation = Quat::IDENTITY;
        pose.speed = 100.0;
        pose.velocity = Vec3::Z * pose.speed;
        pose.x = 0.0;
        pose.z = 0.0;
        pose.y = 40.0;
        let mut camera = ChaseCamera::new();
        let frame = camera.step_with_wind_and_collision(
            &pose,
            &Controls::neutral(),
            SIM_STEP,
            1.6,
            Vec3::ZERO,
            Vec3::ZERO,
            |_, _| 10_000.0,
        );

        assert!(frame.boom_fraction > 0.5);
        assert!(frame_airframe_view_depth(&pose, &frame) >= NEAR + AIRFRAME_NEAR_CLEARANCE - 0.01);
    }

    #[test]
    fn swept_boom_extends_when_oblique_airframe_bounds_exceed_nominal_depth() {
        let mut pose = Pose::start();
        let direction = Vec3::new(11.4, -3.75, 6.25).normalize();
        pose.orientation = Quat::IDENTITY;
        pose.speed = 100.0;
        pose.velocity = Vec3::Z * pose.speed;
        let mut camera = ChaseCamera::new();
        // Hold a deliberately oblique camera view fixed for this paused frame;
        // the aircraft remains level so the bounds support depth is predictable.
        camera.orientation = Quat::from_rotation_arc(Vec3::Z, direction);
        camera.initialized = true;
        assert!(
            safe_boom_limits(
                &pose,
                direction,
                camera.orientation * Vec3::Y,
                Vec3::ZERO,
                Vec3::ZERO,
            )
            .0 > BOOM_BACK
        );
        let frame = camera.step_with_wind_and_collision(
            &pose,
            &Controls::neutral(),
            0.0,
            1.6,
            Vec3::ZERO,
            Vec3::ZERO,
            |_, _| 10_000.0,
        );

        assert!(frame.boom_back_m > BOOM_BACK);
        assert!(frame.boom_back_m < BOOM_BACK * 1.3);
        assert!(frame_airframe_view_depth(&pose, &frame) >= NEAR + AIRFRAME_NEAR_CLEARANCE - 0.01);
    }

    #[test]
    fn compound_roll_pitch_reversal_sideslip_and_ridge_sequence_stays_continuous() {
        let mut pose = Pose::start();
        pose.orientation = Quat::IDENTITY;
        pose.speed = 130.0;
        pose.velocity = Vec3::Z * pose.speed;
        let controls = Controls::neutral();
        let mut camera = ChaseCamera::new();
        let mut previous_orientation: Option<Quat> = None;
        let mut saw_vertical = false;
        let mut saw_inverted = false;
        let mut saw_airflow_blend = false;
        let mut saw_obstruction = false;
        let duration = 14.0;
        let steps = (duration / SIM_STEP) as usize;

        for frame_index in 0..steps {
            let time = frame_index as f32 * SIM_STEP;
            let low_altitude_pass = time >= 12.0;
            let body_roll = if low_altitude_pass { 0.0 } else { 3.1 * time };
            let body_pitch = if low_altitude_pass {
                0.05 * (time * 2.0).sin()
            } else {
                1.52 * (std::f32::consts::TAU * 0.17 * time).sin()
            };
            let body_yaw = 0.45 * (std::f32::consts::TAU * 0.11 * time).sin();
            pose.orientation = (Quat::from_rotation_y(body_yaw)
                * Quat::from_rotation_x(-body_pitch)
                * Quat::from_rotation_z(-body_roll))
            .normalize();
            if low_altitude_pass {
                pose.y = 70.0;
            }

            let body_forward = pose.orientation * Vec3::Z;
            let body_right = pose.orientation * Vec3::X;
            let body_up = pose.orientation * Vec3::Y;
            let high_aoa = (9.0..11.0).contains(&time);
            let forward_speed = if high_aoa { 45.0 } else { pose.speed };
            let vertical_flow = if high_aoa { -body_up * 40.0 } else { Vec3::ZERO };
            let sideslip = body_right * (24.0 * (time * 1.3).sin());
            let previous_velocity = pose.velocity;
            pose.velocity = body_forward * forward_speed + vertical_flow + sideslip;
            let air_acceleration = ((pose.velocity - previous_velocity) / SIM_STEP)
                .clamp_length_max(120.0);
            pose.x += pose.velocity.x * SIM_STEP;
            pose.y += pose.velocity.y * SIM_STEP;
            pose.z += pose.velocity.z * SIM_STEP;

            let anchor_x = pose.x as f64;
            let anchor_y = pose.y;
            let anchor_z = pose.z as f64;
            let collision_height = |x: f64, z: f64| {
                let on_ridge = low_altitude_pass
                    && (x - anchor_x).abs() < 8.0
                    && (anchor_z - 11.0..=anchor_z - 5.0).contains(&z);
                if on_ridge { anchor_y + 8.0 } else { anchor_y - 50.0 }
            };
            let frame = camera.step_with_wind_collision_and_acceleration(
                &pose,
                &controls,
                SIM_STEP,
                16.0 / 9.0,
                Vec3::new(pose.x, pose.y, pose.z),
                Vec3::ZERO,
                air_acceleration,
                collision_height,
            );

            assert!(frame.view_proj.is_finite());
            assert!(frame.eye_world.is_finite());
            assert!(frame.airflow_blend.is_finite());
            assert!(frame.angular_error.is_finite());
            assert!(frame.position_offset.is_finite());
            assert!((0.0..=0.72).contains(&frame.airflow_blend));
            assert!(frame_airframe_view_depth(&pose, &frame)
                >= NEAR + AIRFRAME_NEAR_CLEARANCE - 0.01);
            assert!(frame.boom_fraction <= 1.0);
            assert!(frame.boom_back_m.is_finite());
            if let Some(previous) = previous_orientation {
                assert!(previous.angle_between(camera.orientation()) < 0.12);
            }
            previous_orientation = Some(camera.orientation());

            saw_vertical |= body_forward.y.abs() > 0.995;
            saw_inverted |= body_up.y < -0.8;
            saw_airflow_blend |= frame.airflow_blend > 0.05;
            saw_obstruction |= frame.boom_fraction < 0.99;
        }

        assert!(saw_vertical);
        assert!(saw_inverted);
        assert!(saw_airflow_blend);
        assert!(saw_obstruction);
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
            pose.velocity = pose.orientation * Vec3::Z * pose.speed;
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
            pose.velocity = pose.orientation * Vec3::Z * pose.speed;
            let (vp, eye) = view_proj(&pose, 1.6, origin);
            let area = envelope_quad_area(vp, eye, rel_ground);
            assert!(
                area > 1.0,
                "when pitching down (pitch={pitch}), ground envelope must not collapse (got area={area})"
            );
        }

        // 3. Steep dive (looking directly down at ground): must be fullscreen quad (area = 4.0).
        pose.orientation = glam::Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
        pose.velocity = pose.orientation * Vec3::Z * pose.speed;
        let (vp_dive, eye_dive) = view_proj(&pose, 1.6, origin);
        let area_dive = envelope_quad_area(vp_dive, eye_dive, rel_ground);
        assert_eq!(area_dive, 4.0, "steep dive looking at ground must be fullscreen quad");

        // 4. Steep climb (looking directly up at sky): must cull ground quad (area = 0.0).
        pose.orientation = glam::Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
        pose.velocity = pose.orientation * Vec3::Z * pose.speed;
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
    fn chase_camera_axis_springs_converge_smoothly_without_divergence() {
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
    fn nominal_boom_distance_stays_stable_without_obstructions() {
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
        let mut prev_fov = frame_cruise.fov_y;
        for _ in 0..100 {
            frame_supersonic = cam.step(&pose, &controls, 0.016, aspect, origin);
            assert!(frame_supersonic.fov_y >= prev_fov);
            assert!(frame_supersonic.fov_y - prev_fov < 0.03);
            prev_fov = frame_supersonic.fov_y;
        }
        let dist_supersonic = (frame_supersonic.eye_world - Vec3::new(pose.x, pose.y, pose.z)).length();

        // Distance must remain strictly invariant to floating point precision
        assert!((dist_supersonic - dist_cruise).abs() < 0.001);
        assert!(frame_supersonic.fov_y > BASE_FOV_Y);
        assert!(frame_supersonic.fov_y <= BASE_FOV_Y + 2.0_f32.to_radians());
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

    #[test]
    fn pitch_reversal_keeps_camera_lag_bounded() {
        let mut pose = Pose::start();
        let mut cam = ChaseCamera::new();
        let mut max_lag = 0.0f32;
        for step in 0..(8.0 / SIM_STEP) as usize {
            let controls = if (step / 144) % 2 == 0 {
                Controls { pitch: 1.0, ..Controls::neutral() }
            } else {
                Controls { pitch: -1.0, ..Controls::neutral() }
            };
            pose.step(&controls, SIM_STEP);
            cam.step(&pose, &controls, SIM_STEP, 1.6, Vec3::ZERO);
            let target = ChaseCamera::compute_target_orientation(&pose);
            let lag = 2.0 * cam.orientation.dot(target).abs().clamp(-1.0, 1.0).acos();
            max_lag = max_lag.max(lag);
        }
        assert!(max_lag.to_degrees() < 6.0, "pitch reversal camera lag: {} deg", max_lag.to_degrees());
    }

}
