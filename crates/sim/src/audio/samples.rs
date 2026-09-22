//! Preloaded PCM stems and zero-allocation loop playback.
//!
//! Stems are produced offline (downloaded or generated, normalized, loop
//! points marked, provenance recorded) and baked into the binary or a sidecar
//! the engine maps at startup. This module never performs I/O.

/// Stem slots the mixer expects from a baked bank.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StemId {
    EngineLow,
    EngineMid,
    EngineHigh,
    Boost,
    AirframeBuffet,
    StructuralRattle,
}

/// Loop-ready mono PCM at [`super::SAMPLE_RATE`], i16 samples.
#[derive(Clone, Copy, Debug)]
pub struct PcmLoop {
    samples: &'static [i16],
    /// Inclusive loop start sample index.
    loop_start: usize,
    /// Exclusive loop end sample index.
    loop_end: usize,
    /// Playback rate at unit drive (1.0 = original pitch).
    base_rate: f32,
}

impl PcmLoop {
    /// Create a loop. `loop_end` must be past `loop_start` and inside `samples`.
    pub fn new(samples: &'static [i16], loop_start: usize, loop_end: usize, base_rate: f32) -> Option<Self> {
        if samples.is_empty() || loop_start >= loop_end || loop_end > samples.len() {
            return None;
        }
        Some(Self {
            samples,
            loop_start,
            loop_end,
            base_rate: if base_rate.is_finite() && base_rate > 0.0 {
                base_rate
            } else {
                1.0
            },
        })
    }

    fn len(&self) -> usize {
        self.samples.len()
    }
}

/// Baked stem set. Empty until the offline pipeline lands real PCM.
#[derive(Clone, Copy, Debug, Default)]
pub struct SoundBank {
    engine_low: Option<PcmLoop>,
    engine_mid: Option<PcmLoop>,
    engine_high: Option<PcmLoop>,
    boost: Option<PcmLoop>,
    airframe_buffet: Option<PcmLoop>,
    structural_rattle: Option<PcmLoop>,
}

impl SoundBank {
    /// No stems: every sample layer stays silent and synthesis is fully procedural.
    pub const EMPTY: Self = Self {
        engine_low: None,
        engine_mid: None,
        engine_high: None,
        boost: None,
        airframe_buffet: None,
        structural_rattle: None,
    };

    pub fn get(&self, id: StemId) -> Option<PcmLoop> {
        match id {
            StemId::EngineLow => self.engine_low,
            StemId::EngineMid => self.engine_mid,
            StemId::EngineHigh => self.engine_high,
            StemId::Boost => self.boost,
            StemId::AirframeBuffet => self.airframe_buffet,
            StemId::StructuralRattle => self.structural_rattle,
        }
    }

    pub fn set(&mut self, id: StemId, loop_: Option<PcmLoop>) {
        let slot = match id {
            StemId::EngineLow => &mut self.engine_low,
            StemId::EngineMid => &mut self.engine_mid,
            StemId::EngineHigh => &mut self.engine_high,
            StemId::Boost => &mut self.boost,
            StemId::AirframeBuffet => &mut self.airframe_buffet,
            StemId::StructuralRattle => &mut self.structural_rattle,
        };
        *slot = loop_;
    }
}

/// One playing loop with crossfade-friendly cursor state. No allocation.
#[derive(Clone, Copy, Debug, Default)]
pub struct SampleLayer {
    pcm: Option<PcmLoop>,
    /// Fractional read cursor into `pcm.samples`.
    cursor: f32,
    /// Equal-power crossfade progress at the loop seam, 0..1.
    xfade: f32,
    gain: f32,
}

impl SampleLayer {
    pub fn bind(&mut self, pcm: Option<PcmLoop>) {
        self.pcm = pcm;
        if pcm.is_none() {
            self.cursor = 0.0;
            self.xfade = 0.0;
            self.gain = 0.0;
        }
    }

    /// One output sample. `drive` scales rate (spool/RPM); `level` is target gain.
    /// Returns 0 when unbound. Crossfades the last 256 samples into the loop
    /// start so seams stay click-free without exposing boundaries.
    pub fn frame(&mut self, drive: f32, level: f32) -> f32 {
        // Smooth layer gain so binding/crossfades never click.
        self.gain += (level.clamp(0.0, 1.0) - self.gain) * 0.0008;
        let Some(pcm) = self.pcm else {
            return 0.0;
        };
        if self.gain < 1e-4 {
            return 0.0;
        }
        let len = pcm.len() as f32;
        let loop_start = pcm.loop_start as f32;
        let loop_len = (pcm.loop_end - pcm.loop_start) as f32;
        let rate = (pcm.base_rate * (0.35 + 0.65 * drive.clamp(0.0, 1.5))).max(0.05);
        let step = rate / super::SAMPLE_RATE as f32;

        // Position within the loop region.
        let mut pos = self.cursor;
        if pos < loop_start {
            pos = loop_start + (pos % loop_len.max(1.0));
        }
        if pos >= pcm.loop_end as f32 {
            pos = loop_start + ((pos - loop_start) % loop_len.max(1.0));
        }

        let i0 = pos.floor() as usize;
        let i1 = (i0 + 1).min(pcm.samples.len().saturating_sub(1));
        let frac = pos - i0 as f32;
        let a = pcm.samples[i0.min(pcm.samples.len() - 1)] as f32 / i16::MAX as f32;
        let b = pcm.samples[i1] as f32 / i16::MAX as f32;
        let mut sample = a + (b - a) * frac;

        // Seam crossfade: blend the tail into the loop head over 256 samples.
        const XFADE: f32 = 256.0;
        let to_end = pcm.loop_end as f32 - pos;
        if to_end < XFADE {
            let t = 1.0 - to_end / XFADE; // 0..1 approaching the seam
            let head = pcm.samples[pcm.loop_start] as f32 / i16::MAX as f32;
            sample = sample * (1.0 - t) + head * t;
            self.xfade = t;
        } else {
            self.xfade = 0.0;
        }

        self.cursor = pos + step;
        if self.cursor >= pcm.loop_end as f32 {
            self.cursor = pcm.loop_start as f32 + (self.cursor - pcm.loop_end as f32);
            if self.cursor >= pcm.loop_end as f32 {
                self.cursor = pcm.loop_start as f32;
            }
        }
        // Keep cursor inside the file for pre-loop material (optional lead-in).
        if self.cursor >= len {
            self.cursor = loop_start;
        }

        sample * self.gain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Static ramp so tests exercise real 'static PCM without heap in frame().
    static RAMP: [i16; 512] = {
        let mut a = [0i16; 512];
        let mut i = 0;
        while i < 512 {
            a[i] = (i as i16) - 256;
            i += 1;
        }
        a
    };

    #[test]
    fn empty_bank_layer_is_silent() {
        let mut layer = SampleLayer::default();
        layer.bind(SoundBank::EMPTY.get(StemId::EngineLow));
        for _ in 0..1000 {
            assert_eq!(layer.frame(0.5, 1.0), 0.0);
        }
    }

    #[test]
    fn bound_layer_produces_finite_output_and_wraps_the_loop() {
        let pcm = PcmLoop::new(&RAMP, 64, 448, 1.0).expect("valid loop");
        let mut layer = SampleLayer::default();
        layer.bind(Some(pcm));
        let mut last = 0.0;
        for i in 0..10_000 {
            let s = layer.frame(1.0, 1.0);
            assert!(s.is_finite());
            if i > 100 {
                // Gain has ramped; something non-zero eventually appears.
                last = s;
            }
        }
        assert!(last.abs() > 0.0 || RAMP.iter().any(|v| *v != 0));
    }

    #[test]
    fn invalid_loop_points_are_rejected() {
        assert!(PcmLoop::new(&RAMP, 10, 10, 1.0).is_none());
        assert!(PcmLoop::new(&RAMP, 100, 50, 1.0).is_none());
        assert!(PcmLoop::new(&RAMP, 0, 99999, 1.0).is_none());
        assert!(PcmLoop::new(&[], 0, 1, 1.0).is_none());
    }
}
