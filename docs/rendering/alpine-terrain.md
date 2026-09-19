# Alpine terrain milestone

This replaces the analytic flat ground with a rasterized alpine world: winding
meadow valleys, steep ridged mountains up to 3,123 m, snow above a varied tree
line, turquoise lakes at 185 m, and coarse medieval settlements. Each settlement
has a keep, four towers, four curtain walls, and fifteen gabled houses. A pale
track follows the valley bank. Forests are material patches in this milestone;
individual trees, interiors, inhabitants, and gameplay objectives are not present.

## Research and decisions

Primary sources checked online on 2026-09-19:

- [Malyshau, Six Ways to Draw Vangers](https://arxiv.org/abs/2608.17390),
  submitted 2026-08-18 (preprint, not established peer-reviewed consensus).
  Its shared-data-path comparison finds that terrain methods which look similar
  from above differ markedly at eye-level horizons. The mesh result is fastest
  in that study, with substantial editable-geometry memory cost. This motivates
  explicit horizon/coverage acceptance and measured comparisons here, not a
  claim that this simpler single-layer lattice reproduces its results.
- [Goslin, InfiniteDiffusion](https://arxiv.org/abs/2512.08309),
  revised 2026-05-03. The abstract describes learned, seed-consistent, unbounded
  terrain generation with random access. It is a promising content-generation
  direction; it is not integrated here. No trained models or inference runtime
  are available in this engine, and generation throughput would not establish
  the one-millisecond complete-rendering budget.
- [Asirvatham and Hoppe, GPU-based geometry clipmaps, GPU Gems 2](https://developer.nvidia.com/gpugems/gpugems2/part-i-geometric-complexity/chapter-2-terrain-rendering-using-gpu-based-geometry)
  (published 2005, accessed 2026-09-19) motivates world-aligned regular samples,
  reusable topology, and cached elevation/normal data. This implementation uses
  one fixed-resolution grid, not the paper's nested clipmap levels or transitions.
  It trades more triangles for a surface that never changes with viewer position.
- [Khronos, current vkCmdDraw reference](https://docs.vulkan.org/refpages/latest/refpages/source/vkCmdDraw.html)
  specifies non-indexed draws and the vertex/instance counts used here. The
  existing raw ash/Vulkan pipeline can generate positions from `gl_VertexIndex`
  without a new vertex buffer, descriptor binding, or submission.
- [Chajdas, Optimizing Terrain Shadows, AMD GPUOpen](https://gpuopen.com/learn/optimizing-terrain-shadows/)
  explains that shadow geometry deserves its own budget. Accordingly this
  milestone does not insert the terrain into the aircraft acceleration structure
  or duplicate the entire mesh into a shadow pass. Existing aircraft and cloud
  shadows remain; terrain and buildings do not yet cast long-range shadows.

The older research is used for its still-relevant design reasoning; it is not
evidence of performance on an RTX 4060 Laptop. The API reference was checked
against the current documentation, while shaders still target Vulkan 1.3/SPIR-V
1.4 as the existing engine does.

## Geometry and frame budget

The old exponential grid followed the camera continuously, resampling heights
at different world positions every frame. Its distant triangles spanned kilometres;
face normals made both the facets and moving triangulation visible.

The replacement is a world-aligned 1,024 by 1,024 cell grid at 64 m spacing.
Only the submitted window moves, in whole cells. Retained triangles keep their
world vertices, heights, and diagonal even when crossing cell boundaries or
rebasing the floating origin. Its nearest edge remains over 32.7 km away.
The existing 30 km far plane measures view depth rather than radial distance;
extreme off-axis views can still reach the window boundary. There are no
distance-dependent geometry transitions within the window.

Relief follows a glacial trough cross-section (flat floor, steep sides) with
two larger controls: a 16 km massif envelope decides which ranges reach high
Alps and which stay rolling foothills, and a foothill belt adds pre-alpine
hills between floor and rock. Measured over the tile, about 35 percent of land
sits below 800 m, 49 percent in the 800 to 1,800 m montane band, and 16 percent
above 1,800 m, with peaks past 2,800 m kept for the snowline. Valley floors and
the lake basin are unchanged, so spawn clearance and the settlement site hold.

A 1,024-square RGBA32F image caches one 65.536 km period of height and surface
slopes at startup (16 MiB). Periodic central differences give shared vertex
normals; interpolation smooths terrain lighting while buildings retain face
normals. The vertex shader fetches a texel instead of regenerating terrain noise.
Water clamps vertex heights at 185 m. Landmarks still use the analytic recipe.
The fourth channel stores landform moisture in [0,1], baked once from the
smoothed heightfield: valley floors approach 1 through height above water,
steep ground sheds it through local slope, and hollows gain through profile
concavity. This is a local proxy for the topographic wetness index
ln(a/tan beta) (Beven and Kirkby 1979,
https://doi.org/10.1080/02626667909491834, accessed 2026-09-19): height above
water stands in for upslope contributing area and no flow routing runs on the
periodic tile, so it marks where water would sit, not a measured soil value.
Vegetation masks in `ground.frag` read this channel instead of noise
thresholds. The treeline sits near 1,500 m and shifts with moisture, matching
the thermal treeline limit modulated by drought reported by Korner and by
Xie et al. 2024 (https://doi.org/10.1111/gcb.17260, accessed 2026-09-19).

An immutable uint32 index range appended to the existing aircraft index buffer
reuses terrain vertices. Terrain and landmarks remain one draw: 6,303,120 indices,
2,101,040 triangles, and 1,062,289 addressable vertices including landmarks.
The terrain index range costs approximately 24 MiB. No terrain allocations,
height generation, or uploads occur per frame. This is substantially more
geometry than the old 12,080-triangle combined draw; it does not establish the
1 ms performance target. Chunk culling can reduce this cost without changing
the fixed world surface.

The 14 km landmark cutoff remains a fixed budget: large towers can appear at
that range. Terrain and buildings do not cast long-range shadows, and lake
reflections use the sky gradient rather than nearby geometry.

## Coordinates and flight integration

`crates/world` mirrors `engine/shaders/terrain.inc`. Heights are absolute metres
above sea level; the world repeats seamlessly after 65,536 m. The shader reduces
the existing split `groundOrigin` before adding camera-relative X/Z. CPU queries
reduce f64 absolute X/Z before converting to f32. Terrain therefore survives
floating-origin rebases without moving the physical world. Simulation positions
themselves are still f32; this does not extend their long-distance precision.

The existing UBO prefix remains unchanged: `groundBase.w` is sea level relative
to the floating origin and `groundOrigin` supplies absolute X/Z. A later HUD
uniform may be appended without changing the terrain interface.

Required renderer glue in `engine/src/plane.rs`:

```rust
device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.ground_pipeline);
device.cmd_bind_index_buffer(cmd, self.index_buffer, self.terrain_index_offset, vk::IndexType::UINT32);
device.cmd_draw_indexed(cmd, world::DRAW_INDEX_COUNT, 1, 0, 0, 0);
```

Use `world::collision_height_at(absolute_x, absolute_z)` for a conservative
flight/camera floor. It includes terrain, water, and the highest roof across each
building footprint, including the roof's 1 m overhang and 11 m glider-radius
padding. Add `world::CLEARANCE_METRES` (45 m) for aircraft clearance;
the collision floor also covers the rendered triangles where interpolation is
above the analytic terrain. Collision is
arcade clearance, not detailed mesh contact. The safe valley spawn is exposed as
`SPAWN_X`, `SPAWN_Z`, and `SPAWN_ALTITUDE` (0, 1,050, 1,100 m). Physical terrain
sampling (`height_at`) and visible water surface sampling (`surface_height_at`)
are also available separately.

## Verification and remaining acceptance

The CPU suite checks a spawn with over 500 m clearance, submerged lake beds and
flat water, bounded mountain relief including peaks above 2,800 m, negative and
large coordinates, seamless period boundaries, conservative castle roof
collision, and fixed vertex counts. Full workspace compilation compiles both
ordinary and ray-query ground variants. `tools/check_shaders.py` validates the
resulting SPIR-V for Vulkan 1.3.

Verified on the native Linux workstation on 2026-09-19: workspace tests,
release compilation, and SPIR-V validation. Tests cover the periodic cache,
wrapped normals, index bounds, window radius, and collision against the
rasterized surface. Moving-flight screenshots at frames 120 and 480 were inspected.

A short boosted straight-flight check (calm wind, Balanced, 2880×1646, RTX 4060
Laptop, NVIDIA 580.173.02, MAILBOX, one complete scene per present) measured
104.4 successful presentation submissions/s over 200 samples after 500 warmup
presents. Wall intervals: mean 9.580 ms, p50 9.807 ms, p95 18.055 ms,
p99 18.530 ms. Mean complete GPU frame time was 9.500 ms. This is a smoke
check, not a before/after benchmark or sustained performance acceptance; display
feedback was unavailable and the 1,000/s target was not met. Artifacts were
written under /tmp/explorafly-terrain-check.qqzdu5.

Further visual acceptance should include low shoreline flight, steep banks,
settlements, and extended travel. The fixed 64 m mesh still approximates the
continuous height recipe; this change removes camera-dependent resampling,
not all geometric approximation.
