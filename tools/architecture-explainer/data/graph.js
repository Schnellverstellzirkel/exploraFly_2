// Module and resource relationship graph for the architecture explainer.
// Coordinates are logical schematic positions in a 1600x1000 scene box.
// Drawings are logical architecture schematics, not physical die layouts.
// Nodes never imply fixed core assignment or permanently assigned SMs.

export const LEVELS = [
  { id: 0, key: "system", label: "System" },
  { id: 1, key: "modules", label: "Modules and resources" },
  { id: 2, key: "internals", label: "Core / SM internals" },
];

// domain: cpu | gpu | memory | display | sync | build | inactive
// level: minimum detail level at which the node appears
// group: optional parent id (children draw inside parents when visible)
export const NODES = [
  // ---- System level ----
  {
    id: "host",
    label: "Host: Ryzen 7 7840HS",
    domain: "cpu",
    level: 0,
    x: 40, y: 40, w: 460, h: 300,
    detail:
      "Eight Zen 4 cores, each with two SMT contexts. Local probes confirm " +
      "the topology. The game runs a window thread, one combined simulation " +
      "/ render thread, and one audio thread. Nothing in the frame is pinned " +
      "to a fixed core for correctness.",
    codeRefs: ["engine/src/main.rs:70", "engine/src/app.rs:124", "engine/src/frame_loop.rs:28", "engine/src/audio.rs:63"],
    resources: ["cpu-l3"],
    dependsOn: [],
  },
  {
    id: "sysram",
    label: "System RAM (about 30.5 GiB visible)",
    domain: "memory",
    level: 0,
    x: 40, y: 370, w: 460, h: 160,
    detail:
      "Nominal 32 GB LPDDR5x-6400 soldered configuration, inferred from " +
      "observed capacity and Lenovo platform specifications. Firmware " +
      "memory-details probes were inaccessible. Host-visible Vulkan " +
      "allocations live in system memory; host-visible is an access " +
      "property, not a guarantee that the allocation is ordinary RAM.",
    codeRefs: ["engine/src/gfx.rs:176", "engine/src/plane/frames.rs:77"],
    resources: ["frame-ubo", "staging"],
    dependsOn: [],
  },
  {
    id: "pcie",
    label: "PCIe 4.0 x8 (max host link)",
    domain: "memory",
    level: 0,
    x: 540, y: 370, w: 240, h: 160,
    detail:
      "Maximum host-to-device link for this GPU is PCIe 4.0 x8. Negotiated " +
      "link rate can change with device state. Startup uploads and " +
      "per-frame host writes cross this link only when the memory is not " +
      "already device-resident; the workload does not continuously stream " +
      "large buffers.",
    codeRefs: ["docs/hardware/rtx-4060-laptop-gpu.md"],
    resources: [],
    dependsOn: ["sysram", "dGPU"],
  },
  {
    id: "dGPU",
    label: "GeForce RTX 4060 Laptop (AD107)",
    domain: "gpu",
    level: 0,
    x: 820, y: 40, w: 480, h: 320,
    detail:
      "24 SMs and 32 MiB L2 confirmed through the installed driver. 3,072 " +
      "CUDA cores, 8 GB GDDR6 and a 128-bit interface from NVIDIA " +
      "specifications. The application explicitly selects this device for " +
      "rendering.",
    codeRefs: ["engine/src/gfx.rs:266", "engine/src/gfx.rs:374"],
    resources: ["vram", "gpu-l2"],
    dependsOn: [],
  },
  {
    id: "vram",
    label: "8 GB GDDR6 (device local)",
    domain: "memory",
    level: 0,
    x: 820, y: 370, w: 230, h: 160,
    detail:
      "Persistent geometry, textures, acceleration structures and " +
      "device-local render targets live here. Allocations at or above " +
      "16 MiB request dedicated allocations.",
    codeRefs: ["engine/src/plane/geometry.rs:43", "engine/src/gfx.rs:19"],
    resources: ["geo-vertex", "geo-index", "textures"],
    dependsOn: [],
  },
  {
    id: "gpu-l2",
    label: "32 MiB L2 (shared)",
    domain: "gpu",
    level: 0,
    x: 1070, y: 370, w: 230, h: 160,
    detail:
      "Device-wide L2 reported by a live CUDA query. Capacity context for " +
      "texture locality; no cache-hit rate is claimed for this workload.",
    codeRefs: ["docs/hardware/rtx-4060-laptop-gpu.md"],
    resources: [],
    dependsOn: [],
  },
  {
    id: "present",
    label: "Presentation: NVIDIA render -> Wayland/Mutter -> AMD 780M -> 2880x1800 120 Hz panel",
    domain: "display",
    level: 0,
    x: 40, y: 540, w: 1260, h: 120,
    detail:
      "The NVIDIA device renders. GNOME/Mutter on Wayland presents the " +
      "surface. The AMD Radeon 780M display controller drives the internal " +
      "2880x1800, 120 Hz panel. The iGPU scanout path uses shared system " +
      "memory. Whether each frame is copied or scanned out directly is " +
      "left unspecified; this snapshot does not include presentation " +
      "feedback that would settle it.",
    codeRefs: ["engine/src/gfx.rs:1327", "engine/src/gfx.rs:1473", "docs/hardware/rtx-4060-laptop-gpu.md"],
    resources: ["swapchain"],
    dependsOn: ["dGPU", "igpu"],
  },
  {
    id: "igpu",
    label: "Radeon 780M display controller",
    domain: "display",
    level: 0,
    x: 820, y: 700, w: 480, h: 100,
    detail:
      "Integrated RDNA3 graphics on the Phoenix die drives the internal " +
      "panel. It is the display controller in this topology, not the " +
      "renderer.",
    codeRefs: ["docs/hardware/rtx-4060-laptop-gpu.md"],
    resources: [],
    dependsOn: [],
  },

  // ---- Module level: threads and pipeline resources ----
  {
    id: "win-thread",
    label: "Window / event thread",
    domain: "cpu",
    level: 1,
    group: "host",
    x: 60, y: 80, w: 200, h: 70,
    detail:
      "winit event loop. Publishes keyboard input through three atomics " +
      "(exit, keys, ui). Sleeps when idle. The render thread never waits " +
      "on this thread.",
    codeRefs: ["engine/src/main.rs:70", "engine/src/app.rs:201", "engine/src/app.rs:222"],
    resources: ["input-atomics"],
    dependsOn: [],
  },
  {
    id: "sim-thread",
    label: "Simulation / render thread",
    domain: "cpu",
    level: 1,
    group: "host",
    x: 280, y: 80, w: 200, h: 70,
    detail:
      "One combined thread: fixed 144 Hz flight simulation steps, pose " +
      "interpolation, camera and HUD updates, uniform packing, command " +
      "submission, and display pacing. Pinned cores, locked memory, " +
      "SCHED_FIFO.",
    codeRefs: ["engine/src/frame_loop.rs:28", "engine/src/frame_loop.rs:144", "engine/src/vendor.rs:163"],
    resources: ["frame-ubo", "indirect"],
    dependsOn: ["win-thread"],
  },
  {
    id: "audio-thread",
    label: "Audio synthesis thread (explora-audio)",
    domain: "cpu",
    level: 1,
    group: "host",
    x: 60, y: 170, w: 200, h: 60,
    detail:
      "Separate thread. Renders fixed 480-frame stereo blocks from a " +
      "lock-free DSP graph driven by atomics the render thread publishes. " +
      "Opens ALSA through libloading. Disable with EXPLORA_AUDIO=0.",
    codeRefs: ["engine/src/audio.rs:63", "engine/src/audio.rs:176", "crates/sim/src/audio/mod.rs:31"],
    resources: ["input-atomics"],
    dependsOn: ["sim-thread"],
  },
  {
    id: "build-time",
    label: "Build time (cargo build)",
    domain: "build",
    level: 1,
    x: 40, y: 700, w: 460, h: 150,
    detail:
      "Aircraft generation and baking write OUT_DIR/airframe.bin. GLSL " +
      "compiles to SPIR-V with shaderc on scoped worker threads, cached by " +
      "content hash. Landmark constants are generated into " +
      "world_generated.inc. None of this runs at launch.",
    codeRefs: ["engine/build.rs:115", "engine/build.rs:59", "engine/build.rs:401", "engine/shader_cache.rs"],
    resources: ["baked-airframe", "spirv-blobs"],
    dependsOn: [],
  },
  {
    id: "startup",
    label: "Startup on render thread",
    domain: "cpu",
    level: 1,
    group: "host",
    x: 280, y: 170, w: 200, h: 60,
    detail:
      "Creates instance/device, probes capabilities, builds Vulkan " +
      "pipelines from included SPIR-V, prepares world resources, uploads " +
      "persistent geometry and textures through staged one-time submits, " +
      "and records reusable command buffers once per swapchain image.",
    codeRefs: ["engine/src/gfx.rs:266", "engine/src/plane/build.rs:22", "engine/src/gfx.rs:1031"],
    resources: ["geo-vertex", "geo-index", "textures", "cmd-buffers"],
    dependsOn: ["build-time"],
  },
  {
    id: "cap-probe",
    label: "Capability probes (RT, mesh)",
    domain: "cpu",
    level: 1,
    group: "host",
    x: 60, y: 250, w: 420, h: 50,
    detail:
      "rt_available and mesh_available come from device feature queries. " +
      "EXPLORA_RT_SHADOWS and EXPLORA_MESH_SHADERS are DEBUG_ONLY toggles; " +
      "release builds choose by capability alone. Both paths are " +
      "capability-dependent and marked as such throughout this tour.",
    codeRefs: ["engine/src/gfx.rs:387", "engine/src/gfx.rs:431", "engine/src/flags.rs:37", "engine/src/flags.rs:86"],
    resources: [],
    dependsOn: ["startup"],
  },

  {
    id: "tlas-slot",
    label: "TLAS + instance buffer (per image)",
    domain: "memory",
    level: 1,
    group: "dGPU",
    x: 840, y: 90, w: 200, h: 60,
    detail:
      "Host-visible mapped instance buffer, 64-byte stride per " +
      "VkAccelerationStructureInstanceKHR. TLAS is device-local, created " +
      "ALLOW_UPDATE, built once at startup and updated per measured frame " +
      "when RT is supported.",
    codeRefs: ["engine/src/plane/frames.rs:303", "engine/src/plane/frames.rs:348", "engine/src/plane/mod.rs:38"],
    resources: ["rt-instances"],
    dependsOn: ["cap-probe"],
  },
  {
    id: "frame-ubo",
    label: "Per-image FRAME_BYTES buffer (mapped, mlocked)",
    domain: "memory",
    level: 1,
    group: "dGPU",
    x: 1060, y: 90, w: 220, h: 60,
    detail:
      "One host-visible buffer per swapchain image holds the 1,824-byte " +
      "uniform block plus terrain, vegetation and canopy indirect command " +
      "arrays. Permanently mapped and mlocked. Written every frame from " +
      "the CPU.",
    codeRefs: ["engine/src/plane/frames.rs:71", "engine/src/ubo.rs:29", "engine/src/plane/mod.rs:25"],
    resources: ["ubo", "indirect"],
    dependsOn: ["sim-thread"],
  },
  {
    id: "cmd-buffers",
    label: "Reusable command buffers",
    domain: "gpu",
    level: 1,
    group: "dGPU",
    x: 840, y: 170, w: 200, h: 60,
    detail:
      "One pool and command buffer per swapchain image, recorded once at " +
      "startup. The hot loop waits on a fence, copies uniforms, submits " +
      "and presents. Recording happens outside the steady-state frame.",
    codeRefs: ["engine/src/gfx.rs:1008", "engine/src/gfx.rs:1031", "engine/src/gfx.rs:945"],
    resources: ["cmd-buffers"],
    dependsOn: ["startup"],
  },
  {
    id: "sync-objects",
    label: "Fences and semaphores",
    domain: "sync",
    level: 1,
    group: "dGPU",
    x: 1060, y: 170, w: 220, h: 60,
    detail:
      "Per-image fence and frame_done semaphore. Acquire reuses a binary " +
      "semaphore only after waiting its recorded fence. Submit waits on " +
      "the acquire semaphore at COLOR_ATTACHMENT_OUTPUT, signals " +
      "frame_done, uses the frame fence. Present waits on frame_done. " +
      "Display pacing may block on present_id - 2.",
    codeRefs: ["engine/src/gfx.rs:1345", "engine/src/gfx.rs:1452", "engine/src/gfx.rs:1473", "engine/src/gfx.rs:1514"],
    resources: [],
    dependsOn: ["cmd-buffers", "present"],
  },
  {
    id: "pass-chain",
    label: "Recorded pass order",
    domain: "gpu",
    level: 1,
    group: "dGPU",
    x: 840, y: 240, w: 440, h: 45,
    detail:
      "TLAS update, opaque aircraft, terrain and structures, trees, " +
      "canopy, clouds, sky, plume, trails, glass, composite, HUD. " +
      "Capability-dependent mesh and ray-query branches are annotated in " +
      "chapter 4. GPU vegetation culling exists only behind a DEBUG_ONLY " +
      "flag; normal builds cull on the CPU.",
    codeRefs: ["engine/src/plane/frames.rs:832", "engine/src/plane/frames.rs:1115", "engine/src/plane/frames.rs:1156"],
    resources: ["hdr-target"],
    dependsOn: ["cmd-buffers", "frame-ubo"],
  },
  {
    id: "hdr-target",
    label: "HDR RGBA16F + depth D32F (device local)",
    domain: "gpu",
    level: 1,
    group: "dGPU",
    x: 840, y: 295, w: 440, h: 50,
    detail:
      "Device-local scene attachments. All opaque and transparent scene " +
      "passes write here before the barrier to shader-read and the " +
      "fullscreen composite into the swapchain.",
    codeRefs: ["engine/src/gfx.rs:203", "docs/rendering/hdr-fx-pipeline-architecture.md"],
    resources: [],
    dependsOn: [],
  },
  {
    id: "swapchain",
    label: "Swapchain images",
    domain: "display",
    level: 1,
    group: "present",
    x: 60, y: 570, w: 240, h: 70,
    detail:
      "Acquired with vkAcquireNextImageKHR. Timeouts skip the frame; " +
      "out-of-date rebuilds the swapchain. Completion is a fence wait; " +
      "presentation is vkQueuePresentKHR waiting on frame_done.",
    codeRefs: ["engine/src/gfx.rs:1355", "engine/src/gfx.rs:1374", "engine/src/gfx.rs:1486"],
    resources: ["swapchain"],
    dependsOn: ["pass-chain"],
  },
  {
    id: "compositor",
    label: "Wayland / Mutter compositor",
    domain: "display",
    level: 1,
    group: "present",
    x: 340, y: 570, w: 300, h: 70,
    detail:
      "Manages the Wayland surface and schedules presentation to the " +
      "display controller. Present submissions are not the same as frames " +
      "physically shown by the panel.",
    codeRefs: ["docs/hardware/rtx-4060-laptop-gpu.md"],
    resources: [],
    dependsOn: ["swapchain"],
  },
  {
    id: "panel",
    label: "2880x1800 120 Hz panel",
    domain: "display",
    level: 1,
    group: "present",
    x: 680, y: 570, w: 300, h: 70,
    detail:
      "Internal panel driven by the AMD display controller. At most 120 " +
      "distinct updates per second, independent of how many presents the " +
      "application submits or how often the simulation steps.",
    codeRefs: ["docs/hardware/rtx-4060-laptop-gpu.md"],
    resources: [],
    dependsOn: ["compositor", "igpu"],
  },

  // Resources shown at module level
  {
    id: "ubo",
    label: "Uniform block: 1,824 B",
    domain: "memory",
    level: 1,
    group: "sysram",
    x: 60, y: 395, w: 200, h: 55,
    detail:
      "viewProj 64 B + invViewProj 64 B + nodes[23] 1,472 B + 14 tail " +
      "vec4 224 B = 1,824 B. Asserted by a unit test in the engine.",
    codeRefs: ["engine/src/ubo.rs:29", "engine/src/ubo.rs:109"],
    resources: [],
    dependsOn: [],
  },
  {
    id: "indirect",
    label: "Indirect command arrays (in FRAME_BYTES)",
    domain: "memory",
    level: 1,
    group: "sysram",
    x: 280, y: 395, w: 200, h: 55,
    detail:
      "Terrain draws at 20 B each (DrawIndexedIndirectCommand), " +
      "vegetation and canopy draws at 16 B each (DrawIndirectCommand). " +
      "Filled on the CPU each frame; the GPU consumes them without a " +
      "readback.",
    codeRefs: ["engine/src/plane/mod.rs:16", "engine/src/plane/uniforms.rs:42"],
    resources: ["frame-ubo"],
    dependsOn: [],
  },
  {
    id: "staging",
    label: "Staging uploads (startup)",
    domain: "memory",
    level: 1,
    group: "sysram",
    x: 60, y: 470, w: 420, h: 0, // hidden helper, not drawn as box
    hidden: true,
    detail:
      "Host-visible staging buffer, copy, one-time submit, fence wait. " +
      "Used for geometry, mesh hierarchy, vegetation, textures, FX " +
      "volumes and RT inputs.",
    codeRefs: ["engine/src/plane/geometry.rs:177", "engine/src/plane/textures.rs:95", "engine/src/plane/rt.rs:107"],
    resources: [],
    dependsOn: [],
  },
  {
    id: "geo-vertex",
    label: "Persistent vertex + index buffers",
    domain: "memory",
    level: 1,
    group: "vram",
    x: 840, y: 395, w: 190, h: 55,
    detail:
      "Packed 28-byte airframe vertex stream. Index buffer packs airframe, " +
      "full terrain, performance terrain and cloud sections. Uploaded once " +
      "through staged one-time submits.",
    codeRefs: ["engine/src/plane/geometry.rs:100", "crates/airframe-format/src/lib.rs:28"],
    resources: [],
    dependsOn: [],
  },
  {
    id: "textures",
    label: "Textures, LUTs, FX volumes",
    domain: "memory",
    level: 1,
    group: "vram",
    x: 840, y: 460, w: 190, h: 55,
    detail:
      "Weave, atmosphere LUTs, terrain detail maps and FX noise volumes " +
      "uploaded the same staged way at startup.",
    codeRefs: ["engine/src/plane/textures.rs:32", "engine/src/plane/fx_volumes.rs:59"],
    resources: [],
    dependsOn: [],
  },
  {
    id: "input-atomics",
    label: "Shared input atomics",
    domain: "sync",
    level: 1,
    group: "sysram",
    x: 60, y: 465, w: 420, h: 50,
    detail:
      "Three atomics: exit, keys bitmask, ui bitmask. No locks, no " +
      "channels between the window thread and the render thread.",
    codeRefs: ["engine/src/app.rs:38", "engine/src/frame_loop.rs:120"],
    resources: [],
    dependsOn: [],
  },
  {
    id: "baked-airframe",
    label: "OUT_DIR/airframe.bin",
    domain: "build",
    level: 1,
    group: "build-time",
    x: 60, y: 730, w: 200, h: 50,
    detail:
      "Baked aircraft mesh and hierarchy. Runtime only does " +
      "include_bytes! and decodes it.",
    codeRefs: ["engine/build.rs:115", "engine/src/plane/airframe_mesh.rs:10"],
    resources: [],
    dependsOn: [],
  },
  {
    id: "spirv-blobs",
    label: "SPIR-V modules (OUT_DIR/*.spv)",
    domain: "build",
    level: 1,
    group: "build-time",
    x: 280, y: 730, w: 200, h: 50,
    detail:
      "GLSL compiled offline to Vulkan 1.3 / SPIR-V 1.4. Runtime loads " +
      "them with include_bytes!. tools/check_shaders.py validates them " +
      "with spirv-val.",
    codeRefs: ["engine/build.rs:59", "engine/src/plane/spirv.rs:52", "tools/check_shaders.py:8"],
    resources: [],
    dependsOn: [],
  },
  {
    id: "rt-instances",
    label: "RT instance buffer (64 B stride)",
    domain: "memory",
    level: 1,
    group: "dGPU",
    x: 1290, y: 90, w: 0, h: 0,
    hidden: true,
    detail:
      "VkAccelerationStructureInstanceKHR stride: transform 48 B + two " +
      "packed u32 + device reference 8 B + padding to 16-byte alignment " +
      "= 64 B.",
    codeRefs: ["engine/src/plane/mod.rs:36"],
    resources: [],
    dependsOn: [],
  },

  // ---- Kernel crates: in workspace, not linked into the native game ----
  {
    id: "kernel-engine",
    label: "kernels/engine-core (cdylib, not linked)",
    domain: "inactive",
    level: 1,
    x: 40, y: 880, w: 300, h: 70,
    detail:
      "Workspace member with crate-type cdylib targeting WebAssembly and " +
      "a JS frame loop. No native game crate depends on it. Listed in the " +
      "module inventory outside the active execution flow.",
    codeRefs: ["kernels/engine-core/Cargo.toml", "Cargo.toml:10"],
    resources: [],
    dependsOn: [],
    inactive: true,
  },
  {
    id: "kernel-terrain",
    label: "kernels/terrain-kernel (cdylib, not linked)",
    domain: "inactive",
    level: 1,
    x: 360, y: 880, w: 300, h: 70,
    detail:
      "Workspace member exporting extern \"C\" functions as a cdylib. No " +
      "native game crate depends on it. Module inventory only.",
    codeRefs: ["kernels/terrain-kernel/Cargo.toml", "Cargo.toml:11"],
    resources: [],
    dependsOn: [],
    inactive: true,
  },

  // ---- Core / SM internals (level 2) ----
  // Drawn in the free lane between the host and GPU boxes so they never
  // cover the module-level thread and resource boxes.
  {
    id: "cpu-core",
    label: "One Zen 4 core (x8 schematic)",
    domain: "cpu",
    level: 2,
    group: "host",
    x: 530, y: 50, w: 270, h: 170,
    detail:
      "Logical schematic of one core: two SMT contexts, 32 KiB " +
      "instruction cache, 32 KiB data cache, 1 MiB L2. Cores share the " +
      "16 MiB L3. Scheduling places threads dynamically; no thread is " +
      "bound to one core for correctness. The render thread pins itself " +
      "for latency reasons only.",
    codeRefs: ["engine/src/vendor.rs:163"],
    resources: ["cpu-l3"],
    dependsOn: [],
  },
  {
    id: "smt",
    label: "2 SMT contexts",
    domain: "cpu",
    level: 2,
    group: "cpu-core",
    x: 545, y: 85, w: 115, h: 40,
    detail: "Two hardware contexts per core; 16 contexts total.",
    codeRefs: ["lscpu (local probe)"],
    resources: [],
    dependsOn: [],
  },
  {
    id: "l1i",
    label: "32 KiB L1i",
    domain: "cpu",
    level: 2,
    group: "cpu-core",
    x: 670, y: 85, w: 115, h: 40,
    detail: "Per-core instruction cache.",
    codeRefs: ["lscpu (local probe)"],
    resources: [],
    dependsOn: [],
  },
  {
    id: "l1d",
    label: "32 KiB L1d",
    domain: "cpu",
    level: 2,
    group: "cpu-core",
    x: 545, y: 135, w: 115, h: 40,
    detail: "Per-core data cache.",
    codeRefs: ["lscpu (local probe)"],
    resources: [],
    dependsOn: [],
  },
  {
    id: "cpu-l2",
    label: "1 MiB L2",
    domain: "cpu",
    level: 2,
    group: "cpu-core",
    x: 670, y: 135, w: 115, h: 40,
    detail: "Per-core L2. Eight cores make the 8 MiB total local probe.",
    codeRefs: ["lscpu (local probe)"],
    resources: [],
    dependsOn: [],
  },
  {
    id: "cpu-l3",
    label: "Shared 16 MiB L3",
    domain: "cpu",
    level: 2,
    x: 530, y: 240, w: 270, h: 50,
    detail: "One shared L3 instance across all cores.",
    codeRefs: ["lscpu (local probe)", "amd.com 7840HS specifications"],
    resources: [],
    dependsOn: [],
  },
  {
    id: "sm",
    label: "One SM (24 on this GPU; schematic)",
    domain: "gpu",
    level: 2,
    x: 1320, y: 50, w: 260, h: 270,
    detail:
      "Logical schematic of one Ada SM. Shaders are dispatched across " +
      "SMs by the GPU; no pass owns fixed SMs. 32-thread warps schedule " +
      "on four partitions. Resource limits (registers, shared memory, " +
      "dependencies) cap occupancy below the 48-warp ceiling; actual " +
      "occupancy is not measured in this snapshot.",
    codeRefs: ["docs.nvidia.com/cuda/ada-tuning-guide", "images.nvidia.com Ada whitepaper"],
    resources: [],
    dependsOn: [],
  },
  {
    id: "sched",
    label: "4 scheduler partitions",
    domain: "gpu",
    level: 2,
    group: "sm",
    x: 1335, y: 85, w: 115, h: 40,
    detail: "Each partition has a warp scheduler, dispatch unit, register slice and arithmetic lanes.",
    codeRefs: ["Ada whitepaper"],
    resources: [],
    dependsOn: [],
  },
  {
    id: "warps",
    label: "32-thread warps (up to 48 resident)",
    domain: "gpu",
    level: 2,
    group: "sm",
    x: 1460, y: 85, w: 105, h: 40,
    detail:
      "Scheduling unit. Ada tuning guide: up to 48 resident warps per SM " +
      "before occupancy limits apply.",
    codeRefs: ["docs.nvidia.com/cuda/ada-tuning-guide"],
    resources: [],
    dependsOn: [],
  },
  {
    id: "regs",
    label: "256 KiB register file",
    domain: "gpu",
    level: 2,
    group: "sm",
    x: 1335, y: 135, w: 115, h: 40,
    detail: "64K-entry 32-bit register file per SM.",
    codeRefs: ["docs.nvidia.com/cuda/ada-tuning-guide"],
    resources: [],
    dependsOn: [],
  },
  {
    id: "alu",
    label: "Arithmetic + texture units",
    domain: "gpu",
    level: 2,
    group: "sm",
    x: 1460, y: 135, w: 105, h: 40,
    detail:
      "128 CUDA cores and four texture units per SM in the whitepaper. " +
      "Fragment and vertex work for this game runs here.",
    codeRefs: ["Ada whitepaper"],
    resources: [],
    dependsOn: [],
  },
  {
    id: "l1shared",
    label: "128 KiB L1 / shared complex",
    domain: "gpu",
    level: 2,
    group: "sm",
    x: 1335, y: 185, w: 230, h: 40,
    detail:
      "Unified L1 data cache and shared memory. Shared carveout up to " +
      "100 KiB per SM, at most 99 KiB to one block.",
    codeRefs: ["Ada whitepaper", "Ada tuning guide"],
    resources: [],
    dependsOn: [],
  },
  {
    id: "rt-core",
    label: "1 RT core (ray-query shadows when capable)",
    domain: "gpu",
    level: 2,
    group: "sm",
    x: 1335, y: 235, w: 115, h: 55,
    detail:
      "Third-generation RT core. The game uses ray queries for shadow " +
      "rays through the TLAS when the device reports support and the " +
      "DEBUG_ONLY toggle allows it. Capability-dependent path.",
    codeRefs: ["engine/src/gfx.rs:387", "engine/src/plane/frames.rs:886"],
    resources: [],
    dependsOn: [],
  },
  {
    id: "tensor",
    label: "4 Tensor cores (unused by this game)",
    domain: "gpu",
    level: 2,
    group: "sm",
    x: 1460, y: 235, w: 105, h: 55,
    detail:
      "Fourth-generation Tensor cores exist in the hardware. The current " +
      "renderer does not dispatch work to them. No DLSS, no CUDA, no " +
      "matrix pipelines.",
    codeRefs: ["docs/hardware/rtx-4060-laptop-gpu.md", "engine/src/gfx.rs"],
    resources: [],
    dependsOn: [],
    unused: true,
  },
];

// Edges: kind is control | data | sync.
// control inherits the source node domain color; data is amber; sync is violet.
export const EDGES = [
  { id: "e-build-startup", from: "build-time", to: "startup", kind: "control", level: 1,
    label: "artifacts embedded at compile time" },
  { id: "e-win-sim", from: "win-thread", to: "sim-thread", kind: "sync", level: 1,
    label: "input atomics" },
  { id: "e-sim-audio", from: "sim-thread", to: "audio-thread", kind: "data", level: 1,
    label: "relaxed atomic parameters" },
  { id: "e-startup-probe", from: "startup", to: "cap-probe", kind: "control", level: 1,
    label: "feature queries" },
  { id: "e-sim-ubo", from: "sim-thread", to: "frame-ubo", kind: "data", level: 1,
    label: "mapped uniform + indirect writes" },
  { id: "e-ubo-pcie", from: "frame-ubo", to: "pcie", kind: "data", level: 0,
    label: "host-visible writes reach device through the link" },
  { id: "e-pcie-gpu", from: "pcie", to: "dGPU", kind: "data", level: 0,
    label: "PCIe 4.0 x8" },
  { id: "e-geo-gpu", from: "sysram", to: "pcie", kind: "data", level: 0,
    label: "startup staged uploads" },
  { id: "e-cmdbuf-pass", from: "cmd-buffers", to: "pass-chain", kind: "control", level: 1,
    label: "reused recorded buffers" },
  { id: "e-probe-tlas", from: "cap-probe", to: "tlas-slot", kind: "control", level: 1,
    label: "gated on RT support" },
  { id: "e-tlas-pass", from: "tlas-slot", to: "pass-chain", kind: "data", level: 1,
    label: "TLAS update first in measured frame" },
  { id: "e-pass-sync", from: "pass-chain", to: "sync-objects", kind: "sync", level: 1,
    label: "submit signals frame_done, uses fence" },
  { id: "e-sync-swap", from: "sync-objects", to: "swapchain", kind: "sync", level: 1,
    label: "acquire semaphore then present wait" },
  { id: "e-pass-swap", from: "pass-chain", to: "swapchain", kind: "data", level: 1,
    label: "composite writes swapchain image" },
  { id: "e-swap-comp", from: "swapchain", to: "compositor", kind: "data", level: 1,
    label: "vkQueuePresentKHR" },
  { id: "e-comp-panel", from: "compositor", to: "panel", kind: "control", level: 1,
    label: "compositor schedules scanout" },
  { id: "e-igpu-panel", from: "igpu", to: "panel", kind: "control", level: 0,
    label: "display controller drives panel" },
  { id: "e-gpu-present", from: "dGPU", to: "present", kind: "control", level: 0,
    label: "render device" },
  { id: "e-present-igpu", from: "present", to: "igpu", kind: "control", level: 0,
    label: "presentation topology" },
  { id: "e-hdr-composite", from: "hdr-target", to: "pass-chain", kind: "data", level: 1,
    label: "scene target read after barrier" },
  { id: "e-vram-sm", from: "vram", to: "dGPU", kind: "data", level: 0,
    label: "device-local capacity" },
  { id: "e-l2-sm", from: "gpu-l2", to: "dGPU", kind: "data", level: 0,
    label: "shared L2" },
];

export function nodesAtLevel(level) {
  return NODES.filter((n) => !n.hidden && n.level <= level);
}

export function edgesAtLevel(level) {
  const visible = new Set(nodesAtLevel(level).map((n) => n.id));
  return EDGES.filter((e) => e.level <= level && visible.has(e.from) && visible.has(e.to));
}

export function nodeById(id) {
  return NODES.find((n) => n.id === id) ?? null;
}

export function validateGraph() {
  const errors = [];
  const ids = new Set();
  for (const n of NODES) {
    if (ids.has(n.id)) errors.push(`duplicate node id: ${n.id}`);
    ids.add(n.id);
    if (typeof n.level !== "number" || n.level < 0 || n.level > 2) {
      errors.push(`node ${n.id}: bad level ${n.level}`);
    }
    if (!(n.w >= 0) || !(n.h >= 0)) {
      errors.push(`node ${n.id}: negative size`);
    }
  }
  for (const e of EDGES) {
    if (!ids.has(e.from)) errors.push(`edge ${e.id}: missing from=${e.from}`);
    if (!ids.has(e.to)) errors.push(`edge ${e.id}: missing to=${e.to}`);
    if (!["control", "data", "sync"].includes(e.kind)) {
      errors.push(`edge ${e.id}: bad kind ${e.kind}`);
    }
    if (typeof e.level !== "number" || e.level < 0 || e.level > 2) {
      errors.push(`edge ${e.id}: bad level ${e.level}`);
    }
  }
  for (const n of NODES) {
    for (const dep of n.dependsOn ?? []) {
      if (!ids.has(dep)) errors.push(`node ${n.id}: missing dependsOn=${dep}`);
    }
    for (const parent of [n.group]) {
      if (parent && !ids.has(parent)) {
        errors.push(`node ${n.id}: missing group=${parent}`);
      }
    }
  }
  return errors;
}
