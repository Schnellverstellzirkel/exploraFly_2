//! Uniform Buffer Object (UBO) layouts, physical celestial constants, and coordinate packing.
//!
//! Defines the host-to-GPU uniform layout shared across `plane.frag`, `sky.frag`,
//! `clouds.frag`, and `ground.frag`.

use glam::Vec3;

/// Total size in bytes of the per-frame uniform buffer object.
///
/// Layout breakdown (1,824 bytes total):
/// - `mat4 viewProj` (64 B, offset 0)
/// - `mat4 invViewProj` (64 B, offset 64)
/// - `mat4 nodes[23]` ($23 \times 64 = 1,472$ B, offset 128)
/// - Tail parameters ($14 \times 16 = 224$ B, offset 1,600):
///   - `vec4 flex` (16 B): x=bend, y=time, z=pressure, w=glow
///   - `vec4 campos` (16 B): xyz=camera pos, w=exit radius
///   - `vec4 sunDir` (16 B): xyz=sun unit dir, w=radius
///   - `vec4 sunColor` (16 B): xyz=irradiance, w=elevation
///   - `vec4 skyZenith` (16 B): xyz=zenith radiance, w=cos_radius
///   - `vec4 skyHorizon` (16 B): xyz=horizon radiance, w=inv_one_minus_cos_radius
///   - `vec4 groundBase` (16 B): xyz=ground albedo, w=relative ground height
///   - `vec4 detail` (16 B): x=flicker, y=lambda, z=spool, w=plume_length
///   - `vec4 originShift` (16 B): packed origin shift for trails
///   - `vec4 cameraParams` (16 B): fov_y, aspect, speed, load
///   - `vec4 cameraParams2` (16 B): shake, exposure, mach, wind strength
///   - `vec4 groundOrigin` (16 B): split X/Z origin
///   - `vec4 hudFlight` (16 B): speed, altitude, heading, climb
///   - `vec4 hudState` (16 B): visibility, boost, flags, clearance
pub const UBO_BYTES: usize = 1824;

/// Total number of articulated kinematic nodes on the aircraft airframe.
pub const NODE_COUNT: usize = 23;

/// World-space height of the flat terrain plane (meters).
pub const GROUND_LEVEL: f32 = 0.0;

/// Grid size (in meters) for split-coordinate floating-origin ground texture alignment.
///
/// The ground fragment shader receives the floating-origin anchor as an integer
/// number of 25 cm cells plus a sub-cell remainder. This maintains sub-millimeter
/// texture precision at extreme distances without requiring 64-bit float emulation.
pub const GROUND_FINE_CELL: f32 = 0.25;

/// Plume radial expansion bounding box multiplier.
pub const PLUME_WARP_BOUND: f32 = 1.35;

/// Plume axial bounding box length multiplier.
pub const PLUME_AXIAL_BOUND: f32 = 1.35;

/// Mean solar angular half-radius at 1 AU (~16 arcminutes, in radians).
pub const SUN_RADIUS: f32 = 0.00465;

/// Solar elevation angle above the horizon (radians).
pub const SUN_ELEVATION: f32 = 0.38;

/// Unit vector pointing toward the sun in world space.
pub const SUN_DIR: Vec3 = Vec3::new(0.48540115, 0.37092048, 0.79170936);

/// Top-of-atmosphere extraterrestrial solar irradiance in radiometric HDR units.
pub const SUN_IRRADIANCE: Vec3 = Vec3::new(3.05, 3.00, 2.90);

/// Reference zenith sky radiance.
pub const SKY_ZENITH: Vec3 = Vec3::new(0.08, 0.22, 0.68);

/// Reference horizon atmospheric haze radiance.
pub const SKY_HORIZON: Vec3 = Vec3::new(0.55, 0.72, 0.90);

/// Ground base diffuse albedo.
pub const GROUND_BASE: Vec3 = Vec3::new(0.04335021, 0.05573598, 0.03715732);

/// Cosine of the solar angular radius.
pub const SUN_COS_RADIUS: f32 = 1.0 - SUN_RADIUS * SUN_RADIUS * 0.5;

/// Reciprocal of $(1 - \cos(\text{radius}))$, used for fast solar disc AA limb testing.
pub const INV_ONE_MINUS_SUN_COS_RADIUS: f32 = 1.0 / (1.0 - SUN_COS_RADIUS);

/// Round an address or offset up to Vulkan's required 256-byte alignment.
#[inline]
pub fn align_256(v: u64) -> u64 {
    (v + 255) & !255
}

/// Pack an absolute X/Z world origin into a split 4-float coordinate for `ground.frag`.
///
/// `[cell_x, cell_z, sub_x, sub_z]`
/// Keeping the fractional remainder separate prevents adding a large world
/// coordinate to a small camera-relative hit from erasing sub-meter detail.
pub fn ground_origin_pack(origin: Vec3) -> [f32; 4] {
    let cell = [
        (origin.x / GROUND_FINE_CELL).floor(),
        (origin.z / GROUND_FINE_CELL).floor(),
    ];
    [
        cell[0],
        cell[1],
        origin.x - cell[0] * GROUND_FINE_CELL,
        origin.z - cell[1] * GROUND_FINE_CELL,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ubo_tail_and_bytes_alignment() {
        assert_eq!(UBO_BYTES, 1824);
        assert_eq!(UBO_BYTES % 16, 0);
        assert_eq!(NODE_COUNT, 23);
        let matrix_floats = 16 + 16 + NODE_COUNT * 16;
        let tail_floats = 56;
        assert_eq!((matrix_floats + tail_floats) * std::mem::size_of::<f32>(), UBO_BYTES);
    }

    #[test]
    fn test_ground_origin_pack_preserves_sub_cell_phase() {
        let origins = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.12, 50.0, -0.18),
            Vec3::new(1234.56, 0.0, -9876.54),
            Vec3::new(-500.0, 0.0, 500.0),
        ];
        for orig in origins {
            let p = ground_origin_pack(orig);
            let reconstructed_x = p[0] * GROUND_FINE_CELL + p[2];
            let reconstructed_z = p[1] * GROUND_FINE_CELL + p[3];
            assert!(
                (reconstructed_x - orig.x).abs() < 1e-4,
                "reconstruction mismatch on X: {reconstructed_x} vs {}",
                orig.x
            );
            assert!(
                (reconstructed_z - orig.z).abs() < 1e-4,
                "reconstruction mismatch on Z: {reconstructed_z} vs {}",
                orig.z
            );
            assert!(
                (0.0..GROUND_FINE_CELL).contains(&p[2]),
                "sub-cell X out of bounds: {}",
                p[2]
            );
            assert!(
                (0.0..GROUND_FINE_CELL).contains(&p[3]),
                "sub-cell Z out of bounds: {}",
                p[3]
            );
        }
    }
}
