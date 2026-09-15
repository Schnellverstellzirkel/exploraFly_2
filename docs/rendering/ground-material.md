# Filtered Procedural Ground Material

The flat ground is now more than a constant-colored horizon receiver. It is a
world-stable, filtered procedural meadow/soil material evaluated in
`engine/shaders/ground.frag`. The surface is still mathematically flat at
`Y = 0`; the apparent relief comes from a filtered shading normal, so this
stage does not pretend that a heightfield, collision mesh, or terrain shadow
map already exists.

## Render path

The renderer draws the analytic sky first, then draws the ground as a
horizon-clipped six-vertex screen-space pass. `ground.vert` evaluates the
analytic horizon at the two screen edges, so `ground.frag` is not invoked over
the sky half of the image. The fragment shader still intersects each camera
ray with the plane, writes the projected depth, and shades only hits. Its
separate pipeline uses depth test/write `LESS_OR_EQUAL`, allowing the airframe
and future close terrain geometry to occlude it normally. This costs one
constant-size draw and no terrain geometry regardless of flight distance.

The material is authored in linear HDR values and is assembled from these
world-space filtered channels:

| Scale | Channel | Visual role |
| ---: | --- | --- |
| 96 m | macro value | broad grass-to-soil drift |
| 24 m | patch value | exposed earth and damp regions |
| 6 m | clump value | meadow color breakup |
| 1.5 m | grain value | soil color and small-scale variation |
| 0.5 m | pebble value | sparse pale stones and roughness lift |
| 1.5 m | relief value | stable micro-normal only |

The resulting masks mix two dielectric albedo families (grass and soil), a
small pebble tint, distance-stable wetness darkening, roughness from about
`0.90` when dry to `0.34` when wet, and a restrained ambient-occlusion term.
The defaults are a plausible temperate meadow, not measurements of a
particular location or ground sample.

## Anti-aliasing and world stability

Each procedural octave uses a deterministic integer-hashed value-noise lattice
and estimates the camera-ray footprint with `dFdx`/`dFdy`. Once a pixel covers
the feature, the octave is blended toward its analytic mean instead of being
allowed to shimmer. This follows the important production lesson from
Zirr/Kaplanyan's real-time multiscale-material work: procedural detail needs
pixel-footprint filtering, not merely more octaves.

The floating-origin anchor is split into an integer count of 25 cm cells plus a
positive sub-cell remainder. The fragment shader reconstructs lattice cells
from that representation rather than adding a small hit position to a large
absolute coordinate. A `2^20`-cell hash wrap is a deterministic fallback
period (about 262 km at the anchor scale), not the visible texture scale.
This keeps the phase fixed while the aircraft crosses floating-origin updates.

## Light transport and interaction

- Sky diffuse light uses two evaluations of the analytic vertical sky curve
  around the filtered normal and is scaled by `PI` for irradiance. This is a
  deliberately cheap approximation of the full hemisphere integral, using the
  same zenith, horizon, and below-horizon radiance family as the sky pass.
- The sun is a separate direct light. Diffuse uses a Burley-style rough
  diffuse response; specular uses dielectric GGX with Smith masking and
  Schlick Fresnel. Wetness lowers roughness, so damp areas retain a broader,
  darker diffuse response and a tighter grazing highlight.
- The normal is a four-tap finite difference of the same filtered relief used
  for the material. It changes lighting only; the geometric plane remains
  flat, which avoids inventing collision or silhouette detail.
- AO is applied to indirect material response. It is intentionally not used
  to darken direct sunlight, matching the usual real-time PBR separation.
- The aircraft receives a soft analytic ground shadow: its world-space
  position is projected along the sun direction, and the ellipse expands with
  the sun's angular radius. This is a useful contact cue for the current flat
  scene, not a replacement for terrain self-shadowing or a virtual shadow map.
- View-distance extinction mixes the ground toward the same atmospheric horizon
  radiance as the sky, preventing an artificial far-plane color seam.

The standard PBR split between base color, roughness, normal, reflectance, and
AO follows [Filament's material model](https://google.github.io/filament/main/filament.html).
The implementation is intentionally compact: no texture assets, virtual
texture page table, temporal reservoir, or neural runtime dependency is added
for a flat surface.

## Measured realtime cost

On 2026-09-15, the release build was measured on the project's RTX 4060
Laptop GPU at 2880×1646, with a 120-frame benchmark and one presentation per
frame:

| Metric | Result |
| --- | ---: |
| Real presentation submissions | 1,686.4/s |
| Complete GPU interval | 259 µs |
| Sky + ground timestamp interval | 102 µs |
| Simulation + camera | 9.7 µs |

The sky+ground timestamp covers both draws, so it is not a separate ground-only
counter. The optimized path removed one relief octave per normal sample,
replaced the four-direction sky estimator with two vertical-curve
evaluations, and eliminated sky-side ground fragment invocations while
retaining all five visible material scales. The ground uses a 2×2 fragment
shading rate when `VK_KHR_fragment_shading_rate` exposes the pipeline feature;
the fallback is native shading on devices without it. The screenshot was
captured at the same native resolution; the result remains comfortably
realtime on the target GPU, but a future heightfield, shadow map, or dense
close-up displacement budget must be measured separately.

## Research-to-engine decisions

The current design is the smallest useful subset of several proven directions:

- NVIDIA's multiscale procedural-material work motivates filtered procedural
  octaves and stable transitions rather than unfiltered noise.
- Filament and production game materials motivate the linear HDR PBR channels,
  GGX/Fresnel response, roughness-driven reflections, and indirect-only AO.
- Unreal's [Nanite landscape](https://dev.epicgames.com/documentation/en-us/unreal-engine/using-nanite-with-landscapes-in-unreal-engine)
  and [Virtual Shadow Map](https://dev.epicgames.com/documentation/en-us/unreal-engine/virtual-shadow-maps-in-unreal-engine)
  architecture is relevant once this flat fallback is layered with streamed
  geometry and real terrain occluders.
- Call of Duty's large-scale terrain work and Frostbite's adaptive terrain
  tessellation point toward virtualized height/material pages, geometry
  clipmaps, and procedural shader splatting for the next terrain layer.
- Recent NVIDIA research on procedural enhancement and layered material
  generation is useful as an offline authoring direction, but its neural or
  content-generation machinery is not a justified runtime dependency for
  this deterministic flat baseline: [2025 detail enhancement](https://research.nvidia.com/publication/2025-07_generative-detail-enhancement-physically-based-materials),
  [2026 procedural data enhancement](https://research.nvidia.com/labs/rtr/publication/yu2026toward/).

## Known limits and next terrain step

This material is visually rich but not geographically authored. It has no
height variation, grass blades, stratified soil, rocks, puddle geometry, true
terrain-to-terrain shadows, or measured BRDF/texture set. The aircraft shadow
is the only explicit caster interaction. The next practical step is a bounded
camera-centered geometry/texture clipmap whose far rings hand off to this
analytic plane. Use the same deterministic cell sampler for its height,
material pages, collision, and decoration; then replace the analytic aircraft
shadow with cached shadow pages or ray queries where the hardware budget allows.

## References

- [Zirr and Kaplanyan, *Real-time Rendering of Procedural Multiscale Materials* (I3D 2016)](https://research.nvidia.com/publication/2016-02_real-time-rendering-procedural-multiscale-materials)
- [Google Filament, *Physically Based Rendering in Filament*](https://google.github.io/filament/main/filament.html)
- [Etienne, *Large-Scale Terrain Rendering in Call of Duty* (SIGGRAPH Advances 2023)](https://advances.realtimerendering.com/s2023/Etienne%28ATVI%29-Large%20Scale%20Terrain%20Rendering%20with%20notes%20%28Advances%202023%29.pdf)
- [Frostbite, *Adaptive Terrain Tessellation*](https://media.contentapi.ea.com/content/dam/eacom/frostbite/files/adaptive-terrain-tessellation.pdf)
- [Patry, *The Rendering of Ghost of Tsushima* (SIGGRAPH Advances 2021)](https://advances.realtimerendering.com/s2021/jpatry_advances2021/index.html)
