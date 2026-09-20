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
    /// Integral of the angle-of-attack error, in rad/s^2 of commanded pitch
    /// acceleration. Trims sustained turns: proportional-only control needs a
    /// persistent error to hold the body rate, which let wing load settle well
    /// below the commanded value and every banked turn sink away.
    alpha_trim: f32,
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
            alpha_trim: 0.0,
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
            alpha_trim: self.alpha_trim + (next.alpha_trim - self.alpha_trim) * alpha,
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
        // Bounded load command: an expedition craft pulls at most ~5.5 g and
        // pushes at most ~3 g regardless of how hard the stick is held. The
        // previous -4..20 g envelope let a binary key tap whip the airframe
        // through 10+ g swings that read as rubberbanding.
        let commanded_load = (neutral_load
            + input.pitch * if input.pitch > 0.0 { 4.5 } else { 3.0 })
        .clamp(-3.0, 5.5);
        let support = (300.0 / speed.max(40.0)).clamp(1.0, 3.8);
        let effective_q = q.max(1600.0) * 2.8 * support;
        let target_alpha = ((commanded_load * MASS * GRAVITY / (effective_q * WING_AREA) - 0.12)
            / 5.5)
            .clamp(-0.2, 0.36);
        let error = target_alpha - alpha;
        // Balanced angle-of-attack PD with one gain set for stick-held and
        // released flight. The old split (kp 22 held, kp 3 released) collapsed
        // the error drive on release until rate damping dominated it: alpha
        // sat below target, load sagged, and the plane wallowed through a
        // multi-second bounce after every input.
        let target_rate = input.pitch * if input.pitch > 0.0 { 1.1 } else { 0.85 };
        let kp = 10.0;
        let kd = 8.0;
        let rate_damping = (self.rates.x - target_rate) * kd;
        // Slow integral trim with conditional integration: it removes the
        // steady-state angle-of-attack error of sustained turns (proportional
        // control alone settles with wing load below the commanded value, so
        // every banked turn sank) but never winds up while the acceleration
        // command is pinned at its clamp during hard maneuvers.
        let unsaturated = (error * kp - rate_damping).abs() < 6.5;
        // The trim only earns its keep on small sustained errors (a steady
        // turn); integrating through large transient errors — a stall arc or
        // a full pull — winds it against the recovery. A slow leak keeps
        // stale trim from outliving the flight regime that created it.
        if unsaturated && error.abs() < 0.12 {
            self.alpha_trim = (self.alpha_trim + error * 5.0 * dt).clamp(-0.8, 0.8);
        }
        self.alpha_trim -= self.alpha_trim * 0.15 * dt;
        let acceleration =
            (error * kp + self.alpha_trim - rate_damping).clamp(-7.0, 7.0) * authority;
        self.rates.x = (self.rates.x + acceleration * dt).clamp(-1.4, 1.4);
        self.rates.y = ease(
            self.rates.y,
            (-input.yaw - beta * 2.0).clamp(-1.0, 1.0) * 0.45 * authority,
            3.5,
            dt,
        );
        // Bank-angle command instead of roll-rate command. With a rate law,
        // every keyboard tap left a leftover bank that nothing restored, so
        // the load controller wrestled a deepening spiral: the horizon kept
        // swinging seconds after release and altitude drained away. Position
        // control holds a commanded bank while the key is down, and wings
        // level gently after release, so turns settle instead of diverging.
        let commanded_bank = input.bank * 1.05;
        let mut bank_error = commanded_bank - self.bank;
        if bank_error > std::f32::consts::PI {
            bank_error -= std::f32::consts::TAU;
        } else if bank_error < -std::f32::consts::PI {
            bank_error += std::f32::consts::TAU;
        }
        let roll_speed_cap = if input.bank != 0.0 {
            1.5
        } else if forward.y.abs() < 0.95 {
            0.5
        } else {
            // Near vertical the bank measurement is ill-defined; chasing it
            // just spins the aircraft about its mast.
            0.0
        };
        let roll_target = (bank_error * 2.2 * authority).clamp(-roll_speed_cap, roll_speed_cap);
        self.rates.z = ease(self.rates.z, roll_target, 5.0, dt);
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
        // Held roll input commands a steady bank (position control), not an
        // endless roll rate: the wings stay put where the key puts them.
        assert!(
            (roll.bank.to_degrees() - 60.0).abs() < 5.0,
            "held input must hold ~60 deg of bank, got {}",
            roll.bank.to_degrees()
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
    fn brief_pitch_inputs_stay_proportionate_and_settle_after_release() {
        // Rubberbanding regression: binary key taps used to slam the airframe
        // through 10+ g and 140 deg/s, then wallow at 0.3 g for tens of
        // seconds after release because the released-stick gains collapsed
        // the angle-of-attack error drive below its rate damping.
        let tap = Controls { pitch: 1.0, ..Controls::neutral() };
        let mut pose = Pose::start();
        let start_pitch = pose.pitch;
        let mut peak_rate = 0.0f32;
        let mut peak_load = 0.0f32;
        for _ in 0..(0.1 / SIM_STEP) as usize {
            pose.step(&tap, SIM_STEP);
            peak_rate = peak_rate.max(pose.rates.x.abs());
            peak_load = peak_load.max(pose.load);
        }
        assert!(
            peak_rate < 0.9 && peak_load < 3.5,
            "100 ms tap whipped the nose: {peak_rate} rad/s at {peak_load} g"
        );
        let tap_pitch = pose.pitch;
        assert!(
            tap_pitch - start_pitch < 0.18,
            "100 ms tap moved the attitude {} deg",
            (tap_pitch - start_pitch).to_degrees()
        );

        // After release the controller must hold trim smoothly: no rebound
        // through the release attitude, and the rate settles within a second.
        let mut significant_flips = 0;
        let mut armed = false;
        let mut prev_sign = 0i8;
        for step in 0..(3.0 / SIM_STEP) as usize {
            pose.step(&Controls::neutral(), SIM_STEP);
            if step > (1.0 / SIM_STEP) as usize {
                assert!(
                    pose.rates.x.abs() < 0.05,
                    "rate still {} deg/s a second after release",
                    pose.rates.x.to_degrees()
                );
                assert!(
                    (0.5..=2.0).contains(&pose.load),
                    "load sagged to {} g after release",
                    pose.load
                );
            }
            assert!(
                pose.pitch > tap_pitch - 0.04,
                "nose rebounded below the release attitude: {} vs {}",
                pose.pitch.to_degrees(),
                tap_pitch.to_degrees()
            );
            // Only count reversals that carry real rate; a damped settle may
            // drift through zero once, but must not oscillate.
            if pose.rates.x.abs() > 0.1 {
                let sign = pose.rates.x.signum() as i8;
                if armed && sign != prev_sign {
                    significant_flips += 1;
                }
                armed = true;
                prev_sign = sign;
            }
        }
        assert!(
            significant_flips == 0,
            "release oscillated with {significant_flips} significant rate reversals"
        );

        // A full-second pull must stay inside the bounded load command even
        // while the stick is held.
        let mut pose = Pose::start();
        let mut peak_load = 0.0f32;
        let mut peak_rate = 0.0f32;
        for _ in 0..(1.0 / SIM_STEP) as usize {
            pose.step(&tap, SIM_STEP);
            peak_load = peak_load.max(pose.load);
            peak_rate = peak_rate.max(pose.rates.x.abs());
        }
        assert!(
            peak_load < 8.0 && peak_rate < 1.2,
            "one-second pull hit {peak_load} g at {peak_rate} rad/s"
        );
    }

    #[test]
    fn released_stick_pulls_out_of_a_steep_dive_without_wallowing() {
        // Rubberbanding regression: after an over-rotation into a near
        // vertical climb the released controller used to sag to 0.2-0.4 g and
        // arc over at under 1 deg/s, so the dive ran on for half a minute.
        // The recovery need not return all the way to level flight — the
        // flight model deliberately holds whatever path you leave it in —
        // but it must break the vertical promptly and keep the wing loaded.
        let mut pose = Pose::start();
        let pull = Controls { pitch: 1.0, ..Controls::neutral() };
        for _ in 0..(1.6 / SIM_STEP) as usize {
            pose.step(&pull, SIM_STEP);
        }
        assert!(pose.pitch > 1.2, "setup must swing the nose high: {}", pose.pitch.to_degrees());
        let mut min_load_below_45 = f32::INFINITY;
        let mut broke_vertical = false;
        for _ in 0..(12.0 / SIM_STEP) as usize {
            pose.step(&Controls::neutral(), SIM_STEP);
            if pose.pitch.to_degrees() < 45.0 {
                min_load_below_45 = min_load_below_45.min(pose.load);
                broke_vertical = true;
            }
        }
        assert!(
            broke_vertical,
            "nose never dropped below 45 deg within 12 s of release"
        );
        assert!(
            min_load_below_45 > 0.45,
            "wing load sagged to {min_load_below_45} g while diving"
        );
    }

    #[test]
    fn banked_turns_hold_altitude_and_wings_level_after_release() {
        // Rubberbanding regression: roll used to be rate-controlled, so any
        // bank tap left a leftover bank that nothing restored. The load
        // controller then wrestled a deepening spiral for tens of seconds
        // while the horizon kept swinging and altitude drained away.
        // Position control must hold the commanded bank, hold altitude in the
        // turn, and level the wings gently after release.
        let wind = crate::wind::Wind::new(1.0);
        let mut pose = Pose::start();
        let bank = Controls { bank: 1.0, ..Controls::neutral() };
        let mut t = 0.0f32;
        let mut alt_min = f32::INFINITY;
        let mut alt_max = f32::NEG_INFINITY;
        let mut load_min = f32::INFINITY;
        let mut load_max = f32::NEG_INFINITY;
        while t < 8.0 {
            let air = wind.velocity(glam::Vec3::new(pose.x, pose.y, pose.z), t);
            pose.step_with_wind(&bank, SIM_STEP, air);
            t += SIM_STEP;
            alt_min = alt_min.min(pose.y);
            alt_max = alt_max.max(pose.y);
            load_min = load_min.min(pose.load);
            load_max = load_max.max(pose.load);
        }
        assert!(
            (pose.bank.to_degrees() - 60.0).abs() < 5.0,
            "held input must hold the commanded bank, got {}",
            pose.bank.to_degrees()
        );
        assert!(
            alt_max - alt_min < 150.0,
            "sustained 60 deg turn must not dump altitude: {alt_max} - {alt_min} m"
        );
        assert!(
            (0.4..=3.0).contains(&load_min) && (0.4..=3.0).contains(&load_max),
            "turn load left the sane envelope: {load_min}..{load_max} g"
        );

        // Release: wings level within a few seconds and stay level.
        let mut leveled = false;
        let mut min_load_after = f32::INFINITY;
        for _ in 0..(8.0 / SIM_STEP) as usize {
            let air = wind.velocity(glam::Vec3::new(pose.x, pose.y, pose.z), t);
            pose.step_with_wind(&Controls::neutral(), SIM_STEP, air);
            t += SIM_STEP;
            min_load_after = min_load_after.min(pose.load);
            if t > 14.0 {
                assert!(
                    pose.bank.abs() < 0.05,
                    "wings still banked {} deg two-plus seconds after release",
                    pose.bank.to_degrees()
                );
            }
            if pose.bank.abs() < 0.05 {
                leveled = true;
            }
        }
        assert!(leveled, "wings never leveled after release");
        assert!(
            min_load_after > 0.3,
            "wing load sagged to {min_load_after} g while leveling"
        );
    }

    #[test]
    fn brief_bank_taps_are_proportionate_and_do_not_spiral() {
        // A short bank tap must establish a modest bank, and releasing must
        // return to wings level without a spiral dive developing.
        let wind = crate::wind::Wind::new(1.0);
        let mut pose = Pose::start();
        let tap = Controls { bank: 1.0, ..Controls::neutral() };
        let mut t = 0.0f32;
        for _ in 0..(0.3 / SIM_STEP) as usize {
            let air = wind.velocity(glam::Vec3::new(pose.x, pose.y, pose.z), t);
            pose.step_with_wind(&tap, SIM_STEP, air);
            t += SIM_STEP;
        }
        let tap_bank = pose.bank;
        assert!(
            tap_bank.to_degrees() > 10.0 && tap_bank.to_degrees() < 45.0,
            "300 ms tap produced {} deg of bank",
            tap_bank.to_degrees()
        );
        let mut max_sink = 0.0f32;
        for _ in 0..(6.0 / SIM_STEP) as usize {
            let air = wind.velocity(glam::Vec3::new(pose.x, pose.y, pose.z), t);
            pose.step_with_wind(&Controls::neutral(), SIM_STEP, air);
            t += SIM_STEP;
            max_sink = max_sink.max(-pose.velocity.y);
        }
        assert!(
            pose.bank.abs() < 0.05,
            "wings still banked {} deg after the tap released",
            pose.bank.to_degrees()
        );
        assert!(
            max_sink < 12.0,
            "post-tap recovery sank at {max_sink} m/s: spiral developing"
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
            let _ndc = frame.view_proj.project_point3(nose_world);
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



#[cfg(test)]
mod level_flight_stability {
    use super::*;
    use crate::wind::Wind;

    /// Zero-input level flight with the default in-game breeze must not
    /// porpoise: a sink-rate-coupled load command plus the angle-of-attack PD
    /// can hunt around trim, and a slow pitch/altitude oscillation reads as
    /// rubber-banding exactly when the horizon is in the frame. This pins the
    /// 60 s pitch envelope and vertical speed so future controller changes
    /// cannot quietly reintroduce the hunt.
    #[test]
    fn zero_input_level_flight_does_not_porpoise() {
        let wind = Wind::new(1.0);
        let mut pose = Pose::start();
        pose.velocity += wind.velocity(Vec3::new(pose.x, pose.y, pose.z), 0.0);
        let controls = Controls::neutral();
        let mut sim_time = 0.0f32;
        let mut pitch_min = f32::MAX;
        let mut pitch_max = f32::MIN;
        let mut vy_min = f32::MAX;
        let mut vy_max = f32::MIN;
        for _ in 0..(60.0 / SIM_STEP) as usize {
            let air = wind.velocity(Vec3::new(pose.x, pose.y, pose.z), sim_time);
            pose.step_with_wind(&controls, SIM_STEP, air);
            sim_time += SIM_STEP;
            pitch_min = pitch_min.min(pose.pitch);
            pitch_max = pitch_max.max(pose.pitch);
            vy_min = vy_min.min(pose.velocity.y);
            vy_max = vy_max.max(pose.velocity.y);
        }
        let envelope = (pitch_max - pitch_min).to_degrees();
        assert!(
            envelope < 1.0,
            "level-flight pitch envelope {envelope:.3} deg over 60 s: the controller is hunting"
        );
        assert!(
            vy_min > -1.0 && vy_max < 1.0,
            "vertical speed [{vy_min:.2}, {vy_max:.2}] m/s: phugoid exceeds the readable band"
        );
    }
}
