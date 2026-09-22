//! Per-sample procedural voices: engine harmonics, boost roar, airflow, stress.

use super::control::AcousticState;
use std::f32::consts::TAU;

pub const SAMPLE_RATE: u32 = 48_000;

/// Magic-circle quadrature oscillator: one recurrence yields sin and cos
/// without calling `sin()` per sample. Coeff is `2 * sin(pi * f / sr)`.
#[derive(Clone, Copy, Debug)]
struct QuadOsc {
    x: f32,
    y: f32,
    coeff: f32,
}

impl QuadOsc {
    fn new(freq: f32) -> Self {
        let mut osc = Self {
            x: 1.0,
            y: 0.0,
            coeff: 0.0,
        };
        osc.set_freq(freq);
        osc
    }

    fn set_freq(&mut self, freq: f32) {
        let freq = freq.clamp(0.0, SAMPLE_RATE as f32 * 0.45);
        self.coeff = 2.0 * (std::f32::consts::PI * freq / SAMPLE_RATE as f32).sin();
    }

    /// Advance one sample and return `(sin, cos)`.
    fn tick(&mut self) -> (f32, f32) {
        self.y -= self.coeff * self.x;
        self.x += self.coeff * self.y;
        // Soft renormalize keeps the circle from drifting over long runs.
        let energy = self.x * self.x + self.y * self.y;
        if !(0.98..=1.02).contains(&energy) {
            let inv = (1.0 / energy.max(1e-12)).sqrt();
            self.x *= inv;
            self.y *= inv;
        }
        (self.y, self.x)
    }
}

/// xorshift32 white noise in -1..1. Deterministic from a fixed seed.
#[derive(Clone, Copy, Debug)]
struct Noise {
    state: u32,
}

impl Noise {
    const fn new(seed: u32) -> Self {
        Self {
            state: if seed == 0 { 0x5ab193e7 } else { seed },
        }
    }

    fn tick(&mut self) -> f32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        (x as f64 / u32::MAX as f64 * 2.0 - 1.0) as f32
    }
}

/// One-pole low-pass coefficient for cutoff Hz.
fn lowpass_coeff(cutoff: f32) -> f32 {
    let cutoff = cutoff.clamp(10.0, SAMPLE_RATE as f32 * 0.45);
    1.0 - (-2.0 * std::f32::consts::PI * cutoff / SAMPLE_RATE as f32).exp()
}

/// Turbine fundamental plus two harmonics and a deep rumble bed.
#[derive(Clone, Debug)]
pub(crate) struct EngineVoice {
    tone: QuadOsc,
    rumble: QuadOsc,
    noise: Noise,
    low_noise: f32,
}

impl Default for EngineVoice {
    fn default() -> Self {
        Self {
            tone: QuadOsc::new(95.0),
            rumble: QuadOsc::new(27.0),
            noise: Noise::new(0x5ab193e7),
            low_noise: 0.0,
        }
    }
}

impl EngineVoice {
    pub(crate) fn frame(&mut self, state: &AcousticState, _dt: f32) -> f32 {
        let spool = state.spool;
        self.tone.set_freq(95.0 + 520.0 * spool);
        self.rumble.set_freq(27.0 + 31.0 * spool + 8.0 * state.mach);
        let (s1, c1) = self.tone.tick();
        // sin(2θ)=2sc, sin(3θ)=3s-4s³ from one quadrature pair.
        let s2 = 2.0 * s1 * c1;
        let s3 = s1 * (3.0 - 4.0 * s1 * s1);
        let (rumble, _) = self.rumble.tick();
        let white = self.noise.tick();
        let lp = lowpass_coeff(180.0 + 400.0 * spool);
        self.low_noise += (white - self.low_noise) * lp;
        let stress = ((state.load - 1.0).abs() / 8.0).clamp(0.0, 1.0);
        // Mach shifts spectral weight toward the upper harmonic (sonic boom bed).
        let mach_tilt = (state.mach / 1.2).clamp(0.0, 1.0);
        let tone = s1 * (0.10 + 0.04 * mach_tilt)
            + s2 * (0.035 + 0.03 * mach_tilt)
            + s3 * 0.012;
        tone * (0.22 + spool * 0.78)
            + self.low_noise * (0.30 + spool * 1.8)
            + rumble * (0.018 + stress * 0.045 + spool * 0.055)
    }
}

/// Boost/afterburner roar layered on top of the engine.
#[derive(Clone, Debug)]
pub(crate) struct BoostVoice {
    noise: Noise,
    band: f32,
    lp: f32,
}

impl Default for BoostVoice {
    fn default() -> Self {
        Self {
            noise: Noise::new(0x9e3779b9),
            band: 0.0,
            lp: 0.0,
        }
    }
}

impl BoostVoice {
    pub(crate) fn frame(&mut self, state: &AcousticState, _dt: f32) -> f32 {
        let boost = state.boost;
        if boost <= 0.001 {
            self.band *= 0.999;
            self.lp *= 0.999;
            return 0.0;
        }
        let white = self.noise.tick();
        // Two-pole-ish band: LP of noise then residual for a mid roar.
        let lp_c = lowpass_coeff(900.0 + 1200.0 * boost);
        self.lp += (white - self.lp) * lp_c;
        let hp = white - self.lp;
        self.band += (hp - self.band) * lowpass_coeff(3500.0);
        self.band * (0.15 + 0.55 * boost) * (0.4 + 0.6 * state.spool)
    }
}

/// Slipstream hiss: airspeed/Mach amplitude, sideslip L/R imbalance.
#[derive(Clone, Debug)]
pub(crate) struct AirflowVoice {
    noise_l: Noise,
    noise_r: Noise,
    prev_l: f32,
    prev_r: f32,
    lp_l: f32,
    lp_r: f32,
}

impl Default for AirflowVoice {
    fn default() -> Self {
        Self {
            noise_l: Noise::new(0x1234_5678),
            noise_r: Noise::new(0x8765_4321),
            prev_l: 0.0,
            prev_r: 0.0,
            lp_l: 0.0,
            lp_r: 0.0,
        }
    }
}

impl AirflowVoice {
    pub(crate) fn frame(&mut self, state: &AcousticState, _dt: f32) -> (f32, f32) {
        let speed = (state.airspeed / 650.0).clamp(0.0, 1.0);
        // Dynamic-pressure-like amplitude; altitude thins the bed slightly.
        let thin = 1.0 - (state.altitude / 22000.0).clamp(0.0, 1.0) * 0.35;
        let airflow = 0.26 * speed * speed * thin;
        // Mach brightens the hiss (one-pole HP opens with Mach).
        let hp_l = (0.10 + 0.25 * state.mach.clamp(0.0, 2.0)).min(0.6);
        let slip = state.sideslip.clamp(-0.5, 0.5);
        // Positive sideslip favors the right ear (flow from the left, etc.).
        let bal_l = (1.0 - slip * 0.8).clamp(0.2, 1.5);
        let bal_r = (1.0 + slip * 0.8).clamp(0.2, 1.5);
        let n_l = self.noise_l.tick();
        let n_r = self.noise_r.tick();
        self.lp_l += (n_l - self.lp_l) * 0.10;
        self.lp_r += (n_r - self.lp_r) * 0.10;
        let hiss_l = (n_l - self.prev_l) * bal_l;
        let hiss_r = (n_r - self.prev_r) * bal_r;
        self.prev_l += (n_l - self.prev_l) * hp_l;
        self.prev_r += (n_r - self.prev_r) * hp_l;
        // Keep a little shared low bed so the image is not pure decorrelation.
        let bed = (self.lp_l + self.lp_r) * 0.5;
        (
            (hiss_l + bed * 0.15) * airflow,
            (hiss_r + bed * 0.15) * airflow,
        )
    }
}

/// Structure: load-driven rumble, rate-driven creak, separation buffet.
#[derive(Clone, Debug)]
pub(crate) struct StressVoice {
    noise: Noise,
    buffet_phase: f32,
    creak_env: f32,
    band: f32,
}

impl Default for StressVoice {
    fn default() -> Self {
        Self {
            noise: Noise::new(0xdead_beef),
            buffet_phase: 0.0,
            creak_env: 0.0,
            band: 0.0,
        }
    }
}

impl StressVoice {
    pub(crate) fn frame(&mut self, state: &AcousticState, dt: f32) -> f32 {
        let stress = ((state.load - 1.0).abs() / 8.0).clamp(0.0, 1.0);
        let rate = (state.pitch_rate.abs() + state.roll_rate.abs()).clamp(0.0, 3.0);
        let white = self.noise.tick();

        // Buffet: low-rate AM of band noise driven by flow separation.
        let buffet_hz = 9.0 + 6.0 * state.separation;
        self.buffet_phase = (self.buffet_phase + buffet_hz * dt).fract();
        let am = (self.buffet_phase * TAU).sin() * 0.5 + 0.5;
        let buffet = state.separation.clamp(0.0, 1.0) * am * white * 0.12;

        // Creak: envelope follows |load-1| and body rates, high-passed noise.
        let drive = (stress * 0.7 + rate * 0.25).clamp(0.0, 1.0);
        self.creak_env += (drive - self.creak_env) * if drive > self.creak_env {
            0.002
        } else {
            0.0003
        };
        self.band += (white - self.band) * lowpass_coeff(2400.0);
        let creak = (white - self.band) * self.creak_env * 0.08;

        buffet + creak
    }
}
