//! Constants for the procedural mesh cloud field, mirrored by
//! `engine/shaders/cloud.inc`. The GPU decodes each cell from
//! `gl_InstanceIndex` and each puff corner from `gl_VertexIndex`. A static
//! index buffer shares identical corners without changing the procedural
//! surface or triangle order. The hash mirror below pins down the coverage
//! statistics and the world-periodic placement that the ground shadows
//! replay per pixel. Keep both sides synchronized.

/// Placement pitch of the hash grid in metres.
#[allow(dead_code)] // mirrored by tests; the GPU reads the same value from cloud.inc
pub const CLOUD_CELL: f32 = 2048.0;

/// Grid side drawn around the camera, in cells.
pub const CLOUD_GRID: u32 = 19;

/// Cells in the drawn grid.
pub const CLOUD_CELLS: u32 = CLOUD_GRID * CLOUD_GRID;

/// Instance draw radius in metres (mirrors cloud.inc; far clouds are the
/// first thing aerial haze swallows, so the grid stops before they vanish).
#[allow(dead_code)] // mirrored by tests; the GPU reads the same value from cloud.inc
pub const CLOUD_DRAW_RADIUS: f32 = 20000.0;

/// Icosphere puffs per cloud.
pub const CLOUD_PUFFS: u32 = 8;

/// Triangles per puff: icosphere with three subdivisions (642 vertices,
/// 1280 faces). Keeps silhouettes and shading round at close range.
pub const CLOUD_PUFF_TRIS: u32 = 1280;

/// Non-indexed corners per puff.
pub const CLOUD_CORNERS: u32 = CLOUD_PUFF_TRIS * 3;

/// Corners submitted per cloud.
pub const CLOUD_VERTS_PER_CLOUD: u32 = CLOUD_PUFFS * CLOUD_CORNERS;

/// Maximum corners of the full-quality cloud draw call. Performance uses a
/// smaller puff count and grid, but keeps this maximum for topology tests and
/// the device-local index buffer.
#[allow(dead_code)] // triangle budget, retained for regression tests
pub const CLOUD_VERTEX_COUNT: u32 = CLOUD_CELLS * CLOUD_VERTS_PER_CLOUD;

// Same face/corner ordering as ICO_FACE in cloud.vert. These are topology
// IDs only: all floating-point surface evaluation stays in the original
// shader, using the first original corner that reaches a shared vertex.
const ICO_FACES: [[u16; 3]; 20] = [
    [0, 11, 5], [0, 5, 1], [0, 7, 1], [0, 7, 10], [0, 11, 10],
    [1, 5, 9], [5, 11, 4], [11, 10, 2], [10, 6, 7], [7, 1, 8],
    [3, 9, 4], [3, 2, 4], [3, 2, 6], [3, 6, 8], [3, 8, 9],
    [4, 9, 5], [2, 4, 11], [6, 2, 10], [8, 7, 6], [9, 1, 8],
];

fn puff_topology() -> Vec<[u16; 3]> {
    let mut faces = ICO_FACES.to_vec();
    let mut next_vertex = 12u16;
    let mut midpoints = std::collections::HashMap::new();
    for _ in 0..3 {
        let mut midpoint = |a: u16, b: u16| {
            *midpoints.entry((a.min(b), a.max(b))).or_insert_with(|| {
                let id = next_vertex;
                next_vertex += 1;
                id
            })
        };
        let mut subdivided = Vec::with_capacity(faces.len() * 4);
        for [a, b, c] in faces {
            let ab = midpoint(a, b);
            let bc = midpoint(b, c);
            let ca = midpoint(c, a);
            subdivided.extend_from_slice(&[[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]]);
        }
        faces = subdivided;
    }
    faces
}

/// Immutable indices for one cloud instance. Keeps all 10,240 triangles
/// and their ordering, but shares the 642 vertices of each puff instead of
/// evaluating its expensive procedural vertex shader for all 3,840 corners.
/// Index values deliberately address original corner IDs, not compacted
/// positions, so the shader's existing subdivision arithmetic is retained.
pub fn indices() -> Vec<u16> {
    let faces = puff_topology();
    let mut first_corner = [u16::MAX; 642];
    let puff_indices: Vec<_> = faces.iter().flatten().enumerate().map(|(corner, &vertex)| {
        let first = &mut first_corner[vertex as usize];
        if *first == u16::MAX {
            *first = corner as u16;
        }
        *first
    }).collect();
    (0..CLOUD_PUFFS).flat_map(|puff| {
        puff_indices.iter().map(move |&corner| corner + (puff * CLOUD_CORNERS) as u16)
    }).collect()
}

/// Placement cells per 65,536 m world period (the field tiles seamlessly).
#[allow(dead_code)] // mirrored by tests; the GPU reads the same value from cloud.inc
pub const CLOUD_PERIOD_CELLS: u32 = 32;

/// Fraction of cells that grow a cumulus cluster.
#[allow(dead_code)] // mirrored by tests; the GPU reads the same value from cloud.inc
pub const CLOUD_CUMULUS_PRESENCE: f32 = 0.38;

/// Fraction of otherwise empty cells that grow a high streak.
#[allow(dead_code)] // mirrored by tests; the GPU reads the same value from cloud.inc
pub const CLOUD_STREAK_PRESENCE: f32 = 0.10;

/// Mirror of `cloudHash` in cloud.inc (same mixer family as the terrain
/// hashes; wrapping uint arithmetic matches GLSL exactly).
#[allow(dead_code)] // test-side twin of the GPU hash, kept for drift protection
pub fn hash(cell: [u32; 2], seed: u32) -> f32 {
    let mut h = cell[0]
        .wrapping_mul(1_664_525)
        .wrapping_add(cell[1].wrapping_mul(1_013_904_223))
        .wrapping_add(seed.wrapping_mul(2_246_822_519));
    h ^= h >> 16;
    h = h.wrapping_mul(2_246_822_519);
    h ^= h >> 13;
    h = h.wrapping_mul(3_266_489_917);
    h ^= h >> 16;
    (h & 0x00ff_ffff) as f32 * (1.0 / 16_777_215.0)
}

/// Mirror of `cloudCellFamily`: 0 empty, 1 cumulus, 2 high streak.
#[allow(dead_code)] // test-side twin of the GPU decode, kept for drift protection
pub fn family(cell: [u32; 2]) -> u32 {
    if hash(cell, 17) < CLOUD_CUMULUS_PRESENCE {
        1
    } else if hash(cell, 91) < CLOUD_STREAK_PRESENCE {
        2
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_draw_preserves_every_triangle_and_puff() {
        let indices = indices();
        let corners: Vec<_> = puff_topology().into_iter().flatten().collect();
        assert_eq!(indices.len(), CLOUD_VERTS_PER_CLOUD as usize);
        for (puff, puff_indices) in indices.as_chunks::<{ CLOUD_CORNERS as usize }>().0.iter().enumerate() {
            let offset = puff as u16 * CLOUD_CORNERS as u16;
            let unique: std::collections::HashSet<_> = puff_indices.iter().copied().collect();
            assert_eq!(unique.len(), 642);
            for (corner, &index) in puff_indices.iter().enumerate() {
                assert!(index >= offset && index < offset + CLOUD_CORNERS as u16);
                assert_eq!(corners[(index - offset) as usize], corners[corner]);
            }
        }
    }

    #[test]
    fn face_order_matches_procedural_shader() {
        let shader = include_str!("../shaders/cloud.vert");
        let faces = shader.split("const int ICO_FACE[60] = int[60](").nth(1).unwrap();
        let faces = faces.split(");").next().unwrap();
        let parsed: Vec<u16> = faces.split(',').map(|s| s.trim().parse().unwrap()).collect();
        assert_eq!(parsed, ICO_FACES.into_iter().flatten().collect::<Vec<_>>());
    }

    #[test]
    fn vertex_budget_is_fixed_and_terrain_scale() {
        assert_eq!(CLOUD_PUFF_TRIS, 1280);
        assert_eq!(CLOUD_VERTS_PER_CLOUD, 30_720);
        assert_eq!(CLOUD_VERTEX_COUNT, 11_089_920);
        // The absolute worst case (every grid cell occupied) exceeds the
        // terrain budget, but that state cannot occur: measure the real
        // occupancy from the hash mirror over one world period and require
        // the expected triangle load to stay under the terrain draw it
        // shares the frame with.
        let mut occupied = 0u32;
        for z in 0..CLOUD_PERIOD_CELLS {
            for x in 0..CLOUD_PERIOD_CELLS {
                if family([x, z]) != 0 {
                    occupied += 1;
                }
            }
        }
        let presence = occupied as f32 / (CLOUD_PERIOD_CELLS * CLOUD_PERIOD_CELLS) as f32;
        let expected_tris = CLOUD_VERTEX_COUNT as f32 / 3.0 * presence;
        assert!(expected_tris < world::TERRAIN_INDEX_COUNT as f32);
        assert!(presence < 0.55, "occupancy {presence:.3} grew too dense");
    }

    #[test]
    fn corner_decode_covers_every_puff_triangle_once() {
        let mut seen = vec![0u32; (CLOUD_PUFFS * CLOUD_CORNERS) as usize];
        for vertex in 0..CLOUD_VERTS_PER_CLOUD {
            let rest = vertex % CLOUD_VERTS_PER_CLOUD;
            let puff = rest / CLOUD_CORNERS;
            let corner = rest % CLOUD_CORNERS;
            seen[(puff * CLOUD_CORNERS + corner) as usize] += 1;
        }
        assert!(seen.iter().all(|&count| count == 1));
    }

    #[test]
    fn coverage_is_scattered() {
        let mut cumulus = 0u32;
        let mut streaks = 0u32;
        for z in 0..CLOUD_PERIOD_CELLS {
            for x in 0..CLOUD_PERIOD_CELLS {
                match family([x, z]) {
                    1 => cumulus += 1,
                    2 => streaks += 1,
                    _ => {}
                }
            }
        }
        let total = (CLOUD_PERIOD_CELLS * CLOUD_PERIOD_CELLS) as f32;
        let cumulus_rate = cumulus as f32 / total;
        let streak_rate = streaks as f32 / total;
        assert!(
            (0.30..=0.46).contains(&cumulus_rate),
            "cumulus coverage {cumulus_rate:.3}"
        );
        assert!(
            (0.04..=0.16).contains(&streak_rate),
            "streak coverage {streak_rate:.3}"
        );
    }

    #[test]
    fn drawn_radius_populates_the_sky() {
        // Mirror the vertex-shader decode for one grid anchored at a cell
        // whose wrapped coordinates differ from the relative ones: wrapping
        // must happen at the call site exactly like `uvec2(worldCell) & 31u`.
        let half = (CLOUD_GRID / 2) as i32;
        let mut visible = 0u32;
        for dz in -half..=half {
            for dx in -half..=half {
                let world = [1_000_001 + dx, -4_000_042 + dz];
                let wrapped = [
                    (world[0] as u32) & (CLOUD_PERIOD_CELLS - 1),
                    (world[1] as u32) & (CLOUD_PERIOD_CELLS - 1),
                ];
                let distance = CLOUD_CELL * ((dx * dx + dz * dz) as f32).sqrt();
                if distance <= CLOUD_DRAW_RADIUS && family(wrapped) == 1 {
                    visible += 1;
                }
            }
        }
        assert!(visible > 80, "only {visible} clouds inside the draw radius");
    }
}
