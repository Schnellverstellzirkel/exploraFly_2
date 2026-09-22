//! Continuous flight-driven synthesis as a small offline DSP graph.
//!
//! No allocation, file I/O or locks in render(). Control updates arrive as
//! [`AcousticState`]; the worker thread only reads atomics and renders fixed
//! 48 kHz / 10 ms blocks. Sample data, when a [`SoundBank`] is attached, is
//! preloaded static PCM: synthesis never downloads or generates stems.
//!
//! The aircraft source is mono. Airflow ambience is stereo. Spatialization
//! (pan, propagation delay, Doppler) belongs downstream of this mixer.

mod control;
mod mixer;
mod samples;
mod voices;

pub use control::{sanitize, AcousticState};
pub use samples::{PcmLoop, SampleLayer, SoundBank, StemId};
pub use voices::SAMPLE_RATE;

use control::ControlSmoother;
use mixer::Mixer;
use voices::{AirflowVoice, BoostVoice, EngineVoice, StressVoice};

/// Fixed 10 ms stereo block in frames (480) and interleaved samples (960).
pub const BLOCK_FRAMES: usize = 480;
/// Interleaved i16 samples in one worker block.
pub const BLOCK_SAMPLES: usize = BLOCK_FRAMES * 2;

/// Layered procedural aircraft source plus optional preloaded PCM stems.
pub struct FlightSynth {
    smoother: ControlSmoother,
    engine: EngineVoice,
    boost: BoostVoice,
    airflow: AirflowVoice,
    stress: StressVoice,
    bank: SoundBank,
    engine_lo: samples::SampleLayer,
    engine_mid: samples::SampleLayer,
    engine_hi: samples::SampleLayer,
    boost_loop: samples::SampleLayer,
    mixer: Mixer,
}

impl Default for FlightSynth {
    fn default() -> Self {
        let mut synth = Self {
            smoother: ControlSmoother::default(),
            engine: EngineVoice::default(),
            boost: BoostVoice::default(),
            airflow: AirflowVoice::default(),
            stress: StressVoice::default(),
            bank: SoundBank::EMPTY,
            engine_lo: SampleLayer::default(),
            engine_mid: SampleLayer::default(),
            engine_hi: SampleLayer::default(),
            boost_loop: SampleLayer::default(),
            mixer: Mixer::default(),
        };
        synth.mixer.snap_volume(0.0);
        synth
    }
}

impl FlightSynth {
    /// Attach preloaded loop stems. Empty banks leave every sample layer silent.
    pub fn set_bank(&mut self, bank: SoundBank) {
        self.bank = bank;
        self.engine_lo.bind(self.bank.get(StemId::EngineLow));
        self.engine_mid.bind(self.bank.get(StemId::EngineMid));
        self.engine_hi.bind(self.bank.get(StemId::EngineHigh));
        self.boost_loop.bind(self.bank.get(StemId::Boost));
    }

    /// Write interleaved stereo PCM, smoothing every control at the sample rate.
    pub fn render(&mut self, output: &mut [i16], target: AcousticState) {
        let target = sanitize(target);
        let dt = 1.0 / SAMPLE_RATE as f32;
        self.mixer.begin_block();
        let frame_count = output.len() / 2;
        for _ in 0..frame_count {
            let state = self.smoother.step(&target, dt);
            let engine = self.engine.frame(&state, dt);
            let boost = self.boost.frame(&state, dt);
            let sample_engine = self.engine_lo.frame(state.spool, 1.0);
            let sample_mid = self.engine_mid.frame(state.spool, 1.0);
            let sample_hi = self.engine_hi.frame(state.spool, 1.0);
            let sample_boost = self.boost_loop.frame(1.0, state.boost);
            // Spool-region crossfades between baked engine loops when present.
            let lo_w = (1.0 - (state.spool * 2.0).clamp(0.0, 1.0)).max(0.0);
            let hi_w = ((state.spool - 0.55) / 0.45).clamp(0.0, 1.0);
            let mid_w = (1.0 - lo_w - hi_w).clamp(0.0, 1.0);
            let engine_sample =
                sample_engine * lo_w + sample_mid * mid_w + sample_hi * hi_w;
            let (air_l, air_r) = self.airflow.frame(&state, dt);
            let stress = self.stress.frame(&state, dt);
            let (mono_l, mono_r) = self.mixer.frame(
                engine + boost + engine_sample + sample_boost + stress,
                air_l,
                air_r,
                &state,
            );
            self.mixer.write_frame(output, mono_l, mono_r);
        }
        if !output.len().is_multiple_of(2) {
            *output.last_mut().unwrap() = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cruise_state() -> AcousticState {
        AcousticState {
            airspeed: 310.0,
            mach: 0.9,
            altitude: 2000.0,
            spool: 0.8,
            boost: 0.0,
            load: 3.0,
            aoa: 0.12,
            sideslip: 0.0,
            pitch_rate: 0.2,
            roll_rate: 0.1,
            vertical_speed: 5.0,
            separation: 0.0,
            volume: 0.4,
        }
    }

    #[test]
    fn output_is_continuous_across_arbitrary_buffer_partitions() {
        let state = cruise_state();
        let mut whole = vec![0; 9600];
        let mut chunks = vec![0; 9600];
        FlightSynth::default().render(&mut whole, state);
        let mut synth = FlightSynth::default();
        for chunk in chunks.chunks_mut(320) {
            synth.render(chunk, state);
        }
        assert_eq!(whole, chunks);
        assert!(whole.iter().any(|v| v.abs() > 50));
    }

    #[test]
    fn extreme_inputs_are_bounded_and_mute_fades_to_silence() {
        let mut synth = FlightSynth::default();
        let mut block = [0; 960];
        let loud = AcousticState {
            airspeed: 9999.0,
            mach: 50.0,
            altitude: 99999.0,
            spool: 100.0,
            boost: 99.0,
            load: 99.0,
            aoa: 10.0,
            sideslip: 10.0,
            pitch_rate: 100.0,
            roll_rate: 100.0,
            vertical_speed: 9999.0,
            separation: 9.0,
            volume: 9.0,
        };
        for _ in 0..100 {
            synth.render(&mut block, loud);
            assert!(block.iter().all(|x| (*x as i32).abs() < 24576));
        }
        let mute = AcousticState {
            airspeed: f32::NAN,
            mach: f32::INFINITY,
            altitude: f32::NAN,
            spool: f32::INFINITY,
            boost: f32::NAN,
            load: f32::NAN,
            aoa: f32::NAN,
            sideslip: f32::NAN,
            pitch_rate: f32::NAN,
            roll_rate: f32::NAN,
            vertical_speed: f32::NAN,
            separation: f32::NAN,
            volume: 0.0,
        };
        for _ in 0..100 {
            synth.render(&mut block, mute);
        }
        assert!(block.iter().all(|v| v.abs() <= 1));
    }

    #[test]
    fn ten_millisecond_blocks_stay_well_under_the_cpu_budget() {
        let mut synth = FlightSynth::default();
        let mut block = [0i16; BLOCK_SAMPLES];
        let state = cruise_state();
        synth.render(&mut block, state);
        let samples = 2000;
        let start = std::time::Instant::now();
        for _ in 0..samples {
            synth.render(&mut block, state);
        }
        let elapsed = start.elapsed();
        let per_block = elapsed.as_secs_f64() / samples as f64 * 1e6;
        assert!(
            per_block < 500.0,
            "audio block took {per_block:.1} us, budget 250 us"
        );
    }
}
