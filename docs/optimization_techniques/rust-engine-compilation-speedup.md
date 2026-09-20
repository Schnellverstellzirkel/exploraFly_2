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

---

## 6. Native Cargo/linker audit (2026-09-20)

This audit was performed on the actual target workstation rather than a
portable build host: Ubuntu 24.04.5, Linux 7.0.0-31, Rust/Cargo 1.98.1
(LLVM 22.1.8), Ryzen 7 7840HS, 30 GiB RAM, RTX 4060 Laptop GPU, NVIDIA
580.173.02, 2880x1800 at 120 Hz. The benchmark measured complete scene
present submissions at 2880x1800 output / 2304x1440 scene resolution,
Performance preset, RT shadows on, MAILBOX presentation, and one complete
scene per present. Submission rate is not monitor display rate.

### Change kept

`.cargo/config.toml` now uses the fixed Zen 4 target, Clang as the linker
driver for mold, `--icf=safe`, explicit `-z now`, and two aliases:

```text
cargo maxrun       # the Cargo.toml dist profile
cargo framebench   # the dist profile's 10,000-present benchmark
```

The explicit CPU target remains `znver4`; a manual AVX feature list was not
added. On this host, `rustc -C target-cpu=native --print cfg` and
`rustc -C target-cpu=znver4 --print cfg` expose the same feature set, including
the AVX-512 family, GFNI, and VAES. The live CPU flags also expose those
features. The compiler's CPU-selection and target-feature documentation warns
that explicit target-feature changes are unsafe and that CPU defaults already
select a feature set, so duplicating the list would add maintenance risk
without changing this build.

Safe ICF is the one new link-time optimization. The mold documentation says
that safe ICF uses LLVM's address-significance information and that Clang emits
the required `.llvm_addrsig` section; this is why the configuration uses Clang
as the driver. `--icf=all` was intentionally not used because mold documents
that it can make distinct function pointers compare equal. The final binary
contains mold 2.42.1 and `BIND_NOW`/`NOW` PIE flags.

### Measurements and limits

The pre-change release capture (three 10,000-present runs) produced
289.6--290.3 submissions/s, 3,271--3,313 us mean GPU time, and 5,832--6,068
us wall p99. The post-change release capture was not a valid A/B: the GPU
heated from 53 C to 79 C and its mean GPU time rose from 3,221 to 3,950 us
across the runs. It is therefore recorded as inconclusive rather than as a
regression or gain.

The final `dist` artifact was then run after cooling to 48 C. Three
3,000-present runs measured 270.0--271.1 submissions/s, 3,669--3,686 us mean
GPU time, and 5,090--5,117 us wall p99. The 1,000 submissions/s and 1 ms
mean+p99 target remains unmet. Terrain was about 1,986--1,993 us and composite
about 457--458 us per GPU frame; CPU simulation/camera was about 70 us. This
identifies the current dominant limit as GPU fragment work and presentation
backpressure, not Rust linking or CPU instruction selection.

The Clang/mold safe-ICF artifact was also smaller: release `.text` decreased
from 32,674,207 bytes in the no-ICF GCC/mold native candidate to 32,653,343
bytes, a 20,864-byte (0.064%) reduction. This is an artifact-level comparison
that includes the driver transition; no frame-rate improvement is claimed from
the size reduction. Explicit `-z now` is a policy declaration here; current
rustc already emits `BIND_NOW` for these Linux release executables, so it is not
counted as a new runtime optimization.

PGO remains a separate next experiment. Rust supports an instrument →
representative workload → `llvm-profdata` → profile-use workflow, but it must
not be enabled unconditionally in this shared config because it requires a
valid profile dataset and changes the build/reproducibility contract. The
renderer should first reduce the measured terrain/composite GPU costs.

### Primary references

Accessed 2026-09-20:

- Rust, [Cargo configuration](https://doc.rust-lang.org/cargo/reference/config.html): target-specific `rustflags`, linker selection, and config precedence.
- Rust, [codegen options](https://doc.rust-lang.org/rustc/codegen-options/): `target-cpu`, `target-feature`, codegen units, relocation, and LTO semantics.
- LLVM, [X86 target feature definitions](https://github.com/llvm/llvm-project/blob/main/llvm/lib/Target/X86/X86.td) and [Zen 4 scheduling model](https://github.com/llvm/llvm-project/blob/main/llvm/lib/Target/X86/X86ScheduleZnver4.td): Zen 4 feature dependencies and scheduling model.
- mold, [official usage and ICF documentation](https://github.com/rui314/mold/blob/main/README.md) and [linker manual](https://github.com/rui314/mold/blob/main/docs/mold.md): Clang driver setup, safe ICF requirements, and the safety limit of `--icf=all`.
- Rust, [profile-guided optimization](https://doc.rust-lang.org/nightly/rustc/profile-guided-optimization.html): instrumented profile generation and use; not enabled by default here.
