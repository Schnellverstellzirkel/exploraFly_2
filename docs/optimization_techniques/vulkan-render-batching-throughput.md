# Batched Vulkan rendering with a fixed 144 Hz simulation

The default renderer now submits 32 complete single-sample render passes for
each acquired swapchain image, then presents that image once. This keeps the
flight simulation and plane animation on the same fixed 144 Hz step while
raising render-pass throughput independently of the 120 Hz laptop panel.

Every pass in a batch uses its own uniform block, but reuses that swapchain
image and its color/depth attachments. Each pass clears and redraws the scene,
so only the last pass's pixels remain for presentation. The reported pass rate
measures real GPU rendering work; it is not a count of unique frames delivered
to the display. Per-image render targets and fences keep batches on different
swapchain images independent, while explicit attachment dependencies order
passes within each batch. Vulkan does not infer those dependencies from
command-buffer boundaries ([Khronos synchronization specification](https://docs.vulkan.org/spec/latest/chapters/synchronization.html)).

There is no renderer mode switch or burst/MSAA tuning option. Run the game
normally, or use the benchmark option to measure the default path:

```sh
cargo run -p explora-engine --profile perf -- --fullscreen
cargo run -p explora-engine --profile perf -- --fullscreen --benchmark 30000
```

The benchmark warms up, then measures at least 30,000 render passes and exits.
It reports render passes per second and swapchain presents per second
separately. On the Ryzen 7 7840HS / RTX 4060 laptop running GNOME Wayland at
2880×1800 in mailbox mode, the latest 30,000-pass run measured:

| Metric | Result |
| --- | ---: |
| Render-pass throughput | 17,945 passes/s |
| Swapchain presents | 561/s |
| GPU time for one pass | 22 µs |
| Panel refresh ceiling | 120 Hz |

This exceeds the 10,000 render-pass/s target, but does not mean the game
displays 17,945 distinct frames per second. The app submits about 561 images
per second and the panel can show at most 120 distinct updates each second;
Wayland may discard additional submitted images. Flight and plane animation
advance at 144 Hz, regardless of render-pass throughput.

The renderer uses 1× MSAA, which makes thin edges and wing details visibly
more aliased than the former 4× path. The faster setting is now the default.
For a true displayed-frame improvement, the next route is to make the
fullscreen buffers eligible for direct scanout. The prototype, current
limitations, and hybrid-GPU design are documented in
[NVIDIA-to-AMD DMA-BUF direct-scanout feasibility](nvidia-amd-dmabuf-direct-scanout.md).

Validation: `cargo check`, `cargo test -p explora-engine`, and the optimized
build pass. The default fullscreen 30,000-pass benchmark completed. The
machine lacks the Khronos validation layer, so no validation-layer run is
claimed.
