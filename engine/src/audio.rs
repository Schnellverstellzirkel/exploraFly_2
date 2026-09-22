//! Native PCM output on its own thread. The render thread only publishes atomics.
use sim::audio::{AcousticState, FlightSynth, SAMPLE_RATE};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, Ordering},
};

struct Control {
    running: AtomicBool,
    airspeed: AtomicU32,
    mach: AtomicU32,
    altitude: AtomicU32,
    spool: AtomicU32,
    boost: AtomicU32,
    load: AtomicU32,
    aoa: AtomicU32,
    sideslip: AtomicU32,
    pitch_rate: AtomicU32,
    roll_rate: AtomicU32,
    vertical_speed: AtomicU32,
    separation: AtomicU32,
    volume: AtomicU32,
}

fn store(slot: &AtomicU32, value: f32) {
    slot.store(value.to_bits(), Ordering::Relaxed);
}

fn load(slot: &AtomicU32) -> f32 {
    f32::from_bits(slot.load(Ordering::Relaxed))
}

pub struct Audio {
    control: Arc<Control>,
    worker: Option<std::thread::JoinHandle<()>>,
    master: f32,
}

impl Audio {
    pub fn start() -> Self {
        let master = crate::flags::volume();
        let d = AcousticState::default();
        let control = Arc::new(Control {
            running: AtomicBool::new(true),
            airspeed: AtomicU32::new(d.airspeed.to_bits()),
            mach: AtomicU32::new(d.mach.to_bits()),
            altitude: AtomicU32::new(d.altitude.to_bits()),
            spool: AtomicU32::new(d.spool.to_bits()),
            boost: AtomicU32::new(d.boost.to_bits()),
            load: AtomicU32::new(d.load.to_bits()),
            aoa: AtomicU32::new(d.aoa.to_bits()),
            sideslip: AtomicU32::new(d.sideslip.to_bits()),
            pitch_rate: AtomicU32::new(d.pitch_rate.to_bits()),
            roll_rate: AtomicU32::new(d.roll_rate.to_bits()),
            vertical_speed: AtomicU32::new(d.vertical_speed.to_bits()),
            separation: AtomicU32::new(d.separation.to_bits()),
            volume: AtomicU32::new(0.0f32.to_bits()),
        });
        let worker = if !crate::flags::audio_requested() {
            None
        } else {
            let state = control.clone();
            std::thread::Builder::new()
                .name("explora-audio".into())
                .spawn(move || {
                    if let Err(error) = run_pcm(&state) {
                        eprintln!("audio disabled: {error}");
                    }
                })
                .ok()
        };
        Self {
            control,
            worker,
            master,
        }
    }

    pub fn update(&self, acoustics: AcousticState, audible: bool) {
        let c = &self.control;
        store(&c.airspeed, acoustics.airspeed);
        store(&c.mach, acoustics.mach);
        store(&c.altitude, acoustics.altitude);
        store(&c.spool, acoustics.spool);
        store(&c.boost, acoustics.boost);
        store(&c.load, acoustics.load);
        store(&c.aoa, acoustics.aoa);
        store(&c.sideslip, acoustics.sideslip);
        store(&c.pitch_rate, acoustics.pitch_rate);
        store(&c.roll_rate, acoustics.roll_rate);
        store(&c.vertical_speed, acoustics.vertical_speed);
        store(&c.separation, acoustics.separation);
        store(&c.volume, if audible { self.master } else { 0.0 });
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        self.control.running.store(false, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn read_state(control: &Control) -> AcousticState {
    AcousticState {
        airspeed: load(&control.airspeed),
        mach: load(&control.mach),
        altitude: load(&control.altitude),
        spool: load(&control.spool),
        boost: load(&control.boost),
        load: load(&control.load),
        aoa: load(&control.aoa),
        sideslip: load(&control.sideslip),
        pitch_rate: load(&control.pitch_rate),
        roll_rate: load(&control.roll_rate),
        vertical_speed: load(&control.vertical_speed),
        separation: load(&control.separation),
        volume: load(&control.volume),
    }
}

#[cfg(target_os = "linux")]
fn run_pcm(control: &Control) -> Result<(), String> {
    use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void};
    type Open = unsafe extern "C" fn(*mut *mut c_void, *const c_char, c_int, c_int) -> c_int;
    type Configure = unsafe extern "C" fn(
        *mut c_void,
        c_int,
        c_int,
        c_uint,
        c_uint,
        c_int,
        c_uint,
    ) -> c_int;
    type Write = unsafe extern "C" fn(*mut c_void, *const c_void, c_ulong) -> c_long;
    type Operation = unsafe extern "C" fn(*mut c_void) -> c_int;
    // See ALSA PCM interface: S16_LE=2, RW_INTERLEAVED=3, PLAYBACK=0.
    unsafe {
        let lib = libloading::Library::new("libasound.so.2").map_err(|e| e.to_string())?;
        let open = *lib
            .get::<Open>(b"snd_pcm_open\0")
            .map_err(|e| e.to_string())?;
        let configure = *lib
            .get::<Configure>(b"snd_pcm_set_params\0")
            .map_err(|e| e.to_string())?;
        let write = *lib
            .get::<Write>(b"snd_pcm_writei\0")
            .map_err(|e| e.to_string())?;
        let prepare = *lib
            .get::<Operation>(b"snd_pcm_prepare\0")
            .map_err(|e| e.to_string())?;
        let resume = *lib
            .get::<Operation>(b"snd_pcm_resume\0")
            .map_err(|e| e.to_string())?;
        let close = *lib
            .get::<Operation>(b"snd_pcm_close\0")
            .map_err(|e| e.to_string())?;
        let drop_pcm = *lib
            .get::<Operation>(b"snd_pcm_drop\0")
            .map_err(|e| e.to_string())?;
        let mut pcm = std::ptr::null_mut();
        let device = crate::flags::audio_device();
        let device = std::ffi::CString::new(device).map_err(|e| e.to_string())?;
        let status = open(&mut pcm, device.as_ptr(), 0, 1); // nonblocking
        if status < 0 {
            return Err(format!("PCM device unavailable ({status})"));
        }
        let status = configure(pcm, 2, 3, 2, SAMPLE_RATE, 1, 40_000);
        if status < 0 {
            close(pcm);
            return Err(format!("PCM configuration failed ({status})"));
        }
        let mut synth = FlightSynth::default();
        let mut samples = [0i16; 960]; // fixed ten-millisecond stereo buffer
        let mut failed = None;
        while control.running.load(Ordering::Relaxed) {
            synth.render(&mut samples, read_state(control));
            let mut sent = 0usize;
            while sent < 480 && control.running.load(Ordering::Relaxed) {
                let count = write(
                    pcm,
                    samples.as_ptr().add(sent * 2).cast(),
                    (480 - sent) as c_ulong,
                );
                if count > 0 {
                    sent += count as usize;
                } else if count == -(libc::EAGAIN as c_long) || count == 0 {
                    std::thread::sleep(std::time::Duration::from_millis(2));
                } else if count == -(libc::EPIPE as c_long) && prepare(pcm) >= 0 {
                    // Underrun: restart, retaining unwritten samples and oscillator phase.
                } else if count == -(libc::EINTR as c_long) {
                    // Interrupted system call: retry without discarding the buffer.
                } else if count == -(libc::ESTRPIPE as c_long) {
                    // Unlike snd_pcm_recover's blocking resume loop, one attempt
                    // at a time keeps suspension/shutdown responsive.
                    let status = resume(pcm);
                    if status == -libc::EAGAIN {
                        std::thread::sleep(std::time::Duration::from_millis(2));
                    } else if status < 0 && prepare(pcm) < 0 {
                        failed = Some(format!("PCM resume failed ({status})"));
                        break;
                    }
                } else {
                    failed = Some(format!("PCM write failed ({count})"));
                    break;
                }
            }
            if failed.is_some() {
                break;
            }
        }
        drop_pcm(pcm);
        close(pcm);
        if let Some(error) = failed {
            Err(error)
        } else {
            Ok(())
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn run_pcm(_: &Control) -> Result<(), String> {
    Err("native playback currently targets Linux; use the audio_preview example for WAV output".into())
}
