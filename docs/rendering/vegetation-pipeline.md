# Persistent vegetation pipeline

Status: Phase 3/4 implementation, 2026-09-20.

The renderer now separates individual vegetation from the terrain/indexed
draw. `world::vegetation::build_database()` evaluates the deterministic
scatter and settlement-clearance rules once at startup, sorts accepted primary
and companion trees/boulders into 128 m periodic cells, and uploads a compact
`uvec4` instance record to a device-local storage buffer. The per-present path
only rewrites a bounded host-visible indirect-command range after conservative
CPU cell/frustum culling. `vegetation.vert` expands one instance into 108
vertices; it does not re-run biome hashes, terrain texture fetches, or the
3x3 village loop per vertex.

The current implementation intentionally stops at CPU cell culling plus
`vkCmdDrawIndirect`. GPU compaction, `vkCmdDrawIndirectCount`, far-canopy HLOD,
and dithered tree/canopy transitions remain separate work. This is an
engineering baseline, not a claim of state of the art.

## Bounded data and lifetime

- World period: 65,536 m, with 512 x 512 canonical 128 m cells.
- Instance record: four 32-bit words containing canonical X/Z bits, ground
  height bits, and quantized scale/species/random/boulder metadata.
- The ignored world-level startup invariant measured 2,802,917 packed
  instances in 173,349 occupied cells (about 44.8 MiB of instance words plus
  cell metadata) in 3.689 s on the local optimized test build. The generator
  uses independent source-row workers; this is a startup measurement, not a
  frame-time or GPU result.
- Draw range: a 3,600 m horizontal vegetation budget and at most 4,096
  indirect command records. Unused records have `instanceCount = 0`; the
  command buffer remains reusable while the mapped command data changes.
- The static instance buffer is device-local and is copied once through a
  host-visible staging buffer. Per-frame commands remain in the existing
  mapped UBO/indirect allocation, so no per-present allocation or command
  buffer recording was added.

The 4,096-record bound is deliberately larger than the 61 x 61 worst-case
cell window used by the current range test. It is a safety bound, not a
measured visible-cell count. A future GPU culling pass must keep an explicit
capacity and add a storage/indirect memory barrier before drawing.

## First target-machine smoke measurement

This is a pipeline smoke test, not an acceptance run: Ubuntu Wayland, NVIDIA
GeForce RTX 4060 Laptop GPU, driver 580.173.02, Balanced preset, RT shadows
on, MAILBOX, 2,880 x 1,646 output and scene resolution, one rendered pass per
present, 20 measured present submissions. Display feedback was unavailable,
so the result is submission cadence only. The renderer reported 2,802,917
instances at startup and completed Vulkan pipeline creation and presentation.

| Metric | Result |
| --- | ---: |
| Complete present submissions/s | 168.604 |
| Wall mean / p50 / p95 / p99 / max | 5,931.071 / 5,905.721 / 6,798.585 / 6,824.706 / 6,824.706 us |
| GPU mean / p50 / p95 / p99 / max | 5,918.226 / 5,835.296 / 6,765.344 / 6,805.504 / 6,805.504 us |
| GPU pass averages (opaque / terrain / vegetation / clouds / sky / plume / trail / glass / composite) | 302 / 3,133 / 686 / 397 / 160 / 17 / 792 / 0 / 427 us |
| 1 ms mean+p99 criterion | failed |

The per-pass timestamp now separates terrain and vegetation. This short run
still does not control thermal state or establish an A/B comparison; it is a
smoke measurement showing that the dedicated vegetation draw costs about
0.686 ms in this workload while the terrain pass remains dominant. A longer
controlled comparison is required before attributing a net gain.

## Vulkan evidence and applicability

Accessed 2026-09-20. Dates below distinguish the original extension revision
from the current generated/current-reference pages.

| Source | Publication/update date | What applies here | Limit recorded here |
| --- | --- | --- | --- |
| [Khronos `vkCmdDrawIndirect` reference](https://docs.vulkan.org/refpages/latest/refpages/source/vkCmdDrawIndirect.html) | Current generated reference; page date not separately exposed; accessed 2026-09-20 | Draw parameters can come from a buffer, allowing one recorded call to consume the fixed vegetation command array. | It still requires the host to provide the draw count at command-record time, which is why the implementation uses a bounded count and zero-count tail entries. |
| [Khronos `VK_KHR_draw_indirect_count` reference](https://docs.vulkan.org/refpages/latest/refpages/source/VK_KHR_draw_indirect_count.html) | Ratified revision 1, last modified 2017-08-25; accessed current page 2026-09-20 | Establishes the later count-buffer path and records that the functionality was promoted to Vulkan 1.2. | Promotion does not mean the feature bit is silently enabled on every device; the current implementation does not assume `drawIndirectCount` and therefore does not call the count variant yet. |
| [Khronos Vulkan Guide: `VK_KHR_draw_indirect_count`](https://docs.vulkan.org/guide/latest/extensions/VK_KHR_draw_indirect_count.html) | Current guide page; no publication date exposed, crawled about two weeks before access; accessed 2026-09-20 | Confirms that `vkCmdDrawIndirectCount`/`vkCmdDrawIndexedIndirectCount` use a GPU buffer for the dynamic draw count and require the extension or `VkPhysicalDeviceVulkan12Features::drawIndirectCount`. | A future GPU compaction/count pass must query and enable that feature and synchronize compute writes to the indirect/count reads; this note does not treat it as free. |
| [Khronos `VkPhysicalDeviceVulkan12Features`/latest specification](https://registry.khronos.org/vulkan/specs/latest-ratified/pdf/vkspec.pdf) | Latest ratified specification available at access; PDF result published 2026; accessed 2026-09-20 | The `drawIndirectCount` capability is an explicit Vulkan 1.2 feature member. | Device creation must enable the queried capability; a capability-checked fallback is required for portability. |
| [NVIDIA, Vulkan Device Generated Commands](https://developer.nvidia.com/blog/?p=16697) | NVIDIA technical blog, approximately 2020; current page accessed 2026-09-20 | NVIDIA describes multi-draw indirect as useful for CPU overhead and GPU culling/LOD, which supports separating vegetation from the terrain draw. | The post concerns NVIDIA paths and device-generated commands; it is not evidence that DGC, mesh shaders, or a vendor path will be faster on this project or on AMD. No NVIDIA-specific API is used here. |

These sources justify the command-buffer shape, not a performance result. The
renderer still needs GPU measurements on the RTX 4060 Laptop target with the
actual scene and presentation path. CPU tests, shader validation, and a lower
vegetation vertex count cannot establish the 1,000 complete-presentation/s
target.

## Validation and next measurement

The source-level validation for this phase is:

```text
cargo test --workspace --locked
python3 tools/check_shaders.py
```

The acceptance record must additionally report wall-frame mean/p95/p99/max,
GPU timestamp timings, resolution, preset, scene workload, GPU/driver, and
complete present submissions separately from physically displayed frames. The
next profiling pass should isolate terrain, vegetation, and composite GPU
timestamps before changing the cell size or range.
