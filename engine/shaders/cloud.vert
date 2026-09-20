#version 450

// Procedural mesh cloud field, vertex-pulled exactly like the terrain
// landmarks: gl_VertexIndex decodes to (grid cell, puff, triangle corner) and
// every position is hashed from absolute world cells, so the draw has no
// vertex data, no index buffer, and no per-frame bookkeeping. Clouds drift
// with the prevailing wind through uniforms only. Puff clusters are random
// gathers over the upper hemisphere shaped by a per-family spread profile,
// each puff is a three-subdivision icosphere with analytic smooth normals,
// and the lobed displacement carries three octaves so silhouettes stay
// rounded and detailed up close. cloud.inc is injected by build.rs after
// #version.

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
} ubo;

layout(location = 0) out vec3 vPosition;
layout(location = 1) out vec3 vNormal;
layout(location = 2) out float vHeight01;
layout(location = 3) out float vTone;

// Unit icosphere (12 vertices, 20 CCW-outward faces). Each pass splits every
// face into four similar triangles at mid-edge vertices: three passes give
// the CLOUD_PUFF_TRIS = 1280 budget in cloud.inc.
const vec3 ICO[12] = vec3[12](
    vec3(-0.52573111, 0.85065081, 0.00000000),
    vec3(0.52573111, 0.85065081, 0.00000000),
    vec3(-0.52573111, -0.85065081, 0.00000000),
    vec3(0.52573111, -0.85065081, 0.00000000),
    vec3(0.85065081, 0.00000000, -0.52573111),
    vec3(0.85065081, 0.00000000, 0.52573111),
    vec3(-0.85065081, 0.00000000, -0.52573111),
    vec3(-0.85065081, 0.00000000, 0.52573111),
    vec3(0.00000000, -0.52573111, -0.85065081),
    vec3(0.00000000, 0.52573111, -0.85065081),
    vec3(0.00000000, 0.52573111, 0.85065081),
    vec3(0.00000000, -0.52573111, 0.85065081)
);
const int ICO_FACE[60] = int[60](
    0, 11, 5, 0, 5, 1,
    0, 7, 1, 0, 7, 10,
    0, 11, 10, 1, 5, 9,
    5, 11, 4, 11, 10, 2,
    10, 6, 7, 7, 1, 8,
    3, 9, 4, 3, 2, 4,
    3, 2, 6, 3, 6, 8,
    3, 8, 9, 4, 9, 5,
    2, 4, 11, 6, 2, 10,
    8, 7, 6, 9, 1, 8
);

// One subdivision step: face (a, b, c) splits into four CCW sub-faces and
// sub selects one of them.
void icoSubdivide(inout vec3 a, inout vec3 b, inout vec3 c, uint sub) {
    vec3 ab = normalize(a + b);
    vec3 bc = normalize(b + c);
    vec3 ca = normalize(c + a);
    if (sub == 0u) {
        b = ab; c = ca;
    } else if (sub == 1u) {
        a = ab; c = bc;
    } else if (sub == 2u) {
        a = ca; b = bc;
    } else {
        a = ab; b = bc; c = ca;
    }
}

// Puff surface: lobed radial displacement (continuous in dir, so triangles
// stay watertight and neighbour samples give analytic smooth normals) with
// vertical squash. lf scales the lobe frequency per puff; the third, finest
// octave only runs on the surface point itself, not on normal neighbours.
vec3 puffSurface(vec3 dir, float r0, float lumpAmp, float seedF, float squash,
                 float lf, bool detail) {
    float lump = sin(dir.x * lf + seedF) * sin(dir.y * lf * 0.89 + seedF * 1.7)
        * sin(dir.z * lf * 1.11 + seedF * 2.3);
    float lump2 = sin(dir.x * lf * 2.13 - seedF) * sin(dir.y * lf * 1.72 - seedF * 1.3)
        * sin(dir.z * lf * 2.02 - seedF * 0.7);
    float lumps = lumpAmp * lump + 0.35 * lumpAmp * lump2;
    if (detail) {
        float lump3 = sin(dir.x * lf * 4.31 + seedF * 3.1)
            * sin(dir.y * lf * 3.73 + seedF * 4.7)
            * sin(dir.z * lf * 4.13 + seedF * 5.3);
        lumps += 0.12 * lumpAmp * lump3;
    }
    vec3 p = dir * (r0 * (1.0 + lumps));
    p.y *= squash;
    return p;
}

void main() {
    uint cellId = uint(gl_VertexIndex) / uint(CLOUD_PUFFS * CLOUD_CORNERS);
    uint rest = uint(gl_VertexIndex) % uint(CLOUD_PUFFS * CLOUD_CORNERS);
    uint puff = rest / CLOUD_CORNERS;
    uint corner = rest % CLOUD_CORNERS;
    uint tri = corner / 3u;
    uint v = corner % 3u;

    // Grid cells around the camera in absolute world space, so the field is
    // station-locked: flying only changes which cells are submitted.
    vec2 anchor = cloudWorldAnchor(ubo.groundOrigin);
    vec2 cameraWorld = anchor + ubo.campos.xz;
    ivec2 camCell = ivec2(floor(cameraWorld / CLOUD_CELL));
    ivec2 rel = ivec2(int(cellId % uint(CLOUD_GRID)), int(cellId / uint(CLOUD_GRID)))
        - int(CLOUD_GRID / 2);
    ivec2 worldCell = camCell + rel;
    uvec2 hcell = uvec2(worldCell) & uvec2(CLOUD_PERIOD_CELLS - 1u);

    vPosition = vec3(0.0);
    vNormal = vec3(0.0, 1.0, 0.0);
    vHeight01 = 0.0;
    vTone = 1.0;
    uint family = cloudCellFamily(hcell);
    if (family == 0u) {
        gl_Position = vec4(0.0, 0.0, 2.0, 1.0);
        return;
    }

    // Prevailing breeze advection, wrapped at the field period so long
    // sessions cannot drift the coordinates out of float range. A full wrap
    // shifts every cloud by exactly one world period, which is invisible.
    float drift = mod(ubo.flex.y * ubo.cameraParams2.w * CLOUD_DRIFT_SPEED,
        CLOUD_FIELD_PERIOD);

    vec4 place = cloudCellPlacement(hcell, family);
    vec2 worldXZ = vec2(worldCell) * CLOUD_CELL + place.xy + vec2(drift);
    if (distance(worldXZ, cameraWorld) > CLOUD_DRAW_RADIUS
        || float(puff) >= cloudPuffCount(hcell, family)) {
        gl_Position = vec4(0.0, 0.0, 2.0, 1.0);
        return;
    }

    float heightFactor, lumpAmp, stretch, yaw, squash, shear, shBase, shTop;
    cloudArchetype(hcell, family, heightFactor, lumpAmp, stretch, yaw,
        squash, shear, shBase, shTop);

    // Puff placement: hashed, area-uniform points over the upper hemisphere
    // instead of an index spiral, so every cluster is a genuinely random
    // gather and no rosette skeleton repeats. Puff 0 always crowns the top;
    // the family's spread profile converts direction into a horizontal
    // offset, and dir.y into height, giving domes, heaps, and columns from
    // one placement rule.
    float ct;
    float az0;
    if (puff == 0u) {
        ct = 1.0;
        az0 = 0.0;
    } else {
        ct = cloudHash(hcell, 440u + puff);
        az0 = cloudHash(hcell, 400u + puff) * 6.2831853;
    }
    float st = sqrt(max(1.0 - ct * ct, 0.0));
    float sh = mix(shBase, shTop, ct);
    vec2 local = vec2(cos(az0), sin(az0)) * (st * place.w * sh);
    local.x *= 1.0 + stretch;
    float cy = cos(yaw);
    float sy = sin(yaw);
    float hHash = cloudHash(hcell, 100u + puff);
    float rHash = cloudHash(hcell, 140u + puff);
    float puffY = ct * place.w * heightFactor * (0.6 + 0.5 * hHash)
        + place.w * 0.08 * (hHash - 0.5);
    // Tall clusters lean downwind with height; small ones barely shear.
    vec2 radial = vec2(cy * local.x - sy * local.y, sy * local.x + cy * local.y)
        + vec2(0.70710678) * (puffY * shear);
    float pr = place.w * (0.38 + 0.34 * rHash) * (1.16 - 0.36 * st);
    if (puff == 0u) pr *= 1.18;

    // Three icosphere subdivisions: face f splits into 64 sub-faces, decoded
    // as three quarter selections.
    uint f = tri / 64u;
    uint q = tri % 64u;
    vec3 a = ICO[ICO_FACE[f * 3u + 0u]];
    vec3 b = ICO[ICO_FACE[f * 3u + 1u]];
    vec3 c = ICO[ICO_FACE[f * 3u + 2u]];
    icoSubdivide(a, b, c, (q / 16u) % 4u);
    icoSubdivide(a, b, c, (q / 4u) % 4u);
    icoSubdivide(a, b, c, q % 4u);
    vec3 sp = v == 0u ? a : (v == 1u ? b : c);

    float seedF = cloudHash(hcell, 200u + puff) * 17.0;
    float lf = mix(4.1, 6.9, cloudHash(hcell, 260u + puff));
    float squashP = squash * mix(0.85, 1.15, cloudHash(hcell, 560u + puff));

    // Smooth normal from two tangent neighbour samples of the same surface.
    vec3 p0 = puffSurface(sp, pr, lumpAmp, seedF, squashP, lf, true);
    vec3 t1 = normalize(abs(sp.y) < 0.99 ? cross(sp, vec3(0.0, 1.0, 0.0))
        : vec3(1.0, 0.0, 0.0));
    vec3 t2 = normalize(cross(sp, t1));
    float eps = 0.03;
    vec3 p1 = puffSurface(normalize(sp + t1 * eps), pr, lumpAmp, seedF,
        squashP, lf, false);
    vec3 p2 = puffSurface(normalize(sp + t2 * eps), pr, lumpAmp, seedF,
        squashP, lf, false);
    vec3 n = cross(p1 - p0, p2 - p0);
    vNormal = dot(n, sp) < 0.0 ? normalize(-n) : normalize(n);

    // Flat cut underneath gives the classic shaded cumulus base.
    p0.y = max(p0.y, -0.25 * pr);

    vec3 center = vec3(worldXZ.x - anchor.x, place.z + ubo.groundBase.w,
        worldXZ.y - anchor.y);
    vPosition = center + vec3(radial.x, puffY, radial.y) + p0;
    vHeight01 = clamp(p0.y / (1.1 * pr * squashP) + 0.5, 0.0, 1.0);
    vTone = 0.94 + 0.10 * cloudHash(hcell, 240u + puff);
    gl_Position = ubo.viewProj * vec4(vPosition, 1.0);
}
