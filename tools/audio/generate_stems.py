#!/usr/bin/env python3
"""Build loopable aircraft stems from recorded jet audio and FX noise beds.

The engine body uses a NASA jet-noise recording. The boost layer uses an
eight-second excerpt from a public-domain USAF F-16 burner-run video. Buffet
and structural-rattle fallback effects alone use locally seeded noise.
"""

from __future__ import annotations

import math
import random
import sys
import wave
from array import array
from pathlib import Path


RATE = 48_000
LOOP_SECONDS = 8
LOOP_FRAMES = RATE * LOOP_SECONDS
FX_SECONDS = 6
FX_FRAMES = RATE * FX_SECONDS
XFADE = 256  # Match SampleLayer's tail-to-head crossfade length
REPO_ROOT = Path(__file__).resolve().parents[2]
ENGINE_SOURCE = (
    Path(__file__).resolve().parent / "sources" / "nasa_jet_noise_mic28_2004.wav"
)
BOOST_SOURCE = (
    Path(__file__).resolve().parent / "sources" / "dvids_f16_burner_run_3-11s.wav"
)
DEST = REPO_ROOT / "crates" / "sim" / "assets" / "audio"

# The source clip represents one test condition, so these are explicit
# playback-rate approximations rather than independent measured spool takes.
ENGINE_POINTS = (
    (20, 0.776),
    (40, 0.832),
    (60, 0.888),
    (80, 0.944),
    (100, 1.000),
)
def read_mono_pcm16(path: Path) -> tuple[int, list[float]]:
    with wave.open(str(path), "rb") as wav:
        if (
            wav.getnchannels() != 1
            or wav.getsampwidth() != 2
            or wav.getcomptype() != "NONE"
        ):
            raise ValueError(f"expected mono PCM16 WAV source: {path}")
        rate = wav.getframerate()
        raw = wav.readframes(wav.getnframes())
    values = array("h")
    values.frombytes(raw)
    if sys.byteorder != "little":
        values.byteswap()
    return rate, [sample / 32768.0 for sample in values]


def rate_shifted_steady_loop(
    source: list[float], source_rate: int, rate: float
) -> list[float]:
    """Read a centered, steady section and alter its spectrum by playback rate."""
    source_step = source_rate / RATE * rate
    source_span = (LOOP_FRAMES - 1) * source_step
    start = (len(source) - source_span) * 0.5
    if start < 0.0:
        raise ValueError("recording is shorter than the requested engine loop")

    result: list[float] = []
    position = start
    last = len(source) - 1
    for _ in range(LOOP_FRAMES):
        index = int(position)
        fraction = position - index
        next_index = min(index + 1, last)
        result.append(source[index] + (source[next_index] - source[index]) * fraction)
        position += source_step
    return result


def stochastic_noise(
    seed: int,
    cutoffs: tuple[float, ...],
    weights: tuple[float, ...],
    envelope_range: tuple[float, float],
    burst_rate: float,
    frames: int,
) -> list[float]:
    """Create procedural flight-effect beds with irregular correlated motion."""
    rng = random.Random(seed)
    coefficients = tuple(
        1.0 - math.exp(-2.0 * math.pi * cutoff / RATE) for cutoff in cutoffs
    )
    states = [0.0] * len(cutoffs)
    env = 1.0
    env_target = 1.0
    env_hold = 0
    burst = 0.0
    out: list[float] = []

    for _ in range(frames):
        white = rng.uniform(-1.0, 1.0)
        for index, alpha in enumerate(coefficients):
            states[index] += alpha * (white - states[index])
        bands = [states[0]]
        bands.extend(states[index] - states[index - 1] for index in range(1, len(states)))
        bands.append(white - states[-1])
        shaped = sum(weight * band for weight, band in zip(weights, bands))

        if env_hold == 0:
            env_target = rng.uniform(*envelope_range)
            env_hold = rng.randint(RATE // 10, RATE // 2)
        env_hold -= 1
        env += (env_target - env) * 0.0002

        if rng.random() < burst_rate:
            burst = min(1.0, burst + rng.uniform(0.15, 0.55))
        burst *= 0.9992
        out.append(shaped * env * (1.0 + burst * 0.45))
    return out


def bake_crossfade(samples: list[float]) -> None:
    for index in range(XFADE):
        weight = 0.5 - 0.5 * math.cos(math.pi * index / (XFADE - 1))
        tail = len(samples) - XFADE + index
        samples[tail] = samples[tail] * (1.0 - weight) + samples[index] * weight


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


def write_loop(name: str, samples: list[float], target_rms: float) -> None:
    bake_crossfade(samples)
    pcm = normalize_pcm(samples, target_rms)
    DEST.mkdir(parents=True, exist_ok=True)
    path = DEST / f"{name}.wav"
    with wave.open(str(path), "wb") as wav:
        wav.setnchannels(1)
        wav.setsampwidth(2)
        wav.setframerate(RATE)
        wav.writeframes(pcm)
    print(f"{path.relative_to(REPO_ROOT)}: {len(pcm)} bytes PCM")


def main() -> None:
    source_rate, source = read_mono_pcm16(ENGINE_SOURCE)
    print(
        f"source: {ENGINE_SOURCE.relative_to(REPO_ROOT)} "
        f"({len(source) / source_rate:.3f}s at {source_rate} Hz)"
    )
    for spool, rate in ENGINE_POINTS:
        loop = rate_shifted_steady_loop(source, source_rate, rate)
        write_loop(f"engine_{spool}", loop, target_rms=0.23)

    boost_rate, boost_source = read_mono_pcm16(BOOST_SOURCE)
    if boost_rate != RATE or len(boost_source) != LOOP_FRAMES:
        raise ValueError(
            f"expected an {LOOP_SECONDS}s mono {RATE} Hz boost source: {BOOST_SOURCE}"
        )
    write_loop("boost", boost_source, target_rms=0.16)

    write_loop(
        "airframe_buffet",
        stochastic_noise(
            0x55005,
            (22.0, 60.0, 160.0, 420.0, 900.0, 2400.0),
            (0.8, 3.8, 2.7, 1.0, 0.25, 0.05, 0.02),
            (0.55, 1.32),
            0.00012,
            FX_FRAMES,
        ),
        target_rms=0.18,
    )
    write_loop(
        "structural_rattle",
        stochastic_noise(
            0x66006,
            (120.0, 360.0, 1000.0, 2800.0, 7000.0, 15000.0),
            (0.03, 0.16, 0.75, 1.5, 1.1, 0.32, 0.08),
            (0.42, 1.18),
            0.00042,
            FX_FRAMES,
        ),
        target_rms=0.12,
    )


if __name__ == "__main__":
    main()
