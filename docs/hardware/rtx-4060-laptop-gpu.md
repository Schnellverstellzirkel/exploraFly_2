# RTX 4060 Laptop GPU in this workstation

Machine-specific notes for tuning `exploraFly_2`. Vendor architecture figures
describe the Ada family; machine queries identify what this laptop exposes at
runtime. Clocks, power and link rates are dynamic. Observations below were
collected on 2026-09-15 and should be rechecked after driver, kernel, firmware
or power-profile changes.

## The exact device

The Legion Slim 5 14APH8 contains a GeForce RTX 4060 Laptop GPU based on NVIDIA
Ada Lovelace, die AD107 (the kernel PCI device is `10de:28a0`, marketed in
`lspci` as AD107M). NVIDIA's [Nsight Graphics release notes identify the RTX
4060 Laptop GPU as AD107](https://developer.nvidia.com/nsight-graphics-2022_7).
The installed driver reports the device as a discrete NVIDIA GPU with 8 GiB of
GDDR6. Lenovo's [machine PSREF](https://psref.lenovo.com/syspool/Sys/PDF/Legion/Legion_Slim_5_14APH8/Legion_Slim_5_14APH8_Spec.PDF)
lists 8 GB GDDR6, a 2370 MHz boost clock and 105 W TGP for this model. The
105 W is the laptop's configured maximum, not a promise of a constant draw;
Lenovo also lists Dynamic Boost 2.0. NVIDIA's broader [laptop comparison
table](https://www.nvidia.com/en-us/geforce/laptops/compare/) spans 35–115 W
for the RTX 4060 Laptop GPU, which is a range across laptop designs, not this
machine's limit.

## Compute blocks and caches

The installed CUDA driver reports compute capability 8.9 and 24 streaming
multiprocessors (SMs). This matches the 3,072 CUDA cores in NVIDIA's laptop
comparison table divided by Ada's 128 CUDA cores per SM. The
[Ada tuning guide](https://docs.nvidia.com/cuda/ada-tuning-guide/) describes
the SM as supporting up to 48 resident warps, a 64K-entry 32-bit register
file (256 KiB), and 128 KiB of combined L1/texture cache and shared-memory
capacity. Shared memory can be configured up to 100 KiB per SM, with at most
99 KiB available to one block. Across this 24-SM GPU, that is at most 1,152
resident warps; register, shared-memory, block and instruction dependencies
can lower actual occupancy.

Ada SMs also contain fourth-generation Tensor Cores and third-generation RT
Cores. The architecture whitepaper describes four Tensor Cores and one RT
Core per SM. On this 24-SM configuration that implies 96 Tensor Cores and 24
RT Cores; these totals are derived from per-SM family organization, rather
than a separately published Lenovo SKU count. NVIDIA's
[Ada architecture overview](https://www.nvidia.com/en-us/geforce/ada-lovelace-architecture/)
describes the Tensor and RT generations. The current renderer is a raster and
fragment-shader workload: it does not use Tensor Cores, RT cores, CUDA, or
mesh shaders. Buying into those blocks does not make ordinary GLSL fragment
work faster by itself.

The Ada tuning guide reports twice the FP32 operations per cycle per SM for
compute capability 8.9 versus 8.0. That is family-level throughput context,
not a guarantee for this mobile SKU: sustained rate depends on shader
occupancy, clocks, power allocation and thermals. For this application,
fragment divergence in the exhaust march and the full-screen composite's
texture/ALU work matter more than peak FP32 arithmetic.

## Memory and interconnect

The laptop has 8 GB of GDDR6 on a 128-bit interface. A live CUDA Driver API
query reports 32 MiB of L2 and a maximum memory clock near 8,001 MHz. Using
the double-data-rate transfers of GDDR6 gives a theoretical bandwidth of
about 256 GB/s (`2 × 8.001 GT/s × 128 bits ÷ 8`); it is a calculation, not a
measured app bandwidth. The per-SM 128 KiB L1/texture/shared-memory complex
and 32 MiB device L2 make texture locality and cache reuse important for this
renderer. The 8 GB capacity is ample for its current scene, HDR targets and
small procedural noise volumes; capacity is not the present throughput
constraint.

The GPU's maximum host link is PCIe 4.0 x8. Live negotiated link generation
can change with device state, so the link should be sampled under load before
blaming PCIe for a graphics result. Current scene geometry and uniform data
are small; the workload does not continuously stream large buffers from host
memory.

## Power, CPU and display topology

The host is a Ryzen 7 7840HS (8 cores / 16 threads, Zen 4) with a Radeon 780M
iGPU. Native Rust builds already use `target-cpu=znver4`. The operating system
is Ubuntu 24.04.5, kernel 7.0.0-31-generic, GNOME/Mutter on Wayland, and the
installed proprietary NVIDIA driver is 580.173.02. `vulkaninfo --summary`
reports NVIDIA Vulkan 1.4.312 and also enumerates the Radeon 780M.

The app explicitly selects the RTX 4060 for rendering. The built-in
2880×1800, 120 Hz panel is connected to the AMD display device and GNOME/Mutter
manages the Wayland surface. This is a hybrid render/present path. It does not
prove a GPU-to-GPU copy for every frame; zero-copy scanout must be
established from presentation feedback, not inferred from the topology. The
panel can display at most 120 distinct updates per second even when the app
submits more images.

At the time of inspection the machine was on AC but `powerprofilesctl` was in
`power-saver`, the firmware profile was `low-power`, and AMD P-state exposed
the `powersave` governor with EPP `power`. A temporary `performance` profile
test was restored to `power-saver`. Power-profile changes can affect CPU-side
queue/presentation work, but they do not guarantee a higher GPU TGP; keep them
as explicit benchmark conditions rather than permanently overriding firmware
or thermal control.

The app's `real fps` benchmark counts successful `vkQueuePresentKHR` calls,
not images actually displayed. A 10,000-present benchmark on this tree, at
2880×1646 window extent with mailbox mode and one pass per present, measured
1,379.7 presents/s in the default scene and 1,189.5 presents/s with full boost
and bank forced. GPU pass times were 211 µs and 307 µs respectively; composite
was 91/110 µs and plume 16/91 µs. The test is a present-throughput measurement,
not a 1,400 Hz display claim. Re-run the exact scenarios after shader or
driver changes; clocks, thermal state, WSI backpressure and presentation
topology can move these figures.

## What this means for the renderer

- **Reduce full-screen work first.** At 2880×1646 the composite shades about
  4.74 million pixels per present. Its timestamp grows from 91 µs to 110 µs
  under boost/bank while it performs chromatic dispersion, speed streak taps,
  exposure, vignetting, bloom, sensor grain and tone mapping. Every saved
  texture lookup or transcendental applies to the whole frame.
- **Make volumetric work adaptive and reject empty rays early.** The boost/bank
  plume timestamp reaches 91 µs versus 16 µs in the default scene. Its ray
  march is branchy and texture-heavy; whole-ray bounds and fewer samples for
  low projected detail target this cost more directly than adding threads.
- **Use the caches deliberately.** Reuse low-frequency noise lookups across
  steps only where the visual signal tolerates it, keep hot sampling coherent,
  and avoid multiple high-resolution scene fetches for one composite pixel.
  A single packed 32-bit HDR scene image at this window extent occupies about
  18.1 MiB, so it can fit within the measured 32 MiB L2 in isolation; scene
  attachments, source offsets and other active data compete for that cache.
- **Don't optimize for unsupported blocks.** The present workload has only two
  merged airframe draw calls and no matrix-heavy compute stage suitable for
  Tensor Cores. CUDA interop or ray tracing would add complexity without
  addressing the measured full-screen and volume costs.
- **Separate GPU time from the WSI ceiling.** `queue_present` and acquire
  backpressure account for much of wall time. If a shader change reduces GPU
  time but not present cadence, investigate the AMD-owned display path and
  Vulkan synchronization. The current panel refresh and the benchmark's
  present-call counter are different limits.

## Sources and live probes

- [Lenovo PSREF: Legion Slim 5 14APH8](https://psref.lenovo.com/syspool/Sys/PDF/Legion/Legion_Slim_5_14APH8/Legion_Slim_5_14APH8_Spec.PDF)
- [NVIDIA Nsight Graphics 2022.7: AD107 RTX 4060 Laptop GPU](https://developer.nvidia.com/nsight-graphics-2022_7)
- [NVIDIA GeForce RTX Laptop comparison: CUDA cores, memory and interface width](https://www.nvidia.com/en-us/geforce/laptops/compare/)
- [NVIDIA Ada GPU Architecture whitepaper](https://images.nvidia.com/aem-dam/Solutions/geforce/ada/nvidia-ada-gpu-architecture.pdf)
- [NVIDIA Ada GPU Architecture Tuning Guide](https://docs.nvidia.com/cuda/ada-tuning-guide/)
- [NVIDIA CUDA architecture matrix](https://docs.nvidia.com/datacenter/tesla/drivers/cuda-toolkit-and-architecture-matrix.html)
- [CUDA Driver API device attributes](https://docs.nvidia.com/cuda/cuda-driver-api/group__CUDA__TYPES.html)
- [NVIDIA Linux PRIME Render Offload documentation](https://download.nvidia.com/XFree86/Linux-x86_64/450.66/README/primerenderoffload.html)

Recheck machine-specific data with `lspci -nnk`, `nvidia-smi -q`,
`vulkaninfo --summary`, `/sys/class/drm/*/status`, and
`powerprofilesctl get`. Extension enumeration means that the driver exposes a
capability; it does not mean this engine enables or uses that feature.
