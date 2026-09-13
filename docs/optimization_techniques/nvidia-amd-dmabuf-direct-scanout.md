# NVIDIA Rendering into AMD-Owned DMA-BUFs: Prototype and Direct-Scanout Feasibility

Tested on 2026-09-13. This technique targets hybrid laptops where NVIDIA renders
the game and AMD drives the display. The proposed optimization is to transfer
the finished NVIDIA image into an AMD-owned, scanout-compatible DMA-BUF and
submit it to Mutter for direct scanout. If accepted, this would avoid Mutter's
GPU composition pass while retaining AMD display ownership.

The [standalone prototype](../../engine/examples/hybrid_import.rs) verifies
NVIDIA image import and GPU writing into an AMD allocation, followed by AMD
readback. It does **not** implement a rendered-image transfer or a Wayland
presenter. Direct scanout, reduced presentation latency, and an FPS improvement
from this technique remain unverified. The game measurements below describe
the existing Vulkan swapchain path, not the prototype.

## Machine and observations

- NVIDIA RTX 4060 Laptop, driver 580.173.02: game rendering.
- AMD 780M / RADV 25.2.8: connected eDP-2 panel, 2880×1800 at 120 Hz, scale 2.
- GNOME 46 / Mutter package 46.2-1ubuntu0.24.04.16, Wayland.
- `kms-modifiers` and `variable-refresh-rate` were already enabled. An enabled
  VRR feature does not prove that this panel is currently using VRR.
- The live Wayland registry advertised neither `wp_drm_lease_device_v1` nor
  `wp_linux_drm_syncobj_manager_v1`.

No desktop settings, GPU routing, extensions, or drivers were changed.

## What the measurements actually say

The old benchmark excluded present time, grouped several operations into
"acquire", and converted a sum of overlapping CPU/GPU work into "engine-fps".
It also called `vkAcquireNextImageKHR` with both synchronization handles null.
That is invalid Vulkan usage. The code now supplies and waits for an acquisition
fence, reports actual wall time per submission and present time, and allows
two seconds of warmup followed by 120 warmup submissions.

Measurements below use the isolated pre-texture renderer at commit `2b684d5`
with the presentation corrections and shader hash `7106bec7a1a7ee89`.
Concurrent texture edits were excluded from this comparison. Each completed
sample measures 1,200 submissions after warmup. No Wayland debug tracing during
these throughput measurements.

| Path | Wall time per submission, µs | Median, µs | Submission rate at median, FPS | GPU pass, µs |
| --- | --- | --- | --- | --- |
| Native Wayland, windowed, mailbox | 382.2, 348.3 | 365.3 | ≈2,738 | 27 |
| Native Wayland, fullscreen, mailbox | 382.4, 352.1, 261.2 | 352.1 | ≈2,840 | 30–33 |
| XWayland, fullscreen, immediate | 406.8, 408.2, 427.1 | 408.2 | ≈2,450 | 106–107 |

**Game throughput reached approximately 2,840 FPS at the median of the three
native Wayland fullscreen runs.** The fastest complete sample averaged
approximately 3,828 FPS; this is a short-run average, not an instantaneous peak
or a sustained-performance guarantee. Here FPS counts frames submitted for
presentation, including frames that the compositor may discard.

The panel was operating at 120 Hz, so it can display at most about 120 complete
updates per second in the observed synchronized presentation mode. A sustained
120 unique game frames per second was not established by these throughput
tests. GPU pass times of 30–33 µs cannot be inverted into whole-game FPS because
they omit CPU work, synchronization, and presentation.

One windowed run ended without benchmark output and was excluded. Windowed
extent was 2880×1646; fullscreen was 2880×1800. These short desktop runs have
substantial variability and do not establish a precise fullscreen speedup.
XWayland was slower than native Wayland in this fullscreen comparison.
An initial short FIFO test gave 10.44 ms per submission, but was taken before
the longer warmup was added and should not be treated as a controlled result.

Thousands of submissions per second are **not thousands of displayed frames**.
A separate fullscreen protocol trace produced 207 presented events, all with
flags `7` (vsync, hardware clock, hardware completion), and 4,608 discarded
events. None had bit `8`, the `zero_copy` flag. Outstanding feedback at exit is
not counted. Trace overhead makes this run unsuitable for throughput comparison.

Mutter offered a scanout tranche, but the buffers submitted by NVIDIA WSI used
a NVIDIA vendor modifier (`0x0300000000e08114`). Buffer compatibility is therefore
a strong candidate for the failed scanout, not a proven sole cause. A repeat
using the renderer snapshot with concurrent texture changes gave 203 presented
events, again all flags `7`. That diagnostic run reported 2,877 submissions/s;
its shorter sample and enabled tracing make it unsuitable for comparison with
the untraced throughput runs above.

The [presentation protocol](https://raw.githubusercontent.com/wayland-mirror/wayland-protocols/main/stable/presentation-time/presentation-time.xml)
defines `zero_copy` as passing the client buffer directly to display hardware.
[Mutter 46.2's scanout selection](https://raw.githubusercontent.com/GNOME/mutter/46.2/src/compositor/meta-compositor-view-native.c)
checks surface coverage, transitions, visibility, cursor handling, and other
conditions. Its [DMA-BUF scanout path](https://raw.githubusercontent.com/GNOME/mutter/46.2/src/wayland/meta-wayland-dma-buf.c)
imports the buffer for scanout and checks compatibility. Fullscreen is an
eligibility condition, not a guarantee.

## Proposed presentation path and verified interoperability step

The following is the proposed path, not the game's current implementation:

```text
NVIDIA optimal render target
    → GPU transfer into an AMD-allocated linear DMA-BUF
    → Wayland fullscreen surface
    → Mutter direct scanout, if buffer and surface are eligible
    → AMD display engine → internal panel
```

Keep normal rendering in NVIDIA-local, optimally tiled memory. Only the finished
color image crosses into an AMD-compatible allocation. This targets Mutter's
composition pass and potentially its extra output copy; it does not eliminate
the transfer between GPUs, Wayland control traffic, or display refresh timing.

The standalone experiment:

1. Finds AMD's render node through sysfs, without acquiring display ownership.
2. Allocates a 2880×1800 XRGB8888 GBM buffer with scanout/rendering usage and an
   explicit linear modifier. Pitch is 11,520 bytes.
3. Exports DMA-BUF and imports it as a dedicated NVIDIA Vulkan image.
4. Acquires foreign queue ownership, clears the entire image red on NVIDIA,
   releases ownership, and waits for completion.
5. Maps through AMD GBM and verifies all 5,184,000 RGB pixels.

**Interoperability result: PASS.** This verifies allocation, import, GPU writing,
and AMD readback for the tested format, dimensions, and driver versions.
GBM scanout usage alone does not prove KMS will accept a specific buffer for
the active plane. The probe does not submit a Wayland buffer, measure a real
render-target copy, or establish scanout latency. CPU mapping is diagnostic
only and must not be in the game's presentation loop.

The next implementation must use the compositor's DMA-BUF feedback to select
formats/modifiers, submit opaque fullscreen buffers with the correct dimensions
and scale, and synchronize both GPU completion and compositor buffer release.
This session lacks syncobj explicit-sync advertisement, so merely exporting a
Vulkan semaphore is insufficient: completion must be integrated with the
supported Wayland/DMA-BUF synchronization path, or initially waited on before
commit. Never overwrite a buffer before the compositor releases it.

The [DMA-BUF feedback protocol](https://raw.githubusercontent.com/wayland-mirror/wayland-protocols/main/unstable/linux-dmabuf/linux-dmabuf-unstable-v1.xml)
provides allocation preferences and scanout tranches. A successful presenter
must demonstrate `zero_copy` feedback and lower measured presentation latency
at comparable workload, not merely a higher submission counter.

The uncompressed color payload is 20.736 MB per frame: 2.49 GB/s at 120 Hz, but
41.47 GB/s at 2,000 submissions/s. Those are payload calculations, not measured
bus bandwidth. Present only the latest completed frame at a useful cadence;
copying every uncapped render risks making this route slower.

## Other routes

- **DRM lease/direct KMS:** can bypass the compositor's presentation loop with
  a granted lease or display ownership. This session advertises no lease
  protocol. An ordinary client cannot take over the active internal panel
  while Mutter owns it. A separate VT/direct-KMS session could preserve hybrid
  GPU roles but interrupts normal desktop use. See the
  [kernel's DRM ownership and leasing documentation](https://www.kernel.org/doc./html/next/gpu/drm-uapi.html).
- **NVIDIA-connected external display:** could remove the cross-GPU transfer
  for that display while the desktop stays on AMD. It requires an external
  display and verification of connector routing; none is currently connected.
- **Late input sampling and pacing:** reduce input age and wasted transfers.
  The current loop samples input before blocking acquisition; moving that
  sampling after acquisition is worth testing separately. This reduces latency,
  not compositor execution time.
- **Present thread, nested compositor, more swapchain images:** none establishes
  a compositor bypass. The repository already records a present-thread
  regression from driver contention; repeating that approach is not justified
  by these measurements.

## Reproduce the measurements and prototype

Run these commands from the repository root. Results reflect whichever renderer
is built locally; reproducing the table requires the snapshot described above.

```sh
cargo build -p explora-engine --profile perf
EXPLORA_PRESENT=mailbox target/perf/explora --fullscreen --benchmark 1200
env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET EXPLORA_PRESENT=immediate \
  target/perf/explora --fullscreen --benchmark 1200
cargo run -p explora-engine --example hybrid_import --profile perf

# Diagnostic run only; tracing affects performance.
EXPLORA_PRESENT_FEEDBACK=1 WAYLAND_DEBUG=1 \
  target/perf/explora --fullscreen --benchmark 360 > /tmp/explora-feedback.log 2>&1
```

`EXPLORA_PRESENT` accepts `mailbox`, `fifo`, or `immediate`, and rejects a mode
not offered by the driver. Defaults remain unchanged. The feedback option enables
the NVIDIA-supported Vulkan present-ID/wait features and attaches present IDs;
it does not wait for every frame to be displayed. In the trace, inspect
`wp_presentation_feedback…presented(...)`: the final argument contains the flags.
Bit `8` is the relevant scanout/overlay evidence, not the benchmark FPS value.

Validation: engine perf build and `cargo check` passed at the tested snapshot;
fullscreen feedback smoke run passed; hybrid import/clear/readback probe passed.
A later `cargo check --examples` encountered an unclosed delimiter in concurrently
edited `crates/airframe/src/airframe.rs`; those unrelated edits were left alone.
The final probe was rebuilt and passed again in the isolated checkout. Vulkan
validation layers are not installed, so validation-layer coverage is not claimed.
The synchronization requirement is documented by
[vkAcquireNextImageKHR](https://docs.vulkan.org/refpages/latest/refpages/source/vkAcquireNextImageKHR.html).
