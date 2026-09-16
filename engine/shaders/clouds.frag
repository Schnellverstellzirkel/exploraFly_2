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

// Number of marching steps (balanced with IGN stochastic jitter)
const int CUMULUS_STEPS = 3;
const int CIRRUS_STEPS = 2;

// Distance limits for slab marching
const float CUMULUS_MAX_DIST = 45000.0;
const float CIRRUS_MAX_DIST = 85000.0;


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
    float topFade = smoothstep(1.0, 0.65, heightFraction);
    float profile = baseFade * topFade;
    if (profile <= 0.001) return 0.0;
    
    // Wind advection
    vec3 windPos = pos + vec3(ubo.flex.y * 10.0, 0.0, ubo.flex.y * 15.0);
    
    // Anisotropic coordinate scaling:
    // Scale Y by 2.2 so vertical noise frequency matches the physical dimensions of cloud billows,
    // breaking the flat 2D cylinder extrusion.
    vec3 shapePos = vec3(windPos.x, windPos.y * 2.2, windPos.z);
    
    // Coverage curve:
    // Flat defined base, wide billowing mid-section, narrowing into cauliflower dome tops.
    float coverage = 0.56 + 0.24 * smoothstep(0.35, 0.92, heightFraction)
                          + 0.12 * (1.0 - smoothstep(0.0, 0.14, heightFraction));
    
    // Exact early coarse rejection using base octave (cell = 4000m):
    float baseOct = atmoValueNoise(shapePos, 4000.0, 42u);
    if (0.65 * baseOct + 0.35 < coverage) return 0.0;
    
    // 2 octaves for billows and surface cauliflower erosion
    float oct1 = atmoValueNoise(shapePos * 2.0, 4000.0, 59u);
    float noise = 0.65 * baseOct + 0.35 * oct1;
    if (noise < coverage) return 0.0;
    
    float density = (noise - coverage) / (1.0 - coverage);
    
    // Soft quadratic onset at cloud edges for silky borders without knife-cuts
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
    
    // Below cloud deck: rays below elevation 2.6 deg (dir.y < 0.045) cannot reach cloud slabs within MAX_DIST
    if (h < CUMULUS_BOTTOM - 350.0 && dir.y < 0.045) {
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
    
    vec3 sunDirNorm = normalize(ubo.sunDir.xyz);
    float phase = atmoMiePhase(dot(dir, sunDirNorm));
    vec3 ambientColor = ubo.skyZenith.xyz * 0.4 + ubo.skyHorizon.xyz * 0.3;
    
    // Precompute sun transmittance once per ray at cloud mid-deck altitude
    vec3 midAtmo = vec3(0.0, ATMO_GROUND_R + 4000.0, 0.0);
    vec3 sunTransmittance = exp(-atmoSunOpticalDepth(midAtmo, sunDirNorm));
    
    // Cumulus marching
    if (hitCumulus) {
        float marchDist = cumulusTMax - cumulusTMin;
        float stepSize = marchDist / float(CUMULUS_STEPS);
        float t = cumulusTMin + stepSize * jitter;
        
        for (int i = 0; i < CUMULUS_STEPS; ++i) {
            vec3 samplePos = pos + dir * t;
            float density = getCumulusDensity(samplePos);
            
            if (density > 0.005) {
                if (firstHitT < 0.0) firstHitT = t;
                
                float extinction = density * 0.0035;
                float sampleTransmittance = exp(-extinction * stepSize);
                
                // Silver lining / forward scatter & diffuse fill
                float powder = 1.0 - exp(-density * 3.0);
                vec3 L_direct = ubo.sunColor.xyz * sunTransmittance * (phase * 1.5 + 0.15) * powder;
                
                // Ambient sky lighting
                vec3 L_ambient = ambientColor * (0.7 + 0.3 * (1.0 - density));
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
            float density = getCirrusDensity(samplePos);
            
            if (density > 0.001) {
                if (firstHitT < 0.0) firstHitT = t;
                
                float extinction = density * 0.002;
                float sampleTransmittance = exp(-extinction * stepSize);
                
                vec3 L_direct = ubo.sunColor.xyz * sunTransmittance * (phase * 1.2 + 0.2);
                vec3 L_ambient = ambientColor * 0.85;
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
