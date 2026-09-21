# Airframe build-time bake and runtime crate boundary

This change moves immutable airframe conversion out of `explora-engine`'s
initialization path. The procedural `airframe` crate remains available to the
build-time `airframe-baker`, while the runtime depends only on the small
`airframe-format` crate and the generated packed asset.

## What was measured first

Environment: AMD Ryzen 7 7840HS (16 logical CPUs), NVIDIA GeForce RTX 4060
Laptop GPU, driver 580.173.02, Linux/Wayland, 2026-09-20. The prior
`airframe_mesh()` path was measured in a `dist` build with
`EXPLORA_QUALITY=performance EXPLORA_WIND=0 EXPLORA_FREEZE=1
EXPLORA_PACING=off EXPLORA_RT_SHADOWS=off cargo run --profile dist -p
explora-engine -- --benchmark 1`.

The old path built the procedural mesh twice and then did normal accumulation,
Forsyth index reordering, vertex packing, material partitioning, and RT range
construction at runtime:

| Runtime work | Before |
| --- | ---: |
| First procedural bake | 1.11 ms |
| Second procedural bake | 1.12 ms |
| Normal/packing/index/RT preparation | 10.39 ms |
| **Total airframe preparation** | **12.62 ms** |

These timings measure initialization, not a presented frame. They were the
reason for changing the ownership boundary; no micro-optimization was applied
to the procedural generator before measuring it.

## Design

`airframe-baker` performs the old conversion once from a Cargo build script and
writes `OUT_DIR/airframe.bin`. `airframe-format` owns a versioned, little-endian
format with explicit bounds checks before upload:

- 28-byte packed vertices: position, octahedral normal, half UV, flex, and
  node/material IDs;
- opaque, glass, and RT index sections;
- RT node IDs and contiguous per-node index ranges;
- magic, version, stride, byte counts, and section counts in a 48-byte header.

The runtime uses `include_bytes!` plus validation before making owned section
buffers. GPU upload, vertex layout, index order, material partitioning, and RT
ranges remain unchanged. Thus the optimization changes when work happens and
the crate dependency graph, not rendered geometry or flex-lighting inputs.

The baker test currently locks the measured topology at 46,752 triangles,
28,162 vertices, and 23 RT nodes. The format tests cover round trips and
trailing-data rejection. The engine build emits the same counts as a Cargo
warning so accidental topology changes are visible in CI logs.

## After measurement

The generated asset was 1,345,852 bytes. Runtime decode and validation took
**0.880 ms** in the same `dist`/target-machine setup. GPU resource creation and
the staged device-local upload are unchanged, so the directly attributable
reduction is:

| Runtime work | Before | After | Change |
| --- | ---: | ---: | ---: |
| Airframe preparation/decode | 12.62 ms | 0.880 ms | **11.74 ms saved; 93.0% less** |

The after timing was temporary instrumentation and was removed from the final
runtime source. It is not a frame-time measurement. A final-binary one-present
smoke run still rendered the complete scene on the target GPU; its
single-sample result was 4,825.6 us GPU time and 2,599.0 us submission interval
at 2304x1317 scene resolution / 2880x1646 output, Performance preset,
ray-traced shadows off, MAILBOX present. One sample is not a performance
acceptance run, and the 1,000-present/s target remains unverified and unmet by
this change.

## Validation and limits

Run:

```sh
cargo test -p airframe-format -p airframe-baker --offline
cargo test --workspace --locked
python3 tools/check_shaders.py
```

The format is intentionally copied into owned vectors at load time. That keeps
the existing renderer interfaces and allows GPU staging to use the same stable
buffers, but it is not a zero-copy runtime asset view. If startup profiling
shows decode allocation matters after larger asset bundles are added, the next
step is a borrowed validated view or memory-mapped asset; it should not be
introduced speculatively for this 0.88 ms path.

The baker still depends on the procedural generator and `glam` at build time.
This is a build-time/runtime separation, not a final authored-asset pipeline:
source geometry changes correctly invalidate the engine build. PGO remains a
separate, opt-in build experiment in `tools/pgo-engine.sh`; it is not coupled
to this asset format or enabled by the native Cargo config.

## Primary references

Accessed 2026-09-20:

- Rust, [Cargo build scripts](https://doc.rust-lang.org/cargo/reference/build-scripts.html):
  `OUT_DIR` artifacts and `rerun-if-changed` dependency tracking. This supports
  the build boundary; it does not establish a runtime speedup.
- Rust, [`include_bytes!`](https://doc.rust-lang.org/std/macro.include_bytes.html):
  compile-time inclusion of the validated generated asset. This avoids file I/O
  during engine startup, but the current decoder still allocates owned sections.
