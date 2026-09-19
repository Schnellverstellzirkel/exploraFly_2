# Alpine terrain milestone

This replaces the analytic flat ground with a rasterized alpine world: winding
meadow valleys, steep ridged mountains up to 3,123 m, snow above a varied tree
line, turquoise lakes at 185 m, and coarse medieval settlements. Each settlement
has a keep, four towers, four curtain walls, and fifteen gabled houses. A pale
track follows the valley bank. Forests are material patches in this milestone;
individual trees, interiors, inhabitants, and gameplay objectives are not present.

## Research and decisions

Primary sources checked online on 2026-09-19:

- [Asirvatham and Hoppe, GPU-based geometry clipmaps, GPU Gems 2](https://developer.nvidia.com/gpugems/gpugems2/part-i-geometric-complexity/chapter-2-terrain-rendering-using-gpu-based-geometry)
  establishes the value of a bounded regular mesh, immutable topology, and
  progressively coarser distant sampling. Its full clipmap method requires ring
  stitching and elevation/normal caches. This milestone instead uses one
  continuous exponential lattice, avoiding ring T junctions and cache updates.
  This is a simpler engineering tradeoff, not an implementation of that paper.
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

The 64 by 64 cell lattice emits 24,576 vertices / 8,192 triangles. Axis spacing
grows exponentially from 8.32 m beside the camera to a 51.98 km outer radius.
The same draw emits at most 11,664 landmark vertices / 3,888 triangles in the
nearest nine settlement cells; landmarks beyond 14 km collapse outside the clip
volume before evaluating their height. Total submitted work is fixed at 36,240
vertices / 12,080 triangles. There are no per-frame terrain allocations, CPU
mesh uploads, terrain textures, compute dispatches, or extra draw submissions.

Terrain height is sampled in the vertex stage. The fragment shader shades the
rasterized position, so mountains have silhouettes and occlude the aircraft
through the ordinary depth buffer. Removing `gl_FragDepth` permits normal early
depth rejection. Five bounded value-noise evaluations shape each vertex; no
heightfield ray marching, iterative normal reconstruction, or terrain ray query
is performed per pixel. Existing PBR sun/sky lighting, filtered material detail,
atmospheric perspective, and cloud shadows are retained. Lake reflections use
the sky gradient; they do not reflect nearby mountain geometry.

The topology is non-indexed to reuse the existing empty vertex input and single
draw path. Duplicate vertex evaluation is a measurable tradeoff. If target-GPU
timings show vertex cost matters, an immutable index buffer is the first obvious
optimization; cached clipmaps are a later option. Neither is asserted faster
without measurement. The 1,000 real-presentation-FPS goal remains unverified.

The lattice follows the camera smoothly, so distant terrain is an approximation
that can slowly change its triangulation as the camera moves. There are no
independent LOD seams, but there is also no terrain-error metric or geomorphing.
Far geometry is intentionally coarse and must be inspected on the target GPU.
The pre-existing 30 km camera far plane may clip the mesh earlier than its outer
radius; a 60 km far plane uses the available range without altering the mesh.
This is suitable for a first flight milestone, not a claim of finished terrain
streaming or ground-level walking fidelity.

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
device.cmd_draw(cmd, world::DRAW_VERTEX_COUNT, 1, 0, 0);
```

Use `world::collision_height_at(absolute_x, absolute_z)` for a conservative
flight/camera floor. It includes terrain, water, and the highest roof across each
building footprint. Add `world::CLEARANCE_METRES` (45 m) for aircraft clearance;
this margin also covers the nearby lattice's interpolation error. Collision is
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

Verified on 2026-09-19 with the `explorafly-check` container: all 56 workspace
tests passed (including seven world tests), and all 24 SPIR-V modules validated.
The host does not have native Cargo; the container image also lacks rustfmt, so
no rustfmt success is claimed.

Windows-host CPU/shader verification cannot establish image quality or target
frame time. Acceptance on the Linux/NVIDIA machine must inspect low flight over
the lake shore, the castle/village approach, steep banks, snowy silhouettes,
camera-origin rebases, and distant mesh movement. Run the existing sustained
boost/hard-bank benchmark with the performance preset and report successful
presentation throughput and GPU stage timings; do not substitute draw rate or
geometry-only burst counts for real presentations.
