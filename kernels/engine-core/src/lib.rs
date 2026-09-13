// Webxplora engine core in Rust.
// One game. One canvas. One machine.
// CPU path is for Ryzen 7 7840HS.
// GPU constants are for RTX 4060 in Chrome.
// No detection. No fallback.
//
// Shape: batches only. The frame loop lives on one side
// in JS next to the GL calls. Rust never runs per frame
// per object. One call carries all bodies. Tile work
// already runs batched in the terrain workers.

pub const TERRAIN_WORKERS: u32 = 6;
pub const FOREST_WORKERS: u32 = 4;
pub const SURFACE_SAMPLES: u32 = 4;
pub const RESOLUTION_SCALE: f32 = 1.0;
pub const MAX_PIXEL_RATIO: f32 = 2.0;

#[no_mangle]
pub extern "C" fn engine_alloc(bytes: usize) -> *mut u8 {
    let mut buf = Vec::with_capacity((bytes + 15) & !15);
    let ptr = buf.as_mut_ptr();
    core::mem::forget(buf);
    ptr
}

// Body stride is 8 floats: x, y, z, vx, vy, vz, pad, pad.
// One call steps every body. Used for crash fragments and
// impact motes. The plane itself steps in JS with the sim.
#[no_mangle]
pub extern "C" fn bodies_step(
    ptr: *mut f32,
    count: usize,
    dt: f32,
    gravity: f32,
    drag: f32,
    ground_y: f32,
) {
    if ptr.is_null() || count == 0 || dt <= 0.0 {
        return;
    }
    unsafe {
        let keep = (1.0 - drag * dt).max(0.0);
        for i in 0..count {
            let b = ptr.add(i * 8);
            let mut vx = *b.add(3);
            let mut vy = *b.add(4);
            let mut vz = *b.add(5);
            vy -= gravity * dt;
            vx *= keep;
            vy *= keep;
            vz *= keep;
            let mut x = *b + vx * dt;
            let mut y = *b.add(1) + vy * dt;
            let mut z = *b.add(2) + vz * dt;
            if y < ground_y {
                y = ground_y;
                vx = 0.0;
                vy = 0.0;
                vz = 0.0;
                x = *b;
                z = *b.add(2);
            }
            *b = x;
            *b.add(1) = y;
            *b.add(2) = z;
            *b.add(3) = vx;
            *b.add(4) = vy;
            *b.add(5) = vz;
        }
    }
}
