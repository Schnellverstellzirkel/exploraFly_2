// Vendor and OS layer. Real libraries, fixed target.
// NVIDIA through NVML (libnvidia-ml from the 580 driver).
// AMD through the kernel DRM nodes and hwmon sysfs.
// Ryzen through scheduler affinity. Everything optional
// except the NVIDIA path: missing pieces log and continue,
// except a missing NVIDIA GPU which refuses to boot.

pub struct GpuStats {
    pub temp_c: u32,
    pub clock_mhz: u32,
}

type NvmlInit = unsafe extern "C" fn() -> i32;
type NvmlHandleByIndex = unsafe extern "C" fn(u32, *mut *mut std::ffi::c_void) -> i32;
type NvmlTemp = unsafe extern "C" fn(*mut std::ffi::c_void, u32, *mut u32) -> i32;
type NvmlClock = unsafe extern "C" fn(*mut std::ffi::c_void, u32, *mut u32) -> i32;

const NVML_SUCCESS: i32 = 0;
const NVML_TEMPERATURE_GPU: u32 = 0;
const NVML_CLOCK_SM: u32 = 1;

struct Nvml {
    _lib: libloading::Library,
    handle: *mut std::ffi::c_void,
    temp: NvmlTemp,
    clock: NvmlClock,
}

// NVML is thread-safe. The handle lives for the whole run.
unsafe impl Send for Nvml {}

impl Nvml {
    unsafe fn open() -> Option<Self> {
        let lib = libloading::Library::new("libnvidia-ml.so.1").ok()?;
        let init: libloading::Symbol<NvmlInit> = lib.get(b"nvmlInit_v2").ok()?;
        if init() != NVML_SUCCESS {
            return None;
        }
        let by_index: libloading::Symbol<NvmlHandleByIndex> =
            lib.get(b"nvmlDeviceGetHandleByIndex_v2").ok()?;
        let mut handle: *mut std::ffi::c_void = std::ptr::null_mut();
        if by_index(0, &mut handle) != NVML_SUCCESS {
            return None;
        }
        let temp: NvmlTemp = *lib.get::<NvmlTemp>(b"nvmlDeviceGetTemperature").ok()?;
        let clock: NvmlClock = *lib.get::<NvmlClock>(b"nvmlDeviceGetClockInfo").ok()?;
        Some(Self { _lib: lib, handle, temp, clock })
    }

    fn sample(&self) -> GpuStats {
        let mut temp = 0u32;
        let mut clock = 0u32;
        unsafe {
            if (self.temp)(self.handle, NVML_TEMPERATURE_GPU, &mut temp) != NVML_SUCCESS {
                temp = 0;
            }
            if (self.clock)(self.handle, NVML_CLOCK_SM, &mut clock) != NVML_SUCCESS {
                clock = 0;
            }
        }
        GpuStats { temp_c: temp, clock_mhz: clock }
    }
}

pub struct AmdNode {
    pub name: String,
    pub temp_c: u32,
    pub sclk_mhz: u32,
}

fn read_first_u32(path: &std::path::Path) -> Option<u32> {
    let text = std::fs::read_to_string(path).ok()?;
    text.split_whitespace().next()?.parse().ok()
}

fn amdgpu_nodes() -> Vec<AmdNode> {
    let mut out = Vec::new();
    let drm = std::path::Path::new("/sys/class/drm");
    let entries = std::fs::read_dir(drm).map(|r| r.collect::<Vec<_>>()).unwrap_or_default();
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("card") || name.contains('-') {
            continue;
        }
        let driver = entry.path().join("device/driver");
        let target = std::fs::read_link(&driver).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        if !target.ends_with("amdgpu") {
            continue;
        }
        let device = entry.path().join("device");
        let mut temp_c = 0u32;
        if let Ok(hwmon) = std::fs::read_dir(device.join("hwmon")) {
            for monitor in hwmon.flatten() {
                if let Some(t) = read_first_u32(&monitor.path().join("temp1_input")) {
                    temp_c = t / 1000;
                    break;
                }
            }
        }
        let mut sclk_mhz = 0u32;
        if let Ok(clocks) = std::fs::read_to_string(device.join("pp_dpm_sclk")) {
            for line in clocks.lines() {
                if line.contains('*') {
                    let digits: String = line.chars().filter(|c| c.is_ascii_digit()).collect();
                    // Format is "0: 400Mhz *". Digits run together, so split at the Mhz mark.
                    if let Some(pos) = line.find("Mhz") {
                        let head: String =
                            line[..pos].chars().filter(|c| c.is_ascii_digit()).collect();
                        // Head looks like "0400": index, then megahertz.
                        if head.len() > 1 {
                            sclk_mhz = head[1..].parse().unwrap_or(0);
                        }
                    }
                    let _ = digits;
                    break;
                }
            }
        }
        out.push(AmdNode { name, temp_c, sclk_mhz });
    }
    out
}

pub struct Vendor {
    nvml: Option<Nvml>,
    amd: Vec<AmdNode>,
}

pub fn pin_to_performance_cores() {
    unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        libc::CPU_ZERO(&mut set);
        // Leave CPU 0 to the system. Take the rest.
        let count = libc::sysconf(libc::_SC_NPROCESSORS_ONLN).max(2) as usize;
        for cpu in 1..count.min(libc::CPU_SETSIZE as usize) {
            libc::CPU_SET(cpu, &mut set);
        }
        if libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set) == 0 {
            println!("affinity pinned to {} cores", count - 1);
        } else {
            println!("affinity left alone");
        }
    }
}

impl Vendor {
    pub unsafe fn open() -> Self {
        let nvml = Nvml::open();
        match &nvml {
            Some(_) => println!("NVML bound to libnvidia-ml"),
            None => println!("NVML unavailable"),
        }
        let amd = amdgpu_nodes();
        for node in &amd {
            println!("AMD node {}: {}C {}MHz", node.name, node.temp_c, node.sclk_mhz);
        }
        if amd.is_empty() {
            println!("no AMD render nodes in use");
        }
        Self { nvml, amd }
    }

    pub fn sample(&mut self) -> GpuStats {
        // Refresh AMD clocks once a second alongside the NVIDIA read.
        self.amd = amdgpu_nodes();
        match &self.nvml {
            Some(nvml) => nvml.sample(),
            None => GpuStats { temp_c: 0, clock_mhz: 0 },
        }
    }
}
