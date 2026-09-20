# Weather and image quality

The native renderer has three startup presets, selected with
`EXPLORA_QUALITY=performance|balanced|cinematic`. Performance is the default
so the stock 120 Hz display path retains motion headroom; balanced and
cinematic remain explicit quality choices.
Settings persist through window resizing and are printed at startup.

Balanced renders the scene at the window resolution, with full-rate ground,
plume, and composite shading, 2x2 sky shading, and eight IBL samples.
Cinematic shades all of those passes at full rate and uses sixteen IBL samples.
Performance retains the previous 80% scene dimensions and coarse shading rates.
The explicit `EXPLORA_IBL_SAMPLES` override still takes priority.

## Clouds

The volumetric ray-marched decks were removed. Clouds are now real mesh
geometry, built and drawn the same way as the procedural terrain landmarks:
one fixed non-indexed draw whose vertex shader decodes `gl_VertexIndex` into
(grid cell, puff, triangle corner) and derives every position from hashed
absolute world cells (`cloud.vert` + `cloud.inc`). There is no cloud state,
no per-frame buffer work, and no simulation bookkeeping; the draw submits
11,089,920 corners (19x19 cells within 20 km of the camera, at most eight
three-subdivision icosphere puffs — 1280 triangles each — per cloud) and
empty, too-distant, or truncated clusters degenerate in the vertex stage.
With the measured ~44% cell occupancy the pass averages under 1.7 M
triangles, inside the terrain budget it shares the frame with.

Each occupied cell grows one puff cluster whose layout is a random gather,
not a formula: puff directions are hashed, area-uniform points over the
upper hemisphere (puff 0 always crowns the top), and a per-family spread
profile converts each direction into horizontal offset and height — domes
widen toward the top, fair-weather heaps spread evenly, towering columns
stay narrow and lean downwind with height. Because positions come from
hashes instead of an index spiral, no cluster skeleton repeats. Puffs carry
three octaves of continuous lobed displacement plus a flat cut underneath,
with analytic smooth normals from tangent-neighbour samples, so silhouettes
and shading stay rounded and detailed at close range. Small clouds drop
their outer puffs (four to eight), footprints are power-skewed so most
clouds are small while a few grow huge, and a rare hash (~6% of cumulus
cells) doubles a cluster into a giant congestus tower.

Per-cell hashes choose presence (about 38% of cells), a thin high-streak
family near 8 km (about 6% of cells), altitude, footprint, yaw, and one of
four cumulus base families (fair-weather, mediocris, towering, cauliflower);
each shape parameter then carries its own continuous jitter (height x0.75-1.25,
displacement x0.8-1.3, stretch +-0.45 footprints, squash 0.62-0.95), so
silhouettes form a spectrum rather than four repeated prototypes.

Shading is stylised, not a light-transport solution: wrapped sun diffuse with
altitude transmittance on smooth normals, a vertical sky ambient gradient, a
Henyey-Greenstein forward-scatter rim (Henyey & Greenstein 1941, foundational)
for backlit edges, and the same wavelength-dependent aerial haze as the ground
pass. Back faces shade as a flat bright veil, which is what flying through a
cloud looks like. The geometry is opaque and writes depth, so clouds and
mountains occlude each other exactly through the shared depth buffer
regardless of draw order.

Ground sun shadows replay the same placement hashes: the receiver is
projected onto each nearby cloud's own altitude and shaded by a smooth radial
falloff (`cloudSunVisibility` in cloud.inc). Shadow blobs line up with the
visible clouds one to one, including while the deck drifts downwind; the two
sub-band cell searches are an approximation at very low sun and thin high
streaks cast nothing. The atmosphere model underneath is unchanged
(Hillaire 2020, [sebh.github.io](https://sebh.github.io), EGSR 2020,
accessed 2026-09-19).

Wind

`EXPLORA_WIND` defaults to 1, clamps to 0–3, and accepts 0 for calm conditions.
The CPU wind field is deterministic in world position and simulation time.
Its prevailing flow strengthens and turns with altitude, with smooth horizontal
gusts and vertical motion that fades toward the surface.

Flight forces, speed limits, and camera angle-of-attack buffet use air-relative
velocity. Position integrates world velocity, allowing the aircraft to drift.
Trail particles entrain toward the sampled wind. Mesh clouds use a constant
8.5 m/s X/Z prevailing flow multiplied by the same strength setting; they do
not reproduce every local CPU gust. Cloud shaders receive the strength in the
previously unused `cameraParams2.w` uniform slot, keeping the UBO size unchanged.

## Frame pacing

Presenting as fast as the GPU allows was measured on the target machine
(RTX 4060 Laptop, 2880x1800 eDP at 119.96 Hz, NVIDIA driver 580.173.2,
XWayland; accessed 2026-09-20) presenting at 78-108 FPS with frame times
swinging between roughly 3 and 13 ms. The simulation sampling itself is
time-accurate — replaying the recorded frame-time sequence through the sim and
chase camera reproduces the exact per-frame poses — but the compositor
displays those irregular frames on its regular refresh grid, so motion
periodically freezes and then catches up. The eye reads this as rubberbanding;
it is most visible against the horizon, where a few hundredths of a degree of
attitude shift moves every mountain silhouette several pixels, and invisible
against uniform sky or ground.

The fix paces the render loop to scanout with `VK_KHR_present_wait`
(https://www.khronos.org/registry/vulkan/specs/1.3-extensions/man/html/VK_KHR_present_wait.html;
promoted from the NV extension, Khronos registry, accessed 2026-09-20): after
each present the loop blocks on `vkWaitForPresentKHR` for the just-submitted
present ID, then samples and submits the next frame. Frame times lock to
8333 microseconds (120.0 FPS measured). Present IDs come from
`VK_KHR_present_id`; both extensions are capability-checked, and play mode
falls back to unpaced FIFO after repeated wait timeouts
(`EXPLORA_PACING=off` forces this). Benchmarks never pace: the 1000
presentations-per-second target is an unpaced submission measurement.

## Final image

The composite uses five neighborhood samples for conservative sharpening,
clamped to local color bounds. Those samples are reused for a thirteen-tap
local highlight filter, with one additional tap when motion streaking is active.
Soft highlight extraction and a luminance cap keep glare restrained. The
filter's radius is about four source pixels; it is not broad multiscale bloom.
Tone mapping stays in linear light and the sRGB swapchain applies the display
transfer function. Sensor grain is reduced to retain distant detail.

## Acceptance on the target GPU

CPU tests and offline shader compilation run in the Linux verification
container described in `tools/README.md`. Actual appearance and performance
must also be assessed on the supported Linux/NVIDIA machine:

- Compare balanced and cinematic at the same resolution, pose, and wind setting.
- Use `EXPLORA_FREEZE=1` and screenshot helpers for fixed-pose comparisons.
- Fly below, through, and above the cumulus deck; check puff silhouettes and
  facet shading near and far, the interior veil when diving through a cloud,
  ground shadow alignment under drifting clouds, and ridge/cloud occlusion.
- Fly across world-origin changes and resize the window; weather should remain
  anchored and scene resolution should retain its selected preset.
- Check sun/exhaust highlights, dark silhouettes, and horizon detail for halos.
- Benchmark each preset separately; previous frame-rate measurements do not
  establish the performance of this update.
