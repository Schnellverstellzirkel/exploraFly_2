//! Mono aircraft bus plus stereo ambience, soft-limited to i16.

use super::control::AcousticState;
use super::voices::SAMPLE_RATE;

/// Mixes the mono source equally into L/R and keeps airflow stereo.
#[derive(Clone, Debug)]
pub(crate) struct Mixer {
    volume: f32,
    cursor: usize,
}

impl Default for Mixer {
    fn default() -> Self {
        Self {
            volume: 0.0,
            cursor: 0,
        }
    }
}

impl Mixer {
    pub(crate) fn snap_volume(&mut self, volume: f32) {
        self.volume = volume;
    }

    /// One frame: returns left and right linear floats before i16 quantize.
    pub(crate) fn frame(
        &mut self,
        source: f32,
        air_l: f32,
        air_r: f32,
        state: &AcousticState,
    ) -> (f32, f32) {
        // Volume is already smoothed in ControlSmoother; track it here too so
        // the mixer can later insert a separate master ramp without another
        // smoother instance.
        self.volume = state.volume;
        // Mono aircraft source, equal power center. Ambience stays stereo.
        let src = source * self.volume;
        let left = src + air_l * self.volume;
        let right = src + air_r * self.volume;
        (left, right)
    }

    /// Soft-limit and write one interleaved stereo frame at `cursor`.
    pub(crate) fn write_frame(&mut self, output: &mut [i16], left: f32, right: f32) {
        if self.cursor + 2 > output.len() {
            return;
        }
        let limit = |x: f32| -> i16 {
            let limited = x / (1.0 + x.abs());
            (limited * i16::MAX as f32 * 0.75) as i16
        };
        output[self.cursor] = limit(left);
        output[self.cursor + 1] = limit(right);
        self.cursor += 2;
    }

    /// Reset the write cursor at the start of each render() call.
    pub(crate) fn begin_block(&mut self) {
        self.cursor = 0;
    }
}

/// Shared constant re-export site for block timing tests.
pub(crate) const _SAMPLE_RATE_ASSERT: u32 = SAMPLE_RATE;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_source_lands_equally_on_both_channels() {
        let mut mixer = Mixer::default();
        let state = AcousticState {
            volume: 0.5,
            ..AcousticState::default()
        };
        let (l, r) = mixer.frame(0.4, 0.0, 0.0, &state);
        assert_eq!(l, r);
        assert!((l - 0.2).abs() < 1e-6);
    }

    #[test]
    fn soft_limit_stays_inside_i16_range() {
        let mut mixer = Mixer::default();
        let mut block = [0i16; 4];
        mixer.begin_block();
        mixer.write_frame(&mut block, 100.0, -100.0);
        assert!(block.iter().all(|s| (*s as i32).abs() < 24576));
    }
}
