//! Forgiving aerodynamic flight, ported from webxploraFly's Flight/Handling/Aerodynamics.
//! Native axes: +Y up, +Z nose, -X pilot right. Attitude is body-to-world.
use glam::{Quat, Vec3};

pub const CRUISE_SPEED: f32 = 70.0;
pub const TOP_SPEED: f32 = 1030.0;
pub const SIM_STEP: f32 = 1.0 / 144.0;
const MASS: f32 = 36000.0;
const WING_AREA: f32 = 61.6;
const GRAVITY: f32 = 9.80665;

#[derive(Clone, Copy)]
pub struct Controls {
    /// Positive pulls the nose up in the wing's lift plane.
    pub pitch: f32,
    /// Positive rolls left.
    pub bank: f32,
    /// Positive yaws right.
    pub yaw: f32,
    pub boost: bool,
}

impl Controls {
    pub fn neutral() -> Self {
        Self {
            pitch: 0.0,
            bank: 0.0,
            yaw: 0.0,
            boost: false,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Pose {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// Display angles only; physics and rendering use orientation.
    pub heading: f32,
    pub pitch: f32,
    pub bank: f32,
    /// True airspeed in metres per second, relative to the surrounding air.
    pub speed: f32,
    pub boost: f32,
    pub orientation: Quat,
    /// World-space velocity in metres per second, including wind drift.
    pub velocity: Vec3,
    pub load: f32,
    pub rates: Vec3,
}

fn ease(from: f32, to: f32, rate: f32, dt: f32) -> f32 {
    from + (to - from) * (1.0 - (-rate * dt).exp())
}

fn atmosphere(height: f32) -> (f32, f32) {
    let h = height.clamp(0.0, 25000.0);
    let temperature = crate::effects::isa_temperature(h);
    (
        crate::effects::isa_density(h),
        (1.4 * 287.05 * temperature).sqrt(),
    )
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Pose {
    pub fn start() -> Self {
        let (density, _) = atmosphere(1500.0);
        let effective_q = 0.5 * density * CRUISE_SPEED.powi(2) * WING_AREA * 2.8 * 3.8;
        let trim = (MASS * GRAVITY / effective_q - 0.12) / 5.5;
        Self {
            x: 0.0,
            y: 1500.0,
            z: 1050.0,
            heading: 0.0,
            pitch: trim,
            bank: 0.0,
            speed: CRUISE_SPEED,
            boost: 0.0,
            orientation: Quat::from_rotation_x(-trim),
            velocity: Vec3::Z * CRUISE_SPEED,
            load: 1.0,
            rates: Vec3::ZERO,
        }
    }

    /// Linearly interpolate between two poses for sub-step render interpolation (Glenn Fiedler "Fix Your Timestep!").
    /// Eliminates simulation-to-render beat-frequency aliasing and micro-stutter at any display refresh rate.
    pub fn interpolate(&self, next: &Self, alpha: f32) -> Self {
        let alpha = alpha.clamp(0.0, 1.0);
        Self {
            x: self.x + (next.x - self.x) * alpha,
            y: self.y + (next.y - self.y) * alpha,
            z: self.z + (next.z - self.z) * alpha,
            heading: self.heading + (next.heading - self.heading) * alpha,
            pitch: self.pitch + (next.pitch - self.pitch) * alpha,
            bank: self.bank + (next.bank - self.bank) * alpha,
            speed: self.speed + (next.speed - self.speed) * alpha,
            boost: self.boost + (next.boost - self.boost) * alpha,
            orientation: self.orientation.slerp(next.orientation, alpha).normalize(),
            velocity: self.velocity.lerp(next.velocity, alpha),
            load: self.load + (next.load - self.load) * alpha,
            rates: self.rates.lerp(next.rates, alpha),
        }
    }

    pub fn step(&mut self, input: &Controls, dt: f32) {
        self.step_with_wind(input, dt, Vec3::ZERO);
    }

    /// Advance through an air mass moving at `wind_velocity` in world space.
    /// Aerodynamic forces and speed limits use airspeed; world velocity moves
    /// the aircraft over the ground. Call at the fixed `SIM_STEP` interval.
    pub fn step_with_wind(&mut self, input: &Controls, dt: f32, wind_velocity: Vec3) {
        if dt <= 0.0 || !dt.is_finite() {
            return;
        }
        let wind_velocity = if wind_velocity.is_finite() {
            wind_velocity
        } else {
            Vec3::ZERO
        };
        self.boost = ease(
            self.boost,
            if input.boost { 1.0 } else { 0.0 },
            if input.boost { 1.8 } else { 2.4 },
            dt,
        );
        let (density, sound_speed) = atmosphere(self.y);
        let air_velocity = self.velocity - wind_velocity;
        let speed = air_velocity.length().max(1.0);
        let direction = air_velocity / speed;
        let forward = self.orientation * Vec3::Z;
        let up = self.orientation * Vec3::Y;
        let right = self.orientation * -Vec3::X;
        let alpha = (-air_velocity.dot(up)).atan2(air_velocity.dot(forward));
        let beta = direction.dot(right).clamp(-1.0, 1.0).asin();
        let q = 0.5 * density * speed * speed;
        let mach = speed / sound_speed;
        let authority = (0.75 + q / 9000.0).clamp(0.75, 1.9) / (1.0 + (mach - 1.1).max(0.0) * 0.22);
        let sink_boost = (-air_velocity.y / 10.0).clamp(0.0, 1.8);
        let horizontal_airspeed = air_velocity.x.hypot(air_velocity.z);
        let neutral_load = (air_velocity.y.atan2(horizontal_airspeed).cos() / up.y.max(0.14)
            + sink_boost)
            .clamp(0.4, 6.0);
        let commanded_load = (neutral_load
            + input.pitch * if input.pitch > 0.0 { 16.0 } else { 6.0 })
        .clamp(-4.0, 20.0);
        let support = (300.0 / speed.max(40.0)).clamp(1.0, 3.8);
        let effective_q = q.max(1600.0) * 2.8 * support;
        let target_alpha = ((commanded_load * MASS * GRAVITY / (effective_q * WING_AREA) - 0.12)
            / 5.5)
            .clamp(-0.2, 0.36);
        let error = target_alpha - alpha;
        let target_rate = input.pitch * if input.pitch > 0.0 { 2.0 } else { 1.2 };
        let kp = 3.0 + input.pitch.abs() * 19.0;
        let kd = if input.pitch == 0.0 { 9.0 } else { 8.0 };
        let rate_damping = (self.rates.x - target_rate) * kd;
        let acceleration = (error * kp - rate_damping).clamp(-12.0, 12.0) * authority;
        self.rates.x = (self.rates.x + acceleration * dt).clamp(-2.5, 2.5);
        self.rates.y = ease(
            self.rates.y,
            (-input.yaw - beta * 2.0).clamp(-1.0, 1.0) * 0.45 * authority,
            3.5,
            dt,
        );
        self.rates.z = ease(self.rates.z, input.bank * 1.55 * authority, 4.5, dt);
        // Body-axis rotations accumulate: elevator still pulls toward the wings' up
        // direction when banked or inverted, and rolls can pass through 360 degrees.
        self.orientation = (self.orientation
            * Quat::from_scaled_axis(Vec3::new(-self.rates.x, self.rates.y, -self.rates.z) * dt))
        .normalize();

        let separation = smooth(0.38, 0.64, alpha.abs());
        let cl = (0.12 + 5.5 * alpha).clamp(-1.4, 2.6) * (1.0 - separation)
            + (2.0 * alpha).sin() * 0.75 * separation;
        let wave = 0.033 * smooth(0.82, 1.12, mach) - 0.011 * smooth(1.35, 2.3, mach);
        let cd = 0.022 + 0.048 * cl * cl + wave + separation * 0.45;
        let lift = q * WING_AREA * cl * support * 2.8;
        let drag = q * WING_AREA * cd;
        let target_speed = CRUISE_SPEED + (3.0 * sound_speed - CRUISE_SPEED) * self.boost;
        let thrust = drag + MASS * ((target_speed - speed) * 0.65).clamp(-180.0, 150.0);
        let lift_direction = up - direction * up.dot(direction);
        let force = direction * (thrust - drag)
            + lift_direction / lift_direction.length().max(0.001) * lift
            - right * beta * q * WING_AREA * 0.65;
        self.velocity += (force / MASS - Vec3::Y * GRAVITY) * dt;
        self.velocity = (self.velocity - wind_velocity).clamp_length_max(TOP_SPEED) + wind_velocity;
        self.x += self.velocity.x * dt;
        self.y += self.velocity.y * dt;
        self.z += self.velocity.z * dt;
        // Preserve the native world's existing flight envelope.
        if self.y < 130.0 {
            self.y = 130.0;
            self.velocity.y = self.velocity.y.max(0.0);
        }
        if self.y > 22000.0 {
            self.y = 22000.0;
            self.velocity.y = self.velocity.y.min(0.0);
        }
        self.speed = (self.velocity - wind_velocity).length();
        self.load = lift / (MASS * GRAVITY);
        let nose = self.orientation * Vec3::Z;
        self.pitch = nose.y.clamp(-1.0, 1.0).asin();
        let heading = nose.x.atan2(nose.z);
        self.heading += (heading - self.heading)
            .sin()
            .atan2((heading - self.heading).cos());
        let lateral = self.orientation * Vec3::X;
        let up = self.orientation * Vec3::Y;
        self.bank = (-lateral.y).atan2(up.y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::ChaseCamera;

    fn fly(p: &mut Pose, controls: Controls, seconds: f32) {
        for _ in 0..(seconds / SIM_STEP) as usize {
            p.step(&controls, SIM_STEP);
        }
    }

    fn assert_same_pose(actual: &Pose, expected: &Pose) {
        assert_eq!(actual.orientation, expected.orientation);
        assert_eq!(actual.velocity, expected.velocity);
        assert_eq!(actual.rates, expected.rates);
        assert_eq!(
            [actual.x, actual.y, actual.z, actual.heading, actual.pitch, actual.bank,
                actual.speed, actual.boost, actual.load],
            [expected.x, expected.y, expected.z, expected.heading, expected.pitch, expected.bank,
                expected.speed, expected.boost, expected.load],
        );
    }

    #[test]
    fn zero_wind_preserves_the_calm_flight_api() {
        let mut calm = Pose::start();
        let mut windy = calm;
        let controls = Controls {
            pitch: 0.2,
            bank: -0.3,
            yaw: 0.1,
            boost: true,
        };
        for _ in 0..500 {
            calm.step(&controls, SIM_STEP);
            windy.step_with_wind(&controls, SIM_STEP, Vec3::ZERO);
        }
        assert_same_pose(&windy, &calm);
    }

    #[test]
    fn moving_air_mass_preserves_aerodynamics_and_adds_ground_drift() {
        let wind = Vec3::new(18.0, 0.0, -12.0);
        for initial_speed in [CRUISE_SPEED, TOP_SPEED + 50.0] {
            let mut calm = Pose::start();
            calm.velocity = Vec3::Z * initial_speed;
            let mut drifting = calm;
            drifting.velocity += wind;
            calm.step(&Controls::neutral(), SIM_STEP);
            drifting.step_with_wind(&Controls::neutral(), SIM_STEP, wind);

            assert!(drifting.orientation.abs_diff_eq(calm.orientation, 0.00001));
            assert!((drifting.load - calm.load).abs() < 0.0001);
            assert!((drifting.speed - calm.speed).abs() < 0.0001);
            assert!((drifting.velocity - wind - calm.velocity).length() < 0.0001);
            let displacement = Vec3::new(drifting.x - calm.x, drifting.y - calm.y, drifting.z - calm.z);
            assert!((displacement - wind * SIM_STEP).length() < 0.0002);
            assert!(drifting.speed <= TOP_SPEED + 0.001);
        }
    }

    #[test]
    fn crosswind_changes_sideslip_and_world_trajectory() {
        let mut calm = Pose::start();
        let mut windy = calm;
        let wind = Vec3::X * 15.0;
        for _ in 0..144 {
            calm.step(&Controls::neutral(), SIM_STEP);
            windy.step_with_wind(&Controls::neutral(), SIM_STEP, wind);
        }
        assert!(windy.x > calm.x + 0.05, "crosswind must displace the aircraft");
        assert!((windy.heading - calm.heading).abs() > 0.001);
        assert!((windy.speed - (windy.velocity - wind).length()).abs() < 0.0001);
    }

    #[test]
    fn invalid_timestep_does_not_advance_flight_and_invalid_wind_is_calm() {
        let initial = Pose::start();
        for dt in [0.0, -SIM_STEP, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut pose = initial;
            pose.step_with_wind(&Controls::neutral(), dt, Vec3::X * 15.0);
            assert_same_pose(&pose, &initial);
        }
        let mut expected = initial;
        expected.step(&Controls::neutral(), SIM_STEP);
        for wind in [Vec3::splat(f32::NAN), Vec3::splat(f32::INFINITY)] {
            let mut pose = initial;
            pose.step_with_wind(&Controls::neutral(), SIM_STEP, wind);
            assert_same_pose(&pose, &expected);
        }
    }

    #[test]
    fn prolonged_gusting_flight_preserves_the_flight_envelope() {
        let regimes = [
            Controls::neutral(),
            Controls { boost: true, ..Controls::neutral() },
            Controls { pitch: 0.15, bank: 0.7, yaw: 0.1, boost: true },
        ];
        for strength in [1.0, 3.0] {
            let wind = crate::wind::Wind::new(strength);
            for start_altitude in [131.0, 21999.0] {
                for controls in regimes {
                    let mut pose = Pose::start();
                    pose.y = start_altitude;
                    pose.velocity += wind.velocity(Vec3::new(pose.x, pose.y, pose.z), 0.0);
                    // A minute per regime spans many gust cycles and exercises
                    // both altitude boundaries during cruise, boost and turns.
                    for step in 0..(60.0 / SIM_STEP) as usize {
                        let air_motion = wind.velocity(
                            Vec3::new(pose.x, pose.y, pose.z), step as f32 * SIM_STEP,
                        );
                        pose.step_with_wind(&controls, SIM_STEP, air_motion);
                        assert!(Vec3::new(pose.x, pose.y, pose.z).is_finite());
                        assert!(pose.velocity.is_finite() && pose.rates.is_finite());
                        assert!(pose.orientation.is_finite());
                        assert!((pose.orientation.length() - 1.0).abs() < 0.00001);
                        assert!([pose.speed, pose.load, pose.heading, pose.pitch, pose.bank]
                            .iter().all(|value| value.is_finite()));
                        assert!((130.0..=22000.0).contains(&pose.y));
                        assert!((0.0..=TOP_SPEED + 0.01).contains(&pose.speed),
                            "airspeed {} at strength {strength}, altitude {}, step {step}",
                            pose.speed, pose.y);
                        assert!((pose.speed - (pose.velocity - air_motion).length()).abs() < 0.0001);
                    }
                }
            }
        }
    }

    #[test]
    fn controls_change_world_trajectory() {
        let mut level = Pose::start();
        fly(&mut level, Controls::neutral(), 5.0);
        assert!((level.y - 1500.0).abs() < 15.0);
        assert!(level.z > 1350.0);
        for sign in [-1.0, 1.0] {
            let mut pitch = Pose::start();
            fly(
                &mut pitch,
                Controls {
                    pitch: sign,
                    ..Controls::neutral()
                },
                2.0,
            );
            assert!((pitch.y - level.y) * sign > 5.0);
            let mut bank = Pose::start();
            fly(
                &mut bank,
                Controls {
                    bank: sign,
                    ..Controls::neutral()
                },
                0.7,
            );
            fly(&mut bank, Controls::neutral(), 2.0);
            assert!(
                bank.x * sign > 5.0,
                "bank must bend the world trajectory: {}",
                bank.x
            );
            let mut yaw = Pose::start();
            fly(
                &mut yaw,
                Controls {
                    yaw: sign,
                    ..Controls::neutral()
                },
                2.0,
            );
            assert!(
                yaw.x * sign < -0.1,
                "yaw displacement {}, heading {}",
                yaw.x,
                yaw.heading
            );
        }
        let mut roll = Pose::start();
        fly(
            &mut roll,
            Controls {
                bank: 1.0,
                ..Controls::neutral()
            },
            3.0,
        );
        assert!((roll.orientation.length() - 1.0).abs() < 0.0001);
        assert!(roll.orientation.is_finite() && roll.velocity.is_finite());
        assert!(
            (roll.orientation * Vec3::X).y > 0.5,
            "roll must pass through inverted"
        );
        let mut boosted = Pose::start();
        fly(
            &mut boosted,
            Controls {
                boost: true,
                ..Controls::neutral()
            },
            10.0,
        );
        assert!(boosted.speed > CRUISE_SPEED * 3.0 && boosted.speed <= TOP_SPEED + 0.01);
    }

    #[test]
    fn pitch_control_is_smooth_without_chattering_square_wave() {
        let mut pose = Pose::start();
        let controls = Controls {
            pitch: 1.0,
            bank: 0.5,
            yaw: 0.0,
            boost: false,
        };

        let mut sign_flips = 0;
        let mut prev_diff = 0.0f32;
        let mut prev_rate = pose.rates.x;

        for step in 0..500 {
            pose.step(&controls, SIM_STEP);
            let diff = pose.rates.x - prev_rate;
            if step > 20 && diff.abs() > 0.0001 && prev_diff.abs() > 0.0001 {
                if diff.signum() != prev_diff.signum() {
                    sign_flips += 1;
                }
            }
            prev_diff = diff;
            prev_rate = pose.rates.x;
        }

        // A continuous 2nd order PD response must converge smoothly to trim with minimal damping oscillations.
        // A chattering square wave would flip signs every 1-2 steps (hundreds of flips).
        assert!(
            sign_flips < 6,
            "pitch rate chattered with {} sign flips, indicating square wave instability",
            sign_flips
        );
    }

    #[test]
    fn pitch_release_does_not_rebound_negatively() {
        let mut pose = Pose::start();
        fly(&mut pose, Controls { pitch: 1.0, ..Controls::neutral() }, 0.3);
        println!("At release (0.3s pulse): rates.x = {}, pitch = {}", pose.rates.x, pose.pitch);
        let mut min_rate = pose.rates.x;
        for i in 0..144 {
            pose.step(&Controls::neutral(), SIM_STEP);
            min_rate = min_rate.min(pose.rates.x);
            if i % 6 == 0 {
                println!("step {:3}: rates.x = {:+.3}, pitch = {:+.3}, load = {:.3}", i, pose.rates.x, pose.pitch, pose.load);
            }
        }
        println!("min_rate after release: {}", min_rate);

        let mut pose_bank = Pose::start();
        let mut cam_bank = ChaseCamera::new();
        cam_bank.snap(&pose_bank);
        for _ in 0..72 {
            let ctrl = Controls { pitch: 1.0, bank: 1.0, ..Controls::neutral() };
            pose_bank.step(&ctrl, SIM_STEP);
            let frame = cam_bank.step(&pose_bank, &ctrl, SIM_STEP, 1.6, Vec3::ZERO);
            let nose_world = Vec3::new(pose_bank.x, pose_bank.y, pose_bank.z) + pose_bank.orientation * Vec3::Z * 10.0;
            let ndc = frame.view_proj.project_point3(nose_world);
        }
        println!("Camera bank turn test: releasing controls now");
        for i in 0..72 {
            pose_bank.step(&Controls::neutral(), SIM_STEP);
            let frame = cam_bank.step(&pose_bank, &Controls::neutral(), SIM_STEP, 1.6, Vec3::ZERO);
            let nose_world = Vec3::new(pose_bank.x, pose_bank.y, pose_bank.z) + pose_bank.orientation * Vec3::Z * 10.0;
            let ndc = frame.view_proj.project_point3(nose_world);
            if i % 6 == 0 {
                let target = ChaseCamera::compute_target_orientation(&pose_bank);
                let lag = 2.0 * cam_bank.orientation().dot(target).abs().clamp(-1.0, 1.0).acos().to_degrees();
                println!("step {:2}: nose NDC = ({:+.3}, {:+.3}), lag = {:.2} deg, rates = ({:+.2}, {:+.2}, {:+.2})",
                    i, ndc.x, ndc.y, lag, pose_bank.rates.x, pose_bank.rates.y, pose_bank.rates.z);
            }
        }

    }
}


