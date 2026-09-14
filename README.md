# exploraFly_2
Open-world exploration.
Entirely vibe coded game and game engine to see how far can a 3D game be optimized.

Fixed target: RTX 4060 Laptop GPU on NVIDIA 580 driver for graphics. Ryzen 7 7840HS for CPU work. Linux only. 

Layout:

- `engine` is the native binary. Rust plus raw Vulkan through ash. The probe locks the discrete NVIDIA GPU and reports queues, heaps, and wanted extensions.
- `crates` holds decoupled sub-crates: `airframe` procedural generation and `sim` physics/effects/noise/camera math, keeping iteration builds fast and multi-core.
- `kernels` holds compute crates. Terrain generation and batched body math.
- `docs` holds the reference set. Vulkan registry and specs, vendor specs, allocator reference, man pages, Rust books.
- `engine/shaders` holds GLSL sources. `engine/build.rs` compiles them to
  SPIR-V offline with shaderc at build time, so the binary ships no shader
  compiler. First build needs cmake and python3 for the shaderc sys crate.

Run:

```sh
cargo run -p explora-engine
```

Measure optimized presentation throughput:

```sh
cargo run --release -p explora-engine -- --benchmark 10000
```

The default renders one complete frame per presentation. `EXPLORA_BURST` can
add geometry-only passes for throughput experiments, but lowers real FPS.
The reported real FPS counts successful presentation submissions, not distinct
frames displayed by the monitor; display cadence is limited by its refresh rate.

Material reflections use eight deterministic GGX visible-normal samples per
specular lobe. Set `EXPLORA_IBL_SAMPLES` to `16`, `32`, or `128` for progressively
more expensive reference-quality integration. See
[`docs/rendering/material-realism.md`](docs/rendering/material-realism.md).
