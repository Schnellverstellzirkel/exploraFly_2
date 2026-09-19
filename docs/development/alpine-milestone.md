# Alpine exploration milestone — 2026-09-19

## Intent

Develop a playable foundation for fantasy alpine free flight while retaining a
small explicit Vulkan engine. Priorities for this iteration are visible terrain
and water, expedition-aircraft materials/effects, a flight HUD, flight-driven
procedural sound, and trustworthy performance reporting.

## Parallel ownership

- `codex/alpine-terrain`: terrain model, vertex-generated landscape, materials,
  lakes, and CPU height sampling; the integrator connects drawing and clearance.
- `codex/frame-throughput`: device capability checks, queue/submission costs,
  frame-time accounting and machine-readable benchmark reporting.
- `codex/aircraft-fx`: procedural aircraft detail, palette/material response,
  exhaust and vapor shading within the current packed geometry interface.
- Integration checkout: UI, audio, controls, shared uniform changes, review,
  cross-subsystem tests, documentation, commits, and pushes.

## Acceptance and limits

Every contribution must compile and pass appropriate CPU/shader checks. The
current machine is Windows and has no connection to the target Linux laptop;
the 1 ms GPU target and actual rendered appearance cannot be certified here.
Terrain/settlement content and audio are foundational implementations, not a
complete open-world game. Architectural choices and primary-source research
belong in each subsystem's authored notes.

The HUD should reuse the existing uniform buffer and final composite rather
than add a general UI renderer to the frame. Audio generation belongs on a
separate thread with a fixed buffer, native PCM output, and smoothed controls.
Both can be disabled for clear A/B performance comparisons.
