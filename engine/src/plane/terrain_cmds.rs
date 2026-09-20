//! Conservative terrain chunk visibility and the immutable near-to-far
//! chunk order used to fill the indirect draw commands.

use ash::vk;
use glam::{Mat4, Vec3};

static TERRAIN_FRONT_TO_BACK: std::sync::LazyLock<Vec<usize>> = std::sync::LazyLock::new(|| {
    let side = world::TERRAIN_CHUNKS_PER_AXIS as i32;
    let mut order: Vec<_> = (0..world::TERRAIN_CHUNK_COUNT as usize).collect();
    order.sort_by_key(|&i| {
        let x = 2 * (i as i32 % side) + 1 - side;
        let z = 2 * (i as i32 / side) + 1 - side;
        x * x + z * z
    });
    order
});

// Conservative clip-space plane tests retain any chunk intersecting the view.
// Heights include every cached terrain vertex, so culling cannot expose holes.
pub(super) fn terrain_draw_commands(
    commands: &mut [vk::DrawIndexedIndirectCommand],
    view_proj: Mat4,
    origin: Vec3,
    eye_rel: Vec3,
) {
    let rows = view_proj.transpose();
    let planes = [rows.w_axis + rows.x_axis, rows.w_axis - rows.x_axis,
        rows.w_axis + rows.y_axis, rows.w_axis - rows.y_axis,
        rows.z_axis, rows.w_axis - rows.z_axis];
    let period = world::WORLD_PERIOD as f32;
    let wrapped = glam::Vec2::new(origin.x.rem_euclid(period), origin.z.rem_euclid(period));
    let cell = world::TERRAIN_CELL_METRES;
    let anchor = ((wrapped + glam::Vec2::new(eye_rel.x, eye_rel.z)) / cell).floor()
        * cell - wrapped - glam::Vec2::splat(world::TERRAIN_GRID_CELLS as f32 * cell * 0.5);
    let width = world::TERRAIN_CHUNK_CELLS as f32 * cell;
    let half_height = (world::MAX_TERRAIN_HEIGHT - world::WATER_LEVEL) * 0.5;
    let extent = Vec3::new(width * 0.5, half_height, width * 0.5) + Vec3::splat(1.0);
    for (&i, command) in TERRAIN_FRONT_TO_BACK.iter().zip(commands.iter_mut()) {
        let x = i as u32 % world::TERRAIN_CHUNKS_PER_AXIS;
        let z = i as u32 / world::TERRAIN_CHUNKS_PER_AXIS;
        let center = Vec3::new(anchor.x + (x as f32 + 0.5) * width,
            world::WATER_LEVEL + half_height - origin.y,
            anchor.y + (z as f32 + 0.5) * width);
        // z and w become nearly equal at the distant clip plane. Allow for
        // float rounding in the shader's matrix multiply before rejecting it.
        let clip_slack = 8.0 * f32::EPSILON
            * (rows.w_axis.truncate().abs().dot(center.abs() + extent) + rows.w_axis.w.abs());
        let visible = planes.iter().all(|p| {
            p.truncate().dot(center) + p.w + p.truncate().abs().dot(extent) >= -clip_slack
        });
        *command = vk::DrawIndexedIndirectCommand {
            index_count: world::TERRAIN_CHUNK_INDICES,
            instance_count: u32::from(visible),
            first_index: i as u32 * world::TERRAIN_CHUNK_INDICES,
            vertex_offset: 0,
            first_instance: 0,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terrain_order_keeps_every_chunk_and_increases_distance() {
        let order = &*TERRAIN_FRONT_TO_BACK;
        let unique: std::collections::HashSet<_> = order.iter().copied().collect();
        assert_eq!(unique.len(), world::TERRAIN_CHUNK_COUNT as usize);
        assert!(order.iter().all(|&i| i < world::TERRAIN_CHUNK_COUNT as usize));
        let side = world::TERRAIN_CHUNKS_PER_AXIS as i32;
        let distances: Vec<_> = order.iter().map(|&i| {
            let x = 2 * (i as i32 % side) + 1 - side;
            let z = 2 * (i as i32 / side) + 1 - side;
            x * x + z * z
        }).collect();
        assert!(distances.windows(2).all(|d| d[0] <= d[1]));
    }

    #[test]
    fn terrain_culling_preserves_visible_chunks_through_rebases_and_turns() {
        let mut commands = vec![vk::DrawIndexedIndirectCommand::default(); world::TERRAIN_CHUNK_COUNT as usize];
        for origin in [Vec3::new(0.0, 1100.0, 1050.0), Vec3::new(-65537.0, 2000.0, 131071.0)] {
            for yaw in [0.0f32, 0.7, 2.5, 4.8] {
                let view = Mat4::look_at_rh(Vec3::ZERO, Vec3::new(yaw.sin(), -0.15, yaw.cos()), Vec3::Y);
                let vp = Mat4::perspective_rh(1.25, 1.6, 0.1, 30000.0) * view;
                terrain_draw_commands(&mut commands, vp, origin, Vec3::ZERO);
                let visible = commands.iter().filter(|c| c.instance_count != 0).count();
                assert!(visible > 0 && visible < commands.len() / 2, "visible={visible}");
                let reference: Vec<_> = commands.iter().map(|c| c.instance_count).collect();
                terrain_draw_commands(&mut commands, vp, origin + Vec3::new(65536.0, 0.0, -65536.0), Vec3::ZERO);
                assert_eq!(reference, commands.iter().map(|c| c.instance_count).collect::<Vec<_>>());
                // Every sampled point inside the Vulkan clip volume must keep its chunk.
                let base_x = (origin.x / 64.0).floor() * 64.0 - origin.x - 32768.0;
                let base_z = (origin.z / 64.0).floor() * 64.0 - origin.z - 32768.0;
                for command in &commands {
                    let i = (command.first_index / world::TERRAIN_CHUNK_INDICES) as usize;
                    for dx in [0.0, 1024.0, 2048.0] {
                        for dz in [0.0, 1024.0, 2048.0] {
                            let p = Vec3::new(base_x + (i % 32) as f32 * 2048.0 + dx,
                                1100.0 - origin.y, base_z + (i / 32) as f32 * 2048.0 + dz);
                            let clip = vp * p.extend(1.0);
                            if clip.w > 0.0 && clip.x.abs() <= clip.w && clip.y.abs() <= clip.w
                                && clip.z >= 0.0 && clip.z <= clip.w {
                                assert_eq!(command.instance_count, 1, "culled visible point {p:?}");
                            }
                        }
                    }
                }
            }
        }
    }
}
