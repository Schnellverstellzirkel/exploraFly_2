// Native boot: window, Vulkan device, swapchain, fixed loop.
// One canvas. One GPU. Fixed passes. No fallback.

mod anim;
mod atmo_lut;
mod audio;
mod clouds;
mod detail;
mod frame_budget;
mod fx_gpu;
mod hud;
mod lut;
mod plane;
mod quality;
mod ubo;
mod vendor;
mod world_conformance;

use winit::event_loop::EventLoop;


mod app;
mod frame_loop;
mod gfx;

#[cfg(test)]
use ash::vk;

pub(crate) use gfx::{find_memory_type, spv_words};
use app::{handle_signal, App, Shared, UserEvent};
use std::sync::atomic::{AtomicBool, AtomicU32};

fn main() {
    if std::env::args().any(|arg| arg == "--world-conformance") {
        if let Err(error) = unsafe { world_conformance::run() } {
            eprintln!("world conformance: FAIL: {error}");
            std::process::exit(1);
        }
        return;
    }
    // Low-level hardware & driver tuning for fixed RTX 4060 target:
    // - Uncap driver vblank lock to enable true uncapped presentation (>1,400 FPS)
    // - Set yield policy to NOTHING: replaces thread sleep/yield with busy spinlock for zero-latency WSI dispatch
    // - Enable multi-threaded driver command optimizations
    if std::env::var_os("__GL_SYNC_TO_VBLANK").is_none() {
        std::env::set_var("__GL_SYNC_TO_VBLANK", "0");
    }
    if std::env::var_os("__GL_YIELD").is_none() {
        std::env::set_var("__GL_YIELD", "NOTHING");
    }
    if std::env::var_os("__GL_THREADED_OPTIMIZATIONS").is_none() {
        std::env::set_var("__GL_THREADED_OPTIMIZATIONS", "1");
    }
    if std::env::var_os("__GL_GSYNC_ALLOWED").is_none() {
        std::env::set_var("__GL_GSYNC_ALLOWED", "1");
    }

    // Install SIGINT and SIGTERM handlers to cleanly terminate via terminal Ctrl+C
    unsafe {
        libc::signal(
            libc::SIGINT,
            handle_signal as *const () as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGTERM,
            handle_signal as *const () as libc::sighandler_t,
        );
    }
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .expect("event loop");
    let proxy = event_loop.create_proxy();
    let mut app = App {
        window: None,
        shared: std::sync::Arc::new(Shared {
            exit: AtomicBool::new(false),
            keys: AtomicU32::new(0),
            ui: AtomicU32::new(if std::env::var("EXPLORA_HUD").as_deref() == Ok("0") { hud::DEFAULT & !hud::VISIBLE } else { hud::DEFAULT }),
        }),
        proxy,
        render_thread: None,
        ctrl_pressed: false,
    };
    event_loop.run_app(&mut app).expect("run");
}

#[cfg(test)]
mod tests {
    #[test]
    fn stalled_frames_keep_simulation_time_and_interpolation_phase() {
        let mut accumulator = 0.0;
        let mut elapsed = 0.0f64;
        let mut ticks = 0u32;
        for dt in [0.004f32, 0.035, 0.001, 0.060, 0.003, 0.100, 0.017, 0.008] {
            accumulator += dt;
            elapsed += dt as f64;
            let steps = crate::app::simulation_steps(&mut accumulator);
            assert!(steps <= 15);
            if dt >= 0.060 { assert!(steps > 5); }
            ticks += steps;
            assert!((0.0..sim::flight::SIM_STEP).contains(&accumulator));
            assert!((ticks as f64 * sim::flight::SIM_STEP as f64 + accumulator as f64 - elapsed).abs() < 1e-7);
        }
    }
    use super::*;

    #[test]
    fn queried_present_features_form_an_acyclic_device_chain() {
        let mut present_id = vk::PhysicalDevicePresentIdFeaturesKHR::default().present_id(true);
        let mut present_wait = vk::PhysicalDevicePresentWaitFeaturesKHR::default().present_wait(true);
        // A capability query leaves the extension structures linked together.
        let _query = vk::PhysicalDeviceFeatures2::default()
            .push_next(&mut present_id).push_next(&mut present_wait);
        let mut dynamic = vk::PhysicalDeviceVulkan13Features::default().dynamic_rendering(true);
        let info = crate::gfx::enable_present_feedback(
            vk::DeviceCreateInfo::default().push_next(&mut dynamic),
            &mut present_id, &mut present_wait,
        );
        let mut next = info.p_next.cast::<vk::BaseInStructure<'_>>();
        let mut types = Vec::new();
        // Bound traversal so a regression reports a failure instead of hanging.
        for _ in 0..4 {
            if next.is_null() { break; }
            let node = unsafe { &*next };
            assert!(!types.contains(&node.s_type), "repeated feature in device pNext chain");
            if node.s_type == vk::StructureType::PHYSICAL_DEVICE_PRESENT_ID_FEATURES_KHR {
                let feature = unsafe { &*next.cast::<vk::PhysicalDevicePresentIdFeaturesKHR<'_>>() };
                assert_eq!(feature.present_id, vk::TRUE);
            }
            if node.s_type == vk::StructureType::PHYSICAL_DEVICE_PRESENT_WAIT_FEATURES_KHR {
                let feature = unsafe { &*next.cast::<vk::PhysicalDevicePresentWaitFeaturesKHR<'_>>() };
                assert_eq!(feature.present_wait, vk::TRUE);
            }
            types.push(node.s_type);
            next = node.p_next;
        }
        assert!(next.is_null(), "device feature chain did not terminate");
        assert_eq!(types.len(), 3);
        assert!(types.contains(&vk::StructureType::PHYSICAL_DEVICE_VULKAN_1_3_FEATURES));
        assert!(types.contains(&vk::StructureType::PHYSICAL_DEVICE_PRESENT_ID_FEATURES_KHR));
        assert!(types.contains(&vk::StructureType::PHYSICAL_DEVICE_PRESENT_WAIT_FEATURES_KHR));
    }

    #[test]
    fn test_plane_spv_blobs() {
        // Offline modules from build.rs must load with valid SPIR-V magic.
        assert!(!spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/plane.vert.spv"))).is_empty());
        for samples in ["4", "8", "16", "32", "128"] {
            let path = format!("plane-{samples}.frag.spv");
            let full = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join(&path);
            let bytes = std::fs::read(&full).expect("plane frag spv missing");
            assert!(!spv_words(&bytes).is_empty(), "{path} empty");
        }
        assert!(!spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/sky.vert.spv"))).is_empty());
        assert!(!spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/sky.frag.spv"))).is_empty());
        assert!(!spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/cloud.vert.spv"))).is_empty());
        assert!(!spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/cloud.frag.spv"))).is_empty());
        assert!(!spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/ground.vert.spv"))).is_empty());
        assert!(!spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/vegetation.vert.spv"))).is_empty());
        assert!(!spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/canopy.vert.spv"))).is_empty());
        assert!(!spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/ground.frag.spv"))).is_empty());
        assert!(!spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/depth.frag.spv"))).is_empty());
    }

    #[test]
    fn test_fx_spv_blobs() {
        for name in [
            "plume.vert.spv",
            "plume.frag.spv",
            "trail.vert.spv",
            "trail.frag.spv",
            "composite.vert.spv",
            "composite.frag.spv",
        ] {
            let full = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join(name);
            let bytes = std::fs::read(&full).expect("fx spv missing");
            assert!(!spv_words(&bytes).is_empty(), "{name} empty");
        }
    }

    #[test]
    fn test_shared_exit_and_shortcuts() {
        let shared = Shared {
            exit: AtomicBool::new(false),
            keys: AtomicU32::new(0),
            ui: AtomicU32::new(hud::DEFAULT),
        };
        assert!(!shared.should_exit());
        shared.request_exit();
        assert!(shared.should_exit());
    }
}
