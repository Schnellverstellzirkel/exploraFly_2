// sky_atmo.inc
// ---------------------------------------------------------------------------
// Shared physical sky / atmosphere / cloud library, injected by build.rs
// immediately after the #version line of sky.frag, clouds.frag and
// ground.frag. Pure functions only: no uniform block references, so the same
// text compiles in any shader regardless of its UBO subset.
//
// Units: meters, linear radiometric intensities. The planet is a sphere of
// radius ATMO_GROUND_R centred on the origin of the coordinate space that the
// caller builds with atmoModelOrigin(). All view/sun directions are unit
// vectors in that same space (pure rotation, so world-space directions work
// unchanged).
//
// Model (Hillaire 2020):
//   - reference Rayleigh/Mie/ozone density profiles and coefficients;
//   - precomputed transmittance and multiple-scattering LUTs, avoiding a
//     second light march for every view sample;
//   - throughput/segment integration along the view ray, with the source
//     term attenuated by the current segment transmittance;
//   - photometric solar disc with limb darkening and fractional edge AA;
//   - tiled value-noise + FBM for clouds, every octave sharing one exact
//     8192 km repetition period so the deck never shows a seam.

// ---------------------------------------------------------------------------
// Physical constants
// ---------------------------------------------------------------------------

const float ATMO_PI         = 3.141592653589793;
const float ATMO_TWO_PI     = 6.283185307179586;
const float ATMO_FOUR_PI    = 12.566370614359172;

// The CPU builds these with the same parameterization used by Hillaire's
// reference transmittance and multiple-scattering passes.
layout(set = 0, binding = 6) uniform texture2D atmo_transmittance_tex;
layout(set = 0, binding = 7) uniform sampler atmo_transmittance_smp;
layout(set = 0, binding = 8) uniform texture2D atmo_multiscattering_tex;
layout(set = 0, binding = 9) uniform sampler atmo_multiscattering_smp;

// Scattering/extinction coefficients per metre per unit of relative density,
// evaluated at sea level (density = 1 at h = 0).
const vec3  ATMO_BETA_RAYLEIGH = vec3(5.802e-6, 13.558e-6, 33.1e-6);
const vec3  ATMO_BETA_MIE_SCATTER = vec3(3.996e-6);
const vec3  ATMO_BETA_MIE_ABSORB  = vec3(0.444e-6);
const vec3  ATMO_BETA_MIE_EXTINCT = ATMO_BETA_MIE_SCATTER + ATMO_BETA_MIE_ABSORB;
const vec3  ATMO_BETA_OZONE       = vec3(0.650e-6, 1.881e-6, 0.085e-6);
const vec3  ATMO_BETA_EXTINCT = ATMO_BETA_RAYLEIGH + ATMO_BETA_MIE_EXTINCT
                              + ATMO_BETA_OZONE;

// Density scale heights (metres). Ozone is a stratospheric shell centred on
// Hillaire's ozone tent spans 10--40 km and peaks at 25 km.
const float ATMO_H_RAYLEIGH  = 8000.0;
const float ATMO_H_MIE       = 1200.0;

// Planet geometry.
const float ATMO_GROUND_R = 6360000.0;
const float ATMO_TOP_R    = 6460000.0; // +100 km shell

// Hillaire's LUT-backed view integration. The light path is a texture lookup,
// so this is substantially cheaper than nesting a second ray march here.
const int ATMO_VIEW_STEPS = 30;

// Single-scattering phase asymmetry for the aerosol forward lobe.
const float ATMO_G_MIE = 0.8;

// Hillaire's reference implementation uses normalized solar irradiance.  The
// engine's ACES input is an HDR scene unit, so this is the explicit conversion
// between those two radiometric conventions.  It scales atmospheric sky
// transport only; the finite solar disc remains normalized by its solid angle.
const float ATMO_HDR_RADIANCE_SCALE = 64.0;

// Solar disc. The mean solar half-angle is about 0.266 degrees at 1 AU.
// The radiance is normalized by the exact solid angle of that finite source.
const float ATMO_SUN_RADIUS_RAD = 0.00465;
const float ATMO_SUN_COS_RADIUS = 1.0 - ATMO_SUN_RADIUS_RAD * ATMO_SUN_RADIUS_RAD * 0.5;
const float ATMO_SUN_INV_SOLID  = 1.0 / (2.0 * ATMO_PI * (1.0 - ATMO_SUN_COS_RADIUS));

// ---------------------------------------------------------------------------
// Density and phase helpers
// ---------------------------------------------------------------------------

float atmoRayleighDensity(float h) { return exp(-h / ATMO_H_RAYLEIGH); }
float atmoMieDensity(float h)      { return exp(-h / ATMO_H_MIE); }
float atmoOzoneDensity(float h) {
    float density = (h < 25000.0)
        ? h / 15000.0 - 2.0 / 3.0
        : -h / 15000.0 + 8.0 / 3.0;
    return clamp(density, 0.0, 1.0);
}

float atmoRayleighPhase(float cosTheta) {
    float c = cosTheta * cosTheta;
    return (3.0 / (16.0 * ATMO_PI)) * (1.0 + c);
}

// Cornette-Shanks Mie phase, properly normalised over the sphere.
float atmoMiePhase(float cosTheta) {
    float g   = ATMO_G_MIE;
    float g2  = g * g;
    float den = max(1.0 + g2 - 2.0 * g * cosTheta, 1e-4);
    return (3.0 * (1.0 - g2) * (1.0 + cosTheta * cosTheta))
         / (8.0 * ATMO_PI * (2.0 + g2) * pow(den, 1.5));
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

// First/forward crossing x and exit crossing y of a ray against a sphere
// centred on the origin. Both are -1 when the ray misses.
vec2 atmoRaySphere(vec3 o, vec3 d, float radius) {
    float b     = dot(o, d);
    float c     = dot(o, o) - radius * radius;
    float disc  = b * b - c;
    if (disc < 0.0) {
        return vec2(-1.0);
    }
    float s = sqrt(disc);
    return vec2(-b - s, -b + s);
}

// Camera origin in the atmo model frame: planet centred at (0,0,0), camera on
// the vertical axis at ATMO_GROUND_R + altitude above the local ground plane.
vec3 atmoModelOrigin(vec3 campos, float groundBase) {
    return vec3(0.0, ATMO_GROUND_R + max(campos.y - groundBase, 0.0), 0.0);
}

// Hillaire transmittance-LUT parameterization.  The first coordinate is the
// distance to the top boundary normalized between the tangent and radial
// cases; the second is the altitude parameter rho/H.
vec2 atmoTransmittanceUv(float radius, float mu) {
    float H = sqrt(max(ATMO_TOP_R * ATMO_TOP_R - ATMO_GROUND_R * ATMO_GROUND_R, 0.0));
    float rho = sqrt(max(radius * radius - ATMO_GROUND_R * ATMO_GROUND_R, 0.0));
    float discriminant = radius * radius * (mu * mu - 1.0) + ATMO_TOP_R * ATMO_TOP_R;
    float d = max(-radius * mu + sqrt(max(discriminant, 0.0)), 0.0);
    float dMin = ATMO_TOP_R - radius;
    float dMax = rho + H;
    return clamp(vec2(
        (d - dMin) / max(dMax - dMin, 1e-6),
        rho / max(H, 1e-6)), vec2(0.0), vec2(1.0));
}

vec3 atmoTransmittanceToTop(vec3 p, vec3 direction) {
    float radius = length(p);
    float mu = clamp(dot(direction, p / max(radius, 1e-6)), -1.0, 1.0);
    vec2 uv = atmoTransmittanceUv(radius, mu);
    return textureLod(sampler2D(atmo_transmittance_tex, atmo_transmittance_smp), uv, 0.0).rgb;
}

vec3 atmoMultipleScattering(vec3 p, float sunZenithCos) {
    vec2 uv = clamp(vec2(
        sunZenithCos * 0.5 + 0.5,
        (length(p) - ATMO_GROUND_R) / max(ATMO_TOP_R - ATMO_GROUND_R, 1e-6)),
        vec2(0.0), vec2(1.0));
    const float resolution = 32.0;
    uv = (uv + 0.5 / resolution) * (resolution / (resolution + 1.0));
    return textureLod(sampler2D(atmo_multiscattering_tex, atmo_multiscattering_smp), uv, 0.0).rgb;
}

// Kept as an optical-depth API for ground/cloud callers.  Its value now comes
// from the same curved-path LUT used by the Hillaire view integrator.
vec3 atmoSunOpticalDepth(vec3 p, vec3 sunDir) {
    return -log(max(atmoTransmittanceToTop(p, sunDir), vec3(1e-6)));
}

float atmoEarthShadow(vec3 p, vec3 sunDir) {
    return atmoRaySphere(p, sunDir, ATMO_GROUND_R).x >= 0.0 ? 0.0 : 1.0;
}

// Effective chord length to the atmosphere boundary (or the solid ground,
// whichever comes first) along a view ray, used by the cheap one-slab
// atmosphere below.
float atmoChordLength(vec3 origin, vec3 dir) {
    float groundHit = atmoRaySphere(origin, dir, ATMO_GROUND_R).x;
    if (groundHit > 0.0) {
        return groundHit;
    }
    return max(atmoRaySphere(origin, dir, ATMO_TOP_R).y, 0.0);
}

// ---------------------------------------------------------------------------
// Cheap ground/cloud atmosphere: uniform-slab single scatter in closed form.
// A single slab of effective thickness equal to the geometric chord whose
// density matches the sample altitude captures the colour and luminance
// structure of the sky (bluer and dimmer overhead, warm and bright near the
// horizon, Mie glow beside the sun) in a handful of exps, so it is safe to
// call per fragment on every ground surface pixel.
vec3 atmoRadianceCheapTr(vec3 origin, vec3 dir, vec3 sunDir, vec3 sunIrr, vec3 trSun) {
    float h  = max(length(origin) - ATMO_GROUND_R, 0.0);
    float Lv = atmoChordLength(origin, dir);
    if (Lv <= 0.0) {
        return vec3(0.0);
    }
    float dR = atmoRayleighDensity(h);
    float dM = atmoMieDensity(h);
    float dO = atmoOzoneDensity(h);
    vec3  extV = ATMO_BETA_RAYLEIGH * dR
               + ATMO_BETA_MIE_EXTINCT * dM
               + ATMO_BETA_OZONE * dO;
    vec3  tau = extV * Lv;
    if (tau.x + tau.y + tau.z > 30.0) {
        tau = vec3(30.0); // saturated path: keep anything beyond articulate
    }
    float mu = dot(dir, sunDir);
    vec3  scatterPerM =
        ATMO_BETA_RAYLEIGH * dR * atmoRayleighPhase(mu)
      + ATMO_BETA_MIE_SCATTER * dM * atmoMiePhase(mu);

    // Exact single-scatter integral of a uniform slab: (scatter/ext) * (1-e^-tau).
    vec3  L = sunIrr * trSun * (scatterPerM / max(extV, vec3(1e-7)))
            * (1.0 - exp(-tau));

    // Hillaire's higher-order contribution is isotropic and stored as a
    // transport factor.  Apply it to the local scattering medium instead of
    // injecting a hand-authored sky colour.
    vec3 up = normalize(origin);
    vec3 multiPerM = atmoMultipleScattering(origin, dot(sunDir, up))
                   * (ATMO_BETA_RAYLEIGH * dR + ATMO_BETA_MIE_SCATTER * dM);
    L += sunIrr * (multiPerM / max(extV, vec3(1e-7))) * (1.0 - exp(-tau));
    return L * ATMO_HDR_RADIANCE_SCALE;
}

vec3 atmoRadianceCheap(vec3 origin, vec3 dir, vec3 sunDir, vec3 sunIrr) {
    vec3 trSun = exp(-atmoSunOpticalDepth(origin, sunDir));
    return atmoRadianceCheapTr(origin, dir, sunDir, sunIrr, trSun);
}

// ---------------------------------------------------------------------------
// Full single-scattered sky by view-array marching.
//
// This is Hillaire's throughput/segment form.  Each segment contributes the
// analytically integrated source over its own transmittance, which avoids the
// broad hard fall-off produced by adding sample radiance at a single midpoint.
vec3 atmoIntegrate(vec3 origin, vec3 viewDir, vec3 sunDir, vec3 sunIrr) {
    vec2 topHit = atmoRaySphere(origin, viewDir, ATMO_TOP_R);
    float tTop = topHit.y;
    if (tTop <= 0.0) {
        return vec3(0.0); // already outside the shell, looking away
    }
    float tGround = atmoRaySphere(origin, viewDir, ATMO_GROUND_R).x;
    float tMax = (tGround > 0.0) ? min(tGround, tTop) : tTop;
    if (tMax <= 0.0) {
        return vec3(0.0);
    }

    float cosGamma = dot(viewDir, sunDir);
    float rayleighPhase = atmoRayleighPhase(cosGamma);
    float miePhase = atmoMiePhase(cosGamma);
    vec3 insctr = vec3(0.0);
    vec3 throughput = vec3(1.0);
    for (int i = 0; i < ATMO_VIEW_STEPS; ++i) {
        float s0 = float(i) / float(ATMO_VIEW_STEPS);
        float s1 = float(i + 1) / float(ATMO_VIEW_STEPS);
        float t0 = tMax * s0 * s0;
        float t1 = tMax * s1 * s1;
        float t = t0 + (t1 - t0) * 0.3;
        float dt = t1 - t0;
        vec3 p = origin + viewDir * t;
        float h = max(length(p) - ATMO_GROUND_R, 0.0);
        float dR = atmoRayleighDensity(h);
        float dM = atmoMieDensity(h);
        float dO = atmoOzoneDensity(h);
        vec3 scattering = ATMO_BETA_RAYLEIGH * dR + ATMO_BETA_MIE_SCATTER * dM;
        vec3 extinction = ATMO_BETA_RAYLEIGH * dR
                        + ATMO_BETA_MIE_EXTINCT * dM
                        + ATMO_BETA_OZONE * dO;
        vec3 sampleTransmittance = exp(-extinction * dt);
        vec3 up = p / max(length(p), 1e-6);
        vec3 transSun = atmoTransmittanceToTop(p, sunDir);
        float earthShadow = atmoEarthShadow(p, sunDir);
        vec3 phaseScattering = ATMO_BETA_RAYLEIGH * dR * rayleighPhase
                             + ATMO_BETA_MIE_SCATTER * dM * miePhase;
        vec3 multi = atmoMultipleScattering(p, dot(sunDir, up));
        vec3 source = earthShadow * transSun * phaseScattering + multi * scattering;
        source *= sunIrr;
        vec3 extinctionSafe = max(extinction, vec3(1e-4));
        vec3 segment = (source - source * sampleTransmittance) / extinctionSafe;
        insctr += throughput * segment;
        throughput *= sampleTransmittance;
    }
    return insctr;
}

// ---------------------------------------------------------------------------
// Solar disc
// ---------------------------------------------------------------------------

// Radiant disc as photographed: sunIrr / solid angle times the light
// transmittance from the camera to the sun, with limb darkening. The limb is
// anti-aliased from the actual fragment footprint rather than a fixed angular
// band, so it cannot turn into a second oversized sun.
vec3 atmoSunDisc(vec3 viewDir, vec3 sunDir, vec3 sunIrr, vec3 trSunCam) {
    float cosAngle = dot(viewDir, sunDir);
    float aaWidth = max(fwidth(cosAngle), 1e-6);
    if (cosAngle < ATMO_SUN_COS_RADIUS - aaWidth) {
        return vec3(0.0);
    }
    float rho2 = clamp((1.0 - cosAngle) * (1.0 / (1.0 - ATMO_SUN_COS_RADIUS)),
                       0.0, 1.0);
    float mu = sqrt(max(1.0 - rho2, 0.0));
    float om = 1.0 - mu;
    vec3  limb = vec3(1.0) - vec3(0.54, 0.63, 0.72) * om - vec3(0.18, 0.16, 0.14) * om * om;
    float edge = smoothstep(ATMO_SUN_COS_RADIUS - aaWidth,
                            ATMO_SUN_COS_RADIUS + aaWidth, cosAngle);
    return sunIrr * ATMO_SUN_INV_SOLID * trSunCam * limb * edge;
}

// ---------------------------------------------------------------------------
// Cloud noise
// ---------------------------------------------------------------------------
// Tiled value noise. Every octave's lattice wraps after exactly 8192 km of
// samples, so the chosen power-of-two cell sizes are always a whole number of
// cells per tile: 4000 m -> 2048, 2000 m -> 4096, 1000 m -> 8192 et cetera.

const float ATMO_CLOUD_TILE = 8192000.0; // world tile period for all octaves

uint atmoCloudHashU(uvec3 cell, uint seed) {
    uint h = cell.x * 1664525u + cell.y * 1013904223u + cell.z * 2246822519u
           + seed * 1299709u;
    h ^= h >> 16;
    h *= 2246822519u;
    h ^= h >> 13;
    h *= 3266489917u;
    return h ^ (h >> 16);
}

float atmoCloudHash(uvec3 cell, uint seed) {
    return float(atmoCloudHashU(cell, seed) & 0x00ffffffu) * (1.0 / 16777215.0);
}

// Seamless trilinear-interpolated value noise. cell = size of one lattice
// cell in metres; the lattice index is wrapped modulo the tile so the field
// repeats seamlessly every ATMO_CLOUD_TILE metres on every axis.
float atmoValueNoise(vec3 p, float cell, uint seed) {
    float period = ATMO_CLOUD_TILE / cell; // power of two by construction
    vec3  q   = p / cell;
    vec3  base = floor(q);
    vec3  f   = fract(q);
    vec3  w   = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);

    uint  pCell  = uint(period + 0.5);
    uint  pMask  = pCell - 1u;
    uvec3 c0 = uvec3(mod(base, period));
    uvec3 c1 = (c0 + 1u) & pMask;

    float n000 = atmoCloudHash(uvec3(c0.x, c0.y, c0.z), seed);
    float n100 = atmoCloudHash(uvec3(c1.x, c0.y, c0.z), seed);
    float n010 = atmoCloudHash(uvec3(c0.x, c1.y, c0.z), seed);
    float n110 = atmoCloudHash(uvec3(c1.x, c1.y, c0.z), seed);
    float n001 = atmoCloudHash(uvec3(c0.x, c0.y, c1.z), seed);
    float n101 = atmoCloudHash(uvec3(c1.x, c0.y, c1.z), seed);
    float n011 = atmoCloudHash(uvec3(c0.x, c1.y, c1.z), seed);
    float n111 = atmoCloudHash(uvec3(c1.x, c1.y, c1.z), seed);

    float n00 = mix(n000, n100, w.x);
    float n10 = mix(n010, n110, w.x);
    float n01 = mix(n001, n101, w.x);
    float n11 = mix(n011, n111, w.x);
    float n0  = mix(n00, n10, w.y);
    float n1  = mix(n01, n11, w.y);
    return mix(n0, n1, w.z);
}

// Fractal sum of value noise. Octaves halve their cell each level (so their
// lattices stay on the same power-of-two tile count) and halve their weight.
float atmoCloudFbm(vec3 p, float cell0, int octaves, uint seed) {
    float sum = 0.0;
    float norm = 0.0;
    float amp = 1.0;
    float cell = cell0;
    for (int i = 0; i < octaves; ++i) {
        sum += amp * atmoValueNoise(p, cell, seed + uint(i));
        norm += amp;
        amp *= 0.5;
        cell *= 0.5;
    }
    return sum / max(norm, 1e-6);
}
