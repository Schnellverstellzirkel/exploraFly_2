# Expedition aircraft: materials and wake

Research checked: 2026-09-19. This pass gives the existing swept glider a
handcrafted alpine expedition identity: cream woven sails, deep teal enamel,
painted compass/stripe markings, brass bindings and frames, warm cockpit lights,
and a small turquoise turbine core. The hot nozzle and rotor remain steel.
These are original procedural markings; no game assets or generated textures
are included.

## Implemented

- Canvas now covers the upper and lower main sail. Thin brass leading-spar
  binding catches light across the broad silhouette. The wings, six flaps,
  rotor, ten nozzle petals, and two fins keep their existing animation nodes.
- Hull and tube UVs follow their surfaces. Centered spline tangents keep the
  final tube ring circular instead of collapsing it at the end of a path.
- Broad enamel bands and compass stencils use the existing UVs. Fine seam and
  weave detail fades at the pixel footprint to avoid distant shimmer. Metallic
  fittings use colored specular reflectance and zero diffuse albedo; painted
  surfaces remain dielectric. Roughness and sheen distinguish cloth, leather,
  enamel, and metal.
- The base specular lobe broadens under a rough clear coat using the OpenPBR
  notes' equation 86. With coat IOR fixed to 1.5, the added slope variance is
  `2/3 * coat_weight * coat_roughness^4`. Fractional coat coverage is blended
  in variance space, a deliberate approximation. Existing GGX, Charlie sheen,
  lookup-table energy compensation, and geometric specular filtering remain.
- Port/starboard lights now have distinct steady colors. Turbine emission alone
  tracks spool. The plume combines a warm artistic thermal ramp with a narrow
  turquoise fictional engine core. Shock-cell contrast increases with boost.
  The thermal ramp is explicitly not a spectral blackbody calculation.
- Young trail centers stay coherent while old edges erode into wisps using
  the existing two noise samples. Deposited CPU segments now start at their
  actual emission opacity and reach zero smoothly before expiry; the previous
  seed mixture caused a birth-opacity drop and a visible lifetime cutoff.

## Research and scope

| Primary source | Date | Choice in this pass |
| --- | --- | --- |
| [Portsmouth, Kutz, Hill — OpenPBR: Novel Features and Implementation Details](https://arxiv.org/abs/2512.23696) ([equations and revision history](https://arxiv.org/html/2512.23696v2)) | SIGGRAPH course August 2025; arXiv submitted 2025-12-29, revised 2026-01-14. The served HTML additionally lists an internal v1.3 dated 2026-02-14. | Implement the inexpensive rough-coat broadening heuristic from §5.3. Keep material parameters separated by physical role. Full OpenPBR layering, EON diffuse, spectral thin films, refraction through coats, and full fuzz transport are outside this pass. |
| [Kneiphof, Klein — Real-time Image-based Lighting of Glints](https://arxiv.org/abs/2507.02674) | Submitted 2025-07-03 | Reviewed and deferred. The method adds environment filtering/storage and discrete-microfacet evaluation. Our small brass fittings use stable broad highlights; this implementation does not claim to render that paper's glints. |
| [Google Filament — physically based rendering reference](https://google.github.io/filament/main/filament.html) | Living official documentation, accessed 2026-09-19; the cited Charlie sheen and energy-compensation work dates to 2017–2018 | Retain the existing cloth sheen, conductor/dielectric distinction, GGX energy compensation, and footprint filtering. These are established techniques, not new 2026 research. |

The existing plume's noise, shock-spacing model, Beer–Lambert integration,
and HG/Cornette–Shanks phase functions are likewise established approximations.
This pass improves art direction and lifetime continuity; it does not introduce
a CFD solver or claim physically exact contrail microphysics.

## Cost and compatibility

No new descriptor bindings, textures, render passes, ray queries, or temporal
history buffers are introduced. Airframe geometry is still built once at boot,
fits the packed `u16` vertex budget, and preserves all eight material IDs, all
23 node indices, and the existing packed vertex format. Hull UV changes do not
alter its positions. The wing binding adds two raw parts that the renderer
merges into its existing material/node batches.

The plume keeps its 18-step ceiling, with 14 samples at idle and 16 at medium
spool. It retains one curl lookup per ray and at most one base-volume lookup
per contributing step. Trail shading retains two volume lookups per pixel.
Canvas marks and coat broadening add arithmetic, not texture lookups. No new
CPU allocations are made in the effects update. These bounds support a fast
rendering path, but do not establish a frame rate. The 1,000 FPS aspiration
requires measuring the complete renderer within a 1 ms frame budget on the
target GPU.

## Validation

On 2026-09-19 this branch passed all 53 workspace tests and validated all 24
compiled SPIR-V modules for Vulkan 1.3 in `explorafly-check`.

Regression tests exercise actual generated geometry: tube endpoint radius and
UVs, hull UV seams, finite vertices, valid indices, mirrored wing bounds and
`u16` vertex capacity. A wake lifetime test checks initial opacity, monotonic
fade, near-zero opacity before expiration, and final removal.

Run the complete CPU/shader check from the repository root:

```powershell
docker run --rm -v "${PWD}:/workspace" -v explorafly-cargo:/usr/local/cargo/registry -v explorafly-aircraft-target:/workspace/target explorafly-check
```

This compiles the engine and all shader variants, runs the workspace tests,
and validates SPIR-V for Vulkan 1.3. It does not render the game. Visual review
is still needed on the Linux/Vulkan target: orbit above/below both wings, inspect
the nose collar and brass highlights at low sun, compare idle and full boost,
and watch a turn's wake age from close and distant chase cameras. Capture GPU
timings with the same resolution, render scale, clouds, and ray-query settings
before making performance claims.
