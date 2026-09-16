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
// Model (Hillaire-inspired, no precomputed LUTs):
//   - single scattering marched along the view ray with geometrically spaced
//     steps (exponential fall-offs become uniform in log space, so near-
//     horizon grazing rays stay artifact free without a heavy inner march);
//   - sun optical depth in closed form as straight-chord exponential
//     integrals per species (Rayleigh + Mie + ozone), no inner light march;
//   - cheap isotropic multiple-scattering fill that thickens with the total
//     extinction crossed and damps where the aerosol single-scatter already
//     dominates;
//   - photometric solar disc with limb darkening and fractional edge AA;
//   - tiled value-noise + FBM for clouds, every octave sharing one exact
//     8192 km repetition period so the deck never shows a seam.

// ---------------------------------------------------------------------------
// Physical constants
// ---------------------------------------------------------------------------

const float ATMO_PI         = 3.141592653589793;
const float ATMO_TWO_PI     = 6.283185307179586;
const float ATMO_FOUR_PI    = 12.566370614359172;

// Scattering/extinction coefficients per metre per unit of relative density,
// evaluated at sea level (density = 1 at h = 0).
const vec3  ATMO_BETA_RAYLEIGH = vec3(5.802e-6, 13.558e-6, 33.1e-6);
const vec3  ATMO_BETA_MIE_SCATTER = vec3(1.8e-6);
const vec3  ATMO_BETA_MIE_ABSORB  = vec3(2.0e-6);
const vec3  ATMO_BETA_MIE_EXTINCT = ATMO_BETA_MIE_SCATTER + ATMO_BETA_MIE_ABSORB;
const vec3  ATMO_BETA_OZONE       = vec3(0.650e-6, 2.050e-6, 0.085e-6);
const vec3  ATMO_BETA_EXTINCT = ATMO_BETA_RAYLEIGH + ATMO_BETA_MIE_EXTINCT
                              + ATMO_BETA_OZONE;

// Density scale heights (metres). Ozone is a stratospheric shell centred on
// ATMO_H_OZONE with a Gaussian half-width ATMO_OZONE_SIGMA.
const float ATMO_H_RAYLEIGH  = 8000.0;
const float ATMO_H_MIE       = 1200.0;
const float ATMO_H_OZONE     = 25000.0;
const float ATMO_OZONE_SIGMA = 15000.0;

// Planet geometry.
const float ATMO_GROUND_R = 6360000.0;
const float ATMO_TOP_R    = 6460000.0; // +100 km shell

// Marching cost. The view-integration step count is the only real loop; the
// sun depth is closed form so it costs nothing per step.
const int ATMO_VIEW_STEPS = 3;

// Single-scattering phase asymmetry for the aerosol forward lobe.
const float ATMO_G_MIE = 0.76;

// Multiple-scattering fill (Hillaire-flavoured isotropic approximation).
const vec3  ATMO_MS_COLOR       = vec3(0.18, 0.38, 0.85);
const float ATMO_MS_STRENGTH    = 0.35;
const float ATMO_MS_DAMP        = 2.2; // aerosol opacity at which fill halves

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
    float u = (h - ATMO_H_OZONE) / ATMO_OZONE_SIGMA;
    return exp(-u * u);
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

// Closed-form optical depth from p toward sunDir to the atmosphere boundary.
// For an exponential layer of scale height H along a straight chord of length
// L the optical depth sum is
//     tau(h, L) = beta * dens(h) * H * (1 - exp(-L / H)).
// Integrating the true exponential fall-off over the geometric ray gives
// exactly that for each species; the ozone shell uses its own short scale.
// Unequal scale heights stay independent, so the three add cleanly.
vec3 atmoSunOpticalDepth(vec3 p, vec3 sunDir) {
    float h = max(length(p) - ATMO_GROUND_R, 0.0);
    float L;
    float groundHit = atmoRaySphere(p, sunDir, ATMO_GROUND_R).x;
    if (groundHit > 0.0) {
        L = groundHit; // the light path ends on the solid planet
    } else {
        L = max(atmoRaySphere(p, sunDir, ATMO_TOP_R).y, 0.0);
    }
    if (L <= 0.0) {
        return vec3(0.0);
    }
    float dR = atmoRayleighDensity(h);
    float dM = atmoMieDensity(h);
    float dO = atmoOzoneDensity(h);
    return
        ATMO_BETA_RAYLEIGH * dR * ATMO_H_RAYLEIGH * (1.0 - exp(-L / ATMO_H_RAYLEIGH))
      + ATMO_BETA_MIE_EXTINCT * dM * ATMO_H_MIE * (1.0 - exp(-L / ATMO_H_MIE))
      + ATMO_BETA_OZONE * dO * ATMO_OZONE_SIGMA * (1.0 - exp(-L / ATMO_OZONE_SIGMA));
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

    // Multiple-scatter fill, scaled with the same total extinction.
    vec3  msTau = clamp(tau * 0.65, vec3(0.0), vec3(8.0));
    L += sunIrr * ATMO_MS_COLOR * ATMO_MS_STRENGTH * (1.0 - exp(-msTau));
    return L;
}

vec3 atmoRadianceCheap(vec3 origin, vec3 dir, vec3 sunDir, vec3 sunIrr) {
    vec3 trSun = exp(-atmoSunOpticalDepth(origin, sunDir));
    return atmoRadianceCheapTr(origin, dir, sunDir, sunIrr, trSun);
}

// ---------------------------------------------------------------------------
// Full single-scattered sky by view-array marching.
//
// Steps are geometrically spaced from near the camera out to the far shell
// (or the ground crossing). Optical depths accumulate exactly; sun depth is
// closed form per step; a mid-slab transmittance term avoids quadrature
// staircase on the tall near-horizon slabs.
vec3 atmoIntegrate(vec3 origin, vec3 viewDir, vec3 sunDir, vec3 sunIrr) {
    vec2 topHit = atmoRaySphere(origin, viewDir, ATMO_TOP_R);
    float tTop  = topHit.y;
    if (tTop <= 0.0) {
        return vec3(0.0); // already outside the shell, looking away
    }
    float tStart = max(topHit.x, 0.0);
    float tGround = atmoRaySphere(origin, viewDir, ATMO_GROUND_R).x;
    float tEnd  = (tGround > 0.0) ? min(tGround, tTop) : tTop;
    if (tEnd <= tStart) {
        return vec3(0.0);
    }
    tStart = max(tStart, 1.0);

    float cosGamma = dot(viewDir, sunDir);
    float ratio = exp((log(tEnd) - log(tStart)) / float(ATMO_VIEW_STEPS));
    if (!(ratio > 0.0) || !(ratio < 1e6)) {
        return vec3(0.0); // degenerate guard
    }

    vec3  insctr = vec3(0.0);
    vec3  odPath = vec3(0.0);
    float mieTau = 0.0;
    vec3  trAtm  = vec3(0.0);
    float tA     = tStart;
    for (int i = 0; i < ATMO_VIEW_STEPS; ++i) {
        float tB = tA * ratio;
        float t  = sqrt(tA * tB); // logarithmic midpoint of the slab
        float dl = tB - tA;
        tA       = tB;

        vec3  p  = origin + viewDir * t;
        float h  = max(length(p) - ATMO_GROUND_R, 0.0);

        float dR = atmoRayleighDensity(h);
        float dM = atmoMieDensity(h);
        float dO = atmoOzoneDensity(h);

        vec3   ext = ATMO_BETA_RAYLEIGH * dR
                   + ATMO_BETA_MIE_EXTINCT * dM
                   + ATMO_BETA_OZONE * dO;
        odPath += ext * dl;
        mieTau += (ATMO_BETA_MIE_EXTINCT.x * dM) * dl;

        vec3 trCam = exp(-odPath);
        vec3 trSun = exp(-atmoSunOpticalDepth(p, sunDir));

        vec3 scatterPerM =
            ATMO_BETA_RAYLEIGH * dR * atmoRayleighPhase(cosGamma)
          + ATMO_BETA_MIE_SCATTER * dM * atmoMiePhase(cosGamma);

        vec3 trMid = exp(-(odPath + ext * (dl * 0.5)));
        insctr += trMid * trSun * scatterPerM * sunIrr * dl;
    }
    trAtm = exp(-odPath);

    // Multiple-scatter fill: proportional to the extinction crossed, damped
    // where the forward aerosol lobe already carries most of the light.
    float damp = exp(-max(mieTau - 0.15, 0.0) * ATMO_MS_DAMP);
    vec3  msFill = sunIrr * ATMO_MS_COLOR * ATMO_MS_STRENGTH
                 * (1.0 - trAtm) * (0.35 + 0.65 * damp);
    return insctr + msFill;
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
