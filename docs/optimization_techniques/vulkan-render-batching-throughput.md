# Real FPS presentation pipeline with 8 swapchain images and GPU semaphore synchronization

The renderer is optimized for **Real FPS** (actual swapchain presentations without discarded batch frames). Every frame rendered by the GPU is an independent, complete frame submitted and presented to the Wayland display server (`RENDER_BURST = 1`).

### Synchronization & Pipeline Architecture

1. **8 Swapchain Images (Maximum Hardware Depth)**:
   Instead of clamping to 3 images, the swapchain creates 8 images. This provides a deep queue that prevents the application from stalling when the GNOME Wayland compositor processes buffer releases asynchronously.
2. **GPU Semaphore Acquire Synchronization**:
   Eliminates host CPU stalls. Rather than blocking the CPU with `vkWaitForFences` on image acquisition, `vkAcquireNextImageKHR` signals an `acquire_semaphore`, which is passed to `vkQueueSubmit` at the `COLOR_ATTACHMENT_OUTPUT` stage. The GPU waits in hardware for buffer availability while the CPU advances immediately.
3. **Fixed 144 Hz Simulation**:
   Flight dynamics and plane aerodynamic animations remain stepped at a deterministic 144 Hz rate (`SIM_STEP = 1.0 / 144.0`), fully decoupled from the variable presentation cadence.

### Measuring Performance

Run the game normally or run the benchmark:

```sh
cargo run -p explora-engine --profile perf -- --fullscreen
cargo run -p explora-engine --profile perf -- --fullscreen --benchmark 3000
```

The runtime output explicitly reports **Theoretical FPS** and **Real FPS** in `FPS` units:

```text
benchmark: theoretical fps: 4452.0 FPS (3000 frames, 224.6 us/frame) | real fps: 4452.0 FPS (3000 presents) | acquire+wait+update 62 us submit 21 us present 130 us | sim+camera 3.1 us gpu 24 us
```

| Metric | Previous Batched Mode | Real-FPS Optimized Mode |
| :--- | :---: | :---: |
| **Real FPS (Presents/s)** | **548 FPS** | **~4,450 FPS** (**8.1× faster**) |
| **Theoretical FPS (Render Passes/s)** | 17,558 FPS | ~4,450 FPS |
| **Swapchain Image Queue** | 3 images | 8 images |
| **Acquire Synchronization** | CPU Fence Wait (Synchronous) | GPU Semaphore (Asynchronous) |
| **Passes per Present** | 32 (31 discarded) | 1 (0 discarded) |

### Path Towards 10,000 Real FPS

With CPU-side acquire stalls eliminated and GPU rasterization taking only 24 µs, the remaining barrier to 10,000 Real FPS is the Wayland compositor socket IPC (~130 µs per present):
- Wayland requires a round-trip protocol commit per present over the UNIX domain socket.
- Bypassing the compositor via zero-copy direct scanout ([NVIDIA-AMD DMA-BUF](nvidia-amd-dmabuf-direct-scanout.md)) or direct DRM display ([`VK_KHR_display`](../vulkan/guide/chapters/wsi.adoc)) eliminates this IPC latency.
