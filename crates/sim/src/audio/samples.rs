//! Preloaded PCM stems and zero-allocation loop playback.
//!
//! Stems are produced offline (downloaded or generated, normalized, loop
//! points marked, provenance recorded) and baked into the binary or a sidecar
//! the engine maps at startup. This module never performs I/O.

/// Stem slots the mixer expects from a baked bank.
///
/// Engine assets are sampled exhaust-mixing beds at distinct spool points;
/// fan/compressor tones remain in the procedural voice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StemId {
    /// Recorded exhaust-mixing bed adjusted for 20% engine spool.
    Engine20,
    /// Recorded exhaust-mixing bed adjusted for 40% engine spool.
    Engine40,
    /// Recorded exhaust-mixing bed adjusted for 60% engine spool.
    Engine60,
    /// Recorded exhaust-mixing bed adjusted for 80% engine spool.
    Engine80,
    /// Recorded exhaust-mixing bed at the source recording's reference rate.
    Engine100,
    /// Compatibility alias for [`StemId::Engine20`].
    EngineLow,
    /// Compatibility alias for [`StemId::Engine60`].
    EngineMid,
    /// Compatibility alias for [`StemId::Engine100`].
    EngineHigh,
    /// Afterburner/boost roar loop.
    Boost,
    /// Airframe buffet loop (separation-driven).
    AirframeBuffet,
    /// Structural rattle loop (high-G / rates).
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
    pub fn new(
        samples: &'static [i16],
        loop_start: usize,
        loop_end: usize,
        base_rate: f32,
    ) -> Option<Self> {
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

    pub(crate) fn len(&self) -> usize {
        self.samples.len()
    }
}

/// Optional set of preloaded PCM stems.
#[derive(Clone, Copy, Debug, Default)]
pub struct SoundBank {
    engine: [Option<PcmLoop>; 5],
    boost: Option<PcmLoop>,
    airframe_buffet: Option<PcmLoop>,
    structural_rattle: Option<PcmLoop>,
}

impl SoundBank {
    /// No stems: every sample layer stays silent and synthesis is fully procedural.
    pub const EMPTY: Self = Self {
        engine: [None; 5],
        boost: None,
        airframe_buffet: None,
        structural_rattle: None,
    };

    pub fn get(&self, id: StemId) -> Option<PcmLoop> {
        if let Some(index) = engine_index(id) {
            return self.engine[index];
        }
        match id {
            StemId::Engine20
            | StemId::Engine40
            | StemId::Engine60
            | StemId::Engine80
            | StemId::Engine100
            | StemId::EngineLow
            | StemId::EngineMid
            | StemId::EngineHigh => unreachable!("engine stem mapped above"),
            StemId::Boost => self.boost,
            StemId::AirframeBuffet => self.airframe_buffet,
            StemId::StructuralRattle => self.structural_rattle,
        }
    }

    pub fn set(&mut self, id: StemId, loop_: Option<PcmLoop>) {
        if let Some(index) = engine_index(id) {
            self.engine[index] = loop_;
            return;
        }
        let slot = match id {
            StemId::Engine20
            | StemId::Engine40
            | StemId::Engine60
            | StemId::Engine80
            | StemId::Engine100
            | StemId::EngineLow
            | StemId::EngineMid
            | StemId::EngineHigh => unreachable!("engine stem mapped above"),
            StemId::Boost => &mut self.boost,
            StemId::AirframeBuffet => &mut self.airframe_buffet,
            StemId::StructuralRattle => &mut self.structural_rattle,
        };
        *slot = loop_;
    }

    /// True when any engine spool stem is bound.
    pub fn has_engine(&self) -> bool {
        self.engine.iter().any(Option::is_some)
    }
}

fn engine_index(id: StemId) -> Option<usize> {
    match id {
        StemId::Engine20 | StemId::EngineLow => Some(0),
        StemId::Engine40 => Some(1),
        StemId::Engine60 | StemId::EngineMid => Some(2),
        StemId::Engine80 => Some(3),
        StemId::Engine100 | StemId::EngineHigh => Some(4),
        StemId::Boost | StemId::AirframeBuffet | StemId::StructuralRattle => None,
    }
}

/// One playing loop with crossfade-friendly cursor state. No allocation.
#[derive(Clone, Copy, Debug, Default)]
pub struct SampleLayer {
    pcm: Option<PcmLoop>,
    /// Fractional read cursor into `pcm.samples`.
    cursor: f32,
    gain: f32,
}

impl SampleLayer {
    pub fn bind(&mut self, pcm: Option<PcmLoop>) {
        self.pcm = pcm;
        // Always start a newly bound stem at the loop head.
        self.cursor = pcm.map_or(0.0, |p| p.loop_start as f32);
        if pcm.is_none() {
            self.gain = 0.0;
        }
    }

    pub fn is_bound(&self) -> bool {
        self.pcm.is_some()
    }

    /// Test/inspection helper: source-sample cursor.
    #[cfg(test)]
    pub(crate) fn cursor_for_test(&self) -> f32 {
        self.cursor
    }

    /// One output sample. `drive` scales rate (spool/RPM); `level` is target gain.
    ///
    /// `cursor` is a PCM sample index, so at unit rate each output frame
    /// advances one source sample (`step = rate`), not `rate / SAMPLE_RATE`.
    /// The last 256 source samples crossfade into the matching first 256 head
    /// samples; after the seam the cursor continues at `loop_start + XFADE`
    /// so the already-heard head is not replayed as a jump.
    pub fn frame(&mut self, drive: f32, level: f32) -> f32 {
        self.gain += (level.clamp(0.0, 1.0) - self.gain) * 0.0008;
        let Some(pcm) = self.pcm else {
            return 0.0;
        };
        if self.gain < 1e-4 {
            return 0.0;
        }
        let len = pcm.len();
        let loop_start = pcm.loop_start;
        let loop_end = pcm.loop_end;
        let loop_len = loop_end.saturating_sub(loop_start).max(1);
        let xfade = 256.min(loop_len / 4).max(1);
        let tail_start = (loop_end - xfade) as f32;

        // Rate in source samples per output sample. Same-rate 48 kHz PCM
        // advances 1:1 at unit drive.
        let rate = (pcm.base_rate * (0.35 + 0.65 * drive.clamp(0.0, 1.5))).max(0.05);
        let step = rate;

        let mut pos = self.cursor;
        if !(loop_start as f32..loop_end as f32).contains(&pos) {
            let offset = pos.rem_euclid(loop_len as f32);
            pos = loop_start as f32 + offset.min(loop_len as f32 - 1e-4);
        }

        let sample_at = |p: f32| -> f32 {
            let i0 = p.floor() as usize;
            let i1 = (i0 + 1).min(len.saturating_sub(1));
            let frac = p - i0 as f32;
            let a = pcm.samples[i0.min(len - 1)] as f32 / i16::MAX as f32;
            let b = pcm.samples[i1] as f32 / i16::MAX as f32;
            a + (b - a) * frac
        };

        let mut sample = sample_at(pos);

        // Crossfade tail into the aligned head region (first `xfade` samples).
        if pos >= tail_start {
            let d = pos - tail_start;
            let w = (d / xfade as f32).clamp(0.0, 1.0);
            let head_pos = loop_start as f32 + d;
            let head = sample_at(head_pos.min((loop_end - 1) as f32));
            sample = sample * (1.0 - w) + head * w;
        }

        self.cursor = pos + step;
        if self.cursor >= loop_end as f32 {
            let over = self.cursor - loop_end as f32;
            // Continuity: tail crossfade already mixed head[0..xfade], so resume
            // at loop_start + xfade rather than replaying the blended region.
            self.cursor = loop_start as f32 + xfade as f32 + over;
            if self.cursor >= loop_end as f32 {
                let offset = (self.cursor - loop_start as f32).rem_euclid(loop_len as f32);
                self.cursor = loop_start as f32 + offset;
            }
        }
        if self.cursor >= len as f32 {
            self.cursor = loop_start as f32;
        }

        sample * self.gain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                last = s;
            }
        }
        assert!(last.abs() > 0.0 || RAMP.iter().any(|v| *v != 0));
    }

    #[test]
    fn unit_rate_advances_one_source_sample_per_output_frame() {
        static FLAT: [i16; 48_000] = [4000; 48_000];
        let pcm = PcmLoop::new(&FLAT, 0, 48_000, 1.0).expect("valid");
        let mut layer = SampleLayer::default();
        layer.bind(Some(pcm));
        for _ in 0..8000 {
            let _ = layer.frame(1.0, 1.0);
        }
        // Rebind resets cursor to loop_start (0).
        layer.bind(Some(pcm));
        assert_eq!(layer.cursor_for_test(), 0.0);
        for _ in 0..100 {
            let _ = layer.frame(1.0, 1.0);
        }
        let pos = layer.cursor_for_test();
        // step = base_rate * (0.35 + 0.65 * 1.0) = 1.0 at drive 1.
        assert!(
            (99.0..=101.0).contains(&pos),
            "cursor advanced to {pos}, expected ~100 (step must be rate, not rate/48000)"
        );
    }

    #[test]
    fn old_scale_would_freeze_and_new_scale_progresses() {
        // Regression for step = rate / SAMPLE_RATE: 48000 output frames would
        // advance only 1 source sample. With step = rate, 47000 frames at unit
        // rate advance nearly a full 48000-sample loop.
        static FLAT: [i16; 48_000] = [4000; 48_000];
        let pcm = PcmLoop::new(&FLAT, 0, 48_000, 1.0).expect("valid");
        let mut layer = SampleLayer::default();
        layer.bind(Some(pcm));
        for _ in 0..8000 {
            let _ = layer.frame(1.0, 1.0);
        }
        layer.bind(Some(pcm));
        for _ in 0..47_000 {
            let _ = layer.frame(1.0, 1.0);
        }
        let pos = layer.cursor_for_test();
        assert!(
            pos > 100.0,
            "cursor at {pos} indicates step is still scaled by 1/SAMPLE_RATE"
        );
    }

    #[test]
    fn seam_crossfade_pulls_head_energy_before_wrap() {
        // Head loud, rest quiet: during the tail region output must rise
        // from head content mixed in, not only after a hard wrap.
        let mut buf = vec![0i16; 4096];
        for (i, s) in buf.iter_mut().enumerate() {
            if i < 256 {
                *s = i16::MAX / 2;
            } else if (256..3840).contains(&i) {
                *s = 200;
            }
        }
        let owned = buf.into_boxed_slice();
        let samples: &'static [i16] = Box::leak(owned);
        let pcm = PcmLoop::new(samples, 0, 4096, 1.0).expect("valid");
        let mut layer = SampleLayer::default();
        layer.bind(Some(pcm));
        for _ in 0..8000 {
            let _ = layer.frame(1.0, 1.0);
        }
        layer.bind(Some(pcm));
        // Walk to just before the tail region (tail starts at 4096-256=3840).
        for _ in 0..3800 {
            let _ = layer.frame(1.0, 1.0);
        }
        let mut max_near_seam = 0.0f32;
        for _ in 0..300 {
            let s = layer.frame(1.0, 1.0).abs();
            max_near_seam = max_near_seam.max(s);
        }
        // Tail base is ~200/32767 ≈ 0.006; head is 0.5. Crossfade should
        // pull output well above the quiet tail floor before wrap.
        assert!(
            max_near_seam > 0.05,
            "seam crossfade max {max_near_seam} too low; head not blended"
        );
    }

    #[test]
    fn invalid_loop_points_are_rejected() {
        assert!(PcmLoop::new(&RAMP, 10, 10, 1.0).is_none());
        assert!(PcmLoop::new(&RAMP, 100, 50, 1.0).is_none());
        assert!(PcmLoop::new(&RAMP, 0, 99999, 1.0).is_none());
        assert!(PcmLoop::new(&[], 0, 1, 1.0).is_none());
    }
}
