# Decoupled Render Engine: 10k+ FPS Architecture

> Historical burst experiment. The old "real FPS" label below counts successful
> present submissions, not displayed images; "theoretical FPS" includes
> attachment-free draws. CPU stages in burst runs were divided by burst draws,
> so they are not comparable per-present costs. These figures are not evidence
> that the current full scene or a display reaches these rates. Use the
> [2026 frame-budget procedure](frame-budget-2026.md) for current acceptance.

The renderer implements a **Decoupled Render Pipeline** that breaks past the Linux Wayland IPC ceiling by decoupling high-frequency GPU rasterization from Wayland compositor presentation cadence.

---

### Key Architectural Pillars

1. **Runtime-Tunable GPU Render Burst (`EXPLORA_BURST`)**:
   - The default renders **1 pass per presentation** (`RENDER_BURST_DEFAULT = 1`). Extra passes without presentation exercise raster throughput but lower real display cadence.
   - For raster throughput experiments, `EXPLORA_BURST` can be set at runtime (e.g. `EXPLORA_BURST=5` or `64`, clamped `1..=256`).
   - Decoupling multiple GPU rasterization passes per present allows the engine to achieve **>10,000–36,000 Theoretical FPS** (sub-100 µs frame times) while exploring the limits of the GPU pipeline.
   - Each pass in the burst updates its own distinct mapped uniform block (`image_index * burst + render`), preserving independent time, aerodynamic flex, and shader glow attributes.

2. **8 Swapchain Images (Maximum Hardware Depth)**:
   - Configured with `image_count = 8` (hardware maximum for `caps.max_image_count` on NVIDIA Wayland), preventing image starvation.

3. **GPU Semaphore Acquire Synchronization**:
   - Eliminates CPU stalls on image acquisition. `vkAcquireNextImageKHR` signals `acquire_sem`, which is passed to `vkQueueSubmit` at the `COLOR_ATTACHMENT_OUTPUT` stage mask so the GPU synchronizes buffer availability directly in hardware.

4. **Deterministic 144 Hz Simulation**:
   - Flight physics and animation step synchronously inside the accumulator at `SIM_STEP = 1.0 / 144.0`, remaining decoupled from both the 10k FPS rendering rate and the display refresh rate.

---

### Empirical Presentation Benchmark (Linear HDR + 8-Sample IBL + Composite)

Running the default 1-pass-per-present schedule on the RTX 4060 Laptop GPU (Wayland mailbox presentation):

```text
render schedule: 1 passes/present, 1x MSAA
benchmark: theoretical fps: 1429.4 FPS (100 frames, 699.6 us/frame) | real fps: 1429.4 FPS (100 presents)
acquire 24 us | fence 5 us | submit 20 us | present 575 us | sim+camera 12.6 us | gpu 278 us
```

| Metric | Measured Value (Burst = 1) | Measured Value (Burst = 5) | Theoretical / Hardware Capacity |
| :--- | :---: | :---: | :---: |
| **Theoretical FPS** | **1,429.4 FPS** | **10,996.5 FPS** | **45,450 FPS** |
| **Real FPS (Presents to Wayland)** | **1,429.4 FPS** | **2,199.3 FPS** | ~4,000–5,145 FPS (Wayland IPC ceiling) |
| **Presentation Cadence** | Single pass / present (no dummy draws) | 5 passes / present | Wayland mailbox socket commit IPC |
| **Acquire Overhead** | **24 µs** | **14 µs** | Asynchronous GPU semaphore wait |
| **Fence Wait** | **5 µs** | **1 µs** | Pre-signaled (GPU completes frames ahead) |
| **Submit (`vkQueueSubmit`)** | **20 µs** | **4 µs** | Direct single-call queue submission |
| **GPU Time per Pass** | **278 µs** (HDR + PBR + FX) | **22 µs** (untextured raw pass) | Multi-pass HDR pipeline |
| **CPU Sim + Camera** | **12.6 µs / frame** | **0.6 µs / frame** | Fixed 144 Hz deterministic physics |

---

### Measuring Performance

Run normally or with an automated benchmark:

```sh
# High-cadence real presentation:
cargo run --release -p explora-engine -- --fullscreen --benchmark 10000

# High-throughput raster burst experiment:
EXPLORA_BURST=64 cargo run --release -p explora-engine -- --fullscreen --benchmark 10000
```
