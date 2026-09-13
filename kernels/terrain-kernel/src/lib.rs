mod forsyth;
mod geology;
mod glacier;
mod hydro;
mod noise;
mod surface;
mod walk;

use surface::terrain_sample_full;

/// Sample geological bedrock elevation at world coordinates `(x, z)` for the specified procedural seed.
#[no_mangle]
pub extern "C" fn bedrock_height(x: f64, z: f64, seed: u32) -> f64 {
    geology::bedrock_height(x, z, seed)
}

/// Allocate a raw 4-byte aligned buffer for WebAssembly foreign function calls.
#[no_mangle]
pub extern "C" fn alloc(bytes: usize) -> *mut u8 {
    let mut buf = Vec::with_capacity((bytes + 3) & !3);
    let ptr = buf.as_mut_ptr();
    core::mem::forget(buf);
    ptr
}

/// Batch-sample geological bedrock heights for `n` 2D coordinate pairs `[x0, z0, x1, z1, ...]`.
#[no_mangle]
pub extern "C" fn bedrock_height_batch(coords: *const f64, n: usize, seed: u32, out: *mut f64) {
    if n == 0 {
        return;
    }
    unsafe {
        for i in 0..n {
            let x = *coords.add(i * 2);
            let z = *coords.add(i * 2 + 1);
            *out.add(i) = bedrock_height(x, z, seed);
        }
    }
}

/// Sample full 9-parameter terrain state at `(x, z)` into the caller's output buffer.
#[no_mangle]
pub extern "C" fn terrain_sample(x: f64, z: f64, seed: u32, out: *mut f64) {
    let mut values = [0.0f64; 9];
    terrain_sample_full(x, z, seed, &mut values);
    unsafe {
        std::ptr::copy_nonoverlapping(values.as_ptr(), out, 9);
    }
}

static mut SAMPLE_OUT: [f64; 9] = [0.0; 9];

/// Sample full terrain state at `(x, z)` using internal thread-static scratch storage.
#[no_mangle]
pub extern "C" fn terrain_sample_cached(x: f64, z: f64, seed: u32) -> *mut f64 {
    unsafe {
        terrain_sample_full(x, z, seed, &mut *std::ptr::addr_of_mut!(SAMPLE_OUT));
        (*std::ptr::addr_of_mut!(SAMPLE_OUT)).as_mut_ptr()
    }
}

/// Batch-sample full 9-parameter terrain state for `n` coordinate pairs.
#[no_mangle]
pub extern "C" fn terrain_sample_batch(coords: *const f64, n: usize, seed: u32, out: *mut f64) {
    if n == 0 {
        return;
    }
    unsafe {
        for i in 0..n {
            let x = *coords.add(i * 2);
            let z = *coords.add(i * 2 + 1);
            let mut values = [0.0f64; 9];
            terrain_sample_full(x, z, seed, &mut values);
            std::ptr::copy_nonoverlapping(values.as_ptr(), out.add(i * 9), 9);
        }
    }
}

std::thread_local! {
    static SCRATCH: std::cell::RefCell<Vec<u8>> = std::cell::RefCell::new(Vec::new());
}

/// Obtain a thread-local scratch buffer resized to hold at least `nbytes`.
#[no_mangle]
pub extern "C" fn scratch_ptr(nbytes: usize) -> *mut u8 {
    SCRATCH.with(|s| {
        let mut s = s.borrow_mut();
        if s.len() < nbytes {
            s.resize(nbytes.max(1 << 20), 0);
        }
        s.as_mut_ptr()
    })
}

/// Build distant terrain mesh tile for chunk `(cx, cz)`.
#[no_mangle]
pub extern "C" fn far_tile_build(cx: i32, cz: i32, seed: u32) {
    walk::far_tile_build(cx, cz, seed)
}

/// Copy distant terrain tile memory layout offsets into caller buffer.
#[no_mangle]
pub extern "C" fn far_tile_layout(out: *mut usize) {
    let mut layout = [0usize; 24];
    walk::far_tile_layout(&mut layout);
    unsafe {
        std::ptr::copy_nonoverlapping(layout.as_ptr(), out, 24);
    }
}

/// Copy distant terrain generation profiling statistics into caller buffer.
#[no_mangle]
pub extern "C" fn far_tile_stats(out: *mut usize) {
    let mut stats = [0usize; 9];
    walk::far_tile_stats(&mut stats);
    unsafe {
        std::ptr::copy_nonoverlapping(stats.as_ptr(), out, 9);
    }
}
