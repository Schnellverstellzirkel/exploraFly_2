//! Per-frame uniform packing: the shared UBO tail, the animated plume
//! proxy volume, and the trail ribbon rings. Uses the same node transform
//! path as the vertex shader so FX track the geometry exactly.

use ash::vk;
use glam::{Mat4, Vec3};

use crate::anim::Anim;
use crate::ubo::*;
use super::Plane;
use super::terrain_cmds::terrain_draw_commands;

impl Plane {
    pub unsafe fn update(
        &mut self,
        pose: &sim::flight::Pose,
        view_proj: &Mat4,
        origin: Vec3,
        eye_rel: Vec3,
        time: f32,
        image_index: usize,
        presented: bool,
        fx: &sim::effects::Effects,
        sim_stepped: bool,
        cam_frame: &sim::camera::CameraFrame,
        wind_strength: f32,
    ) {
        let pressure = Anim::pressure(pose.speed);
        // Use the same body attitude as physics and world-space emitters.
        let rotation = Mat4::from_quat(pose.orientation);
        let rel = Vec3::new(pose.x, pose.y, pose.z) - origin;
        let model = Mat4::from_translation(rel) * rotation;
        let campos = eye_rel;
        let inv_view_proj = view_proj.inverse();
        // One coherent copy: view-proj, inv-view-proj, all nodes, plane frame, flex, camera, sun.
        let dst = self.ubo_mapped[image_index] as *mut f32;
        if presented {
            let commands = std::slice::from_raw_parts_mut(
                self.ubo_mapped[image_index].add(UBO_BYTES) as *mut vk::DrawIndexedIndirectCommand,
                world::TERRAIN_CHUNK_COUNT as usize,
            );
            terrain_draw_commands(commands, *view_proj, origin, eye_rel);
        }
        std::ptr::copy_nonoverlapping(view_proj.to_cols_array().as_ptr(), dst, 16);
        std::ptr::copy_nonoverlapping(inv_view_proj.to_cols_array().as_ptr(), dst.add(16), 16);
        for n in 0..NODE_COUNT {
            let m = model * self.node_matrix(n);
            std::ptr::copy_nonoverlapping(m.to_cols_array().as_ptr(), dst.add(32 + n * 16), 16);
        }
        // Per-node instance transforms for the ray-traced shadow TLAS.
        // Host-coherent: the measured pass builds the structure from these.
        if self.rt_supported
            && self.rt_instance_count > 0
            && presented
            && image_index < self.rt_instance_mapped.len()
        {
            let instances = self.rt_instance_mapped[image_index] as *mut vk::AccelerationStructureInstanceKHR;
            for (i, node) in self.rt_geom_nodes.iter().enumerate() {
                let m = model * self.node_matrix(*node as usize);
                let c = m.to_cols_array();
                let mask: u8 = if *node == 2 || (*node >= 4 && *node <= 6) {
                    0x02
                } else if *node == 3 || (*node >= 7 && *node <= 9) {
                    0x04
                } else {
                    0x01
                };
                let inst = vk::AccelerationStructureInstanceKHR {
                    transform: vk::TransformMatrixKHR {
                        matrix: [
                            c[0], c[4], c[8], c[12],
                            c[1], c[5], c[9], c[13],
                            c[2], c[6], c[10], c[14],
                        ],
                    },
                    instance_custom_index_and_mask: vk::Packed24_8::new(*node as u32, mask),
                    instance_shader_binding_table_record_offset_and_flags: vk::Packed24_8::new(
                        0,
                        vk::GeometryInstanceFlagsKHR::TRIANGLE_FACING_CULL_DISABLE.as_raw() as u8,
                    ),
                    acceleration_structure_reference: vk::AccelerationStructureReferenceKHR {
                        device_handle: self.rt_blas_addresses[i],
                    },
                };
                unsafe {
                    *instances.add(i) = inst;
                }
            }
            if self.rt_terrain_blas_address != 0 {
                let inst_idx = self.rt_geom_nodes.len();
                let period = world::WORLD_PERIOD as f32;
                let camera_world = origin + eye_rel;
                let base_tile_x = (camera_world.x / period).floor() * period;
                let base_tile_z = (camera_world.z / period).floor() * period;
                let tx = base_tile_x - origin.x;
                let ty = -origin.y;
                let tz = base_tile_z - origin.z;
                let inst = vk::AccelerationStructureInstanceKHR {
                    transform: vk::TransformMatrixKHR {
                        matrix: [
                            1.0, 0.0, 0.0, tx,
                            0.0, 1.0, 0.0, ty,
                            0.0, 0.0, 1.0, tz,
                        ],
                    },
                    instance_custom_index_and_mask: vk::Packed24_8::new(100, 0x10),
                    instance_shader_binding_table_record_offset_and_flags: vk::Packed24_8::new(
                        0,
                        vk::GeometryInstanceFlagsKHR::TRIANGLE_FACING_CULL_DISABLE.as_raw() as u8,
                    ),
                    acceleration_structure_reference: vk::AccelerationStructureReferenceKHR {
                        device_handle: self.rt_terrain_blas_address,
                    },
                };
                unsafe {
                    *instances.add(inst_idx) = inst;
                }
            }
            if self.rt_structures_blas_address != 0 {
                let inst_idx = self.rt_geom_nodes.len() + 1;
                let period = world::WORLD_PERIOD as f32;
                let camera_world = origin + eye_rel;
                let base_tile_x = (camera_world.x / period).floor() * period;
                let base_tile_z = (camera_world.z / period).floor() * period;
                let tx = base_tile_x - origin.x;
                let ty = -origin.y;
                let tz = base_tile_z - origin.z;
                let inst = vk::AccelerationStructureInstanceKHR {
                    transform: vk::TransformMatrixKHR {
                        matrix: [
                            1.0, 0.0, 0.0, tx,
                            0.0, 1.0, 0.0, ty,
                            0.0, 0.0, 1.0, tz,
                        ],
                    },
                    instance_custom_index_and_mask: vk::Packed24_8::new(101, 0x08),
                    instance_shader_binding_table_record_offset_and_flags: vk::Packed24_8::new(
                        0,
                        vk::GeometryInstanceFlagsKHR::TRIANGLE_FACING_CULL_DISABLE.as_raw() as u8,
                    ),
                    acceleration_structure_reference: vk::AccelerationStructureReferenceKHR {
                        device_handle: self.rt_structures_blas_address,
                    },
                };
                unsafe {
                    *instances.add(inst_idx) = inst;
                }
            }
        }
        let sun_radius = SUN_RADIUS;
        let sun_elevation = SUN_ELEVATION;
        let sun_dir = SUN_DIR;
        let sun_irr = SUN_IRRADIANCE;
        let flicker = 0.90 + 0.06 * (time * 57.0).sin() + 0.04 * (time * 91.0).sin();
        let glow = (0.8 + self.anim.spool * 2.2) * flicker;
        let zenith_sky = SKY_ZENITH;
        let horizon_haze = SKY_HORIZON;
        let ground_base = GROUND_BASE;
        let cos_radius = SUN_COS_RADIUS;
        let inv_one_minus_cos_radius = INV_ONE_MINUS_SUN_COS_RADIUS;
        // FX uniforms in spare tail slots.
        let spool = self.anim.spool;
        let lambda = fx.plume.cell_lambda;
        let (_, exit_radius) = self.nozzle_exit();
        // The ground is authored at world Y=0. Store it relative to the same
        // floating origin used by the camera and airframe.
        let ground_height = GROUND_LEVEL - origin.y;
        let ground_origin = ground_origin_pack(origin);
        let mut shift = Vec3::ZERO;
        if presented {
            self.fill_cone(image_index, fx, model, rotation);
            let filled = self.trail_filled.get(image_index).copied().unwrap_or(false);
            if sim_stepped || !filled {
                self.fill_trail(image_index, fx, origin, eye_rel);
                if image_index < self.trail_origin.len() {
                    self.trail_origin[image_index] = origin;
                    self.trail_filled[image_index] = true;
                }
            } else if image_index < self.trail_origin.len() {
                shift = self.trail_origin[image_index] - origin;
            }
        }

        let tail: [f32; 56] = [
            self.anim.bend,
            time,
            pressure,
            glow,
            campos.x,
            campos.y,
            campos.z,
            exit_radius,
            sun_dir.x,
            sun_dir.y,
            sun_dir.z,
            sun_radius,
            sun_irr.x,
            sun_irr.y,
            sun_irr.z,
            sun_elevation,
            zenith_sky.x,
            zenith_sky.y,
            zenith_sky.z,
            cos_radius,
            horizon_haze.x,
            horizon_haze.y,
            horizon_haze.z,
            inv_one_minus_cos_radius,
            ground_base.x,
            ground_base.y,
            ground_base.z,
            ground_height,
            // detail: shared plume flicker, shock-cell wavelength, spool, length.
            flicker,
            lambda,
            spool,
            fx.plume.length_m,
            // packed-origin minus current origin for trail.vert.
            shift.x,
            shift.y,
            shift.z,
            0.0,
            // Camera & optics parameters:
            cam_frame.fov_y,
            cam_frame.aspect,
            cam_frame.speed,
            cam_frame.load,
            cam_frame.shake_intensity,
            cam_frame.exposure,
            cam_frame.mach,
            // cameraParams2.w: cloud advection shares the CPU weather strength.
            wind_strength,
            // groundOrigin: floor(origin.xz / 0.25 m), then the positive
            // sub-cell remainder. ground.frag reconstructs stable global noise
            // cells from this split without large-coordinate cancellation.
            ground_origin[0],
            ground_origin[1],
            ground_origin[2],
            ground_origin[3],
            self.hud[0], self.hud[1], self.hud[2], self.hud[3],
            self.hud[4], self.hud[5], self.hud[6], self.hud[7],
        ];
        std::ptr::copy_nonoverlapping(tail.as_ptr(), dst.add(32 + NODE_COUNT * 16), tail.len());
    }

    /// Rewrite the host-visible volume bounds to nozzle state (relative to origin).
    /// Uses the exact same model and rotation transforms as the aircraft airframe and
    /// plume shaders, guaranteeing 1-to-1 sync during violent rolls, dives, and pitch maneuvers.
    unsafe fn fill_cone(
        &mut self,
        image_index: usize,
        fx: &sim::effects::Effects,
        model: Mat4,
        rotation: Mat4,
    ) {
        if image_index >= self.cone_mapped.len() {
            return;
        }
        let (nozzle_local, exit_radius) = self.nozzle_exit();
        let nozzle = (model * nozzle_local.extend(1.0)).truncate();
        let dir = (rotation * glam::Vec4::new(0.0, 0.0, -1.0, 0.0)).truncate().normalize_or_zero();
        let u = (rotation * glam::Vec4::new(1.0, 0.0, 0.0, 0.0)).truncate().normalize_or_zero();
        let v = (rotation * glam::Vec4::new(0.0, 1.0, 0.0, 0.0)).truncate().normalize_or_zero();
        // Conservative axial margin: the proxy extends 35% beyond nominal fluid
        // length so the downstream end cap and polygon edges remain in empty air
        // where fluid density and emission have already reached strictly zero.
        let len = fx.plume.length_m.max(0.5) * PLUME_AXIAL_BOUND;
        // The generated curl field has components bounded to +/-1.0, so
        // 1.35 widths conservatively contain every warped contributing ray.
        // Circumscribe that envelope at each z so the proxy cannot clip it.
        let radial_bound =
            PLUME_WARP_BOUND
                / (std::f32::consts::PI / crate::fx_gpu::CONE_SEGMENTS as f32).cos();
        let spool = self.anim.spool;
        let dst = self.cone_mapped[image_index] as *mut f32;
        let n = self.unit_cone.len();
        for (i, uv) in self.unit_cone.iter().enumerate().take(n) {
            let z = -uv[2] * len;
            let width = (exit_radius * (1.0 + 0.12 * spool) + z * 0.055).max(0.05);
            let rad = width * radial_bound;
            let w = nozzle + u * (uv[0] * rad) + v * (uv[1] * rad) + dir * z;
            *dst.add(i * 5) = w.x;
            *dst.add(i * 5 + 1) = w.y;
            *dst.add(i * 5 + 2) = w.z;
            *dst.add(i * 5 + 3) = uv[3];
            *dst.add(i * 5 + 4) = uv[4];
        }
    }

    /// Pack newest live trail segments into the host-visible ribbon buffer.
    /// Scans back from each pool head (newest first), capped per emitter.
    unsafe fn fill_trail(
        &mut self,
        image_index: usize,
        fx: &sim::effects::Effects,
        origin: Vec3,
        eye_rel: Vec3,
    ) {
        use sim::effects::{EMITTER_COUNT, POOL_N};
        use crate::fx_gpu::{ribbon_quad, TRAIL_MAX_QUADS_PER_EMITTER};
        if image_index >= self.trail_mapped.len() {
            return;
        }
        const SCAN_PER_EMITTER: usize = 1200;
        const MAX_VERTS: usize = EMITTER_COUNT * TRAIL_MAX_QUADS_PER_EMITTER * 2;
        let dst = self.trail_mapped[image_index] as *mut f32;
        // Fixed emitter slots must agree with build_trail_indices.
        std::ptr::write_bytes(dst, 0, MAX_VERTS * 13);
        for e in 0..EMITTER_COUNT {
            let pool = &fx.pools[e];
            let mut quads = 0usize;
            let mut prev_center = fx.emitters[e].pos - origin;
            let mut previous_side = Vec3::ZERO;
            let mut last = [[0.0; 13]; 2];
            // Dead pools stay short: never scan past what was ever pushed.
            let scan = SCAN_PER_EMITTER.min(pool.live);
            for k in 0..scan {
                if quads == TRAIL_MAX_QUADS_PER_EMITTER { break; }
                let idx = (pool.head + POOL_N - 1 - k) % POOL_N;
                let s = &pool.segs[idx];
                if s.density <= 0.0 || s.age >= s.life { continue; }
                let center = s.pos - origin;
                let older = &pool.segs[(idx + POOL_N - 1) % POOL_N];
                let tangent = if older.density > 0.0 && older.age < older.life {
                    older.pos - (if quads == 0 { fx.emitters[e].pos } else { prev_center + origin })
                } else { center - prev_center };
                let cam_dir = (eye_rel - center).normalize_or_zero();
                let formation = if e == 0 { 12.0 } else { 3.0 } / fx.speed_ms.max(20.0);
                let fade = ((s.age - formation) / formation).clamp(0.0, 1.0);
                let fade = fade * fade * (3.0 - 2.0 * fade);
                let tail = ((TRAIL_MAX_QUADS_PER_EMITTER - 1 - quads) as f32 / 24.0).clamp(0.0, 1.0);
                let mut quad = ribbon_quad(
                    center, center - tangent, cam_dir, s.radius.max(0.025), s.age,
                    s.density * fade * tail, [s.flow_uv[0] * 2.0, 0.0], e as f32 * 0.173, s.ice,
                );
                // Keep the ribbon's two edges continuous when viewed along its axis.
                let side = Vec3::new(quad[0][3], quad[0][4], quad[0][5]);
                if side.dot(previous_side) < 0.0 {
                    for v in &mut quad { for c in &mut v[3..6] { *c = -*c; } }
                }
                previous_side = Vec3::new(quad[0][3], quad[0][4], quad[0][5]);
                let base = (e * TRAIL_MAX_QUADS_PER_EMITTER + quads) * 26;
                std::ptr::copy_nonoverlapping(quad[0].as_ptr(), dst.add(base), 13);
                std::ptr::copy_nonoverlapping(quad[1].as_ptr(), dst.add(base + 13), 13);
                last = quad;
                quads += 1;
                prev_center = center;
            }
            // Collapse the unused tail at the final point, never at world origin.
            if quads > 0 {
                for v in &mut last { v[7] = 0.0; }
                for q in quads..TRAIL_MAX_QUADS_PER_EMITTER {
                    let base = (e * TRAIL_MAX_QUADS_PER_EMITTER + q) * 26;
                    std::ptr::copy_nonoverlapping(last[0].as_ptr(), dst.add(base), 13);
                    std::ptr::copy_nonoverlapping(last[1].as_ptr(), dst.add(base + 13), 13);
                }
            }
        }
    }

}
