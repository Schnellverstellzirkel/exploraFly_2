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

void main() {
    vec3 view_dir = normalize(vRay);
    // The rasterized terrain is finite. Keep atmospheric background below the
    // horizon too, so rays beyond the far plane never expose the clear color.
    
    vec3 atmo_origin = atmoModelOrigin(ubo.campos.xyz, ubo.groundBase.w);
    vec3 sun_dir = normalize(ubo.sunDir.xyz);
    vec3 sun_irr = ubo.sunColor.rgb;
    vec3 tr_sun = atmoTransmittanceToTop(atmo_origin, sun_dir);
    
    vec3 sky = atmoIntegrate(atmo_origin, view_dir, sun_dir, sun_irr)
             * ATMO_HDR_RADIANCE_SCALE;
    
    // Sun transmittance and sun disc
    sky += atmoSunDisc(view_dir, sun_dir, sun_irr, tr_sun);
    
    outColor = vec4(sky, 1.0);
}
