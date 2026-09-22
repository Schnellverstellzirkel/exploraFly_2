// Static fact cards and resource-size arithmetic.
// Every quantity is either a hardware specification, a local probe reading,
// or an arithmetic derivation from named constants. Each fact carries
// provenance. Calculations are labeled as calculations.

import { prov, SNAPSHOT } from "./hardware.js";

// ---- Constants mirrored from the engine (see provenance on each card) ----
export const CONST = {
  UBO_BYTES: 1824,
  NODE_COUNT: 23,
  SIM_HZ: 144,
  VERTEX_BYTES: 28,
  TERRAIN_CHUNK_COUNT: 1024, // (1024/32)^2 from crates/world
  TERRAIN_COMMAND_BYTES: 20, // sizeof(VkDrawIndexedIndirectCommand)
  DRAW_COMMAND_BYTES: 16, // sizeof(VkDrawIndirectCommand)
  VEGETATION_COMMAND_CAPACITY: 4096,
  CANOPY_COMMAND_CAPACITY: 16384,
  RT_INSTANCE_BYTES: 64,
  CONE_VERTEX_BUFFER_BYTES: 4096,
  TRAIL_VERTEX_BUFFER_BYTES: 262144,
  VEGETATION_VISIBLE_CAPACITY: 262144,
  DEDICATE_ABOVE_BYTES: 16 * 1024 * 1024,
  GPU_STAMPS_PER_FRAME: 11,
  CUDA_PER_SM: 128,
  MAX_RESIDENT_WARPS_PER_SM: 48,
};

// Resource-size arithmetic. Pure functions, unit-tested.
export function uniformLayout() {
  const matrices = 2 + CONST.NODE_COUNT; // viewProj, invViewProj, nodes[23]
  const matrixBytes = matrices * 64;
  const tailVec4 = 14;
  const tailBytes = tailVec4 * 16;
  return {
    matrices,
    matrixBytes,
    tailVec4,
    tailBytes,
    total: matrixBytes + tailBytes,
  };
}

export function frameBufferLayout() {
  const terrainBytes = CONST.TERRAIN_CHUNK_COUNT * CONST.TERRAIN_COMMAND_BYTES;
  const vegetationBytes = CONST.VEGETATION_COMMAND_CAPACITY * CONST.DRAW_COMMAND_BYTES;
  const canopyBytes = CONST.CANOPY_COMMAND_CAPACITY * CONST.DRAW_COMMAND_BYTES;
  const uboOffset = 0;
  const terrainOffset = uboOffset + CONST.UBO_BYTES;
  const vegetationOffset = terrainOffset + terrainBytes;
  const canopyOffset = vegetationOffset + vegetationBytes;
  const total = canopyOffset + canopyBytes;
  return {
    uboOffset,
    terrainOffset,
    terrainBytes,
    vegetationOffset,
    vegetationBytes,
    canopyOffset,
    canopyBytes,
    total,
  };
}

export function vegetationOutputBytes() {
  // DrawIndirectCommand header (16 B) + VEGETATION_VISIBLE_CAPACITY * [u32; 4].
  // Mirrors VEGETATION_OUTPUT_BYTES in engine/src/plane/mod.rs:29-31.
  return CONST.DRAW_COMMAND_BYTES + CONST.VEGETATION_VISIBLE_CAPACITY * 16;
}

export function smTotals(smCount) {
  return {
    cudaCores: smCount * CONST.CUDA_PER_SM,
    rtCores: smCount * 1,
    tensorCores: smCount * 4,
    maxResidentWarps: smCount * CONST.MAX_RESIDENT_WARPS_PER_SM,
  };
}

export function gddr6BandwidthGBs(memoryClockGTs, busBits) {
  // Theoretical: 2 (DDR) * clock(GT/s) * bus(bits) / 8 = GB/s. Calculation only.
  return (2 * memoryClockGTs * busBits) / 8;
}

// ---- Fact cards ----
export function factCards() {
  const ubo = uniformLayout();
  const frame = frameBufferLayout();
  const sm = smTotals(24);
  const band = gddr6BandwidthGBs(8.001, 128);

  return [
    {
      id: "fact-cpu",
      title: "CPU topology",
      value: "8 cores, 16 SMT contexts",
      body:
        "Each core: 32 KiB L1i, 32 KiB L1d, 1 MiB L2. Shared 16 MiB L3. " +
        "Confirmed by local lscpu instances; AMD lists the same aggregates.",
      kind: "probe+spec",
      provenance: [
        prov("code", "lscpu (2026-09-22)"),
        prov("url", "https://www.amd.com/en/products/processors/laptop/ryzen/7000-series/amd-ryzen-7-7840hs.html", {
          limits: "Aggregate caches only; per-core split is arithmetic over 8 cores.",
        }),
      ],
    },
    {
      id: "fact-ram",
      title: "System RAM",
      value: "30.5 GiB OS-visible; nominal 32 GB LPDDR5x-6400",
      body:
        "MemTotal 32019116 kB on 2026-09-22. The 32 GB LPDDR5x-6400 " +
        "configuration is inferred from observed capacity plus Lenovo " +
        "PSREF listings; firmware memory details were inaccessible.",
      kind: "probe+inferred",
      provenance: [
        prov("code", "MemTotal (2026-09-22)"),
        prov("url", "https://psref.lenovo.com/syspool/Sys/PDF/Legion/Legion_Slim_5_14APH8/Legion_Slim_5_14APH8_Spec.PDF", {
          limits: "Lists 16 GB or 32 GB soldered LPDDR5x-6400 for the platform.",
        }),
      ],
    },
    {
      id: "fact-gpu",
      title: "GPU compute",
      value: "24 SMs, 3,072 CUDA cores, 32 MiB L2",
      body:
        "SM count and L2 from the installed driver. 3,072 cores = 24 SMs x " +
        "128 cores/SM from the NVIDIA laptop comparison table and the Ada " +
        "whitepaper. 8 GB GDDR6, 128-bit.",
      kind: "probe+spec",
      provenance: [
        prov("code", "nvidia-smi / CUDA query (2026-09-22)"),
        prov("url", "https://www.nvidia.com/en-us/geforce/laptops/compare/", {
          limits: "Marketing comparison table for the product family.",
        }),
        prov("url", "https://images.nvidia.com/aem-dam/Solutions/geforce/ada/nvidia-ada-gpu-architecture.pdf", {
          published: "2022-09-20",
          limits: "Per-SM organization for AD10x.",
        }),
      ],
    },
    {
      id: "fact-sm-derived",
      title: "Derived SM totals (calculation)",
      value:
        `${sm.cudaCores} CUDA cores, ${sm.rtCores} RT cores, ` +
        `${sm.tensorCores} Tensor cores, up to ${sm.maxResidentWarps} resident warps`,
      body:
        "Arithmetic: per-SM whitepaper counts times 24 SMs. The warp " +
        "figure is the tuning-guide ceiling before resource limits. " +
        "Tensor cores exist; this game does not use them.",
      kind: "calculation",
      calculation:
        "24 SMs x 128 CUDA/SM = 3,072; x 1 RT/SM = 24; x 4 Tensor/SM = 96; " +
        "x 48 resident warps/SM = 1,152.",
      provenance: [
        prov("url", "https://images.nvidia.com/aem-dam/Solutions/geforce/ada/nvidia-ada-gpu-architecture.pdf", {
          published: "2022-09-20",
        }),
        prov("url", "https://docs.nvidia.com/cuda/ada-tuning-guide/", {
          limits: "Ceiling only; register and shared-memory pressure reduce occupancy.",
        }),
      ],
    },
    {
      id: "fact-bandwidth",
      title: "Theoretical GDDR6 bandwidth (calculation)",
      value: `about ${band.toFixed(0)} GB/s`,
      body:
        "Arithmetic from an observed memory clock near 8.001 GT/s and a " +
        "128-bit bus. Not a measured application bandwidth.",
      kind: "calculation",
      calculation: "2 x 8.001 GT/s x 128 bit / 8 = 256.032 GB/s",
      provenance: [
        prov("code", "docs/hardware/rtx-4060-laptop-gpu.md (CUDA clock query, 2026-09-15)", {
          published: "2026-09-15",
        }),
      ],
    },
    {
      id: "fact-sim",
      title: "Simulation frequency",
      value: "144 Hz fixed step",
      body:
        "SIM_STEP = 1.0/144.0. Wall delta clamped to 100 ms bounds a " +
        "frame to at most 15 steps. Pose is interpolated between steps.",
      kind: "code",
      provenance: [
        prov("code", "crates/sim/src/flight.rs:7"),
        prov("code", "engine/src/frame_loop.rs:144"),
      ],
    },
    {
      id: "fact-ubo",
      title: "Uniform block size",
      value: "1,824 bytes",
      body:
        "Layout: viewProj 64 B, invViewProj 64 B, nodes[23] 1,472 B, " +
        "14 tail vec4 = 224 B.",
      kind: "code",
      calculation: "64 + 64 + 23x64 + 14x16 = 1,824",
      provenance: [prov("code", "engine/src/ubo.rs:29 (asserted at engine/src/ubo.rs:111)")],
    },
    {
      id: "fact-frame-bytes",
      title: "Per-image FRAME_BYTES (calculation)",
      value: `${frame.total.toLocaleString("en-US")} bytes`,
      body:
        "Uniform block plus terrain, vegetation and canopy indirect " +
        "command arrays in one host-visible buffer per swapchain image.",
      kind: "calculation",
      calculation:
        `${CONST.UBO_BYTES} + ${CONST.TERRAIN_CHUNK_COUNT}x${CONST.TERRAIN_COMMAND_BYTES}` +
        ` + ${CONST.VEGETATION_COMMAND_CAPACITY}x${CONST.DRAW_COMMAND_BYTES}` +
        ` + ${CONST.CANOPY_COMMAND_CAPACITY}x${CONST.DRAW_COMMAND_BYTES}` +
        ` = ${frame.total}`,
      provenance: [
        prov("code", "engine/src/plane/mod.rs:16-26"),
        prov("code", "crates/world/src/lib.rs:42-43 (terrain chunk count)"),
        prov("code", "crates/world/src/vegetation.rs:21,26 (command capacities)"),
      ],
    },
    {
      id: "fact-rt-instance",
      title: "RT instance stride",
      value: "64 bytes",
      body:
        "VkAccelerationStructureInstanceKHR: transform 48 B + two packed " +
        "u32 + device reference 8 B + padding to 16-byte alignment.",
      kind: "code",
      provenance: [prov("code", "engine/src/plane/mod.rs:36-38")],
    },
    {
      id: "fact-fx-buffers",
      title: "FX vertex rings (per image)",
      value: "cone 4,096 B; trails 262,144 B",
      body: "Permanently mapped host-visible buffers overwritten each frame.",
      kind: "code",
      provenance: [prov("code", "engine/src/plane/frames.rs:533,538")],
    },
    {
      id: "fact-veg-output",
      title: "GPU vegetation cull output buffer (calculation)",
      value: `${vegetationOutputBytes().toLocaleString("en-US")} bytes`,
      body:
        "Used only by the DEBUG_ONLY GPU cull path. Normal builds cull " +
        "on the CPU and never write this buffer.",
      kind: "calculation",
      calculation:
        `${CONST.DRAW_COMMAND_BYTES} + ${CONST.VEGETATION_VISIBLE_CAPACITY} x 16 = ${vegetationOutputBytes()}`,
      provenance: [
        prov("code", "engine/src/plane/mod.rs:29-31"),
        prov("code", "engine/src/flags.rs:218 (gpu_vegetation is DEBUG_ONLY)"),
      ],
    },
    {
      id: "fact-panel",
      title: "Panel",
      value: "2880x1800, 120 Hz",
      body:
        "Internal panel on the AMD display path. At most 120 distinct " +
        "updates per second regardless of submission rate.",
      kind: "probe+spec",
      provenance: [
        prov("code", "/sys/class/drm modes (2026-09-22)"),
        prov("url", "https://psref.lenovo.com/syspool/Sys/PDF/Legion/Legion_Slim_5_14APH8/Legion_Slim_5_14APH8_Spec.PDF"),
      ],
    },
    {
      id: "fact-target",
      title: "Project performance target (context only)",
      value: "1,000 presents/s = 1 ms budget",
      body:
        "The project targets 1,000 complete presentation submissions per " +
        "second on the RTX 4060 Laptop GPU. This page shows no " +
        "measurement; the target is context for chapter 5.",
      kind: "project-target",
      provenance: [prov("code", "AGENTS.md (project contract)")],
    },
    {
      id: "fact-stamps",
      title: "GPU timestamps per measured frame",
      value: "11",
      body:
        "q0 start plus one stamp after each measured stage: TLAS/opaque, " +
        "terrain, trees, canopy, clouds, sky, plume, trails, glass, " +
        "composite.",
      kind: "code",
      provenance: [prov("code", "engine/src/plane/mod.rs:32-35")],
    },
    {
      id: "fact-pcie",
      title: "Host link",
      value: "PCIe 4.0 x8 maximum",
      body:
        "Maximum host-to-device link for this GPU. Negotiated rate can " +
        "vary with device state.",
      kind: "spec",
      provenance: [prov("code", "docs/hardware/rtx-4060-laptop-gpu.md")],
    },
    {
      id: "fact-snapshot",
      title: "Snapshot",
      value: `${SNAPSHOT.machine}, inspected ${SNAPSHOT.inspectionDate}`,
      body:
        `Source revision ${SNAPSHOT.sourceRevision}. ${SNAPSHOT.os}. ` +
        `${SNAPSHOT.session}. Driver ${SNAPSHOT.driver}.`,
      kind: "snapshot",
      provenance: [prov("code", "docs/development/architecture-explainer.md")],
    },
  ];
}
