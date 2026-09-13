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
        self.pitch += (u.pitch * 0.6 - self.pitch) * k;
        self.bank += (u.bank * 1.1 - self.bank) * k;

        // Coordinated turn dynamics: banking induces heading change (yaw rate),
        // supplemented by direct rudder yaw.
        self.heading += -self.bank * 0.35 * dt + u.yaw * 0.5 * dt;

        // Engine thrust and aerodynamic drag response.
        let want = if u.boost { TOP_SPEED } else { CRUISE_SPEED };
        self.speed += (want - self.speed) * (1.0 - (-0.8 * dt).exp());
        self.boost += ((if u.boost { 1.0 } else { 0.0 }) - self.boost) * k;

        // Project forward velocity along heading and pitch into 3D Cartesian motion.
        let cp = self.pitch.cos();
        self.x += self.heading.sin() * cp * self.speed * dt;
        self.z += self.heading.cos() * cp * self.speed * dt;
        self.y += self.pitch.sin() * self.speed * dt;

        // Clamp altitude to operational flight envelope (terrain floor to stratosphere).
        self.y = self.y.clamp(130.0, 22000.0);
    }
}
