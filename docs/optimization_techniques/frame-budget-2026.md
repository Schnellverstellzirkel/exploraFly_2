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

### Ground material gating (kept; visually coherent)

`ground.frag` evaluated ~15 value-noise octaves and ~10 photographic texture
fetches for every terrain fragment even when the consuming mix weight was
exactly zero (mid/far `micro_fade`/`rock_fade` distance fades, snow/rock/
outcrop/flower/edelweiss coverage gates). Every fetch now sits behind the
exact-zero mask of its only consumers, with defaults chosen so unfetched
values remain irrelevant to the final material. A frozen-scene screenshot
against the pre-change binary was visually coherent with no attention-worthy
material seam or missing detail. Ground pass 4,175 → 3,957-3,979 µs across
three 3,000-present runs on the integrated tree; the split carries roughly
±0.15 ms run-to-run noise.

### Depth-only terrain prepass (tested; rejected)

A depth-only terrain prepass ahead of the atmosphere passes early-Z-culled
every sky/cloud fragment hidden behind ridgelines (sky 799 → 221 µs), but the
prepass itself rasterizes ~2.1 M terrain triangles with 8×MSAA depth writes
and cost ≈ 1,060 µs. At the spawn scene the cloud slab sits almost entirely
above the ridgelines, so clouds saved nothing: net wash (gpu_frame 9,011 →
9,072 µs). The screenshot remained visually coherent, but the pass was still
removed because it made the frame slower. Reordering the passes front-to-back
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
visually and is available if a future vertex consumer needs the shared
atmosphere include.

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

## Terrain pass cost attribution by shader ablation (2026-09-20)

After the cloud mesh milestone and the front-to-back pass reorder
(opaque → terrain → clouds → sky), the terrain pass dominates the frame.
Shader-only ablations — building one variant per cost center with that work
disabled and benchmarking 1,500-present bursts at level and pitched-down
views on an idle GPU — attribute the terrain pass (level view, 3,024 µs):

| Cost center removed | Δ terrain pass |
| --- | --- |
| Cloud sun visibility (`cloudSunVisibility`) | −541 µs |
| All filtered value-noise octaves | −612 µs |
| All ray-traced shadow paths (aircraft, structures, settlement) | −37 µs |
| Water shading block | −56 µs |
| Photographic detail texture fetches | ~0 (bandwidth-hidden) |

The remaining ~1.8 ms is material assembly, PBR, and atmospheric perspective
ALU. Content dependence is large: the same views measured terrain 3,024 µs
(level) to 3,527 µs (pitched down) and composite 525-1,722 µs depending on
FXAA search length over ridge density. RT shadow tracing is already cheap and
well gated; detail textures are fully latency-hidden. Anyone optimizing the
terrain pass should target the cloud-shadow neighborhood walk and the noise
stack; anyone targeting the composite should know its cost is scene-content
driven, not fixed.

### Bounded cloud-shadow cell prune (tested; rejected)

An exact conservative prune for `cloudSunVisibility` was derived from the
placement bounds (in-cell offset, altitude 2,400-4,400 m, radius ≤ 980 × 1.6
so the shadow factor is exactly 1.0 beyond 1,881.6 m) and visually coherent on
a frozen-scene capture. It measured *slower*: nine per-cell rectangle-distance
evaluations (FP32/SFU) cost more than the integer hash
chain they replace, and at 38 % cumulus density most in-range cells pass the
family test anyway (+115 µs level, +239 µs pitched down). Reverted. The
measurement contradicts the intuition that "fewer placement hashes is
cheaper"; hash chains run on the integer pipe and the prune's floating-point work did
not.

### Frame-pacing fix for horizon rubber-banding (2026-09-20, kept)

Gameplay now presents with FIFO pacing (`pick_present_mode`): rendering
148-165 fps into the 119.96 Hz panel with mailbox made the completed-frame
age at each scanout drift on a beat cycle, which read as rubber-banding most
visibly along the horizon line. Gameplay presents locked to a steady ~112/s;
benchmarks keep the uncapped mailbox preference, so the submission-capability
measurement is unchanged. A per-frame GPU spike logger
(`EXPLORA_GPU_SPIKE_US`, default 9 ms) attributes any hitching frame to its
pass; it showed opaque+TLAS is stable at 352-373 µs on an idle GPU and that
earlier 400-1,221 µs excursions were contention from concurrent game
sessions on this machine.

## World-content landing and current budget (2026-09-20 midday)

With the settlement expansion (39 structure archetypes), procedural conifer
groves and boulder scatter, indexed cloud draws, near-to-far terrain ordering,
and the composite FXAA rework all landed, two 2,000-present bursts on an idle
GPU measure gpu_frame 4,627-4,631 µs (p50 5,313 µs, p99 6,289-6,331 µs) and
182 present submissions/s at 2880×1646 Balanced: terrain 3,348 µs (scatter and
structures included), clouds 423 µs, composite 408 µs, opaque+TLAS 275 µs,
sky 150 µs. The scatter draw adds ~330 µs over the bare-terrain pass; trunk
bases reconstruct ground height from the terrain cell's stored height and
slopes, which matches the piecewise-linear mesh exactly at zero analytic-cost.
`cargo test --workspace --locked`: 108 passed, 0 failed; 52 SPIR-V modules
validate for Vulkan 1.3.

## Terrain shader partition and cloud-shadow hoist (2026-09-21)

The target-machine baseline was captured before this change on the RTX 4060
Laptop GPU, NVIDIA 580.173.02, Wayland, 2,880×1,646 output, one complete
rendered scene per present, MAILBOX, with the fixed scene frozen. Performance
used a 2,304×1,317 scene at 2×2 ground shading and RT off; Balanced used native
2,880×1,646 shading with RT on. The baseline's dominant terrain intervals were
about 1,363–1,393 µs (Performance) and 3,251–3,283 µs (Balanced).

Two changes attack measured hot paths:

1. The terrain draw now uses a compile-time terrain-only fragment module. The
   general landmark/vegetation material code is retained for its own draws,
   reducing the terrain module from 6,087 to 4,173 SPIR-V instructions (RT
   variant 6,445 to 4,533). This is a register/live-range and instruction-cache
   change, not a visual simplification of the terrain recipe.
2. `cloudSunVisibility` previously walked up to a 4×4 cloud-cell neighborhood
   per terrain fragment. The field is smooth at the 64 m terrain vertex spacing
   relative to its 170–1,568 m cloud footprints, so the shared cloud recipe is
   evaluated once per terrain vertex and perspective-interpolated to the
   fragment. Landmark and vegetation materials retain their fragment path.

The three-run Balanced RT-on A/B measured terrain averages of 2,729, 2,732,
and 2,760 µs after the change, with complete GPU-frame means of 5,533, 5,551,
and 5,589 µs. The preceding split-only runs measured terrain at 3,195–3,219
µs and complete GPU frames at 6,026–6,064 µs. This is approximately a 15–16%
terrain reduction and 8–9% complete-GPU reduction in this workload. The
Performance run was more clock-variable (terrain 977–1,181 µs across its
three runs; one run down-clocked materially), so it is recorded as evidence,
not as a certified fixed percentage.

The cloud-shadow hoist is an intentional, bounded approximation: the terrain
height/material/geometry recipe remains unchanged, but shadow visibility is
interpolated instead of independently sampled per pixel. A frozen Performance
capture of the split-only and hoisted variants was inspected at native
resolution; the terrain, forest, and aircraft remained visually coherent with
no attention-worthy block boundary. The tradeoff is accepted because the
shadow field's documented footprint is much larger than one terrain triangle;
it must be re-profiled and visually rechecked if terrain cell size or cloud
radii change.

The target remains unmet: these runs are roughly 183 complete submissions/s
for Balanced and roughly 508/s for the current Performance capture, not
1,000/s. The Performance preset's 67% scene scale plus atmospheric cloud LOD
are intentional game quality/performance trades: native screenshot inspection
found the valley, forest bands, cloud layers, aircraft, wing lighting, LOD
transitions, and HUD coherent, with only mild scene softness. The HUD is
full-rate, so instruments remain readable during flight.
GPU clocks were not locked by the host, so pass timings are reported with
ranges and the benchmark JSON is retained under ignored `target/` files.

## Rust/GLSL world conformance (2026-09-21)

`cargo worldcheck` now creates a headless Vulkan compute device, dispatches the
compiled `world_conformance.comp` SPIR-V for 8,192 deterministic coordinates,
and compares terrain height/surface plus vegetation probability/forest-cover
against the public Rust recipe. The first target-GPU run passed with maximum
absolute errors of 0.013947 m for height/surface, 0.00000101 for presence, and
0.00000107 for forest cover. Tolerances are explicit in the harness; a future
equation edit must either preserve the result or update the test with a
documented numerical reason.

This complements the generated landmark/vegetation constants: data tables are
emitted from `crates/world` into `world_generated.inc`, while the conformance
dispatch covers equations that remain independently written in Rust and GLSL.
It is a correctness gate, not a performance benchmark.

The ocean extension adds the generated `terrainOceanMask` to that same GPU
comparison. The target-GPU run passed with maximum absolute errors of
0.013947 m for height/surface, 0.00000316 for presence, 0.00000098 for
forest cover, and 0.00000298 for the ocean mask. This catches a coastline
classification drift as well as a height drift before a visual run.

Target screenshots were then inspected as game images, not pixel hashes. A
frozen Performance capture at the open-water position showed the blue deep
ocean, long swell, foam, and mountain horizon; a second Balanced capture near
the coast showed the waterline, shoreline foam, and shelf transition without
an attention-worthy hard material break. The ocean run's representative
Performance terrain GPU samples were 0.65--0.71 ms on the RTX 4060 Laptop
GPU (NVIDIA 580.173.02) at a 1,930x1,103 scene target. That is a pass-local
observation, not a claim that the complete 1 ms presentation target is met.

## Vegetation HLOD follow-up and target-GPU evidence (2026-09-21)

The persistent vegetation path now keeps full tree geometry through 0.9 km,
crossed-plane tree geometry through the late 4.5--5.0 km handoff, and a
lower-frequency canopy field through 7.8 km. The final canopy shader uses a
four-by-four patch (54 vertices per visible cell, down from the initial
six-by-six/150-vertex experiment), and CPU command generation skips zero-density
canopy records. The current static database contains 2,029,770 packed
instances, 93,045 tree cells, and 89,435 nonzero canopy cells. A historical
pre-density-reduction capture used 2,802,917 instances; the current result is
the number used by the measurements below. This removes repeated per-vertex
scatter/settlement evaluation from the render loop, but it is not a claim that
the 1 ms target is achieved.

Final target-machine captures were made with the GPU idle: Ubuntu Wayland,
NVIDIA GeForce RTX 4060 Laptop GPU, NVIDIA 580.173.02, MAILBOX, one complete
rendered scene per present, and no display-timing feedback. Performance used
2,880×1,646 output with a 1,930×1,103 scene, 2×2 ground shading, RT shadows
off, calm wind, a frozen pose, six cloud puffs per cluster, and a 17×17 cloud
grid. Two 1,000-present runs measured wall means of 1,976.045 and 1,968.787 µs
(p99 2,722.502 and 2,698.756 µs) and GPU means of 1,949.939 and 1,941.542 µs
(p99 2,599.840 and 2,604.480 µs), producing 506.061 and 507.927 complete
submissions/s. Representative GPU stages were opaque+RT 0.052 ms, terrain
1.210–1.221 ms, trees 0.158–0.162 ms, canopy 0.063–0.065 ms, clouds
0.263–0.264 ms, sky 0.009 ms, plume 0.007–0.008 ms, and composite 0.164 ms.
The matched same-density 1x1 composite control at the former 80% Performance
scale measured 2.918 ms GPU mean and 0.755 ms composite-stage time; the split
path removes about 0.59 ms from that GPU frame while keeping the HUD in a
separate full-rate pass. The scale, terrain-detail, cloud-shadow, and cloud-LOD
changes were accepted from native screenshots and these timings, not from a
bitwise image comparison.

The current Performance-only reductions are: a macro terrain material LOD
keeps biome/slope/snow/forest/water lighting but skips photographic detail once
the terrain footprint is larger than the reduced scene can resolve; the terrain
draw keeps full 64 m topology in a 32 km central flight bubble and uses a
128 m outer-ring topology; terrain texture/micro-normal fades reach the
subpixel cutoff earlier; terrain cloud shadows sample a broad 3×3 neighborhood
at the ray midpoint instead of the full swept 4×4 search; and the cloud draw
keeps six puffs over a 17×17 cell grid instead of the full eight-puff 19×19
field. The full-resolution terrain index stream remains the RT BLAS source.
Settlement geometry, cloud recipe, cloud placement, aircraft, and HUD paths
remain shared with the quality modes.

The macro material LOD was measured in two matched Performance runs on the
same RTX 4060 workload: GPU mean 1,852.854 and 1,876.885 µs, GPU p99
2,535.040 and 2,535.936 µs, and 526.287 and 523.362 submissions/s. The
preceding two runs with the richer material path measured GPU means of
1,949.939 and 1,941.542 µs and 506.061 and 507.927 submissions/s. The gain is
about 4% in GPU mean and 3–4% in submission throughput; p99 remains dominated
by the broader frame workload. Native Performance screenshots on the calm and
boost/bank routes were inspected for terrain banding, material cutoffs, lake
edges, cloud-shadow seams, aircraft readability, and wing lighting; no
attention-worthy artifact was found. The all-coarse 128 m mesh was rejected
after its screenshot showed visibly angular mountain silhouettes. The accepted
hybrid mesh retained the central 32 km full-resolution bubble and removed the
visible near-field faceting.

The accepted hybrid mesh was then run twice in the same target workload. GPU
means were 1,717.365 and 1,741.575 µs, p99 values were 2,413.248 and 2,419.200
µs, and complete submissions were 551.291 and 550.965/s. The wall p99 was
3,700.858–4,618.652 µs under the live compositor, so this still does not meet
the 1 ms submission criterion. Balanced native RT-on regression remained
5,394.142 µs GPU mean and 184.785 submissions/s, within normal run variance
of the previous Balanced capture.

An otherwise promising 4×4 ground VRS experiment measured 1.191–1.205 ms GPU
means and about 595 submissions/s, but its native screenshot showed obvious
coarse blocks and bands across the foreground and mountain faces. It was
rejected and Performance remains at 2×2 ground shading. This is an intentional
game-quality decision, not a bit-preservation requirement.

A separate Balanced native-resolution, RT-on, boost-plus-hard-bank workload
ran for 1,000 presents. It measured 183.171 submissions/s; wall mean/p50/p95/
p99/max were 5,459.373/5,549.486/6,665.557/6,819.907/6,958.822 µs and GPU
mean/p50/p95/p99/max were 5,440.885/5,542.720/6,586.464/6,725.408/6,863.072
µs. Its final stage sample was opaque+RT 190 µs, terrain 2,959 µs, trees
313 µs, canopy 80 µs, clouds 409 µs, sky 188 µs, plume 100 µs, trail
557 µs, glass 13 µs, and composite 627 µs.

The no-hoist control and the final hoisted build were inspected as game
screenshots from the same frozen route at native resolution. The terrain and
forest lighting stayed coherent through the near/mid/far transition, with no
attention-worthy seam, grid, hard LOD pop, or wing-lighting discontinuity.
This visual smoke check—not bit preservation—is the acceptance criterion for
the approximation.

The evidence moves the hot-path priority from far-canopy geometry to terrain
and near/mid tree work. The existing compute compaction experiment remains
opt-in because its cull/finalize overhead was slower than the bounded CPU
indirect path in the controlled scene; a future GPU-driven redesign must
amortize that work and use an explicit indirect-count/barrier path before it is
made default. Both workloads remain far above the 1,000 submissions/s target,
and because display feedback was unavailable neither establishes a displayed
frame rate.

### Rendering references checked for this change

| Source | Publication/update date | Applicability | Limit | Accessed |
| --- | --- | --- | --- | --- |
| [Khronos Vulkan specification, fragment interpolation](https://registry.khronos.org/vulkan/specs/latest/html/vkspec.html#primsrast-interpolation) | Latest ratified/current registry page; section date not separately exposed | Defines the interpolation semantics used by the terrain varying | Interpolation semantics do not prove that a particular varying is visually or faster; target-GPU measurement and image checks remain necessary | 2026-09-21 |
| [Khronos GLSL 4.60 specification](https://registry.khronos.org/OpenGL/specs/gl/GLSLangSpec.4.60.pdf) | GLSL 4.60 specification, 2017-07-23 | Defines shader interface/interpolation qualifiers and numerical language rules used by the generated modules | OpenGL GLSL language rules are not a Vulkan-driver performance guarantee | 2026-09-21 |
| [NVIDIA Nsight Graphics Shader Profiler](https://docs.nvidia.com/nsight-graphics/UserGuide/shader-profiler.html) | Current NVIDIA User Guide; page update date not exposed | Primary target-GPU tool for validating occupancy, instruction mix, stalls, and shader cost after the source-level change | Nsight was not available in this shell session; Vulkan timestamp A/B and image checks are evidence, but do not replace a counter trace | 2026-09-21 |
