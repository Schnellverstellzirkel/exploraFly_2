//! The dedicated render/simulation thread: fixed-step aerodynamics,
//! camera-relative floating-origin transforms, batched submission, and
//! per-present telemetry.

use crate::app::{controls_from, Shared, UserEvent, simulation_steps};
use crate::flags;
use crate::frame_budget;
use crate::gfx::{DrawResult, Gfx, StageStats, SHADER_MARKER};
use crate::vendor;
use crate::audio;
use crate::hud;
use sim::camera::ChaseCamera;
use sim::effects::{self, Effects};
use sim::flight::{Pose, SIM_STEP};
use sim::wind::Wind;
use std::sync::atomic::Ordering;
use std::time::Instant;
use winit::window::Window;

/// Dedicated high-frequency render and simulation loop thread.
///
/// Runs decoupled from the window event loop:
/// - Advances aerodynamic flight simulation in fixed `SIM_STEP` intervals.
/// - Calculates camera-relative floating-origin transforms.
/// - Submits batched Vulkan command buffers and handles swapchain presentation.
/// - Profiles CPU/GPU stage latencies and updates window title with real-time telemetry.
pub(crate) fn render_main(
    window: std::sync::Arc<Window>,
    shared: std::sync::Arc<Shared>,
    proxy: winit::event_loop::EventLoopProxy<UserEvent>,
) {
    vendor::pin_to_performance_cores();
    let mut gfx = unsafe { Gfx::new(&window) };
    {
        let mut hash = 0u64;
        for b in SHADER_MARKER.bytes() {
            hash = hash.wrapping_mul(1099511628211).wrapping_add(b as u64);
        }
        println!("plane shader hash: {:016x}", hash);
    }
    let mut vendor = unsafe { vendor::Vendor::open() };
    let wind = Wind::new(flags::wind_strength());
    println!(
        "weather wind strength: {} (0 = calm, 1 = breeze, 3 = strong)",
        wind.strength()
    );
    let mut pose = Pose::start();
    pose.x = world::SPAWN_X;
    pose.z = world::SPAWN_Z;
    pose.y = world::SPAWN_ALTITUDE;
    // Diagnostic start coordinates keep visual acceptance of distant biomes
    // reproducible without flying a benchmark aircraft across the whole tile.
    // DEBUG_ONLY: release builds ignore EXPLORA_X / Z / ALT / HEADING.
    if let Some(x) = flags::spawn_x() {
        pose.x = x;
    }
    if let Some(z) = flags::spawn_z() {
        pose.z = z;
    }
    if let Some(alt) = flags::spawn_alt() {
        pose.y = alt;
    }
    if let Some(hdg) = flags::spawn_heading() {
        pose.heading = hdg;
        pose.orientation = glam::Quat::from_rotation_y(hdg);
        pose.velocity = pose.orientation * glam::Vec3::Z * pose.speed;
    }
    // Start trimmed relative to the air mass, with its drift already included
    // in world velocity. This avoids a sudden sideslip impulse at spawn.
    pose.velocity += wind.velocity(glam::Vec3::new(pose.x, pose.y, pose.z), 0.0);
    let mut world_neighborhood = world::WorldNeighborhood::new();
    pose.y = pose.y.max(
        world_neighborhood.collision_height_at(pose.x as f64, pose.z as f64)
            + world::CLEARANCE_METRES,
    );
    let spawn_pose = pose;
    let audio = audio::Audio::start();
    let mut prev_pose = pose;
    let mut previous_camera_acceleration = glam::Vec3::ZERO;
    let mut camera_acceleration = glam::Vec3::ZERO;
    let mut fx = effects::Effects::new();
    fx.fx_enabled = flags::fx_enabled();
    let mut chase_cam = ChaseCamera::new();
    chase_cam.set_airframe_bounds(gfx.plane.camera_bounds());
    let mut accumulator = 0.0f32;
    let mut simulation_time = 0.0f32;
    let mut last = Instant::now();
    let boot = last;
    let mut stages = StageStats::default();
    let mut stat_timer = 0.0f32;
    let mut stat_frames = 0u32;
    let mut stat_presents = 0u32;
    let mut stat_skipped = 0u64;
    let shot_path = flags::shot_path();
    let shot_frames = flags::shot_frames();
    let mut presented_total = 0u64;
    // Benchmark uses present requests, never attachment-free burst draws.
    let benchmarking = gfx.benchmark.is_some();
    // Play mode paces frames to the display scanout (VK_KHR_present_wait) so
    // the simulation is sampled once per refresh. Benchmarks must stay
    // unpaced to measure the submission-rate target. EXPLORA_PACING=off is
    // DEBUG_ONLY for diagnosing display problems.
    let mut display_pacing = !benchmarking && flags::display_pacing(true);
    let mut pacing_strikes = 0u32;
    let frozen = flags::frozen();
    // EXPLORA_FREEZE=pose freezes the sim pose but lets time run,
    // isolating time-driven terms from pose-driven ones.
    let freeze_pose = flags::freeze_pose();
    // Startup-only diagnostics: do not perform environment lookups at 1000 Hz.
    let force_boost = flags::force_boost();
    let force_bank = flags::force_bank();
    let force_pitch = flags::force_pitch();
    loop {
        if shared.should_exit() {
            break;
        }
        let now = Instant::now();
        let wall_dt = (now - last).as_secs_f32();
        let dt = wall_dt.clamp(0.0, 0.1);
        last = now;
        accumulator += dt;
        let ui = shared.ui.fetch_and(!hud::RESET, Ordering::Relaxed);
        let paused = ui & hud::PAUSED != 0;
        if ui & hud::RESET != 0 {
            pose = spawn_pose;
            prev_pose = pose;
            previous_camera_acceleration = glam::Vec3::ZERO;
            camera_acceleration = glam::Vec3::ZERO;
            fx = Effects::new();
            fx.fx_enabled = flags::fx_enabled();
            gfx.plane.reset_flight();
            simulation_time = 0.0;
            chase_cam.snap(&pose);
            accumulator = 0.0;
        }
        if paused {
            accumulator = 0.0;
            prev_pose = pose;
            previous_camera_acceleration = glam::Vec3::ZERO;
            camera_acceleration = glam::Vec3::ZERO;
        }
        let cpu0 = Instant::now();
        let mut controls = controls_from(shared.keys.load(Ordering::Relaxed));
        // Screenshot helpers: force flight regimes without keyboard input.
        // EXPLORA_BOOST=1 holds full burner, EXPLORA_BANK=1 holds a hard left turn.
        if force_boost {
            controls.boost = true;
        }
        if force_bank {
            controls.bank = 1.0;
        }
        if let Some(pitch) = force_pitch { controls.pitch = pitch; }
        // dt is capped at 100 ms, so draining it is bounded to at most 15
        // fixed steps. Preserve the fractional remainder for interpolation.
        let steps = simulation_steps(&mut accumulator);
        for _ in 0..steps {
            prev_pose = pose;
            let air_motion = wind.velocity(
                glam::Vec3::new(pose.x, pose.y, pose.z),
                simulation_time,
            );
            if !freeze_pose {
                let previous_air_velocity = pose.velocity - air_motion;
                pose.step_with_wind(&controls, SIM_STEP, air_motion);
                // Gentle free-flight safety floor, including lake and landmark roofs.
                let floor = world_neighborhood.collision_height_at(pose.x as f64, pose.z as f64)
                    + world::CLEARANCE_METRES;
                if pose.y < floor { pose.y = floor; pose.velocity.y = pose.velocity.y.max(0.0); }
                previous_camera_acceleration = camera_acceleration;
                camera_acceleration = ((pose.velocity - air_motion - previous_air_velocity)
                    / SIM_STEP)
                    .clamp_length_max(120.0);
            } else {
                previous_camera_acceleration = glam::Vec3::ZERO;
                camera_acceleration = glam::Vec3::ZERO;
            }
            if !frozen {
                gfx.plane.step_animation(&controls, &pose, SIM_STEP);
                // FX sim at same 144 Hz: emitters from node path, load from wing G.
                let (epos, edir) = gfx.plane.emitter_world(&pose);
                let load = pose.load.clamp(-4.0, 3.5);
                fx.step_with_wind(
                    SIM_STEP,
                    &epos,
                    &edir,
                    gfx.plane.engine_spool(),
                    pose.speed,
                    pose.y,
                    load,
                    air_motion,
                );
                simulation_time += SIM_STEP;
            }
        }
        // Sub-step pose interpolation: eliminates simulation-to-render beat-frequency
        // micro-stutter and tail jitter at any display refresh rate (60Hz, 165Hz, 240Hz, or uncapped).
        let alpha = if freeze_pose || paused {
            0.0
        } else {
            (accumulator / SIM_STEP).clamp(0.0, 1.0)
        };
        let render_pose = prev_pose.interpolate(&pose, alpha);
        let render_camera_acceleration = if paused || frozen || freeze_pose {
            glam::Vec3::ZERO
        } else {
            previous_camera_acceleration.lerp(camera_acceleration, alpha)
        };
        let audible = ui & hud::AUDIO != 0 && !paused && !frozen;
        audio.update(
            sim::audio::AcousticState {
                airspeed: render_pose.speed,
                mach: render_pose.mach,
                altitude: render_pose.y,
                spool: gfx.plane.engine_spool(),
                boost: render_pose.boost,
                load: render_pose.load,
                aoa: render_pose.aoa,
                sideslip: render_pose.sideslip,
                pitch_rate: render_pose.rates.x,
                roll_rate: render_pose.rates.z,
                vertical_speed: render_pose.velocity.y,
                separation: render_pose.separation,
                volume: if audible { 1.0 } else { 0.0 },
            },
            audible,
        );
        gfx.plane.set_hud(hud::pack(render_pose.speed, render_pose.y, render_pose.heading,
            render_pose.velocity.y, gfx.plane.engine_spool(),
            render_pose.y
                - world_neighborhood.collision_height_at(render_pose.x as f64, render_pose.z as f64),
            ui));

        let sim_stepped = steps > 0;
        let cpu1 = Instant::now();
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            continue;
        }
        let aspect = size.width as f32 / size.height as f32;
        // Floating origin at the plane. World coordinates reach
        // kilometers; rendering relative keeps float32 exact.
        let origin = glam::Vec3::new(render_pose.x, render_pose.y, render_pose.z);
        fx.set_origin(origin);
        let camera_wind = wind.velocity(origin, simulation_time);
        let camera_step_start = Instant::now();
        let cam_frame = chase_cam.step_with_wind_collision_and_acceleration(
            &render_pose,
            &controls,
            if paused || frozen { 0.0 } else { dt },
            aspect,
            origin,
            camera_wind,
            render_camera_acceleration,
            |x, z| world_neighborhood.collision_height_at(x, z),
        );
        let camera_step_ns = camera_step_start.elapsed().as_nanos() as u64;
        stages.add_camera_detail(camera_step_ns, cam_frame.collision_queries, benchmarking);
        let view_proj = cam_frame.view_proj;
        let eye_rel = cam_frame.eye_rel;
        let cpu2 = Instant::now();
        stages.add_cpu(
            cpu1.duration_since(cpu0).as_nanos() as u64,
            cpu2.duration_since(cpu1).as_nanos() as u64,
        );
        if gfx.extent.width != size.width || gfx.extent.height != size.height {
            unsafe { gfx.recreate(&window) };
            if let Some(capture) = gfx.benchmark.as_mut() { capture.restart(); }
            stages = StageStats::default();
            continue;
        }
        match unsafe {
            gfx.draw(
                &render_pose,
                &controls,
                &view_proj,
                origin,
                eye_rel,
                simulation_time,
                &fx,
                sim_stepped,
                &mut stages,
                &cam_frame,
                wind.strength(),
            )
        }
 {
            DrawResult::Rebuild => {
                unsafe { gfx.recreate(&window) };
                if let Some(capture) = gfx.benchmark.as_mut() { capture.restart(); }
                stages = StageStats::default();
                continue;
            }
            DrawResult::Skipped => {
                stat_skipped += 1;
            }
            DrawResult::Presented(rendered) => {
                stat_frames += rendered;
                stat_presents += 1;
                presented_total += 1;
                if let Some(path) = shot_path.as_ref() {
                    if shot_frames.contains(&presented_total) {
                        let numbered = match path.rfind('.') {
                            Some(dot) => {
                                format!("{}-{}{}", &path[..dot], presented_total, &path[dot..])
                            }
                            None => format!("{}-{}", path, presented_total),
                        };
                        unsafe { gfx.screenshot(&numbered) };
                    }
                }
                if benchmarking {
                    let progress = gfx.benchmark.as_mut().unwrap()
                        .submitted(Instant::now(), boot.elapsed(), gfx.present_id);
                    if progress == frame_budget::Progress::Started {
                        stages = StageStats::default();
                        stages.add_camera_detail(
                            camera_step_ns,
                            cam_frame.collision_queries,
                            true,
                        );
                    }
                    if gfx.present_id % 64 == 0 || progress == frame_budget::Progress::Complete {
                        unsafe { gfx.collect_display_timings(); }
                    }
                    if progress == frame_budget::Progress::Complete {
                        let (acq, wait_fence, sub, pre, sim_ns, cam_ns, fx_ns, gpu_us) = stages.report();
                        let gp = stages.gpu_pass_avg();
                        println!("stages per present: acquire {acq} us fence {wait_fence} us submit {sub} us present {pre} us | sim {:.1} us camera-prep+sweep {:.1} us fx {:.1} us gpu {gpu_us} us [opq+rt {} ter {} trees {} canopy {} cld {} sky {} plu {} trl {} gls {} cmp {}]",
                            sim_ns as f64 / 1000.0, cam_ns as f64 / 1000.0, fx_ns as f64 / 1000.0,
                            gp[0], gp[1], gp[2], gp[3], gp[4], gp[5], gp[6], gp[7], gp[8], gp[9]);
                        unsafe {
                            gfx.finish_gpu_capture();
                            gfx.collect_display_timings();
                        }
                        let camera_diagnostics = stages.camera_diagnostics_json();
                        let metadata = format!("{},\"width\":{},\"height\":{},\"scene_width\":{},\"scene_height\":{},\"present_mode\":\"{:?}\",\"camera_step\":{camera_diagnostics}}}",
                            gfx.benchmark_metadata.trim_end_matches('}'),
                            gfx.extent.width, gfx.extent.height, gfx.scene_extent.width, gfx.scene_extent.height, gfx.present_mode);
                        let metadata = format!("{},\"hud_flags\":{},\"audio_requested\":{},\"wind\":{},\"force_boost\":{},\"force_bank\":{},\"force_pitch\":{},\"frozen\":{}}}",
                            metadata.strip_suffix('}').expect("benchmark metadata object"), ui & 15,
                            flags::audio_requested() && ui & hud::AUDIO != 0,
                            wind.strength(), force_boost, force_bank,
                            force_pitch.map(|p| p.to_string()).unwrap_or_else(|| "null".into()), freeze_pose);
                        let json = gfx.benchmark.as_mut().unwrap().report(&metadata);
                        println!("{json}");
                        if let Some(path) = flags::bench_json() {
                            std::fs::write(path, format!("{json}\n")).expect("benchmark JSON output");
                        }
                        break;
                    }
                }
            }
        }
        if display_pacing {
            // A single timeout is tolerated (dropped frame, window drag);
            // repeated failures mean present_wait does not actually fire on
            // this compositor, so stop pacing instead of throttling frames.
            if unsafe { gfx.wait_displayed(60_000_000) } {
                pacing_strikes = 0;
            } else {
                pacing_strikes += 1;
                if pacing_strikes > 2 {
                    display_pacing = false;
                    eprintln!("display pacing unavailable: present_wait never completed; continuing unpaced");
                }
            }
        }
        stat_timer += wall_dt;
        if stat_timer >= 1.0 && !benchmarking {
            let render_pass_rate = stat_frames as f32 / stat_timer;
            let present_rate = stat_presents as f32 / stat_timer;
            stat_timer = 0.0;
            stat_frames = 0;
            stat_presents = 0;
            let skipped = stat_skipped;
            stat_skipped = 0;
            let (acq, wait_fence, sub, pre, sim_ns, cam_ns, fx_ns, gpu_us) = stages.report();
            let gp = stages.gpu_pass_avg();
            stages = StageStats::default();
            let stats = vendor.sample();
            println!(
                "theoretical fps: {:.1} FPS ({:.1} us/frame) | real fps: {:.1} FPS | raster passes: {:.1}/s | present submissions: {:.1}/s | acq {acq} fence {wait_fence} sub {sub} pre {pre} us | sim {sim_ns} cam {cam_ns} fx {fx_ns} ns gpu {gpu_us} us [opq+rt {} ter {} trees {} canopy {} cld {} sky {} plu {} trl {} gls {} cmp {}] | skipped {} | speed {:.0} kt {} | GPU {}C {}MHz | fx noz {} tip {} plume {:.1}m M{:.2} lam{:.2}",
                render_pass_rate,
                1_000_000.0 / render_pass_rate.max(1.0),
                present_rate,
                render_pass_rate,
                present_rate,
                gp[0], gp[1], gp[2], gp[3], gp[4], gp[5], gp[6], gp[7], gp[8], gp[9],
                skipped,
                pose.speed * 1.944,
                if pose.boost > 0.5 { "BOOST" } else { "glide" },
                stats.temp_c,
                stats.clock_mhz,
                fx.live_count(effects::EMITTER_NOZZLE),
                fx.live_count(effects::EMITTER_TIP_L) + fx.live_count(effects::EMITTER_TIP_R),
                fx.plume.length_m,
                fx.plume.mj,
                fx.plume.cell_lambda,
            );
            window.set_title(&format!(
                "explora | theoretical {:.0} FPS | real {:.0} FPS | {:.0} kt {} | GPU {}C {}MHz",
                render_pass_rate,
                present_rate,
                pose.speed * 1.944,
                if pose.boost > 0.5 { "BOOST" } else { "glide" },
                stats.temp_c,
                stats.clock_mhz,
            ));
        }
    }
    // Ensure all in-flight GPU operations have completed before destroying resources.
    unsafe {
        gfx.device.device_wait_idle().ok();
    }
    let _ = proxy.send_event(UserEvent::RenderDone);
}
