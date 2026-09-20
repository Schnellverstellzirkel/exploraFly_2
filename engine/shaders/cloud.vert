#version 450

// Procedural mesh cloud field, vertex-pulled exactly like the terrain
// landmarks: gl_VertexIndex decodes to (grid cell, puff, triangle corner) and
// every position is hashed from absolute world cells, so the draw has no
// vertex data, no index buffer, and no per-frame bookkeeping. Clouds drift
// with the prevailing wind through uniforms only. Puffs are two-subdivision
// icospheres shaded with analytic smooth normals so billows read as soft
// water, not faceted crystals. cloud.inc is injected by build.rs after
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
// face into four similar triangles at mid-edge vertices: two passes give the
// CLOUD_PUFF_TRIS = 320 budget in cloud.inc.
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
        a = a; b = ab; c = ca;
    } else if (sub == 1u) {
        a = ab; b = b; c = bc;
    } else if (sub == 2u) {
        a = ca; b = bc; c = c;
    } else {
        a = ab; b = bc; c = ca;
    }
}

// Puff surface: lobed radial displacement (continuous in dir, so triangles
// stay watertight and neighbour samples give analytic smooth normals) with
// vertical squash. lf scales the lobe frequency per cloud.
vec3 puffSurface(vec3 dir, float r0, float lumpAmp, float seedF, float squash,
                 float lf) {
    float lump = sin(dir.x * lf + seedF) * sin(dir.y * lf * 0.89 + seedF * 1.7)
        * sin(dir.z * lf * 1.11 + seedF * 2.3);
    float lump2 = sin(dir.x * lf * 2.13 - seedF) * sin(dir.y * lf * 1.72 - seedF * 1.3)
        * sin(dir.z * lf * 2.02 - seedF * 0.7);
    float r = r0 * (1.0 + lumpAmp * lump + 0.35 * lumpAmp * lump2);
    vec3 p = dir * r;
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

    float heightFactor, lumpAmp, stretch, yaw, squash, shear;
    cloudArchetype(hcell, family, heightFactor, lumpAmp, stretch, yaw,
        squash, shear);

    // Puffs walk a golden-angle spiral over the footprint (Vogel 1979), with
    // per-puff angle and radius jitter so clusters never settle into the same
    // rosette; puff 0 crowns the centre and outer puffs shrink.
    float t = sqrt(float(puff) / float(CLOUD_PUFFS));
    float la = float(puff) * 2.39996323 + yaw
        + (cloudHash(hcell, 300u + puff) - 0.5) * 1.1;
    float radJ = 0.8 + 0.4 * cloudHash(hcell, 340u + puff);
    vec2 local = vec2(cos(la), sin(la)) * (t * place.w * radJ);
    local.x *= 1.0 + stretch;
    float cy = cos(yaw);
    float sy = sin(yaw);
    float hHash = cloudHash(hcell, 100u + puff);
    float rHash = cloudHash(hcell, 140u + puff);
    float puffY = place.w * heightFactor * (1.0 - t * t) * (0.6 + 0.5 * hHash)
        + place.w * 0.08 * (hHash - 0.5);
    // Tall clusters lean downwind with height; small ones barely shear.
    vec2 radial = vec2(cy * local.x - sy * local.y, sy * local.x + cy * local.y)
        + vec2(0.70710678) * (puffY * shear);
    float pr = place.w * (0.38 + 0.34 * rHash) * (1.16 - 0.36 * t);
    if (puff == 0u) pr *= 1.18;

    // Two icosphere subdivisions: face f splits into 16 sub-faces, decoded as
    // two quarter selections.
    uint f = tri / 16u;
    uint q = tri % 16u;
    vec3 a = ICO[ICO_FACE[f * 3u + 0u]];
    vec3 b = ICO[ICO_FACE[f * 3u + 1u]];
    vec3 c = ICO[ICO_FACE[f * 3u + 2u]];
    icoSubdivide(a, b, c, q / 4u);
    icoSubdivide(a, b, c, q % 4u);
    vec3 sp = v == 0u ? a : (v == 1u ? b : c);

    float seedF = cloudHash(hcell, 200u + puff) * 17.0;
    float lf = mix(4.1, 6.9, cloudHash(hcell, 260u + puff));

    // Smooth normal from two tangent neighbour samples of the same surface.
    vec3 p0 = puffSurface(sp, pr, lumpAmp, seedF, squash, lf);
    vec3 t1 = normalize(abs(sp.y) < 0.99 ? cross(sp, vec3(0.0, 1.0, 0.0))
        : vec3(1.0, 0.0, 0.0));
    vec3 t2 = normalize(cross(sp, t1));
    float eps = 0.045;
    vec3 p1 = puffSurface(normalize(sp + t1 * eps), pr, lumpAmp, seedF, squash, lf);
    vec3 p2 = puffSurface(normalize(sp + t2 * eps), pr, lumpAmp, seedF, squash, lf);
    vec3 n = cross(p1 - p0, p2 - p0);
    vNormal = dot(n, sp) < 0.0 ? normalize(-n) : normalize(n);

    // Flat cut underneath gives the classic shaded cumulus base.
    p0.y = max(p0.y, -0.25 * pr);

    vec3 center = vec3(worldXZ.x - anchor.x, place.z + ubo.groundBase.w,
        worldXZ.y - anchor.y);
    vPosition = center + vec3(radial.x, puffY, radial.y) + p0;
    vHeight01 = clamp(p0.y / (1.1 * pr * squash) + 0.5, 0.0, 1.0);
    vTone = 0.94 + 0.10 * cloudHash(hcell, 240u + puff);
    gl_Position = ubo.viewProj * vec4(vPosition, 1.0);
}
