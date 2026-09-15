#version 450

// Sky fragment. Linear HDR output, using physical atmosphere from sky_atmo.inc.
// Note: build.rs injects sky_atmo.inc after #version 450

layout(set = 0, binding = 0) uniform UBO {
    mat4 viewProj;
    mat4 invViewProj;
    mat4 nodes[23];
    vec4 flex;
    vec4 campos;
    vec4 sunDir;
    vec4 sunColor;
    vec4 skyZenith;
    vec4 skyHorizon;
    vec4 groundBase;
    vec4 detail;
} ubo;

layout(location = 0) in vec3 vRay;
layout(location = 0) out vec4 outColor;

void main() {
    vec3 view_dir = normalize(vRay);
    
    vec3 atmo_origin = atmoModelOrigin(ubo.campos.xyz, ubo.groundBase.w);
    vec3 sun_dir = normalize(ubo.sunDir.xyz);
    vec3 sun_irr = ubo.sunColor.rgb;
    
    // Full marched sky
    vec3 sky = atmoIntegrate(atmo_origin, view_dir, sun_dir, sun_irr);
    
    // Below the horizon, blend toward the cheap closed-form atmosphere which
    // gracefully fades into the ground-plane colour.
    if (view_dir.y < -0.001) {
        float blend = smoothstep(-0.001, -0.05, view_dir.y);
        vec3 below = atmoRadianceCheap(atmo_origin, view_dir, sun_dir, sun_irr);
        sky = mix(sky, below, blend);
    }
    
    // Sun transmittance and sun disc
    vec3 tr_sun = exp(-atmoSunOpticalDepth(atmo_origin, sun_dir));
    sky += atmoSunDisc(view_dir, sun_dir, sun_irr, tr_sun);
    
    // Aureole glow around the sun
    float cos_gamma = dot(view_dir, sun_dir);
    if (cos_gamma > 0.0) {
        float aureole = pow(cos_gamma, 12.0) * 0.40 + pow(cos_gamma, 64.0) * 1.6;
        sky += sun_irr * tr_sun * (aureole * 0.45);
    }
    
    outColor = vec4(sky, 1.0);
}
