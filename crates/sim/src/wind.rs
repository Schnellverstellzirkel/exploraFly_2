//! Deterministic atmospheric flow in world coordinates (metres and seconds).
//!
//! The prevailing breeze strengthens and veers with altitude. Broad travelling
//! gusts add gentle shear and vertical motion without per-frame random impulses.
use glam::Vec3;

/// Maximum wind magnitude at unit strength, in metres per second.
pub const MAX_WIND_SPEED: f32 = 32.0;

#[derive(Clone, Copy, Debug)]
pub struct Wind {
    strength: f32,
}

impl Default for Wind {
    fn default() -> Self {
        Self::new(1.0)
    }
}

impl Wind {
    /// Scale the complete wind field. Zero is calm, one is a moderate breeze,
    /// and three is the strongest supported weather. Invalid values are calm.
    pub fn new(strength: f32) -> Self {
        Self {
            strength: if strength.is_finite() {
                strength.clamp(0.0, 3.0)
            } else {
                0.0
            },
        }
    }

    pub fn strength(&self) -> f32 {
        self.strength
    }

    /// World-space air velocity, sampled at absolute position and simulation
    /// time. No accumulated state: identical samples reproduce identical flow.
    pub fn velocity(&self, position: Vec3, time: f32) -> Vec3 {
        if self.strength == 0.0 || !position.is_finite() || !time.is_finite() {
            return Vec3::ZERO;
        }

        let altitude = position.y.clamp(0.0, 25000.0);
        let shear = altitude / (altitude + 1800.0);
        let heading = 1.0 - 0.28 * shear;
        let prevailing = Vec3::new(heading.sin(), 0.0, heading.cos()) * (4.0 + 12.0 * shear);

        // Kilometre-scale gusts evolve over tens of seconds. Different wave
        // directions avoid a repeating pulse along the aircraft's flight path.
        let broad = position.x * 0.0011 + position.z * 0.0007 - time * 0.11;
        let cross = position.x * -0.0008 + position.z * 0.0013 - time * 0.17 + 1.7;
        let ripple = position.x * 0.0023 + position.z * -0.0016 + time * 0.23 + 0.6;
        let gust_scale = 0.45 + 0.55 * shear;
        // Vertical gusts fade at ground level to avoid flow through the surface.
        let vertical_scale = altitude / (altitude + 350.0);
        let gust = Vec3::new(
            broad.sin() * 2.6 + cross.sin() * 1.2,
            (cross.sin() * 0.85 + ripple.sin() * 0.45) * vertical_scale,
            cross.cos() * 2.0 + ripple.sin() * 0.8,
        ) * gust_scale;

        (prevailing + gust).clamp_length_max(MAX_WIND_SPEED) * self.strength
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calm_and_invalid_samples_are_finite_and_stationary() {
        let position = Vec3::new(1200.0, 1500.0, -4000.0);
        for strength in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(Wind::new(strength).velocity(position, 42.0), Vec3::ZERO);
        }
        let wind = Wind::default();
        assert_eq!(wind.velocity(Vec3::splat(f32::NAN), 0.0), Vec3::ZERO);
        assert_eq!(wind.velocity(position, f32::INFINITY), Vec3::ZERO);
        assert_eq!(Wind::new(100.0).strength(), 3.0);
    }

    #[test]
    fn samples_are_repeatable_bounded_and_scale_with_strength() {
        let wind = Wind::default();
        for i in 0..400 {
            let time = i as f32 * 7.3;
            let position = Vec3::new(time * -75.0, i as f32 * 60.0, time * 42.0);
            let sample = wind.velocity(position, time);
            assert_eq!(sample, wind.velocity(position, time));
            assert!(sample.is_finite() && sample.length() <= MAX_WIND_SPEED);
            assert_eq!(Wind::new(3.0).velocity(position, time), sample * 3.0);
            assert!((wind.velocity(position, time + 0.01) - sample).length() < 0.02);
            assert!((wind.velocity(position + Vec3::ONE, time) - sample).length() < 0.03);
        }
    }

    #[test]
    fn prevailing_wind_strengthens_with_altitude_and_gusts_evolve() {
        let wind = Wind::default();
        let low = wind.velocity(Vec3::ZERO, 0.0);
        let high = wind.velocity(Vec3::Y * 11000.0, 0.0);
        assert_eq!(low.y, 0.0);
        assert!(high.length() > low.length() * 1.5);
        assert!((wind.velocity(Vec3::Y * 1500.0, 15.0)
            - wind.velocity(Vec3::Y * 1500.0, 0.0))
            .length() > 0.5);
    }
}
