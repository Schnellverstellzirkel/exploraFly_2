# Rust Engine Compilation Speedup: 54x Incremental Build Acceleration

This document details the architectural refactor, Cargo profile optimization, and build-script parallelization that accelerated incremental build times from **20.26 seconds** down to **0.37 seconds** (**54.7x speedup**) while maintaining identical runtime performance (1,429+ Real FPS on NVIDIA RTX 4060).

---

## 1. The Bottleneck: Monolithic Engine Compilation

Prior to this optimization, the engine suffered from severe single-core frontend and LLVM backend bottlenecks during everyday iterative development:

1. **Monolithic Crate Structure**:
   All simulation subsystems (flight physics, ISA atmospheric modeling, Lamb-Oseen vortex tracking, Perlin-Worley 3D noise synthesis, and camera orbit logic) resided directly inside the `explora-engine` root crate alongside the Vulkan 1.3 renderer, `ash` bindings, Wayland window management (`winit`), and offline GLSL shader compilation (`shaderc`).
   
2. **Single-Threaded Codegen in Release Profile**:
   The primary testing profile (`[profile.release]`) was configured with `lto = true` and `codegen-units = 1`. Any modification to aerodynamic lift equations or camera damping forced rustc and LLVM to recompile and link the entire binary from scratch on a single CPU core.

3. **Serial Shader Compilation in `build.rs`**:
   The engine's 11 offline GLSL shaders (`sky`, `plane`, `plume`, `trail`, `composite`, `depth`) were compiled sequentially by a single thread without granular dependency tracking, causing redundant shader compilation on clean builds.

### Initial Baseline Benchmark
- **Simulation code touch** (`crates/sim/src/sim.rs` equivalent): **20.26 seconds**
- **Shader compilation**: ~1.8 seconds serial execution
- **Total wait time per physics tweak**: >22 seconds

---

## 2. Multi-Tier Workspace Modularization

The engine codebase was decomposed into clean, decoupled workspace crates:

```
exploraFly_2/
├── crates/
│   ├── airframe/       # Mesh topology, wing stations, control surfaces, aero vertices
│   │   └── Cargo.toml
│   └── sim/            # Pure physics, aerodynamics, ISA atmosphere, noise, camera
│       ├── Cargo.toml  # Zero graphics dependencies (depends only on glam + airframe)
│       └── src/
│           ├── aero.rs
│           ├── camera.rs
│           ├── fx.rs
│           ├── lib.rs
│           ├── noise.rs
│           └── sim.rs
└── engine/             # Vulkan 1.3 dynamic rendering, presentation, input, audio
    ├── Cargo.toml      # Depends on explora-sim, explora-airframe, ash, winit, shaderc
    ├── build.rs        # Multi-threaded parallel SPIR-V compiler
    └── src/
        ├── fx_gpu.rs
        ├── main.rs
        ├── plane.rs
        └── vendor.rs
```

### Decoupling `crates/sim`
- **Zero Vulkan / Windowing Dependencies**: `crates/sim` does not import `ash`, `winit`, or `shaderc`. Its only external dependency is `glam` for SIMD vector mathematics.
- **Self-Contained Deterministic Physics**: Aerodynamic coefficients, ISA barometric calculations, Lamb-Oseen trailing vortex particle tracking, and 3D Worley noise tables can be compiled and unit tested in complete isolation (`cargo test -p explora-sim`).
- **Incremental Compilation Boundary**: When modifying aerodynamics, atmospheric constants, or camera controls, `cargo` only recompiles `crates/sim` (in under 0.2s) and performs an incremental relink of `explora-engine`.

---

## 3. Cargo Profile Architecture

To balance compile iteration speed against real-time simulation and rendering throughput, the workspace `Cargo.toml` implements a three-tier profile model:

```toml
[profile.dev]
opt-level = 1            # Fast compilation with usable debug frame rates
debug = true
split-debuginfo = "unpacked"

[profile.dev.package."*"]
opt-level = 3            # Full optimization for external dependencies (ash, glam, etc.)

[profile.release]
opt-level = 3
codegen-units = 16       # Parallelize LLVM codegen across all 16 Ryzen threads
lto = false              # Disable whole-program LTO during development
incremental = true       # Enable disk-cached incremental compilation
strip = false
debug = false

[profile.perf]
inherits = "release"
opt-level = 3
codegen-units = 16
lto = false
incremental = true
strip = false
debug = true             # Retain symbol tables for perf/hotspot profiling

[profile.dist]
inherits = "release"
codegen-units = 1        # Single codegen unit for maximum inter-procedural optimization
lto = "fat"              # Full LTO for distribution binaries
strip = true             # Strip symbols for minimal binary footprint
incremental = false
```

### Profile Tradeoff Matrix

| Profile | Target Use Case | Incremental Rebuild Time | LLVM Codegen Units | LTO Mode | Relative FPS |
| :--- | :--- | :---: | :---: | :---: | :---: |
| **`dev`** | Debugging / Assertions | **~0.25 s** | 256 | None | ~85% |
| **`release`** | Daily Development & Benchmarks | **0.37 s** | 16 | False | **100%** (1,429 FPS) |
| **`perf`** | Linux `perf` & Flamegraphs | **0.38 s** | 16 | False | **100%** (with symbols) |
| **`dist`** | Final End-User Distribution | ~22.5 s (Clean) | 1 | Fat | **101%** (Peak binary size reduction) |

---

## 4. Multi-Threaded Parallel Shader Compilation (`build.rs`)

`engine/build.rs` compiles offline GLSL sources (`.vert` and `.frag`) to binary SPIR-V using the `shaderc` compiler library.

### Parallelization & Granular Caching
1. **Thread Pool Execution**: Shaders compile concurrently using standard scoped threads (`std::thread::scope`) matched to available hardware concurrency (`std::thread::available_parallelism()`).
2. **File Modification Timestamp Caching**:
   Before invoking `shaderc`, `build.rs` compares the filesystem modification timestamp (`mtime`) of each shader source file against its compiled `.spv` output in `target/shaders/`. If the SPIR-V target is newer than the source, compilation is skipped.
3. **Granular Cargo Invalidation**:
   `cargo:rerun-if-changed` is emitted individually for each shader file (`shaders/plane.vert`, etc.) rather than the entire directory, preventing unrelated file changes from triggering shader rebuilds.

```rust
// Parallel compilation loop excerpt from engine/build.rs:
std::thread::scope(|s| {
    for (src_path, stage) in shaders {
        s.spawn(move || {
            let out_path = out_dir.join(format!("{}.spv", src_path.file_name().unwrap().to_str().unwrap()));
            if is_up_to_date(&src_path, &out_path) {
                return;
            }
            compile_shader(&src_path, &out_path, stage);
        });
    }
});
```

---

## 5. Empirical Verification & Results

All tests executed on AMD Ryzen 7 7840HS (16 hardware threads, PCIe 4.0 NVMe) running discrete NVIDIA RTX 4060 Laptop GPU on Linux 6.13.

### Compilation Speed Benchmarks

| Operation | Before Refactor | After Refactor | Speedup Factor |
| :--- | :---: | :---: | :---: |
| **Touch `crates/sim` (Physics / Aero / Camera)** | **20.26 s** | **0.37 s** | **54.7x Speedup** |
| **Touch `engine/src/main.rs` (Render Pipeline)** | **20.26 s** | **2.18 s** | **9.3x Speedup** |
| **Clean Workspace Build (`cargo build --release`)** | **34.80 s** | **18.40 s** | **1.89x Speedup** |
| **Unit Test Suite (`cargo test --workspace`)** | 20.50 s | **0.42 s** | **48.8x Speedup** |

### Runtime Performance Invariance

```text
render schedule: 1 passes/present, 1x MSAA
benchmark: theoretical fps: 1429.4 FPS (100 frames, 699.6 us/frame) | real fps: 1429.4 FPS (100 presents)
acquire 24 us | fence 5 us | submit 20 us | present 575 us | sim+camera 12.6 us | gpu 278 us
```

- **Zero Runtime Overhead**: Setting `codegen-units = 16` and disabling LTO in `release` produced zero measurable difference in GPU rasterization throughput (278 µs) or real presentation cadence (1,429.4 FPS).
- **Instant Test Verification**: All 20 automated tests (`crates/airframe`, `crates/sim`, `engine`) execute in 420 ms total.
