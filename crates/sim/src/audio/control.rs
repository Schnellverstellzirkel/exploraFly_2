//! Control surface and per-sample smoothing for the audio graph.

/// Full aircraft acoustics published by the sim each frame.
///
/// Replaces the old `{airspeed, spool, load, volume}` surface. Values arrive
/// once per control period and are smoothed at the sample rate inside
/// [`super::FlightSynth::render`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AcousticState {
    /// True airspeed in m/s.
    pub airspeed: f32,
    /// Mach number (dimensionless).
    pub mach: f32,
    /// Altitude above mean sea level in metres.
    pub altitude: f32,
    /// Engine spool 0..1.
    pub spool: f32,
    /// Afterburner/boost demand 0..1.
    pub boost: f32,
    /// Wing load in g.
    pub load: f32,
    /// Angle of attack in radians.
    pub aoa: f32,
    /// Sideslip angle in radians.
    pub sideslip: f32,
    /// Body pitch rate in rad/s.
    pub pitch_rate: f32,
    /// Body roll rate in rad/s.
    pub roll_rate: f32,
    /// Vertical speed in m/s (positive up).
    pub vertical_speed: f32,
    /// Flow separation 0..1 (buffet drive from the flight model).
    pub separation: f32,
    /// Master gain 0..1 (mute ramps this to zero).
    pub volume: f32,
}

impl Default for AcousticState {
    fn default() -> Self {
        Self {
            airspeed: 70.0,
            mach: 0.2,
            altitude: 1500.0,
            spool: 0.15,
            boost: 0.0,
            load: 1.0,
            aoa: 0.02,
            sideslip: 0.0,
            pitch_rate: 0.0,
            roll_rate: 0.0,
            vertical_speed: 0.0,
            separation: 0.0,
            volume: 0.35,
        }
    }
}

fn finite(value: f32, fallback: f32, min: f32, max: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}

/// Clamp every field into a safe flight envelope. Non-finite values fall back
/// to the default so NaNs never reach a coefficient or gain.
pub fn sanitize(target: AcousticState) -> AcousticState {
    let d = AcousticState::default();
    AcousticState {
        airspeed: finite(target.airspeed, 0.0, 0.0, 1200.0),
        mach: finite(target.mach, 0.0, 0.0, 5.0),
        altitude: finite(target.altitude, 1500.0, 0.0, 25000.0),
        spool: finite(target.spool, 0.0, 0.0, 1.0),
        boost: finite(target.boost, 0.0, 0.0, 1.0),
        load: finite(target.load, 1.0, -4.0, 20.0),
        aoa: finite(target.aoa, d.aoa, -0.8, 0.8),
        sideslip: finite(target.sideslip, 0.0, -0.8, 0.8),
        pitch_rate: finite(target.pitch_rate, 0.0, -3.0, 3.0),
        roll_rate: finite(target.roll_rate, 0.0, -4.0, 4.0),
        vertical_speed: finite(target.vertical_speed, 0.0, -400.0, 400.0),
        separation: finite(target.separation, 0.0, 0.0, 1.0),
        volume: finite(target.volume, 0.0, 0.0, 1.0),
    }
}

/// One-pole exponential smoother for every acoustic control.
#[derive(Clone, Copy, Debug)]
pub struct ControlSmoother {
    state: AcousticState,
}

impl Default for ControlSmoother {
    fn default() -> Self {
        Self {
            // Start muted; volume ramps in when the worker receives a target.
            state: AcousticState {
                volume: 0.0,
                ..AcousticState::default()
            },
        }
    }
}

impl ControlSmoother {
    /// Advance one sample toward `target`.
    pub fn step(&mut self, target: &AcousticState, dt: f32) -> AcousticState {
        // ~35 ms general response; slower turbine spool and boost.
        let general = 1.0 - (-dt / 0.035).exp();
        let spool = 1.0 - (-dt / 0.18).exp();
        let boost = 1.0 - (-dt / 0.12).exp();
        let s = &mut self.state;
        s.airspeed += (target.airspeed - s.airspeed) * general;
        s.mach += (target.mach - s.mach) * general;
        s.altitude += (target.altitude - s.altitude) * general;
        s.spool += (target.spool - s.spool) * spool;
        s.boost += (target.boost - s.boost) * boost;
        s.load += (target.load - s.load) * general;
        s.aoa += (target.aoa - s.aoa) * general;
        s.sideslip += (target.sideslip - s.sideslip) * general;
        s.pitch_rate += (target.pitch_rate - s.pitch_rate) * general;
        s.roll_rate += (target.roll_rate - s.roll_rate) * general;
        s.vertical_speed += (target.vertical_speed - s.vertical_speed) * general;
        s.separation += (target.separation - s.separation) * general;
        s.volume += (target.volume - s.volume) * general;
        self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::SAMPLE_RATE;

    #[test]
    fn sanitize_replaces_non_finite_and_clamps_extremes() {
        let dirty = AcousticState {
            airspeed: f32::NAN,
            mach: f32::INFINITY,
            altitude: -10.0,
            spool: 100.0,
            boost: f32::NEG_INFINITY,
            load: 1e9,
            aoa: 50.0,
            sideslip: f32::NAN,
            pitch_rate: 1e9,
            roll_rate: f32::NAN,
            vertical_speed: 1e9,
            separation: 9.0,
            volume: 9.0,
        };
        let clean = sanitize(dirty);
        assert!(clean.airspeed.is_finite());
        assert_eq!(clean.mach, 0.0);
        assert!(clean.altitude >= 0.0);
        assert_eq!(clean.spool, 1.0);
        assert_eq!(clean.boost, 0.0);
        assert_eq!(clean.volume, 1.0);
        assert_eq!(clean.separation, 1.0);
        assert!(clean.aoa.abs() <= 0.8);
        assert!(clean.sideslip.is_finite());
    }

    #[test]
    fn smoother_approaches_target_without_overshoot() {
        let mut smooth = ControlSmoother::default();
        let target = sanitize(AcousticState {
            volume: 0.5,
            spool: 1.0,
            ..AcousticState::default()
        });
        let dt = 1.0 / SAMPLE_RATE as f32;
        let mut last = smooth.state.volume;
        for _ in 0..SAMPLE_RATE as usize / 2 {
            let s = smooth.step(&target, dt);
            assert!(s.volume >= last - 1e-6);
            last = s.volume;
        }
        assert!((last - 0.5).abs() < 0.05);
    }
}
