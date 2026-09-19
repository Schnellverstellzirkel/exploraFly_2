# Project direction and development contract

Build a free-flight game in a heightened alpine fantasy world: readable valleys,
meadows, mountain silhouettes, lakes, and eventual medieval settlements. Use the
user's named games as broad visual references, not as sources of copied assets.
The aircraft should feel like an expedition craft that belongs in this world.

## Evidence and performance

- Research rendering changes against current primary publications, author
  project pages, Khronos specifications, and NVIDIA/AMD technical documentation.
  Record URLs, publication/update dates, access dates, applicability, and limits
  in the relevant authored documentation. Distinguish recent papers from older
  foundational methods; do not call an implementation state of the art merely
  because a recent source is cited.
- Keep the custom Rust/ash Vulkan renderer. Prefer explicit GPU resource lifetime,
  bounded work, reused command buffers, small updates, and capability-checked
  native paths. Vendor-specific APIs are tools, not guaranteed speedups.
- The performance target is 1,000 complete presentation submissions per second
  on the target RTX 4060 Laptop GPU: a 1 ms budget. Report wall frame times,
  percentiles, resolution, preset, scene/workload, GPU/driver, and GPU timings.
  Distinguish submitted presents from frames physically displayed by a monitor.
- Never claim the target is achieved based on CPU tests, skipped GPU work,
  repeated geometry passes, estimates, or measurements from another GPU.
- Windows/Docker builds validate CPU behavior and offline Vulkan shaders. Target
  GPU visual and performance acceptance remain separate requirements.

## Parallel work and integration

- Use isolated worktrees and explicit subsystem ownership for independent agents.
  Keep shared shader uniform layouts, entry-point integration, and manifests
  coordinated. Send integration requirements early.
- Commit coherent, tested changes frequently with descriptive messages. Push
  integrated milestones when authorized and access permits; do not force-push.
- Preserve unrelated user edits. The large `docs/` reference trees are vendored;
  edit authored notes instead of rewriting reference sources.
- Run `cargo test --workspace --locked` and validate compiled SPIR-V using
  `tools/check_shaders.py`. See `tools/README.md` for the Linux check container.
