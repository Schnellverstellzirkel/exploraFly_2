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
    // The rasterized terrain is finite. Keep atmospheric background below the
    // horizon too, so rays beyond the far plane never expose the clear color.
    
    vec3 atmo_origin = atmoModelOrigin(ubo.campos.xyz, ubo.groundBase.w);
    vec3 sun_dir = normalize(ubo.sunDir.xyz);
    vec3 sun_irr = ubo.sunColor.rgb;
    vec3 tr_sun = exp(-atmoSunOpticalDepth(atmo_origin, sun_dir));
    
    vec3 sky = atmoIntegrate(atmo_origin, view_dir, sun_dir, sun_irr);
    
    // Sun transmittance and sun disc
    sky += atmoSunDisc(view_dir, sun_dir, sun_irr, tr_sun);
    
    // The circumsolar aureole comes from the Mie phase term in
    // atmoIntegrate; adding a screen-space halo here would double-count it.
    
    outColor = vec4(sky, 1.0);
}
