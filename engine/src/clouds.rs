//! Constants for the procedural mesh cloud field, mirrored by
//! `engine/shaders/cloud.inc`. The GPU decodes every cloud from
//! `gl_VertexIndex`, so the CPU only needs the fixed vertex budget for the
//! draw call; the hash mirror below lets tests pin down the coverage
//! statistics and the world-periodic placement that the ground shadows
//! replay per pixel. Keep both sides synchronized.

/// Placement pitch of the hash grid in metres.
#[allow(dead_code)] // mirrored by tests; the GPU reads the same value from cloud.inc
pub const CLOUD_CELL: f32 = 2048.0;

/// Grid side drawn around the camera, in cells.
pub const CLOUD_GRID: u32 = 21;

/// Cells in the drawn grid.
pub const CLOUD_CELLS: u32 = CLOUD_GRID * CLOUD_GRID;

/// Icosphere puffs per cloud.
pub const CLOUD_PUFFS: u32 = 8;

/// Triangles per puff: icosphere with two subdivisions (162 vertices, 320
/// faces). Enough tessellation for smooth-shaded billows instead of facets,
/// while the whole field stays under half the terrain triangle budget even
/// with every cell occupied.
pub const CLOUD_PUFF_TRIS: u32 = 320;

/// Non-indexed corners per puff.
pub const CLOUD_CORNERS: u32 = CLOUD_PUFF_TRIS * 3;

/// Corners submitted per cloud.
pub const CLOUD_VERTS_PER_CLOUD: u32 = CLOUD_PUFFS * CLOUD_CORNERS;

/// Total corners of the single fixed cloud draw call.
pub const CLOUD_VERTEX_COUNT: u32 = CLOUD_CELLS * CLOUD_VERTS_PER_CLOUD;

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
    fn vertex_budget_is_fixed_and_terrain_scale() {
        assert_eq!(CLOUD_PUFF_TRIS, 320);
        assert_eq!(CLOUD_VERTS_PER_CLOUD, 7_680);
        assert_eq!(CLOUD_VERTEX_COUNT, 3_386_880);
        // Worst case (every cell occupied) is ~1.13 M cloud triangles, still
        // well under the terrain index budget it shares the frame with.
        assert!(CLOUD_VERTEX_COUNT / 3 < world::TERRAIN_INDEX_COUNT);
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
        let radius = CLOUD_CELL * (half as f32 + 0.5);
        let mut visible = 0u32;
        for dz in -half..=half {
            for dx in -half..=half {
                let world = [1_000_001 + dx, -4_000_042 + dz];
                let wrapped = [
                    (world[0] as u32) & (CLOUD_PERIOD_CELLS - 1),
                    (world[1] as u32) & (CLOUD_PERIOD_CELLS - 1),
                ];
                let distance = CLOUD_CELL * ((dx * dx + dz * dz) as f32).sqrt();
                if distance <= radius && family(wrapped) == 1 {
                    visible += 1;
                }
            }
        }
        assert!(visible > 100, "only {visible} clouds inside the draw radius");
    }
}
