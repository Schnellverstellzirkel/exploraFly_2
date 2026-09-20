//! Disk-backed graphics pipeline cache. The driver compiles every graphics
//! pipeline from its SPIR-V on first creation; a warm cache file lets cold
//! boots reuse that compiled state instead of paying for it again. The file
//! is keyed by driver version and an FNV-1a recipe hash of every SPIR-V
//! module the creation passes consume, and the driver still validates each
//! internal entry, so any mismatch degrades to a normal (slower) build
//! rather than a wrong pipeline.

use std::path::PathBuf;
use ash::vk;

const MAGIC: [u8; 8] = *b"EXPLPC01";

fn cache_path(driver_version: u32) -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    Some(base.join("explora").join(format!("pipelines-{:08x}.bin", driver_version)))
}

/// FNV-1a over a SPIR-V module; folds into a running recipe hash.
pub(super) fn hash_module(seed: u64, words: &[u32]) -> u64 {
    let mut h = seed;
    for w in words {
        for b in w.to_le_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}

pub(super) unsafe fn load(
    device: &ash::Device,
    driver_version: u32,
    recipe: u64,
) -> vk::PipelineCache {
    let mut initial: Vec<u8> = Vec::new();
    if let Some(path) = cache_path(driver_version) {
        if let Ok(bytes) = std::fs::read(&path) {
            if bytes.len() > 20
                && bytes[0..8] == MAGIC
                && u32::from_le_bytes(bytes[8..12].try_into().unwrap()) == driver_version
                && u64::from_le_bytes(bytes[12..20].try_into().unwrap()) == recipe
            {
                initial = bytes[20..].to_vec();
            }
        }
    }
    let info = vk::PipelineCacheCreateInfo::default().initial_data(&initial);
    device.create_pipeline_cache(&info, None).expect("pcache")
}

pub(super) unsafe fn store(
    device: &ash::Device,
    cache: vk::PipelineCache,
    driver_version: u32,
    recipe: u64,
) {
    let Ok(data) = device.get_pipeline_cache_data(cache) else {
        return;
    };
    if data.is_empty() {
        return;
    }
    let Some(path) = cache_path(driver_version) else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("tmp");
    let mut out = Vec::with_capacity(20 + data.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&driver_version.to_le_bytes());
    out.extend_from_slice(&recipe.to_le_bytes());
    out.extend_from_slice(&data);
    if std::fs::write(&tmp, &out).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

pub(super) unsafe fn destroy(device: &ash::Device, cache: vk::PipelineCache) {
    device.destroy_pipeline_cache(cache, None);
}
