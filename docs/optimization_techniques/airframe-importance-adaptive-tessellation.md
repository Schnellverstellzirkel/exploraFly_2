# Airframe semantic importance, adaptive tessellation, and LOD policy

This change cuts baked airframe geometry nearly in half without touching
`airframe.rs` runtime cost. Parts are classified by semantic importance, offline
primitive tessellation adapts to a per-class error budget, the baker picks LOD
and RT-proxy budgets from the same class, and the task shader drops low-importance
parts that fall below a few pixels on screen. Format version stays at 2.

## Measured counts

Environment: same baker and generator path as
[airframe-build-time-bake.md](airframe-build-time-bake.md), 2026-09-22.

| Metric | Before | After | Change |
| --- | ---: | ---: | ---: |
| Triangles (LOD0) | 46,752 | 24,486 | -47.6% |
| Vertices | 28,162 | 15,341 | -45.5% |
| RT proxy indices | 14,298 | 8,544 | -40.2% |
| Meshlets (all LODs) | not tracked | 872 | new |
| Meshlet vertices | not tracked | 46,614 | new |
| Packed asset size | 1,345,852 B | 1,009,445 B | -24.9% |
| RT nodes | 23 | 23 | unchanged |

Full LOD chain after the change:

| Level | Triangles | Meshlets |
| --- | ---: | ---: |
| 0 | 24,486 | 335 |
| 1 | 15,733 | 220 |
| 2 | 9,383 | 143 |
| 3 | 4,610 | 97 |
| 4 | 2,527 | 77 |

Per-importance LOD0 breakdown (56 parts):

| Importance | Parts | Triangles |
| --- | ---: | ---: |
| Silhouette | 12 | 10,936 |
| Structural | 24 | 9,080 |
| Detail | 10 | 2,996 |
| Interior | 6 | 1,144 |
| Emitter | 4 | 330 |

These are baked topology counters. They are not GPU frame times and not present
submissions. The 1,000-present/s acceptance target is unchanged and unverified
by this work.

## Importance classes

`airframe::Importance` is a five-value enum with explicit `#[repr(u8)]`
discriminants. `Importance::packed_id()` is the single wire value: the baker
packs it into `PartDesc::flags` bits 8..12 (bit 0 remains `PART_FLAG_GLASS`).
Format version stays at 2. Validation rejects `part_importance >= IMPORTANCE_COUNT`.
`MatId::packed_id()` and `Node::packed_id()` are the same single source for the
vertex-stream material byte and the 23-slot node UBO; the baker no longer keeps
duplicate index maps.

Each class returns a compile-time `LodPolicy` from `Importance::policy()`:

| Class | Wire | Meaning | Tess error (m) | Error scale | LOD level-4 error budget | LOD level-4 tri ratio | RT density | Drop px | Impostor px | Preserve anim |
| --- | ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| Silhouette | 0 | hull shell, sail, fins | 0.001 | 0.5 | 0.032 | 0.10 | 0.10 | 0.0 (never) | 0.0002 | yes |
| Structural | 1 | flaps, booms, binding, petals | 0.005 | 1.0 | 0.064 | 0.0625 | 0.08 | 0.0 (never) | 0.0005 | yes |
| Detail | 2 | struts, knobs, rotor | 0.012 | 2.0 | 0.128 | 0.02 | 0.05 | 0.002 | 0.0 (never) | no |
| Interior | 3 | battens, tub, liner, seat | 0.025 | 4.0 | 0.256 | 0.01 | 0.04 | 0.004 | 0.0 (never) | no |
| Emitter | 4 | nav glow, rotor glow | 0.020 | 2.5 | 0.16 | 0.02 | 0.0 (no RT geometry) | 0.001 | 0.0 (never) | no |

`lod_error_scale` multiplies the baker's structural base row
`[0.0, 0.001, 0.004, 0.016, 0.064]`. Error budgets are fractions of the
part's bounding-box max extent and become the object-space errors the task
shader projects to screen space. Triangle ratios are still per-class tables
because they are not scalar multiples of one another. `drop_px` and
`impostor_px` are projected bounding-sphere radii in NDC half-height units.
`TESS_REL_ERROR = 0.004` is layered on top of the absolute tess error so
large smooth radii do not demand absurd segment counts: effective epsilon is
`max(absolute, radius * TESS_REL_ERROR)`.

Semantic split examples (from `airframe.rs`):

- Hull is shell (Silhouette) + fairings (Structural) + longerons (Structural) +
  stringers (Interior), not one monolithic mesh.
- Wing per side is sail + leading edge + tip + battens (Interior) + binding
  (Structural) + glow (Emitter) + flap surfaces (Structural) + flap hardware
  (Detail).
- Tail fins are outline extrude (Silhouette) + external spar tubes
  (Structural). The spar sits just outside the fin slab and reads as fin
  structure from outside, so it keeps fixed tessellation instead of the
  interior-classified adaptive minimum.
- Nozzle petals are Structural with columns derived from the parabola sagitta
  and adaptive rows.

## Adaptive primitive tessellation

`circle_segments(radius, eps)` computes segment count from the chordal
(sagitta) formula and clamps to 4..64. `adaptive_segments(sample, eps, min, max)`
refines an interval by midpoint deviation until the error budget is met or the
max is reached, then resamples uniformly at the final count. Applied to:

- hull profile rings (16..64),
- sail and flap grid rows/columns,
- petal cross-section columns and longitudinal rows,
- tube path segments and radial counts (`tube_fit`),
- ellipsoid lon/lat (`ellipsoid_fit`).

Straight stretches stay at the minimum; curved regions refine. No per-frame
tessellation is added; all of this runs in `airframe-baker` at build time.

## Part behaviour flags

`RawPart.flags` is a `PartFlags` bitset: `CAST_RT`, `ALLOW_IMPOSTOR`,
`PRESERVE_ANIMATION`, `TWO_SIDED`, `ALPHA`, `SILHOUETTE_CRITICAL`.
`PartFlags::for_part(mat, importance)` sets defaults. Material category does
not decide behaviour by itself: the baker reads `ALPHA` for the glass split
and `CAST_RT` for shadow-proxy membership instead of matching `MatId::Glass`
or special-casing emitters. `ALLOW_IMPOSTOR` is set only for silhouette and
structural classes, which is the set that may later cross over under the
reserved `FEATURE_IMPOSTOR_LODS` bit.

## LOD and RT policy

`build_part_lods` still indexes per-class triangle-ratio tables, but error
budgets are `BASE_LOD_ERROR_BUDGET * LodPolicy::lod_error_scale` and the RT
proxy takes `LodPolicy::rt_density` directly. QEM simplification is unchanged;
only the per-level targets differ. Emitter parts get no RT proxy
(`rt_density = 0.0`; soft-shadow rays do not need a 4 cm glow blob) as long
as every animation node keeps some other RT geometry. All 23 nodes still keep
an RT geometry range (`rt_nodes == 23` is locked).

## Task-shader screen-size cull

`engine/shaders/plane.task` reads importance from `PartDesc::flags` and drops
a part when its projected bounding-sphere radius (in NDC half-height units)
falls below:

| Importance | Min screen radius |
| --- | ---: |
| Interior | 0.004 |
| Detail | 0.002 |
| Emitter | 0.001 |
| Structural, Silhouette | never dropped |

0.004 is about two pixels at 1080p, matching `SSE_THRESHOLD`. Glass stays on
the legacy two-draw path and is unaffected.

## What was deliberately deferred

- Full wing vertex-sharing and left/right instancing needs format v3. The
  legacy `plane.vert` path reads the node from the vertex stream, flex sign
  differs per side, and RT instance matrices are per-node. Packing shared
  vertices would break those readers without a version bump.
- Reshadeable impostors (`FEATURE_IMPOSTOR_LODS`, reserved bit 3) are not
  implemented. Far behavior is delivered by importance LOD collapse plus the
  task-shader screen cull instead. RiLoD-style impostors remain future work
  under the reserved feature bit.

## Validation

```sh
cargo test -p airframe -p airframe-format -p airframe-baker --locked
cargo test --workspace --locked
python3 tools/check_shaders.py   # 160 SPIR-V modules, vulkan1.3
cargo run -p airframe-baker --locked -- /tmp/airframe.bin --dump-stats --analyze
```

Locked topology test: 24,486 triangles, 15,341 vertices, 23 RT nodes.
`semantic_parts_separate_silhouette_from_hidden_detail` asserts every importance
class appears and that wing/flap nodes carry interior parts.
`part_flags_carry_importance_and_glass` asserts packed flags round-trip and
that importance triangle sums equal the total.
`interior_budgets_are_looser_than_silhouette_budgets` locks the budget ordering.

No GPU visual acceptance was run for this change. Wireframe counts and LOD
error budgets are offline measurements only. Screenshot or GPU-timing claims
require the target RTX 4060 Laptop GPU and the frame-budget procedure in
[frame-budget-2026.md](frame-budget-2026.md).

## Primary references

Accessed 2026-09-22:

- Microsoft, [Aircraft Modeling General Principles](https://docs.flightsimulator.com/msfs2024/html/3_Models_And_Textures/Modeling/Aircraft/Aircraft_Modeling_General_Principles.htm)
  (MSFS 2024 SDK). Modular per-part LOD selection driven by projected bounding
  sphere size on screen, with recommended triangle budgets per screen-size
  band. This is the industry practice our per-part importance LOD chain
  follows; it does not measure our asset.
- Microsoft, [LOD Selection System](https://docs.flightsimulator.com/msfs2024/html/3_Models_And_Textures/Modeling/LODs/LOD_Selection_System.htm)
  (MSFS 2024 SDK). minSize as a percentage of vertical screen size of the
  bounding sphere; effective minSize is the max of XML value and a
  vertex-count-driven curve. Foundational product documentation, not a paper.
- Kuth et al., [Real-time meshlet decompression](https://doi.org/10.1016/j.cag.2025.104292),
  *Computers & Graphics* 2025 (also VMV 2024 preprint as "Towards Practical
  Meshlet Compression", [arXiv:2404.06359](https://arxiv.org/abs/2404.06359)).
  Recent applied work on meshlet codecs for mesh-shader decompression. We do
  not compress meshlets; we cite it only for the meshlet representation our
  baker already emits (64 verts / 126 tris).
- Mlakar, Steinberger, Schmalstieg, [End-to-End Compressed Meshlet Rendering](https://doi.org/10.1111/cgf.15002),
  *Computer Graphics Forum* 43(1), 2024. Foundational recent work on keeping
  meshes compressed through the mesh-shader path. Context for meshlet residency
  limits; not implemented here.
- Wu, Zeng, Zhu, Wang, [Reshadable Impostors with Level-of-Detail for
  Real-Time Distant Objects Rendering](https://doi.org/10.1111/cgf.70183),
  *Computer Graphics Forum* 44(4), EGSR 2025. Recent impostor/LOD paper. Cited
  only as the deferred alternative to our far-LOD collapse under the reserved
  `FEATURE_IMPOSTOR_LODS` bit; not implemented and not state of the art by
  citation alone.
- NVIDIA, [Introduction to Turing Mesh Shaders](https://developer.nvidia.com/blog/introduction-turing-mesh-shaders),
  2018/2023. Meshlet limits of 64 vertices and 126 primitives; task-shader
  cluster culling and LOD selection. Vendor guidance our baker and
  `plane.task` follow.
- NVIDIA, [Using Mesh Shaders for Professional Graphics](https://developer.nvidia.com/blog/using-mesh-shaders-for-professional-graphics),
  2020. Task-shader culling with bounding shape and normal cone; one meshlet
  per thread. Matches our `plane.task` structure.
- Khronos, [`VK_EXT_mesh_shader`](https://registry.khronos.org/vulkan/specs/latest/man/html/VK_EXT_mesh_shader.html).
  Task and mesh stages; capability-checked with the legacy two-draw fallback.
  Accessed 2026-09-22.
