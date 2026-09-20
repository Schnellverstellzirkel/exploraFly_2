# Alpine grass surface

Authored 2026-09-20. This describes the current raster terrain material in
`engine/shaders/ground.frag`; the older `ground-material.md` documents the
superseded flat-ground stage.

## Appearance and material response

The old meadow received low roughness wherever the baked moisture channel was
high. Moist valley floors also became mud regardless of their distance above
the lake. Combined with a smooth-interface Fresnel term in the sky lighting,
this produced broad white highlights and cyan, reflective hillsides.

Grass now has a rough canopy (base perceptual roughness 0.93–0.99). Soil
moisture still determines vegetation cover, but wet shading applies in
proportion to exposed ground. Mud is restricted to gentle wet shoreline ground
within nine vertical metres of the water level. Rock, scree, forest, snow,
roads, and the existing lake material retain their separate blends.

The land sky response uses an analytic GGX environment-BRDF fit. Its integrated
reflectance also controls the sky diffuse/specular split, so rough grass
retains color when viewed at grazing angles. A restrained normalized wrap
diffuse blend softens the aggregate grass response to sun. This is an artistic
canopy approximation, not a measured grass BSDF or actual light transmission
through leaves. Water and building sky responses use their existing path.

## Surface detail and bounded work

- Two existing 2K meadow slots now contain Poly Haven's CC0 **Grass Ground**
  diffuse and OpenGL normal pair, replacing Aerial Grass Rock. No new GPU
  textures, descriptors, draws, geometry, or per-frame uploads are added.
- The photo covers 2.5 m, with a rotated 6.75 m layer to reduce repetition.
  Scalar luminance preserves blade/thatch contrast while an authored
  yellow-green palette supplies fresh valleys, warm slopes, and high pasture.
  The diffuse image's decoded linear Rec. 709 luminance mean is 0.12318757;
  the shader uses 0.1232 to keep mip averages near the authored color.
- World-anchored 3 m and 9 m noise supplies tussock color and height gradients,
  replacing two unrelated 20 m / 8 m normal offsets. Analytic derivatives
  share the four lattice hashes used for each value. Their relief amplitudes
  are 0.14 m / 0.28 m; these perturb lighting only, not the mesh or collision.
- Texture gradients are calculated before meadow detail branches. The small
  broad UV warp is excluded from this mip estimate; its modest distortion is
  an approximation. The secondary normal slopes receive the inverse UV
  rotation, and world X/Z gradients are projected onto the terrain surface.
- Procedural values fade toward their mean and gradients toward zero as the
  pixel footprint grows. Hardware mipmaps and anisotropy filter the photo.
  Detail gates bound work; a small grass cavity factor affects indirect light.

The change does not add standing grass silhouettes, wind animation, parallax,
or blade shadows. At flight altitude individual blades are subpixel; the
material must resolve into a matte, varied meadow rather than persistent
high-frequency noise. Very close ground still exposes the existing terrain
mesh and lacks blade geometry. Coarse shading in the performance preset
reduces visible detail further.

## Primary sources and limits

All sources accessed 2026-09-20. These are established material techniques,
not evidence that this renderer is state of the art or meets its frame budget.

| Source | Publication / update date | Application here | Limits |
| --- | --- | --- | --- |
| [Karis, Physically Based Shading on Mobile](https://www.unrealengine.com/en-US/blog/physically-based-shading-on-mobile) | 2014-09-10 | Analytic environment-BRDF fit combines roughness and view angle without another sampler. | Foundational approximation; the sky-gradient direction used here is not a convolved environment map. The article explicitly permits reuse of its sample code. |
| [Guy / Agopian, Physically Based Rendering in Filament](https://google.github.io/filament/main/filament.html) | First public version 2018-08-03; living documentation, latest page update unspecified | Separate base color, normal, roughness, AO, and integrated specular response; normalized wrap diffuse as a bounded soft-lighting cue. | Filament's cloth discussion is not a grass measurement; only the wrap principle is used, not its cloth BRDF. |
| [Zirr / Kaplanyan, Real-time Rendering of Procedural Multiscale Materials](https://research.nvidia.com/publication/2016-02_real-time-rendering-procedural-multiscale-materials) | 2016-02-01 | Filter procedural detail by its pixel footprint and converge unresolved variation toward a stable mean. | Uses this principle, not the paper's full multiscale BRDF model. |
| [Khronos, pipeline fragment shading rate](https://docs.vulkan.org/refpages/latest/refpages/source/VkPipelineFragmentShadingRateStateCreateInfoKHR.html) | Current reference, no per-page update date provided | Existing capability-checked shading rate explains why performance and native screenshots resolve different detail. | No shading-rate or Vulkan resource-layout changes; shader target remains Vulkan 1.3. |
| [Poly Haven, Grass Ground](https://polyhaven.com/a/grass_ground), [asset metadata](https://api.polyhaven.com/info/grass_ground), [license](https://polyhaven.com/license) | Asset published 2026-09-18; license page update unspecified | Charlotte Baglioni's CC0 scan supplies turf structure. Exact downloads and hashes are in `engine/assets/detail/ATTRIBUTION.md`. | Recent asset release, not recent rendering research; not a survey of alpine plant species. The source's brown color is not used as the meadow palette. |

## Validation

`cargo test --workspace --locked` passed all 100 tests. The required
`tools/check_shaders.py` validated 52 compiled SPIR-V modules for Vulkan 1.3,
including the ground shadow variants. The release binary also built.
`spirv-val` was obtained from Ubuntu's `spirv-tools`
`2025.1~rc1-1~ubuntu0.24.04.2` package, extracted in a task-local temporary
directory without installing system packages.

The environment skill's baseline was rechecked before testing: native Linux,
Wayland, RTX 4060 Laptop GPU, NVIDIA **580.173.02**. The built-in panel runs at
120.001 Hz. The following are complete rendered scene **presentation
submissions**, not counts of physically displayed frames. No display timing
feedback was available. Every run used `EXPLORA_BURST=1`, RT shadows on,
MAILBOX presentation, calm wind, audio off, and HUD hidden.

| Workload | Performance: frozen cruise | Cinematic: frozen low flight |
| --- | ---: | ---: |
| Output pixels | 2880×1646 | 2880×1646 |
| Scene pixels | 2304×1317 | 2880×1646 |
| Ground shading rate | capability-selected 2×2 | 1×1 |
| Aircraft altitude / heading | 1100 m / default | 380 m / 1.57 rad |
| Measured presents after warmup | 2000 | 750 |
| Submitted presents/s | 256.932 | 120.336 |
| Wall interval mean | 3.892 ms | 8.310 ms |
| Wall p50 / p95 / p99 | 3.941 / 5.081 / 6.476 ms | 8.405 / 9.444 / 10.821 ms |
| GPU mean | 3.842 ms | 8.294 ms |
| GPU p50 / p95 / p99 | 3.898 / 4.958 / 4.978 ms | 8.333 / 9.042 / 9.490 ms |
| Terrain timestamp interval, mean | 1.343 ms | 5.642 ms |

The **1 ms / 1000 submissions/s target is not met**. These are diagnostic
runs of the integrated working scene, not a performance certification. Other
renderer work landed during this session, so the initial and final captures
are not an isolated timing comparison of the grass change. The two runs above
used the same copied release executable, SHA-256
`a374c162b4f235071b9351c61379148ddf3a0433ff243c9e8c81630c850490a9`.

At both flight heights, image review confirmed warmer green ground, loss of
the broad white meadow highlight, and fine surface contrast. The lake remains
reflective. Terrain triangle/shadow artifacts visible in these images belong
to the existing geometry and shadow path; this material work does not address
them. Still images do not prove absence of temporal shimmer.

A moving bank/boost smoke run also completed: Performance, the same
resolutions, 380 m / 1.57 rad starting pose, 1200 measured presents, and
`EXPLORA_BOOST=1 EXPLORA_BANK=1` without freeze. It recorded 238.270 submitted
presents/s; wall mean/p50/p95/p99 were 4.197/4.097/5.273/6.449 ms, GPU
mean/p50/p95/p99 were 4.075/4.055/5.148/5.241 ms. Synchronous screenshots ran
inside this measurement window, which includes a 119.681 ms maximum wall
interval. Treat it as a moving-scene smoke check, not a controlled throughput
comparison. Its full report is retained alongside the frozen runs.

Reproduce the frozen cruise workload from the repository root:

```sh
EXPLORA_QUALITY=performance EXPLORA_FREEZE=1 EXPLORA_WIND=0 \
EXPLORA_AUDIO=0 EXPLORA_HUD=0 EXPLORA_BURST=1 \
EXPLORA_GPU_SPIKE_US=100000 EXPLORA_SHOT=/tmp/alpine-grass.ppm \
EXPLORA_BENCH_JSON=/tmp/alpine-grass.json \
target/release/explora --benchmark 2000
```

For low flight, use `EXPLORA_QUALITY=cinematic EXPLORA_ALT=380
EXPLORA_HEADING=1.57` and `--benchmark 750`. `EXPLORA_FREEZE=1` fixes both
pose and simulation time; the screenshots are taken at present 120, before
the measurement window. The runner waits two seconds and 500 warmup presents
before collecting the reported wall intervals. Local screenshots, JSON
reports, and logs are retained under `target/shots/alpine-grass/` (ignored
build artifacts, not required game assets).
