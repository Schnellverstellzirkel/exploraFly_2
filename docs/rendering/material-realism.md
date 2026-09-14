# Airframe Material Realism

This note records the optical model, implementation choices, measured cost,
and the next steps required to turn the procedural airframe into a calibrated
digital material asset. It was researched and validated on 2026-09-14 for the
RTX 4060 Laptop GPU and NVIDIA 580.173.02 driver used by this project.

## What realism requires

A single metallic/roughness pair is unable to describe this aircraft. The
visible surfaces contain different scattering mechanisms:

| Surface | Dominant physics | Real-time model used here |
|---|---|---|
| Polyester sail | Diffuse fibers, directional weave, grazing sheen | Dielectric diffuse + Charlie sheen + filtered weave normal |
| Painted composite | Pigmented substrate below clear resin | Diffuse dielectric + energy-attenuating GGX coat |
| Carbon laminate | Dark woven substrate below resin | Directional twill + anisotropic base lobe + GGX coat |
| Titanium | Colored conductor Fresnel and brushed grooves | No diffuse term + anisotropic GGX |
| Black coating | Absorbing pigment in a rough dielectric binder | Low linear diffuse reflectance + broad dielectric GGX |
| Leather seat | Rough pigmented fibers | Diffuse dielectric + weak Charlie sheen |
| Canopy | Smooth dielectric reflection and transmission | IOR 1.5 Fresnel + thin-sheet alpha transmission |

The parameters in the shader are physically plausible priors. They are not
measurements of a real manufactured aircraft. Indistinguishable-from-reality
rendering ultimately requires calibrated geometry, spectral or at least
linear-HDR illumination, measured material data, camera response, and a
reference-image validation loop. Measured BRDF data is the proper target for
this work [14, 15]. A better BRDF cannot recover detail absent from the asset.

## Implemented model

`engine/src/plane.wgsl` now uses a material record per surface rather than
packing color and metallicity into one shared response. Material IDs are a
flat interpolation so primitive identity does not drift across triangles.

Direct and environmental specular use anisotropic GGX with height-correlated
Smith masking-shadowing, the production model family documented by Filament
and PBRT [1, 2]. Environment integration uses deterministic visible normal
sampling from Dupuy and Benyoub's spherical-cap construction [3]. This is
more faithful and cheaper than sampling an NDF and rejecting invisible
microfacets. The contribution uses the analytic `f cos / pdf` simplification,
including `G2/G1`, rather than averaging arbitrary blurred sky directions.

Diffuse sky illumination uses eight cosine-weighted directions. The solar
disk is excluded from both environment integrals and evaluated once as a
direct light, preventing the previous double count. Coated composite and
carbon evaluate a second dielectric GGX lobe and attenuate the substrate as
light crosses the top interface, following real-time layered-BRDF work [6, 7].
Cloth and leather add the Charlie fiber distribution described by Estevez and
Kulla [4]. Microstructure fades according
to the pixel footprint so distant weave becomes a broader lobe rather than
shimmering.

The directional-albedo texture is regenerated at startup with deterministic
Hammersley integration for the exact height-correlated Smith function used in
the shader. The gain `1 + F0 * (1 / Ess - 1)` is a low-cost single-scatter
energy-loss compensation approximation of the production method documented
by Filament [2]. It approximates energy recovered by true multiple scattering,
whose Smith-model theory is described by Heitz et al. [5]; it is not that
stochastic multiple-scattering BSDF. For anisotropic lobes the lookup uses
geometric-mean slope, an explicit approximation because the table is two-dimensional.

The default uses eight visible-normal samples per specular lobe. The engine
generates compile-time shader variants through `EXPLORA_IBL_SAMPLES=8|16|32|128`.
The 128-sample variant is useful as an image reference; it is not intended as
the default play setting. No sample count branch or dynamic loop bound remains
in the generated SPIR-V.

The octahedral normal decoder also had a correctness defect: after unfolding
the lower hemisphere it recomputed a positive Z value. Back-facing normals
therefore reflected the same hemisphere as front-facing normals. Both CPU and
WGSL decoders now preserve the original signed Z, and an exhaustive lattice
round-trip test covers both hemispheres.

## GPU design

The implementation preserves the engine's two indexed draws and compressed
28-byte vertex stream. Tangents are reconstructed from position and UV
derivatives, avoiding another vertex attribute and its bandwidth. Constants
are authored in a branch-friendly material switch; the airframe occupies a
small screen region and the merged draw avoids costly pipeline and draw-call
churn. Hammersley azimuths are embedded constants, removing trigonometric work
from each specular sample. The diffuse integral has only eight evaluations.

This matches NVIDIA's advice to compress vertex data, reuse command buffers,
minimize pipeline binds and submissions, and control shader register pressure
[17].
The eight-sample choice is empirical: at 2880x1646 it differed from 16 samples
by only 0.11--0.14/255 mean absolute channel error over large quarter and side
view crops, with 48.7--48.9 dB PSNR. This is pointwise crop PSNR, not a
perceptual difference score; NVIDIA FLIP is a better comparison metric for a
future measured reference [16]. These comparisons used 8-bit post-tonemap
captures, with crop rectangles `[300, 600, 2700, 1450]` at quarter and side
views and `[120, 980, 2760, 1250]` for the 16-versus-128 chase-view comparison.
The captures and comparison script remain in `/tmp`, not in the repository,
so the image metrics are author-measured results rather than archived test
fixtures. The 16-sample result retained 47.5 dB PSNR against 128 samples over
the chase-view aircraft crop.

Measured with a frozen pose on the 2880x1646 display, release-equivalent
`perf` profile, 20,000 frames for the chase view; the 8-sample range includes
additional 5,000-frame quarter and side-view runs:

| Variant | Real presentation submissions/s | GPU interval | Purpose |
|---|---:|---:|---|
| Previous shared reflection model | 2,826 | 96 us | Baseline |
| 8-sample material model | 1,522--1,969 | 216--313 us | Default; repeated chase, quarter, and side views |
| 16-sample material model | 1,587 | 313 us | Higher quality |
| 32-sample material model | 1,109 | 497 us | Higher quality |
| 128-sample material model | 433 | 1,617 us | Reference |

The presentation count is not monitor refresh. The built-in display scans out
at 120 Hz. Even the reference variant remains real-time, while eight samples
keeps the earlier 1,400-submission/s target in all measured views.

## Limits that still matter

The analytic sky is an artistic RGB approximation. It is not the multiple-
scattering atmosphere from Hillaire 2020, and its sampled integral is only as
physical as the radiance function being sampled. The renderer also tone maps
the aircraft before blending the canopy over an already tone-mapped sky.
Correct glass needs reflection and transmission composed in linear HDR, with
refraction, Beer-Lambert absorption, thickness, and ideally polarization.

There is no self-reflection, terrain reflection, shadowing from the aircraft,
or multi-bounce transport because the engine has no scene acceleration
structure. NVIDIA's path-tracing, denoising, and resampling references show
what a more complete stack involves [10, 11]. The installed GPU and driver expose `VK_KHR_ray_query`,
`VK_KHR_acceleration_structure`, `VK_NV_ray_tracing_invocation_reorder`, and
`VK_NV_cooperative_vector`. That makes ray queries and neural shading possible
on this machine, but neither can produce missing material measurements.

The model lacks scratches, oxidation, fingerprints, seams, fasteners, resin
depth variation, fiber orientation maps, canopy thickness, and manufacturing
imperfections. Those mesoscopic features are among the strongest cues that
separate a clean procedural model from a photograph.

## Recommended path to measured realism

1. Acquire real samples or a specific aircraft reference. Use calibrated
   cross-polarized RAW/HDR captures to estimate diffuse/base color. Capture
   parallel-polarized or otherwise controlled multi-angle/multi-light images,
   or measure a BRDF directly, for roughness and clearcoat response. Obtain
   geometry and normals from photogrammetry, structured light, or a fitted
   asset. Use the RGL measured BRDF database as validation material.
2. Build a spectral Mitsuba 3 reference scene for the same geometry and
   camera. Enable its polarization support only if polarimetric measurements
   are acquired. Fit the real-time parameters against controlled photographs,
   then use NVIDIA FLIP to compare reference and real-time output.
3. Move rendering to a linear HDR target. Add physically calibrated exposure,
   camera response, and post-tonemap canopy composition.
4. Replace the analytic sky approximation with Hillaire's transmittance,
   multiscattering, sky-view, and aerial-perspective LUTs. The public Unreal
   Engine Sky Atmosphere repository is the closest implementation reference.
5. Add ray-query self-reflections and shadow rays over BLAS geometry, then add
   temporal accumulation and a small SVGF/RELAX-style filter. NVIDIA RTXPT and
   NRD are architecture references; direct copying would bring large framework
   and license costs into this small raw-Vulkan engine.
6. If measured multilayer BRDFs become too expensive, neural appearance
   models offer an alternative representation [8]. NVIDIA's RTXNS is a
   working Linux/Vulkan reference and the RTX 4060 satisfies its stated GPU and
   driver requirements. This only becomes useful after representative training
   data and a trustworthy offline target exist.

## Primary references and implementations

1. Pharr, Jakob, Humphreys, *Physically Based Rendering, 4th ed.* — conductor,
   dielectric, rough dielectric, and layered BSDF reference:
   https://pbr-book.org/4ed/contents
2. Google Filament, *Physically Based Rendering in Filament* — production GGX,
   clear coat, anisotropy, cloth, IBL, and energy compensation:
   https://google.github.io/filament/main/filament.html
3. Dupuy and Benyoub, *Sampling Visible GGX Normals with Spherical Caps*, HPG
   2023 — paper and code: https://ggx-research.github.io/publication/2023/06/09/publication-ggx.html
4. Estevez and Kulla, *Production Friendly Microfacet Sheen BRDF*, SIGGRAPH
   2017 course: https://blog.selfshadow.com/publications/s2017-shading-course/imageworks/s2017_pbs_imageworks_slides_v2.pdf
5. Heitz et al., *Multiple-Scattering Microfacet BSDFs with the Smith Model*,
   SIGGRAPH 2016: https://doi.org/10.1145/2897824.2925943
6. Belcour, *Efficient Rendering of Layered Materials using an Atomic
   Decomposition with Statistical Operators*, SIGGRAPH 2018:
   https://belcour.github.io/blog/research/publication/2018/05/05/brdf-realtime-layered.html
7. de Dinechin and Belcour, *Rendering Layered Materials with Diffuse
   Interfaces*, I3D 2022:
   https://belcour.github.io/blog/research/publication/2022/04/13/brdf-layered-diffuse.html
8. Zeltner et al., *Real-Time Neural Appearance Models*, SIGGRAPH 2024:
   https://research.nvidia.com/labs/rtr/neural_appearance_models/
9. NVIDIA RTX Neural Shading SDK, Vulkan cooperative-vector samples:
   https://github.com/NVIDIA-RTX/RTXNS
10. Lin, Kettunen, and Wyman, *ReSTIR PT Enhanced*, I3D 2026:
    https://research.nvidia.com/labs/rtr/publication/lin2026restirptenhanced/
11. NVIDIA RTXPT and NRD implementation references:
    https://github.com/NVIDIA-RTX/RTXPT and https://github.com/NVIDIA-RTX/NRD
12. Hillaire, *A Scalable and Production Ready Sky and Atmosphere Rendering
    Technique*, EGSR 2020: https://doi.org/10.1111/cgf.14050
13. Unreal Engine Sky Atmosphere reference implementation:
    https://github.com/sebh/UnrealEngineSkyAtmosphere
14. Mitsuba 3, spectral differentiable reference renderer with optional
    polarization support:
    https://github.com/mitsuba-renderer/mitsuba3
15. Dupuy and Jakob measured material database: https://rgl.epfl.ch/materials
16. NVIDIA FLIP perceptual image-difference metric:
    https://research.nvidia.com/publication/2020-07_flip-difference-evaluator-alternating-images
17. NVIDIA, *Advanced API Performance: Shaders* and *Vulkan Dos and Don'ts*:
    https://developer.nvidia.com/blog/advanced-api-performance-shaders/ and
    https://developer.nvidia.com/blog/vulkan-dos-donts/
