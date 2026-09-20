# Landmark Structures Ray Tracing and 3D Architectural Shading

## Architectural Context and Objectives

Free flight across the alpine fantasy world features medieval landmark settlements:
a central fortified castle (keep, four 175m corner towers, four curtain walls)
surrounded by thirteen timber and stone village houses plus a chapel, barn,
watermill, well house, tavern, and granary, a six-stone meadow circle, a
hillside watchtower, a ruined tower, a windmill, a mountain shrine, and a
lakeside stilt hut.

Previously, these structures appeared visually flat:
1. They had zero presence in the ray-tracing acceleration structures (BLAS/TLAS).
   Neither buildings nor adjacent terrain received cast shadows from towers,
   walls, or roof overhangs.
2. Building walls (`vMaterial == 1u`) and roofs (`vMaterial == 2u`) lacked 3D normal
   perturbation, rendering as flat mathematical polygons with uniform lighting.
3. Shading normals lacked ashlar block bevels, rock surface roughness, shingle
   relief, and contact ambient occlusion.

This subsystem provides hardware ray-traced shadows for all landmark structures
via `VK_KHR_ray_query`, together with 3D ashlar masonry and layered alpine
shingle surface shading.

## Primary References and Technical Foundations

- **Khronos Group (2020)**. *Vulkan Extension Specification: `VK_KHR_ray_query`*.
  [https://registry.khronos.org/vulkan/specs/1.3-extensions/man/html/VK_KHR_ray_query.html](https://registry.khronos.org/vulkan/specs/1.3-extensions/man/html/VK_KHR_ray_query.html).
  Accessed 2026-09-20. Used for inline ray query execution in `ground.frag` and `ground-rt.frag`.
- **Pharr, M., Jakob, W., & Humphreys, G. (2016)**. *Physically Based Rendering: From Theory to Implementation (3rd ed.)*.
  Morgan Kaufmann. Chapters 2-4 (Geometry & Acceleration Structures) and Chapter 14 (Light Transport).
  Applied for normal-aligned ray bias offset ($t_{\text{min}}$ and probe origin shift) to avoid self-intersection while resolving sub-metre contact shadows.
- **Zirr, T., & Kaplanyan, A. S. (2016)**. *Real-time Multiscale Material Shading*.
  Proceedings of the ACM SIGGRAPH Symposium on Interactive 3D Graphics and Games (I3D 2016).
  [https://doi.org/10.1145/2856400.2856409](https://doi.org/10.1145/2856400.2856409).
  Applied for pixel-footprint-dependent geometric slope fade into roughness.

## Landmark Geometry and Acceleration Structures

### BLAS Geometry Generation (`crates/world/src/lib.rs`)

`world::landmark_structure_triangles()` generates non-indexed float triangles
representing all 39 structures across 16 settlements in one 65,536 m `WORLD_PERIOD`:
- **Wall Boxes**: 12 triangles (36 vertices) per structure, scaled to foundation
  elevation, wall height, and horizontal extents.
- **Roofs**: 6 triangles (18 vertices) per structure, with a 1.0 m eaves overhang
  extending beyond the wall perimeter.
- Total vertices: $16 \times 39 \times 54 = 33,696$ vertices ($11,232$ triangles),
  consuming 394.9 KiB of vertex data.

### Ray Tracing Pipeline Integration (`engine/src/plane.rs`)

1. **BLAS Build**: A dedicated bottom-level acceleration structure is built alongside
   the 23 airframe nodes and 2.1M-triangle terrain mesh using `vk::IndexType::NONE_KHR`
   with `PREFER_FAST_TRACE`.
2. **TLAS Placement**: An instance (custom index 101, mask `0x10`) is updated per frame
   with floating-origin coordinate alignment matching the terrain instance.
3. Total acceleration structures: 23 airframe BLAS + terrain (2.1M tris) + structures (11,232 tris),
   513.0 KiB caster memory, 143.4 MiB structure memory.

## Ray-Traced Shadow Gating and Probe Bias (`engine/shaders/ground.frag`)

Ray query evaluation in `ground.frag` uses three differentiated probe paths:

1. **Aircraft Shadow**: Gate `rtShadowGate(local_xz, hit.y) > 0.0` restricts aircraft
   shadow tracing to the projected ellipse penumbra.
2. **Structure Self-Shadowing and Cast Shadows (`vMaterial != 0u`)**:
   - Building walls and roofs are evaluated with probe offset `probe = hit + n * 0.18`.
   - Normal bias of 18 cm starts rays outside the wall face.
   - $t_{\text{min}} = 0.05\text{ m}$ (5 cm) allows the 1.0 m roof overhang directly
     above wall pixels to cast sharp shadow eaves, while towers cast long shadows
     across curtain walls and courtyards.
3. **Settlement Ground Cast Shadows (`vMaterial == 0u`)**:
   - Evaluated within a 450 m radius of the settlement center when $hit_t < 3500\text{ m}$.
   - Ground probe `probe = hit + vTerrainNormal * 0.7` with $t_{\text{min}} = 0.35\text{ m}$
     and $t_{\text{max}} = 800\text{ m}$ captures castle towers, curtain walls, and houses
     casting realistic shadows across village paths, courtyards, and pastures.

## 3D Architectural Surface Shading

### Ashlar Stone Masonry (`vMaterial == 1u`)

- **Course Staggering**: 1.2 m vertical course height with alternating 2.4 m block
  lengths offset by 1.2 m.
- **Edge Bevels**: Distance to block boundaries forms rounded edge bevels that
  slope normals inward toward recessed mortar joints ($d_{bu}, d_{bv}$).
- **Rock Texture Integration**: `detail_rock_nor_tex` and `detail_rock_diff_tex`
  are sampled in wall tangent space (`T_wall`, `B_wall`), adding micro-chiseled
  rock texture that fades smoothly with pixel footprint.
- **Foundation Contact AO & Lancet Windows**: Contact ambient occlusion darkens the
  base of walls ($y < 3\text{ m}$) with soil dampness, while dark recessed lancet
  windows puncture tall fortification walls.

### Layered Alpine Shingles (`vMaterial == 2u`)

- **Sawtooth Relief**: 0.40 m row spacing with sawtooth normal tilt along slope
  tangents ($B_{\text{slope}}$) steps back sharply at the lip of each shingle course.
- **Staggered Courses**: 0.30 m horizontal tile width with alternating row offsets.
- **Palette Differentiation**:
  - Castle towers and keep ($y > 50\text{ m}$): weathered alpine slate and zinc gray.
  - Village houses ($y \le 50\text{ m}$): warm terracotta tiles and aged cedar shingles.
- **Crevice Ambient Occlusion**: Shadowing at shingle overlap lips ($AO \approx 0.65$)
  enhances tangible depth in aerial and close-up views.

## Verification and Performance

On the target RTX 4060 Laptop GPU at native 2880×1646 resolution:
- **Presentation submissions**: 155.5/s.
- **Pass attribution**:
  - `opq+rt`: 254 µs.
  - `ter` (terrain + structures + detail maps): 3,461 µs.
  - `cld` (geometric cloud mesh): 1,205 µs.
  - `sky`: 151 µs.
  - `plu`: 17 µs.
  - `cmp` (FXAA + composite): 1,349 µs.
- Workspace validation: `cargo test --workspace --locked` passes 95 unit tests (37 engine, 46 sim, 12 world) with zero warnings or errors.
