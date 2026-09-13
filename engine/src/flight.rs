// Soft glider sim. Fixed rates. One plane.
// Ported from the web prototype. Same constants.

pub const CRUISE_SPEED: f32 = 70.0;
pub const TOP_SPEED: f32 = 1030.0;

#[derive(Clone, Copy)]
pub struct Controls {
    pub pitch: f32,
    pub bank: f32,
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
    pub heading: f32,
    pub pitch: f32,
    pub bank: f32,
    pub speed: f32,
    pub boost: f32,
}

impl Pose {
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

    pub fn step(&mut self, u: &Controls, dt: f32) {
        let k = 1.0 - (-4.0 * dt).exp();
        self.pitch += (u.pitch * 0.6 - self.pitch) * k;
        self.bank += (u.bank * 1.1 - self.bank) * k;
        self.heading += -self.bank * 0.35 * dt + u.yaw * 0.5 * dt;
        let want = if u.boost { TOP_SPEED } else { CRUISE_SPEED };
        self.speed += (want - self.speed) * (1.0 - (-0.8 * dt).exp());
        self.boost += ((if u.boost { 1.0 } else { 0.0 }) - self.boost) * k;
        let cp = self.pitch.cos();
        self.x += self.heading.sin() * cp * self.speed * dt;
        self.z += self.heading.cos() * cp * self.speed * dt;
        self.y += self.pitch.sin() * self.speed * dt;
        self.y = self.y.clamp(130.0, 22000.0);
    }
}
