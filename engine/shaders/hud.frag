#version 450

// Full-rate cockpit overlay. The scene composite can use coarse fragment
// shading, but readable instruments must stay on the output pixel grid.

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
    vec4 trailShift;
    vec4 cameraParams;
    vec4 cameraParams2;
    vec4 groundOrigin;
    vec4 hudFlight;
    vec4 hudState;
} ubo;

layout(location = 0) in vec2 vUv;
layout(location = 0) out vec4 outColor;

const uint HUD_GLYPH[40] = uint[40](31599u,29850u,29671u,31207u,18925u,31183u,31695u,9383u,31727u,31215u,23530u,15083u,25166u,15211u,29391u,4815u,27470u,23533u,29847u,11044u,23277u,29257u,23549u,24573u,11114u,4843u,28522u,23275u,14478u,9367u,31597u,11117u,24557u,23213u,9389u,29351u,4772u,448u,8192u,1040u);

float hudGlyph(vec2 p, uint id) {
    if (id >= 40u || any(lessThan(p, vec2(0))) || any(greaterThanEqual(p, vec2(3,5)))) return 0.0;
    uint bit = uint(floor(p.x)) + 3u * uint(floor(p.y));
    return float((HUD_GLYPH[id] >> bit) & 1u);
}

float hudText(vec2 p, uvec2 words, float size) {
    p /= size;
    if (p.x < 0.0 || p.x >= 40.0 || p.y < 0.0 || p.y >= 5.0) return 0.0;
    uint cell = uint(p.x) / 4u;
    uint word = cell < 5u ? words.x : words.y;
    return hudGlyph(vec2(mod(p.x, 4.0), p.y), (word >> (6u * (cell % 5u))) & 63u);
}

float hudNumber(vec2 p, float value, int digits, float size) {
    p /= size;
    if (p.x < 0.0 || p.y < 0.0 || p.y >= 5.0 || p.x >= float(digits * 4)) return 0.0;
    int column = int(p.x) / 4;
    int divisor = int(pow(10.0, float(digits - column - 1)));
    return hudGlyph(vec2(mod(p.x, 4.0), p.y), uint(int(value + 0.5) / divisor % 10));
}

float hudBox(vec2 p, vec2 lo, vec2 hi) {
    return float(all(greaterThanEqual(p, lo)) && all(lessThan(p, hi)));
}

// Keep the overlay premultiplied while composing its own layers. The final
// color is unpremultiplied for the ordinary SRC_ALPHA blend state.
void hudOver(inout vec3 premul, inout float alpha, vec3 color, float layer_alpha) {
    premul = color * layer_alpha + premul * (1.0 - layer_alpha);
    alpha = layer_alpha + alpha * (1.0 - layer_alpha);
}

void hudEmit(vec3 premul, float alpha) {
    if (alpha <= 0.0001) discard;
    outColor = vec4(premul / alpha, alpha);
}

void main() {
    if (ubo.hudState.x < 0.5) discard;
    float scale = clamp(1.0 / (900.0 * fwidth(vUv).y), 0.65, 1.6);
    vec2 viewport = 1.0 / max(fwidth(vUv), vec2(0.00001));
    vec2 p = vUv * viewport / scale;
    vec2 extent = viewport / scale;
    const vec3 ink = vec3(0.008,0.020,0.026);
    const vec3 paper = vec3(0.89,0.86,0.72);
    const vec3 brass = vec3(0.64,0.39,0.13);
    const vec3 teal = vec3(0.11,0.60,0.48);
    uint flags = uint(ubo.hudState.z);

    if (hudBox(p, vec2(24,24), vec2(266,99)) > 0.5) {
        vec2 q = p - vec2(24,24);
        vec3 c = vec3(0.0);
        float a = 0.0;
        hudOver(c, a, ink, 0.78);
        hudOver(c, a, brass, hudBox(q, vec2(0), vec2(3,75)));
        hudOver(c, a, paper, hudText(q-vec2(18,16), uvec2(408262734u, 1073738395u), 4.0));
        hudOver(c, a, teal, hudText(q-vec2(18,47), uvec2(255387343u, 491062421u), 2.0));
        hudEmit(c, a);
        return;
    }

    // Compass ribbon uses continuous heading offsets; label is true heading.
    vec2 cp = p - vec2(extent.x * 0.5 - 142.0, 24);
    if (hudBox(cp, vec2(0), vec2(284,54)) > 0.5) {
        vec3 c = vec3(0.0);
        float a = 0.0;
        hudOver(c, a, ink, 0.68);
        float tick = float(mod(cp.x - 142.0 + ubo.hudFlight.z * 2.0, 20.0) < 1.0)
            * hudBox(cp, vec2(8,36), vec2(276,44));
        float mark = hudBox(cp, vec2(141,43), vec2(143,53));
        hudOver(c, a, brass, max(tick * 0.65, mark));
        hudOver(c, a, paper, hudNumber(cp-vec2(125,12), ubo.hudFlight.z, 3, 3.0));
        hudEmit(c, a);
        return;
    }

    vec2 left = p - vec2(24, extent.y-130);
    if (hudBox(left, vec2(0), vec2(252,106)) > 0.5) {
        vec3 c = vec3(0.0);
        float a = 0.0;
        hudOver(c, a, ink, 0.79);
        float label = hudText(left-vec2(16,12), uvec2(426882186u, 1073533838u), 2.0);
        float num = hudNumber(left-vec2(16,32), ubo.hudFlight.x, 4, 6.0);
        float units = hudText(left-vec2(120,48), uvec2(1073739604u, 1073741823u), 2.0);
        float boost = hudText(left-vec2(16,82), uvec2(493979147u, 1073741823u), 2.0);
        float track = hudBox(left, vec2(70,83), vec2(236,91));
        float fill = hudBox(left, vec2(70,83), vec2(70 + 166.0*ubo.hudState.y,91));
        hudOver(c, a, paper * 0.50, max(label, boost));
        hudOver(c, a, paper, max(num, units));
        hudOver(c, a, teal * 0.15, track);
        hudOver(c, a, teal, fill);
        hudEmit(c, a);
        return;
    }

    vec2 right = p - vec2(extent.x-276, extent.y-130);
    if (hudBox(right, vec2(0), vec2(252,106)) > 0.5) {
        vec3 c = vec3(0.0);
        float a = 0.0;
        hudOver(c, a, ink, 0.79);
        float label = hudText(right-vec2(16,12), uvec2(491377994u, 1073537886u), 2.0);
        float num = hudNumber(right-vec2(16,32), ubo.hudFlight.y, 5, 6.0);
        float units = hudText(right-vec2(144,48), uvec2(1073741782u, 1073741823u), 2.0);
        float agl = hudText(right-vec2(16,82), uvec2(1073566730u, 1073741823u), 2.0);
        float ground = hudNumber(right-vec2(60,80), ubo.hudState.w, 5, 2.5);
        hudOver(c, a, paper * 0.5, label);
        hudOver(c, a, paper, max(num, units));
        hudOver(c, a, ubo.hudState.w < 100.0 ? brass : teal, max(agl, ground));
        hudEmit(c, a);
        return;
    }

    vec2 center = p - extent * 0.5;
    if ((flags & 8u) != 0u && hudBox(center, vec2(-108,-35), vec2(108,45)) > 0.5) {
        vec3 c = vec3(0.0);
        float a = 0.0;
        hudOver(c, a, ink, 0.86);
        float text = hudText(center + vec2(69,17), uvec2(242344601u, 1073741773u), 6.0);
        float hint = hudText(center - vec2(-47,22), uvec2(473546713u, 1073538462u), 2.0);
        hudOver(c, a, paper, max(text,hint));
        hudEmit(c, a);
        return;
    }

    // Keep help away from instrument panels on narrow windows.
    vec2 hp = p - vec2(extent.x*0.5-146, extent.y-124);
    if ((flags & 2u) != 0u && extent.x > 920.0 && hudBox(hp,vec2(0),vec2(292,100)) > 0.5) {
        vec3 c = vec3(0.0);
        float a = 0.0;
        hudOver(c, a, ink, 0.70);
        float t = hudText(hp-vec2(14,12), uvec2(436062496u, 1061472082u), 2.0);
        t = max(t, hudText(hp-vec2(158,12), uvec2(201120010u, 1073563082u), 2.0));
        t = max(t, hudText(hp-vec2(14,34), uvec2(587000090u, 1073739786u), 2.0));
        t = max(t, hudText(hp-vec2(158,34), uvec2(490546268u, 493979147u), 2.0));
        t = max(t, hudText(hp-vec2(14,56), uvec2(506044377u, 1073738652u), 2.0));
        t = max(t, hudText(hp-vec2(158,56), uvec2(473546715u, 1073739598u), 2.0));
        t = max(t, hudText(hp-vec2(14,78), uvec2(509726678u, 1073738583u), 2.0));
        t = max(t, hudText(hp-vec2(158,78), uvec2(239595599u, 1073739349u), 2.0));
        hudOver(c, a, paper * 0.7, t);
        hudEmit(c, a);
        return;
    }

    discard;
}
