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
