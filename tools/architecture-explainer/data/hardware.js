// Hardware profile for the exploraFly architecture explainer.
// Every fact carries provenance: a code reference or source URL, a
// publication/update date where one exists, an access date, and the
// applicability limits that apply to the claim.

export const SNAPSHOT = {
  id: "legion-slim5-2026-09-22",
  machine: "Lenovo Legion Slim 5 14APH8",
  inspectionDate: "2026-09-22",
  localNoteDate: "2026-09-15",
  sourceRevision: "b76d732",
  os: "Ubuntu 24.04.5, kernel 7.0.0-31-generic",
  session: "GNOME/Mutter on Wayland",
  driver: "NVIDIA 580.173.02",
  notes:
    "Local probes run on 2026-09-22 confirm topology and capacities. " +
    "The authored note docs/hardware/rtx-4060-laptop-gpu.md records the " +
    "2026-09-15 inspection; this page's snapshot date is the later recheck.",
};

// Provenance helper. kind is "code" or "url".
export function prov(kind, ref, extra = {}) {
  return {
    kind,
    ref,
    published: extra.published ?? null,
    accessed: extra.accessed ?? SNAPSHOT.inspectionDate,
    limits: extra.limits ?? "",
  };
}

const ACCESS = SNAPSHOT.inspectionDate;

export const HARDWARE = {
  cpu: {
    id: "cpu",
    name: "Ryzen 7 7840HS",
    arch: "Zen 4",
    cores: 8,
    threadsPerCore: 2,
    smtContexts: 16,
    l1iPerCoreKiB: 32,
    l1dPerCoreKiB: 32,
    l2PerCoreKiB: 1024,
    l3SharedMiB: 16,
    provenance: [
      prov("url", "https://www.amd.com/en/products/processors/laptop/ryzen/7000-series/amd-ryzen-7-7840hs.html", {
        published: null,
        accessed: ACCESS,
        limits:
          "AMD lists aggregate caches: L1 512 KB, L2 8 MB, L3 16 MB. " +
          "Per-core 32 KiB L1i, 32 KiB L1d and 1 MiB L2 follow from the " +
          "8-core division and match local lscpu instances.",
      }),
      prov("code", "lscpu (local probe 2026-09-22)", {
        accessed: ACCESS,
        limits: "Reports 16 threads, 8 cores, L1d 256 KiB x8, L1i 256 KiB x8, L2 8 MiB x8, L3 16 MiB x1.",
      }),
      prov("code", ".cargo/config.toml (target-cpu=znver4)", {
        accessed: ACCESS,
        limits: "Build flag only; does not restate cache sizes.",
      }),
    ],
  },

  ram: {
    id: "ram",
    name: "System RAM",
    osVisibleGiB: 30.5,
    nominalConfig: "32 GB LPDDR5x-6400 (soldered)",
    nominalInferred: true,
    provenance: [
      prov("code", "MemTotal 32019116 kB (local probe 2026-09-22)", {
        accessed: ACCESS,
        limits: "OS-visible capacity after firmware reservation. Equals 30.5 GiB.",
      }),
      prov("url", "https://psref.lenovo.com/syspool/Sys/PDF/Legion/Legion_Slim_5_14APH8/Legion_Slim_5_14APH8_Spec.PDF", {
        published: null,
        accessed: ACCESS,
        limits:
          "Lenovo PSREF lists 16 GB or 32 GB soldered LPDDR5x-6400 for this " +
          "platform. Firmware memory-details probes were inaccessible, so the " +
          "exact SKU speed is inferred from observed capacity plus this listing.",
      }),
    ],
  },

  gpu: {
    id: "gpu",
    name: "GeForce RTX 4060 Laptop GPU",
    die: "AD107 (Ada Lovelace)",
    pciId: "10de:28a0",
    smCount: 24,
    cudaCores: 3072,
    l2MiB: 32,
    vramGiB: 8,
    vramType: "GDDR6",
    memoryBusBits: 128,
    pcie: "PCIe 4.0 x8 (maximum host link)",
    computeCapability: "8.9",
    provenance: [
      prov("code", "nvidia-smi / CUDA driver query (local probe 2026-09-22)", {
        accessed: ACCESS,
        limits: "Reports 24 SMs, 8188 MiB, driver 580.173.02, 32 MiB L2, CC 8.9.",
      }),
      prov("code", "docs/hardware/rtx-4060-laptop-gpu.md", {
        published: "2026-09-15",
        accessed: ACCESS,
        limits: "Authored local note. Records the earlier inspection; clocks and power state are dynamic.",
      }),
      prov("url", "https://www.nvidia.com/en-us/geforce/laptops/compare/", {
        published: null,
        accessed: ACCESS,
        limits: "NVIDIA laptop comparison table: 3,072 CUDA cores, 8 GB GDDR6, 128-bit for RTX 4060 Laptop GPU. Family table, not a live reading.",
      }),
      prov("url", "https://images.nvidia.com/aem-dam/Solutions/geforce/ada/nvidia-ada-gpu-architecture.pdf", {
        published: "2022-09-20 (Ada whitepaper)",
        accessed: ACCESS,
        limits: "Architecture whitepaper. Describes AD10x SM organization, not this laptop's clocks or power limit.",
      }),
      prov("url", "https://docs.nvidia.com/cuda/ada-tuning-guide/", {
        published: "CUDA 12.0 era",
        accessed: ACCESS,
        limits: "Occupancy and register/shared-memory limits are family-level guidance, not measured occupancy of this workload.",
      }),
      prov("url", "https://psref.lenovo.com/syspool/Sys/PDF/Legion/Legion_Slim_5_14APH8/Legion_Slim_5_14APH8_Spec.PDF", {
        published: null,
        accessed: ACCESS,
        limits: "8 GB GDDR6, 2370 MHz boost, 105 W TGP configured maximum for this model. TGP is not a constant draw promise.",
      }),
    ],
  },

  smInternals: {
    id: "sm",
    name: "Inside an Ada SM",
    schedulerPartitions: 4,
    warpsPerPartitionNote: "32-thread warps; four scheduler partitions per SM",
    maxResidentWarpsPerSm: 48,
    registerFileKiB: 256,
    l1SharedKiB: 128,
    sharedMaxKiB: 100,
    rtCoresPerSm: 1,
    tensorCoresPerSm: 4,
    tensorCoresUsedByGame: false,
    provenance: [
      prov("url", "https://images.nvidia.com/aem-dam/Solutions/geforce/ada/nvidia-ada-gpu-architecture.pdf", {
        published: "2022-09-20",
        accessed: ACCESS,
        limits: "Whitepaper: four processing blocks per SM, each with a warp scheduler; 128 CUDA cores, one RT core, four Tensor cores, 128 KB L1/shared per SM.",
      }),
      prov("url", "https://docs.nvidia.com/cuda/ada-tuning-guide/", {
        published: "CUDA 12.0 era",
        accessed: ACCESS,
        limits: "Up to 48 resident warps per SM, 64K-entry register file, shared-memory carveouts 0/8/16/32/64/100 KB. Actual occupancy depends on register, shared-memory and dependency pressure; not measured here.",
      }),
      prov("code", "engine/src/gfx.rs (rt_available, mesh_available probes)", {
        accessed: ACCESS,
        limits: "Game uses raster/fragment paths plus optional ray-query shadows and mesh shaders. No Tensor Core or CUDA usage.",
      }),
    ],
  },

  display: {
    id: "display",
    name: "Presentation path",
    rendererGpu: "NVIDIA RTX 4060 (render)",
    compositor: "Wayland / Mutter",
    displayController: "AMD Radeon 780M (Phoenix1, 1002:15bf)",
    panel: "2880x1800, 120 Hz",
    sharedMemory: true,
    copyVersusScanout: "unspecified",
    provenance: [
      prov("code", "lspci /sys/class/drm /proc (local probe 2026-09-22)", {
        accessed: ACCESS,
        limits: "eDP-2 connected on card2 (AMD); session Type=wayland; NVIDIA and AMD PCI devices both present.",
      }),
      prov("code", "docs/hardware/rtx-4060-laptop-gpu.md", {
        published: "2026-09-15",
        accessed: ACCESS,
        limits:
          "Panel connected to the AMD display device; Mutter manages the Wayland surface. " +
          "Hybrid topology does not prove per-frame GPU-to-GPU copy; zero-copy scanout must come " +
          "from presentation feedback, which this snapshot does not include.",
      }),
      prov("code", "docs/rendering/weather-and-quality.md (119.96 Hz observation, accessed 2026-09-20)", {
        accessed: "2026-09-20",
        limits: "Refresh observed near 120 Hz. Panel can show at most 120 distinct updates per second.",
      }),
    ],
  },
};

export const DOMAIN_LABELS = {
  cpu: "CPU",
  gpu: "GPU",
  memory: "Memory",
  display: "Display",
  sync: "Synchronization",
  build: "Build time",
  inactive: "Not in native game",
};
