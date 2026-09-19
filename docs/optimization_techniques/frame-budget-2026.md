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
