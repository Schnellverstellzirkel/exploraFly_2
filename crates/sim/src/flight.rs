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
    pub speed: f32,
    pub boost: f32,
    pub orientation: Quat,
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

    pub fn step(&mut self, input: &Controls, dt: f32) {
        if dt <= 0.0 || !dt.is_finite() {
            return;
        }
        self.boost = ease(
            self.boost,
            if input.boost { 1.0 } else { 0.0 },
            if input.boost { 1.8 } else { 2.4 },
            dt,
        );
        let (density, sound_speed) = atmosphere(self.y);
        let speed = self.velocity.length().max(1.0);
        let direction = self.velocity / speed;
        let forward = self.orientation * Vec3::Z;
        let up = self.orientation * Vec3::Y;
        let right = self.orientation * -Vec3::X;
        let alpha = (-self.velocity.dot(up)).atan2(self.velocity.dot(forward));
        let beta = direction.dot(right).clamp(-1.0, 1.0).asin();
        let q = 0.5 * density * speed * speed;
        let mach = speed / sound_speed;
        let authority = (0.75 + q / 9000.0).clamp(0.75, 1.9) / (1.0 + (mach - 1.1).max(0.0) * 0.22);
        let sink_boost = (-self.velocity.y / 10.0).clamp(0.0, 1.8);
        let ground_speed = self.velocity.x.hypot(self.velocity.z);
        let neutral_load = (self.velocity.y.atan2(ground_speed).cos() / up.y.max(0.14)
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
        let rate_damping = (self.rates.x - target_rate) * 8.0;
        let acceleration = (error * 22.0 - rate_damping).clamp(-12.0, 12.0) * authority;
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
        self.velocity = self.velocity.clamp_length_max(TOP_SPEED);
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
        self.speed = self.velocity.length();
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

    fn fly(p: &mut Pose, controls: Controls, seconds: f32) {
        for _ in 0..(seconds / SIM_STEP) as usize {
            p.step(&controls, SIM_STEP);
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
}

