//! Offline SPIR-V loaders and CPU-baked textures for the sail cloth.
//! Compiled by build.rs (shaderc); large IBL counts are an offline
//! reference of the same sky/BRDF, not a separate cheaper model.



/// Sail cloth weave, same pattern as the web prototype: warm gray
/// base, fine grid, heavier lines every sixteen pixels. Returns all
/// mip levels with CPU box filtering so minification never aliases
/// into static. Each entry is (width, height, rgba bytes).
pub(super) fn weave_mips() -> Vec<(u32, u32, Vec<u8>)> {
    let base = [0xDAu8, 0xD6, 0xC7, 0xFF];
    let fine = [0xC3u8, 0xBF, 0xAF, 0xFF];
    let heavy = [0xAAu8, 0xA9, 0x9A, 0xFF];
    let mut level = vec![0u8; 64 * 64 * 4];
    for y in 0..64 {
        for x in 0..64 {
            let color = if x % 16 == 0 || y % 16 == 0 {
                heavy
            } else if x % 4 == 0 || y % 4 == 0 {
                fine
            } else {
                base
            };
            level[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4].copy_from_slice(&color);
        }
    }
    let mut out = vec![(64u32, 64u32, level)];
    while out.last().map(|(w, _, _)| *w).unwrap_or(1) > 1 {
        let (w, h, prev) = out.last().unwrap().clone();
        let (nw, nh) = (w / 2, h / 2);
        let mut next = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..4 {
                    let sum = prev[(((2 * y) * w + 2 * x) * 4 + c) as usize] as u32
                        + prev[(((2 * y) * w + 2 * x + 1) * 4 + c) as usize] as u32
                        + prev[(((2 * y + 1) * w + 2 * x) * 4 + c) as usize] as u32
                        + prev[(((2 * y + 1) * w + 2 * x + 1) * 4 + c) as usize] as u32;
                    next[((y * nw + x) * 4 + c) as usize] = (sum / 4) as u8;
                }
            }
        }
        out.push((nw, nh, next));
    }
    out
}

/// IBL quality variants are compiled offline by build.rs (shaderc) from
/// shaders/plane.frag with an injected ENV header. Large counts are an offline
/// visual reference for the same sky/BRDF, not a separate cheaper model.
pub(super) fn plane_frag_spv(samples: u32) -> Vec<u32> {
    match samples {
        4 => crate::spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/plane-4.frag.spv"))),
        8 => crate::spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/plane-8.frag.spv"))),
        16 => crate::spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/plane-16.frag.spv"))),
        32 => crate::spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/plane-32.frag.spv"))),
        128 => {
            crate::spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/plane-128.frag.spv")))
        }
        _ => panic!("EXPLORA_IBL_SAMPLES must be 4, 8, 16, 32, or 128"),
    }
}

/// Ray-query variants of the material fragment shaders (ENABLE_RT header).
pub(super) fn plane_frag_spv_rt(samples: u32) -> Vec<u32> {
    match samples {
        4 => crate::spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/plane-4-rt.frag.spv"))),
        8 => crate::spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/plane-8-rt.frag.spv"))),
        16 => crate::spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/plane-16-rt.frag.spv"))),
        32 => crate::spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/plane-32-rt.frag.spv"))),
        128 => {
            crate::spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/plane-128-rt.frag.spv")))
        }
        _ => panic!("EXPLORA_IBL_SAMPLES must be 4, 8, 16, 32, or 128"),
    }
}

pub(super) fn plane_task_spv() -> Vec<u32> {
    crate::spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/plane.task.spv")))
}

pub(super) fn plane_mesh_spv() -> Vec<u32> {
    crate::spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/plane.mesh.spv")))
}

pub(super) fn ground_frag_spv_rt() -> Vec<u32> {
    crate::spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/ground-rt.frag.spv")))
}

pub(super) fn ground_terrain_frag_spv() -> Vec<u32> {
    crate::spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/ground-terrain.frag.spv")))
}

pub(super) fn ground_terrain_frag_spv_performance() -> Vec<u32> {
    crate::spv_words(include_bytes!(concat!(
        env!("OUT_DIR"),
        "/ground-terrain-performance.frag.spv"
    )))
}

pub(super) fn ground_terrain_frag_spv_rt() -> Vec<u32> {
    crate::spv_words(include_bytes!(concat!(env!("OUT_DIR"), "/ground-terrain-rt.frag.spv")))
}

pub(super) fn ground_terrain_frag_spv_performance_rt() -> Vec<u32> {
    crate::spv_words(include_bytes!(concat!(
        env!("OUT_DIR"),
        "/ground-terrain-performance-rt.frag.spv"
    )))
}
