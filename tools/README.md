# Linux verification container

The renderer still targets the native Linux/NVIDIA machine described in the
project README. This container compiles shaders and runs CPU regression tests
from other hosts; it does not launch or benchmark the game.

Build the development image from the repository root:

```sh
docker build -f tools/Dockerfile.check -t explorafly-check tools
```

On Linux:

```sh
docker run --rm -v "$PWD:/workspace" -v explorafly-cargo:/usr/local/cargo/registry -v explorafly-target:/workspace/target explorafly-check
```

On Windows PowerShell, replace the first mount with `-v "${PWD}:/workspace"`.
The named volumes retain dependencies and build products between runs. The
container overrides the repository's Zen 4 CPU flag so tests can execute on
other x86-64 hosts. Pass a command after the image name to run a narrower check,
for example `cargo test -p sim --locked`.

The default command also runs `tools/check_shaders.py`, which checks every
compiled module with `spirv-val --target-env vulkan1.3` and fails if none exist.
GPU screenshots and performance measurements must be taken on the target
Linux machine; a passing CPU/shader build does not establish visual quality or
frame rate.

## Profile-guided build

On the target machine, `tools/pgo-engine.sh` is an opt-in instrument/train/use
workflow. It requires `llvm-profdata` with the same LLVM major version as
`rustc`, runs cruise, bank+boost, and vegetation/RT training workloads, and
writes the PGO-use binary under
`target/pgo/use/x86_64-unknown-linux-gnu/dist/`. Keep its workload,
power state, resolution, and driver fixed when comparing it with the unprofiled
`dist` binary; the script does not claim a speedup by itself.

The exact Rust-matched tool is installed with:

```sh
rustup component add llvm-tools-preview
tools/pgo-engine.sh
```

The script discovers `llvm-profdata` inside the active rustup toolchain. A
system LLVM 22 installation can also be selected with `LLVM_PROFDATA=/path/to/llvm-profdata`.

## Rust/GLSL world conformance

The native Vulkan device can compare the independently implemented world
equations before a rendering benchmark:

```sh
cargo worldcheck
```

This dispatches 8,192 coordinates through the compiled SPIR-V and compares
terrain height/surface plus vegetation probability/forest-cover fields against
the Rust implementation. It is a correctness gate, not a frame-rate test.
