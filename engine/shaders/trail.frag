#version 450

// Trail ribbon fragment. Double HG forward peak for ice, flow-map UV noise,
// age erosion. Standard alpha blend over HDR linear.

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

layout(set = 1, binding = 0) uniform texture3D base_vol;
layout(set = 1, binding = 1) uniform sampler base_smp;

layout(location = 0) in vec2 vUv;
layout(location = 1) in float vAge;
layout(location = 2) in float vDensity;
layout(location = 3) in vec3 vWorld;
layout(location = 4) in float vSeed;
layout(location = 5) in float vIce;
layout(location = 6) in float vAcross;

layout(location = 0) out vec4 outColor;

float phaseHg(float mu, float g) {
    float gg = g * g;
    float d = max(1.0 + gg - 2.0 * g * mu, 1e-4);
    return (1.0 - gg) / (12.566371 * d * sqrt(d));
}

void main() {
    vec3 view = normalize(ubo.campos.xyz - vWorld);
    float mu = dot(view, ubo.sunDir.xyz);
    // Integrate a round, soft density profile through the ribbon's cross-section.
    // Across never wraps or drifts with the noise coordinates.
    float across = clamp(vAcross, -1.0, 1.0);
    float edge = 1.0 - abs(across);
    if (edge < 0.005) discard;
    float age_fade = exp(-vAge * 0.012);
    if (vDensity * age_fade < 0.002) discard;

    float chord = sqrt(max(0.0, 1.0 - across * across));
    float z = chord * 0.57735;
    vec3 p1 = vec3(vUv.x, across * 0.7, z * 0.7 + vSeed * 7.0);
    vec3 p2 = vec3(vUv.x, across * 0.7, -z * 0.7 + vSeed * 7.0);
    vec3 s1 = textureLod(sampler3D(base_vol, base_smp), p1, 0.0).rgb;
    vec3 s2 = textureLod(sampler3D(base_vol, base_smp), p2, 0.0).rgb;
    vec3 n = (s1 + s2) * 0.5;
    float r2 = across * across + z * z;
    float density = exp(-3.5 * r2) * (1.0 - smoothstep(0.65, 1.0, r2));
    // Newly shed vapor keeps a continuous center. Older edges separate into
    // coherent wisps, using the same two volume samples and deposited UVs.
    float maturity = smoothstep(0.25, 6.0, vAge);
    float erosion = smoothstep(0.20 + maturity * 0.18, 0.72,
        n.r * 0.72 + n.g * 0.28 + (1.0 - abs(across)) * 0.22);
    float structure = mix(0.90 + n.r * 0.15, 0.28 + erosion * 0.95, maturity);
    float optical = density * structure * chord * 2.0;

    float a = (1.0 - exp(-vDensity * optical * 2.2)) * age_fade;
    if (a < 0.004) {
        discard;
    }
    float g_fwd = mix(0.55, 0.78, clamp(vIce, 0.0, 1.0));
    float phase = 0.85 * phaseHg(mu, g_fwd) + 0.15 * phaseHg(mu, -0.25);
    vec3 sun = ubo.sunColor.rgb * phase * 2.2;
    vec3 amb = mix(ubo.skyHorizon.rgb, ubo.skyZenith.rgb, 0.45) * (0.55 + 0.45 * n.b);
    float rim = mix(0.72, 1.0, smoothstep(0.0, 0.6, edge));
    // Warm near-nozzle vapor tends toward a cool, pale ice wake with age.
    // This is a restrained artistic tint; scattering still follows the sun.
    vec3 tint = mix(vec3(1.03, 0.96, 0.86), vec3(0.87, 0.97, 1.03),
        clamp(vIce * 0.75 + maturity * 0.25, 0.0, 1.0));
    vec3 color = (sun + amb) * tint * rim * (0.75 + 0.5 * vIce);
    outColor = vec4(color, a);
}
