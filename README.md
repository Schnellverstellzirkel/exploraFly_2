# exploraFly_2
Open-world exploration.
Entirely vibe coded game and game engine to see how far can a 3D game be optimized.

Fixed target: RTX 4060 Laptop GPU on NVIDIA 580 driver for graphics. Ryzen 7 7840HS for CPU work. Linux only. 

Layout:

- `engine` is the native binary. Rust plus raw Vulkan through ash. The probe locks the discrete NVIDIA GPU and reports queues, heaps, and wanted extensions.
- `kernels` holds compute crates. Terrain generation and batched body math.
- `docs` holds the reference set. Vulkan registry and specs, vendor specs, allocator reference, man pages, Rust books.

Run:

```sh
cargo run -p explora-engine
```
