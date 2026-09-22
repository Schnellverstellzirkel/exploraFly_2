# Built-in aircraft audio stems

The normal engine bed uses a NASA recorded jet-noise clip. The mixer crossfades
five rate-shifted versions of that single recording at 20%, 40%, 60%, 80%, and
100% spool. At 100%, it plays at the source rate; the ratios are 0.776, 0.832,
0.888, 0.944, and 1.0. These are authored playback variants, not five
independent engine measurements. The 88% sample share leaves a restrained 12%
procedural blade/compressor layer.

The boost bus uses a six-second excerpt from an audio-only F-4 Phantom
afterburner recording, with procedural BoostVoice turbulence modulating the
recorded engine bed. Airflow, buffet, and structural rattle remain procedural
effects.

This is a source-quality step, not a complete semantic engine bank. There are
still no separately recorded fan/intake or core stems, and no shock-noise layer
or observer-angle model. The NASA engine clip is a single-microphone,
model-scale dual-stream jet test; the F-16 excerpt is a mixed test-cell
recording. Treating their playback-rate variants as distinct measured spool
points would overstate the source data.

The source WAVs are checked in under
[`tools/audio/sources`](../../../../tools/audio/sources). Rebuild all built-in
stems with [`tools/audio/generate_stems.py`](../../../../tools/audio/generate_stems.py).
The five engine loops are eight seconds long, the boost loop is six seconds,
and buffet and structural-rattle fallbacks are six seconds long. All generated
loops are mono, signed 16-bit PCM at 48 kHz. The 256-sample raised-cosine tail
crossfade matches `SampleLayer`'s runtime seam treatment. Only the buffet and
structural-rattle effects use the script's locally seeded stochastic-noise
generator.

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
- The boost sample is from [Afterburner sound.wav by StoneyJ on
  Freesound](https://freesound.org/people/StoneyJ/sounds/104883/). The page
  describes a USAF F-4 Phantom taxi, engine run-up, and full-afterburner
  takeoff, and marks the upload CC0. It lists the original as 39.781 seconds,
  stereo 44.1 kHz 16-bit WAV. The checked-in source is the site's public
  high-quality MP3 preview decoded and downmixed to mono PCM16 at 44.1 kHz;
  the generator takes seconds 22–28 and resamples that six-second excerpt to
  48 kHz. Accessed 2026-09-22.

  The uploader's comment says they could not recall the original recording's
  source. Freesound displays the CC0 release, but independent chain-of-title
  details are unavailable; the sound is used as uploaded with that limitation.

## Source-model research

NASA's paper, [Auralization of a Supersonic Business Jet Using Advanced Takeoff
Procedures](https://ntrs.nasa.gov/citations/20200002602), published 2020-01-06,
publishes separate jet, core, fan-exhaust broadband, and fan-exhaust tonal
pseudorecordings. Its source levels vary with engine state and emission angle,
and the simulated propagation includes Doppler and atmospheric effects. The
broadband synthesis filters random noise to spectra from the model. This is
useful evidence for component roles and condition/directivity structure, but
the distributed waveforms are civil supersonic-business-jet flyover
auralizations, not isolated recordings of a fighter's fan, core, or
afterburner. They are not used as a replacement for the sampled engine
identity here. Accessed 2026-09-22.

No NASA name, logo, or endorsement is used in-game.
