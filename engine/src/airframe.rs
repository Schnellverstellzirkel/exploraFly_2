// Glider airframe generator. Same planform as the web prototype.
// Output is indexed parts with baked flex weights, ready for one
// interleaved device-local stream and one uber-shader pipeline.

pub use super::airframe_util::{MatId, Node};
use super::airframe_util::{RawPart, RawVert};
use glam::Vec3;

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn wing_point(side: f32, t: f32, chord: f32) -> Vec3 {
    let x = 0.42 + 10.4 * t;
    let leading = -1.4 + 0.9 * t + 2.7 * t * t;
    let width = (2.35 - 1.65 * t) * (1.0 - t.powi(12) * 0.87);
    let y = 0.08 + 0.22 * t + 0.65 * t.powi(5) + (chord * std::f32::consts::PI).sin() * 0.14 * (1.0 - t);
    Vec3::new(side * (x - 1.2), y, leading + width * chord)
}

struct Part {
    node: Node,
    mat: MatId,
    /// Plane-local x offset of the part origin, for flex weights.
    off_x: f32,
    side: f32,
    verts: Vec<RawVert>,
    idx: Vec<u32>,
}

impl Part {
    fn new(node: Node, mat: MatId, off_x: f32, side: f32) -> Self {
        Self { node, mat, off_x, side, verts: Vec::new(), idx: Vec::new() }
    }

    fn vert(&mut self, p: Vec3, uv: [f32; 2]) -> u32 {
        // Mirror z so the nose faces +z like the sim. Flex weight
        // matches the prototype: span fraction from plane-local x.
        let flex = ((p.x + self.off_x).abs() - 0.42) / 10.4;
        let flex = flex.clamp(0.0, 1.0) * self.side;
        let id = self.verts.len() as u32;
        self.verts.push(RawVert { pos: [p.x, p.y, -p.z], uv, flex });
        id
    }

    fn tri(&mut self, a: u32, b: u32, c: u32) {
        // Winding flips with the mirror.
        self.idx.extend_from_slice(&[a, c, b]);
    }

    fn grid(
        &mut self,
        rows: usize,
        cols: usize,
        mut point: impl FnMut(usize, usize) -> (Vec3, [f32; 2]),
    ) {
        let base = self.verts.len() as u32;
        for i in 0..=rows {
            for j in 0..=cols {
                let (p, uv) = point(i, j);
                self.vert(p, uv);
            }
        }
        let stride = cols + 1;
        for i in 0..rows {
            for j in 0..cols {
                let a = base + (i * stride + j) as u32;
                let b = a + 1;
                let c = a + stride as u32;
                let d = c + 1;
                self.tri(a, b, d);
                self.tri(a, d, c);
            }
        }
    }

    fn catmull(points: &[Vec3], t: f32) -> Vec3 {
        let n = points.len();
        if n == 1 {
            return points[0];
        }
        let segs = (n - 1) as f32;
        let f = (t.clamp(0.0, 1.0) * segs).min(segs - 1e-4);
        let i = f.floor() as usize;
        let u = f - i as f32;
        let p0 = points[i.saturating_sub(1)];
        let p1 = points[i];
        let p2 = points[(i + 1).min(n - 1)];
        let p3 = points[(i + 2).min(n - 1)];
        let u2 = u * u;
        let u3 = u2 * u;
        p1 * 2.0
            + (p2 - p0) * u
            + (p0 * 2.0 - p1 * 5.0 + p2 * 4.0 - p3) * u2
            + (p3 - p0 + p1 * 3.0 - p2 * 3.0) * u3
    }

    fn tube(&mut self, points: &[Vec3], radius: f32, segs: usize, radial: usize) {
        // Rings along a Catmull-Rom curve with parallel transport frames.
        let mut frames: Vec<(Vec3, Vec3, Vec3)> = Vec::with_capacity(segs + 1);
        let mut prev_n = Vec3::new(1.0, 0.0, 0.0);
        for i in 0..=segs {
            let t = i as f32 / segs as f32;
            let c = Self::catmull(points, t);
            let c2 = Self::catmull(points, (t + 0.01).min(1.0));
            let tangent = (c2 - c).normalize_or_zero();
            let up = if tangent.y.abs() > 0.94 { Vec3::X } else { Vec3::Y };
            let mut n = prev_n - tangent * prev_n.dot(tangent);
            if n.length_squared() < 1e-8 {
                n = (up - tangent * up.dot(tangent)).normalize_or_zero();
            } else {
                n = n.normalize_or_zero();
            }
            prev_n = n;
            let b = tangent.cross(n).normalize_or_zero();
            frames.push((c, n, b));
        }
        let base = self.verts.len() as u32;
        for (c, n, b) in &frames {
            for j in 0..=radial {
                let a = j as f32 / radial as f32 * std::f32::consts::TAU;
                let p = *c + (*n * a.cos() + *b * a.sin()) * radius;
                self.vert(p, [0.0, 0.0]);
            }
        }
        let stride = radial + 1;
        for i in 0..segs {
            for j in 0..radial {
                let a = base + (i * stride + j) as u32;
                let b = a + 1;
                let c = a + stride as u32;
                let d = c + 1;
                self.tri(a, c, d);
                self.tri(a, d, b);
            }
        }
    }

    fn ellipsoid(&mut self, center: Vec3, radii: Vec3, lon: usize, lat: usize) {
        let base = self.verts.len() as u32;
        for i in 0..=lat {
            let v = i as f32 / lat as f32;
            let phi = v * std::f32::consts::PI;
            for j in 0..=lon {
                let u = j as f32 / lon as f32;
                let theta = u * std::f32::consts::TAU;
                let p = Vec3::new(
                    center.x + radii.x * phi.sin() * theta.cos(),
                    center.y + radii.y * phi.cos(),
                    center.z + radii.z * phi.sin() * theta.sin(),
                );
                self.vert(p, [u, v]);
            }
        }
        let stride = lon + 1;
        for i in 0..lat {
            for j in 0..lon {
                let a = base + (i * stride + j) as u32;
                let b = a + 1;
                let c = a + stride as u32;
                let d = c + 1;
                self.tri(a, c, d);
                self.tri(a, d, b);
            }
        }
    }

    fn torus(&mut self, radius: f32, tube: f32, z: f32, radial: usize, tubular: usize) {
        let base = self.verts.len() as u32;
        for i in 0..=tubular {
            let u = i as f32 / tubular as f32 * std::f32::consts::TAU;
            for j in 0..=radial {
                let v = j as f32 / radial as f32 * std::f32::consts::TAU;
                let p = Vec3::new(
                    (radius + tube * v.cos()) * u.cos(),
                    (radius + tube * v.cos()) * u.sin(),
                    z + tube * v.sin(),
                );
                self.vert(p, [u, v]);
            }
        }
        let stride = radial + 1;
        for i in 0..tubular {
            for j in 0..radial {
                let a = base + (i * stride + j) as u32;
                let b = a + 1;
                let c = a + stride as u32;
                let d = c + 1;
                self.tri(a, c, d);
                self.tri(a, d, b);
            }
        }
    }

    fn lathe_z(&mut self, profile: &[[f32; 2]], segments: usize, y_scale: f32) {
        // Profile is (radius, z) pairs revolved around the z axis.
        let base = self.verts.len() as u32;
        for [r, z] in profile {
            for j in 0..=segments {
                let a = j as f32 / segments as f32 * std::f32::consts::TAU;
                self.vert(Vec3::new(r * a.cos(), r * a.sin() * y_scale, *z), [0.0, 0.0]);
            }
        }
        let stride = segments + 1;
        for i in 0..profile.len() - 1 {
            for j in 0..segments {
                let a = base + (i * stride + j) as u32;
                let b = a + 1;
                let c = a + stride as u32;
                let d = c + 1;
                self.tri(a, c, d);
                self.tri(a, d, b);
            }
        }
    }

    fn cylinder_z(
        &mut self,
        r_bottom: f32,
        r_top: f32,
        z0: f32,
        z1: f32,
        radial: usize,
        flip: bool,
    ) {
        let base = self.verts.len() as u32;
        for j in 0..=radial {
            let a = j as f32 / radial as f32 * std::f32::consts::TAU;
            self.vert(Vec3::new(r_bottom * a.cos(), r_bottom * a.sin(), z0), [0.0, 0.0]);
            self.vert(Vec3::new(r_top * a.cos(), r_top * a.sin(), z1), [0.0, 0.0]);
        }
        for j in 0..radial {
            let a = base + (j * 2) as u32;
            if flip {
                self.tri(a, a + 1, a + 3);
                self.tri(a, a + 3, a + 2);
            } else {
                self.tri(a, a + 2, a + 3);
                self.tri(a, a + 3, a + 1);
            }
        }
    }

    fn quad(&mut self, p: [Vec3; 4]) {
        let base = self.verts.len() as u32;
        for v in p {
            self.vert(v, [0.0, 0.0]);
        }
        self.tri(base, base + 1, base + 2);
        self.tri(base, base + 2, base + 3);
    }

    fn extrude(&mut self, outline: &[[f32; 2]], z0: f32, z1: f32) {
        // Flat plate from a simple polygon. Front and back fans plus rim.
        // Outlines here are convex-ish; ear clipping handles the rest.
        let mut poly: Vec<[f32; 2]> = outline.to_vec();
        if signed_area(&poly) < 0.0 {
            poly.reverse();
        }
        let mut indices = ear_clip(&poly);
        let base = self.verts.len() as u32;
        for [x, y] in &poly {
            self.vert(Vec3::new(*x, *y, z0), [0.0, 0.0]);
        }
        let back = self.verts.len() as u32;
        for [x, y] in &poly {
            self.vert(Vec3::new(*x, *y, z1), [0.0, 0.0]);
        }
        for tri in &indices {
            self.tri(base + tri[0], base + tri[1], base + tri[2]);
            self.tri(back + tri[0], back + tri[2], back + tri[1]);
        }
        let n = poly.len() as u32;
        for i in 0..n {
            let j = (i + 1) % n;
            self.tri(base + i, back + i, back + j);
            self.tri(base + i, back + j, base + j);
        }
        let _ = &mut indices;
    }
}

fn signed_area(poly: &[[f32; 2]]) -> f32 {
    let mut area = 0.0f32;
    for i in 0..poly.len() {
        let [x0, y0] = poly[i];
        let [x1, y1] = poly[(i + 1) % poly.len()];
        area += x0 * y1 - x1 * y0;
    }
    area * 0.5
}

fn ear_clip(poly: &[[f32; 2]]) -> Vec<[u32; 3]> {
    let mut remaining: Vec<usize> = (0..poly.len()).collect();
    let mut tris = Vec::new();
    let mut guard = poly.len() * poly.len();
    while remaining.len() > 3 && guard > 0 {
        guard -= 1;
        let n = remaining.len();
        let mut cut = None;
        for i in 0..n {
            let a = remaining[(i + n - 1) % n];
            let b = remaining[i];
            let c = remaining[(i + 1) % n];
            let [ax, ay] = poly[a];
            let [bx, by] = poly[b];
            let [cx, cy] = poly[c];
            let cross = (bx - ax) * (cy - ay) - (by - ay) * (cx - ax);
            if cross <= 0.0 {
                continue;
            }
            let mut inside = false;
            for &k in &remaining {
                if k == a || k == b || k == c {
                    continue;
                }
                if point_in_tri(poly[k], [ax, ay], [bx, by], [cx, cy]) {
                    inside = true;
                    break;
                }
            }
            if !inside {
                cut = Some(i);
                break;
            }
        }
        match cut {
            Some(i) => {
                let n = remaining.len();
                tris.push([remaining[(i + n - 1) % n] as u32, remaining[i] as u32, remaining[(i + 1) % n] as u32]);
                remaining.remove(i);
            }
            None => break,
        }
    }
    if remaining.len() == 3 {
        tris.push([remaining[0] as u32, remaining[1] as u32, remaining[2] as u32]);
    }
    tris
}

fn point_in_tri(p: [f32; 2], a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> bool {
    let d = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
    if d.abs() < 1e-9 {
        return false;
    }
    let l1 = ((b[1] - c[1]) * (p[0] - c[0]) + (c[0] - b[0]) * (p[1] - c[1])) / d;
    let l2 = ((c[1] - a[1]) * (p[0] - c[0]) + (a[0] - c[0]) * (p[1] - c[1])) / d;
    l1 > 0.0 && l2 > 0.0 && l1 + l2 < 1.0
}

fn sail_rows(start: f32, end: f32) -> usize {
    ((end - start) * 36.0).ceil().max(4.0) as usize
}

fn build_sail(part: &mut Part, side: f32, start: f32, end: f32, front: f32, back: f32, underside: bool) {
    let rows = sail_rows(start, end);
    let cols = 12usize;
    part.grid(rows, cols, |i, j| {
        let t = lerp(start, end, i as f32 / rows as f32);
        let c = lerp(front, back, j as f32 / cols as f32);
        let mut p = wing_point(side, t, c);
        if underside {
            p.y -= (c * std::f32::consts::PI).sin() * 0.2 * (1.0 - t);
        }
        (p, [t * 12.0, c])
    });
}

fn build_wing(parts: &mut Vec<Part>, side: f32) {
    let node = if side < 0.0 { Node::WingL } else { Node::WingR };
    let comp_x = side * 1.2;
    // Sail cloth in one part, graphite bits merged into one part.
    let mut sail = Part::new(node, MatId::Sail, comp_x, side);
    build_sail(&mut sail, side, 0.0, 0.89, 0.14, 0.78, false);
    build_sail(&mut sail, side, 0.0, 0.42, 0.78, 1.0, false);
    parts.push(sail);
    let mut dark = Part::new(node, MatId::Graphite, comp_x, side);
    build_sail(&mut dark, side, 0.0, 1.0, 0.0, 0.14, false);
    build_sail(&mut dark, side, 0.89, 1.0, 0.14, 1.0, false);
    build_sail(&mut dark, side, 0.0, 1.0, 0.0, 0.78, true);
    for i in 1..9 {
        let t = i as f32 / 10.0;
        let pts: Vec<Vec3> = (0..13)
            .map(|j| wing_point(side, t, j as f32 / 12.0 * 0.76) + Vec3::new(0.0, 0.004, 0.0))
            .collect();
        dark.tube(&pts, 0.005, 36, 5);
    }
    parts.push(dark);
    let tip = wing_point(side, 0.99, 0.35);
    let mut glow = Part::new(node, MatId::Glow, comp_x, side);
    glow.ellipsoid(tip, Vec3::new(0.07, 0.08, 0.14), 10, 7);
    parts.push(glow);
    // Feather ailerons: panel plus edge hardware per flap.
    for i in 0..3 {
        let start = 0.425 + i as f32 * 0.155;
        let end = start + 0.15;
        let pivot = wing_point(side, (start + end) / 2.0, 0.77);
        let id = (if side < 0.0 { 0 } else { 3 } + i) as u8;
        let mut flap = Part::new(Node::Flap(id), MatId::Graphite, comp_x + pivot.x, side);
        let rows = sail_rows(start, end);
        let cols = 6usize;
        flap.grid(rows, cols, |a, b| {
            let t = lerp(start, end, a as f32 / rows as f32);
            let c = lerp(0.79, 1.0, b as f32 / cols as f32);
            (wing_point(side, t, c) - pivot, [t * 12.0, c])
        });
        let edge: Vec<Vec3> = (0..10)
            .map(|j| wing_point(side, lerp(start, end, j as f32 / 9.0), 1.0) - pivot)
            .collect();
        flap.tube(&edge, 0.022, 24, 6);
        flap.ellipsoid(Vec3::new(0.0, -0.025, 0.0), Vec3::new(0.14, 0.055, 0.065), 10, 7);
        parts.push(flap);
    }
}

fn hull_profile() -> Vec<[f32; 2]> {
    let raw = [
        [0.0, -4.3],
        [0.14, -3.8],
        [0.38, -2.8],
        [0.57, -1.4],
        [0.59, -0.4],
        [0.48, 0.8],
        [0.38, 1.8],
        [0.26, 2.5],
        [0.0, 2.65],
    ];
    // Catmull-Rom through the profile for a smooth hull.
    let pts: Vec<Vec3> = raw.iter().map(|[r, z]| Vec3::new(*r, *z, 0.0)).collect();
    (0..=64)
        .map(|i| {
            let p = Part::catmull(&pts, i as f32 / 64.0);
            [p.x.max(0.0), p.y]
        })
        .collect()
}

fn build_hull(parts: &mut Vec<Part>) {
    let mut shell = Part::new(Node::Hull, MatId::Composite, 0.0, 0.0);
    shell.lathe_z(&hull_profile(), 48, 0.88);
    parts.push(shell);
    let mut graphite = Part::new(Node::Hull, MatId::Graphite, 0.0, 0.0);
    graphite.ellipsoid(Vec3::new(0.0, -0.24, -0.8), Vec3::new(0.49, 0.27, 2.8), 20, 12);
    for side in [-1.0f32, 1.0] {
        graphite.ellipsoid(
            Vec3::new(side * 0.59, 0.05, 0.2),
            Vec3::new(0.5, 0.22, 1.2),
            16,
            10,
        );
    }
    parts.push(graphite);
    let mut metal = Part::new(Node::Hull, MatId::Titanium, 0.0, 0.0);
    for side in [-1.0f32, 1.0] {
        metal.tube(
            &[
                Vec3::new(side * 0.08, 0.0, -4.05),
                Vec3::new(side * 0.43, 0.12, -2.6),
                Vec3::new(side * 0.56, 0.08, -0.8),
                Vec3::new(side * 0.4, 0.0, 1.4),
            ],
            0.025,
            30,
            6,
        );
    }
    parts.push(metal);
    let mut dark = Part::new(Node::Hull, MatId::Dark, 0.0, 0.0);
    for side in [-1.0f32, 1.0] {
        for i in 0..7 {
            dark.tube(
                &[
                    Vec3::new(side * 0.43, 0.32, 0.35 + i as f32 * 0.15),
                    Vec3::new(side * 0.59, 0.06, 0.42 + i as f32 * 0.15),
                ],
                0.032,
                8,
                5,
            );
        }
    }
    parts.push(dark);
}

fn build_canopy(parts: &mut Vec<Part>) {
    // Node origin sits at (0, 0.37, -1.25) in plane space.
    let off = Vec3::new(0.0, 0.37, -1.25);
    let at = |p: Vec3| p - off;
    let mut dark = Part::new(Node::Canopy, MatId::Dark, off.x, 0.0);
    dark.ellipsoid(at(Vec3::new(0.0, -0.15, 0.0)), Vec3::new(0.37, 0.16, 1.15), 18, 12);
    dark.ellipsoid(at(Vec3::new(0.0, 0.01, -0.62)), Vec3::new(0.3, 0.18, 0.16), 14, 10);
    parts.push(dark);
    let mut seat = Part::new(Node::Canopy, MatId::Seat, off.x, 0.0);
    seat.ellipsoid(at(Vec3::new(0.0, -0.04, 0.38)), Vec3::new(0.25, 0.25, 0.3), 12, 8);
    parts.push(seat);
    let mut glow = Part::new(Node::Canopy, MatId::Glow, off.x, 0.0);
    for i in -1..=1 {
        glow.ellipsoid(
            at(Vec3::new(i as f32 * 0.14, 0.15, -0.52)),
            Vec3::new(0.043, 0.045, 0.016),
            8,
            6,
        );
    }
    parts.push(glow);
    let mut glass = Part::new(Node::Canopy, MatId::Glass, off.x, 0.0);
    glass.ellipsoid(at(Vec3::new(0.0, 0.12, 0.0)), Vec3::new(0.385, 0.43, 1.22), 24, 14);
    parts.push(glass);
    let mut frame = Part::new(Node::Canopy, MatId::Titanium, off.x, 0.0);
    for side in [-1.0f32, 1.0] {
        frame.tube(
            &[
                at(Vec3::new(0.0, 0.06, -1.22)),
                at(Vec3::new(side * 0.33, 0.12, -0.6)),
                at(Vec3::new(side * 0.37, 0.1, 0.2)),
                at(Vec3::new(0.0, 0.05, 1.22)),
            ],
            0.028,
            24,
            6,
        );
    }
    let hoop: Vec<Vec3> = (0..17)
        .map(|i| {
            let a = i as f32 / 16.0 * std::f32::consts::PI;
            at(Vec3::new(a.cos() * 0.389 * 0.89, 0.12 + a.sin() * 0.435 * 0.89, 0.55))
        })
        .collect();
    frame.tube(&hoop, 0.025, 30, 6);
    frame.tube(
        &[
            at(Vec3::new(0.0, 0.12, -1.22)),
            at(Vec3::new(0.0, 0.53, -0.3)),
            at(Vec3::new(0.0, 0.5, 0.4)),
            at(Vec3::new(0.0, 0.12, 1.22)),
        ],
        0.017,
        24,
        5,
    );
    parts.push(frame);
}

fn build_engine(parts: &mut Vec<Part>) {
    // Engine group origin sits at (0, 0.34, 1.35) in original space.
    // Static parts bake the mirrored offset. Rotor and petals animate.
    let off = Vec3::new(0.0, 0.34, 1.35);
    // Shell and liner are positioned in engine space; bake the offset.
    let mut shell = Part::new(Node::Hull, MatId::Dark, 0.0, 0.0);
    shell.cylinder_z(0.54, 0.46, 0.65 - 0.7, 0.65 + 0.7, 40, false);
    for v in &mut shell.verts {
        v.pos[0] += off.x;
        v.pos[1] += off.y;
        v.pos[2] += -(off.z);
    }
    parts.push(shell);
    let mut rings = Part::new(Node::Hull, MatId::Titanium, 0.0, 0.0);
    for (z, r) in [(-0.08, 0.55), (1.25, 0.55), (1.44, 0.48)] {
        rings.torus(r, 0.045, z + off.z, 40, 8);
    }
    parts.push(rings);
    let mut glow_ring = Part::new(Node::Hull, MatId::Glow, 0.0, 0.0);
    glow_ring.torus(0.43, 0.015, 1.37 + off.z, 32, 6);
    parts.push(glow_ring);
    let mut fairing = Part::new(Node::Hull, MatId::Graphite, 0.0, 0.0);
    fairing.lathe_z(
        &[
            [0.34, -0.28 + off.z],
            [0.51, -0.12 + off.z],
            [0.58, 0.25 + off.z],
            [0.56, 0.8 + off.z],
            [0.49, 1.25 + off.z],
        ],
        48,
        1.0,
    );
    for v in &mut fairing.verts {
        v.pos[1] += off.y;
    }
    parts.push(fairing);
    let mut struts = Part::new(Node::Hull, MatId::Dark, 0.0, 0.0);
    let mut knobs = Part::new(Node::Hull, MatId::Titanium, 0.0, 0.0);
    for i in 0..32 {
        let a = i as f32 / 32.0 * std::f32::consts::TAU;
        struts.tube(
            &[
                Vec3::new(a.cos() * 0.565, a.sin() * 0.565 + off.y, 0.52 + off.z),
                Vec3::new(a.cos() * 0.54, a.sin() * 0.54 + off.y, 0.89 + off.z),
            ],
            0.009,
            6,
            4,
        );
        knobs.ellipsoid(
            Vec3::new(a.cos() * 0.51, a.sin() * 0.51 + off.y, 1.17 + off.z),
            Vec3::new(0.014, 0.014, 0.014),
            6,
            4,
        );
    }
    parts.push(struts);
    parts.push(knobs);
    // Rotor spins around z in engine space; parts stay centered on it.
    let mut rotor_glow = Part::new(Node::Rotor, MatId::Glow, 0.0, 0.0);
    rotor_glow.ellipsoid(Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.19, 0.19, 0.27), 12, 8);
    rotor_glow.torus(0.33, 0.016, 0.10, 28, 6);
    parts.push(rotor_glow);
    let mut rotor = Part::new(Node::Rotor, MatId::Titanium, 0.0, 0.0);
    for i in 0..24 {
        let a = i as f32 / 24.0 * std::f32::consts::TAU;
        rotor.tube(
            &[
                Vec3::new(a.cos() * 0.18, a.sin() * 0.18, 0.03),
                Vec3::new((a + 0.3).cos() * 0.31, (a + 0.3).sin() * 0.31, 0.09),
                Vec3::new((a + 0.55).cos() * 0.43, (a + 0.55).sin() * 0.43, 0.02),
            ],
            0.019,
            12,
            5,
        );
        let quad = [
            Vec3::new(a.cos() * 0.17, a.sin() * 0.17, -0.09),
            Vec3::new((a + 0.35).cos() * 0.43, (a + 0.35).sin() * 0.43, -0.02),
            Vec3::new((a + 0.49).cos() * 0.42, (a + 0.49).sin() * 0.42, 0.11),
            Vec3::new((a + 0.14).cos() * 0.17, (a + 0.14).sin() * 0.17, 0.05),
        ];
        rotor.quad(quad);
    }
    rotor.ellipsoid(Vec3::new(0.0, 0.0, -0.02), Vec3::new(0.16, 0.16, 0.24), 12, 8);
    parts.push(rotor);
    let mut liner = Part::new(Node::Hull, MatId::Dark, 0.0, 0.0);
    liner.cylinder_z(0.39, 0.43, 1.03 - 0.24 + off.z, 1.03 + 0.24 + off.z, 40, true);
    parts.push(liner);
    // Petals live in petal space. The node transform carries the
    // hinge position, the hinge cant, and the deploy angle.
    for i in 0..10 {
        let mut petal = Part::new(Node::Petal(i), MatId::Titanium, 0.0, 0.0);
        petal.ellipsoid(Vec3::new(0.0, 0.0, 0.28), Vec3::new(0.125, 0.045, 0.38), 10, 7);
        petal.tube(
            &[Vec3::new(0.0, 0.04, 0.0), Vec3::new(0.0, 0.048, 0.3), Vec3::new(0.0, 0.015, 0.63)],
            0.014,
            12,
            5,
        );
        parts.push(petal);
    }
}

fn fin_outline() -> Vec<[f32; 2]> {
    // Bezier outline sampled, matching the prototype fin shape.
    let mut pts = Vec::new();
    let cubic = |p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], p3: [f32; 2], n: usize| {
        (0..=n)
            .map(|i| {
                let t = i as f32 / n as f32;
                let u = 1.0 - t;
                [
                    u * u * u * p0[0] + 3.0 * u * u * t * p1[0] + 3.0 * u * t * t * p2[0] + t * t * t * p3[0],
                    u * u * u * p0[1] + 3.0 * u * u * t * p1[1] + 3.0 * u * t * t * p2[1] + t * t * t * p3[1],
                ]
            })
            .collect::<Vec<_>>()
    };
    pts.extend(cubic([0.0, -0.6], [0.7, -0.6], [1.45, 0.25], [1.75, 0.8], 12));
    pts.extend(cubic([1.75, 0.8], [1.3, 0.85], [0.55, 0.6], [0.0, 0.55], 12).into_iter().skip(1));
    pts
}

fn build_tail(parts: &mut Vec<Part>) {
    // Tail group origin sits at (0, 0.2, 2.5) in original space.
    // Static booms bake it. Fins animate on their own nodes.
    let off = Vec3::new(0.0, 0.2, 2.5);
    let at = |p: Vec3| p + off;
    for (fi, side) in [-1.0f32, 1.0].iter().enumerate() {
        let mut booms = Part::new(Node::Hull, MatId::Graphite, 0.0, 0.0);
        booms.tube(
            &[
                at(Vec3::new(side * 0.7, -0.12, -1.4)),
                at(Vec3::new(side * 0.9, 0.05, 0.3)),
                at(Vec3::new(side * 0.65, 0.65, 2.7)),
            ],
            0.075,
            24,
            7,
        );
        parts.push(booms);
        let mut brace = Part::new(Node::Hull, MatId::Titanium, 0.0, 0.0);
        brace.tube(
            &[
                at(Vec3::new(side * 0.7, -0.2, -1.4)),
                at(Vec3::new(side * 0.76, -0.18, 0.8)),
                at(Vec3::new(side * 0.65, 0.65, 2.7)),
            ],
            0.023,
            16,
            5,
        );
        parts.push(brace);
        // Fin node: fixed cant plus animated pitch.
        let fin_pos = at(Vec3::new(side * 0.65, 0.65, 2.0));
        let _ = fin_pos;
        let mut fin = Part::new(Node::Fin(fi as u8), MatId::Graphite, 0.0, 0.0);
        let outline = fin_outline();
        // Outline lives in fin space; rotate for the left side like the prototype.
        let plate: Vec<[f32; 2]> = outline
            .iter()
            .map(|[x, y]| if *side < 0.0 { [-x, *y] } else { [*x, *y] })
            .collect();
        fin.extrude(&plate, -0.028, 0.028);
        parts.push(fin);
        let mut spar = Part::new(Node::Fin(fi as u8), MatId::Titanium, 0.0, 0.0);
        spar.tube(
            &[
                Vec3::new(0.0, 0.06, -0.6),
                Vec3::new(side * 0.65, 0.06, -0.3),
                Vec3::new(side * 1.75, 0.06, 0.8),
            ],
            0.028,
            16,
            5,
        );
        for i in 1..5 {
            spar.tube(
                &[
                    Vec3::new(side * i as f32 * 0.27, 0.06, -0.45 + i as f32 * 0.15),
                    Vec3::new(side * i as f32 * 0.27, 0.06, 0.58),
                ],
                0.013,
                6,
                4,
            );
        }
        parts.push(spar);
    }
}

pub fn build_airframe() -> Vec<RawPart> {
    let mut parts: Vec<Part> = Vec::new();
    build_hull(&mut parts);
    build_canopy(&mut parts);
    build_wing(&mut parts, -1.0);
    build_wing(&mut parts, 1.0);
    build_engine(&mut parts);
    build_tail(&mut parts);
    parts
        .into_iter()
        .map(|p| RawPart { node: p.node, mat: p.mat, verts: p.verts, idx: p.idx })
        .collect()
}
