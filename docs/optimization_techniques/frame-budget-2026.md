# One millisecond frame budget: September 2026

The goal is 1,000 complete rendered scenes submitted for presentation each second,
with both mean and p99 intervals at or below 1,000 microseconds. This is a target,
not a measured result of this change. The implementation was developed on a
Windows host without access to the target Linux RTX 4060 rendering session.
Unit tests and compilation do not establish GPU speed or visual equivalence.

## What the measurements mean

| Field | Observation | What it cannot prove |
| --- | --- | --- |
| `submission_interval` | Wall time between successful `vkQueuePresentKHR` returns, including simulation, GPU backpressure, and instrumentation | That the compositor displayed each image |
| `gpu_frame` | GPU timestamp interval covering TLAS/opaque/sky/ground/clouds/plume/trails/glass/composite for one complete scene | Presentation latency, CPU time, or display cadence |
| `display_interval` | Consecutive distinct `actualPresentTime` values from `VK_GOOGLE_display_timing`, when supported | Complete capture coverage when feedback is delayed or absent |
| Raster passes/s | Complete scene passes plus optional attachment-free burst workload | Complete rendered FPS; extra burst draws are not extra visible frames |

The report includes sample count, mean, p50, p95, p99, maximum, and the number of
intervals over 1 ms. Percentiles use nearest rank over exact nanosecond samples.
`submission_target_met` requires mean **and** p99 <= 1 ms. `display_target_met`
is `null` unless every measured present has distinct display feedback and at
least two display timestamps are available. A `null` is unverified, never a pass.
Mailbox may replace queued images; monitor refresh and compositor policy can
limit visible updates independently of GPU throughput. No frame generation is
included in the scene count.

With IMMEDIATE presentation, distinct display timings may refer to torn scanout
updates, not complete monitor refreshes. Even complete timing coverage is not
proof that a monitor displayed 1000 whole frames per second. Normal-play console
rates now use unclamped wall time; simulation delta clamping cannot inflate them
after a long stall.

The benchmark warms up for two seconds and then 500 successful present calls.
`--benchmark N` measures exactly N further present calls, regardless of burst
size. It restarts measurement after swapchain recreation. GPU results are read
only after the corresponding image fence; final outstanding queries are drained
after wall measurement ends. Optional display feedback is polled every 64
presents and at completion. All capture/poll costs during the measured loop are
included. A final GPU drain does not guarantee all display feedback has arrived.

## Reproduce on the rendering machine

Build the native Linux executable using the normal release profile, then run:

```sh
cargo build --release -p explora-engine
python3 tools/benchmark-frames.py --binary target/release/explora \
  --quality balanced --presents 10000 --runs 3 --fullscreen \
  --output benchmark-balanced.json
```

The runner enforces `EXPLORA_BURST=1`, records configuration and EXPLORA scene
environment variables, checks capture length, and requires every run to pass.
Use `--require display` to fail when the display goal is unmet **or unverified**.
Use `--present immediate`, `mailbox`, or `fifo` only when deliberately comparing
those modes; the engine rejects unsupported explicit modes. To capture one run
without the runner, use `EXPLORA_BENCH_JSON=run.json ... --benchmark 10000`.

Keep resolution, preset, scene/wind controls, driver, power state, and presentation
mode identical between baseline and candidate. Run alternating baseline/candidate
captures several times. Warm up clocks, disable validation for performance
captures, and also run validation separately for synchronization correctness.
Do not interpret a synthetic burst result as the full-scene target. The old
`EXPLORA_NO_GPU` diagnostic now fails at startup: its empty queue submit did not
perform the swapchain layout transition required for a valid new image present.

## Changes and evidence limits

The renderer already reuses command buffers and submits a complete batch once per
present. That matches NVIDIA and AMD guidance to avoid many small submissions;
adding command-recording workers to a loop which does not record commands would
not remove its current work. CPU uniform/simulation, GPU rendering, and display
work already overlap through the existing in-flight images. Measure the stage
breakdown before adding asynchronous compute or more queues.

Normal play now reads GPU queries once per 16 present calls. Set
`EXPLORA_GPU_READBACK_EVERY=1` for full diagnostic sampling or `0` to disable CPU
readback; benchmarks always sample every frame. GPU timestamps remain recorded.
This removes 15 of every 16 normal-play query-readback calls; the FPS benefit is
unmeasured. Recorded-command pools no longer request individual command-buffer
reset support, which they never use. No GPU speedup is claimed for that change.

The selected device's reported queue count limits queue creation. A single-queue
family shares rendering/presentation safely; two supported queues preserve the
existing schedule. Acquire semaphore reuse waits for the fence of its consuming
submission. Completed mappings are cleared before fence reuse. Optional present
ID/wait features and all ray-query extension dependencies are checked before
enabling them. The selected physical device is retained rather than rediscovered,
avoiding mismatched properties on machines with several NVIDIA GPUs. Timestamp
arithmetic honors the queue's valid counter bits. The renderer still targets a
discrete NVIDIA GPU and requires graphics timestamps; no AMD hardware support or
validation is claimed merely because vendor-neutral Vulkan is used.

`EXPLORA_RT_SHADOWS=auto` preserves the existing feature-gated default.
`on` requires support; `off` selects the existing analytic shadow path for an
explicit visual/performance comparison. It removes TLAS builds and ray-query
shadow work, but changes shadow quality. Balanced and cinematic defaults,
resolution, material shaders, and default ray-query selection are unchanged.

## Primary references checked on 2026-09-19

- NVIDIA, [Vulkan Dos and Don'ts](https://developer.nvidia.com/blog/vulkan-dos-donts/),
  updated 2025-01-14: command-buffer reuse, fewer submissions, precise barriers,
  and profiling in a stable performance environment.
- AMD GPUOpen, [RDNA Performance Guide](https://gpuopen.com/learn/rdna-performance-guide/):
  avoid small command buffers, minimize submission cost, and profile vendor-specific
  performance instead of assuming extra threading is useful.
- Khronos, [VkDeviceQueueCreateInfo](https://docs.vulkan.org/refpages/latest/refpages/source/VkDeviceQueueCreateInfo.html)
  and [VkQueueFamilyProperties](https://docs.vulkan.org/refpages/latest/refpages/source/VkQueueFamilyProperties.html):
  requested queues must fit the family, and timestamp width is a queue property.
- Khronos, [vkGetQueryPoolResults](https://docs.vulkan.org/refpages/latest/refpages/source/vkGetQueryPoolResults.html):
  fence completion prevents stale query reuse; host reads need not add a GPU wait.
- Khronos, [VK_GOOGLE_display_timing](https://docs.vulkan.org/refpages/latest/refpages/source/VK_GOOGLE_display_timing.html):
  display timing describes actual presentation, separately from queue submission.
- AMD GPUOpen, [Vulkan Barriers Explained](https://gpuopen.com/learn/vulkan-barriers-explained/):
  scope dependencies to the actual producer and consumer; do not remove required
  synchronization just to make CPU timings appear smaller.

These are implementation guidance and API semantics, not benchmarks of this
application. Newer publication dates alone do not establish suitability or speed.

## Pass-level GPU attribution and exact-zero terrain shading (2026-09-19 evening)

Environment: target RTX 4060 Laptop GPU, driver 580.173.02, Vulkan 1.3.275,
2880×1646 windowed, Balanced preset, `EXPLORA_BURST=1`, ray-traced shadows on,
MAILBOX present. Baseline captured 2026-09-19 ~19:54 local before the changes
below (2,500-present burst):

- present submissions 110.4/s; gpu_frame mean 9,014 µs (p50 9,172 µs)
- per-pass stamps: opaque+TLAS 377 µs, sky 799 µs, clouds 2,943 µs,
  ground 4,175 µs, plume 25 µs, composite 685 µs

The per-pass timestamps previously placed two adjacent stamps between the
terrain and plume passes, so sky, clouds, and ground were reported as one
~7.9 ms lump. `plane::GPU_STAMPS_PER_FRAME` (9 slots per swapchain image)
now attributes opaque(+TLAS), sky, clouds, ground, plume, trail, glass, and
composite separately in the `stages per present` line.

### Ground material gating (kept; bit-exact)

`ground.frag` evaluated ~15 value-noise octaves and ~10 photographic texture
fetches for every terrain fragment even when the consuming mix weight was
exactly zero (mid/far `micro_fade`/`rock_fade` distance fades, snow/rock/
outcrop/flower/edelweiss coverage gates). Every fetch now sits behind the
exact-zero mask of its only consumers, with defaults chosen so unfetched
values multiply by exactly zero or mix at weight zero. A frozen-scene capture
(`EXPLORA_FREEZE=1 EXPLORA_SHOT_FRAME=120`) against the pre-change binary is
bit-identical: 0 of 4.7 M pixels differ. Ground pass 4,175 → 3,957-3,979 µs
across three 3,000-present runs on the integrated tree; the split carries
roughly ±0.15 ms run-to-run noise.

### Depth-only terrain prepass (tested; rejected)

A depth-only terrain prepass ahead of the atmosphere passes early-Z-culled
every sky/cloud fragment hidden behind ridgelines (sky 799 → 221 µs), but the
prepass itself rasterizes ~2.1 M terrain triangles with 8×MSAA depth writes
and cost ≈ 1,060 µs. At the spawn scene the cloud slab sits almost entirely
above the ridgelines, so clouds saved nothing: net wash (gpu_frame 9,011 →
9,072 µs), pixel-identical output. Removed. Reordering the passes front-to-back
without a prepass would change the designed rule that mountains always paint
over the cloud volume, so it was not attempted.

### Sun-transmittance hoist to the vertex stage (tested; rejected)

Hoisting `tr_sun = atmoTransmittanceToTop(atmoModelOrigin(...), sun)` from the
sky fragment into `sky.vert` as a flat varying is mathematically a per-frame
constant, but VS/FS compiler rounding of the same expression diverges inside
the transmittance LUT lookup, and the solar disc multiplies the value by
`ATMO_SUN_INV_SOLID` before the composite veiling-glare gate reads it: the
sun disc and glare visibly change brightness. Saving only ≈ 80 µs of sky time
did not justify a visible change; reverted. An `ATMO_FRAGMENT_STAGE` define
that gates `fwidth` uses out of vertex-compiled includes was validated
(pixel-identical) and is available if a future vertex consumer needs the
shared atmosphere include.

### Integrated tree result (same evening, three 3,000-present runs)

With the parallel cloud re-architecture (procedural mesh puffs replacing the
fullscreen slab march) plus the changes above, gpu_frame mean is 5,972-6,053 µs
(clouds 99-101 µs, ground 3,957-3,979 µs, sky 800-811 µs, composite 694-703 µs,
opaque+TLAS 393-430 µs) and present submissions reach 164-167/s at p50
6,136-6,195 µs. The mean+p99 ≤ 1 ms submission target remains unmet; no
achievement is claimed. The remaining budget is dominated by the ground
fragment shader; CPU sim+camera dropped to 121-125 µs and fx to 139-143 µs per
present with the balanced pitch PD landing (a900aa6).

### Verification and limits

`cargo test --workspace --locked`: 96 passed, 0 failed. All 50 compiled SPIR-V
modules validate with `spirv-val --target-env vulkan1.3` (spirv-tools 2025.1,
userspace extract; `spirv-val` is not installed on this host and the Docker
check container is unavailable here). Benchmark runs were taken only when
`nvidia-smi` showed the GPU idle; earlier captures during concurrent
development showed 15 % clock/thermal contention noise and were discarded.
