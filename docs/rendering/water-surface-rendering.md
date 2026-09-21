# Water Surface Rendering: Short-Crested Alpine Wave Synthesis & Multiscale Optics

## Executive Summary

Explora's alpine water surface rendering replaces legacy two-sine wave perturbations
with a physically grounded, non-repetitive, multiscale short-crested wave model.
The same pass now serves two water bodies: sheltered alpine lakes and a separate
open-ocean shelf biome. The ocean reuses the filtered analytic path but selects
longer swell, stronger open-water chop, deeper absorption, and a blue coastal
palette from the generated world classification.
The implementation executes entirely within the primary opaque ground pass (`engine/shaders/ground.frag`),
requiring zero auxiliary compute passes, zero screen-space reflection textures, and zero frame memory allocation,
keeping the added biome bounded to the existing terrain-pass budget. The
complete 1 ms presentation target remains unachieved and is measured separately.

---

## Primary Publications & Theoretical Foundation

### 1. Directional Wave Dispersion & Short-Crested Wave Synthesis
* **Tessendorf, Jerry (2001)**. *"Simulating Ocean Water"*. SIGGRAPH 2001 Course Notes.
  * URL: <https://people.computing.clemson.edu/~jtessen/reports/papers_files/coursenotes2004.pdf>
  * Access Date: 2026-09-20.
  * *Applicability*: Foundational gravity wave dispersion relation in deep water: $\omega^2 = g k$, where phase velocity $c(k) = \sqrt{g / k}$.
  * *Limits*: Pure Fourier synthesis requires 2D FFT grids that tile periodically over large flight expanses; analytic summation with incommensurate frequencies is used instead to eliminate spatial repetition.

* **Finch, Mark (2004)**. *"Effective Water Simulation from Physical Models"*. *GPU Gems 1*, Chapter 1, Addison-Wesley / NVIDIA.
  * URL: <https://developer.nvidia.com/gpugems/gpugems/part-i-natural-effects/chapter-1-effective-water-simulation-physical-models>
  * Access Date: 2026-09-20.
  * *Applicability*: Trochoidal Gerstner wave derivative shaping. Wave crests sharpen via $\cos(\phi)(1 + Q \sin(\phi))$ while troughs broaden and flatten, providing liquid surface characteristics without numerical derivatives.

* **Mitsuyasu et al. (1975) & Hasselmann et al. (1973/1980)**. *"Observations of the directional spectrum of ocean waves"* & JONSWAP Project.
  * *Applicability*: Directional spreading function $D(\theta) \propto \cos^{2s}(\theta/2)$ and fetch-limited peak frequency scaling.
  * *Limits*: Alpine mountain lakes are sheltered and fetch-limited ($< 2$ km), meaning long-period ocean swells ($\lambda > 30$ m) do not develop. The lake spectrum is restricted to high-frequency chop and capillary ripples ($\lambda \in [0.95, 15]$ m); the ocean variant expands the same components to approximately $[4, 62]$ m rather than pretending the lake spectrum describes open water.

* **Jeschke, Stefan & Wojtan, Chris (2017)**. *"Water Wave Animation via Wavefront Tracking"*. *ACM Transactions on Graphics* (SIGGRAPH 2017).
  * URL: <https://visualcomputing.ist.ac.at/publications/2017/WWAvWT/>
  * Access Date: 2026-09-20.
  * *Applicability*: Transversal crest envelope modulation. Real lake waves are **short-crested**: wave crests have a finite lateral span ($2.5$ to $3.5$ wavelengths) rather than infinite parallel ridges. Modulating the wave amplitude along $\mathbf{d}^\perp = (-d_y, d_x)$ breaks 1D parallel swells into discrete 3D wave mounds.

### 2. Multiscale Filtering & Hierarchical Geometry-to-BRDF Transition
* **Bruneton, Eric; Neyret, Fabrice; Holzschuch, Nicolas (2010)**. *"Real-time Realistic Ocean Lighting using Seamless Transitions from Geometry to BRDF"*. *Computer Graphics Forum* (Eurographics 2010), 29(2), pp. 487–496.
  * URL: <https://hal.inria.fr/inria-00443630>
  * Access Date: 2026-09-20.
  * *Applicability*: Hierarchical transition across spatial scales. Geometric waves that cannot be resolved by the pixel footprint are band-limited, and their unresolved normal slope variance is integrated into microfacet specular roughness ($\sigma_{\text{BRDF}}^2$). This completely prevents specular flickering, moiré, and temporal crawling at flight altitudes up to 5,000 m.

* **Zirr, Tobias & Kaplanyan, Anton (2016)**. *"Real-time Rendering of Procedural Multiscale Materials"*. *Computer Graphics Forum* (EGSR 2016), 35(4).
  * URL: <https://cg.ivd.kit.edu/publications/2016/procedural_multiscale/>
  * Access Date: 2026-09-20.
  * *Applicability*: Pixel footprint clamping $\text{coverage} = 1 - \text{smoothstep}(\lambda \cdot 0.35, \lambda \cdot 1.60, \text{footprint})$ for procedural procedural features.

* **Dupuy, Jonathan & Bruneton, Eric (2012)**. *"Real-time Animation and Rendering of Ocean Whitecaps"*. *ACM Transactions on Graphics* (SIGGRAPH 2012), 31(4).
  * URL: <https://hal.inria.fr/hal-00701131>
  * Access Date: 2026-09-20.
  * *Applicability*: Wave deformation/steepness thresholding for foam generation. Whitecaps form where the local wave surface gradient $\|\nabla \eta\|$ exceeds the breaking threshold $S_{\text{crit}} \approx 0.038$, modulated by wind gust intensity.

* **Cox, Charles & Munk, Walter (1954)**. *"Measurement of the Roughness of the Sea Surface from Photographs of the Sun's Glitter"*. *Journal of the Optical Society of America*, 44(11), pp. 838–850.
  * *Applicability*: Empirical calibration of mean-square surface slope $\sigma^2$ vs wind speed, setting alpine breeze slope variance to $\sigma \approx 0.035$.

### 3. Subsurface Transmission & Depth Optics
* **Beer-Lambert Multi-Spectral Absorption (Schumann 1996)**:
  * Pure water and glacial silt absorption: $\mathbf{T}(d) = \exp(-\mathbf{\beta} \cdot d)$, with $\mathbf{\beta} \approx (0.24, 0.048, 0.015)\ \text{m}^{-1}$.
  * Red wavelengths attenuate rapidly ($< 4$ m), green at mid-depths ($< 12$ m), while blue penetrates deepest ($> 25$ m), producing the signature alpine glacial turquoise-to-sapphire gradient.
  * Submerged lake-bed terrain visibility is preserved in shallow water ($d < 2.5$ m) via exponential bed transmission $\exp(-d \cdot 0.70)$.

* **Stam, Jos (1996) / Guardado & Sanchez-Crespo (2004)**. *"Rendering Water Caustics"*. *GPU Gems 1*, Ch. 2.
  * *Applicability*: Animated bi-directional caustic interference network on shallow submerged bed sediments.

---

## Architectural Implementation

The water system is implemented directly in `engine/shaders/ground.frag` under the terrain material branch (`vMaterial == 0u` when `water > 0.0`):

1. **Dual-Tier Continuous Domain Warping**:
   * Coarse octave at 130 m scale ($\pm 12.0$ m offset) breaks valley-scale rectilinear alignment.
   * Fine octave at 42 m scale ($\pm 4.5$ m offset) breaks wavefront coherence across adjacent ripple trains.
2. **Alpine Wind & Sheltered Slick Modulation**:
   * Macro wind streaks at 360 m and 140 m scales create organic transitions between wind lanes and mountain-sheltered calm slicks.
   * In calm slicks, wave chop is dampened by 90%, and surface roughness drops to mirror-glass levels ($R \approx 0.020$), reflecting the sky horizon and surrounding mountain massifs.
   * In wind lanes, wave steepness rises to $0.024$ and roughness to $0.082$, producing glistening sunlight sparkles.
3. **8 Short-Crested Wave Components**:
   * Golden-ratio angular distribution spanning $> 180^\circ$ relative to prevailing breeze.
   * Lakes use incommensurate wavelengths ($\lambda \in [0.95, 14.8]$ m); ocean pixels widen those wavelengths by a 4.2 swell factor ($\lambda \in [3.99, 62.16]$ m) and raise amplitude modestly. This is a bounded visual ocean approximation, not a full FFT/JONSWAP spectrum.
   * Transversal crest modulation bounds each wave crest to $\approx 2.8\ \lambda$, preventing continuous parallel stripe patterns.
4. **Distance Pre-filtering into GGX Roughness**:
   * Each wave component fades smoothly as the camera pixel footprint exceeds its wavelength.
   * Residual slope variance is added into GGX roughness: $\Delta R = (1 - \text{fade}_i) \cdot S_i \cdot 0.55$.
5. **Glacial Palette & Shallow Caustics**:
   * Exponential depth absorption across three key spectral bands (crystalline turquoise shallows, emerald shelf, sapphire deeps).
   * Dancing caustics illuminate submerged gravel and silt along the shoreline.
   * Animated wave-lapping foam fringe along the lake perimeter.

6. **Ocean separation and coast**:
   * `crates/world` emits the periodic coastline and shelf recipe into the
     generated GLSL include. CPU height/collision, cached terrain samples, and
     shader classification therefore agree on which submerged triangles are
     ocean rather than lake.
   * The fragment stage evaluates this mask only for water and a 90 m
     near-shore band. A sand transition above the 185 m waterline and stronger
     ocean foam make the shelf readable without a second mesh or a per-vertex
     trigonometric cost.
   * The ocean floor is deliberately bounded to 82--235 m and the water
     surface remains flat at 185 m. This is a gameplay/visual biome, not a
     bathymetric simulation; the terrain grid's 64/128 m raster LOD still
     limits shoreline geometric detail.

---

## Verification & Performance

* **SPIR-V Compilation**: Validated via offline `shaderc` at build time in `engine/build.rs` (targeting Vulkan 1.3, SPIR-V 1.4).
* **Test Suite**: `cargo test --workspace --locked` passes, and
  `tools/check_shaders.py` validates all 232 compiled SPIR-V modules for
  Vulkan 1.3.
* **GPU Budget**: The new ocean path has no separate dispatch or allocation and
  moves classification out of the terrain vertex stage. A target-machine
  screenshot run at Performance (1930×1103 scene, RTX 4060 Laptop GPU, NVIDIA
  580.173.02, RT on) recorded terrain GPU samples around 0.65--0.71 ms while
  viewing the ocean. A 1,000-present MAILBOX benchmark of the same open-water
  Performance scene with RT shadows off measured 1.622 ms mean GPU time,
  2.427 ms GPU p99, and 546.8 complete submissions/s. These are scene-local
  observations, not proof that the complete 1 ms presentation target is met;
  full frame timing remains governed by the other passes and present path.
