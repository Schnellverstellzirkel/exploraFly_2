//! Zero-allocation check for the audio render path.
//! Lives in its own integration-test binary so no parallel unit test
//! can pollute the allocation counter while the window is open.

use sim::audio::{AcousticState, FlightSynth, BLOCK_SAMPLES};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct CountingAlloc;

static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static MEASURE: AtomicBool = AtomicBool::new(false);

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if MEASURE.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if MEASURE.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

#[test]
fn render_allocates_nothing_in_the_audio_path() {
    let mut synth = FlightSynth::default();
    let mut block = [0i16; BLOCK_SAMPLES];
    let state = AcousticState {
        airspeed: 310.0,
        mach: 0.9,
        altitude: 2000.0,
        spool: 0.8,
        boost: 0.2,
        load: 3.0,
        aoa: 0.12,
        sideslip: 0.05,
        pitch_rate: 0.2,
        roll_rate: 0.1,
        vertical_speed: 5.0,
        separation: 0.1,
        volume: 0.4,
    };
    synth.render(&mut block, state);
    MEASURE.store(true, Ordering::SeqCst);
    for _ in 0..50 {
        synth.render(&mut block, state);
    }
    MEASURE.store(false, Ordering::SeqCst);
    let count = ALLOCS.load(Ordering::SeqCst);
    assert_eq!(count, 0, "render must not allocate, saw {count}");
}
