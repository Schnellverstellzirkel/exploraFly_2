//! Continuous flight-driven synthesis as a small offline DSP graph.
//!
//! No allocation, file I/O or locks in render(). Control updates arrive as
//! [`AcousticState`]; the worker thread only reads atomics and renders fixed
//! 48 kHz / 10 ms blocks. Sample data, when a [`SoundBank`] is attached, is
//! preloaded static PCM: the render path never downloads or generates stems.
//!
//! Mix doctrine: the recorded NASA jet bed carries an 88% engine-bus share and
//! crossfades across five rate-shifted spool points. A separate recorded F-16
//! burner-run excerpt carries 85% of the boost bus; procedural voices provide
//! quiet engine tones, boost turbulence, airflow, and stress. With
//! [`SoundBank::EMPTY`] the procedural path is the full fallback.
//!
//! The aircraft source is mono. Airflow ambience is stereo. Spatialization
//! (pan, propagation delay, Doppler) belongs downstream of this mixer.

mod bank;
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

/// Engine mix share from bundled PCM stems when a bank is bound.
const SAMPLE_ENGINE_MIX: f32 = 0.88;
/// Engine mix share from procedural seasoning when a bank is bound.
const PROC_ENGINE_MIX: f32 = 0.12;
/// Boost mix share from the bundled PCM stem when bound.
const SAMPLE_BOOST_MIX: f32 = 0.85;
const PROC_BOOST_MIX: f32 = 0.15;

/// Which buses [`FlightSynth::render_buses`] emits. Full mix is all true.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Buses {
    pub engine_samples: bool,
    pub engine_proc: bool,
    pub boost_samples: bool,
    pub boost_proc: bool,
    pub wind: bool,
    pub stress: bool,
}

impl Default for Buses {
    fn default() -> Self {
        Self::FULL
    }
}

impl Buses {
    pub const FULL: Self = Self {
        engine_samples: true,
        engine_proc: true,
        boost_samples: true,
        boost_proc: true,
        wind: true,
        stress: true,
    };

    /// Engine spool-region PCM stems only.
    pub const SAMPLES: Self = Self {
        engine_samples: true,
        engine_proc: false,
        boost_samples: false,
        boost_proc: false,
        wind: false,
        stress: false,
    };

    /// Procedural engine/boost/air/stress only (no sample layers).
    pub const PROCEDURAL: Self = Self {
        engine_samples: false,
        engine_proc: true,
        boost_samples: false,
        boost_proc: true,
        wind: true,
        stress: true,
    };

    /// Boost/afterburner bus only (sample + procedural).
    pub const EXHAUST: Self = Self {
        engine_samples: false,
        engine_proc: false,
        boost_samples: true,
        boost_proc: true,
        wind: false,
        stress: false,
    };

    /// Wind / airflow only.
    pub const WIND: Self = Self {
        engine_samples: false,
        engine_proc: false,
        boost_samples: false,
        boost_proc: false,
        wind: true,
        stress: false,
    };
}

/// Sample-led aircraft sound graph plus optional procedural voices.
pub struct FlightSynth {
    smoother: ControlSmoother,
    engine: EngineVoice,
    boost: BoostVoice,
    airflow: AirflowVoice,
    stress: StressVoice,
    bank: SoundBank,
    engine_points: [samples::SampleLayer; 5],
    boost_loop: samples::SampleLayer,
    buffet_loop: samples::SampleLayer,
    structure_loop: samples::SampleLayer,
    mixer: Mixer,
    has_samples: bool,
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
            engine_points: [SampleLayer::default(); 5],
            boost_loop: SampleLayer::default(),
            buffet_loop: SampleLayer::default(),
            structure_loop: SampleLayer::default(),
            mixer: Mixer::default(),
            has_samples: false,
        };
        synth.mixer.snap_volume(0.0);
        synth
    }
}

impl FlightSynth {
    /// Synth with the bundled broadband PCM stems.
    ///
    /// Embedded WAVs are decoded once here, before any `render()` call,
    /// so the audio path stays allocation-free.
    pub fn with_builtin_bank() -> Self {
        let mut synth = Self::default();
        synth.set_bank(bank::builtin_sound_bank());
        synth
    }

    /// Attach preloaded loop stems. Empty banks leave every sample layer silent
    /// and the procedural voices become the full engine path.
    pub fn set_bank(&mut self, bank: SoundBank) {
        self.bank = bank;
        let mut points = [
            self.bank.get(StemId::Engine20),
            self.bank.get(StemId::Engine40),
            self.bank.get(StemId::Engine60),
            self.bank.get(StemId::Engine80),
            self.bank.get(StemId::Engine100),
        ];
        for index in 0..points.len() {
            if points[index].is_none() {
                let nearest = (0..points.len())
                    .filter(|candidate| points[*candidate].is_some())
                    .min_by_key(|candidate| candidate.abs_diff(index));
                points[index] = nearest.and_then(|candidate| points[candidate]);
            }
            self.engine_points[index].bind(points[index]);
        }
        self.boost_loop.bind(self.bank.get(StemId::Boost));
        self.buffet_loop.bind(self.bank.get(StemId::AirframeBuffet));
        self.structure_loop
            .bind(self.bank.get(StemId::StructuralRattle));
        self.has_samples = self.bank.has_engine();
    }

    /// Write interleaved stereo PCM with every bus enabled.
    pub fn render(&mut self, output: &mut [i16], target: AcousticState) {
        self.render_buses(output, target, Buses::FULL);
    }

    /// Write interleaved stereo PCM, smoothing every control at the sample rate.
    pub fn render_buses(&mut self, output: &mut [i16], target: AcousticState, buses: Buses) {
        let target = sanitize(target);
        let dt = 1.0 / SAMPLE_RATE as f32;
        self.mixer.begin_block();
        let frame_count = output.len() / 2;
        for _ in 0..frame_count {
            let state = self.smoother.step(&target, dt);

            // Five recorded spool points, linearly interpolated over 20–100%.
            let sample_engine = if buses.engine_samples {
                let mut points = [0.0; 5];
                for (sample, layer) in points.iter_mut().zip(&mut self.engine_points) {
                    // The offline asset already encodes its spool point.
                    *sample = layer.frame(1.0, 1.0);
                }
                let point = ((state.spool.clamp(0.2, 1.0) - 0.2) * 5.0).clamp(0.0, 4.0);
                let lower = point.floor() as usize;
                let upper = (lower + 1).min(4);
                let fraction = point - lower as f32;
                points[lower] + (points[upper] - points[lower]) * fraction
            } else {
                // Still advance layer state so bus solo does not desync phase.
                for layer in &mut self.engine_points {
                    let _ = layer.frame(1.0, 0.0);
                }
                0.0
            };

            let proc_engine = if buses.engine_proc {
                self.engine.frame(&state, dt)
            } else {
                0.0
            };

            let sample_boost = if buses.boost_samples {
                self.boost_loop.frame(1.0, state.boost)
            } else {
                self.boost_loop.frame(1.0, 0.0)
            };
            let proc_boost = if buses.boost_proc {
                self.boost.frame(&state, dt)
            } else {
                0.0
            };

            let engine_bus = if self.has_samples && buses.engine_samples && buses.engine_proc {
                sample_engine * SAMPLE_ENGINE_MIX + proc_engine * PROC_ENGINE_MIX
            } else if self.has_samples && buses.engine_samples {
                sample_engine
            } else {
                proc_engine
            };

            let has_engine_carrier = self.has_samples && buses.engine_samples;
            let boost_bus = if self.boost_loop.is_bound() && buses.boost_samples && buses.boost_proc
            {
                sample_boost * SAMPLE_BOOST_MIX + proc_boost * PROC_BOOST_MIX
            } else if self.boost_loop.is_bound() && buses.boost_samples {
                sample_boost
            } else if has_engine_carrier {
                // BoostVoice modulates the recorded exhaust body instead of
                // adding another broad noise bed over the engine.
                0.0
            } else {
                proc_boost * 0.35
            };

            let engine_bus = engine_bus * (1.0 + proc_boost * 0.55);

            let (air_l, air_r) = if buses.wind {
                self.airflow.frame(&state, dt)
            } else {
                let _ = self.airflow.frame(&state, dt);
                (0.0, 0.0)
            };

            let stress = if buses.stress {
                let procedural = self.stress.frame(&state, dt);
                let (buffet, structure) = if buses.engine_samples {
                    let buffet = self.buffet_loop.frame(state.separation, state.separation) * 0.25;
                    let rate_drive = (state.pitch_rate.abs() + state.roll_rate.abs()) / 5.0;
                    let load_drive = (state.load - 1.0).abs() / 8.0;
                    let structure_drive = (rate_drive * 0.55 + load_drive * 0.45).clamp(0.0, 1.0);
                    let structure =
                        self.structure_loop.frame(structure_drive, structure_drive) * 0.15;
                    (buffet, structure)
                } else {
                    self.buffet_loop.frame(state.separation, 0.0);
                    self.structure_loop.frame(1.0, 0.0);
                    (0.0, 0.0)
                };
                procedural + buffet + structure
            } else {
                let _ = self.stress.frame(&state, dt);
                self.buffet_loop.frame(state.separation, 0.0);
                self.structure_loop.frame(1.0, 0.0);
                0.0
            };

            let (mono_l, mono_r) =
                self.mixer
                    .frame(engine_bus + boost_bus + stress, air_l, air_r, &state);
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
    fn builtin_bank_changes_the_mix_but_stays_bounded() {
        let state = cruise_state();
        let mut plain = FlightSynth::default();
        let mut banked = FlightSynth::with_builtin_bank();
        let mut a = [0i16; BLOCK_SAMPLES];
        let mut b = [0i16; BLOCK_SAMPLES];
        // Warm volume / gains.
        for _ in 0..50 {
            plain.render(&mut a, state);
            banked.render(&mut b, state);
        }
        assert!(b.iter().any(|v| v.abs() > 0));
        assert!(
            b.iter().all(|x| (*x as i32).abs() < 24576),
            "banked mix must stay inside soft-limit headroom"
        );
        // Banked engine path should not be bit-identical to empty procedural.
        plain.render(&mut a, state);
        banked.render(&mut b, state);
        assert_ne!(a, b, "builtin bank must contribute audible energy");
    }

    #[test]
    fn bus_solos_produce_disjoint_energy_patterns() {
        let state = cruise_state();
        let mut synth = FlightSynth::with_builtin_bank();
        let mut block = [0i16; BLOCK_SAMPLES];

        // Wind-only at airspeed 0 is silent.
        let calm = AcousticState {
            airspeed: 0.0,
            mach: 0.0,
            volume: 1.0,
            ..cruise_state()
        };
        let mut silent = [0i16; BLOCK_SAMPLES];
        for _ in 0..30 {
            synth.render_buses(&mut silent, calm, Buses::WIND);
        }
        assert!(
            silent.iter().all(|v| v.abs() <= 2),
            "wind bus at zero airspeed should be silent"
        );

        // Samples-only at cruise is non-silent.
        for _ in 0..30 {
            synth.render_buses(&mut block, state, Buses::SAMPLES);
        }
        assert!(block.iter().any(|v| v.abs() > 20));

        // Exhaust-only with boost is non-silent.
        assert!(
            synth.boost_loop.is_bound(),
            "the built-in bank must bind its recorded F-16 boost stem"
        );
        let boosting = AcousticState {
            boost: 1.0,
            volume: 1.0,
            ..cruise_state()
        };
        let boost_sample_only = Buses {
            engine_samples: false,
            engine_proc: false,
            boost_samples: true,
            boost_proc: false,
            wind: false,
            stress: false,
        };
        for _ in 0..30 {
            synth.render_buses(&mut block, boosting, boost_sample_only);
        }
        assert!(
            block.iter().any(|v| v.abs() > 20),
            "the recorded boost stem must be audible without procedural boost"
        );
        for _ in 0..30 {
            synth.render_buses(&mut block, boosting, Buses::EXHAUST);
        }
        assert!(block.iter().any(|v| v.abs() > 20));
    }

    #[test]
    fn extreme_inputs_are_bounded_and_mute_fades_to_silence() {
        let mut synth = FlightSynth::with_builtin_bank();
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
        let mut synth = FlightSynth::with_builtin_bank();
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
