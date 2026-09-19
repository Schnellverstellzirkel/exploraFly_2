#version 450

// Note: sky_atmo.inc injected here by build.rs

layout(set = 0, binding = 0) uniform UBO {
    mat4 viewProj;
    mat4 invViewProj;
    mat4 nodes[23];
    vec4 flex;         // x: bend, y: time, z: pressure, w: glow
    vec4 campos;       // xyz: camera pos, w: exit radius
    vec4 sunDir;       // xyz: dir, w: radius
    vec4 sunColor;     // xyz: irr, w: elevation
    vec4 skyZenith;    // xyz: zenith, w: cos_radius
    vec4 skyHorizon;   // xyz: horizon, w: inv_one_minus_cos_radius
    vec4 groundBase;   // xyz: ground albedo, w: relative ground height
    vec4 detail;       // x: plume_flicker, y: lambda, z: spool, w: plume_length
} ubo;

layout(location = 0) in vec3 vRay;
layout(location = 0) out vec4 outColor;

// Constants for cloud layers
const float CUMULUS_BOTTOM = 2600.0;
const float CUMULUS_TOP = 5600.0;
const float CIRRUS_BOTTOM = 9500.0;
const float CIRRUS_TOP = 11500.0;

// Number of marching steps
const int CUMULUS_STEPS = 4;
const int CIRRUS_STEPS = 2;

// Distance limits for slab marching
const float CUMULUS_MAX_DIST = 45000.0;
const float CIRRUS_MAX_DIST = 85000.0;

// Henyey-Greenstein phase function (Henyey & Greenstein 1941)
float henyeyGreenstein(float cosTheta, float g) {
    float g2 = g * g;
    float den = max(1.0 + g2 - 2.0 * g * cosTheta, 1e-4);
    return (1.0 - g2) / (4.0 * ATMO_PI * pow(den, 1.5));
}

// Dual-lobe Henyey-Greenstein phase function for cloud water droplets
// (Bouthors et al. 2008, Wrenninge 2012, Schneider 2015).
// Water droplets (10-20 um) exhibit a pronounced forward diffraction peak (silver lining)
// and a retroreflection backscatter peak (cloud glory / opposition surge).
float cloudPhase(float cosTheta) {
    float fwd = henyeyGreenstein(cosTheta, 0.82);
    float bwd = henyeyGreenstein(cosTheta, -0.22);
    return 0.78 * fwd + 0.22 * bwd;
}

float getCumulusDensity(vec3 pos) {
    float h = pos.y - ubo.groundBase.w;
    
    // 2D horizontal undulating base offset
    vec3 baseCoord = vec3(pos.x + ubo.flex.y * 5.0, 0.0, pos.z + ubo.flex.y * 5.0);
    float offset = atmoValueNoise(baseCoord, 16000.0, 200u) * 600.0 - 300.0;
    
    float bottom = CUMULUS_BOTTOM + offset;
    float top = CUMULUS_TOP + offset;
    
    if (h < bottom || h > top) return 0.0;
    
    float heightFraction = (h - bottom) / (top - bottom);
    
    // Cumulus vertical profile:
    // Crisp condensation base, billowing body, dome top
    float baseFade = smoothstep(0.0, 0.07, heightFraction);
    float topFade = smoothstep(1.0, 0.68, heightFraction);
    float profile = baseFade * topFade;
    if (profile <= 0.001) return 0.0;
    
    // Wind advection
    vec3 windPos = pos + vec3(ubo.flex.y * 10.0, 0.0, ubo.flex.y * 15.0);
    
    // Anisotropic coordinate scaling:
    // Scale Y by 1.15 to preserve towering dome billows
    vec3 shapePos = vec3(windPos.x, windPos.y * 1.15, windPos.z);
    
    // Coverage curve:
    // Defined condensation base, wide billowing mid-section, narrowing dome top
    float coverage = 0.55 + 0.22 * smoothstep(0.35, 0.92, heightFraction)
                          + 0.10 * (1.0 - smoothstep(0.0, 0.14, heightFraction));
    
    // Coarse rejection using base octave (cell = 4000m)
    float baseOct = atmoValueNoise(shapePos, 4000.0, 42u);
    if (0.65 * baseOct + 0.35 < coverage) return 0.0;
    
    // 2 octaves for billows and surface cauliflower erosion
    float oct1 = atmoValueNoise(shapePos * 2.0, 4000.0, 59u);
    float noise = 0.65 * baseOct + 0.35 * oct1;
    if (noise < coverage) return 0.0;
    
    float density = (noise - coverage) / (1.0 - coverage);
    
    // Soft onset for natural billow transitions
    density = density * density * (3.0 - 2.0 * density);
    
    return density * profile;
}

float getCirrusDensity(vec3 pos) {
    float h = pos.y - ubo.groundBase.w;
    
    if (h < CIRRUS_BOTTOM || h > CIRRUS_TOP) return 0.0;
    
    // Vertical fade
    float heightFraction = (h - CIRRUS_BOTTOM) / (CIRRUS_TOP - CIRRUS_BOTTOM);
    float profile = smoothstep(0.0, 0.20, heightFraction) * smoothstep(1.0, 0.80, heightFraction);
    
    // Single 8000m octave with vertical scaling for thin wisps
    vec3 windPos = pos + vec3(ubo.flex.y * 20.0, 0.0, 0.0);
    vec3 shapePos = vec3(windPos.x, windPos.y * 1.5, windPos.z);
    float noise = atmoValueNoise(shapePos, 8000.0, 71u);
    
    float coverage = 0.68;
    if (noise < coverage) return 0.0;
    float density = (noise - coverage) / (1.0 - coverage);
    
    // Very thin wisps
    return density * profile * 0.12;
}

// Ray-slab intersection (flat earth approximation)
bool intersectSlab(float y, float dirY, float bottom, float top, out float tMin, out float tMax) {
    if (abs(dirY) < 1e-6) {
        if (y >= bottom && y <= top) {
            tMin = 0.0;
            tMax = 100000.0;
            return true;
        }
        return false;
    }
    
    float t1 = (bottom - y) / dirY;
    float t2 = (top - y) / dirY;
    
    tMin = min(t1, t2);
    tMax = max(t1, t2);
    
    tMin = max(0.0, tMin);
    if (tMin > tMax || tMax < 0.0) return false;
    
    return true;
}

void main() {
    vec3 dir = normalize(vRay);
    vec3 pos = ubo.campos.xyz;
    float h = pos.y - ubo.groundBase.w;
    
    // Below cloud deck: any ray pointing horizontal or downward cannot hit clouds
    if (h < CUMULUS_BOTTOM - 300.0 && dir.y <= 0.0001) {
        discard;
    }
    
    // Intersect layers
    float cumulusTMin, cumulusTMax;
    bool hitCumulus = intersectSlab(pos.y, dir.y,
                                    ubo.groundBase.w + CUMULUS_BOTTOM - 300.0,
                                    ubo.groundBase.w + CUMULUS_TOP + 300.0,
                                    cumulusTMin, cumulusTMax);
    
    float cirrusTMin, cirrusTMax;
    bool hitCirrus = intersectSlab(pos.y, dir.y,
                                   ubo.groundBase.w + CIRRUS_BOTTOM,
                                   ubo.groundBase.w + CIRRUS_TOP,
                                   cirrusTMin, cirrusTMax);
    
    if (hitCumulus) {
        if (cumulusTMin >= CUMULUS_MAX_DIST) {
            hitCumulus = false;
        } else {
            cumulusTMax = min(cumulusTMax, min(cumulusTMin + 35000.0, CUMULUS_MAX_DIST));
            if (cumulusTMax <= cumulusTMin) hitCumulus = false;
        }
    }
    
    if (hitCirrus) {
        if (cirrusTMin >= CIRRUS_MAX_DIST) {
            hitCirrus = false;
        } else {
            cirrusTMax = min(cirrusTMax, min(cirrusTMin + 50000.0, CIRRUS_MAX_DIST));
            if (cirrusTMax <= cirrusTMin) hitCirrus = false;
        }
    }
    
    if (!hitCumulus && !hitCirrus) {
        discard;
    }
    
    float jitter = 0.5;
    
    float transmittance = 1.0;
    vec3 scatterColor = vec3(0.0);
    float firstHitT = -1.0;
    
    // -----------------------------------------------------------------------
    // Incident Radiance and Irradiance Fields Reaching Clouds (Radiative Transfer)
    // -----------------------------------------------------------------------
    vec3 sunDirNorm = normalize(ubo.sunDir.xyz);
    float cosTheta = dot(dir, sunDirNorm);
    
    // Cloud droplet scattering phase octaves (Wrenninge 2012, Schneider 2015)
    float p0 = cloudPhase(cosTheta);
    float p1 = henyeyGreenstein(cosTheta, 0.35); // 1st multi-scatter octave (broad forward)
    
    // 1. Direct Solar Irradiance reaching cloud deck altitudes:
    vec3 midCumulus = vec3(0.0, ATMO_GROUND_R + 4000.0, 0.0);
    vec3 sunTrCumulus = exp(-atmoSunOpticalDepth(midCumulus, sunDirNorm));
    vec3 sunIrrCumulus = ubo.sunColor.rgb * sunTrCumulus;
    
    vec3 midCirrus = vec3(0.0, ATMO_GROUND_R + 10500.0, 0.0);
    vec3 sunTrCirrus = exp(-atmoSunOpticalDepth(midCirrus, sunDirNorm));
    vec3 sunIrrCirrus = ubo.sunColor.rgb * sunTrCirrus;
    
    // 2. Downwelling Diffuse Skylight reaching cloud tops (upper hemisphere integral):
    vec3 L_sky_zenith = ubo.skyZenith.rgb;
    vec3 L_sky_horizon = ubo.skyHorizon.rgb;
    vec3 L_sky_down = (2.0 / 3.0) * L_sky_zenith + (1.0 / 3.0) * L_sky_horizon;
    
    // 3. Upwelling Ground-Reflected Light reaching cloud bases (Earthshine):
    vec3 groundAtmo = vec3(0.0, ATMO_GROUND_R, 0.0);
    vec3 sunTrGround = exp(-atmoSunOpticalDepth(groundAtmo, sunDirNorm));
    vec3 E_sun_ground = ubo.sunColor.rgb * sunTrGround * max(sunDirNorm.y, 0.0);
    vec3 E_sky_ground = ATMO_PI * ((1.0 / 3.0) * L_sky_zenith + (2.0 / 3.0) * L_sky_horizon);
    vec3 groundAlbedo = max(ubo.groundBase.rgb * 2.2, vec3(0.08, 0.11, 0.06));
    vec3 L_ground_up = groundAlbedo * (E_sun_ground + E_sky_ground) * (0.90 / ATMO_PI);
    
    // Cumulus marching
    if (hitCumulus) {
        float marchDist = cumulusTMax - cumulusTMin;
        float stepSize = marchDist / float(CUMULUS_STEPS);
        float t = cumulusTMin + stepSize * jitter;
        
        for (int i = 0; i < CUMULUS_STEPS; ++i) {
            vec3 samplePos = pos + dir * t;
            
            // Seamless distance fade near max range so clouds don't pop when moving
            float distFade = 1.0 - smoothstep(32000.0, 44000.0, t);
            float rawDensity = getCumulusDensity(samplePos);
            float density = rawDensity * distFade;
            
            if (density > 0.005) {
                if (firstHitT < 0.0) firstHitT = t;
                
                float extinction = density * 0.0030;
                float sampleTransmittance = exp(-extinction * stepSize);
                
                // Height in cloud layer for vertical ambient coupling
                float hSample = samplePos.y - ubo.groundBase.w;
                float hNorm = clamp((hSample - CUMULUS_BOTTOM) / (CUMULUS_TOP - CUMULUS_BOTTOM), 0.0, 1.0);
                
                // Ambient lighting reaching this parcel:
                // Cloud tops receive downwelling skylight; cloud bottoms receive upwelling earthshine.
                vec3 L_ambient_in = mix(L_ground_up, L_sky_down, smoothstep(0.0, 0.85, hNorm));
                vec3 L_ambient = L_ambient_in * (0.55 + 0.45 * (1.0 - density * 0.5));
                
                // Direct sunlight reaching this parcel:
                // Evaluates in-cloud optical depth toward the sun for volumetric self-shadowing
                vec3 lightPos = samplePos + sunDirNorm * 450.0;
                float lightDensity = getCumulusDensity(lightPos) * distFade;
                float sunCloudTau = lightDensity * (0.0035 * 600.0);
                
                // Multiple scattering & single scattering:
                // In water droplet clouds (single-scattering albedo ~0.9999), multiple scattering
                // ensures high diffuse albedo (~0.80) so clouds appear bright white, not gray.
                // Outer edges face unattenuated sunlight and stay bright and translucent without dark outlines.
                float t0 = exp(-sunCloudTau);
                float directSingle = t0 * p0 * 1.2;
                
                float t1 = exp(-sunCloudTau * 0.25);
                float directMulti = t1 * (p1 * 0.8 + 0.36);
                
                vec3 L_direct = sunIrrCumulus * (directSingle + directMulti);
                vec3 L_source = L_direct + L_ambient;
                
                scatterColor += L_source * (1.0 - sampleTransmittance) * transmittance;
                transmittance *= sampleTransmittance;
                
                if (transmittance < 0.01) break;
            }
            t += stepSize;
        }
    }
    
    // Cirrus marching
    if (hitCirrus && transmittance > 0.01) {
        float marchDist = cirrusTMax - cirrusTMin;
        float stepSize = marchDist / float(CIRRUS_STEPS);
        float t = cirrusTMin + stepSize * jitter;
        
        for (int i = 0; i < CIRRUS_STEPS; ++i) {
            vec3 samplePos = pos + dir * t;
            float distFade = 1.0 - smoothstep(60000.0, 84000.0, t);
            float density = getCirrusDensity(samplePos) * distFade;
            
            if (density > 0.001) {
                if (firstHitT < 0.0) firstHitT = t;
                
                float extinction = density * 0.002;
                float sampleTransmittance = exp(-extinction * stepSize);
                
                // Cirrus ice crystal phase function: forward diffraction + diffuse lateral scatter
                float pIce = 0.85 * henyeyGreenstein(cosTheta, 0.82) + 0.15 * (1.0 / (4.0 * ATMO_PI));
                vec3 L_direct = sunIrrCirrus * (pIce * 1.5 + 0.35);
                
                // Ambient reaching high-altitude cirrus (10.5 km)
                vec3 L_ambient = (L_sky_down * 0.70 + L_ground_up * 0.30);
                vec3 L_source = L_direct + L_ambient;
                
                scatterColor += L_source * (1.0 - sampleTransmittance) * transmittance;
                transmittance *= sampleTransmittance;
                
                if (transmittance < 0.01) break;
            }
            t += stepSize;
        }
    }
    
    float alpha = 1.0 - transmittance;
    if (alpha < 0.01) {
        discard;
    }
    
    // Physical atmospheric aerial perspective between camera and cloud
    // (Kokhanovsky 2006, Hillaire 2016, 2020)
    if (firstHitT > 0.0) {
        vec3 atmo_origin = atmoModelOrigin(ubo.campos.xyz, ubo.groundBase.w);
        vec3 trSunCam = exp(-atmoSunOpticalDepth(atmo_origin, sunDirNorm));
        vec3 haze = atmoRadianceCheapTr(atmo_origin, dir, sunDirNorm, ubo.sunColor.rgb, trSunCam);
        
        float cam_h = max(ubo.campos.y - ubo.groundBase.w, 0.0);
        float dR = exp(-cam_h / 8000.0);
        float dM = exp(-cam_h / 1200.0);
        vec3 ext = ATMO_BETA_RAYLEIGH * dR + ATMO_BETA_MIE_EXTINCT * dM;
        vec3 atmoTrans = exp(-firstHitT * ext);
        
        // Premultiplied alpha blend: attenuate cloud radiance by intervening atmosphere,
        // and add in-scattered atmospheric haze.
        scatterColor = scatterColor * atmoTrans + haze * (vec3(1.0) - atmoTrans) * alpha;
    }
    
    // Calculate gl_FragDepth safely within [0.0, 0.999999]
    float depthT = (firstHitT > 0.0) ? firstHitT : cumulusTMin;
    vec4 clipPos = ubo.viewProj * vec4(pos + dir * max(depthT, 0.1), 1.0);
    if (clipPos.w <= 0.0) {
        gl_FragDepth = 0.999999;
    } else {
        gl_FragDepth = clamp(clipPos.z / clipPos.w, 0.0, 0.999999);
    }
    
    outColor = vec4(scatterColor, alpha);
}
