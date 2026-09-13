// Soft glider sim. Fixed rates. One plane.
// Ported from the web prototype. Same constants.

/// Nominal gliding speed in meters per second (~136 knots).
pub const CRUISE_SPEED: f32 = 70.0;
/// Maximum speed under afterburner boost in meters per second (~2000 knots).
pub const TOP_SPEED: f32 = 1030.0;
/// Fixed physics simulation timestep (144 Hz) to decouple flight math from display refresh.
pub const SIM_STEP: f32 = 1.0 / 144.0;

/// Pilot input control state aggregated from raw keyboard scan codes.
#[derive(Clone, Copy)]
pub struct Controls {
    /// Elevator input (-1.0 to +1.0): positive pitches nose down, negative pitches nose up.
    pub pitch: f32,
    /// Aileron input (-1.0 to +1.0): positive rolls left, negative rolls right.
    pub bank: f32,
    /// Rudder input (-1.0 to +1.0): positive yaws right, negative yaws left.
    pub yaw: f32,
    /// Boost throttle active (e.g. holding Shift).
    pub boost: bool,
}

impl Controls {
    /// Neutral controls with zero deflection and boost off.
    #[allow(dead_code)]
    pub fn neutral() -> Self {
        Self {
            pitch: 0.0,
            bank: 0.0,
            yaw: 0.0,
            boost: false,
        }
    }
}

/// 6-DOF aircraft spatial state and aerodynamics tracking in world space.
/// Coordinate system: Right-handed Y-up (+X right, +Y up, +Z forward).
#[derive(Clone, Copy)]
pub struct Pose {
    /// World X position in meters.
    pub x: f32,
    /// World Y altitude in meters.
    pub y: f32,
    /// World Z forward position in meters.
    pub z: f32,
    /// Compass heading angle around Y axis (radians).
    pub heading: f32,
    /// Pitch angle around local X axis (radians).
    pub pitch: f32,
    /// Bank (roll) angle around local Z axis (radians).
    pub bank: f32,
    /// Current ground/airspeed in meters per second.
    pub speed: f32,
    /// Smoothed boost factor in range [0.0, 1.0] for engine glow and spool animation.
    pub boost: f32,
}

impl Pose {
    /// Initial spawn pose: elevated at 1500 m cruise altitude, level flight facing north.
    pub fn start() -> Self {
        Self {
            x: 0.0,
            y: 1500.0,
            z: 1050.0,
            heading: 0.0,
            pitch: 0.0,
            bank: 0.0,
            speed: CRUISE_SPEED,
            boost: 0.0,
        }
    }

    /// Advance flight state by fixed time step `dt` using aerodynamic response curves.
    pub fn step(&mut self, u: &Controls, dt: f32) {
        // First-order exponential response factor for control surfaces.
        let k = 1.0 - (-4.0 * dt).exp();

        // Attitude angular rates damped towards target deflections.
        // Pitch is commanded relative to the aircraft's lift plane (wing lateral axis).
        self.pitch += (u.pitch * 0.6 - self.pitch) * k;
        self.bank += (u.bank * 1.1 - self.bank) * k;

        // Coordinated turn dynamics:
        // 1. Passive banking induces aerodynamic slip turn.
        // 2. Elevator pitch in a banked attitude pulls the nose through the turn
        //    proportional to the horizontal component of the lift vector (sin(bank)).
        // 3. Direct rudder yaw command.
        let turn_from_bank = -self.bank * 0.35;
        let turn_from_lift_pitch = -self.pitch * self.bank.sin() * 1.6;
        let turn_from_rudder = u.yaw * 0.5;
        self.heading += (turn_from_bank + turn_from_lift_pitch + turn_from_rudder) * dt;

        // Engine thrust and aerodynamic drag response.
        let want = if u.boost { TOP_SPEED } else { CRUISE_SPEED };
        self.speed += (want - self.speed) * (1.0 - (-0.8 * dt).exp());
        self.boost += ((if u.boost { 1.0 } else { 0.0 }) - self.boost) * k;

        // Project forward velocity along the aircraft's 3D orientation.
        // Pitch is relative to the lift plane (rolled by bank):
        //   forward_local = R_z(bank) * R_x(-pitch) * (0, 0, 1)
        //   forward_world = R_y(heading) * forward_local
        let sp = self.pitch.sin();
        let cp = self.pitch.cos();
        let sb = self.bank.sin();
        let cb = self.bank.cos();

        let local_x = -sp * sb;
        let local_y = sp * cb;
        let local_z = cp;

        let sh = self.heading.sin();
        let ch = self.heading.cos();

        let forward_x = local_z * sh + local_x * ch;
        let forward_y = local_y;
        let forward_z = local_z * ch - local_x * sh;

        self.x += forward_x * self.speed * dt;
        self.y += forward_y * self.speed * dt;
        self.z += forward_z * self.speed * dt;

        // Clamp altitude to operational flight envelope (terrain floor to stratosphere).
        self.y = self.y.clamp(130.0, 22000.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    #[test]
    fn test_lift_plane_pitch_dynamics() {
        // 1. Wings level: pitch up should produce vertical climb and zero horizontal turn.
        let mut p = Pose::start();
        p.pitch = 0.5;
        p.bank = 0.0;
        let u = Controls {
            pitch: 1.0,
            bank: 0.0,
            yaw: 0.0,
            boost: false,
        };
        let y_before = p.y;
        p.step(&u, 0.1);
        assert!(p.y > y_before, "wings-level pitch up must climb");
        assert_eq!(p.heading, 0.0, "wings-level pitch up must not change heading");

        // 2. Banked 90 degrees left: pitch up should turn heading left and produce zero vertical climb.
        let mut p_left = Pose::start();
        p_left.pitch = 0.6;
        p_left.bank = PI / 2.0;
        let u_bank_left = Controls {
            pitch: 1.0,
            bank: (PI / 2.0) / 1.1, // Maintains bank at PI/2
            yaw: 0.0,
            boost: false,
        };
        let y_start = p_left.y;
        p_left.step(&u_bank_left, 0.01);
        assert!(
            (p_left.y - y_start).abs() < 0.05,
            "knife-edge pitch up must have minimal vertical climb relative to ground, got delta {}",
            p_left.y - y_start
        );
        assert!(
            p_left.heading < 0.0,
            "banked left pitch up must turn heading left (negative)"
        );

        // 3. Banked 90 degrees right: pitch up should turn heading right and produce zero vertical climb.
        let mut p_right = Pose::start();
        p_right.pitch = 0.6;
        p_right.bank = -PI / 2.0;
        let u_bank_right = Controls {
            pitch: 1.0,
            bank: (-PI / 2.0) / 1.1, // Maintains bank at -PI/2
            yaw: 0.0,
            boost: false,
        };
        let y_start = p_right.y;
        p_right.step(&u_bank_right, 0.01);
        assert!(
            (p_right.y - y_start).abs() < 0.05,
            "knife-edge pitch up must have minimal vertical climb relative to ground, got delta {}",
            p_right.y - y_start
        );
        assert!(
            p_right.heading > 0.0,
            "banked right pitch up must turn heading right (positive)"
        );

        // 4. Inverted flight: pitch up relative to lift plane must dive toward ground.
        let mut p_inv = Pose::start();
        p_inv.pitch = 0.6;
        p_inv.bank = PI;
        let u_inv = Controls {
            pitch: 1.0,
            bank: PI / 1.1,
            yaw: 0.0,
            boost: false,
        };
        let y_start = p_inv.y;
        p_inv.step(&u_inv, 0.01);
        assert!(
            p_inv.y < y_start,
            "inverted pitch up relative to lift plane must dive toward ground"
        );
    }
}
