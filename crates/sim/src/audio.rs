//! Continuous flight-driven synthesis. No allocation, file I/O or locks in render().
//! A stylized source model, not a calibrated recording of a particular aircraft.
use std::f32::consts::TAU;

pub const SAMPLE_RATE: u32 = 48_000;

#[derive(Clone, Copy, Debug)]
pub struct SoundState {
    pub airspeed: f32,
    pub spool: f32,
    pub load: f32,
    pub volume: f32,
}

impl Default for SoundState {
    fn default() -> Self {
        Self { airspeed: 70.0, spool: 0.15, load: 1.0, volume: 0.35 }
    }
}

fn finite(value: f32, fallback: f32, min: f32, max: f32) -> f32 {
    if value.is_finite() { value.clamp(min, max) } else { fallback }
}

pub struct FlightSynth {
    state: SoundState,
    phase: f32,
    rumble_phase: f32,
    noise_state: u32,
    low_noise: [f32; 2],
    previous_noise: [f32; 2],
}

impl Default for FlightSynth {
    fn default() -> Self {
        Self {
            state: SoundState { volume: 0.0, ..SoundState::default() },
            phase: 0.0, rumble_phase: 0.0, noise_state: 0x5ab193e7,
            low_noise: [0.0; 2], previous_noise: [0.0; 2],
        }
    }
}

impl FlightSynth {
    fn noise(&mut self) -> f32 {
        let mut x = self.noise_state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.noise_state = x;
        (x as f64 / u32::MAX as f64 * 2.0 - 1.0) as f32
    }

    /// Write interleaved stereo PCM, smoothing every control at the sample rate.
    pub fn render(&mut self, output: &mut [i16], target: SoundState) {
        let target = SoundState {
            airspeed: finite(target.airspeed, 0.0, 0.0, 1200.0),
            spool: finite(target.spool, 0.0, 0.0, 1.0),
            load: finite(target.load, 1.0, -4.0, 20.0),
            volume: finite(target.volume, 0.0, 0.0, 1.0),
        };
        let dt = 1.0 / SAMPLE_RATE as f32;
        // ~35ms control response, slower turbine spool-up, click-free gain.
        let smooth = 1.0 - (-dt / 0.035).exp();
        let spool_smooth = 1.0 - (-dt / 0.18).exp();
        for frame in output.as_chunks_mut::<2>().0 {
            self.state.airspeed += (target.airspeed - self.state.airspeed) * smooth;
            self.state.spool += (target.spool - self.state.spool) * spool_smooth;
            self.state.load += (target.load - self.state.load) * smooth;
            self.state.volume += (target.volume - self.state.volume) * smooth;
            let speed = (self.state.airspeed / 650.0).clamp(0.0, 1.0);
            let spool = self.state.spool;
            let stress = ((self.state.load - 1.0).abs() / 8.0).clamp(0.0, 1.0);
            self.phase = (self.phase + (95.0 + 520.0 * spool) * dt).fract();
            self.rumble_phase = (self.rumble_phase + (27.0 + 31.0 * spool) * dt).fract();
            let tone = (self.phase * TAU).sin() * 0.10
                + (self.phase * TAU * 2.0).sin() * 0.035
                + (self.phase * TAU * 3.0).sin() * 0.012;
            let rumble = (self.rumble_phase * TAU).sin();
            let airflow = 0.025 + 0.26 * speed * speed;
            for (channel, sample) in frame.iter_mut().enumerate() {
                let noise = self.noise();
                self.low_noise[channel] += (noise - self.low_noise[channel]) * (0.018 + spool * 0.05);
                // A one-pole high-pass makes the slipstream brighter than the turbine bed.
                let hiss = noise - self.previous_noise[channel];
                self.previous_noise[channel] += (noise - self.previous_noise[channel]) * 0.10;
                let engine = tone * (0.22 + spool * 0.78)
                    + self.low_noise[channel] * (0.30 + spool * 1.8)
                    + rumble * (0.018 + stress * 0.045 + spool * 0.055);
                let mixed = (engine + hiss * airflow) * self.state.volume;
                // Bounded soft saturation protects against unusual flight states.
                let limited = mixed / (1.0 + mixed.abs());
                *sample = (limited * i16::MAX as f32 * 0.75) as i16;
            }
        }
        if !output.len().is_multiple_of(2) { *output.last_mut().unwrap() = 0; }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_is_continuous_across_arbitrary_buffer_partitions() {
        let state = SoundState { airspeed: 310.0, spool: 0.8, load: 3.0, volume: 0.4 };
        let mut whole = vec![0; 9600];
        let mut chunks = vec![0; 9600];
        FlightSynth::default().render(&mut whole, state);
        let mut synth = FlightSynth::default();
        for chunk in chunks.chunks_mut(320) { synth.render(chunk, state); }
        assert_eq!(whole, chunks);
        assert!(whole.iter().any(|v| v.abs() > 50));
    }

    #[test]
    fn extreme_inputs_are_bounded_and_mute_fades_to_silence() {
        let mut synth = FlightSynth::default();
        let mut block = [0; 960];
        for _ in 0..100 {
            synth.render(&mut block, SoundState { airspeed: 9999.0, spool: 100.0, load: 99.0, volume: 9.0 });
            assert!(block.iter().all(|x| (*x as i32).abs() < 24576));
        }
        for _ in 0..100 {
            synth.render(&mut block, SoundState { airspeed: f32::NAN, spool: f32::INFINITY, load: f32::NAN, volume: 0.0 });
        }
        assert!(block.iter().all(|v| v.abs() <= 1));
    }
}
