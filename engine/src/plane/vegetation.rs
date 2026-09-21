//! Persistent vegetation upload and bounded CPU cell culling.
//!
//! Placement is generated once by `world::vegetation::build_database()`. The
//! per-present work here is only a small periodic cell-window traversal and a
//! conservative frustum test that rewrites the already-mapped indirect draw
//! commands. Empty command slots are retained so command buffers remain
//! reusable across frames without a GPU-side count dependency.

use ash::vk;
use glam::{Mat4, Vec2, Vec3};

use super::geometry::upload_buffer;

const VEGETATION_RANGE_METRES: f32 = world::vegetation::VEGETATION_RANGE_METRES;
const VEGETATION_LOD_FADE_START_METRES: f32 = 900.0;
const VEGETATION_LOD_FADE_END_METRES: f32 = 3_000.0;
const CANOPY_RANGE_METRES: f32 = 7_800.0;
const MAX_TREE_RADIUS: f32 = world::vegetation::VEGETATION_MAX_RADIUS;
const MAX_TREE_HEIGHT: f32 = world::vegetation::VEGETATION_MAX_HEIGHT;
const MAX_CANOPY_RADIUS: f32 = 72.0;

/// Upload one immutable packed record array into device-local storage.
unsafe fn upload_records(
    device: &ash::Device,
    mem_props: &vk::PhysicalDeviceMemoryProperties,
    queue_family: u32,
    queue: vk::Queue,
    records: &[[u32; 4]],
    label: &str,
) -> (vk::Buffer, vk::DeviceMemory) {
    let record_bytes = std::mem::size_of_val(records);
    let byte_count = record_bytes.max(std::mem::size_of::<[u32; 4]>());
    let (buffer, memory) = upload_buffer(
        device,
        mem_props,
        byte_count as u64,
        vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
    );

    let pool_info = vk::CommandPoolCreateInfo::default()
        .queue_family_index(queue_family)
        .flags(vk::CommandPoolCreateFlags::TRANSIENT);
    let pool = device
        .create_command_pool(&pool_info, None)
        .expect("vegetation upload pool");
    let alloc = vk::CommandBufferAllocateInfo::default()
        .command_pool(pool)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(1);
    let cmd = device
        .allocate_command_buffers(&alloc)
        .expect("vegetation upload command buffer")[0];

    let stage_info = vk::BufferCreateInfo::default()
        .size(byte_count as u64)
        .usage(vk::BufferUsageFlags::TRANSFER_SRC)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let stage = device
        .create_buffer(&stage_info, None)
        .expect("vegetation staging buffer");
    let stage_req = device.get_buffer_memory_requirements(stage);
    let stage_index = crate::find_memory_type(
        mem_props,
        stage_req.memory_type_bits,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
    );
    let stage_alloc = vk::MemoryAllocateInfo::default()
        .allocation_size(stage_req.size)
        .memory_type_index(stage_index);
    let stage_memory = device
        .allocate_memory(&stage_alloc, None)
        .expect("vegetation staging memory");
    device
        .bind_buffer_memory(stage, stage_memory, 0)
        .expect("vegetation staging bind");
    let mapped = device
        .map_memory(stage_memory, 0, stage_req.size, vk::MemoryMapFlags::empty())
        .unwrap_or_else(|error| panic!("{label} staging map: {error:?}"))
        as *mut u8;
    if record_bytes != 0 {
        std::ptr::copy_nonoverlapping(records.as_ptr() as *const u8, mapped, record_bytes);
    }
    device.unmap_memory(stage_memory);

    let begin =
        vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
    device
        .begin_command_buffer(cmd, &begin)
        .unwrap_or_else(|error| panic!("{label} upload begin: {error:?}"));
    let copy = vk::BufferCopy::default().size(record_bytes as u64);
    if record_bytes != 0 {
        device.cmd_copy_buffer(cmd, stage, buffer, &[copy]);
    }
    device
        .end_command_buffer(cmd)
        .unwrap_or_else(|error| panic!("{label} upload end: {error:?}"));
    let fence = device
        .create_fence(&vk::FenceCreateInfo::default(), None)
        .unwrap_or_else(|error| panic!("{label} upload fence: {error:?}"));
    let command_buffers = [cmd];
    let submit = vk::SubmitInfo::default().command_buffers(&command_buffers);
    device
        .queue_submit(queue, &[submit], fence)
        .unwrap_or_else(|error| panic!("{label} upload submit: {error:?}"));
    device
        .wait_for_fences(&[fence], true, u64::MAX)
        .unwrap_or_else(|error| panic!("{label} upload wait: {error:?}"));
    device.destroy_fence(fence, None);
    device.destroy_command_pool(pool, None);
    device.destroy_buffer(stage, None);
    device.free_memory(stage_memory, None);
    (buffer, memory)
}

/// Upload the immutable packed tree instance array into device-local storage.
pub(super) unsafe fn upload_database(
    device: &ash::Device,
    mem_props: &vk::PhysicalDeviceMemoryProperties,
    queue_family: u32,
    queue: vk::Queue,
    database: &world::vegetation::VegetationDatabase,
) -> (vk::Buffer, vk::DeviceMemory) {
    upload_records(
        device,
        mem_props,
        queue_family,
        queue,
        &database.instances,
        "tree",
    )
}

/// Upload the aggregate far-canopy field into a separate storage buffer. It is
/// indexed by canonical cell, so `firstInstance` can remain the cell index
/// even when the visible command list is reordered by distance.
pub(super) unsafe fn upload_canopy_database(
    device: &ash::Device,
    mem_props: &vk::PhysicalDeviceMemoryProperties,
    queue_family: u32,
    queue: vk::Queue,
    database: &world::vegetation::VegetationDatabase,
) -> (vk::Buffer, vk::DeviceMemory) {
    upload_records(
        device,
        mem_props,
        queue_family,
        queue,
        &database.canopies,
        "canopy",
    )
}

/// Upload the compact cell range table used by the GPU culler. Keeping the
/// table in device-local memory makes the hot pass read contiguous range
/// metadata instead of touching host memory or rebuilding a visibility list.
pub(super) unsafe fn upload_cells(
    device: &ash::Device,
    mem_props: &vk::PhysicalDeviceMemoryProperties,
    queue_family: u32,
    queue: vk::Queue,
    database: &world::vegetation::VegetationDatabase,
) -> (vk::Buffer, vk::DeviceMemory) {
    let cell_bytes = database.cells.len() * std::mem::size_of::<world::vegetation::VegetationCell>();
    let byte_count = cell_bytes.max(std::mem::size_of::<world::vegetation::VegetationCell>());
    let (buffer, memory) = upload_buffer(
        device,
        mem_props,
        byte_count as u64,
        vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_DST,
    );
    let pool = device
        .create_command_pool(
            &vk::CommandPoolCreateInfo::default()
                .queue_family_index(queue_family)
                .flags(vk::CommandPoolCreateFlags::TRANSIENT),
            None,
        )
        .expect("vegetation cell pool");
    let cmd = device
        .allocate_command_buffers(
            &vk::CommandBufferAllocateInfo::default()
                .command_pool(pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1),
        )
        .expect("vegetation cell command buffer")[0];
    let stage = device
        .create_buffer(
            &vk::BufferCreateInfo::default()
                .size(byte_count as u64)
                .usage(vk::BufferUsageFlags::TRANSFER_SRC)
                .sharing_mode(vk::SharingMode::EXCLUSIVE),
            None,
        )
        .expect("vegetation cell staging buffer");
    let stage_req = device.get_buffer_memory_requirements(stage);
    let stage_index = crate::find_memory_type(
        mem_props,
        stage_req.memory_type_bits,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
    );
    let stage_memory = device
        .allocate_memory(
            &vk::MemoryAllocateInfo::default()
                .allocation_size(stage_req.size)
                .memory_type_index(stage_index),
            None,
        )
        .expect("vegetation cell staging memory");
    device
        .bind_buffer_memory(stage, stage_memory, 0)
        .expect("vegetation cell staging bind");
    let mapped = device
        .map_memory(stage_memory, 0, stage_req.size, vk::MemoryMapFlags::empty())
        .expect("vegetation cell staging map") as *mut u8;
    if cell_bytes != 0 {
        std::ptr::copy_nonoverlapping(database.cells.as_ptr() as *const u8, mapped, cell_bytes);
    }
    device.unmap_memory(stage_memory);
    device
        .begin_command_buffer(
            cmd,
            &vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
        )
        .expect("vegetation cell upload begin");
    if cell_bytes != 0 {
        device.cmd_copy_buffer(
            cmd,
            stage,
            buffer,
            &[vk::BufferCopy::default().size(cell_bytes as u64)],
        );
    }
    device
        .end_command_buffer(cmd)
        .expect("vegetation cell upload end");
    let fence = device
        .create_fence(&vk::FenceCreateInfo::default(), None)
        .expect("vegetation cell upload fence");
    device
        .queue_submit(queue, &[vk::SubmitInfo::default().command_buffers(&[cmd])], fence)
        .expect("vegetation cell upload submit");
    device
        .wait_for_fences(&[fence], true, u64::MAX)
        .expect("vegetation cell upload wait");
    device.destroy_fence(fence, None);
    device.destroy_command_pool(pool, None);
    device.destroy_buffer(stage, None);
    device.free_memory(stage_memory, None);
    (buffer, memory)
}

/// Rewrite the fixed-capacity non-indexed indirect command array for the
/// visible periodic cell window. Commands are ordered front-to-back by cell
/// centre distance; full crowns are kept near the aircraft and compact
/// crossed-plane crowns cover the mid range. Zero-count tail entries are
/// legal and keep the recorded command buffer independent of camera movement.
pub(super) fn vegetation_draw_commands(
    commands: &mut [vk::DrawIndirectCommand],
    database: &world::vegetation::VegetationDatabase,
    view_proj: Mat4,
    origin: Vec3,
    eye_rel: Vec3,
) {
    commands.fill(vk::DrawIndirectCommand::default());
    if database.instances.is_empty() {
        return;
    }

    let rows = view_proj.transpose();
    let planes = [
        rows.w_axis + rows.x_axis,
        rows.w_axis - rows.x_axis,
        rows.w_axis + rows.y_axis,
        rows.w_axis - rows.y_axis,
        rows.z_axis,
        rows.w_axis - rows.z_axis,
    ];
    let period = world::WORLD_PERIOD as f32;
    let cell_width = world::vegetation::VEGETATION_CELL_METRES;
    let axis = world::vegetation::VEGETATION_CELLS_PER_AXIS as i32;
    let camera_world = origin + eye_rel;
    let wrapped_camera = Vec2::new(
        camera_world.x.rem_euclid(period),
        camera_world.z.rem_euclid(period),
    );
    let camera_cell = Vec2::new(
        (wrapped_camera.x / cell_width).floor(),
        (wrapped_camera.y / cell_width).floor(),
    );
    let radius = (VEGETATION_RANGE_METRES / cell_width).ceil() as i32 + 1;
    let half_height = (world::MAX_TERRAIN_HEIGHT - world::WATER_LEVEL) * 0.5 + MAX_TREE_HEIGHT;
    let extent = Vec3::new(
        cell_width * 0.5 + MAX_TREE_RADIUS,
        half_height,
        cell_width * 0.5 + MAX_TREE_RADIUS,
    );

    let mut visible = Vec::new();
    for dz in -radius..=radius {
        for dx in -radius..=radius {
            let cx = camera_cell.x as i32 + dx;
            let cz = camera_cell.y as i32 + dz;
            let canonical_x = cx.rem_euclid(axis);
            let canonical_z = cz.rem_euclid(axis);
            let cell_index = (canonical_z * axis + canonical_x) as usize;
            let cell = database.cells[cell_index];
            if cell.instance_count == 0 {
                continue;
            }
            let canonical_center = Vec2::new(
                (canonical_x as f32 + 0.5) * cell_width,
                (canonical_z as f32 + 0.5) * cell_width,
            );
            let delta = Vec2::new(
                (canonical_center.x - wrapped_camera.x + period * 0.5).rem_euclid(period)
                    - period * 0.5,
                (canonical_center.y - wrapped_camera.y + period * 0.5).rem_euclid(period)
                    - period * 0.5,
            );
            let center = Vec3::new(
                eye_rel.x + delta.x,
                world::WATER_LEVEL + half_height - origin.y,
                eye_rel.z + delta.y,
            );
            let horizontal_distance = delta.length();
            if horizontal_distance > VEGETATION_RANGE_METRES + extent.x {
                continue;
            }
            let clip_slack = 8.0
                * f32::EPSILON
                * (rows.w_axis.truncate().abs().dot(center.abs() + extent) + rows.w_axis.w.abs());
            let in_frustum = planes.iter().all(|plane| {
                plane.truncate().dot(center) + plane.w + plane.truncate().abs().dot(extent)
                    >= -clip_slack
            });
            if in_frustum {
                visible.push((center.distance_squared(eye_rel), cell));
            }
        }
    }
    visible.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
    let mut command_index = 0usize;
    for (distance_squared, cell) in visible {
        let distance = distance_squared.sqrt();
        let in_lod_fade = distance > VEGETATION_LOD_FADE_START_METRES
            && distance < VEGETATION_LOD_FADE_END_METRES;
        let emit = |commands: &mut [vk::DrawIndirectCommand], command_index: &mut usize,
                    full_lod: bool| {
            if *command_index >= commands.len() {
                return false;
            }
            commands[*command_index] = vk::DrawIndirectCommand {
                vertex_count: if full_lod {
                    world::vegetation::VEGETATION_INSTANCE_VERTICES
                } else {
                    world::vegetation::VEGETATION_MID_VERTICES
                },
                instance_count: cell.instance_count,
                first_vertex: if full_lod {
                    0
                } else {
                    world::vegetation::VEGETATION_INSTANCE_VERTICES
                },
                first_instance: cell.first_instance,
            };
            *command_index += 1;
            true
        };
        if in_lod_fade {
            if !emit(commands, &mut command_index, true)
                || !emit(commands, &mut command_index, false)
            {
                break;
            }
        } else if !emit(
            commands,
            &mut command_index,
            distance <= VEGETATION_LOD_FADE_START_METRES,
        ) {
            break;
        }
    }
}

/// Rewrite the far-field aggregate canopy command array. One command points
/// at one canonical cell record; the vertex shader expands that record into a
/// low-relief terrain-following field sample. Every visible canonical cell is
/// retained, including low-density records, so neighboring patches share the
/// same forest function at their boundaries.
pub(super) fn canopy_draw_commands(
    commands: &mut [vk::DrawIndirectCommand],
    database: &world::vegetation::VegetationDatabase,
    view_proj: Mat4,
    origin: Vec3,
    eye_rel: Vec3,
) {
    commands.fill(vk::DrawIndirectCommand::default());
    if database.canopies.is_empty() {
        return;
    }

    let rows = view_proj.transpose();
    let planes = [
        rows.w_axis + rows.x_axis,
        rows.w_axis - rows.x_axis,
        rows.w_axis + rows.y_axis,
        rows.w_axis - rows.y_axis,
        rows.z_axis,
        rows.w_axis - rows.z_axis,
    ];
    let period = world::WORLD_PERIOD as f32;
    let cell_width = world::vegetation::VEGETATION_CELL_METRES;
    let axis = world::vegetation::VEGETATION_CELLS_PER_AXIS as i32;
    let camera_world = origin + eye_rel;
    let wrapped_camera = Vec2::new(
        camera_world.x.rem_euclid(period),
        camera_world.z.rem_euclid(period),
    );
    let camera_cell = Vec2::new(
        (wrapped_camera.x / cell_width).floor(),
        (wrapped_camera.y / cell_width).floor(),
    );
    // 125 x 125 cells is bounded by CANOPY_COMMAND_CAPACITY while reaching
    // 7.8 km horizontally. The +1 margin prevents a cell edge from popping
    // out at the range boundary as the floating origin advances.
    let radius = (CANOPY_RANGE_METRES / cell_width).ceil() as i32 + 1;
    let extent = Vec3::new(
        cell_width * 0.5 + MAX_CANOPY_RADIUS,
        (world::MAX_TERRAIN_HEIGHT - world::WATER_LEVEL) * 0.5
            + world::vegetation::CANOPY_MAX_HEIGHT,
        cell_width * 0.5 + MAX_CANOPY_RADIUS,
    );

    let mut visible = Vec::new();
    for dz in -radius..=radius {
        for dx in -radius..=radius {
            let cx = camera_cell.x as i32 + dx;
            let cz = camera_cell.y as i32 + dz;
            let canonical_x = cx.rem_euclid(axis);
            let canonical_z = cz.rem_euclid(axis);
            let cell_index = (canonical_z * axis + canonical_x) as usize;
            // Empty aggregate records are retained for canonical indexing, but
            // they cannot contribute any visible canopy. Avoid launching their
            // fixed far-field vertex budget and fragment work.
            if world::vegetation::canopy_density(database.canopies[cell_index]) <= 0.0 {
                continue;
            }
            let canonical_center = Vec2::new(
                (canonical_x as f32 + 0.5) * cell_width,
                (canonical_z as f32 + 0.5) * cell_width,
            );
            let delta = Vec2::new(
                (canonical_center.x - wrapped_camera.x + period * 0.5).rem_euclid(period)
                    - period * 0.5,
                (canonical_center.y - wrapped_camera.y + period * 0.5).rem_euclid(period)
                    - period * 0.5,
            );
            let center = Vec3::new(
                eye_rel.x + delta.x,
                world::WATER_LEVEL + (world::MAX_TERRAIN_HEIGHT - world::WATER_LEVEL) * 0.5
                    - origin.y,
                eye_rel.z + delta.y,
            );
            if delta.length() > CANOPY_RANGE_METRES + extent.x {
                continue;
            }
            let clip_slack = 8.0
                * f32::EPSILON
                * (rows.w_axis.truncate().abs().dot(center.abs() + extent) + rows.w_axis.w.abs());
            let in_frustum = planes.iter().all(|plane| {
                plane.truncate().dot(center) + plane.w + plane.truncate().abs().dot(extent)
                    >= -clip_slack
            });
            if in_frustum {
                visible.push((center.distance_squared(eye_rel), cell_index as u32));
            }
        }
    }
    visible.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
    for (command, (_, cell_index)) in commands.iter_mut().zip(visible) {
        *command = vk::DrawIndirectCommand {
            vertex_count: world::vegetation::CANOPY_INSTANCE_VERTICES,
            instance_count: 1,
            first_vertex: 0,
            first_instance: cell_index,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn culling_keeps_periodic_cell_visible_across_rebase() {
        let database = world::vegetation::VegetationDatabase {
            instances: vec![[0; 4]],
            cells: {
                let mut cells = vec![
                    world::vegetation::VegetationCell::default();
                    world::vegetation::VEGETATION_CELL_COUNT as usize
                ];
                cells[0] = world::vegetation::VegetationCell {
                    first_instance: 0,
                    instance_count: 1,
                };
                cells
            },
            canopies: vec![[0; 4]; world::vegetation::VEGETATION_CELL_COUNT as usize],
        };
        let mut commands = vec![
            vk::DrawIndirectCommand::default();
            world::vegetation::VEGETATION_COMMAND_CAPACITY as usize
        ];
        let view = Mat4::look_at_rh(Vec3::new(0.0, 900.0, 0.0), Vec3::ZERO, Vec3::Z);
        let projection = Mat4::perspective_rh(1.2, 1.6, 0.1, 30_000.0);
        vegetation_draw_commands(
            &mut commands,
            &database,
            projection * view,
            Vec3::new(0.0, 900.0, 0.0),
            Vec3::ZERO,
        );
        assert!(commands.iter().any(|c| c.instance_count == 1));
    }

    #[test]
    fn far_canopy_culling_emits_a_canonical_cell_command() {
        let mut canopies = vec![[0; 4]; world::vegetation::VEGETATION_CELL_COUNT as usize];
        canopies[0] = [
            64.0f32.to_bits(),
            64.0f32.to_bits(),
            400.0f32.to_bits(),
            160u32 | (96u32 << 8),
        ];
        let database = world::vegetation::VegetationDatabase {
            instances: Vec::new(),
            cells: Vec::new(),
            canopies,
        };
        let mut commands = vec![
            vk::DrawIndirectCommand::default();
            world::vegetation::CANOPY_COMMAND_CAPACITY as usize
        ];
        let view = Mat4::look_at_rh(Vec3::new(0.0, 900.0, 0.0), Vec3::ZERO, Vec3::Z);
        let projection = Mat4::perspective_rh(1.2, 1.6, 0.1, 30_000.0);
        canopy_draw_commands(
            &mut commands,
            &database,
            projection * view,
            Vec3::new(0.0, 900.0, 0.0),
            Vec3::ZERO,
        );
        assert!(commands
            .iter()
            .any(|c| c.instance_count == 1 && c.first_instance == 0));
    }
}
