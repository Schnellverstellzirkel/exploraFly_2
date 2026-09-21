# Airframe vertex-cache order: meshopt, FIFO, and the retired Forsyth path

Date: 2026-09-22. Target: NVIDIA GeForce RTX 4060 Laptop GPU, driver
580.173.02, AMD Ryzen 7 7840HS, Linux/Wayland.

The bake chain is `QEM simplify -> cache reorder -> build_meshlets(64
vertices, 126 triangles) -> task/mesh shader`. Triangle order feeds
`build_meshlets`, so the order changes vertex-cache behavior and meshlet
composition at the same time. The previous order was a hand-rolled 2006
Forsyth heuristic in `crates/airframe/src/forsyth.rs` with a hardcoded
32-entry cache score. This change measures three orders, ships the winner,
and deletes the Forsyth code.

## What ran

CPU analysis: `cargo run -p airframe-baker -- --compare-cache-order`.
It builds every part's five LODs once from the shared simplifier output,
reorders each level under each strategy, then records for all 56 parts x
5 LODs (280 samples per order):

- ACMR and ATVR from `meshopt_rs::vertex::cache::analyze_vertex_cache`
  with cache size 16, warp size 0, primitive-group size 0. The model is a
  FIFO approximation of a post-transform cache, not a GPU trace.
- triangle count;
- downstream meshlets from the same `build_meshlets(64v, 126t)` call the
  bake uses, plus fill (triangles against 126 per meshlet), local reuse
  (triangles x 3 / summed meshlet vertex counts), and normal-cone stats
  (fraction with `cone_cutoff < 1.0`, mean cutoff).

GPU A/B: three release binaries of `explora-engine`, each built with
`EXPLORA_AIRFRAME_CACHE_ORDER` set to `meshopt`, `fifo`, or `forsyth`.
The baked `airframe.bin` hashes were verified distinct per order. Runs
alternated meshopt and forsyth three times each, then fifo three times:
`--benchmark 2000` after the engine's 2-second plus 500-present warmup,
`EXPLORA_BURST=1 EXPLORA_MESH_SHADERS=on EXPLORA_WIND=0 EXPLORA_AUDIO=0
EXPLORA_HUD=0 EXPLORA_PRESENT=mailbox EXPLORA_WINDOWED=1`. Idle GPU
utilization between runs was 10-25 percent (gnome-shell and Firefox held
the card). The scene could not be frozen: `EXPLORA_FREEZE` is DEBUG_ONLY
and release builds run the Cinematic preset at 2880x1646 with RT shadows
on. `opq+rt` is the engine's opaque-plus-TLAS stage stamp, the stage that
draws the airframe meshlets.

## CPU results

Aggregate over all five LODs and all parts:

| order | ACMR | ATVR | meshlets | fill % | tris/vert | cone % | mean cutoff |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| meshopt | 0.7587 | 1.0986 | 835 | 53.9 | 3.853 | 46.5 | 0.7350 |
| fifo-16 | 0.7754 | 1.1228 | 851 | 52.9 | 3.787 | 43.4 | 0.7591 |
| forsyth | 0.8347 | 1.2087 | 872 | 51.6 | 3.652 | 46.0 | 0.7200 |

Level 0 alone (24,486 triangles):

| order | ACMR | ATVR | meshlets | fill % | tris/vert |
| --- | ---: | ---: | ---: | ---: | ---: |
| meshopt | 0.6944 | 1.1083 | 300 | 64.8 | 4.135 |
| fifo-16 | 0.7280 | 1.1620 | 307 | 63.3 | 4.064 |
| forsyth | 0.8409 | 1.3422 | 335 | 58.0 | 3.694 |

Meshopt wins ACMR, ATVR, meshlet count, fill, and local reuse at every
level. Forsyth is best only on mean cone cutoff (0.7200 versus 0.7350),
a margin that does not offset worse vertex reuse. Triangle counts are
identical across orders, as the baker tests require. Per-part rows are
produced by the same command; the aggregate table is what this note
records.

## GPU results

Three runs per order, alternating, `opq+rt` stage and complete GPU frame:

| order | opq+rt us (3 runs) | gpu_frame mean us | gpu_frame p99 us | submissions/s |
| --- | --- | --- | --- | --- |
| meshopt | 191.0 / 192.0 / 193.0 | 6151.8 / 6189.2 / 6186.5 | 7045.3 / 7063.7 / 7067.5 | 162.0 / 161.1 / 161.1 |
| forsyth | 226.0 / 226.0 / 226.0 | 6249.4 / 6229.9 / 6244.5 | 7108.9 / 7092.4 / 7103.6 | 159.5 / 160.0 / 159.7 |
| fifo-16 | 193.0 / 193.0 / 193.0 | 6188.6 / 6199.5 / 6206.8 | 7075.3 / 7076.0 / 7052.9 | 161.1 / 160.8 / 160.6 |

Terrain stage stayed 3620-3656 us across every run, so the environment
held still enough to separate the orders. The airframe stage is about
34 us faster with meshopt than with forsyth (192.0 versus 226.0 us mean,
about 15 percent of that stage). Complete GPU frames follow the same
ordering: meshopt 6175.8 us mean, fifo 6198.3 us, forsyth 6241.2 us.
FIFO tracks meshopt inside run noise on this GPU while scoring worse on
ACMR, which matches meshoptimizer's warning that FIFO is the weaker
choice on most GPUs and exists because it optimizes faster, a cost that
does not matter for offline baking.

These numbers are not the 1,000-present/s acceptance run. The complete
frame is dominated by terrain and FX on the Cinematic preset at roughly
161 submissions/s. The claim here is only the airframe-stage difference
between three index orders in one alternating capture.

## Decision

Ship meshopt adaptive (`optimize_vertex_cache` from meshopt-rs 0.1.2)
as the production order. Keep FIFO-16 selectable through
`EXPLORA_AIRFRAME_CACHE_ORDER=fifo` and `--cache-order fifo` as the
measurement control. The Forsyth implementation is deleted from the
workspace; `CacheOrder::parse("forsyth")` returns an error that points
at this note. `kernels/terrain-kernel` still contains its own runtime
copy of the same algorithm and was not part of this change.

## Limits

- ACMR and ATVR come from a FIFO cache model with cache size 16. Real
  GPUs reuse vertices with different mechanics; meshoptimizer states
  this in its own analyzer docs.
- The GPU capture ran on a desktop with 10-25 percent background
  utilization, an unfrozen live scene, and the Cinematic preset. The
  consistency of the opq+rt samples (forsyth exactly 226 us three
  times, meshopt within 191-193 us) supports the ordering, not a
  universal percentage.
- Overdraw optimization and vertex-fetch reorder from the meshoptimizer
  pipeline were not added. Per-meshlet `optimizeMeshlet` was not added.
- The 1,000 complete presentation submissions per second target remains
  unverified and unmet by this change.

## Reproduce

```sh
cargo run -p airframe-baker -- --compare-cache-order
EXPLORA_AIRFRAME_CACHE_ORDER=forsyth  # rejected; retired order
cargo build --release -p explora-engine
EXPLORA_BURST=1 EXPLORA_MESH_SHADERS=on EXPLORA_PRESENT=mailbox \
  target/release/explora --benchmark 2000
```

## Primary references

Accessed 2026-09-22:

| Source | Publication/update date | Used for | Limit | Accessed |
| --- | --- | --- | --- | --- |
| [meshoptimizer README](https://github.com/zeux/meshoptimizer/blob/master/README.md) | Master branch; release notes advertise v1.2 | Adaptive vertex-cache optimizer rationale, FIFO-16 control behavior, pipeline order (vertex cache, overdraw, vertex fetch), statement that modern GPUs still reuse vertices with different mechanics | Upstream project documentation, not a measurement of this airframe | 2026-09-22 |
| [meshopt-rs README](https://github.com/gwihlidal/meshopt-rs) | Repository for the Rust wrapper used by `airframe-baker` | ACMR and ATVR definitions, analyzer caveats, pipeline order | Wrapper docs; the crate here is 0.1.2 with the `experimental` feature | 2026-09-22 |
| [meshopt-rs `analyze_vertex_cache`](https://docs.rs/meshopt-rs/0.1.2/meshopt_rs/vertex/cache/fn.analyze_vertex_cache.html) | meshopt-rs 0.1.2 published 2026-08-01 | FIFO cache model semantics behind the reported ACMR/ATVR | Approximate model; results may not match actual GPU performance | 2026-09-22 |
| [Khronos `VK_EXT_mesh_shader`](https://registry.khronos.org/vulkan/specs/latest/man/html/VK_EXT_mesh_shader.html) | Current registry specification page | Task/mesh draw path that consumes the baked meshlets | Capability-checked extension; does not prescribe an index order | 2026-09-22 |

Related authored notes: [airframe-build-time-bake.md](airframe-build-time-bake.md),
[frame-budget-2026.md](frame-budget-2026.md).
