# Hardware-level tuning

Deep hardware calls and driver-facing optimizations applied without changing
the rendered image. Every item below was verified with a frozen-scene
screenshot A/B: the optimized build is pixel-for-pixel identical to the
unmodified build at 2880x1646 (max channel difference 0), and a rerun of the
same binary reproduces its screenshot exactly, so the comparison is
deterministic. Unless stated otherwise, all features are capability-checked
and degrade to the portable path.

## Ray-traced shadow structure compaction

The sun-shadow casters (23 airframe node BLAS, the 2.1 M-triangle terrain
BLAS, and the landmark-structure BLAS) are built once at boot with
PREFER_FAST_TRACE. That mode reserves rebuild slack the casters never use,
so after the build fence the engine queries the compacted size of every
structure (`vkCmdWriteAccelerationStructuresProperties2` with
`VK_QUERY_TYPE_ACCELERATION_STRUCTURE_COMPACTED_SIZE`), repacks them
contiguously into a second buffer with
`vkCmdCopyAccelerationStructureKHR` in COMPACT mode, and swaps the handles
and device addresses the per-frame TLAS instances reference. The original
pool, the build scratch buffer (dead the moment the build fence signals),
and the pre-compaction handles are released.

- Result on the RTX 4060 Laptop target: 147.0 MiB -> 55.9 MiB of
  acceleration-structure storage (62% smaller). The compaction log line at
  boot records the exact figures per run.
- Applicability: geometry is copied verbatim by COMPACT mode, so shadow rays
  traverse identical data; the frozen-scene A/B confirmed no visual change.
- Limits: compaction is a boot-time trade (two extra submits) and pays off
  only for static casters; per-frame TLAS rebuilds are untouched.
- Source: Khronos Vulkan spec, VK_KHR_acceleration_structure,
  `vkCmdCopyAccelerationStructure` / compacted-size queries
  (https://docs.vulkan.org/spec/latest/chapters/accelstructures.html),
  accessed 2026-09-20.

## Dedicated allocations for large resources

Device-local allocations at or above 16 MiB (BLAS storage pools, mesh and
index buffers, the per-swapchain HDR and depth attachments) pass
`VK_MEMORY_DEDICATED_ALLOCATE_INFO` so the driver places them without
suballocation. This keeps the per-frame targets and the shadow pools out of
the suballocation heap, avoiding fragmentation-driven placement churn across
swapchain resizes.

- Source: NVIDIA, "Advanced API Performance: Dedicated Allocations"
  (https://developer.nvidia.com/blog/advanced-api-performance-dedicated-allocations/),
  accessed 2026-09-20; Khronos VK_KHR_dedicated_allocation.
- Limits: placement only; contents and addresses-as-queried are unchanged,
  and the frozen-scene A/B confirmed an identical image.

## CPU submission-path hardening

`vendor.rs` applies three OS-level controls before the render thread starts,
each capability-checked with a logged fallback:

- `mlockall(MCL_CURRENT)`: locks the existing address space so a minor page
  fault cannot land inside the submit loop (a fault costs tens of
  microseconds and appears directly in the submission-interval tail). Falls
  back with a log line when RLIMIT_MEMLOCK is too small.
- `sched_setscheduler(SCHED_FIFO, priority 1)`: removes scheduler-injection
  spikes from the submit cadence. Requires CAP_SYS_NICE; the stock target
  machine runs without it, and the log records the fallback to SCHED_OTHER.
- Core affinity already pins all threads off CPU 0 (existing work).

- Source: Linux man-pages `mlockall(2)`, `sched(7)` (real-time scheduling
  requirements), accessed 2026-09-20.
- Limits: SCHED_FIFO priority 1 is deliberately the lowest real-time
  priority; the engine never starves the driver threads on purpose.

## Measured on the target GPU

Frozen alpine scene, Balanced preset, 2880x1646, RT shadows on, wind 0,
burst 1 (per-present GPU stage stamps, contended-machine runs, so absolute
numbers drift with other GPU tenants; the A/B pairs below were taken minutes
apart under the same contention):

- HEAD vs hardware pass: opaque+RT 281 us vs 280 us, terrain 4126 vs 4115,
  clouds 389 vs 389, composite 419 vs 419 us. The optimizations are memory
  placement, VRAM footprint, and CPU jitter work: they do not shorten shader
  paths, and the A/B confirms no regression.
- VRAM for shadow casters: 147.0 -> 55.9 MiB.
- Submission cadence with mlockall active showed no page-fault spikes; the
  p99 is bounded by GPU duration under contention, not CPU stalls.

The 1,000 presents/second submission target remains bounded by GPU duration
on this shared machine; per the project contract, that target is judged from
clean-machine runs, not from these contended comparisons.
