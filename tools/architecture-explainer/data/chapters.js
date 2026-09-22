// Chapter scripts for the guided tour.
// Each chapter owns an ordered list of steps. Steps reference graph node
// and edge ids; validateChapters() checks them. Animation timing is
// explanatory: step weights order the tour and nothing more. No step
// claims a measured duration.

export const CHAPTERS = [
  {
    id: "ch1",
    number: 1,
    title: "Before launch",
    summary:
      "Aircraft generation and GLSL compilation happen during the build. " +
      "Startup creates pipelines, prepares world resources, uploads " +
      "persistent data and records reusable command buffers.",
    detailHint: 0,
    steps: [
      {
        id: "ch1-s1",
        title: "Build: bake the aircraft",
        body:
          "cargo build runs airframe_baker and writes OUT_DIR/airframe.bin. " +
          "Geometry generation, LOD chain, meshlet partitioning and RT proxy " +
          "construction all happen here. The running game never generates " +
          "the airframe; it only decodes the baked blob with include_bytes!.",
        focus: ["build-time", "baked-airframe"],
        edges: [],
        weight: 1,
        kind: "control",
      },
      {
        id: "ch1-s2",
        title: "Build: compile GLSL to SPIR-V",
        body:
          "The build script compiles every shader with shaderc on scoped " +
          "worker threads, targeting Vulkan 1.3 and SPIR-V 1.4, with a " +
          "content-hash cache. Landmark constants are generated into " +
          "world_generated.inc. tools/check_shaders.py later validates the " +
          "modules with spirv-val.",
        focus: ["build-time", "spirv-blobs"],
        edges: [{ edge: "e-build-startup", animate: true }],
        weight: 1,
        kind: "control",
      },
      {
        id: "ch1-s3",
        title: "Startup: device, swapchain, pipelines",
        body:
          "On the render thread, Gfx::new creates the instance and device, " +
          "probes capabilities, and creates the swapchain. Plane::build " +
          "creates the scene and FX pipelines from the included SPIR-V " +
          "blobs. Capability probes decide the ray-query and mesh-shader " +
          "paths; both are capability-dependent.",
        focus: ["startup", "cap-probe"],
        edges: [{ edge: "e-startup-probe", animate: true }],
        weight: 1,
        kind: "control",
      },
      {
        id: "ch1-s4",
        title: "Startup: world resources and persistent uploads",
        body:
          "Vegetation database, geometry, textures, atmosphere LUTs and FX " +
          "noise volumes move into device-local memory through staged " +
          "one-time submits: host-visible staging buffer, copy, submit, " +
          "fence wait. BLAS inputs for animated nodes and terrain casters " +
          "are built once.",
        focus: ["startup", "geo-vertex", "textures"],
        edges: [{ edge: "e-geo-gpu", animate: true }, { edge: "e-pcie-gpu", animate: true }],
        weight: 1,
        kind: "data",
      },
      {
        id: "ch1-s5",
        title: "Startup: record command buffers once",
        body:
          "One command pool and command buffer per swapchain image are " +
          "recorded once, covering the full pass order. The steady-state " +
          "loop only waits on a fence, copies uniforms, submits and " +
          "presents. Recording is not repeated every frame.",
        focus: ["cmd-buffers", "pass-chain"],
        edges: [{ edge: "e-cmdbuf-pass", animate: true }],
        weight: 1,
        kind: "control",
      },
      {
        id: "ch1-s6",
        title: "Kernel crates sit outside this flow",
        body:
          "kernels/engine-core and kernels/terrain-kernel are workspace " +
          "members with crate-type cdylib. No native game crate depends " +
          "on them. They appear in the module inventory but never run in " +
          "the game process.",
        focus: ["kernel-engine", "kernel-terrain"],
        edges: [],
        weight: 0.75,
        kind: "control",
      },
    ],
  },
  {
    id: "ch2",
    number: 2,
    title: "CPU execution",
    summary:
      "The window thread publishes input. The simulation/render thread " +
      "steps flight at a fixed 144 Hz, interpolates the pose, updates " +
      "camera and HUD data, and prepares rendering. Audio runs on its own " +
      "thread.",
    detailHint: 1,
    steps: [
      {
        id: "ch2-s1",
        title: "Window thread publishes input",
        body:
          "Keyboard events set bits in a shared keys atomic; HUD toggles " +
          "xor a ui atomic; focus loss clears keys. Three atomics total, " +
          "no locks and no channels. The event loop sleeps between events " +
          "and the render thread never waits on it.",
        focus: ["win-thread", "input-atomics"],
        edges: [{ edge: "e-win-sim", animate: true }],
        weight: 1,
        kind: "sync",
      },
      {
        id: "ch2-s2",
        title: "Fixed 144 Hz simulation steps",
        body:
          "SIM_STEP is 1/144 s. A wall-clock delta, clamped to 100 ms, " +
          "feeds an accumulator. Each frame runs zero or more full steps: " +
          "pose.step_with_wind, airframe animation, and FX updates at the " +
          "same fixed rate. The clamp bounds one frame to at most 15 steps.",
        focus: ["sim-thread"],
        edges: [],
        weight: 1.25,
        kind: "control",
      },
      {
        id: "ch2-s3",
        title: "Interpolate the render pose",
        body:
          "alpha = accumulator / SIM_STEP. The render pose is " +
          "prev_pose.interpolate(pose, alpha). The simulation runs on a " +
          "fixed step; the rendered pose is continuous between steps.",
        focus: ["sim-thread"],
        edges: [],
        weight: 1,
        kind: "control",
      },
      {
        id: "ch2-s4",
        title: "Camera, HUD, uniforms",
        body:
          "The chase camera steps with a terrain clearance floor for the " +
          "eye. HUD values are packed into vec4s. The uniform block and " +
          "the CPU-filled indirect command lists are written into the " +
          "mapped FRAME_BYTES buffer for this swapchain image.",
        focus: ["sim-thread", "frame-ubo", "ubo", "indirect"],
        edges: [{ edge: "e-sim-ubo", animate: true }],
        weight: 1,
        kind: "data",
      },
      {
        id: "ch2-s5",
        title: "Audio thread synthesizes independently",
        body:
          "The render thread publishes 13 floats plus volume as relaxed " +
          "atomics. The explora-audio thread reads them and renders fixed " +
          "480-frame stereo blocks through a lock-free DSP graph into " +
          "ALSA. Audio does not sit on the frame critical path.",
        focus: ["audio-thread", "sim-thread"],
        edges: [{ edge: "e-sim-audio", animate: true }],
        weight: 1,
        kind: "data",
      },
      {
        id: "ch2-s6",
        title: "Threads are not pinned to roles by hardware",
        body:
          "The diagram draws threads inside one CPU box. The render thread " +
          "pins its own cores and raises scheduling priority for latency. " +
          "Nothing assigns the simulation permanently to specific cores " +
          "for correctness, and no module owns fixed cores.",
        focus: ["host", "sim-thread", "cpu-core"],
        edges: [],
        weight: 0.75,
        kind: "control",
      },
    ],
  },
  {
    id: "ch3",
    number: 3,
    title: "Memory and submission",
    summary:
      "Persistent assets differ from per-frame uniforms, indirect " +
      "commands, FX buffers and acceleration-structure instances. Mapped " +
      "writes, startup transfers, buffer reuse, fences and semaphores " +
      "move data and enforce order.",
    detailHint: 1,
    steps: [
      {
        id: "ch3-s1",
        title: "Persistent versus per-frame",
        body:
          "Persistent: vertex and index buffers, mesh hierarchy, " +
          "vegetation and canopy records, textures, LUTs, FX volumes, " +
          "BLAS builds. Per-frame: the 1,824-byte uniform block, terrain " +
          "/ vegetation / canopy indirect arrays, FX vertex rings, TLAS " +
          "instances and the TLAS update itself.",
        focus: ["geo-vertex", "textures", "frame-ubo", "tlas-slot"],
        edges: [],
        weight: 1.25,
        kind: "data",
      },
      {
        id: "ch3-s2",
        title: "FRAME_BYTES layout",
        body:
          "One host-visible buffer per image: uniform block, then terrain " +
          "draws at 20 bytes each, vegetation draws at 16 bytes each, " +
          "canopy draws at 16 bytes each. The descriptor range exposed to " +
          "shaders is the uniform block only; the tail is indirect " +
          "commands consumed without a descriptor.",
        focus: ["frame-ubo", "ubo", "indirect"],
        edges: [{ edge: "e-sim-ubo", animate: true }],
        weight: 1.25,
        kind: "data",
      },
      {
        id: "ch3-s3",
        title: "Host-visible is an access property",
        body:
          "Vulkan memory types advertise properties such as host-visible, " +
          "host-coherent and device-local. Host-visible means the memory " +
          "can be mapped for host access. It does not by itself mean " +
          "ordinary system RAM, and device-local does not forbid host " +
          "visibility on some implementations. The engine picks types " +
          "with the flags it needs.",
        focus: ["sysram", "frame-ubo", "vram"],
        edges: [{ edge: "e-ubo-pcie", animate: true }],
        weight: 1,
        kind: "data",
      },
      {
        id: "ch3-s4",
        title: "Startup transfers use staging",
        body:
          "Device-local payloads are written through a host-visible " +
          "staging buffer: record a copy, submit once, wait on a fence. " +
          "This pattern covers geometry, meshlets, vegetation, textures, " +
          "FX volumes and RT inputs. After startup these buffers are not " +
          "rewritten.",
        focus: ["startup", "geo-vertex", "vram"],
        edges: [{ edge: "e-geo-gpu", animate: true }, { edge: "e-pcie-gpu", animate: true }],
        weight: 1,
        kind: "data",
      },
      {
        id: "ch3-s5",
        title: "Buffer reuse and mapped writes",
        body:
          "The FRAME_BYTES buffer, TLAS instance buffer, cone and trail " +
          "vertex rings are permanently mapped. The frame loop writes " +
          "into the slot for the acquired image after waiting that " +
          "image's fence. Old contents are overwritten in place; the " +
          "buffers are reused every frame.",
        focus: ["frame-ubo", "tlas-slot", "sync-objects"],
        edges: [{ edge: "e-sim-ubo", animate: true }],
        weight: 1,
        kind: "data",
      },
      {
        id: "ch3-s6",
        title: "Fences and semaphores order the frame",
        body:
          "Before reusing an acquire semaphore the CPU waits the fence " +
          "recorded for that slot. After acquiring, the CPU waits the " +
          "image's frame fence, resets it, then writes uniforms. Submit " +
          "waits the acquire semaphore at COLOR_ATTACHMENT_OUTPUT, " +
          "signals frame_done, and attaches the frame fence. Present " +
          "waits on frame_done.",
        focus: ["sync-objects", "cmd-buffers", "swapchain"],
        edges: [{ edge: "e-pass-sync", animate: true }, { edge: "e-sync-swap", animate: true }],
        weight: 1.25,
        kind: "sync",
      },
      {
        id: "ch3-s7",
        title: "TLAS update when ray queries are available",
        body:
          "The measured frame updates the TLAS with a host barrier, an " +
          "UPDATE build, and a ready barrier, but only when the device " +
          "supports the ray-query path. Instance data is a 64-byte " +
          "structured write per instance. Without RT support this step " +
          "is skipped.",
        focus: ["tlas-slot", "cap-probe", "pass-chain"],
        edges: [{ edge: "e-probe-tlas", animate: true }, { edge: "e-tlas-pass", animate: true }],
        weight: 1,
        kind: "sync",
      },
    ],
  },
  {
    id: "ch4",
    number: 4,
    title: "GPU rendering",
    summary:
      "Follow the recorded pass order. Shaders run on SMs, rasterization " +
      "and depth processing happen in fixed-function stages, textures are " +
      "read through the cache hierarchy, and shadow rays traverse the " +
      "acceleration structure on RT hardware when available.",
    detailHint: 1,
    steps: [
      {
        id: "ch4-s1",
        title: "Pass order overview",
        body:
          "TLAS update, opaque aircraft, terrain and structures, trees, " +
          "far canopy, clouds, sky, plume, trails, glass, composite, HUD. " +
          "Each pass writes the HDR RGBA16F target and depth D32F until " +
          "the barrier that makes the scene readable for the composite.",
        focus: ["pass-chain", "hdr-target"],
        edges: [{ edge: "e-hdr-composite", animate: true }],
        weight: 1.25,
        kind: "control",
      },
      {
        id: "ch4-s2",
        title: "Opaque aircraft: mesh or indexed path",
        body:
          "When mesh shaders are available and the toggle allows it, the " +
          "airframe draws through the mesh-shader hierarchy. Otherwise " +
          "the legacy indexed path runs. Both paths are capability-" +
          "dependent; release builds choose by capability alone. One " +
          "merged draw covers the opaque parts, glass is a second draw.",
        focus: ["pass-chain", "cap-probe", "sm"],
        edges: [],
        weight: 1,
        kind: "control",
      },
      {
        id: "ch4-s3",
        title: "Terrain, structures, trees, canopy",
        body:
          "Terrain uses indirect draws filled on the CPU. Landmarks draw " +
          "as ground structures. Trees: normal builds cull on the CPU " +
          "and emit draw commands; a GPU compute cull path exists only " +
          "behind the DEBUG_ONLY EXPLORA_GPU_VEGETATION flag for A/B " +
          "comparison. Far canopy is another CPU-filled indirect list.",
        focus: ["pass-chain", "indirect"],
        edges: [],
        weight: 1.25,
        kind: "control",
      },
      {
        id: "ch4-s4",
        title: "Clouds, sky, plume, trails, glass",
        body:
          "Clouds and the sky quad fill the background. The plume " +
          "raymarches inside a proxy cone. Trails ribbon the wake. Glass " +
          "blends last among scene passes. These are fragment-heavy " +
          "passes on the SMs with texture reads through L1 and L2.",
        focus: ["pass-chain", "sm", "l1shared"],
        edges: [],
        weight: 1.25,
        kind: "control",
      },
      {
        id: "ch4-s5",
        title: "Shader execution on SMs",
        body:
          "Work is dispatched as 32-thread warps onto the four scheduler " +
        "partitions of each SM. Registers, shared memory and dependency " +
        "stalls limit how many warps stay resident; occupancy below the " +
        "48-warp ceiling is expected but not measured in this snapshot. " +
        "No pass owns fixed SMs.",
        focus: ["sm", "sched", "warps", "regs", "alu"],
        edges: [],
        weight: 1,
        kind: "control",
      },
      {
        id: "ch4-s6",
        title: "Texture reads and the cache hierarchy",
        body:
          "Texture units sample through the 128 KiB L1/shared complex, " +
          "then the shared 32 MiB L2, then GDDR6. Capacity figures come " +
          "from specifications and live queries; this page reports no " +
          "cache-hit measurements for the workload.",
        focus: ["l1shared", "gpu-l2", "vram", "alu"],
        edges: [],
        weight: 1,
        kind: "data",
      },
      {
        id: "ch4-s7",
        title: "Shadow rays on RT hardware (capability-dependent)",
        body:
          "When the device reports ray-query support, fragment shaders " +
          "cast shadow rays against the TLAS and the RT core accelerates " +
          "traversal. When support is missing the alternate path runs. " +
          "Tensor cores stay unused: no matrix work is dispatched to them.",
        focus: ["rt-core", "tensor", "tlas-slot", "cap-probe"],
        edges: [{ edge: "e-tlas-pass", animate: true }],
        weight: 1.25,
        kind: "control",
      },
      {
        id: "ch4-s8",
        title: "Composite and HUD",
        body:
          "A barrier moves the HDR attachment to shader-read state. The " +
          "fullscreen composite tonemaps into the swapchain image. A " +
          "second pass draws the HUD on top at full rate, then the image " +
          "transitions to PRESENT_SRC_KHR.",
        focus: ["pass-chain", "swapchain", "hdr-target"],
        edges: [{ edge: "e-pass-swap", animate: true }],
        weight: 1,
        kind: "control",
      },
    ],
  },
  {
    id: "ch5",
    number: 5,
    title: "Presentation and repetition",
    summary:
      "Acquire, complete, present, repeat. Keep the 144 Hz simulation, " +
      "presentation submissions, panel refresh and the 1 ms performance " +
      "target distinct.",
    detailHint: 0,
    steps: [
      {
        id: "ch5-s1",
        title: "Acquire the next swapchain image",
        body:
          "vkAcquireNextImageKHR returns an image index and signals an " +
          "acquire semaphore. A timeout skips the frame. Out-of-date " +
          "rebuilds the swapchain. The CPU then waits that image's frame " +
          "fence before touching its buffers.",
        focus: ["swapchain", "sync-objects"],
        edges: [{ edge: "e-sync-swap", animate: true }],
        weight: 1,
        kind: "sync",
      },
      {
        id: "ch5-s2",
        title: "Submit, complete, present",
        body:
          "Submit waits the acquire semaphore, signals frame_done, uses " +
          "the frame fence. Present waits on frame_done and hands the " +
          "image to the queue. Completion of GPU work and presentation " +
          "of the image are separate events.",
        focus: ["sync-objects", "swapchain", "pass-chain"],
        edges: [{ edge: "e-pass-sync", animate: true }, { edge: "e-pass-swap", animate: true }],
        weight: 1,
        kind: "sync",
      },
      {
        id: "ch5-s3",
        title: "Wayland presentation and the display controller",
        body:
          "The Wayland compositor receives the presented surface and " +
          "schedules it. The AMD Radeon 780M display controller drives " +
          "the 2880x1800 panel. The iGPU scanout path uses shared system " +
          "memory. Whether frames are copied or scanned out directly is " +
          "left unspecified here.",
        focus: ["present", "compositor", "igpu", "panel", "swapchain"],
        edges: [{ edge: "e-swap-comp", animate: true }, { edge: "e-comp-panel", animate: true }],
        weight: 1.25,
        kind: "data",
      },
      {
        id: "ch5-s4",
        title: "Four different clocks",
        body:
          "Simulation steps at a fixed 144 Hz. Presentation submissions " +
          "are whatever the loop completes; the benchmark counts " +
          "successful vkQueuePresentKHR calls, not frames shown. The " +
          "panel refreshes at most 120 times per second. The project " +
          "performance target is 1,000 complete presentation submissions " +
          "per second, a 1 ms budget on the RTX 4060 Laptop GPU, " +
          "reported separately from this page.",
        focus: ["sim-thread", "swapchain", "panel"],
        edges: [],
        weight: 1.25,
        kind: "control",
      },
      {
        id: "ch5-s5",
        title: "Display pacing and the loop repeats",
        body:
          "When display pacing is on, the loop blocks on present_id - 2 " +
          "before starting the next frame. The accumulator advances, " +
          "simulation steps run again, uniforms are rewritten into the " +
          "reused buffers, and the same recorded command buffers are " +
          "submitted for the next image.",
        focus: ["sync-objects", "sim-thread", "cmd-buffers", "frame-ubo"],
        edges: [{ edge: "e-sim-ubo", animate: true }, { edge: "e-cmdbuf-pass", animate: true }],
        weight: 1.25,
        kind: "control",
      },
      {
        id: "ch5-s6",
        title: "What this page does not measure",
        body:
          "No live monitoring, no utilization percentages, no cache-hit " +
          "rates, no invented execution durations. Animation on this page " +
          "uses explanatory time to show order and dependencies, not " +
          "hardware scheduling claims. Performance numbers belong to " +
          "controlled measurements on the target machine.",
        focus: [],
        edges: [],
        weight: 0.75,
        kind: "control",
      },
    ],
  },
];

export function allSteps() {
  const out = [];
  for (const ch of CHAPTERS) {
    for (let i = 0; i < ch.steps.length; i++) {
      out.push({ chapterId: ch.id, chapterNumber: ch.number, stepIndexInChapter: i, ...ch.steps[i] });
    }
  }
  return out;
}

export function chapterIndexById(id) {
  return CHAPTERS.findIndex((c) => c.id === id);
}

export function validateChapters(nodeIds, edgeIds) {
  const errors = [];
  const seen = new Set();
  let prevChapter = null;
  for (const ch of CHAPTERS) {
    if (!ch.steps.length) errors.push(`chapter ${ch.id} has no steps`);
    for (const s of ch.steps) {
      if (seen.has(s.id)) errors.push(`duplicate step id: ${s.id}`);
      seen.add(s.id);
      for (const f of s.focus ?? []) {
        if (!nodeIds.has(f)) errors.push(`step ${s.id}: unknown focus node ${f}`);
      }
      for (const e of s.edges ?? []) {
        if (!edgeIds.has(e.edge)) errors.push(`step ${s.id}: unknown edge ${e.edge}`);
      }
      if (typeof s.weight !== "number" || !(s.weight > 0)) {
        errors.push(`step ${s.id}: weight must be > 0`);
      }
      if (!["control", "data", "sync"].includes(s.kind)) {
        errors.push(`step ${s.id}: bad kind ${s.kind}`);
      }
    }
    if (prevChapter && CHAPTERS.indexOf(ch) !== prevChapter + 1) {
      errors.push(`chapter order broken at ${ch.id}`);
    }
    prevChapter = CHAPTERS.indexOf(ch);
  }
  if (CHAPTERS.length !== 5) errors.push(`expected 5 chapters, got ${CHAPTERS.length}`);
  const numbers = CHAPTERS.map((c) => c.number);
  const expected = [1, 2, 3, 4, 5];
  if (numbers.some((n, i) => n !== expected[i])) {
    errors.push(`chapter numbers must be 1..5, got ${numbers.join(",")}`);
  }
  return errors;
}
