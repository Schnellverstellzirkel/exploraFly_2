# exploraFly_2
Open-world exploration.
Entirely vibe coded game and game engine to see how far can a 3D game be optimized.

Fixed target: RTX 4060 Laptop GPU on NVIDIA 580 driver for graphics. Ryzen 7 7840HS for CPU work. Linux only. 

Layout:

- `engine` is the native binary. Rust plus raw Vulkan through ash. The probe locks the discrete NVIDIA GPU and reports queues, heaps, and wanted extensions.
- `crates` holds decoupled sub-crates: `airframe` procedural generation (build-time only), `airframe-baker` and `airframe-format` for the packed runtime mesh boundary, `world` deterministic alpine heights and landmark clearance, and `sim` physics/effects/audio/camera math.
- `kernels` holds compute crates. Terrain generation and batched body math.
- `docs` holds the reference set. Vulkan registry and specs, vendor specs, allocator reference, man pages, Rust books.
- `engine/shaders` holds GLSL sources. `engine/build.rs` compiles them to
  SPIR-V offline with shaderc at build time, so the binary ships no shader
  compiler. First build needs cmake and python3 for the shaderc sys crate.

Run:

```sh
cargo run -p explora-engine
```

There is one playable quality preset: `cinematic`. It renders native resolution
with full shading rates and 16 IBL samples. Release and dist builds always use
this preset and ignore `EXPLORA_QUALITY`. Debug builds can select reduced
presets for development:

| `EXPLORA_QUALITY` (DEBUG_ONLY) | Scene resolution | Sky / clouds shading | Ground / composite shading | IBL samples |
| --- | --- | --- | --- | --- |
| `performance` | 80% per dimension | 4x4 | 4x2 / 2x2 | 4 |
| `balanced` | Native | 2x2 | 1x1 | 8 |
| `cinematic` (default, only playable) | Native | 1x1 | 1x1 | 16 |

Shading rates apply when the GPU supports fragment shading rate. Aircraft
geometry stays full rate. `EXPLORA_IBL_SAMPLES` is a DEBUG_ONLY override of the
preset's reflection sample count. The final image uses bounded contrast-adaptive
sharpening, a small spatial HDR highlight glow, and restrained sensor grain.
Bright pixels contribute to nearby pixels before tone mapping, with a cap on
glare intensity.

All `EXPLORA_*` process flags are declared once in
[`engine/src/flags.rs`](engine/src/flags.rs). Play flags configure window, audio,
wind, and HUD and work in every build. DEBUG_ONLY flags select reduced quality,
alternate GPU paths, pacing overrides, forced flight regimes, and diagnostics.
They compile under `debug_assertions` only, so release and dist builds never
read them.

Measure optimized presentation throughput (playable preset, unpaced benchmark):

```sh
cargo framebench
```

Validate that the Rust and Vulkan world equations still agree on the target
GPU before changing terrain or vegetation code:

```sh
cargo worldcheck
```

The high-load acceptance case keeps the burner and hard bank engaged:

```sh
EXPLORA_WIND=0 EXPLORA_BOOST=1 EXPLORA_BANK=1 \
  cargo run --profile dist -p explora-engine -- --benchmark 10000
```

`EXPLORA_BOOST` and `EXPLORA_BANK` are DEBUG_ONLY forced-flight helpers. In
playable builds use Shift and A/D instead. The default renders one complete
frame per presentation. DEBUG_ONLY `EXPLORA_BURST` can add geometry-only passes
for throughput experiments, but lowers real FPS. The reported real FPS counts
successful presentation submissions, not distinct frames displayed by the
monitor; display cadence is limited by its refresh rate.

Material reflections use 16 deterministic GGX visible-normal samples per
specular lobe in the playable preset. DEBUG_ONLY `EXPLORA_IBL_SAMPLES` can
select `4`, `8`, `16`, `32`, or `128`. See
[`docs/rendering/material-realism.md`](docs/rendering/material-realism.md).

The horizon-stabilized chase camera uses damped orientation tracking, a constant
field of view and boom distance, aerodynamic buffet, and altitude-correct Mach
response. The optical pass adds curvilinear lens distortion, vignetting, subtle
sensor grain, and local highlight glare. See
[`docs/rendering/camera-immersion-and-optics.md`](docs/rendering/camera-immersion-and-optics.md).

The landscape uses a fixed world-aligned grid with cached heights and smooth
normals, so camera movement does not reshape it: alpine valleys, ridgelines,
snow, meadows, an open ocean shelf with beaches, flat alpine lakes, and
procedural medieval keeps, walls, and villages.
Terrain and landmarks share one indexed draw. Hardware depth
handles mountain silhouettes; flight and camera clearance query the same terrain
recipe on the CPU. See [terrain research and limits](docs/rendering/alpine-terrain.md).
Meadows use matte grass, photographed blade detail, and filtered tussock relief;
see [alpine grass material and validation](docs/rendering/alpine-grass.md).
This is a repeating procedural world, not a finished authored open-world map.

Clouds now share a world-anchored density field with the ground's moving cloud
shadows. Cumulus lighting includes a local sun-occlusion probe, and distant
clouds fade into atmospheric haze. Shadowed ground retains ambient skylight.
These are bounded procedural approximations. See
[weather and image-quality notes](docs/rendering/weather-and-quality.md) for
the implementation, sampling budgets, and target-GPU acceptance checks.

Wind is enabled by default. `EXPLORA_WIND=0` selects calm air, `1` a moderate
breeze, and `3` stronger conditions (values clamp to 0–3). Smooth deterministic
gusts and altitude shear affect airspeed, lift, sideslip, and ground drift.
Vapor trails respond to the same sampled wind, while large cloud masses follow
a shared prevailing flow scaled by the wind setting. Use calm air for repeatable
comparisons with the original flight behavior.

```sh
EXPLORA_WIND=1 cargo run --release -p explora-engine
```

For CPU tests and shader compilation from Windows or another host without the
Linux toolchain, see [the verification container](tools/README.md). Visual and
performance acceptance still require the target Linux/NVIDIA machine.

## Flight instruments, controls, and sound

The expedition aircraft now uses cream canvas, teal markings, brass trim,
navigation lights, and responsive turquoise/thermal exhaust. See
[aircraft material and effects notes](docs/rendering/aircraft-art-and-fx.md).

The HUD displays airspeed in knots, altitude and terrain clearance in metres,
heading, and engine spool. W/S or arrows pitch; A/D or arrows bank; Q/E yaw;
Shift boosts. P pauses, R resets, H toggles the HUD, F1 toggles help, M mutes,
F11 toggles fullscreen, and Escape exits. Reset clears the aircraft and wake
state. A forgiving terrain/roof clearance floor assists exploration; it is not
a crash simulation.

`EXPLORA_HUD=0` starts with the overlay hidden for screenshots or A/B timing.

DEBUG_ONLY spawn helpers `EXPLORA_X`, `EXPLORA_Z`, `EXPLORA_ALT`, and
`EXPLORA_HEADING` override the startup pose for one debug run. Playable builds
always spawn at the valley.

Play mode paces rendering to the display scanout with `VK_KHR_present_wait`
when the driver supports it, so the simulation is sampled once per refresh.
Presenting unlocked lets the compositor display irregular frames on its own
regular grid, which reads as a periodic freeze-and-lurch, most visible against
the horizon. DEBUG_ONLY `EXPLORA_PACING=off` disables this for display
diagnostics; benchmarks always run unpaced to measure the submission-rate
target.

Flight-driven stereo turbine, wind, and load sounds are synthesized without
recording assets. `EXPLORA_AUDIO=0` disables playback; `EXPLORA_VOLUME` sets
0 to 1 master gain (default 0.35). Native PCM playback currently targets
ALSA/Linux; this milestone does not port the renderer to Windows. See
[interface and audio research](docs/rendering/flight-interface-and-audio.md).

The [frame-budget runner](tools/benchmark-frames.py) records exact mean/p50/p95/p99,
GPU timings, resolution, driver, scene settings, and optional display feedback.
DEBUG_ONLY `EXPLORA_RT_SHADOWS=off` selects analytic aircraft shadows for a
controlled comparison. No 1000 FPS result has been established by Windows/Docker
checks. See [measurement protocol](docs/optimization_techniques/frame-budget-2026.md).
