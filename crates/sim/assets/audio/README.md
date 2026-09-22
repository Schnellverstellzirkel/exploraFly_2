# Built-in aircraft audio stems

The engine bed now comes from a NASA acoustic test recording, rather than
synthetic colored noise. The captured jet-mixing sound supplies five
rate-shifted spool points (20%, 40%, 60%, 80%, and 100%); the mixer linearly
interpolates adjacent points. At 100%, the recording plays at its original
rate. These are authored operating-point variants of one recording, not five
independent measurements. The source is a single-microphone, model-scale
dual-stream jet test, not a full aircraft flyby or an isolated set of fan,
core, and afterburner recordings. The current mix therefore treats it as the
exhaust-mixing body. The generated playback-rate ratios are 0.776, 0.832,
0.888, 0.944, and 1.0, respectively. Subtle fan/compressor tones stay in
`EngineVoice`, and `BoostVoice` adds stochastic exhaust modulation.
Observer-angle filtering is not implemented yet.

The source WAV is checked in at
[`tools/audio/sources/nasa_jet_noise_mic28_2004.wav`](../../../../tools/audio/sources/nasa_jet_noise_mic28_2004.wav).
Rebuild all built-in stems with
[`tools/audio/generate_stems.py`](../../../../tools/audio/generate_stems.py).
The five engine loops are eight seconds long; buffet and structural-rattle
fallbacks are six seconds long. All generated loops are mono, signed 16-bit
PCM at 48 kHz. The 256-sample raised-cosine tail crossfade matches
`SampleLayer`'s runtime seam treatment. Only the airflow/buffet and structural
rattle effects use the script's locally seeded stochastic-noise generator.

## Recording provenance and use

- NASA Langley Applied Acoustics Branch, [Aircraft Flyover Simulation data
  page](https://stabserv.larc.nasa.gov/flyover/), lists Web Clip 3 as
  “Recorded jet noise” and identifies it as the same source used for NASA
  paper AIAA-2004-1029. Accessed 2026-09-22.
- Original audio file:
  [web_clip_03-mic28_short.wav](https://stabservdata.larc.nasa.gov/flyover/AIAA-2005-2983/web_clip_03-mic28_short.wav),
  10.0 seconds, mono, 16-bit PCM, 44.1 kHz. The corresponding NASA technical
  paper is [Temporal Characterization of Aircraft Noise
  Sources](https://ntrs.nasa.gov/api/citations/20040027959/downloads/20040027959.pdf),
  AIAA-2004-1029 (2004). Accessed 2026-09-22.
- NASA's [media usage
  guidelines](https://www.nasa.gov/nasa-brand-center/images-and-media/)
  state that NASA audio and other media are generally not subject to U.S.
  copyright, require NASA source acknowledgment, and must not imply NASA
  endorsement. The NASA flyover page showed no separate third-party credit for
  this clip when accessed; retain this attribution and re-check the source
  notice before redistributing the audio separately. Accessed 2026-09-22.

The NASA recording is kept unchanged as the source asset; the build script
trims its steady middle, rate-shifts the five spool variants, then normalizes
and resamples them into runtime loops. No NASA name, logo, or endorsement is
used in-game.
