//! Native PCM output on its own thread. The render thread only publishes atomics.
use sim::audio::{FlightSynth, SoundState, SAMPLE_RATE};
use std::sync::{Arc, atomic::{AtomicBool, AtomicU32, Ordering}};

struct Control {
    running: AtomicBool,
    speed: AtomicU32,
    spool: AtomicU32,
    load: AtomicU32,
    volume: AtomicU32,
}

pub struct Audio {
    control: Arc<Control>,
    worker: Option<std::thread::JoinHandle<()>>,
    master: f32,
}

impl Audio {
    pub fn start() -> Self {
        let master = crate::flags::volume();
        let control = Arc::new(Control {
            running: AtomicBool::new(true),
            speed: AtomicU32::new(70.0f32.to_bits()),
            spool: AtomicU32::new(0.15f32.to_bits()),
            load: AtomicU32::new(1.0f32.to_bits()),
            volume: AtomicU32::new(0.0f32.to_bits()),
        });
        let worker = if !crate::flags::audio_requested() {
            None
        } else {
            let state = control.clone();
            std::thread::Builder::new().name("explora-audio".into())
                .spawn(move || {
                    if let Err(error) = run_pcm(&state) { eprintln!("audio disabled: {error}"); }
                }).ok()
        };
        Self { control, worker, master }
    }

    pub fn update(&self, speed: f32, spool: f32, load: f32, audible: bool) {
        self.control.speed.store(speed.to_bits(), Ordering::Relaxed);
        self.control.spool.store(spool.to_bits(), Ordering::Relaxed);
        self.control.load.store(load.to_bits(), Ordering::Relaxed);
        self.control.volume.store(if audible { self.master } else { 0.0 }.to_bits(), Ordering::Relaxed);
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        self.control.running.store(false, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() { let _ = worker.join(); }
    }
}

#[cfg(target_os = "linux")]
fn run_pcm(control: &Control) -> Result<(), String> {
    use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void};
    type Open = unsafe extern "C" fn(*mut *mut c_void, *const c_char, c_int, c_int) -> c_int;
    type Configure = unsafe extern "C" fn(*mut c_void, c_int, c_int, c_uint, c_uint, c_int, c_uint) -> c_int;
    type Write = unsafe extern "C" fn(*mut c_void, *const c_void, c_ulong) -> c_long;
    type Operation = unsafe extern "C" fn(*mut c_void) -> c_int;
    // See ALSA PCM interface: S16_LE=2, RW_INTERLEAVED=3, PLAYBACK=0.
    unsafe {
        let lib = libloading::Library::new("libasound.so.2").map_err(|e| e.to_string())?;
        let open = *lib.get::<Open>(b"snd_pcm_open\0").map_err(|e| e.to_string())?;
        let configure = *lib.get::<Configure>(b"snd_pcm_set_params\0").map_err(|e| e.to_string())?;
        let write = *lib.get::<Write>(b"snd_pcm_writei\0").map_err(|e| e.to_string())?;
        let prepare = *lib.get::<Operation>(b"snd_pcm_prepare\0").map_err(|e| e.to_string())?;
        let resume = *lib.get::<Operation>(b"snd_pcm_resume\0").map_err(|e| e.to_string())?;
        let close = *lib.get::<Operation>(b"snd_pcm_close\0").map_err(|e| e.to_string())?;
        let drop_pcm = *lib.get::<Operation>(b"snd_pcm_drop\0").map_err(|e| e.to_string())?;
        let mut pcm = std::ptr::null_mut();
        let device = crate::flags::audio_device();
        let device = std::ffi::CString::new(device).map_err(|e| e.to_string())?;
        let status = open(&mut pcm, device.as_ptr(), 0, 1); // nonblocking
        if status < 0 { return Err(format!("PCM device unavailable ({status})")); }
        let status = configure(pcm, 2, 3, 2, SAMPLE_RATE, 1, 40_000);
        if status < 0 { close(pcm); return Err(format!("PCM configuration failed ({status})")); }
        let mut synth = FlightSynth::default();
        let mut samples = [0i16; 960]; // fixed ten-millisecond stereo buffer
        let mut failed = None;
        while control.running.load(Ordering::Relaxed) {
            synth.render(&mut samples, SoundState {
                airspeed: f32::from_bits(control.speed.load(Ordering::Relaxed)),
                spool: f32::from_bits(control.spool.load(Ordering::Relaxed)),
                load: f32::from_bits(control.load.load(Ordering::Relaxed)),
                volume: f32::from_bits(control.volume.load(Ordering::Relaxed)),
            });
            let mut sent = 0usize;
            while sent < 480 && control.running.load(Ordering::Relaxed) {
                let count = write(pcm, samples.as_ptr().add(sent * 2).cast(), (480 - sent) as c_ulong);
                if count > 0 { sent += count as usize; }
                else if count == -(libc::EAGAIN as c_long) || count == 0 {
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
            if failed.is_some() { break; }
        }
        drop_pcm(pcm);
        close(pcm);
        if let Some(error) = failed { Err(error) } else { Ok(()) }
    }
}

#[cfg(not(target_os = "linux"))]
fn run_pcm(_: &Control) -> Result<(), String> {
    Err("native playback currently targets Linux; use the audio_preview example for WAV output".into())
}
