//! Deterministic alpine terrain shared by flight clearance and the ground shader.
//!
//! Heights are metres above sea level. Query with absolute world X/Z, never a
//! camera-relative position. The 65.536 km period is deliberate: reducing f64
//! coordinates before converting to f32 preserves local precision after rebasing.
//! The terrain recipe is also emitted to `world_generated.inc` for the shader
//! build. Keep CPU and GPU world data in this crate; do not hand-edit a second
//! structure table in GLSL.

use std::fmt::Write as _;
use std::sync::OnceLock;

pub const WORLD_PERIOD: f64 = 65_536.0;
pub const WATER_LEVEL: f32 = 185.0;
pub const MAX_TERRAIN_HEIGHT: f32 = 3_123.0;
pub const TERRAIN_GRID_CELLS: u32 = 1024;
pub const TERRAIN_CELL_METRES: f32 = 64.0;
pub const TERRAIN_VERTEX_COUNT: u32 = (TERRAIN_GRID_CELLS + 1) * (TERRAIN_GRID_CELLS + 1);
pub const TERRAIN_INDEX_COUNT: u32 = TERRAIN_GRID_CELLS * TERRAIN_GRID_CELLS * 6;
pub const TERRAIN_CHUNK_CELLS: u32 = 32;
pub const TERRAIN_CHUNKS_PER_AXIS: u32 = TERRAIN_GRID_CELLS / TERRAIN_CHUNK_CELLS;
pub const TERRAIN_CHUNK_COUNT: u32 = TERRAIN_CHUNKS_PER_AXIS * TERRAIN_CHUNKS_PER_AXIS;
pub const TERRAIN_CHUNK_INDICES: u32 = TERRAIN_CHUNK_CELLS * TERRAIN_CHUNK_CELLS * 6;
// Performance terrain reuses the same 1025x1025 height texture and vertex
// shader, but walks every other lattice vertex. At the reduced scene scale a
// 128 m triangle is below the useful silhouette/detail frequency outside the
// immediate flight bubble; the full 64 m mesh remains available for RT BLAS.
pub const PERFORMANCE_TERRAIN_STEP: u32 = 2;
pub const PERFORMANCE_TERRAIN_CHUNKS_PER_AXIS: u32 = TERRAIN_CHUNKS_PER_AXIS / PERFORMANCE_TERRAIN_STEP;
pub const PERFORMANCE_TERRAIN_CHUNK_COUNT: u32 =
    PERFORMANCE_TERRAIN_CHUNKS_PER_AXIS * PERFORMANCE_TERRAIN_CHUNKS_PER_AXIS;
pub const PERFORMANCE_TERRAIN_CHUNK_INDICES: u32 = TERRAIN_CHUNK_INDICES;
// Keep the full mesh in the inner 16x16 chunks (32 km across); only the
// atmospheric outer ring uses the coarse topology. The command count stays
// bounded and the near flight bubble keeps the original silhouette fidelity.
pub const PERFORMANCE_TERRAIN_NEAR_FULL_CHUNKS: u32 = 16;
pub const PERFORMANCE_TERRAIN_COMMAND_COUNT: u32 =
    PERFORMANCE_TERRAIN_NEAR_FULL_CHUNKS * PERFORMANCE_TERRAIN_NEAR_FULL_CHUNKS
        + PERFORMANCE_TERRAIN_CHUNK_COUNT
        - (PERFORMANCE_TERRAIN_NEAR_FULL_CHUNKS / PERFORMANCE_TERRAIN_STEP)
            * (PERFORMANCE_TERRAIN_NEAR_FULL_CHUNKS / PERFORMANCE_TERRAIN_STEP);
pub const LANDMARK_STRUCTURES: u32 = 52;
/// Per-structure vertex budget: a wall box (36), a roof prism (18), three
/// detail boxes (36 each), a roofline band (36), and a roof tip (24). Every
/// structure owns the same slot count so the fixed vertex-pulled draw decodes
/// without per-slot bookkeeping; empty add-ons collapse to degenerate
/// triangles. Mirrored in `terrain.inc`.
pub const STRUCTURE_WALL_VERTICES: u32 = 36;
pub const STRUCTURE_ROOF_VERTICES: u32 = 18;
pub const STRUCTURE_DETAIL_VERTICES: u32 = 36;
pub const STRUCTURE_BAND_VERTICES: u32 = 36;
pub const STRUCTURE_TIP_VERTICES: u32 = 24;
pub const STRUCTURE_VERTICES: u32 = STRUCTURE_WALL_VERTICES
    + STRUCTURE_ROOF_VERTICES
    + STRUCTURE_DETAIL_VERTICES
    + STRUCTURE_DETAIL_VERTICES
    + STRUCTURE_DETAIL_VERTICES
    + STRUCTURE_BAND_VERTICES
    + STRUCTURE_TIP_VERTICES;
pub const LANDMARK_VERTEX_COUNT: u32 = 9 * LANDMARK_STRUCTURES * STRUCTURE_VERTICES;
/// Legacy source-lattice constants for deterministic tree collision and the
/// startup vegetation database. They are no longer a per-frame vertex budget:
/// the renderer uploads accepted instances and draws them separately.
pub const SCATTER_GRID: u32 = 121;
pub const SCATTER_PITCH: f32 = 24.0;
pub const SCATTER_CORNERS_PER_SLOT: u32 = 216;
pub const SCATTER_VERTEX_COUNT: u32 = SCATTER_GRID * SCATTER_GRID * SCATTER_CORNERS_PER_SLOT;
/// Only landmarks remain in the terrain index stream. Vegetation is stored in
/// a compact instance database and issued through its own indirect draw.
pub const GROUND_FEATURE_INDEX_COUNT: u32 = LANDMARK_VERTEX_COUNT;
pub const DRAW_INDEX_COUNT: u32 = TERRAIN_INDEX_COUNT + GROUND_FEATURE_INDEX_COUNT;
pub const CLEARANCE_METRES: f32 = 45.0;

pub mod vegetation;
pub const SPAWN_X: f32 = 0.0;
pub const SPAWN_Z: f32 = 1_050.0;
pub const SPAWN_ALTITUDE: f32 = 1_100.0;
const SETTLEMENT_SPACING: f32 = 16_384.0;

/// Immutable topology. Adjacent triangles reuse the same height/normal fetch.
pub fn terrain_indices() -> Vec<u32> {
    let mut indices = terrain_indices_with_step(1);
    indices.extend(TERRAIN_VERTEX_COUNT..TERRAIN_VERTEX_COUNT + GROUND_FEATURE_INDEX_COUNT);
    indices
}

/// Reduced terrain topology for the Performance preset. Indices still point
/// into the canonical vertex-pulled 1025x1025 lattice, so the shader's world
/// addressing, rebasing, and material recipe remain shared with full quality.
pub fn performance_terrain_indices() -> Vec<u32> {
    terrain_indices_with_step(PERFORMANCE_TERRAIN_STEP)
}

fn terrain_indices_with_step(step: u32) -> Vec<u32> {
    assert!(step > 0 && TERRAIN_GRID_CELLS % (TERRAIN_CHUNK_CELLS * step) == 0);
    let chunks_per_axis = TERRAIN_CHUNKS_PER_AXIS / step;
    let cells_per_chunk = TERRAIN_CHUNK_CELLS;
    let index_count = chunks_per_axis * chunks_per_axis * cells_per_chunk
        * cells_per_chunk * 6;
    let mut indices = Vec::with_capacity(index_count as usize);
    let stride = TERRAIN_GRID_CELLS + 1;
    for cz in 0..chunks_per_axis {
        for cx in 0..chunks_per_axis {
            for z in 0..cells_per_chunk {
                for x in 0..cells_per_chunk {
                    let lattice_z = (cz * cells_per_chunk + z) * step;
                    let lattice_x = (cx * cells_per_chunk + x) * step;
                    let a = lattice_z * stride + lattice_x;
                    indices.extend_from_slice(&[
                        a,
                        a + step * stride,
                        a + step,
                        a + step,
                        a + step * stride,
                        a + step * stride + step,
                    ]);
                }
            }
        }
    }
    indices
}

/// One periodic, immutable height/normal cache, sampled at world lattice points.
/// RGB stores land height and the two surface slopes; water has a flat normal.
/// W stores landform moisture in [0,1]: valley floors and hollows hold water,
/// steep high ground sheds it. Local proxy for the topographic wetness index
/// ln(a/tan beta) (Beven & Kirkby 1979): height above water stands in for
/// upslope contributing area, local slope for tan beta, and smoothed-profile
/// concavity marks hollows. Full flow routing is skipped: the tile is periodic
/// and the cache builds once at startup.
fn build_terrain_samples() -> Vec<[f32; 4]> {
    let n = TERRAIN_GRID_CELLS as usize;
    let step = TERRAIN_CELL_METRES as f64;
    let heights: Vec<f32> = (0..n * n)
        .map(|i| height_at((i % n) as f64 * step, (i / n) as f64 * step))
        .collect();
    let surface = |x: usize, z: usize| heights[(z % n) * n + x % n].max(WATER_LEVEL);
    // Micro relief (128 m octave and below) would dominate a raw laplacian and
    // turn moisture back into fine noise. A 5x5 box blur keeps valley/ridge
    // structure while discarding sub-300 m roughness.
    let blurred: Vec<f32> = (0..n * n)
        .map(|i| {
            let x = i % n;
            let z = i / n;
            let mut acc = 0.0f32;
            for dz in 0..5 {
                for dx in 0..5 {
                    acc += surface((x + n + dx - 2) % n, (z + n + dz - 2) % n);
                }
            }
            acc / 25.0
        })
        .collect();
    let smooth_h = |x: usize, z: usize| blurred[(z % n) * n + x % n];
    (0..n * n)
        .map(|i| {
            let x = i % n;
            let z = i / n;
            let dzdx = (surface(x + 1, z) - surface(x + n - 1, z)) / (2.0 * TERRAIN_CELL_METRES);
            let dzdz = (surface(x, z + 1) - surface(x, z + n - 1)) / (2.0 * TERRAIN_CELL_METRES);
            let c = smooth_h(x, z);
            let slope = ((smooth_h(x + 1, z) - smooth_h(x + n - 1, z)).powi(2)
                + (smooth_h(x, z + 1) - smooth_h(x, z + n - 1)).powi(2))
            .sqrt()
                / (2.0 * TERRAIN_CELL_METRES);
            let laplacian = (smooth_h(x + 1, z)
                + smooth_h(x + n - 1, z)
                + smooth_h(x, z + 1)
                + smooth_h(x, z + n - 1)
                - 4.0 * c)
                / (TERRAIN_CELL_METRES * TERRAIN_CELL_METRES);
            [heights[i], dzdx, dzdz, moisture_at(c, slope, laplacian)]
        })
        .collect()
}

/// Permanently retained periodic terrain cache shared by rendering, collision,
/// scatter generation, and ray-tracing mesh construction. Keeping one 16 MiB
/// copy avoids regenerating the same 5x5-filtered landform samples in each
/// subsystem and gives hot collision queries direct indexed access.
pub fn terrain_samples_static() -> &'static [[f32; 4]] {
    static SAMPLES: OnceLock<Vec<[f32; 4]>> = OnceLock::new();
    SAMPLES.get_or_init(build_terrain_samples).as_slice()
}

/// Owned compatibility form for callers that need to retain or mutate their
/// own sample storage. New runtime paths should use [`terrain_samples_static`].
pub fn terrain_samples() -> Vec<[f32; 4]> {
    terrain_samples_static().to_vec()
}

/// Landform moisture from smoothed height `h`, slope magnitude, and profile
/// concavity (positive in hollows). Valley floors approach 1, dry ridges
/// approach the 0.08 floor, and hollow gullies gain up to 0.45 over the same
/// altitude and slope.
fn moisture_at(surface_h: f32, slope: f32, laplacian: f32) -> f32 {
    let above_water = (surface_h - WATER_LEVEL).max(0.0);
    let valley = (-above_water / 250.0).exp();
    let drain = 1.0 - smooth(0.45, 1.0, slope);
    let hollow = smooth(-0.0003, 0.0012, laplacian);
    (0.75 * valley * drain + 0.45 * hollow + 0.08).clamp(0.0, 1.0)
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn hash(x: u32, z: u32, seed: u32) -> f32 {
    let mut h = x
        .wrapping_mul(1_664_525)
        .wrapping_add(z.wrapping_mul(1_013_904_223))
        .wrapping_add(seed.wrapping_mul(2_246_822_519));
    h ^= h >> 16;
    h = h.wrapping_mul(2_246_822_519);
    h ^= h >> 13;
    h = h.wrapping_mul(3_266_489_917);
    h ^= h >> 16;
    (h & 0x00ff_ffff) as f32 * (1.0 / 16_777_215.0)
}

fn noise(p: [f32; 2], cell: f32, seed: u32) -> f32 {
    let q = [p[0] / cell, p[1] / cell];
    let c = [q[0].floor() as u32, q[1].floor() as u32];
    let mask = (WORLD_PERIOD as f32 / cell) as u32 - 1;
    let f = [q[0] - q[0].floor(), q[1] - q[1].floor()];
    let w = f.map(|x| x * x * x * (x * (x * 6.0 - 15.0) + 10.0));
    mix(
        mix(
            hash(c[0] & mask, c[1] & mask, seed),
            hash((c[0] + 1) & mask, c[1] & mask, seed),
            w[0],
        ),
        mix(
            hash(c[0] & mask, (c[1] + 1) & mask, seed),
            hash((c[0] + 1) & mask, (c[1] + 1) & mask, seed),
            w[0],
        ),
        w[1],
    )
}

fn valley_center(z: f32) -> f32 {
    900.0 * (z * (std::f32::consts::TAU / 32_768.0)).sin()
        + 380.0 * (z * (std::f32::consts::TAU / 8_192.0)).sin()
}

/// Rock/soil elevation. Lake beds remain below the water level.
pub fn height_at(x: f64, z: f64) -> f32 {
    let p = [
        x.rem_euclid(WORLD_PERIOD) as f32,
        z.rem_euclid(WORLD_PERIOD) as f32,
    ];
    let cross_valley = (p[0] - valley_center(p[1]) + 8_192.0).rem_euclid(16_384.0) - 8_192.0;
    let ax = cross_valley.abs();
    // Glacial trough: flat floor, steep sides.
    let shoulder = smooth(420.0, 3_800.0, ax).powf(1.25);
    let ridge = 1.0 - (2.0 * noise(p, 2_048.0, 31) - 1.0).abs();
    let ridge = ridge.powf(0.75);
    let crag = 1.0 - (2.0 * noise(p, 512.0, 67) - 1.0).abs();
    // Massif envelope: slow height variation decides which ranges become high
    // Alps and which stay rolling foothills, so peaks stop sharing one height.
    // The remap gives flat-topped high zones where full-height peaks form.
    let massif = (0.30 + 0.70 * smooth(0.20, 0.75, noise(p, 16_384.0, 311)))
        * (0.80 + 0.20 * noise(p, 8_192.0, 317));
    // Foothill belt: rolling pre-alpine hills between floor and high rock.
    let foothill_belt = smooth(500.0, 1_500.0, ax) * (1.0 - shoulder);
    let foothill = noise(p, 1_024.0, 331) * 2.0 - 1.0;
    let elevation = 235.0
        + noise(p, 1_024.0, 101) * 90.0
        + foothill_belt * (180.0 + 220.0 * foothill * massif).max(0.0)
        + shoulder * massif * (950.0 + 1_550.0 * ridge * ridge + 280.0 * crag * crag * crag)
        + noise(p, 128.0, 223) * 18.0 * (0.2 + 0.8 * shoulder);
    let lake_z = (p[1] - 4_800.0 + 8_192.0).rem_euclid(16_384.0) - 8_192.0;
    let lake_d = (cross_valley / 850.0).powi(2) + (lake_z / 1_400.0).powi(2);
    let basin = 1.0 - smooth(0.62, 1.30, lake_d);
    mix(elevation, 110.0 + 14.0 * noise(p, 256.0, 193), basin)
}

/// Visible surface, including the flat alpine lakes.
pub fn surface_height_at(x: f64, z: f64) -> f32 {
    height_at(x, z).max(WATER_LEVEL)
}

/// The same fixed diagonal and clamped corner heights used by the indexed mesh.
/// Collision must cover interpolation above the analytic surface in concavities.
fn mesh_height_at(x: f64, z: f64) -> f32 {
    let step = TERRAIN_CELL_METRES as f64;
    let x0 = (x / step).floor() * step;
    let z0 = (z / step).floor() * step;
    let u = ((x - x0) / step) as f32;
    let v = ((z - z0) / step) as f32;
    let samples = terrain_samples_static();
    let n = TERRAIN_GRID_CELLS as i64;
    let sample = |cx: i64, cz: i64| {
        samples[(cz.rem_euclid(n) * n + cx.rem_euclid(n)) as usize][0].max(WATER_LEVEL)
    };
    let ix = x0.div_euclid(step) as i64;
    let iz = z0.div_euclid(step) as i64;
    let a = sample(ix, iz);
    let b = sample(ix + 1, iz);
    let c = sample(ix, iz + 1);
    if u + v <= 1.0 {
        a * (1.0 - u - v) + b * u + c * v
    } else {
        let d = sample(ix + 1, iz + 1);
        d * (u + v - 1.0) + b * (1.0 - v) + c * (1.0 - u)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Structure {
    pub x: f32,
    pub z: f32,
    pub half_x: f32,
    pub half_z: f32,
    pub wall_height: f32,
    pub roof_height: f32,
}

/// One roof add-on shared by the landmark raster decode and the CPU RT mesh:
/// a box defined by local offsets, half extents, base height and height. `None`
/// in the per-structure table means the add-on collapses to degenerate
/// triangles in both decoders.
#[derive(Clone, Copy, Debug)]
struct Addon {
    dx: f32,
    dz: f32,
    half_x: f32,
    half_z: f32,
    y_base: f32,
    height: f32,
}

// Roof tips: kind 1 is a spire octahedron standing on the roof ridge; kind 2
// is a windmill sail cross. Values like the geometry tables feed both the
// raster vertex shader (`terrain.inc`/`ground.vert`) and the CPU triangle
// generator for the RT acceleration structure, so they must stay mirrored.
#[derive(Clone, Copy, Debug)]
struct Tip {
    kind: u8,
    /// Horizontal offset from the structure centre (spires) or hub offset.
    dx: f32,
    /// Base or pivot height above the foundation.
    y_base: f32,
    /// Spire: octahedron half width and half height. Sails: blade length and
    /// a small visual rake used by the decoder.
    half_w: f32,
    half_h: f32,
}

/// A keep, four towers, four curtain walls, thirteen gabled village houses,
/// a chapel, barn, mill, well, tavern, and granary, a six-stone meadow circle,
/// a hillside watchtower, a ruined tower, a windmill, a mountain shrine, and
/// a lakeside stilt hut. Local coordinates are mirrored by the procedural
/// landmark vertex shader and `engine/shaders/terrain.inc`.
pub fn structure(index: u32) -> Structure {
    assert!(index < LANDMARK_STRUCTURES);
    let (x, z, half_x, half_z, wall_height, roof_height) = match index {
        0 => (0.0, 0.0, 42.0, 32.0, 100.0, 34.0),
        1..=4 => {
            let corner = index - 1;
            (
                if corner & 1 == 0 { -64.0 } else { 64.0 },
                if corner & 2 == 0 { -52.0 } else { 52.0 },
                15.0,
                15.0,
                135.0,
                40.0,
            )
        }
        5..=6 => (
            if index == 5 { -64.0 } else { 64.0 },
            0.0,
            7.0,
            52.0,
            40.0,
            5.0,
        ),
        7..=8 => (
            0.0,
            if index == 7 { -52.0 } else { 52.0 },
            64.0,
            7.0,
            40.0,
            5.0,
        ),
        9..=21 => {
            let house = index - 9;
            (
                -260.0 + (house % 5) as f32 * 64.0,
                -220.0 - (house / 5) as f32 * 68.0,
                16.0,
                13.0,
                18.0 + (house % 3) as f32 * 5.0,
                15.0,
            )
        }
        // Chapel: narrow, tall nave with a steep roof, west of the house grid.
        22 => (-388.0, -254.0, 11.0, 17.0, 26.0, 24.0),
        // Barn: wide and low with a long shallow roof.
        23 => (-388.0, -364.0, 24.0, 15.0, 12.0, 13.0),
        // Watermill tower at the village's stream side.
        24 => (96.0, -430.0, 11.0, 11.0, 32.0, 9.0),
        // Well house on the village square between keep and houses.
        25 => (34.0, -166.0, 4.5, 4.5, 3.5, 4.0),
        // Tavern: the largest house, facing the keep across the square.
        26 => (-196.0, -158.0, 22.0, 16.0, 20.0, 17.0),
        // Granary: small, tall, and airy at the village's west edge.
        27 => (-452.0, -160.0, 13.0, 10.0, 9.0, 10.0),
        // Six standing stones on the western meadow, heights cycling 8/11/14.
        28..=33 => {
            let k = index - 28;
            let angle = k as f32 * std::f32::consts::TAU / 6.0;
            (
                -560.0 + 44.0 * angle.cos(),
                140.0 + 44.0 * angle.sin(),
                2.4,
                2.4,
                8.0 + (k % 3) as f32 * 3.0,
                1.2,
            )
        }
        // Watchtower on the eastern valley shoulder, overlooking the village.
        34 => (640.0, -220.0, 12.0, 12.0, 58.0, 14.0),
        // Ruined tower: tall walls, all but collapsed roof, on the ridge foot.
        35 => (720.0, 300.0, 9.0, 9.0, 44.0, 2.5),
        // Windmill on the western shoulder above the stone circle.
        36 => (-760.0, -120.0, 10.0, 10.0, 34.0, 11.0),
        // Mountain shrine on the south-western shoulder.
        37 => (-700.0, 320.0, 5.0, 5.0, 9.0, 9.0),
        38 => (560.0, 1_350.0, 12.0, 9.0, 7.0, 8.0),
        // High Alpine Cloister: mountain monastery on eastern promontory.
        39 => (740.0, -680.0, 14.0, 22.0, 28.0, 18.0),
        // Alpine Almhütte: high pasture herder chalet with low stone roof.
        40 => (-480.0, 540.0, 11.0, 8.5, 8.5, 6.0),
        // Valley Arched Stone Bridge across the stream.
        41 => (180.0, -340.0, 5.0, 28.0, 14.0, 1.5),
        // Mountain Signal Fire / Beacon Tower on western ridge.
        42 => (-740.0, -460.0, 7.5, 7.5, 46.0, 5.0),
        // Meadow Hay Barn (Heustadel).
        43 => (360.0, -580.0, 12.0, 9.0, 9.0, 9.0),
        // Wayside Shrine (Bildstock) at path fork.
        44 => (-110.0, -310.0, 1.8, 1.8, 5.0, 3.0),
        // Castle Barbican / Gatehouse with flanking defense turrets.
        45 => (0.0, -76.0, 18.0, 11.0, 36.0, 8.0),
        // High Pass Watch-Post.
        46 => (710.0, 780.0, 11.0, 11.0, 18.0, 12.0),
        // Lakeside Boat House & Timber Pier.
        47 => (480.0, 1_200.0, 15.0, 10.0, 7.5, 7.0),
        // High Summit Cross & Beacon on highest horn.
        48 => (680.0, -880.0, 4.0, 4.0, 8.0, 2.0),
        // Cliffside Hermitage on western precipice.
        49 => (-620.0, 240.0, 9.0, 11.0, 16.0, 7.0),
        // Avalanche Shelter Gallery on eastern pass.
        50 => (420.0, -120.0, 7.0, 28.0, 14.0, 4.0),
        // Alpine Sawmill & Log Flume on mountain stream.
        _ => (220.0, -480.0, 11.0, 14.0, 9.0, 8.0),
    };
    Structure {
        x,
        z,
        half_x,
        half_z,
        wall_height,
        roof_height,
    }
}

/// First roof-level detail box per structure. Houses and the tavern get ridge
/// chimneys, the chapel a west belfry, the barn a hay-loft cupola, the mill a
/// flue stack and (as detail B) a waterside wheel, the keep and towers corbelled
/// galleries or a ridge beacon block, the windmill its rotating cap, and the
/// ruins jagged broken teeth.
fn detail_a(index: u32) -> Option<Addon> {
    let group = |house: u32| {
        let side = if house % 2 == 0 { 1.0 } else { -1.0 };
        Addon {
            dx: side * 13.0,
            dz: 0.0,
            half_x: 1.1,
            half_z: 1.5,
            y_base: 0.0,
            height: 0.0,
        }
    };
    let wall = structure(index);
    let apex = wall.wall_height + wall.roof_height;
    let addon = match index {
        0 => Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: 7.0,
            half_z: 7.0,
            y_base: apex - 6.0,
            height: 10.0,
        },
        1..=4 => Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: 17.0,
            half_z: 17.0,
            y_base: wall.wall_height - 11.0,
            height: 7.0,
        },
        9..=21 => {
            let mut a = group(index - 9);
            a.y_base = apex - 4.0;
            a.height = 6.0;
            a
        }
        22 => Addon {
            dx: 0.0,
            dz: -8.0,
            half_x: 4.4,
            half_z: 4.4,
            y_base: apex - 12.0,
            height: 16.0,
        },
        23 => Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: 4.2,
            half_z: 6.0,
            y_base: apex - 10.0,
            height: 12.0,
        },
        24 => Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: 2.2,
            half_z: 2.2,
            y_base: wall.wall_height - 2.0,
            height: 10.0,
        },
        26 => {
            let mut a = group(1);
            a.dx = -18.0;
            a.y_base = apex - 4.0;
            a.height = 6.0;
            a
        }
        27 => Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: 3.4,
            half_z: 4.0,
            y_base: 12.0,
            height: 6.0,
        },
        34 => Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: 13.5,
            half_z: 13.5,
            y_base: 50.0,
            height: 5.0,
        },
        35 => Addon {
            dx: 3.0,
            dz: -3.0,
            half_x: 2.6,
            half_z: 2.6,
            y_base: 36.0,
            height: 8.0,
        },
        36 => Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: 5.6,
            half_z: 5.6,
            y_base: 38.0,
            height: 8.0,
        },
        37 => Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: 3.2,
            half_z: 3.2,
            y_base: 11.0,
            height: 4.0,
        },
        38 => Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: 1.3,
            half_z: 1.3,
            y_base: apex - 2.0,
            height: 3.0,
        },
        39 => Addon {
            dx: 0.0,
            dz: -10.0,
            half_x: 4.0,
            half_z: 4.0,
            y_base: apex - 6.0,
            height: 18.0,
        },
        40 => Addon {
            dx: 7.0,
            dz: 0.0,
            half_x: 1.4,
            half_z: 1.6,
            y_base: apex - 2.0,
            height: 5.0,
        },
        41 => Addon {
            dx: 0.0,
            dz: -26.0,
            half_x: 6.5,
            half_z: 4.0,
            y_base: 0.0,
            height: 15.0,
        },
        42 => Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: 3.2,
            half_z: 3.2,
            y_base: apex - 1.0,
            height: 6.0,
        },
        43 => Addon {
            dx: 0.0,
            dz: 8.5,
            half_x: 2.5,
            half_z: 0.8,
            y_base: 8.0,
            height: 4.0,
        },
        44 => Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: 1.5,
            half_z: 1.5,
            y_base: 2.0,
            height: 2.2,
        },
        45 => Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: 5.0,
            half_z: 12.0,
            y_base: 0.0,
            height: 14.0,
        },
        46 => Addon {
            dx: 6.0,
            dz: 6.0,
            half_x: 4.0,
            half_z: 4.0,
            y_base: 16.0,
            height: 14.0,
        },
        47 => Addon {
            dx: 0.0,
            dz: 9.0,
            half_x: 6.0,
            half_z: 2.0,
            y_base: 0.0,
            height: 6.0,
        },
        48 => Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: 4.5,
            half_z: 1.0,
            y_base: 12.0,
            height: 1.6,
        },
        49 => Addon {
            dx: 6.0,
            dz: 0.0,
            half_x: 2.5,
            half_z: 8.0,
            y_base: 8.0,
            height: 6.0,
        },
        50 => Addon {
            dx: 5.5,
            dz: 0.0,
            half_x: 2.0,
            half_z: 30.0,
            y_base: 0.0,
            height: 18.0,
        },
        51 => Addon {
            dx: 0.0,
            dz: -16.0,
            half_x: 2.2,
            half_z: 8.0,
            y_base: 6.5,
            height: 3.0,
        },
        _ => return None,
    };
    Some(addon)
}

/// Second roof-level detail box. A village shed on a third of the houses, the
/// tavern's second chimney, the watermill's paddle wheel on the stream side,
/// and a second broken tooth on the ruined tower.
fn detail_b(index: u32) -> Option<Addon> {
    let wall = structure(index);
    let apex = wall.wall_height + wall.roof_height;
    let addon = match index {
        9..=21 => {
            let house = index - 9;
            if house % 3 != 0 {
                return None;
            }
            Addon {
                dx: 1.6,
                dz: wall.half_z + 0.6,
                half_x: 3.2,
                half_z: 1.7,
                y_base: 0.0,
                height: 7.5,
            }
        }
        26 => Addon {
            dx: 18.0,
            dz: 0.0,
            half_x: 1.2,
            half_z: 1.6,
            y_base: apex - 4.0,
            height: 6.0,
        },
        24 => Addon {
            dx: 0.0,
            dz: wall.half_z + 0.6,
            half_x: 5.6,
            half_z: 0.6,
            y_base: 1.0,
            height: 7.0,
        },
        35 => Addon {
            dx: -4.0,
            dz: 4.0,
            half_x: 2.2,
            half_z: 2.2,
            y_base: 40.0,
            height: 6.0,
        },
        39 => Addon {
            dx: -16.0,
            dz: 0.0,
            half_x: 3.5,
            half_z: 18.0,
            y_base: 0.0,
            height: 9.0,
        },
        40 => Addon {
            dx: -12.0,
            dz: 0.0,
            half_x: 2.5,
            half_z: 7.0,
            y_base: 0.0,
            height: 6.5,
        },
        41 => Addon {
            dx: 0.0,
            dz: 26.0,
            half_x: 6.5,
            half_z: 4.0,
            y_base: 0.0,
            height: 15.0,
        },
        42 => Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: 10.5,
            half_z: 10.5,
            y_base: 0.0,
            height: 12.0,
        },
        43 => Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: 12.5,
            half_z: 9.5,
            y_base: 0.0,
            height: 1.5,
        },
        44 => Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: 2.8,
            half_z: 2.8,
            y_base: 0.0,
            height: 1.0,
        },
        45 => Addon {
            dx: -16.0,
            dz: -2.0,
            half_x: 5.0,
            half_z: 5.0,
            y_base: 0.0,
            height: 44.0,
        },
        46 => Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: 16.0,
            half_z: 16.0,
            y_base: 0.0,
            height: 3.2,
        },
        47 => Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: 16.0,
            half_z: 11.0,
            y_base: -3.0,
            height: 3.5,
        },
        48 => Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: 1.0,
            half_z: 4.5,
            y_base: 12.0,
            height: 1.6,
        },
        49 => Addon {
            dx: -4.0,
            dz: -6.0,
            half_x: 2.5,
            half_z: 2.5,
            y_base: 16.0,
            height: 12.0,
        },
        50 => Addon {
            dx: -5.5,
            dz: 0.0,
            half_x: 1.8,
            half_z: 30.0,
            y_base: 0.0,
            height: 12.0,
        },
        51 => Addon {
            dx: 11.0,
            dz: 0.0,
            half_x: 3.0,
            half_z: 5.0,
            y_base: 0.0,
            height: 6.5,
        },
        _ => return None,
    };
    Some(addon)
}

/// Third detail tier: a mid-wall volume that changes each building's
/// silhouette. Houses and the tavern wear a jettied upper storey, the keep a
/// recessed upper keep, the towers a slimmer top storey, the curtain walls a
/// central gate-house block, the chapel a west porch, the barn a long lean-to,
/// the mill and windmill gallery balcony rings, the well a seat plinth, the
/// granary a staddle band, the ruin a tumbled rubble pile, the shrine a
/// stepped plinth, and the stilt hut a lakeside deck.
fn detail_c(index: u32) -> Option<Addon> {
    let wall = structure(index);
    let addon = match index {
        0 => Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: 30.0,
            half_z: 22.0,
            y_base: 62.0,
            height: 36.0,
        },
        1..=4 => Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: 12.0,
            half_z: 12.0,
            y_base: 88.0,
            height: 40.0,
        },
        5..=8 => Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: 7.6,
            half_z: 7.6,
            y_base: 0.0,
            height: wall.wall_height + 2.0,
        },
        9..=21 => {
            let house = index - 9;
            let wh = 18.0 + (house % 3) as f32 * 5.0;
            Addon {
                dx: 0.0,
                dz: 0.0,
                half_x: 16.6,
                half_z: 13.6,
                y_base: wh * 0.52,
                height: wh * 0.44,
            }
        }
        22 => Addon {
            dx: -(wall.half_x + 2.0),
            dz: 0.0,
            half_x: 2.6,
            half_z: 7.0,
            y_base: 0.0,
            height: 13.0,
        },
        23 => Addon {
            dx: 0.0,
            dz: -(wall.half_z + 0.7),
            half_x: wall.half_x + 0.7,
            half_z: 3.2,
            y_base: 0.0,
            height: 5.0,
        },
        24 => Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: 13.0,
            half_z: 13.0,
            y_base: 18.0,
            height: 1.6,
        },
        25 => Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: 5.0,
            half_z: 5.0,
            y_base: 0.0,
            height: 0.9,
        },
        26 => Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: 20.0,
            half_z: 15.0,
            y_base: 9.0,
            height: 11.0,
        },
        27 => Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: 13.5,
            half_z: 10.5,
            y_base: 1.2,
            height: 1.5,
        },
        35 => Addon {
            dx: 6.0,
            dz: 4.0,
            half_x: 11.0,
            half_z: 9.0,
            y_base: 0.0,
            height: 3.5,
        },
        36 => Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: 11.2,
            half_z: 11.2,
            y_base: 26.0,
            height: 1.6,
        },
        37 => Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: 6.2,
            half_z: 6.2,
            y_base: 0.0,
            height: 2.0,
        },
        38 => Addon {
            dx: 0.0,
            dz: -(wall.half_z + 4.0),
            half_x: 9.0,
            half_z: 4.0,
            y_base: 0.0,
            height: 1.1,
        },
        39 => Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: 18.0,
            half_z: 26.0,
            y_base: 0.0,
            height: 4.5,
        },
        40 => Addon {
            dx: 0.0,
            dz: -9.2,
            half_x: 9.5,
            half_z: 1.8,
            y_base: 3.2,
            height: 4.5,
        },
        41 => Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: 7.0,
            half_z: 6.0,
            y_base: 0.0,
            height: 10.0,
        },
        42 => Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: 9.2,
            half_z: 9.2,
            y_base: 38.0,
            height: 2.0,
        },
        43 => Addon {
            dx: 0.0,
            dz: -10.0,
            half_x: 4.0,
            half_z: 3.0,
            y_base: 0.0,
            height: 4.0,
        },
        44 => Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: 3.8,
            half_z: 3.8,
            y_base: 0.0,
            height: 0.4,
        },
        45 => Addon {
            dx: 16.0,
            dz: -2.0,
            half_x: 5.0,
            half_z: 5.0,
            y_base: 0.0,
            height: 44.0,
        },
        46 => Addon {
            dx: -12.0,
            dz: 0.0,
            half_x: 3.5,
            half_z: 9.0,
            y_base: 0.0,
            height: 8.0,
        },
        47 => Addon {
            dx: -16.0,
            dz: 4.0,
            half_x: 3.0,
            half_z: 14.0,
            y_base: 0.0,
            height: 1.0,
        },
        48 => Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: 6.5,
            half_z: 6.5,
            y_base: 0.0,
            height: 3.5,
        },
        49 => Addon {
            dx: -7.0,
            dz: 0.0,
            half_x: 3.0,
            half_z: 10.0,
            y_base: 0.0,
            height: 12.0,
        },
        50 => Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: 4.5,
            half_z: 32.0,
            y_base: 0.0,
            height: 2.2,
        },
        51 => Addon {
            dx: -10.0,
            dz: 4.0,
            half_x: 3.5,
            half_z: 6.5,
            y_base: 0.0,
            height: 5.5,
        },
        _ => return None,
    };
    Some(addon)
}

/// Roofline band at the wall-to-roof seat: a projecting machicolation corbel
/// band on fortifications, a broad timber-softit eave fascia on timber
/// buildings, and a plain stone corbel band everywhere else. Sits just proud
/// of the wall and pokes a hand above the wall top, where the roof eave
/// overhang hides the seam. Omitted where the silhouette is intentionally
/// plain (standing stones) or collapsed (the ruin).
fn detail_d(index: u32) -> Option<Addon> {
    if (28..=33).contains(&index) || index == 35 || index == 41 || index == 44 {
        return None;
    }
    let wall = structure(index);
    let addon = if index <= 8
        || index == 34
        || index == 42
        || index == 45
        || index == 46
        || index == 48
        || index == 50
    {
        // Fortification machicolation band, one man-height tall.
        Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: wall.half_x + 0.6,
            half_z: wall.half_z + 0.6,
            y_base: wall.wall_height - 2.8,
            height: 3.0,
        }
    } else {
        // Eave fascia, two metres tall under the roof seat.
        Addon {
            dx: 0.0,
            dz: 0.0,
            half_x: wall.half_x + 0.5,
            half_z: wall.half_z + 0.5,
            y_base: wall.wall_height - 1.8,
            height: 2.0,
        }
    };
    Some(addon)
}

/// Roof-top element: spire octahedron (kind 1) or windmill sail cross (kind 2).
fn tip(index: u32) -> Option<Tip> {
    let wall = structure(index);
    let apex = wall.wall_height + wall.roof_height;
    let t = match index {
        0 => Tip {
            kind: 1,
            dx: 0.0,
            y_base: apex + 4.0,
            half_w: 2.6,
            half_h: 14.0,
        },
        1..=4 => Tip {
            kind: 1,
            dx: 0.0,
            y_base: apex,
            half_w: 8.5,
            half_h: 26.0,
        },
        22 => Tip {
            kind: 1,
            dx: 0.0,
            y_base: apex - 12.0 + 16.0,
            half_w: 3.2,
            half_h: 12.0,
        },
        23 => Tip {
            kind: 1,
            dx: 0.0,
            y_base: apex + 2.0,
            half_w: 2.4,
            half_h: 8.0,
        },
        24 => Tip {
            kind: 1,
            dx: 0.0,
            y_base: apex,
            half_w: 5.6,
            half_h: 7.0,
        },
        25 => Tip {
            kind: 1,
            dx: 0.0,
            y_base: apex,
            half_w: 2.2,
            half_h: 3.6,
        },
        27 => Tip {
            kind: 1,
            dx: 0.0,
            y_base: apex,
            half_w: 1.8,
            half_h: 4.0,
        },
        34 => Tip {
            kind: 1,
            dx: 0.0,
            y_base: apex,
            half_w: 8.2,
            half_h: 13.0,
        },
        36 => Tip {
            kind: 2,
            dx: 0.0,
            y_base: 30.0,
            half_w: 12.0,
            half_h: 0.35,
        },
        37 => Tip {
            kind: 1,
            dx: 0.0,
            y_base: apex,
            half_w: 2.6,
            half_h: 8.0,
        },
        39 => Tip {
            kind: 1,
            dx: 0.0,
            y_base: apex + 6.0,
            half_w: 2.8,
            half_h: 16.0,
        },
        42 => Tip {
            kind: 1,
            dx: 0.0,
            y_base: apex,
            half_w: 2.2,
            half_h: 4.5,
        },
        43 => Tip {
            kind: 1,
            dx: 0.0,
            y_base: apex,
            half_w: 1.2,
            half_h: 3.0,
        },
        44 => Tip {
            kind: 1,
            dx: 0.0,
            y_base: apex,
            half_w: 0.8,
            half_h: 2.5,
        },
        45 => Tip {
            kind: 1,
            dx: -16.0,
            y_base: 44.0,
            half_w: 3.5,
            half_h: 10.0,
        },
        46 => Tip {
            kind: 1,
            dx: 6.0,
            y_base: 30.0,
            half_w: 2.5,
            half_h: 7.0,
        },
        48 => Tip {
            kind: 1,
            dx: 0.0,
            y_base: 8.0,
            half_w: 1.0,
            half_h: 10.0,
        },
        49 => Tip {
            kind: 1,
            dx: -4.0,
            y_base: 28.0,
            half_w: 2.0,
            half_h: 4.0,
        },
        51 => Tip {
            kind: 1,
            dx: 4.0,
            y_base: apex - 2.0,
            half_w: 1.2,
            half_h: 2.5,
        },
        _ => return None,
    };
    Some(t)
}

fn shader_float(value: f32) -> String {
    if value == 0.0 {
        "0.0".to_owned()
    } else {
        format!("{value:.9}")
    }
}

fn emit_shader_vec4_array(output: &mut String, name: &str, rows: &[[f32; 4]]) {
    let count = rows.len();
    writeln!(output, "const vec4 {name}[{count}] = vec4[{count}](").unwrap();
    for (index, row) in rows.iter().enumerate() {
        let comma = if index + 1 == count { "" } else { "," };
        writeln!(
            output,
            "    vec4({}, {}, {}, {}){comma}",
            shader_float(row[0]),
            shader_float(row[1]),
            shader_float(row[2]),
            shader_float(row[3]),
        )
        .unwrap();
    }
    output.push_str(");\n");
}

fn emit_shader_vec2_array(output: &mut String, name: &str, rows: &[[f32; 2]]) {
    let count = rows.len();
    writeln!(output, "const vec2 {name}[{count}] = vec2[{count}](").unwrap();
    for (index, row) in rows.iter().enumerate() {
        let comma = if index + 1 == count { "" } else { "," };
        writeln!(
            output,
            "    vec2({}, {}){comma}",
            shader_float(row[0]),
            shader_float(row[1]),
        )
        .unwrap();
    }
    output.push_str(");\n");
}

fn emit_shader_float_array(output: &mut String, name: &str, values: &[f32]) {
    let count = values.len();
    writeln!(output, "const float {name}[{count}] = float[{count}](").unwrap();
    for (index, value) in values.iter().enumerate() {
        let comma = if index + 1 == count { "" } else { "," };
        writeln!(output, "    {}{comma}", shader_float(*value)).unwrap();
    }
    output.push_str(");\n");
}

fn emit_shader_uint_array(output: &mut String, name: &str, values: &[u32]) {
    let count = values.len();
    writeln!(output, "const uint {name}[{count}] = uint[{count}](").unwrap();
    for (index, value) in values.iter().enumerate() {
        let comma = if index + 1 == count { "" } else { "," };
        writeln!(output, "    {value}u{comma}").unwrap();
    }
    output.push_str(");\n");
}

/// Emit the packed landmark data consumed by the raster vertex shader.
///
/// This is deliberately generated from the same Rust functions used for
/// collision and the CPU ray-tracing mesh. The shader receives flat arrays and
/// performs indexed loads instead of repeating the structure/add-on match
/// ladders for every landmark vertex.
pub fn landmark_shader_inc() -> String {
    let count = LANDMARK_STRUCTURES as usize;
    let mut output = String::from(
        "// Generated by world::landmark_shader_inc; do not edit.\n"
    );
    writeln!(output, "const float TERRAIN_PERIOD = {WORLD_PERIOD:.9};").unwrap();
    writeln!(output, "const float TERRAIN_WATER = {WATER_LEVEL:.9};").unwrap();
    writeln!(output, "const uint TERRAIN_CELLS = {}u;", TERRAIN_GRID_CELLS).unwrap();
    writeln!(output, "const float TERRAIN_CELL_METRES = {TERRAIN_CELL_METRES:.9};").unwrap();
    writeln!(output, "const uint TERRAIN_VERTICES = (TERRAIN_CELLS + 1u) * (TERRAIN_CELLS + 1u);")
        .unwrap();
    writeln!(output, "const uint TERRAIN_STRUCTURES = {LANDMARK_STRUCTURES}u;").unwrap();
    writeln!(output, "const uint STRUCTURE_WALL_VERTICES = {STRUCTURE_WALL_VERTICES}u;").unwrap();
    writeln!(output, "const uint STRUCTURE_ROOF_VERTICES = {STRUCTURE_ROOF_VERTICES}u;").unwrap();
    writeln!(output, "const uint STRUCTURE_DETAIL_VERTICES = {STRUCTURE_DETAIL_VERTICES}u;").unwrap();
    writeln!(output, "const uint STRUCTURE_BAND_VERTICES = {STRUCTURE_BAND_VERTICES}u;").unwrap();
    writeln!(output, "const uint STRUCTURE_TIP_VERTICES = {STRUCTURE_TIP_VERTICES}u;").unwrap();
    output.push_str(
        "const uint STRUCTURE_VERTICES = STRUCTURE_WALL_VERTICES + STRUCTURE_ROOF_VERTICES\n"
    );
    output.push_str(
        "    + STRUCTURE_DETAIL_VERTICES + STRUCTURE_DETAIL_VERTICES\n"
    );
    output.push_str(
        "    + STRUCTURE_DETAIL_VERTICES + STRUCTURE_BAND_VERTICES\n"
    );
    output.push_str("    + STRUCTURE_TIP_VERTICES;\n");
    output.push_str("const uint LANDMARK_VERTEX_COUNT = 9u * TERRAIN_STRUCTURES * STRUCTURE_VERTICES;\n");
    writeln!(output, "const float SCATTER_PITCH = {SCATTER_PITCH:.9};").unwrap();
    writeln!(output, "const float VEGETATION_CELL_METRES = {:.9};", vegetation::VEGETATION_CELL_METRES).unwrap();
    writeln!(output, "const uint VEGETATION_CELL_AXIS = {}u;", vegetation::VEGETATION_CELLS_PER_AXIS).unwrap();
    writeln!(output, "const float VEGETATION_RANGE_METRES = {:.9};", vegetation::VEGETATION_RANGE_METRES).unwrap();
    writeln!(output, "const float VEGETATION_MAX_RADIUS = {:.9};", vegetation::VEGETATION_MAX_RADIUS).unwrap();
    writeln!(output, "const float VEGETATION_MAX_HEIGHT = {:.9};", vegetation::VEGETATION_MAX_HEIGHT).unwrap();
    writeln!(output, "const uint VEGETATION_CULL_RADIUS_CELLS = {}u;", vegetation::VEGETATION_CULL_RADIUS_CELLS).unwrap();
    writeln!(output, "const uint VEGETATION_CULL_DIAMETER = {}u;", vegetation::VEGETATION_CULL_DIAMETER).unwrap();
    writeln!(output, "const uint VEGETATION_CULL_CELL_COUNT = {}u;", vegetation::VEGETATION_CULL_CELL_COUNT).unwrap();
    writeln!(output, "const uint VEGETATION_INSTANCE_VERTICES = {}u;", vegetation::VEGETATION_INSTANCE_VERTICES).unwrap();
    writeln!(output, "const uint VEGETATION_VISIBLE_CAPACITY = {}u;", vegetation::VEGETATION_VISIBLE_CAPACITY).unwrap();

    let mut structure_shapes = Vec::with_capacity(count);
    let mut structure_heights = Vec::with_capacity(count);
    for index in 0..LANDMARK_STRUCTURES {
        let structure = structure(index);
        structure_shapes.push([
            structure.x,
            structure.z,
            structure.half_x,
            structure.half_z,
        ]);
        structure_heights.push([structure.wall_height, structure.roof_height]);
    }
    emit_shader_vec4_array(&mut output, "WORLD_STRUCTURES", &structure_shapes);
    emit_shader_vec2_array(&mut output, "WORLD_STRUCTURE_HEIGHTS", &structure_heights);

    let addon_functions: [fn(u32) -> Option<Addon>; 4] = [detail_a, detail_b, detail_c, detail_d];
    for (slot, function) in addon_functions.into_iter().enumerate() {
        let mut shapes = Vec::with_capacity(count);
        let mut heights = Vec::with_capacity(count);
        let mut enabled = Vec::with_capacity(count);
        for index in 0..LANDMARK_STRUCTURES {
            if let Some(addon) = function(index) {
                shapes.push([addon.dx, addon.dz, addon.half_x, addon.half_z]);
                heights.push([addon.y_base, addon.height]);
                enabled.push(1);
            } else {
                shapes.push([0.0; 4]);
                heights.push([0.0; 2]);
                enabled.push(0);
            }
        }
        let name = ["A", "B", "C", "D"][slot];
        emit_shader_vec4_array(&mut output, &format!("WORLD_DETAIL_{name}"), &shapes);
        emit_shader_vec2_array(&mut output, &format!("WORLD_DETAIL_{name}_HEIGHTS"), &heights);
        emit_shader_uint_array(&mut output, &format!("WORLD_DETAIL_{name}_ENABLED"), &enabled);
    }

    let mut tip_values = Vec::with_capacity(count);
    let mut tip_heights = Vec::with_capacity(count);
    for index in 0..LANDMARK_STRUCTURES {
        if let Some(tip) = tip(index) {
            tip_values.push([
                tip.kind as f32,
                tip.dx,
                tip.y_base,
                tip.half_w,
            ]);
            tip_heights.push(tip.half_h);
        } else {
            tip_values.push([0.0; 4]);
            tip_heights.push(0.0);
        }
    }
    emit_shader_vec4_array(&mut output, "WORLD_TIPS", &tip_values);
    emit_shader_float_array(&mut output, "WORLD_TIP_HALF_HEIGHT", &tip_heights);

    output.push_str(
        "\nvec4 terrainStructure(uint index, out float wallHeight, out float roofHeight) {\n"
    );
    output.push_str("    vec2 heights = WORLD_STRUCTURE_HEIGHTS[index];\n");
    output.push_str("    wallHeight = heights.x;\n    roofHeight = heights.y;\n");
    output.push_str("    return WORLD_STRUCTURES[index];\n}\n\n");
    for name in ["A", "B", "C", "D"] {
        writeln!(
            output,
            "vec4 terrainDetail{name}(uint index, float wallHeight, float roofHeight, out float yBase, out float height) {{"
        )
        .unwrap();
        writeln!(output, "    vec2 values = WORLD_DETAIL_{name}_HEIGHTS[index];").unwrap();
        writeln!(output, "    yBase = values.x; height = values.y;").unwrap();
        writeln!(output, "    if (WORLD_DETAIL_{name}_ENABLED[index] == 0u) {{ yBase = 0.0; height = 0.0; }}").unwrap();
        writeln!(output, "    return WORLD_DETAIL_{name}[index];\n}}\n").unwrap();
    }
    output.push_str(
        "vec4 terrainTip(uint index, float wallHeight, float roofHeight, out float halfH) {\n"
    );
    output.push_str("    halfH = WORLD_TIP_HALF_HEIGHT[index];\n    return WORLD_TIPS[index];\n}\n");
    output
}

/// Highest rendered point above the foundation for a structure, including
/// roof add-ons and tips. The flight clearance floor uses this so spires and
/// sails keep the forgiving arcade ceiling honest.
pub fn structure_top(index: u32) -> f32 {
    let s = structure(index);
    let mut top = s.wall_height + s.roof_height;
    for addon in [
        detail_a(index),
        detail_b(index),
        detail_c(index),
        detail_d(index),
    ]
    .into_iter()
    .flatten()
    {
        top = top.max(addon.y_base + addon.height);
    }
    if let Some(t) = tip(index) {
        top = match t.kind {
            1 => top.max(t.y_base + 2.0 * t.half_h),
            _ => top.max(t.y_base + t.half_w),
        };
    }
    top
}

const BOX_CORNERS: [[f32; 3]; 8] = [
    [-1.0, 0.0, -1.0],
    [1.0, 0.0, -1.0],
    [-1.0, 1.0, -1.0],
    [1.0, 1.0, -1.0],
    [-1.0, 0.0, 1.0],
    [1.0, 0.0, 1.0],
    [-1.0, 1.0, 1.0],
    [1.0, 1.0, 1.0],
];
const BOX_TRIS: [usize; 36] = [
    0, 2, 1, 1, 2, 3, 5, 7, 4, 4, 7, 6, 4, 6, 0, 0, 6, 2, 1, 3, 5, 5, 3, 7, 2, 6, 3, 3, 6, 7, 4, 0,
    5, 5, 0, 1,
];
const ROOF_CORNERS: [[f32; 3]; 6] = [
    [-1.0, 0.0, -1.0],
    [1.0, 0.0, -1.0],
    [-1.0, 0.0, 1.0],
    [1.0, 0.0, 1.0],
    [0.0, 1.0, -1.0],
    [0.0, 1.0, 1.0],
];
const ROOF_TRIS: [usize; 18] = [0, 2, 5, 0, 5, 4, 1, 4, 5, 1, 5, 3, 0, 4, 1, 2, 3, 5];
// Standing octahedron: axis-aligned diamond used for spires, crowns, and the
// windmill cap. Mirrors `OCTA`/`OCTA_TRI` in ground.vert.
const OCTA_CORNERS: [[f32; 3]; 6] = [
    [1.0, 0.0, 0.0],
    [-1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0],
    [0.0, 0.0, -1.0],
    [0.0, 1.0, 0.0],
    [0.0, -1.0, 0.0],
];
const OCTA_TRIS: [usize; 24] = [
    0, 2, 4, 2, 1, 4, 1, 3, 4, 3, 0, 4, 2, 0, 5, 1, 2, 5, 3, 1, 5, 0, 3, 5,
];
const TAU: f32 = std::f32::consts::TAU;

/// Triangle vertices (x, y, z floats) for all landmark structures across one
/// world period, in the same corner order as the ground.vert decode so the
/// acceleration structure matches the rasterized silhouettes exactly:
/// wall box, roof prism, detail boxes A/B/C, the roofline band, then the tip.
pub fn landmark_structure_triangles() -> Vec<f32> {
    let mut verts =
        Vec::with_capacity(16 * LANDMARK_STRUCTURES as usize * STRUCTURE_VERTICES as usize * 3);
    for tz in 0..4 {
        let base_z = tz as f32 * SETTLEMENT_SPACING + 3_450.0;
        let valley = valley_center(base_z.rem_euclid(WORLD_PERIOD as f32));
        for tx in 0..4 {
            let base_x = tx as f32 * SETTLEMENT_SPACING + valley + 1_180.0;
            for index in 0..LANDMARK_STRUCTURES {
                let s = structure(index);
                let center_x = base_x + s.x;
                let center_z = base_z + s.z;
                let foundation = height_at(center_x as f64, center_z as f64).max(WATER_LEVEL);

                fn push(
                    verts: &mut Vec<f32>,
                    corners: &[[f32; 3]],
                    tris: &[usize],
                    scale: [f32; 3],
                    base: [f32; 3],
                ) {
                    for &t in tris {
                        let c = corners[t];
                        verts.extend_from_slice(&[
                            base[0] + c[0] * scale[0],
                            base[1] + c[1] * scale[1],
                            base[2] + c[2] * scale[2],
                        ]);
                    }
                }
                // Wall box.
                push(
                    &mut verts,
                    &BOX_CORNERS,
                    &BOX_TRIS,
                    [s.half_x, s.wall_height, s.half_z],
                    [center_x, foundation, center_z],
                );
                // Roof prism with a one-metre eave overhang.
                push(
                    &mut verts,
                    &ROOF_CORNERS,
                    &ROOF_TRIS,
                    [s.half_x + 1.0, s.roof_height, s.half_z + 1.0],
                    [center_x, foundation + s.wall_height, center_z],
                );
                // Four detail boxes and the roof tip; absent add-ons collapse
                // to a degenerate point at the foundation centre.
                for addon in [
                    detail_a(index),
                    detail_b(index),
                    detail_c(index),
                    detail_d(index),
                ] {
                    match addon {
                        Some(a) => push(
                            &mut verts,
                            &BOX_CORNERS,
                            &BOX_TRIS,
                            [a.half_x, a.height, a.half_z],
                            [center_x + a.dx, foundation + a.y_base, center_z + a.dz],
                        ),
                        None => {
                            for _ in 0..STRUCTURE_DETAIL_VERTICES {
                                verts.extend_from_slice(&[center_x, foundation, center_z]);
                            }
                        }
                    }
                }
                match tip(index) {
                    Some(t) if t.kind == 1 => {
                        let base = [center_x + t.dx, foundation + t.y_base + t.half_h, center_z];
                        push(
                            &mut verts,
                            &OCTA_CORNERS,
                            &OCTA_TRIS,
                            [t.half_w, t.half_h, t.half_w],
                            base,
                        );
                    }
                    Some(t) if t.kind == 2 => {
                        // The windmill's four-sail cross on the south face.
                        let sail_z = center_z - (s.half_z + 4.0);
                        for tri in 0..8 {
                            let ang0 = tri as f32 * TAU / 8.0;
                            for v in 0..3 {
                                let dir = if v == 0 {
                                    [0.0, 0.0]
                                } else {
                                    let ang = ang0 + if v == 1 { 0.0 } else { TAU / 8.0 };
                                    let len = if tri & 1 == 0 {
                                        t.half_w
                                    } else {
                                        t.half_w * 0.78
                                    };
                                    [ang.cos() * len, ang.sin() * len]
                                };
                                verts.extend_from_slice(&[
                                    center_x + t.dx + dir[0],
                                    foundation + t.y_base + dir[1],
                                    sail_z,
                                ]);
                            }
                        }
                    }
                    _ => {
                        for _ in 0..STRUCTURE_TIP_VERTICES {
                            verts.extend_from_slice(&[center_x, foundation, center_z]);
                        }
                    }
                }
            }
        }
    }
    verts
}

/// Conservative collision surface for the nearest settlement and water/terrain.
/// Buildings use their highest roof point across the footprint. This is an
/// intentionally forgiving arcade floor, not detailed rigid-body collision.
pub fn collision_height_at(x: f64, z: f64) -> f32 {
    let mut height = surface_height_at(x, z).max(mesh_height_at(x, z));
    // Reduce first, just as the vertex shader does; inspecting adjacent cells
    // catches the village extents even at settlement boundaries.
    let p = [
        x.rem_euclid(WORLD_PERIOD) as f32,
        z.rem_euclid(WORLD_PERIOD) as f32,
    ];
    let tile = [
        (p[0] / SETTLEMENT_SPACING).floor(),
        (p[1] / SETTLEMENT_SPACING).floor(),
    ];
    for dz in -1..=1 {
        let base_z = (tile[1] + dz as f32) * SETTLEMENT_SPACING + 3_450.0;
        let valley = valley_center(base_z.rem_euclid(WORLD_PERIOD as f32));
        for dx in -1..=1 {
            let base_x = (tile[0] + dx as f32) * SETTLEMENT_SPACING + valley + 1_180.0;
            // Outbuildings reach 770 m west (windmill) and the stilt hut sits
            // 1350 m south, so the coarse extent check must cover the full
            // landmark spread before per-structure footprints are tested.
            if (p[0] - base_x).abs() > 800.0 || (p[1] - base_z).abs() > 1420.0 {
                continue;
            }
            for index in 0..LANDMARK_STRUCTURES {
                let s = structure(index);
                let sx = base_x + s.x;
                let sz = base_z + s.z;
                // Match the shader's one-metre eaves and pad for the glider's
                // approximately 11 m half-span while approaching a roof edge.
                if (p[0] - sx).abs() <= s.half_x + 12.0 && (p[1] - sz).abs() <= s.half_z + 12.0 {
                    height =
                        height.max(surface_height_at(sx as f64, sz as f64) + structure_top(index));
                }
            }
        }
    }
    height.max(scatter_collision_at(x, z))
}

#[derive(Clone, Copy)]
struct CachedSettlement {
    x: f32,
    z: f32,
    half_x: f32,
    half_z: f32,
    top: f32,
}

/// Reusable collision neighborhood for the high-frequency flight/camera loop.
///
/// The terrain sample table is immutable and global; this object caches the
/// spatially sparse, view-independent objects that sit on top of it. A 17x17
/// scatter-slot envelope is rebuilt only when the tracked 24 m cell moves far
/// enough that the existing envelope can no longer answer a query. The common
/// plane/HUD/camera queries therefore reuse the same `ScatterItem` and
/// settlement records instead of re-running the 5x5 slot and 3x3 settlement
/// searches every present.
pub struct WorldNeighborhood {
    center_cell: Option<(i64, i64)>,
    scatter: Vec<ScatterItem>,
    settlements: Vec<CachedSettlement>,
}

const NEIGHBORHOOD_SLOT_RADIUS: i64 = 8;
const NEIGHBORHOOD_QUERY_RADIUS: i64 = 6;

impl Default for WorldNeighborhood {
    fn default() -> Self {
        Self::new()
    }
}

impl WorldNeighborhood {
    pub fn new() -> Self {
        Self {
            center_cell: None,
            scatter: Vec::with_capacity(
                ((NEIGHBORHOOD_SLOT_RADIUS * 2 + 1).pow(2) as usize) * 2,
            ),
            settlements: Vec::with_capacity(9 * LANDMARK_STRUCTURES as usize),
        }
    }

    fn query_cell(x: f64, z: f64) -> (i64, i64) {
        (
            (x / SCATTER_PITCH as f64).floor() as i64,
            (z / SCATTER_PITCH as f64).floor() as i64,
        )
    }

    fn contains(&self, cell: (i64, i64)) -> bool {
        self.center_cell.is_some_and(|center| {
            (cell.0 - center.0).abs() <= NEIGHBORHOOD_QUERY_RADIUS
                && (cell.1 - center.1).abs() <= NEIGHBORHOOD_QUERY_RADIUS
        })
    }

    /// Recenter the retained records around a world-space point. Calling this
    /// explicitly before a batch of queries is optional; `collision_height_at`
    /// recenters on demand for teleports or unusually distant camera offsets.
    pub fn recenter(&mut self, x: f64, z: f64) {
        let center = Self::query_cell(x, z);
        self.center_cell = Some(center);
        self.scatter.clear();
        for dz in -NEIGHBORHOOD_SLOT_RADIUS..=NEIGHBORHOOD_SLOT_RADIUS {
            for dx in -NEIGHBORHOOD_SLOT_RADIUS..=NEIGHBORHOOD_SLOT_RADIUS {
                let cx = center.0 + dx;
                let cz = center.1 + dz;
                let Some(primary) = scatter_slot(cx, cz) else {
                    continue;
                };
                self.scatter.push(primary);
                if let Some(companion) = scatter_companion(cx, cz, &primary) {
                    self.scatter.push(companion);
                }
            }
        }

        let p = [
            x.rem_euclid(WORLD_PERIOD) as f32,
            z.rem_euclid(WORLD_PERIOD) as f32,
        ];
        let tile_x = (p[0] / SETTLEMENT_SPACING).floor() as i64;
        let tile_z = (p[1] / SETTLEMENT_SPACING).floor() as i64;
        self.settlements.clear();
        for tz in tile_z - 1..=tile_z + 1 {
            let base_z = tz as f32 * SETTLEMENT_SPACING + 3_450.0;
            let valley = valley_center(base_z.rem_euclid(WORLD_PERIOD as f32));
            for tx in tile_x - 1..=tile_x + 1 {
                let base_x = tx as f32 * SETTLEMENT_SPACING + valley + 1_180.0;
                for index in 0..LANDMARK_STRUCTURES {
                    let s = structure(index);
                    self.settlements.push(CachedSettlement {
                        x: base_x + s.x,
                        z: base_z + s.z,
                        half_x: s.half_x + 12.0,
                        half_z: s.half_z + 12.0,
                        top: surface_height_at(
                            (base_x + s.x) as f64,
                            (base_z + s.z) as f64,
                        ) + structure_top(index),
                    });
                }
            }
        }
    }

    /// Query the conservative collision floor using the retained neighborhood.
    pub fn collision_height_at(&mut self, x: f64, z: f64) -> f32 {
        let cell = Self::query_cell(x, z);
        if !self.contains(cell) {
            self.recenter(x, z);
        }
        let mut height = surface_height_at(x, z).max(mesh_height_at(x, z));
        let p = [
            x.rem_euclid(WORLD_PERIOD) as f32,
            z.rem_euclid(WORLD_PERIOD) as f32,
        ];
        for settlement in &self.settlements {
            if (p[0] - settlement.x).abs() <= settlement.half_x
                && (p[1] - settlement.z).abs() <= settlement.half_z
            {
                height = height.max(settlement.top);
            }
        }
        for item in &self.scatter {
            let dx = x - item.x as f64;
            let dz = z - item.z as f64;
            let reach = item.radius + 12.0;
            if dx * dx + dz * dz <= (reach * reach) as f64 {
                height = height.max(item.top);
            }
        }
        height
    }
}

/// One placed scatter item (mirrors a surviving ground.vert slot): position,
/// crown radius, and the collision top of the taller crown.
#[derive(Clone, Copy)]
pub struct ScatterItem {
    pub x: f32,
    pub z: f32,
    pub radius: f32,
    pub top: f32,
    pub ground: f32,
    /// Boulder slots never carry a companion tree.
    pub boulder: bool,
    /// Species index used by the foliage material; boulders retain the
    /// deterministic species hash result for stable material variation.
    pub species: u8,
    /// Primary size hash, also used as the foliage material's stable random
    /// channel. Companions inherit it from their primary.
    pub size_r: f32,
    /// Rendered scale in metres relative to the species envelope.
    pub size: f32,
}

/// Terrain-cache values for one lattice cell, identical to the `terrain_tex`
/// texels the vertex stage fetches: height, x/z surface slopes, moisture.
fn lattice_sample(cx: i64, cz: i64) -> [f32; 4] {
    // Direct-mapped memo: the same cells are re-queried every frame by the
    // flight and camera clearance paths. The permanently retained terrain
    // sample table makes a miss a single cache-line-sized indexed load rather
    // than a 5x5 blur of analytic heights. Bounded, no allocation after warmup.
    thread_local! {
        static CACHE: std::cell::RefCell<Vec<(u64, [f32; 4])>> =
            std::cell::RefCell::new(vec![(u64::MAX, [0.0; 4]); 1 << 16]);
    }
    // Texture addressing wraps before lookup. Canonical keys also keep
    // cell (-1, -1) from aliasing the empty-entry sentinel u64::MAX.
    let n = TERRAIN_GRID_CELLS as i64;
    let cx = cx.rem_euclid(n);
    let cz = cz.rem_euclid(n);
    let key = ((cx as u64) << 32) | cz as u64;
    let index = (key.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 48) as usize;
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let slot = index & (cache.len() - 1);
        if cache[slot].0 == key {
            return cache[slot].1;
        }
        let sample = terrain_samples_static()[(cz * n + cx) as usize];
        cache[slot] = (key, sample);
        sample
    })
}

/// Placement for one 24 m scatter slot, mirroring the ground.vert scatter
/// branch hash-for-hash: 401/407 jitter, 419 presence, 431 species, 433 size.
/// The biome gates read the same lattice texels, so CPU clearance and GPU
/// draw agree on which slots hold a tree or boulder.
pub fn scatter_slot(cx: i64, cz: i64) -> Option<ScatterItem> {
    // The shader hashes 16-bit slot coordinates, including for negative X/Z.
    let hx = (cx as u32) & 65535;
    let hz = (cz as u32) & 65535;
    let presence = hash(hx, hz, 419);
    let species = hash(hx, hz, 431);
    let size_r = hash(hx, hz, 433);
    let offset = vegetation::candidate_offset(cx, cz);
    let x = cx as f32 * SCATTER_PITCH + offset[0];
    let z = cz as f32 * SCATTER_PITCH + offset[1];

    // Scatter slots are 24 m apart; terrain texels are 64 m apart. Read the
    // cell containing the jittered world position, just like ground.vert.
    // Using the slot index sampled distant mountains and created midair
    // collision floors over the valley and lakes.
    let cell_x = (x / TERRAIN_CELL_METRES).floor() as i64;
    let cell_z = (z / TERRAIN_CELL_METRES).floor() as i64;
    let sample = lattice_sample(cell_x, cell_z);
    let cell_normal_y = 1.0 / (sample[1] * sample[1] + sample[2] * sample[2] + 1.0).sqrt();
    let slope = 1.0 - cell_normal_y;
    let moist = sample[3];
    let alt = sample[0];
    let forest = smooth(0.34, 0.52, moist);
    let presence_p = vegetation::tree_presence_probability_at(alt, slope, moist, x, z);
    if presence >= presence_p {
        return None;
    }
    let is_boulder = vegetation::is_boulder(slope, species);
    let size = 1.35 + 0.9 * size_r;
    let species_index = vegetation::tree_species(alt, species, forest);
    let (radius, top) = if is_boulder {
        vegetation::boulder_envelope(size, size_r)
    } else {
        vegetation::tree_crown_envelope(species_index, size, size_r)
    };
    // Village clearing, mirroring the vertex stage's 3x3 tile test.
    let p = [
        x.rem_euclid(WORLD_PERIOD as f32),
        z.rem_euclid(WORLD_PERIOD as f32),
    ];
    let tile = [
        (p[0] / SETTLEMENT_SPACING).floor(),
        (p[1] / SETTLEMENT_SPACING).floor(),
    ];
    for dz in -1..=1 {
        let base_z = (tile[1] + dz as f32) * SETTLEMENT_SPACING + 3_450.0;
        let valley = valley_center(base_z.rem_euclid(WORLD_PERIOD as f32));
        for dx in -1..=1 {
            let base_x = (tile[0] + dx as f32) * SETTLEMENT_SPACING + valley + 1_180.0;
            if (p[0] - base_x).abs() < 820.0 && (p[1] - base_z).abs() < 1450.0 {
                return None;
            }
        }
    }
    // Trees sit on the rendered triangle, not on its lower-left corner.
    let ground = mesh_height_at(x as f64, z as f64).max(WATER_LEVEL);
    Some(ScatterItem {
        x,
        z,
        radius,
        top: ground + top,
        ground,
        boulder: is_boulder,
        species: species_index,
        size_r,
        size,
    })
}

/// The companion tree for a slot: 55% of tree slots carry a smaller
/// same-species tree on a ring 13-22 m from the primary trunk, mirroring
/// ground.vert's secondary-corner decode (ring angle seed 409, distance seed
/// 411, presence seed 421, size seed 427). Boulder slots never carry one.
pub fn scatter_companion(cx: i64, cz: i64, primary: &ScatterItem) -> Option<ScatterItem> {
    if primary.boulder {
        return None;
    }
    let hx = (cx as u32) & 65535;
    let hz = (cz as u32) & 65535;
    if hash(hx, hz, 421) >= 0.55 {
        return None;
    }
    let angle = hash(hx, hz, 409) * TAU;
    let dist = (0.55 + 0.35 * hash(hx, hz, 411)) * SCATTER_PITCH;
    let size_ratio = 0.55 + 0.35 * hash(hx, hz, 427);
    let x = primary.x + angle.cos() * dist;
    let z = primary.z + angle.sin() * dist;
    let ground = mesh_height_at(x as f64, z as f64).max(WATER_LEVEL);
    Some(ScatterItem {
        x,
        z,
        radius: primary.radius * size_ratio,
        top: ground + (primary.top - primary.ground) * size_ratio,
        ground,
        boulder: false,
        species: primary.species,
        size_r: primary.size_r,
        size: primary.size * size_ratio,
    })
}

/// Collision floor from scatter items near a query point: the neighbouring
/// slots fully cover any crown (radius at most ~16 m plus the 12 m
/// glider pad and up to 17 m slot jitter) around a 24 m pitch lattice.
pub fn scatter_collision_at(x: f64, z: f64) -> f32 {
    let base_x = (x / SCATTER_PITCH as f64).floor() as i64;
    let base_z = (z / SCATTER_PITCH as f64).floor() as i64;
    let mut height = 0.0f32;
    for dz in -2..=2 {
        for dx in -2..=2 {
            let Some(primary) = scatter_slot(base_x + dx, base_z + dz) else {
                continue;
            };
            for item in [
                Some(primary),
                scatter_companion(base_x + dx, base_z + dz, &primary),
            ] {
                let Some(item) = item else {
                    continue;
                };
                let ddx = x - item.x as f64;
                let ddz = z - item.z as f64;
                let reach = item.radius + 12.0;
                if ddx * ddx + ddz * ddz <= (reach * reach) as f64 {
                    height = height.max(item.top);
                }
            }
        }
    }
    height
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_is_in_open_valley_with_clearance() {
        assert!(SPAWN_ALTITUDE > collision_height_at(SPAWN_X as f64, SPAWN_Z as f64) + 500.0);
    }

    #[test]
    fn lake_is_flat_and_has_a_submerged_bed() {
        let z = 4_800.0;
        let x = valley_center(z);
        assert!(height_at(x as f64, z as f64) < WATER_LEVEL - 50.0);
        assert_eq!(surface_height_at(x as f64, z as f64), WATER_LEVEL);
        assert_eq!(surface_height_at(x as f64 + 100.0, z as f64), WATER_LEVEL);
    }

    #[test]
    fn alpine_relief_is_bounded_and_has_high_peaks() {
        let mut maximum = 0.0f32;
        for z in (0..65_536).step_by(512) {
            for x in (0..65_536).step_by(512) {
                let h = height_at(x as f64, z as f64);
                assert!(
                    h.is_finite() && (100.0..=MAX_TERRAIN_HEIGHT).contains(&h),
                    "{x}, {z}: {h}"
                );
                maximum = maximum.max(h);
            }
        }
        assert!(maximum > 2_800.0, "maximum {maximum}");
    }

    #[test]
    fn wrapping_and_rebasing_preserve_elevation() {
        for (x, z) in [
            (17.25, 1050.5),
            (-900.125, -12_002.75),
            (32_767.5, 12_001.25),
        ] {
            let expected = height_at(x, z);
            assert_eq!(
                expected,
                height_at(x + WORLD_PERIOD * 1_000_000.0, z - WORLD_PERIOD * 900_000.0)
            );
            let anchor = [4096.0, -8192.0];
            assert_eq!(
                expected,
                height_at(anchor[0] + (x - anchor[0]), anchor[1] + (z - anchor[1]))
            );
        }
    }

    #[test]
    fn world_period_has_no_height_seam() {
        for z in (0..65_536).step_by(127) {
            assert!((height_at(-0.01, z as f64) - height_at(0.01, z as f64)).abs() < 0.1);
            assert!((height_at(z as f64, -0.01) - height_at(z as f64, 0.01)).abs() < 0.1);
        }
    }

    #[test]
    fn castle_collision_includes_roof() {
        let z = 3_450.0;
        let x = valley_center(z) + 1_180.0;
        let floor = surface_height_at(x as f64, z as f64);
        // Keep wall 100 + roof 34, plus the ridge beacon block and the
        // flagpole spire: structure_top reaches 166 above the foundation.
        assert!((collision_height_at(x as f64, z as f64) - floor - 166.0).abs() < 0.01);
    }

    #[test]
    fn keep_roof_overhang_receives_collision_clearance() {
        let z = 3450.0;
        let x = valley_center(z) + 1180.0;
        let ridge = surface_height_at(x as f64, z as f64) + 166.0;
        assert!((collision_height_at(x as f64, z as f64 + 32.5) - ridge).abs() < 0.01);
    }

    #[test]
    fn every_landmark_fits_the_collision_extent_guard() {
        for index in 0..LANDMARK_STRUCTURES {
            let s = structure(index);
            assert!(
                s.x.abs() + s.half_x + 12.0 <= 800.0,
                "structure {index} x {} exceeds the east/west guard",
                s.x
            );
            assert!(
                s.z.abs() + s.half_z + 12.0 <= 1420.0,
                "structure {index} z {} exceeds the north/south guard",
                s.z
            );
        }
    }

    #[test]
    fn standalone_landmarks_raise_collision_floors() {
        // The watchtower on the eastern shoulder: 58 m walls, 14 m roof, a
        // gallery band, and a cone spire top out at 98 m above the floor.
        let tower_z = 3_450.0 - 220.0;
        let tower_x = valley_center(3_450.0) + 1_180.0 + 640.0;
        let floor = surface_height_at(tower_x as f64, tower_z as f64);
        assert!(
            (collision_height_at(tower_x as f64, tower_z as f64) - floor - 98.0).abs() < 0.01,
            "watchtower collision {} vs floor {floor}",
            collision_height_at(tower_x as f64, tower_z as f64)
        );
        // The first standing stone (8 m stone plus 1.2 m cap) on the meadow.
        let stone_z = 3_450.0 + 140.0;
        let stone_x = valley_center(3_450.0) + 1_180.0 - 516.0;
        let stone_floor = surface_height_at(stone_x as f64, stone_z as f64);
        assert!(
            (collision_height_at(stone_x as f64, stone_z as f64) - stone_floor - 9.2).abs() < 0.01,
            "stone collision {} vs floor {stone_floor}",
            collision_height_at(stone_x as f64, stone_z as f64)
        );
    }

    #[test]
    fn scatter_is_deterministic_and_biome_gated() {
        // Determinism: the same slot always places (or refuses) identically;
        // the vertex shader and this collision mirror must never drift.
        let first = scatter_slot(3_401, -127).map(|s| (s.x, s.z, s.top));
        for _ in 0..3 {
            assert_eq!(scatter_slot(3_401, -127).map(|s| (s.x, s.z, s.top)), first);
        }
        // Everything that survives the gates stands above water.
        let mut placed = 0u32;
        let mut cleared = 0u32;
        for cz in -40..40 {
            for cx in -40..40 {
                match scatter_slot(cx, cz) {
                    Some(item) => {
                        placed += 1;
                        assert!(
                            item.ground >= WATER_LEVEL,
                            "item below water at {} {}",
                            item.x,
                            item.z
                        );
                        assert!(
                            item.top > item.ground,
                            "item without height at {} {}",
                            item.x,
                            item.z
                        );
                        assert!(item.radius > 0.0);
                    }
                    None => cleared += 1,
                }
            }
        }
        assert!(
            placed > 10,
            "biome gates rejected nearly everything: {placed}"
        );
        assert!(
            cleared > 10,
            "biome gates accepted nearly everything: {cleared}"
        );
    }

    #[test]
    fn scatter_keeps_the_settlement_clearing() {
        // The exclusion invariant is world-space: every surviving slot's item
        // sits at least 820 m east/west or 1450 m north/south of the
        // settlement base. Sweep every cell whose item could possibly land
        // inside the strip; a cell straddling the boundary may place an item
        // outside it, which still satisfies the invariant.
        let base_z = 3_450.0f64;
        let base_x = valley_center(3_450.0) as f64 + 1_180.0;
        let pitch = SCATTER_PITCH as f64;
        let mut cz = (base_z - 1_700.0) / pitch;
        while cz * pitch < base_z + 1_700.0 {
            let mut cx = (base_x - 1_200.0) / pitch;
            while cx * pitch < base_x + 1_200.0 {
                if let Some(item) = scatter_slot(cx.floor() as i64, cz.floor() as i64) {
                    let clear = (item.x as f64 - base_x).abs() >= 820.0
                        || (item.z as f64 - base_z).abs() >= 1450.0;
                    assert!(
                        clear,
                        "scatter at ({},{}) inside the village clearing",
                        item.x, item.z
                    );
                }
                cx += 1.0;
            }
            cz += 1.0;
        }
    }

    #[test]
    fn trees_raise_collision_floors_within_their_crowns() {
        // Find a placed tree near the spawn valley and verify the collision
        // surface reaches its crown inside the footprint, and that just past
        // the crown edge the collision equals the max of the ground and any
        // other crown covering that point (groves overlap their 48 m cells).
        let mut found = None;
        'search: for cz in -200..200 {
            for cx in -200..200 {
                if let Some(item) = scatter_slot(cx, cz) {
                    if (cx as f32 * SCATTER_PITCH - SPAWN_X as f32).abs() < 1_200.0
                        && (cz as f32 * SCATTER_PITCH - SPAWN_Z as f32).abs() < 1_200.0
                    {
                        found = Some(item);
                        break 'search;
                    }
                }
            }
        }
        let Some(item) = found else {
            panic!("no scatter within 1.2 km of the spawn valley; gates are wrong");
        };
        let at_center = collision_height_at(item.x as f64, item.z as f64);
        assert!(
            at_center >= item.top - 0.5,
            "collision {at_center} below the crown top {} at the trunk",
            item.top
        );

        let mut neighbours = Vec::new();
        let bx = (item.x / SCATTER_PITCH).floor() as i64;
        let bz = (item.z / SCATTER_PITCH).floor() as i64;
        for dz in -2..=2 {
            for dx in -2..=2 {
                if let Some(other) = scatter_slot(bx + dx, bz + dz) {
                    neighbours.push(other);
                }
            }
        }
        let sample_x = (item.x + item.radius + 1.0) as f64;
        let sample_z = item.z as f64;
        let mut expected =
            surface_height_at(sample_x, sample_z).max(mesh_height_at(sample_x, sample_z));
        for other in &neighbours {
            let reach = (other.radius + 12.0) as f64;
            if ((sample_x - other.x as f64).powi(2) + (sample_z - other.z as f64).powi(2))
                <= reach * reach
            {
                expected = expected.max(other.top);
            }
        }
        let clear = collision_height_at(sample_x, sample_z);
        assert!(
            (clear - expected).abs() < 0.05,
            "collision {clear} vs expected {expected} past the crown edge"
        );
    }

    #[test]
    fn procedural_draw_budget_is_fixed() {
        assert_eq!(TERRAIN_VERTEX_COUNT, 1_050_625);
        assert_eq!(
            DRAW_INDEX_COUNT,
            TERRAIN_INDEX_COUNT + LANDMARK_VERTEX_COUNT
        );
        assert_eq!(
            TERRAIN_GRID_CELLS as f64 * TERRAIN_CELL_METRES as f64,
            WORLD_PERIOD
        );
        // Recentring keeps a minimum 32.7 km radius around the camera.
        assert!((TERRAIN_GRID_CELLS / 2 - 1) as f32 * TERRAIN_CELL_METRES > 30_000.0);
        let indices = terrain_indices();
        assert_eq!(indices.len(), DRAW_INDEX_COUNT as usize);
        assert!(
            indices
                .iter()
                .all(|&i| i < TERRAIN_VERTEX_COUNT + GROUND_FEATURE_INDEX_COUNT)
        );
        assert_eq!(&indices[..6], &[0, 1025, 1, 1, 1025, 1026]);
        assert_eq!(indices[TERRAIN_INDEX_COUNT as usize], TERRAIN_VERTEX_COUNT);
        let performance = performance_terrain_indices();
        assert_eq!(
            performance.len(),
            PERFORMANCE_TERRAIN_CHUNK_COUNT as usize * PERFORMANCE_TERRAIN_CHUNK_INDICES as usize
        );
        assert!(performance.iter().all(|&i| i < TERRAIN_VERTEX_COUNT));
        assert_eq!(&performance[..6], &[0, 2050, 2, 2, 2050, 2052]);
    }

    #[test]
    fn landform_moisture_follows_water_and_sheds_ridges() {
        let n = TERRAIN_GRID_CELLS as usize;
        let cell = TERRAIN_CELL_METRES as f64;
        let samples = terrain_samples();
        assert!(samples.iter().all(|s| (0.0..=1.0).contains(&s[3])));
        let spot = |x: f64, z: f64| {
            let ix = (x.rem_euclid(WORLD_PERIOD) / cell) as usize % n;
            let iz = (z.rem_euclid(WORLD_PERIOD) / cell) as usize % n;
            samples[iz * n + ix][3]
        };
        // Valley floor near spawn holds water; a high ridge sheds it.
        let floor = spot(0.0, 1_050.0);
        let ridge = spot(valley_center(1_050.0) as f64 + 6_500.0, 1_050.0);
        assert!(floor > 0.55, "valley floor {floor}");
        assert!(ridge < 0.6, "ridge {ridge}");
        assert!(floor > ridge, "floor {floor} ridge {ridge}");
        // Lakeshore is wet.
        let shore = spot(valley_center(4_800.0) as f64 + 750.0, 4_800.0);
        assert!(shore > 0.6, "lakeshore {shore}");
        // Wet valley floors are a minority; dry high ground is substantial;
        // low ground is wetter on average than high ground.
        let wet = samples.iter().filter(|s| s[3] > 0.7).count();
        let dry = samples.iter().filter(|s| s[3] < 0.3).count();
        assert!(
            (0.02..0.4).contains(&(wet as f32 / samples.len() as f32)),
            "wet {wet}"
        );
        assert!(dry as f32 / samples.len() as f32 > 0.3, "dry {dry}");
        let (mut low_sum, mut low_n, mut high_sum, mut high_n) = (0.0f32, 0u32, 0.0f32, 0u32);
        for s in &samples {
            if s[0] < 400.0 {
                low_sum += s[3];
                low_n += 1;
            } else if s[0] > 1800.0 {
                high_sum += s[3];
                high_n += 1;
            }
        }
        assert!(
            low_sum / low_n as f32 > high_sum / high_n as f32 + 0.15,
            "low {} high {}",
            low_sum / low_n as f32,
            high_sum / high_n as f32
        );
    }

    #[test]
    fn cached_terrain_matches_world_and_wraps_normals() {
        let samples = terrain_samples();
        let n = TERRAIN_GRID_CELLS as usize;
        assert!(samples.iter().flatten().all(|v| v.is_finite()));
        for (x, z) in [(0, 0), (n - 1, n - 1), (17, 75), (400, 600)] {
            let wx = x as f64 * TERRAIN_CELL_METRES as f64;
            let wz = z as f64 * TERRAIN_CELL_METRES as f64;
            let sample = samples[z * n + x];
            assert_eq!(sample[0], height_at(wx, wz));
            assert_eq!(
                sample[1],
                (surface_height_at(wx + 64.0, wz) - surface_height_at(wx - 64.0, wz)) / 128.0
            );
            assert_eq!(
                sample[2],
                (surface_height_at(wx, wz + 64.0) - surface_height_at(wx, wz - 64.0)) / 128.0
            );
        }
    }

    #[test]
    fn collision_covers_triangle_interpolation_and_rebases() {
        for z in (0..65_536).step_by(397) {
            for x in (0..65_536).step_by(431) {
                let x = x as f64 + 17.25;
                let z = z as f64 + 29.5;
                let height = mesh_height_at(x, z);
                assert!(collision_height_at(x, z) >= height);
                assert_eq!(height, mesh_height_at(x - WORLD_PERIOD, z + WORLD_PERIOD));
            }
        }
    }

    #[test]
    fn landmark_triangles_count_and_bounds() {
        let tris = landmark_structure_triangles();
        assert_eq!(
            tris.len(),
            16 * LANDMARK_STRUCTURES as usize * STRUCTURE_VERTICES as usize * 3
        );
        for i in 0..tris.len() / 3 {
            let x = tris[i * 3];
            let y = tris[i * 3 + 1];
            let z = tris[i * 3 + 2];
            assert!(x.is_finite() && y.is_finite() && z.is_finite());
            assert!(y >= WATER_LEVEL);
        }
    }

    #[test]
    fn neighborhood_reuses_the_exact_collision_recipe() {
        let mut neighborhood = WorldNeighborhood::new();
        for (x, z) in [
            (SPAWN_X as f64, SPAWN_Z as f64),
            (valley_center(3_450.0) as f64 + 1_180.0, 3_450.0),
            (12_345.25, -8_901.75),
            (WORLD_PERIOD + 12_345.25, -WORLD_PERIOD - 8_901.75),
        ] {
            let cached = neighborhood.collision_height_at(x, z);
            let reference = collision_height_at(x, z);
            assert!((cached - reference).abs() < 0.001, "{x}, {z}: {cached} vs {reference}");
        }
    }
}
