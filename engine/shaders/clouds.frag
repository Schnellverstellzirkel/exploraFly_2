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
const int CUMULUS_STEPS = 10;
const int CIRRUS_STEPS = 8;

float getCumulusDensity(vec3 pos) {
    float h = pos.y - ubo.groundBase.w;
    
    // Noise-based vertical offset (16 km cell for smooth undulating cloud deck base)
    float offset = atmoValueNoise(pos + vec3(ubo.flex.y * 5.0, 0.0, ubo.flex.y * 5.0), 16000.0, 200u) * 700.0 - 350.0;
    
    float bottom = CUMULUS_BOTTOM + offset;
    float top = CUMULUS_TOP + offset;
    
    if (h < bottom || h > top) return 0.0;
    
    // Vertical fade
    float heightFraction = (h - bottom) / (top - bottom);
    float profile = smoothstep(0.0, 0.2, heightFraction) * smoothstep(1.0, 0.8, heightFraction);
    
    // FBM 4 octaves, 4000m base cell
    float noise = atmoCloudFbm(pos + vec3(ubo.flex.y * 10.0, 0.0, ubo.flex.y * 15.0), 4000.0, 4, 42u);
    
    // Coverage threshold ~0.35
    float coverage = 0.35;
    float density = max(0.0, noise - coverage) / (1.0 - coverage);
    
    return density * profile;
}

float getCirrusDensity(vec3 pos) {
    float h = pos.y - ubo.groundBase.w;
    
    if (h < CIRRUS_BOTTOM || h > CIRRUS_TOP) return 0.0;
    
    // Vertical fade
    float heightFraction = (h - CIRRUS_BOTTOM) / (CIRRUS_TOP - CIRRUS_BOTTOM);
    float profile = smoothstep(0.0, 0.2, heightFraction) * smoothstep(1.0, 0.8, heightFraction);
    
    // FBM 3 octaves, 8000m base cell
    float noise = atmoCloudFbm(pos + vec3(ubo.flex.y * 20.0, 0.0, 0.0), 8000.0, 3, 71u);
    
    float coverage = 0.2;
    float density = max(0.0, noise - coverage) / (1.0 - coverage);
    
    // Very thin wisps
    return density * profile * 0.15;
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
    
    vec3 atmo_origin = atmoModelOrigin(ubo.campos.xyz, ubo.groundBase.w);
    
    // Intersect layers
    float cumulusTMin, cumulusTMax;
    bool hitCumulus = intersectSlab(pos.y, dir.y, ubo.groundBase.w + CUMULUS_BOTTOM - 350.0, ubo.groundBase.w + CUMULUS_TOP + 350.0, cumulusTMin, cumulusTMax);
    
    float cirrusTMin, cirrusTMax;
    bool hitCirrus = intersectSlab(pos.y, dir.y, ubo.groundBase.w + CIRRUS_BOTTOM, ubo.groundBase.w + CIRRUS_TOP, cirrusTMin, cirrusTMax);
    
    if (!hitCumulus && !hitCirrus) {
        discard;
    }
    
    // Maximum distance to march
    float maxDist = 200000.0;
    if (hitCumulus) cumulusTMax = min(cumulusTMax, maxDist);
    if (hitCirrus) cirrusTMax = min(cirrusTMax, maxDist);
    
    float transmittance = 1.0;
    vec3 scatterColor = vec3(0.0);
    float firstHitT = -1.0;
    
    float phase = atmoMiePhase(dot(dir, ubo.sunDir.xyz));
    vec3 ambientColor = ubo.skyZenith.xyz * 0.3 + ubo.skyHorizon.xyz * 0.2;
    
    // Cumulus marching
    if (hitCumulus) {
        float stepSize = (cumulusTMax - cumulusTMin) / float(CUMULUS_STEPS);
        float t = cumulusTMin + stepSize * 0.5;
        
        for (int i = 0; i < CUMULUS_STEPS; ++i) {
            vec3 samplePos = pos + dir * t;
            float density = getCumulusDensity(samplePos);
            
            if (density > 0.01) {
                if (firstHitT < 0.0) firstHitT = t;
                
                float extinction = density * 0.01;
                float sampleTransmittance = exp(-extinction * stepSize);
                
                // Lighting
                vec3 sampleAtmo = vec3(0.0, ATMO_GROUND_R + max(samplePos.y - ubo.groundBase.w, 0.0), 0.0);
                vec3 sunTransmittance = exp(-atmoSunOpticalDepth(sampleAtmo, normalize(ubo.sunDir.xyz)));
                
                // Powder effect / Silver lining
                float powder = 1.0 - exp(-density * 0.1);
                vec3 S = ubo.sunColor.xyz * sunTransmittance * phase * powder * density;
                
                // Multiple scatter ambient
                vec3 ambient = ambientColor * (1.0 - density * 0.5) * density;
                
                S += ambient;
                
                scatterColor += S * transmittance * (1.0 - sampleTransmittance) / extinction;
                transmittance *= sampleTransmittance;
                
                if (transmittance < 0.01) break;
            }
            t += stepSize;
        }
    }
    
    // Cirrus marching
    if (hitCirrus && transmittance > 0.01) {
        float stepSize = (cirrusTMax - cirrusTMin) / float(CIRRUS_STEPS);
        float t = cirrusTMin + stepSize * 0.5;
        
        for (int i = 0; i < CIRRUS_STEPS; ++i) {
            vec3 samplePos = pos + dir * t;
            float density = getCirrusDensity(samplePos);
            
            if (density > 0.001) {
                if (firstHitT < 0.0) firstHitT = t;
                
                float extinction = density * 0.005;
                float sampleTransmittance = exp(-extinction * stepSize);
                
                vec3 sampleAtmo = vec3(0.0, ATMO_GROUND_R + max(samplePos.y - ubo.groundBase.w, 0.0), 0.0);
                vec3 sunTransmittance = exp(-atmoSunOpticalDepth(sampleAtmo, normalize(ubo.sunDir.xyz)));
                
                vec3 S = ubo.sunColor.xyz * sunTransmittance * phase * density;
                vec3 ambient = ambientColor * (1.0 - density * 0.2) * density;
                S += ambient;
                
                scatterColor += S * transmittance * (1.0 - sampleTransmittance) / max(extinction, 1e-5);
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
    vec4 clipPos = ubo.viewProj * vec4(pos + dir * max(firstHitT, 0.1), 1.0);
    if (clipPos.w <= 0.0) {
        gl_FragDepth = 0.999999;
    } else {
        gl_FragDepth = clamp(clipPos.z / clipPos.w, 0.0, 0.999999);
    }
    
    // Premultiplied alpha blend requires scatterColor * alpha if scatterColor is pure radiance,
    // but here we already accumulated scattered light.
    // Assuming scatterColor is already the correct in-scattered radiance.
    // The requirement is: "Output: premultiplied alpha: outColor = vec4(cloud_color * alpha, alpha);"
    // We'll normalize scatterColor by alpha first to match the literal formula if needed,
    // but typical volumetric raymarching accumulates premultiplied alpha directly.
    // We will just supply the accumulated scatterColor.
    outColor = vec4(scatterColor, alpha);
}
