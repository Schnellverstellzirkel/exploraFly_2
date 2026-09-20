#version 450

// Procedural mesh cloud field, vertex-pulled exactly like the terrain
// landmarks: gl_InstanceIndex selects a cell; gl_VertexIndex selects a
// shared (puff, triangle corner), using a static topology index buffer, and
// every position is hashed from absolute world cells, so the draw has no
// vertex data and no per-frame bookkeeping. Clouds drift
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
layout(location = 4) flat out vec3 vExtinction;

const vec3 ATMO_BETA_RAYLEIGH = vec3(5.802e-6, 13.558e-6, 33.1e-6);
const vec3 ATMO_BETA_MIE_EXTINCT = vec3(4.44e-6);
const vec3 ATMO_BETA_OZONE = vec3(0.650e-6, 1.881e-6, 0.085e-6);

float atmoOzoneDensity(float h) {
    float density = (h < 25000.0)
        ? h / 15000.0 - 2.0 / 3.0
        : -h / 15000.0 + 8.0 / 3.0;
    return clamp(density, 0.0, 1.0);
}

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
    if (sub == 0u) {
        b = normalize(a + b);
        c = normalize(c + a);
    } else if (sub == 1u) {
        vec3 ab = normalize(a + b);
        c = normalize(b + c);
        a = ab;
    } else if (sub == 2u) {
        vec3 ca = normalize(c + a);
        b = normalize(b + c);
        a = ca;
    } else {
        vec3 ab = normalize(a + b);
        vec3 bc = normalize(b + c);
        c = normalize(c + a);
        a = ab;
        b = bc;
    }
}

// Puff surface: lobed radial displacement (continuous in dir, so triangles
// stay watertight and neighbour samples give analytic smooth normals) with
// vertical squash. lf scales the lobe frequency per puff; the third, finest
// octave only runs on the surface point itself, not on normal neighbours.
vec3 puffSurface(vec3 dir, float r0, float lumpAmp, float seedF, float squash,
                 float lf, bool detail) {
    vec3 s1 = sin(dir * (lf * vec3(1.0, 0.89, 1.11)) + (seedF * vec3(1.0, 1.7, 2.3)));
    float lump = s1.x * s1.y * s1.z;
    vec3 s2 = sin(dir * (lf * vec3(2.13, 1.72, 2.02)) - (seedF * vec3(1.0, 1.3, 0.7)));
    float lump2 = s2.x * s2.y * s2.z;
    float lumps = lumpAmp * (lump + 0.35 * lump2);
    if (detail) {
        vec3 s3 = sin(dir * (lf * vec3(4.31, 3.73, 4.13)) + (seedF * vec3(3.1, 4.7, 5.3)));
        lumps += (0.12 * lumpAmp) * (s3.x * s3.y * s3.z);
    }
    vec3 p = dir * (r0 * (1.0 + lumps));
    p.y *= squash;
    return p;
}

void getFrustumPlanes(out vec4 np0, out vec4 np1, out vec4 np2, out vec4 np3, out vec4 np4, out vec4 np5) {
    vec4 r0 = vec4(ubo.viewProj[0][0], ubo.viewProj[1][0], ubo.viewProj[2][0], ubo.viewProj[3][0]);
    vec4 r1 = vec4(ubo.viewProj[0][1], ubo.viewProj[1][1], ubo.viewProj[2][1], ubo.viewProj[3][1]);
    vec4 r2 = vec4(ubo.viewProj[0][2], ubo.viewProj[1][2], ubo.viewProj[2][2], ubo.viewProj[3][2]);
    vec4 r3 = vec4(ubo.viewProj[0][3], ubo.viewProj[1][3], ubo.viewProj[2][3], ubo.viewProj[3][3]);
    vec4 p0 = r3 + r0;
    vec4 p1 = r3 - r0;
    vec4 p2 = r3 + r1;
    vec4 p3 = r3 - r1;
    vec4 p4 = r2;
    vec4 p5 = r3 - r2;
    np0 = p0 * inversesqrt(dot(p0.xyz, p0.xyz));
    np1 = p1 * inversesqrt(dot(p1.xyz, p1.xyz));
    np2 = p2 * inversesqrt(dot(p2.xyz, p2.xyz));
    np3 = p3 * inversesqrt(dot(p3.xyz, p3.xyz));
    np4 = p4 * inversesqrt(dot(p4.xyz, p4.xyz));
    np5 = p5 * inversesqrt(dot(p5.xyz, p5.xyz));
}

bool sphereInFrustum(vec3 center, float r, vec4 np0, vec4 np1, vec4 np2, vec4 np3, vec4 np4, vec4 np5) {
    if (dot(np0.xyz, center) + np0.w < -r) return false;
    if (dot(np1.xyz, center) + np1.w < -r) return false;
    if (dot(np2.xyz, center) + np2.w < -r) return false;
    if (dot(np3.xyz, center) + np3.w < -r) return false;
    if (dot(np4.xyz, center) + np4.w < -r) return false;
    if (dot(np5.xyz, center) + np5.w < -r) return false;
    return true;
}

void main() {
    uint cellId = uint(gl_InstanceIndex);
    uint rest = uint(gl_VertexIndex);
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
    vec2 cam_delta = worldXZ - cameraWorld;
    if (dot(cam_delta, cam_delta) > CLOUD_DRAW_RADIUS * CLOUD_DRAW_RADIUS
        || float(puff) >= cloudPuffCount(hcell, family)) {
        gl_Position = vec4(0.0, 0.0, 2.0, 1.0);
        return;
    }

    // Pre-extract normalized view frustum planes once per surviving cloud cluster
    vec4 np0, np1, np2, np3, np4, np5;
    getFrustumPlanes(np0, np1, np2, np3, np4, np5);

    // Early cluster frustum rejection: if entire cloud is outside view frustum, cull
    vec3 cloud_center = vec3(worldXZ.x - anchor.x, place.z + ubo.groundBase.w + place.w * 0.4, worldXZ.y - anchor.y);
    float cloud_radius = place.w * 3.2;
    if (!sphereInFrustum(cloud_center, cloud_radius, np0, np1, np2, np3, np4, np5)) {
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

    vec3 center = vec3(worldXZ.x - anchor.x, place.z + ubo.groundBase.w,
        worldXZ.y - anchor.y);
    vec3 puffCenter = center + vec3(radial.x, puffY, radial.y);
    float puffBoundR = pr * (1.0 + lumpAmp * 1.5);
    if (!sphereInFrustum(puffCenter, puffBoundR, np0, np1, np2, np3, np4, np5)) {
        gl_Position = vec4(0.0, 0.0, 2.0, 1.0);
        return;
    }

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

    float distToCam = length(puffCenter - ubo.campos.xyz);
    bool needDetail = distToCam < 3500.0;
    vec3 p0 = puffSurface(sp, pr, lumpAmp, seedF, squashP, lf, needDetail);

    // Smooth normal from tangent neighbours for nearby clouds, or analytic squashed sphere normal for distant clouds
    vec3 n;
    if (distToCam < 6000.0) {
        vec3 t1 = abs(sp.y) < 0.99 ? vec3(sp.z, 0.0, -sp.x) * inversesqrt(sp.z * sp.z + sp.x * sp.x)
            : vec3(1.0, 0.0, 0.0);
        vec3 t2 = cross(sp, t1);
        const float eps = 0.03;
        const float eps_norm = 1.0 - 0.5 * eps * eps;
        vec3 p1 = puffSurface((sp + t1 * eps) * eps_norm, pr, lumpAmp, seedF,
            squashP, lf, false);
        vec3 p2 = puffSurface((sp + t2 * eps) * eps_norm, pr, lumpAmp, seedF,
            squashP, lf, false);
        vec3 cross_n = cross(p1 - p0, p2 - p0);
        n = dot(cross_n, sp) < 0.0 ? -cross_n : cross_n;
    } else {
        n = vec3(sp.x, sp.y / (squashP * squashP), sp.z);
    }
    vNormal = n;

    // Flat cut underneath gives the classic shaded cumulus base.
    p0.y = max(p0.y, -0.25 * pr);

    vPosition = puffCenter + p0;
    vHeight01 = clamp(p0.y / (1.1 * pr * squashP) + 0.5, 0.0, 1.0);
    vTone = 0.94 + 0.10 * cloudHash(hcell, 240u + puff);

    float cam_h = max(ubo.campos.y - ubo.groundBase.w, 0.0);
    float dR = exp(-cam_h / 8000.0);
    float dM = exp(-cam_h / 1200.0);
    float dO = atmoOzoneDensity(cam_h);
    vExtinction = ATMO_BETA_RAYLEIGH * dR + ATMO_BETA_MIE_EXTINCT * dM + ATMO_BETA_OZONE * dO;

    gl_Position = ubo.viewProj * vec4(vPosition, 1.0);
}
