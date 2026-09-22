# Built-in aircraft audio stems

These six mono 48 kHz PCM loops are generated locally by
[tools/audio/generate_stems.py](../../../../tools/audio/generate_stems.py).
They are original, deterministic sound-design assets, not third-party
recordings. Re-run that script to rebuild them.

The three engine loops share a broadband turbulent-noise source and have
different low, middle, and high spectral balances for spool crossfades. Boost
adds slower exhaust breathing; buffet and structural rattle have their own
spectral balance and modulation. A 256-sample raised-cosine crossfade is baked
at the loop tail, matching the runtime SampleLayer seam crossfade. The
procedural engine voice contributes only a quiet tonal layer.

Each file is six seconds long, mono, signed 16-bit PCM at 48 kHz. The source
script uses fixed seeds and only Python's standard library.
