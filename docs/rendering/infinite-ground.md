# Infinite Ground and the Terrain Handoff

The current scene renders a mathematically infinite flat ground at world
`Y = 0`. It is not a large mesh and it does not move with the aircraft.

`engine/shaders/sky.frag` reconstructs the normalized camera ray, solves

$$
t = \frac{y_{ground} - y_{camera}}{d_y}, \qquad
\mathbf{p} = \mathbf{o} + t\mathbf{d},
$$

and writes the projected hit depth with `gl_FragDepth`. The ground branch is
fused into the existing fullscreen sky draw, so the flat case costs no extra
geometry or draw call. The ground height is sent relative to the renderer's
floating origin; translating the origin therefore cannot make the plane drift.
Distances beyond the camera far range remain shaded and are faded into the
analytic atmospheric horizon instead of producing a visible hard cutoff.

## Why this is the right flat case

- An infinite plane is an implicit surface. Rasterizing a finite proxy adds
  triangles, clipping limits, edge cases near the horizon, and floating-point
  error without adding information.
- The depth write keeps the ground a real scene surface: the aircraft, future
  terrain patches, effects, and shadow receivers can occlude it normally.
- The fullscreen pass is already required for the sky, so the flat baseline is
  one constant-size draw and O(screen pixels), independent of world distance.

## Terrain roadmap

The flat fallback stays permanently useful as the far/background surface. Close
terrain should be layered over it in this order:

1. Use nested GPU geometry clipmaps for the camera neighborhood. Each level is a
   toroidally addressed regular height grid, with rings rather than duplicated
   interiors and a morph band between resolutions.
2. Keep height and material pages in a bounded GPU cache. Stream or synthesize
   pages by distance and view importance; do not rebuild CPU meshes whenever the
   aircraft crosses a cell boundary.
3. Add screen-space-error culling and, when the Vulkan target path is ready,
   task/mesh-shader amplification for the outer levels. The current raster
   fullscreen baseline does not require mesh-shader availability.
4. Use the same deterministic terrain sample for rendering, collision, and
   decoration. Store global cell coordinates separately from local `f32`
   positions so generated detail remains stable at arbitrarily large travel
   distances.

This combines the cheapest exact solution for the flat case with the proven
scalable structure for actual heightfields. It also avoids making Nanite-style
virtualized geometry a dependency before there is terrain geometry to stream.

## Research and production references

- [Losasso and Hoppe, *Geometry Clipmaps: Terrain Rendering Using Nested Regular Grids*](https://hhoppe.com/proj/gpugcm/)
- [Asirvatham and Hoppe, *Terrain Rendering Using GPU-Based Geometry Clipmaps* (GPU Gems 2)](https://developer.nvidia.com/gpugems/gpugems2/part-i-geometric-complexity/chapter-2-terrain-rendering-using-gpu-based-geometry)
- [Etienne, *Large-Scale Terrain Rendering in Call of Duty* (SIGGRAPH Advances 2023)](https://advances.realtimerendering.com/s2023/Etienne%28ATVI%29-Large%20Scale%20Terrain%20Rendering%20with%20notes%20%28Advances%202023%29.pdf)
- [Dimitrijević and Rančić, *High-performance Ellipsoidal Clipmaps* (Graphical Models 2023)](https://doi.org/10.1016/j.gmod.2023.101209)
- [Epic, *World Partition in Unreal Engine*](https://dev.epicgames.com/documentation/en-us/unreal-engine/world-partition-in-unreal-engine)
- [Epic, *Using Nanite with Landscapes*](https://dev.epicgames.com/documentation/en-us/unreal-engine/using-nanite-with-landscapes-in-unreal-engine)
- [Microsoft GDC, *Global Terrain Technology for Flight Simulation*](https://gdcvault.com/play/1013204/Global-Terrain-Technology-for-Flight)

The production systems above solve the larger problem differently: clipmaps
and hierarchical meshes solve scalable heightfield geometry; Call of Duty and
Unreal virtualize terrain data and stream only what matters; flight-simulation
systems additionally solve geospatial precision. None of those mechanisms is
needed to represent a perfectly flat plane, so they remain the next terrain
layer rather than burdening this baseline.
