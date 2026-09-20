# Midair clearance jumps during turns

Verified 2026-09-20 with the CPU flight/world integration tests. This fixes
collision queries; the rendered terrain, flight handling, and existing
45 m exploration safety margin are unchanged by this fix.

`engine/src/frame_loop.rs` raises the aircraft immediately when its altitude
falls below `world::collision_height_at(x, z) + CLEARANCE_METRES`. The camera
also uses that world query. Incorrect tree heights therefore act as invisible
obstacles even when the aircraft is hundreds of metres above the landscape.

The CPU scatter query used a 24 m tree-slot index to address the 64 m terrain
lattice. It could take heights and biome gates from a distant mountainside
while the vertex shader placed the visible tree on the local valley floor.
It also omitted the shader's 16-bit slot-hash mask on negative coordinates and
used one corner's height instead of the rendered triangle under the tree.

The corrected query follows `engine/shaders/ground.vert`:

- Mask the slot hash inputs to 16 bits before selecting placement and species.
- Derive the terrain cell from the jittered world position for biome gates.
- Interpolate the water-clamped terrain triangle for the tree's base height.
- Normalize periodic terrain cache keys, avoiding the empty-cache sentinel
  collision at cell `(-1, -1)`.
- Check a bounded 5x5 slot neighbourhood so placement jitter, crown radius,
  and the existing aircraft padding are all covered.

Before the fix, repeated bank inputs at the default spawn with wind strength
1 produced clearance jumps as large as 163.14 m for bank -1 and 76.53 m for
bank +1 during separate 30-second replays. Each replay held bank for 100 fixed
steps out of every 576, leaving pitch, yaw, and boost neutral. Both replays
produced zero clearance jumps after the fix. A valley sample at world
`(1140, 1700)` previously returned a collision height of 1585.23 m over terrain
at 255.44 m.

`engine/tests/flight_clearance.rs` checks both turns over the first 20 seconds,
requiring at least 250 m of terrain clearance and no intersection with the
collision floor. `crates/world/tests/scatter_clearance.rs` checks lake exclusion,
triangle base heights, signed/hash-wrapped coordinates, and crown coverage.
All five regression tests failed before the correction and pass afterward.

These are CPU correctness checks, not target-GPU visual or presentation-rate
acceptance. The floor remains a forgiving canopy/roof envelope rather than
detailed aircraft collision geometry.
