# Flight interface and sample-led aircraft audio — 2026-09-19, revised 2026-09-22

## Implementation

The instrument layer is an atlas-free bitmap overlay in the existing final
composite. It displays knots, altitude in metres, heading, terrain/roof clearance,
and engine spool. It uses two additional vec4 values (32 bytes) in the existing
per-image mapped uniform buffer, preserving its original 1792-byte prefix. Total
descriptor range is 1824 bytes. No extra draw, texture, descriptor, or command
buffer recording is needed. Text is applied after lens effects and tone mapping.
Panel branches restrict glyph work to small screen regions. Cost still needs
GPU measurement: putting UI in a composite shader is not automatically faster
than a dedicated UI pass on every architecture.

Controls: W/S or arrows pitch; A/D or arrows bank; Q/E yaw; Shift boost; P pause;
R reset to the initial spawn; H hide/show instruments; F1 help; M mute; F11
fullscreen; Escape quit. Repeat events do not toggle UI state repeatedly, and
losing focus clears held flight keys. Terrain clearance is a gentle safety floor,
not an impact/crash simulation.

Sound is a sample-first graph driven by `AcousticState`. It embeds six original,
deterministically generated WAV loops: low/mid/high spool engine, boost,
airframe buffet, and structural rattle. The audio worker decodes their PCM once
at startup; the engine stems carry an 88% mix share and boost stems 85% when
bound. The reproducible generator and asset notes live under
`crates/sim/assets/audio/`. Procedural voices add weak blade/compressor tones,
boost turbulence, stereo airflow hiss, and load/rate/separation stress. With
an empty bank the procedural path is the full fallback. `SampleLayer` advances
at `step = rate` so 48 kHz PCM plays at unit pitch; the loop seam crossfades the
last 256 source samples into the matching first 256 head samples. The aircraft
source is mono; airflow ambience is stereo. Control smoothing prevents abrupt
gain/frequency jumps. A fixed
10 ms stereo buffer feeds 48 kHz S16 PCM through dynamically loaded native
ALSA on its own thread. The render thread only publishes atomics. Missing
audio hardware/library disables playback without stopping the game. Partial
writes, nonblocking waits, and underruns are handled. Interrupted writes
retry; suspended devices use nonblocking resume attempts so shutdown remains
responsive. Spatialization (pan, propagation delay, Doppler) is deferred and
belongs downstream of the mixer.

Sources for this revision, accessed 2026-09-22:
- [Microsoft Flight Simulator: Engine Audio Setup](https://docs.flightsimulator.com/msfs2024/html/4_Sound/Aircraft_Audio/Engine_Audio_Setup.htm),
  current product documentation. Layered idle/turbine/throttle regions
  informed the spool-region sample crossfade design.
- [Rizzi and Sahai, Auralization of Air Vehicle Noise for Community Impact Assessment](https://ntrs.nasa.gov/citations/20200002351),
  NASA technical report, accessed 2026-09-22. Core-noise inventory classifies
  compressor and turbine as broadband plus discrete tones, combustor as
  broadband, and jet mixing and broadband shock noise as broadband. Informed
  the sample-first energy split: broadband stems dominant, discrete tones weak.
- [oddio](https://github.com/Ralith/oddio), open-source real-time audio
  library. Evaluated as a future spatialization dependency; not adopted yet.
- [Steam Audio](https://partner.steamgames.com/doc/features/steam_audio),
  Valve HRTF/propagation documentation. Explicitly not adopted for this
  project.

`EXPLORA_AUDIO=0` disables the worker, `EXPLORA_VOLUME=0.35` sets master volume
(0–1), and `EXPLORA_AUDIO_DEVICE` selects an ALSA device. Muting and pausing ramp
gain to silence. The native game remains Linux-targeted; Windows can build and
validate it in Docker, but this milestone is not a Windows renderer port.

`EXPLORA_HUD=0` starts with no visible overlay. Benchmark JSON records HUD flags,
requested sound, wind, and forced flight controls so comparisons can hold these
settings constant. Requested sound does not prove an audio device opened.

Generate a standalone scripted multi-regime WAV for listening on any OS
(about 27 seconds: idle, boost, Mach pass, hard-G, stall, sideslip, mute):

```sh
cargo run -p sim --example audio_preview -- /tmp/flight-preview.wav
```

A/B individual buses against the same script:

```sh
cargo run -p sim --example audio_preview -- /tmp/samples.wav samples
cargo run -p sim --example audio_preview -- /tmp/procedural.wav procedural
cargo run -p sim --example audio_preview -- /tmp/exhaust.wav exhaust
cargo run -p sim --example audio_preview -- /tmp/wind.wav wind
```

## Primary research and applicability

All sources accessed 2026-09-19 except the audio graph sources above
(accessed 2026-09-22).

- [Khronos: constant data in Vulkan](https://docs.vulkan.org/samples/latest/samples/performance/constant_data/README.html),
  current living documentation. Its comparisons show that the best uniform-data
  strategy depends on architecture. Reusing the existing mapped UBO suits this
  engine's prerecorded commands; replacing it with push constants would require
  recording changing values. This is an architectural choice, not a measured win.
- [Doerfler and Wyse, EUSIPCO 2026](https://arxiv.org/abs/2603.07584),
  submitted 2026-03-08, revised 2026-06-02. The paper's control-conditioned
  harmonic-plus-noise synthesis informed source separation and smooth control.
  This small synthesizer does **not** implement the paper's recording analysis,
  fitted fingerprints, dataset, or calibrated automotive engine model.
- [ALSA PCM interface](https://www.alsa-project.org/alsa-doc/alsa-lib/group___p_c_m.html),
  current official API reference. Native playback uses interleaved signed 16-bit
  stereo, nonblocking write calls, and prepare-after-underrun recovery. Library
  and device are opened only by the audio worker.

## Verification and remaining acceptance

CPU tests cover identical PCM across buffer partitions, bounded extreme inputs,
mute fade, zero-allocation during `render()` (isolated integration test, both
empty and builtin banks), sample-layer unit-rate advancement and seam
crossfade, bus solo energy, builtin-bank mix difference, a generous 10 ms
block-time budget, sample-layer loop wrapping, sanitize clamping, finite HUD
inputs, heading wrap, unit conversion, and uniform size.
All compiled shader variants are checked with spirv-val. Real GPU readability,
audio-device latency, subjective mix quality, and frame-time overhead remain
runtime acceptance items. Do not infer 1000 FPS or visual approval from these tests.
