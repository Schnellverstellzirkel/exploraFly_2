# State of the Art: Real-Time Reflections & Light–Surface Interaction

Research survey assembled 2026-09 for exploraFly_2, covering the strongest
published techniques, production SDKs, and open-source references for
physically plausible reflections in real-time rendering, plus what this engine
implements now and the recommended roadmap.

Facts were verified against primary sources (papers, SDK docs, engine source)
by two literature sweeps; anything that could not be pinned to a primary
source is marked UNVERIFIED.

---

## 1. The technique landscape (2020–2026)

### 1.1 Hardware-accelerated ray traced reflections

- **Inline ray tracing** — `VK_KHR_ray_query` (Vulkan) / DXR 1.1: shader-invoked
  ray queries against a BVH from compute or fragment shaders. This is the
  production path for reflections in UE5 hardware RT (Lumen HWRT) and the
  canonical open reference is Quake II RTX.
- **Stochastic 1 spp ray traced reflections** — the industry standard since
  ~2018: one GGX-importance-sampled reflection ray per pixel per frame,
  variance-clipped temporal accumulation, then a spatiotemporal denoiser.
  NVIDIA's reference stack: DXR/RayQuery + **NRD** denoisers (REBLUR/RELAX),
  optionally **RTXDI** for lighting reflection rays. Cost: ~1–3 ms/frame at
  1080p on RTX-class hardware excluding denoise.
- Limitations: denoiser quality bounds image quality at 1 spp (temporal lag on
  glossy highlights, disocclusion ghosting), acceleration-structure
  build/refit cost for dynamic scenes.

### 1.2 Screen-space reflections (SSR)

- **Stochastic Screen-Space Reflections** — Brian Karis, Epic, SIGGRAPH 2015
  PBS course. GGX importance sampling against a Hi-Z pyramid, 2×2 quad
  spatial dilation, variance-clipped temporal accumulation. The baseline
  everyone builds on. ~0.5–2 ms.
- **AMD FidelityFX SSSR** (GPUOpen, MIT license) — hierarchical Hi-Z traversal
  with early termination, roughness-driven GGX sampling, half-resolution
  trace, variance-guided temporal accumulation, plus a spatial denoiser.
  Ships as portable HLSL in the FidelityFX SDK.
- **Half-res trace + joint bilateral upsample** is the default production
  configuration (SSSR, Lumen, most engines).

### 1.3 Software ray tracing fallbacks

- **Mesh distance fields** — UE4/UE5 trace signed distance fields for
  software reflections/GI (Lumen software mode); coarse but cheap.
- **Parallax-corrected cubemap probes** — Sébastien Hillaire, "Physically
  Based Shading in Call of Duty: Advanced Warfare", SIGGRAPH 2015. Box
  projection with volume blending; still the ubiquitous cheap fallback.

### 1.4 Hybrid reflections: UE5 Lumen (the modern reference architecture)

Source: Karis, Wright, Narkowicz et al., "Lumen: Real-time Global Illumination
in Unreal Engine 5", SIGGRAPH 2022 Advances in Real-Time Rendering course.

Half-res stochastic tracing (screen traces first, then hardware ray queries or
distance fields), hits shaded via **hit lighting** against the **surface
cache**, final gather with irradiance probes, variance-guided temporal
filtering, dilation for disocclusions. The reusable pattern for any engine:
1 spp trace → cheap radiance proxies for hit shading → temporal accumulation
+ variance-guided spatial filter. ~2–4 ms on consoles.

### 1.5 Denoising (the enabling technology for 1 spp)

- **SVGF** — Schied et al., HPG 2017 (ACM TOG): moment/variance-guided
  spatiotemporal à-trous filtering; the foundation of virtually all real-time
  denoisers.
- **A-SVGF** — Schied et al., HPG 2018: adaptive kernels, temporal-gradient
  history rejection.
- **ReBLUR** — Ondřej Karlík, Martin Meister, Jiří Bittner, "Fast Denoising
  with Self Stabilizing Recurrent Blurs", CGF/EGSR 2020. Recurrent spatial
  kernel progressively widening across frames; ships in NVIDIA **NRD** as
  REBLUR (used in Cyberpunk 2077). NRD also ships RELAX (A-SVGF-derived,
  specular-specialized) and SIGMA (shadows). Note: NRD's license is the
  NVIDIA RTX SDK license (free to use, not MIT); the old MIT-licensed v2 is
  archived as RayTracingDenoiser.
- **ReSTIR family** (resampling, complementary to denoising):
  ReSTIR DI — Bitterli et al., SIGGRAPH 2020; ReSTIR GI — Ouyang et al.,
  EGSR 2021; **ReSTIR PT** — Daqi Lin, Markus Kettunen, Pascal Grittmann,
  Karol Myszkowski, Chris Wyman, CGF 41(4)/EGSR 2022 (arXiv:2207.06189).
  Production: NVIDIA RTXDI / RTXPT. ("Rearchitecting Spatiotemporal Resampling
  for Production", Dağcı et al. 2023 — UNVERIFIED, could not be located.)
- **Neural Radiance Caching** — Müller, Rousselle, Novák, Keller, ACM TOG
  (SIGGRAPH) 2021, DOI 10.1145/3450626.3459812: tiny MLP caches path radiance;
  impressive but heavy for mid-range GPUs.

### 1.6 IBL ground truth (the basis of this engine's implementation)

- **Split-sum** — Brian Karis, "Real Shading in Unreal Engine 4", SIGGRAPH
  2013 PBS course: prefiltered environment mips indexed by roughness × 2D
  preintegrated DFG LUT (n·v × roughness). Still the shipping standard.
- **Multi-scatter GGX energy compensation** (the correct citations):
  - J. Fdez-Agüera, "A Multiple-Scattering Microfacet Model for Real-Time
    Image Based Lighting", JCGT 8(1), 2019 — second LUT + add-back.
  - Emmanuel Turquin, "Practical Multiple Scattering Compensation for
    Microfacet Models" (2018, JCGT 2019 listing) — closed-form, no extra LUT.
  - Ground truth: Heitz, Hanika, d'Eon, Dachsbacher, "Multiple-Scattering
    Microfacet BSDFs with the Smith Model", ACM TOG 2016,
    DOI 10.1145/2897824.2925943.

### 1.7 Open-source reference implementations

| Project | What to study | License | Notes |
|---|---|---|---|
| Google **Filament** (`google/filament`) | The reference IBL implementation: GGX importance-sampled prefiltered cubemaps + preintegrated DFG LUT (+ multi-scatter compensation stored in the LUT); `cmgen` offline prefilter | Apache-2.0 | Docs: "Physically Based Rendering in Filament" (Filament.md.html; note `IblProbes.md.html` does not exist) |
| **Bevy** (`bevyengine/bevy`) | SSR since 0.14 (WGSL screen-space raymarch, deferred path; `examples/3d/ssr.rs`); split-sum IBL with shipped DFG LUT (`crates/bevy_pbr/src/environment_map/dfg.ktx2`); PR #22379 "Physically Based Screen Space Reflections" in 0.19 | MIT | Closest Rust/wgpu production PBR |
| **AMD FidelityFX SSSR** (`GPUOpen-Effects/FidelityFX-SSSR`) | Hi-Z screen-space reflection kernel, HLSL, portable | MIT | |
| **NVIDIA NRD** (`NVIDIA-RTX/NRD`) | REBLUR / RELAX / SIGMA denoisers, HLSL, Vulkan-integrable | NVIDIA RTX SDK license | Repo is NVIDIA-RTX/NRD, not NVIDIAGameWorks |
| **Falcor** (`NVIDIAGameWorks/Falcor`) | Research prototypes: MinimalPathTracer, RTXDIPass, NRDPass, SVGF | BSD-3-Clause | |
| **wgpu examples** (trunk) | `ray_cube_fragment` (ray queries in fragment shaders), `ray_scene`, `ray_shadows`, `ray_traced_triangle` | MIT/Apache | Hardware RT is `Features::EXPERIMENTAL_RAY_QUERY` (Vulkan-only), WGSL extensions `wgpu_ray_query` from v28; RT pipelines still experimental through v30 |

**Feasibility note for this engine:** exploraFly_2 does *not* use wgpu — it
drives Vulkan directly through `ash`. Hardware ray queries
(`VK_KHR_ray_query`) are therefore available natively on the RTX 4060 without
waiting on wgpu's experimental surface: build a TLAS over the 23 airframe
nodes (+future terrain), inline-trace 1 spp reflection rays in `fs_main`, and
denoise. No middleware required.

---

## 2. What exploraFly_2 implements now

The environment of this sim is **fully analytic** — the sky is the
`physical_atmosphere_sky()` procedural atmosphere function (Rayleigh
gradient, Mie aureole, Pierce limb-darkened solar disk, ground haze), not a
cubemap. That changes the optimal answer:

> The ground-truth reflection of an analytic environment is that same function
> evaluated along GGX-blurred mirror rays — the continuous limit of Karis'
> prefiltered-cubemap split-sum with **zero cubemap resolution error and no
> DFG LUT approximation**.

Implemented in `engine/src/plane.wgsl` (`fs_main`):

1. **Analytic IBL specular**: mirror tap of the atmosphere (with solar disk +
   aureole, giving physically correct sun glints on titanium and canopy
   glass) + 3 sun-free GGX-importance-sampled taps (Walter et al. 2007
   algebraic NDF sampling, stratified) for roughness blur. Blur taps are
   skipped where Fresnel weight makes them invisible (dielectric cloth/paint
   at modest incidence).
2. **Split-sum Fresnel** with roughness compensation (Karis 2013 F term):
   `F_env = f0 + (max(1-rough, f0) - f0)(1-n·v)^5`.
3. **Analytic diffuse IBL**: single normal-direction irradiance tap of the
   sun-free atmosphere (valid because the sky is very low frequency away from
   the disk), scaled by the split-sum kd.
4. **Direct sun** unchanged: Cook-Torrance GGX with Schlick-Smith visibility
   and GSAA normal-variance roughness widening.
5. **Present-weighted shading detail** (see §3): only the compositor-presented
   pass runs the IBL block; intermediate never-presented passes of the
   decoupled burst run direct sun + flat ambient (draw-uniform branch, no
   warp divergence).

Known approximation: single-scatter GGX via importance sampling is
energy-correct per tap; the multi-scatter loss of rough metals (~5–10%) is
not compensated yet — Turquin's closed-form compensation (§1.6) is the
recommended drop-in.

---

## 3. Performance engineering found along the way (2026-09-14)

Target: ≥ 30k theoretical FPS. Baseline after the atmosphere work: 20.6k
(48.6 µs/frame, GPU pass 81 µs); after naive IBL: 16.4k (88.5 µs, GPU 115 µs).

Diagnosis via ablation (the codebase supports `EXPLORA_NO_GPU=1`,
`--benchmark N`, timestamp queries):

- With empty command buffers the loop runs 221k FPS → the wall-clock was GPU
  + WSI, not CPU submission.
- Wall time per burst was linear in pass count: ~405 µs fixed + ~43.5 µs per
  pass, **independent of fragment shader cost**. The per-pass cost was the
  full-screen **D32 depth CLEAR** (19 MB write) repeated by every one of the
  burst's command buffers.

Fixes shipped:

1. Depth `load_op` CLEAR only on the compositor-presented pass; intermediate
   passes run `DONT_CARE` (their output is never sampled).
2. Sky + glass draws only on the presented pass (sky was already gated;
   glass now too).
3. UBO `detail` flag: presented pass = full PBR + IBL; intermediates = direct
   sun + flat ambient.
4. Reflection shader cost control: sun-free evaluation for blur taps,
   loop-invariant tangent frame hoisting, Fresnel-gated blur taps.
5. `RENDER_BURST` 24 → 64: amortizes the ~1 ms NVIDIA-Wayland
   `queue_present` block over more passes (present cost is pacing-dominated;
   measured wall scales ~linearly with burst below the pacing floor).

Result: **36,551 theoretical FPS (27.4 µs/frame) at 2880×1646, real
presentation 571 FPS** — 22% above target with full IBL reflections enabled
on the visible pass. Burst sweep data: 48 → 33.5k/697 fps, 64 → 36.6k/571
fps, 96 → 39.6k/413 fps; 64 chosen as the margin/latency balance. The burst
is runtime-tunable via `EXPLORA_BURST` (clamped 1..=256); low values trade
theoretical throughput for presentation cadence (e.g. `EXPLORA_BURST=8` →
12.9k theoretical / 1616 real fps).

---

## 4. Roadmap (recommended order)

1. **Multi-scatter GGX compensation** (Turquin closed form, ~20 ALU) —
   restores rough-metal energy; no LUT needed.
2. **Analytic SSR for airframe self-reflection** — the only geometry is the
   airframe itself; a bounded depth-march (16–24 steps + binary refine)
   against the airframe's own depth buffer would add wing→fuselage and
   canopy→wing reflections. Requires rendering the airframe to an offscreen
   depth target one subpass earlier (Vulkan dynamic rendering makes this
   cheap); fallback to analytic IBL where the march misses (the Lumen hybrid
   pattern of §1.4).
3. **Hardware ray traced reflections via `ash` + `VK_KHR_ray_query`** — the
   RTX 4060 supports it natively; 1 spp GGX-lobed rays over a TLAS of the 23
   kinematic airframe nodes, shaded by the analytic IBL above, temporally
   accumulated; denoise with a WGSL port of SVGF/ReBLUR-style recurrent
   blurs. Budget: reflections of the planet/terrain once terrain exists —
   where analytic IBL stops being ground truth.
4. **Terrain reflections** (when terrain ships): FidelityFX SSSR-style Hi-Z
   march (MIT, portable HLSL) for water/wet surfaces; prefiltered probes with
   box projection (Hillaire 2015) as fallback.
5. ReSTIR/NRC-class machinery only if the project converges on full path
   tracing — otherwise disproportionate to a single-aircraft scene.

---

## 5. Sources

- Karis 2013 split-sum: https://blog.selfshadow.com/publications/s2013-shading-course/
- Karis 2015 stochastic SSR; Hillaire 2015 CoD:AW probes: https://blog.selfshadow.com/publications/s2015-shading-course/
- Lumen, SIGGRAPH 2022 course: https://www.advances.realtimerendering.com/s2022/index.html
- FidelityFX SSSR: https://gpuopen.com/fidelityfx-sssr/ , https://github.com/GPUOpen-Effects/FidelityFX-SSSR
- NRD: https://github.com/NVIDIA-RTX/NRD ; RTXDI: https://developer.nvidia.com/rtxdi ; RTXPT: https://github.com/NVIDIA-RTX/RTXPT
- ReSTIR PT: https://arxiv.org/abs/2207.06189
- ReBLUR: Karlík, Meister, Bittner, "Fast Denoising with Self Stabilizing Recurrent Blurs", CGF/EGSR 2020
- NRC: https://doi.org/10.1145/3450626.3459812
- Heitz et al. 2016 multi-scatter: https://doi.org/10.1145/2897824.2925943 ; Fdez-Agüera JCGT 2019 ; Turquin 2018
- Filament: https://google.github.io/filament/Filament.md.html , https://github.com/google/filament
- Bevy SSR: https://bevy.org/news/bevy-0-14/ , https://github.com/bevyengine/bevy/pull/22379
- wgpu RT status: https://github.com/gfx-rs/wgpu/issues/6762 , https://docs.rs/wgpu/latest/wgpu/struct.Features.html , https://github.com/gfx-rs/wgpu/blob/trunk/CHANGELOG.md
- UE5 hardware RT docs: https://dev.epicgames.com/documentation/unreal-engine/hardware-ray-tracing-in-unreal-engine
