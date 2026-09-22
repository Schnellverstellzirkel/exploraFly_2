#!/usr/bin/env python3
"""Rebuild the deterministic, loopable mono aircraft audio stems.

The source is intentionally broadband and noise-led. Engine stems emphasize
different parts of the fan/core/exhaust spectrum; boost, buffet, and structure
have distinct modulation and spectral balance. Discrete tones remain in the
runtime EngineVoice at low level.
"""

from __future__ import annotations

import math
import random
import sys
import wave
from array import array
from pathlib import Path


RATE = 48_000
SECONDS = 6
FRAMES = RATE * SECONDS
XFADE = 256  # Match SampleLayer's tail-to-head crossfade length
DEST = Path(__file__).resolve().parents[2] / "crates" / "sim" / "assets" / "audio"


def coefficients(cutoffs: tuple[float, ...]) -> tuple[float, ...]:
    return tuple(1.0 - math.exp(-2.0 * math.pi * cutoff / RATE) for cutoff in cutoffs)


def color_noise(
    seed: int,
    cutoffs: tuple[float, ...],
    weights: tuple[float, ...],
    modulation: tuple[float, float, float],
    envelope_range: tuple[float, float],
) -> list[float]:
    """Shape seeded white noise into wide, overlapping turbulent bands."""
    rng = random.Random(seed)
    coeffs = coefficients(cutoffs)
    states = [0.0] * len(cutoffs)
    phases = (rng.random() * math.tau, rng.random() * math.tau, rng.random() * math.tau)
    mod_hz, flutter_hz, slow_hz = modulation
    env = 1.0
    env_target = 1.0
    env_hold = 0
    out: list[float] = []

    for i in range(FRAMES):
        white = rng.uniform(-1.0, 1.0)
        for j, alpha in enumerate(coeffs):
            states[j] += alpha * (white - states[j])
        bands = [states[0]]
        bands.extend(states[j] - states[j - 1] for j in range(1, len(states)))
        bands.append(white - states[-1])
        shaped = sum(weight * band for weight, band in zip(weights, bands))

        if env_hold == 0:
            env_target = rng.uniform(*envelope_range)
            env_hold = rng.randint(RATE // 12, RATE // 3)
        env_hold -= 1
        env += (env_target - env) * 0.00035

        t = i / RATE
        # Slow thrust breathing plus quiet blade-pass flutter; broadband energy
        # stays dominant and no narrow midrange oscillator defines the engine.
        breath = 0.91 + 0.065 * math.sin(math.tau * mod_hz * t + phases[0])
        flutter = 0.965 + 0.025 * math.sin(math.tau * flutter_hz * t + phases[1])
        slow = 0.98 + 0.02 * math.sin(math.tau * slow_hz * t + phases[2])
        out.append(shaped * env * breath * flutter * slow)
    return out


def normalize_pcm(samples: list[float], target_rms: float) -> bytes:
    rms = math.sqrt(sum(value * value for value in samples) / len(samples))
    peak = max(abs(value) for value in samples)
    gain = min(target_rms / max(rms, 1e-9), 0.82 / max(peak, 1e-9))
    values = array(
        "h",
        (
            max(-32768, min(32767, round(value * gain * 32767.0)))
            for value in samples
        ),
    )
    if sys.byteorder != "little":
        values.byteswap()
    return values.tobytes()


def write_loop(name: str, seed: int, cutoffs: tuple[float, ...],
               weights: tuple[float, ...], modulation: tuple[float, float, float],
               envelope_range: tuple[float, float], target_rms: float) -> None:
    samples = color_noise(seed, cutoffs, weights, modulation, envelope_range)

    # Bake the same raised-cosine tail-to-head crossfade used by SampleLayer.
    fade = XFADE
    for j in range(fade):
        w = 0.5 - 0.5 * math.cos(math.pi * j / (fade - 1))
        tail = FRAMES - fade + j
        samples[tail] = samples[tail] * (1.0 - w) + samples[j] * w

    pcm = normalize_pcm(samples, target_rms)
    DEST.mkdir(parents=True, exist_ok=True)
    path = DEST / f"{name}.wav"
    with wave.open(str(path), "wb") as wav:
        wav.setnchannels(1)
        wav.setsampwidth(2)
        wav.setframerate(RATE)
        wav.writeframes(pcm)
    print(f"{path.relative_to(DEST.parents[2])}: {len(pcm)} bytes PCM")


def main() -> None:
    # Wide overlapping bands deliberately avoid the focused 500 Hz whine that
    # made the previous procedural core resemble a household appliance.
    engine_cutoffs = (45.0, 120.0, 350.0, 1000.0, 3000.0, 9000.0)
    write_loop(
        "engine_low", 0x11001, engine_cutoffs,
        (3.8, 3.2, 2.8, 1.55, 0.60, 0.14, 0.03),
        (7.0, 31.0, 1.5), (0.78, 1.16), 0.24,
    )
    write_loop(
        "engine_mid", 0x22002, engine_cutoffs,
        (2.2, 3.0, 3.2, 2.5, 1.3, 0.35, 0.04),
        (9.0, 43.0, 1.0), (0.76, 1.18), 0.24,
    )
    write_loop(
        "engine_high", 0x33003, engine_cutoffs,
        (0.8, 1.3, 2.2, 3.2, 3.0, 1.3, 0.18),
        (11.0, 57.0, 2.0), (0.75, 1.17), 0.24,
    )
    write_loop(
        "boost", 0x44004, (30.0, 90.0, 240.0, 650.0, 1700.0, 5000.0),
        (4.2, 3.6, 2.5, 1.35, 0.55, 0.12, 0.02),
        (5.0, 19.0, 0.5), (0.68, 1.22), 0.23,
    )
    write_loop(
        "airframe_buffet", 0x55005, (22.0, 60.0, 160.0, 420.0, 900.0, 2400.0),
        (0.8, 3.8, 2.7, 1.0, 0.25, 0.05, 0.02),
        (8.5, 13.0, 1.0), (0.55, 1.32), 0.18,
    )
    write_loop(
        "structural_rattle", 0x66006, (200.0, 600.0, 1600.0, 4200.0, 10000.0, 18000.0),
        (0.05, 0.25, 1.1, 2.8, 2.2, 0.8, 0.2),
        (6.0, 37.0, 1.5), (0.45, 1.35), 0.12,
    )


if __name__ == "__main__":
    main()
