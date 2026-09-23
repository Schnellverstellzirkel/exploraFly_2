//! Procedural flight animation, aeroelastic wing flex, and kinematic node transforms.
//!
//! Models physical airframe responses including:
//! - Aeroelastic wing bending as a second-order damped harmonic oscillator driven by normal G-load
//! - Progressive roll and pitch deflection across 6 trailing-edge flaps
//! - Canted V-tail elevator and rudder mixing on 2 tail fins
//! - Continuous turbine rotor spin with spool-up dynamics
//! - Exhaust nozzle petal aperture dilation and thrust vectoring

use glam::{Mat4, Vec3};
use airframe::{
    flap_limit, BEND_LIMITS, ELEVATOR_LIMIT, FLEX_GUST_AMPLITUDE, PETAL_LIMITS,
};
use sim::flight::{Controls, SIM_STEP};

/// Exponential critical-damping blend toward a target value.
#[inline]
pub fn damp(current: f32, target: f32, lambda: f32, dt: f32) -> f32 {
    current + (target - current) * (1.0 - (-lambda * dt).exp())
}

/// Evaluate a 3D point along the glider wing geometry.
///
/// Parameters:
/// - `side`: $-1.0$ for port (left), $+1.0$ for starboard (right)
/// - `t`: Spanwise parameter in $[0.0, 1.0]$ from root to wingtip
/// - `chord`: Chordwise parameter in $[0.0, 1.0]$ from leading edge to trailing edge
pub fn wing_point(side: f32, t: f32, chord: f32) -> Vec3 {
    airframe::wing_point(side, t, chord)
}

/// Evaluate the hinge pivot point for trailing edge flap `k` on the given wing `side`.
pub fn flap_pivot(side: f32, k: usize) -> Vec3 {
    airframe::flap_pivot(side, k)
}

/// Procedural animation state tracking physical deflections and turbine dynamics.
pub struct Anim {
    /// Engine spool RPM factor $[0.0, 1.0]$ driving thrust glow and rotor speed.
    pub spool: f32,
    /// Elapsed continuous animation time (seconds).
    pub time: f32,
    /// Wing structural bending deflection angle (radians) driven by G-load.
    pub bend: f32,
    /// Wing bending harmonic oscillation velocity ($\text{rad}/\text{s}$).
    pub bend_vel: f32,
    /// Trailing edge flap deflection angles for 6 control flaps.
    pub flaps: [f32; 6],
    /// Canted V-tail elevator/rudder deflection angles for port and starboard fins.
    pub elevators: [f32; 2],
    /// Cumulative turbine rotor spin angle (radians).
    pub rotor: f32,
    /// Articulation opening angles for 10 exhaust vectoring petals.
    pub petals: [f32; 10],
}

impl Anim {
    /// Initialize default neutral animation state.
    pub fn new() -> Self {
        Self {
            spool: 0.0,
            time: 0.0,
            bend: 0.0,
            bend_vel: 0.0,
            flaps: [0.0; 6],
            elevators: [0.0; 2],
            rotor: 0.0,
            petals: [0.12; 10],
        }
    }

    /// Advance physics-driven airframe animation states.
    ///
    /// Integrates:
    /// - Wing flex using sub-step Euler integration of a damped oscillator
    /// - Flap angles based on bank and pitch control demands
    /// - V-tail ruddervator mixing based on pitch and yaw control demands
    /// - Engine rotor rotation based on spool speed
    /// - Exhaust petal expansion
    pub fn step(&mut self, u: &Controls, load: f32, boost: f32, dt: f32) {
        self.time += dt;
        self.spool = damp(self.spool, boost, 4.0, dt);
        let target = ((load - 1.0) * 0.15).clamp(-0.4, 1.1);
        let steps = (dt / SIM_STEP).ceil().max(1.0) as usize;
        let h = dt / steps as f32;
        for _ in 0..steps {
            self.bend_vel += (45.0 * (target - self.bend) - 10.0 * self.bend_vel) * h;
            self.bend += self.bend_vel * h;
            self.bend = self.bend.clamp(BEND_LIMITS[0], BEND_LIMITS[1]);
        }
        for (j, flap) in self.flaps.iter_mut().enumerate() {
            let side = if j < 3 { -1.0 } else { 1.0 };
            let k = j % 3;
            let goal = u.bank * side * (0.22 + k as f32 * 0.04) - u.pitch * 0.07;
            *flap = damp(
                *flap,
                goal.clamp(-flap_limit(k), flap_limit(k)),
                12.0 - k as f32 * 2.0,
                dt,
            );
        }
        for (i, elev) in self.elevators.iter_mut().enumerate() {
            let rudder = if i == 1 { 1.0 } else { -1.0 };
            *elev = damp(
                *elev,
                (-u.pitch * 0.23 + u.yaw * rudder * 0.16)
                    .clamp(-ELEVATOR_LIMIT, ELEVATOR_LIMIT),
                10.0,
                dt,
            );
        }
        self.rotor += (2.5 + self.spool * 14.0) * dt;
        for petal in self.petals.iter_mut() {
            *petal = damp(
                *petal,
                (0.12 + 0.30 * self.spool).clamp(PETAL_LIMITS[0], PETAL_LIMITS[1]),
                8.0,
                dt,
            );
        }
    }

    /// Normalized dynamic pressure factor based on true airspeed.
    #[inline]
    pub fn pressure(speed: f32) -> f32 {
        (speed / 100.0).min(1.0)
    }

    /// Compute the local $4 \times 4$ transformation matrix for kinematic node `node` $\in [0, 22]$.
    pub fn node_matrix(&self, node: usize) -> Mat4 {
        airframe::node_matrix(node, &self.flaps, &self.elevators, self.rotor, &self.petals)
    }

    /// Absolute aircraft model root matrix (centered at world zero) for emitter simulation.
    pub fn model_abs(pose: &sim::flight::Pose) -> Mat4 {
        let rotation = Mat4::from_quat(pose.orientation);
        Mat4::from_translation(Vec3::new(pose.x, pose.y, pose.z)) * rotation
    }

    /// Exhaust exit follows animated petal tips, returning `(center, radius)`.
    pub fn nozzle_exit(&self) -> (Vec3, f32) {
        let tips: [Vec3; 10] = std::array::from_fn(|i| {
            self.node_matrix(11 + i)
                .transform_point3(Vec3::new(0.0, 0.0, -0.63))
        });
        let center = tips.iter().copied().sum::<Vec3>() / 10.0;
        let radius = tips.iter().map(|p| p.distance(center)).sum::<f32>() / 10.0;
        (center, radius)
    }

    /// Wing emitter tip position accounting for wing aeroelastic bend and aero flutter.
    pub fn wing_emitter(&self, side: f32, speed: f32) -> Vec3 {
        let span: f32 = 1.0;
        let mut p = wing_point(side, span, 1.0);
        p.z = -p.z; // Mesh-space Z flip matching airframe Part::vert convention
        let t = self.time;
        let gust = (t * 5.1 - span * 3.0 + side).sin() * 0.65
            + (t * 8.3 - span * 5.0).sin() * 0.35;
        p.y += self.bend * span * span
            + Self::pressure(speed) * FLEX_GUST_AMPLITUDE * span.powi(3) * gust;
        p
    }

    /// World-space emitter positions and direction vectors for the 5 aerodynamic vapor FX sources.
    ///
    /// Indices match `sim::effects::EMITTER_*`:
    /// 0. Engine exhaust nozzle
    /// 1. Port wingtip
    /// 2. Starboard wingtip
    /// 3. Port flap trailing edge
    /// 4. Starboard flap trailing edge
    pub fn emitter_world(&self, pose: &sim::flight::Pose) -> ([Vec3; 5], [Vec3; 5]) {
        let model = Self::model_abs(pose);
        let fwd = (model * glam::Vec4::new(0.0, 0.0, 1.0, 0.0))
            .truncate()
            .normalize_or_zero();
        let back = -fwd;

        // Body and node transformations
        let tip_l = self.wing_emitter(-1.0, pose.speed);
        let tip_r = self.wing_emitter(1.0, pose.speed);
        let flap_edge = |side: f32| {
            let span: f32 = 0.885;
            let p = wing_point(side, span, 1.0) - flap_pivot(side, 2);
            let gust = (self.time * 5.1 - span * 3.0 + side).sin() * 0.65
                + (self.time * 8.3 - span * 5.0).sin() * 0.35;
            Vec3::new(
                p.x,
                p.y + self.bend * span * span
                    + Self::pressure(pose.speed) * FLEX_GUST_AMPLITUDE * span.powi(3) * gust,
                -p.z,
            )
        };
        let (nozzle_local, _) = self.nozzle_exit();
        let p_noz = model * nozzle_local.extend(1.0);
        let p_tl = model * self.node_matrix(2) * glam::Vec4::new(tip_l.x, tip_l.y, tip_l.z, 1.0);
        let p_tr = model * self.node_matrix(3) * glam::Vec4::new(tip_r.x, tip_r.y, tip_r.z, 1.0);
        let p_fl = model * self.node_matrix(6) * flap_edge(-1.0).extend(1.0);
        let p_fr = model * self.node_matrix(9) * flap_edge(1.0).extend(1.0);
        let pos = [
            Vec3::new(p_noz.x, p_noz.y, p_noz.z) / p_noz.w.max(1e-6),
            Vec3::new(p_tl.x, p_tl.y, p_tl.z) / p_tl.w.max(1e-6),
            Vec3::new(p_tr.x, p_tr.y, p_tr.z) / p_tr.w.max(1e-6),
            Vec3::new(p_fl.x, p_fl.y, p_fl.z) / p_fl.w.max(1e-6),
            Vec3::new(p_fr.x, p_fr.y, p_fr.z) / p_fr.w.max(1e-6),
        ];
        (pos, [back; 5])
    }
}

impl Default for Anim {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_anim_step_dynamics() {
        let mut anim = Anim::new();
        let controls = Controls {
            pitch: 0.5,
            bank: -0.5,
            yaw: 0.2,
            boost: true,
        };
        for _ in 0..60 {
            anim.step(&controls, 2.5, 1.0, 1.0 / 60.0);
        }
        assert!(anim.spool > 0.5, "spool={}", anim.spool);
        assert!(anim.rotor > 0.0, "rotor={}", anim.rotor);
        assert!(anim.bend.is_finite());
        for flap in anim.flaps {
            assert!(flap.is_finite());
        }
        for elev in anim.elevators {
            assert!(elev.is_finite());
        }
        for petal in anim.petals {
            assert!(petal.is_finite());
        }
    }

    #[test]
    fn test_emitter_world_finite() {
        let anim = Anim::new();
        let pose = sim::flight::Pose::start();
        let (pos, dir) = anim.emitter_world(&pose);
        assert_eq!(pos.len(), 5);
        assert_eq!(dir.len(), 5);
        for p in pos {
            assert!(p.is_finite(), "emitter pos not finite: {:?}", p);
        }
        for d in dir {
            assert!(d.is_finite(), "emitter dir not finite: {:?}", d);
            assert!((d.length() - 1.0).abs() < 1e-4, "dir not unit: {:?}", d);
        }
    }
}
