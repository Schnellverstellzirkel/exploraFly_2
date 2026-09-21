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
`canopy.vert` expands every visible nonzero canonical 128 m cell into a low-relief
terrain-following surface whose density is reconstructed from the same shared
forest-cover × forest-patch function used by near placement. Low-density
fragments are discarded, keeping neighboring cells continuous while the
surface remains visually subordinate to the tree silhouettes and terrain
material. The individual forest gate uses that same stand mask, so trees
cluster inside the same stands instead of filling meadows that the far field
marks as empty. Near placement applies a 2.4x saturated density boost inside
the mask (with a thresholded cover ramp and square-root edge remap) to fill
the stand with individual silhouettes before HLOD begins while leaving
low-cover outskirts empty.
Full tree crowns transition to the compact mid LOD over 0.9--3.0 km; compact
trees then hand off to the aggregate canopy only over the late 4.5--5.0 km
band. The stable
world-cell dither keeps the low hash values for nearby trees and the high values
for the aggregate canopy, so the opaque passes do not leave a lighter-ground
band between them. Canopy vertices reconstruct the exact piecewise-linear
terrain triangles and remain 3 m above that surface to avoid depth fighting.
GPU compaction,
`vkCmdDrawIndirectCount`, and a compute-generated visibility list remain
separate work. This is an engineering implementation, not a claim of state
of the art.

## Bounded data and lifetime

- World period: 65,536 m, with 512 x 512 canonical 128 m cells.
- Instance record: four 32-bit words containing canonical X/Z bits, ground
  height bits, and quantized scale/species/random/boulder metadata.
- The current world-level startup invariant measures 2,029,770 packed
  instances in 93,045 occupied tree cells and 89,435 nonzero canopy-field
  cells (about 31.7 MiB of tree instance words plus cell metadata, with a
  separate 4 MiB canonical canopy table) on the local optimized test build.
  A historical pre-density-reduction capture measured 2,802,917 instances;
  that number is retained below only to explain the earlier GPU smoke run. The generator
  uses independent source-row workers and one shared terrain cache for the
  aggregate field; this is a startup measurement, not a frame-time or GPU
  result.
- Tree range: a 5,000 m horizontal individual-geometry budget and at most
  4,096 indirect command records. Full crowns are used through 900 m; a
  dithered overlap band from 900--3,000 m hands coverage to compact
  crossed-plane silhouettes. Compact tree and aggregate-canopy coverage then
  crossfade over 4,500--5,000 m.
- Canopy field: one packed four-word aggregate record per canonical cell,
  with quantized density, height, representative species, and stable random.
  Zero-density records retain canonical coordinates for stable periodic
  indexing, but CPU command generation skips them because their fragments
  would be discarded unconditionally. The current generator reports the
  nonzero canopy count in the startup log.
- Canopy range: 7,800 m horizontally and at most 16,384 indirect command
  records. `canopy.vert` uses 54 vertices per visible cell and reconstructs a
  four-by-four field sample; unused command slots have `instanceCount = 0`,
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
        +-- full individual tree cells, 0--0.9 km
        |       108-vertex tree/boulder expansion
        |
        +-- dithered full + compact overlap, 0.9--3.0 km
        |       complementary world-cell coverage, no hard replacement
        |
        +-- compact individual tree cells, 0.9--5.0 km
        |       12-vertex crossed-plane expansion
        |       fade out: 3.0--5.0 km
        |
        +-- aggregate canopy field, 4.5--7.8 km
                54-vertex four-by-four terrain-following cell surface
                complementary fade in: 4.5--5.0 km
```

The canopy record is sampled from the same cached height, slope, moisture,
`forest_cover`, and periodic patch functions that drive placement. It is not a
second procedural biome: near candidates use the same continuous cover term
and stand mask as the far surface, with `forest_patch = smoothstep(0.36, 0.68,
field)` plus the saturated near-density multiplier. The current implementation uses CPU cell traversal to produce fixed
indirect arrays; it does not yet run `foliage-cull.comp` or use
`vkCmdDrawIndirectCount`. Tree crown palettes are intentionally restricted to
green values; seasonal gold, flower, and snow-color overrides are not used for
the individual foliage pass.

## Historical target-machine smoke measurement

This pre-reduction capture is retained for context, not as the current pipeline
measurement. It used the initial six-by-six canopy patch (150 generated
vertices per cell), before the final four-by-four reduction and zero-density
command skip. The workload was Ubuntu Wayland, NVIDIA GeForce RTX 4060 Laptop
GPU, driver 580.173.02, Balanced preset, RT shadows on, MAILBOX, 2,880 x 1,646
output and scene resolution, one rendered pass per present, 20 measured present
submissions. Display feedback was unavailable,
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

## Final HLOD target-machine validation (2026-09-21)

The final path uses the four-by-four canopy patch (54 generated vertices per
cell) and omits CPU indirect records for zero-density canopy fields. The
captures were taken after the GPU was idle on Ubuntu Wayland with an NVIDIA
GeForce RTX 4060 Laptop GPU, NVIDIA 580.173.02, Vulkan 1.3.275, MAILBOX, and
one complete rendered scene per present. `VK_GOOGLE_display_timing` supplied no
feedback, so these are submitted-present measurements, not physically displayed
frame rates.

| Workload | Output / scene | Wall mean / p50 / p95 / p99 / max | GPU mean / p50 / p95 / p99 / max | Complete submissions/s |
| --- | --- | ---: | ---: | ---: |
| Performance, RT off, frozen calm scene, 2 runs × 1,000 | 2,880×1,646 / 1,930×1,103 | 1,968.787–1,976.045 / 1,592.183–1,611.180 / 2,651.264–2,653.638 / 2,698.756–2,722.502 / 4,635.259–5,524.554 µs (run range) | 1,941.542–1,949.939 / 1,581.312–1,596.704 / 2,564.672–2,565.120 / 2,599.840–2,604.480 / 2,616.512–2,725.888 µs (run range) | 506.061–507.927 |
| Balanced, RT on, boost + hard bank, 1,000 | 2,880×1,646 / 2,880×1,646 | 5,459.373 / 5,549.486 / 6,665.557 / 6,819.907 / 6,958.822 µs | 5,440.885 / 5,542.720 / 6,586.464 / 6,725.408 / 6,863.072 µs | 183.171 |

The current Performance run's representative GPU stages were approximately
opaque+RT 0.052 ms, terrain 1.210–1.221 ms, individual trees 0.158–0.162 ms,
canopy 0.063–0.065 ms, clouds 0.263–0.264 ms, sky 0.009 ms, plume
0.007–0.008 ms, and composite 0.164 ms. The matched same-density 1x1
composite control at the former 80% scene scale measured 2.918 ms GPU mean and
0.755 ms composite-stage time; the split path therefore removes about 0.59 ms
from that GPU frame while keeping the HUD in a separate full-rate pass. The
Balanced run's final stage sample
reported opaque+RT 0.190 ms, terrain 2.959 ms, trees 0.313 ms, canopy 0.080
ms, clouds 0.409 ms, sky 0.188 ms, plume 0.100 ms, trail 0.557 ms, glass
0.013 ms, and composite 0.627 ms. The cloud-shadow hoist
removed the repeated per-fragment cloud neighborhood work from foliage; the
remaining hot paths are terrain, composite, and the RT-on workload's
high-resolution shading. The 1,000 submitted-presentations/s target remains
unmet, and the display rate is unverified.

The clean Performance A/B was also checked as a game image, not just as a
counter result. The final split-composite build and the 67%-scale Performance
build were captured on the frozen route and inspected at native output
resolution. The forest bands, terrain lighting, aircraft silhouette,
near/mid/far transitions, and full-rate HUD remained coherent; there was no
attention-worthy seam, canopy grid, sudden LOD pop, chunky instrument text, or
wing-lighting discontinuity. The 67% scene is mildly softer by design, but the
image still reads as the same game. Screenshot inspection is the acceptance
criterion here; image numbers are only diagnostic support.

The GPU compaction path remains capability-safe and opt-in rather than being
made the default: the controlled earlier A/B on this scene measured its
compute cull/finalize overhead above the fixed CPU-command path. The next
high-value experiment is therefore a counter-guided compaction redesign that
amortizes cull work and feeds indirect-count draws, not enabling the existing
slower path by default.

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
