# Persistent vegetation pipeline

Status: Phase 5/6 implementation, 2026-09-21.

The renderer now separates individual vegetation from the terrain/indexed
draw. `world::vegetation::build_database()` evaluates the deterministic
scatter and settlement-clearance rules once at startup, sorts accepted primary
and companion trees/boulders into 128 m periodic cells, and uploads a compact
`uvec4` instance record to a device-local storage buffer. The per-present path
only rewrites bounded host-visible indirect-command ranges after conservative
CPU cell/frustum culling. `vegetation.vert` expands a near instance into 108
vertices or a mid-range instance into a 12-vertex crossed-plane silhouette;
it does not re-run biome hashes, terrain texture fetches, or the 3x3 village
loop per vertex.

The far path now has a separate aggregate canopy field and raster pipeline.
`canopy.vert` expands every visible canonical 128 m cell into a low-relief
terrain-following surface whose density is reconstructed from the same shared
forest function. Low-density fragments are discarded, keeping neighboring
cells continuous while the surface remains visually subordinate to the tree
silhouettes and terrain material. Full tree crowns transition to the compact
mid LOD near 1.8 km; the tree fade and far-field canopy overlap from roughly
4.5--5.0 km, with the canopy fading out by 6.5 km. GPU compaction,
`vkCmdDrawIndirectCount`, and a compute-generated visibility list remain
separate work. This is an engineering implementation, not a claim of state
of the art.

## Bounded data and lifetime

- World period: 65,536 m, with 512 x 512 canonical 128 m cells.
- Instance record: four 32-bit words containing canonical X/Z bits, ground
  height bits, and quantized scale/species/random/boulder metadata.
- The ignored world-level startup invariant measured 2,802,917 packed
  instances in 173,349 occupied tree cells and 115,053 nonzero canopy-field
  cells
  (about 44.8 MiB of instance words plus cell metadata) in 3.911 s on the
  local optimized test build. The generator
  uses independent source-row workers and one shared terrain cache for the
  aggregate field; this is a startup measurement, not a frame-time or GPU
  result.
- Tree range: a 5,000 m horizontal individual-geometry budget and at most
  4,096 indirect command records. Full crowns are used through 1,800 m;
  compact crossed-plane silhouettes cover the mid range. Tree coverage fades
  out over 3,000--5,000 m.
- Canopy field: one packed four-word aggregate record per canonical cell,
  with quantized density, height, representative species, and stable random.
  Low-density records retain canonical coordinates so the GPU record index is
  the canonical cell index and adjacent visible patches share the field
  boundary. The current generator reports the nonzero canopy count in the
  startup log.
- Canopy range: 7,800 m horizontally and at most 16,384 indirect command
  records. `canopy.vert` uses 150 vertices per visible cell and reconstructs a
  six-by-six field sample; unused command slots have `instanceCount = 0`,
  while low-density fragments are rejected in the shared material shader.
- Both static storage buffers are device-local and copied once through
  host-visible staging buffers. Per-frame tree and canopy commands remain in
  the mapped UBO/indirect allocation, so no per-present allocation or command
  buffer recording was added.

The tree 4,096-record bound is a fixed cap for the 5,000 m window; nearest
visible cells are retained after sorting if the theoretical window exceeds the
cap. The canopy 16,384-record bound covers the 125 x 125
worst-case window used by the 7,800 m range test. They are safety bounds, not
measured visible-cell counts. A future GPU culling pass must keep explicit
capacities and add a storage/indirect memory barrier before drawing.

## LOD representation

```text
terrain depth/material
        |
        +-- full individual tree cells, 0--1.8 km
        |       108-vertex tree/boulder expansion
        |
        +-- compact individual tree cells, 1.8--5.0 km
        |       12-vertex crossed-plane expansion
        |       fade out: 3.0--5.0 km
        |
        +-- aggregate canopy field, 4.5--7.8 km
                150-vertex six-by-six terrain-following cell surface
                fade in: 4.5--6.5 km
```

The canopy record is sampled from the same cached height, slope, moisture,
`forest_cover`, and periodic patch functions that drive placement. It is not a
second procedural biome. The current implementation uses CPU cell traversal
to produce fixed indirect arrays; it does not yet run `foliage-cull.comp` or
use `vkCmdDrawIndirectCount`.

## First target-machine smoke measurement

This is a pipeline smoke test, not an acceptance run: Ubuntu Wayland, NVIDIA
GeForce RTX 4060 Laptop GPU, driver 580.173.02, Balanced preset, RT shadows
on, MAILBOX, 2,880 x 1,646 output and scene resolution, one rendered pass per
present, 20 measured present submissions. Display feedback was unavailable,
so the result is submission cadence only. The renderer reported 2,802,917 tree
instances and 115,053 nonzero canopy-field cells, built the canopy pipeline,
and completed Vulkan presentation.

| Metric | Result |
| --- | ---: |
| Complete present submissions/s | 136.658 |
| Wall mean / p50 / p95 / p99 / max | 7,317.545 / 6,911.420 / 13,370.470 / 13,401.341 / 13,401.341 us |
| GPU mean / p50 / p95 / p99 / max | 6,962.136 / 7,111.776 / 7,973.536 / 8,272.352 / 8,272.352 us |
| GPU pass averages (opaque / terrain / trees / canopy / clouds / sky / plume / trail / glass / composite) | 214 / 3,640 / 1,552 / 395 / 433 / 153 / 18 / 0 / 0 / 559 us |
| 1 ms mean+p99 criterion | failed |

The per-pass timestamp now separates terrain, individual trees, and aggregate
canopy. This short run still does not control thermal state or establish an
A/B comparison; it shows approximately 1.552 ms for the full/mid individual
tree pass and 0.395 ms for the far canopy in this workload. The measured
1,000 complete-presentations/s target is not achieved.

For visual acceptance, a separate boosted 3,000-frame route was captured at
frames 8, 750, 1,500, 2,250, and 3,000 on the same target. The screenshots
were inspected at native 2,880 x 1,646 output: full crowns give way to the
compact crossed-plane silhouettes, then the terrain forest field and subdued
far canopy carry the horizon without a visible grid, empty band, or hard LOD
pop in the sampled route. This is a visual smoke check, not a performance
claim.

## Vulkan evidence and applicability

Accessed 2026-09-20. Dates below distinguish the original extension revision
from the current generated/current-reference pages.

| Source | Publication/update date | What applies here | Limit recorded here |
| --- | --- | --- | --- |
| [Khronos `vkCmdDrawIndirect` reference](https://docs.vulkan.org/refpages/latest/refpages/source/vkCmdDrawIndirect.html) | Current generated reference; page date not separately exposed; accessed 2026-09-20 | Draw parameters can come from a buffer, allowing one recorded call to consume the fixed vegetation command array. | It still requires the host to provide the draw count at command-record time, which is why the implementation uses a bounded count and zero-count tail entries. |
| [Khronos `VK_KHR_draw_indirect_count` reference](https://docs.vulkan.org/refpages/latest/refpages/source/VK_KHR_draw_indirect_count.html) | Ratified revision 1, last modified 2017-08-25; accessed current page 2026-09-20 | Establishes the later count-buffer path and records that the functionality was promoted to Vulkan 1.2. | Promotion does not mean the feature bit is silently enabled on every device; the current implementation does not assume `drawIndirectCount` and therefore does not call the count variant yet. |
| [Khronos Vulkan Guide: `VK_KHR_draw_indirect_count`](https://docs.vulkan.org/guide/latest/extensions/VK_KHR_draw_indirect_count.html) | Current guide page; no publication date exposed, crawled about two weeks before access; accessed 2026-09-20 | Confirms that `vkCmdDrawIndirectCount`/`vkCmdDrawIndexedIndirectCount` use a GPU buffer for the dynamic draw count and require the extension or `VkPhysicalDeviceVulkan12Features::drawIndirectCount`. | A future GPU compaction/count pass must query and enable that feature and synchronize compute writes to the indirect/count reads; this note does not treat it as free. |
| [Khronos `VkPhysicalDeviceVulkan12Features`/latest specification](https://registry.khronos.org/vulkan/specs/latest-ratified/pdf/vkspec.pdf) | Latest ratified specification available at access; PDF result published 2026; accessed 2026-09-20 | The `drawIndirectCount` capability is an explicit Vulkan 1.2 feature member. | Device creation must enable the queried capability; a capability-checked fallback is required for portability. |
| [Khronos GPU Rendering and Multi-Draw Indirect sample](https://docs.vulkan.org/samples/latest/samples/performance/multi_draw_indirect/README.html) | Current Vulkan Samples page; crawled/accessed 2026-09-20 | Describes fixed indirect command arrays, instance-count zeroing, CPU frustum culling, and GPU command generation as interchangeable stages. The current renderer uses the bounded CPU variant. | The sample demonstrates command generation, not a forest HLOD appearance or transition quality; its GPU path still requires explicit synchronization and capacity discipline. |
| [Mantler, *Interactive Vegetation Rendering*](https://repositum.tuwien.at/bitstream/20.500.12708/14633/2/Mantler%20Stephan%20-%202007%20-%20Interactive%20vegetation%20rendering.pdf) | Foundational dissertation, 2007; accessed 2026-09-20 | Surveys far-field vegetation, impostor validity, and distance-based LOD representations, motivating a lower-frequency aggregate canopy instead of carrying full tree geometry to the horizon. | This is older foundational work and discusses image-based/impostor approaches; the current implementation uses a procedural terrain-following canopy field, so it is not evidence that this exact representation is optimal. |
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
next profiling pass should compare the tree-only, canopy-only, and combined
paths under the same camera route before changing the cell size or transition
band.
