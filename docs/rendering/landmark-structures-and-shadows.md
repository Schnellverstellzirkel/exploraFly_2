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
representing all 48 structures across 16 settlements in one 65,536 m `WORLD_PERIOD`:
- **Wall Boxes**: 12 triangles (36 vertices) per structure, scaled to foundation
  elevation, wall height, and horizontal extents.
- **Roofs**: 6 triangles (18 vertices) per structure, with a 1.0 m eaves overhang
  extending beyond the wall perimeter.
- **Detail Boxes A/B/C**: 12 triangles (36 vertices) each, per structure. The tables in
  `world::detail_a()`, `world::detail_b()`, and `world::detail_c()` place ridge
  chimneys on houses, a beacon block on the keep, corbelled gallery bands on the
  towers, the chapel belfry and west porch, the barn's hay-loft cupola and long
  lean-to, the watermill's flue and paddle wheel, a mid-wall jettied upper storey
  on every house and the tavern, the granary loft and staddle plinth, the
  watchtower gallery, broken ruins teeth and rubble pile, the windmill cap
  gallery, the shrine altar block and plinth, the well seat, the stilt hut
  smoke flue and deck, the High Alpine Cloister arcaded belfry and courtyard ambulatory,
  the Almhütte chimney and Schrot veranda, the Stone Bridge approach abutments and
  cutwater river piers, the Mountain Signal Fire beacon cage, the Meadow Hay Barn
  ramp and gable ventilator, the Wayside Shrine stepped plinth, the Castle Barbican
  twin guard turrets and portcullis passage, the High Pass Watch-Post lookout turret,
  and the Lakeside Boat House slipway and mooring pier. Absent add-ons collapse to
  degenerate triangles at the foundation center.
- **Roofline Band**: 12 triangles (36 vertices) per structure. `world::detail_d()`
  sits a band just proud of the wall at the roof seat: a machicolation corbelled
  band on fortifications, a broad dark timber fascia on timber and plain stone
  buildings.
- **Roof Tips**: 8 triangles (24 vertices) per structure. `world::tip()` emits either
  a spire octahedron standing on the ridge (keep flagpole, tower spires, chapel
  spire, cloister spire, beacon cage, barn and shrine finials, barbican and watchtower
  spires) or the windmill's four-sail cross on the south face, built from eight fold-triangles.
- Total vertices: $16 \times 48 \times 222 = 170,496$ vertices ($56,832$ triangles),
  consuming 2,046 KiB of vertex data.

The GPU decode in `engine/shaders/ground.vert` reproduces this corner order
pixel-identically from the `vType` structure index, `vPart` submesh id (0 wall,
1 roof, 2/3 detail A/B, 6 detail C, 7 roofline band, 4 spire, 5 sails) and the
`vShape = (wallHeight, roofHeight)` tuple, so the rasterized geometry and the
ray-trace acceleration geometry never diverge.

### Ray Tracing Pipeline Integration (`engine/src/plane.rs`)

1. **BLAS Build**: A dedicated bottom-level acceleration structure is built alongside
   the 23 airframe nodes and 2.1M-triangle terrain mesh using `vk::IndexType::NONE_KHR`
   with `PREFER_FAST_TRACE`.
2. **TLAS Placement**: An instance (custom index 101, mask `0x10`) is updated per frame
   with floating-origin coordinate alignment matching the terrain instance.
3. Total acceleration structures: 23 airframe BLAS + terrain (2.1M tris) + structures (31,200 tris),
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

### Ashlar Stone Masonry (`vMaterial == 1u`, fortifications and civic stone)

- **Course Staggering**: 1.2 m vertical course height with alternating 2.4 m block
  lengths offset by 1.2 m.
- **Edge Bevels**: Distance to block boundaries forms rounded edge bevels that
  slope normals inward toward recessed mortar joints ($d_{bu}, d_{bv}$).
- **Rock Texture Integration**: `detail_rock_nor_tex` and `detail_rock_diff_tex`
  are sampled in wall tangent space (`T_wall`, `B_wall`), adding micro-chiseled
  rock texture that fades smoothly with pixel footprint.
- **Foundation Contact AO & Arrow Slits**: Contact ambient occlusion darkens the
  base of walls ($y < 3\text{ m}$) with soil dampness. Arrow-slit windows
  puncture tall fortification walls only (`vType <= 8 || 34`); village and civic
  stone walls stay un-pierced. The fortification roofline band (`vPart == 7u`)
  reads as machicolation: slat-edge highlights step out between dark corbel
  brackets that cast onto the wall beneath the band.

### Half-Timbered Village Walls (`vMaterial == 1u`, timber structures)

The thirteen houses, barn, tavern, granary, and stilt hut carry a fachwerk pass that
replaces ashlar on their wall faces:
- **Bay Posts and Rails**: 3.0 m wall bays and 2.0 m storeys define dark oak
  members over plaster infill panels, staggered per storey, with a diagonal
  brace in the outer half of every other 6.0 m bay group.
- **Windows**: a dark-leaded window lights the storey middle of every other
  panel, reading as inhabited rather than painted.
- **Jetty soffit**: the underside of the jettied upper storey (`vPart == 6u`)
  carries a dark timber soffit with mossy hash, so the overhang casts a real
  shadow onto the street below.
- **Roofline fascia**: the same eave-seat band (`vPart == 7u`) is a broad dark
  timber fascia over the wall top instead of castle corbels.

### Layered Alpine Shingles (`vMaterial == 2u`)

- **Sawtooth Relief**: 0.40 m row spacing with sawtooth normal tilt along slope
  tangents ($B_{\text{slope}}$) steps back sharply at the lip of each shingle course.
- **Staggered Courses**: 0.30 m horizontal tile width with alternating row offsets.
- **Palette Differentiation**:
  - Castle towers and keep ($y > 50\text{ m}$): weathered alpine slate and zinc gray.
  - Village houses ($y \le 50\text{ m}$): warm terracotta tiles and aged cedar shingles.
- **Crevice Ambient Occlusion**: Shadowing at shingle overlap lips ($AO \approx 0.65$)
  enhances tangible depth in aerial and close-up views.

### Spire and Sail Tips (`vMaterial == 2u`, `vPart 4/5`)

- **Spires**: weathered slate with faint vertical rib bands and a chalky
  windward patina over the octahedron; the tips read as wrought finials against
  the mountain sky.
- **Windmill Sails**: stained off-white canvas over the four arms, slat ribs
  worn darker toward the mast, a dark bolt ring around the hub, weathering
  darker toward the mast and frame.

## Procedural Scatter Decode and Rich Alpine Biome Variation

Trees and boulders share the structure stage's vertex budget. Each of the 121×121
scatter slots (24 m pitch, spanning roughly 2.9 km around the camera) owns 108
corners:
- **Trunk box**: 36 corners. Spruces and broadleaf trees get a dark vertical-grain
  bark body (`vMaterial == 4u`) with birch slivers on broadleafs and furrowed red-brown
  bark on larches; boulders reuse the central box as their primary mineral monolith (`vMaterial == 5u`).
- **Three crown octahedra**: 24 corners each (72 corners total per slot):
  - **Tree Canopies**: Stacked half-widths and centres give conifers a tapering
    spire silhouette (2.9/2.0/1.2 size widths, 26 size apex) and broadleafs a broad
    low crown with rounded upper storeys (7.0/5.0/3.0 widths, 21.2 size apex).
  - **Glacial Erratic Formations**: Instead of collapsing unused crown corners to
    zero, boulder slots repurpose all 72 vertices into three flanking satellite
    boulders and weathered talus blocks clustered at horizontal offsets around
    the central monolith (`c0off`, `c1off`, `c2off`).
- **Foliage Coloration and Alpine Flora Species**:
  Scatter slots use `vType = 50u` (trees) and `51u` (boulders), strictly preserving the 0..47 landmark structure ID space. Species selection mirrors `world::scatter_slot` hash-for-hash across five distinct archetypes:
  - Norway Spruce (`kind 0.0`): deep emerald subalpine needles, conical spire tiers.
  - Mountain Broadleaf (`kind 1.0`): beech, birch, maple with russet/crimson autumn accents.
  - Alpine Larch (`kind 2.0`, *Larix decidua*): radiant golden-amber autumn foliage with tiered spreading boughs.
  - Swiss Stone Pine (`kind 3.0`, *Pinus cembra* / Zirbe): gnarled high-altitude hardy pine with blue-green needles.
  - Dwarf Mountain Shrub / Alpenrose (`kind 4.0`, *Rhododendron ferrugineum*): subalpine cushions with magenta blooms.
  Each owns tailored crown tiers and proportions mirroring the collision radii in `world::scatter_slot`.
- **Faceted Glacial Erratics**: Boulders deploy their three crown octahedron slots as
  interlocking faceted limestone crag shards and satellite talus boulders around the
  main block foundation. Shaded bases develop velvet green moss while sunward facets
  accumulate golden and orange crustose lichen (Rhizocarpon geographicum, Xanthoria elegans).
- **Canopy Backlight Translucency / Forward Scattering**: Thin foliage leaves, pine needle
  clusters, and golden larch canopies transmit sunlight forward when backlit by the sun
  (`pow(max(dot(-v, sun), 0.0), 3.2)`), radiating warm golden-amber and emerald light
  rather than falling into pitch-black cutouts against the sky.
- Crown corners (`vPart == 7u`) alone take the canopy sway pass keyed to the
  slot hash; trunk corners (`vPart == 8u`) and boulders stand fast.

## Verification and Performance

Measured on the target RTX 4060 Laptop GPU at native 2880×1646 resolution:
- **Presentation Submissions**: 119.1 presents/s (~8.36 ms GPU per frame) at full 2880×1646
  native resolution with ray-traced landmark and aircraft shadows active.
- **CPU Clearance & Shader Parity**: All 108 tests pass (`cargo test --workspace --locked`),
  including the 5×5 neighbor scatter reach, wrap tests across 65,536 m period boundaries,
  and dynamic flight clearance sweeps over spawn valleys and ridgelines.
- **Shader SPIR-V Validation**: Offline Vulkan compilation confirms zero errors across all
  modules in the shader pipeline.
