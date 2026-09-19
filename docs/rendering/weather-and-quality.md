# Weather and image quality

The native renderer has three startup presets, selected with
`EXPLORA_QUALITY=performance|balanced|cinematic`. Performance is the default
so the stock 120 Hz display path retains motion headroom; balanced and
cinematic remain explicit quality choices.
Settings persist through window resizing and are printed at startup.

Balanced renders the scene at the window resolution, with full-rate ground,
plume, and composite shading, 2x2 sky/cloud shading, and eight IBL samples.
Cinematic shades all of those passes at full rate and uses sixteen IBL samples.
Performance retains the previous 80% scene dimensions and coarse shading rates.
The explicit `EXPLORA_IBL_SAMPLES` override still takes priority.

## Clouds and sunlight

`cloud_weather.inc` defines one density field for visible cumulus and the ground
shadow calculation. Its horizontal noise uses the existing split world-origin
anchor, while vertical samples use altitude above the ground. Moving the
floating origin therefore preserves cloud positions and shadow alignment.

The cumulus view march uses eight cubic distance intervals, concentrating
samples near the camera. Each interval uses its own length in exponential
extinction. The first sample of a 45 km horizontal ray is about 44 m away.
A short sun probe gives occupied samples some self-shadowing. Cirrus uses two
view samples, and layers are composited in distance order with premultiplied
alpha. Distant radiance receives atmospheric haze weighted by opacity.

Ground sunlight visibility uses three strata through the same cumulus volume.
It attenuates direct diffuse and specular light while leaving the ambient sky
contribution intact. These bounded approximations can undersample distant or
thin clouds; they are not a converged volumetric solution or a cloud shadow map.
The new density and light samples cost more GPU work than the earlier clouds.

## Wind

`EXPLORA_WIND` defaults to 1, clamps to 0–3, and accepts 0 for calm conditions.
The CPU wind field is deterministic in world position and simulation time.
Its prevailing flow strengthens and turns with altitude, with smooth horizontal
gusts and vertical motion that fades toward the surface.

Flight forces, speed limits, and camera angle-of-attack buffet use air-relative
velocity. Position integrates world velocity, allowing the aircraft to drift.
Trail particles entrain toward the sampled wind. Large clouds use a constant
8.5 m/s X/Z prevailing flow multiplied by the same strength setting; they do
not reproduce every local CPU gust. Cloud shaders receive the strength in the
previously unused `cameraParams2.w` uniform slot, keeping the UBO size unchanged.

## Final image

The composite uses five neighborhood samples for conservative sharpening,
clamped to local color bounds. Those samples are reused for a thirteen-tap
local highlight filter, with one additional tap when motion streaking is active.
Soft highlight extraction and a luminance cap keep glare restrained. The
filter's radius is about four source pixels; it is not broad multiscale bloom.
Tone mapping stays in linear light and the sRGB swapchain applies the display
transfer function. Sensor grain is reduced to retain distant detail.

## Acceptance on the target GPU

CPU tests and offline shader compilation run in the Linux verification
container described in `tools/README.md`. Actual appearance and performance
must also be assessed on the supported Linux/NVIDIA machine:

- Compare balanced and cinematic at the same resolution, pose, and wind setting.
- Use `EXPLORA_FREEZE=1` and screenshot helpers for fixed-pose comparisons.
- Fly below, through, and above both cloud decks; watch nearby cloud coverage,
  aircraft occlusion, ground shadow motion, and transparency ordering.
- Fly across world-origin changes and resize the window; weather should remain
  anchored and scene resolution should retain its selected preset.
- Check sun/exhaust highlights, dark silhouettes, and horizon detail for halos.
- Benchmark each preset separately; previous frame-rate measurements do not
  establish the performance of this update.
