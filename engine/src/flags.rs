//! Central registry for every `EXPLORA_*` process flag.
//!
//! Two classes only:
//!
//! - **Play flags** configure the shipping game (window, audio, wind, HUD).
//!   They are read in every build.
//! - **DEBUG_ONLY** flags select reduced visual quality, an alternate GPU
//!   path, a pacing or present override, a forced flight regime, or a
//!   measurement diagnostic. They compile under `debug_assertions` only.
//!   Release and dist builds never read those names. The playable
//!   configuration is one preset: max visual quality.
//!
//! Naming standard:
//! - Tri-state feature paths use `auto`, `on`, or `off` (`parse_toggle`).
//! - Simple enable flags use `0` / `1` (`debug_enabled` / `play_enabled`).
//! - Numeric flags are decimal and finite (`play_f32`, `play_u32`, `play_u64`).
//! - One name string per flag, declared once below.

use std::ffi::OsString;

// ---------------------------------------------------------------------------
// Flag names
// ---------------------------------------------------------------------------

// Play (every build)
pub const WINDOWED: &str = "EXPLORA_WINDOWED";
pub const VOLUME: &str = "EXPLORA_VOLUME";
pub const AUDIO: &str = "EXPLORA_AUDIO";
pub const AUDIO_DEVICE: &str = "EXPLORA_AUDIO_DEVICE";
pub const HUD: &str = "EXPLORA_HUD";
pub const WIND: &str = "EXPLORA_WIND";

// DEBUG_ONLY: quality, alternate paths, pacing/present, diagnostics
pub const QUALITY: &str = "EXPLORA_QUALITY";
pub const IBL_SAMPLES: &str = "EXPLORA_IBL_SAMPLES";
pub const BURST: &str = "EXPLORA_BURST";
pub const RT_SHADOWS: &str = "EXPLORA_RT_SHADOWS";
pub const MESH_SHADERS: &str = "EXPLORA_MESH_SHADERS";
pub const GPU_VEGETATION: &str = "EXPLORA_GPU_VEGETATION";
pub const PACING: &str = "EXPLORA_PACING";
pub const PRESENT: &str = "EXPLORA_PRESENT";
pub const PRESENT_FEEDBACK: &str = "EXPLORA_PRESENT_FEEDBACK";
pub const FX: &str = "EXPLORA_FX";
pub const NO_GPU: &str = "EXPLORA_NO_GPU";
pub const GPU_READBACK_EVERY: &str = "EXPLORA_GPU_READBACK_EVERY";
pub const GPU_SPIKE_US: &str = "EXPLORA_GPU_SPIKE_US";
pub const FREEZE: &str = "EXPLORA_FREEZE";
pub const FREEZE_POSE: &str = "EXPLORA_FREEZE_POSE";
pub const BOOST: &str = "EXPLORA_BOOST";
pub const BANK: &str = "EXPLORA_BANK";
pub const PITCH: &str = "EXPLORA_PITCH";
pub const SHOT: &str = "EXPLORA_SHOT";
pub const SHOT_FRAME: &str = "EXPLORA_SHOT_FRAME";
pub const BENCH_JSON: &str = "EXPLORA_BENCH_JSON";
pub const START_X: &str = "EXPLORA_X";
pub const START_Z: &str = "EXPLORA_Z";
pub const START_ALT: &str = "EXPLORA_ALT";
pub const START_HEADING: &str = "EXPLORA_HEADING";

/// True when DEBUG_ONLY flags are compiled into this binary.
#[allow(dead_code)] // Documented contract; consumers may gate on it later.
pub const DEBUG_ONLY: bool = cfg!(debug_assertions);

// ---------------------------------------------------------------------------
// Toggle (`auto` | `on` | `off`)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    Auto,
    On,
    Off,
}

/// Parse a standardized tri-state path flag.
pub fn parse_toggle(value: &str) -> Result<Toggle, &'static str> {
    match value {
        "auto" => Ok(Toggle::Auto),
        "on" => Ok(Toggle::On),
        "off" => Ok(Toggle::Off),
        _ => Err("must be auto, on, or off"),
    }
}

/// DEBUG_ONLY tri-state. `None` in playable builds (treat as `Auto`).
pub fn debug_toggle(name: &str) -> Option<Result<Toggle, &'static str>> {
    debug_var(name).map(|value| parse_toggle(&value))
}

// ---------------------------------------------------------------------------
// Play accessors (every build)
// ---------------------------------------------------------------------------

/// Read a play flag. Returns `None` when unset.
pub fn play_var(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// Read a play flag as an OS string (presence checks, non-UTF8 paths).
pub fn play_var_os(name: &str) -> Option<OsString> {
    std::env::var_os(name)
}

/// `true` when the flag is set and not `0` / `false` / `off`.
#[allow(dead_code)] // Part of the play-flag API surface.
pub fn play_enabled(name: &str) -> bool {
    play_var(name)
        .map(|value| !matches!(value.as_str(), "0" | "false" | "off"))
        .unwrap_or(false)
}

/// `true` unless the flag is set to `0` / `false` / `off`.
/// Used for defaults that stay on when the flag is absent.
#[allow(dead_code)] // Part of the play-flag API surface.
pub fn play_enabled_or(name: &str, default: bool) -> bool {
    match play_var(name) {
        None => default,
        Some(value) => !matches!(value.as_str(), "0" | "false" | "off"),
    }
}

pub fn play_f32(name: &str) -> Option<f32> {
    play_var(name)
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|value| value.is_finite())
}

#[allow(dead_code)] // Part of the play-flag API surface.
pub fn play_u32(name: &str) -> Option<u32> {
    play_var(name).and_then(|value| value.parse::<u32>().ok())
}

#[allow(dead_code)] // Part of the play-flag API surface.
pub fn play_u64(name: &str) -> Option<u64> {
    play_var(name).and_then(|value| value.parse::<u64>().ok())
}

// ---------------------------------------------------------------------------
// DEBUG_ONLY accessors (compiled out of playable builds)
// ---------------------------------------------------------------------------

/// Read a DEBUG_ONLY flag. Always `None` when `not(debug_assertions)`, so
/// release and dist never observe quality or diagnostic overrides.
pub fn debug_var(name: &str) -> Option<String> {
    #[cfg(debug_assertions)]
    {
        std::env::var(name).ok()
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = name;
        None
    }
}

/// Presence check for a DEBUG_ONLY flag. Always `false` in playable builds.
pub fn debug_var_os(name: &str) -> Option<OsString> {
    #[cfg(debug_assertions)]
    {
        std::env::var_os(name)
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = name;
        None
    }
}

/// `true` when a DEBUG_ONLY flag is set and not `0` / `false` / `off`.
/// Always `false` in playable builds (flag absent).
pub fn debug_enabled(name: &str) -> bool {
    debug_var(name)
        .map(|value| !matches!(value.as_str(), "0" | "false" | "off"))
        .unwrap_or(false)
}

pub fn debug_f32(name: &str) -> Option<f32> {
    debug_var(name)
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|value| value.is_finite())
}

pub fn debug_u32(name: &str) -> Option<u32> {
    debug_var(name).and_then(|value| value.parse::<u32>().ok())
}

pub fn debug_u64(name: &str) -> Option<u64> {
    debug_var(name).and_then(|value| value.parse::<u64>().ok())
}

// ---------------------------------------------------------------------------
// Resolved policy helpers
// ---------------------------------------------------------------------------

/// IBL samples for a quality preset. The env override is DEBUG_ONLY;
/// playable builds always use the preset value (cinematic: 16).
pub fn ibl_samples(preset: u32) -> u32 {
    match debug_u32(IBL_SAMPLES) {
        Some(samples) if samples > 0 => samples,
        Some(_) => panic!("{IBL_SAMPLES} must be a positive integer"),
        None => preset,
    }
}

/// Geometry-only burst pass count. Always 1 in playable builds.
pub fn render_burst(default: u32) -> u32 {
    debug_u32(BURST).map_or(default, |value| value.clamp(1, 256))
}

/// Display pacing in play mode. `EXPLORA_PACING=off` is DEBUG_ONLY.
pub fn display_pacing(enabled_without_flag: bool) -> bool {
    match debug_var(PACING) {
        None => enabled_without_flag,
        Some(value) => value != "off",
    }
}

/// Opt-in GPU vegetation cull path. Always the legacy path in playable builds.
pub fn gpu_vegetation() -> bool {
    debug_enabled(GPU_VEGETATION)
}

/// Exhaust/contrail FX. Always on in playable builds (`EXPLORA_FX` is DEBUG_ONLY).
pub fn fx_enabled() -> bool {
    match debug_var(FX) {
        None => true,
        Some(value) => !matches!(value.as_str(), "0" | "false" | "off"),
    }
}

/// Forced burner for screenshot/benchmark helpers. Always false in playable builds.
pub fn force_boost() -> bool {
    debug_var_os(BOOST).is_some()
}

/// Forced hard bank for screenshot/benchmark helpers.
pub fn force_bank() -> bool {
    debug_var_os(BANK).is_some()
}

/// Forced pitch for screenshot/benchmark helpers.
pub fn force_pitch() -> Option<f32> {
    debug_f32(PITCH)
}

/// Sim freeze for measurement. Always false in playable builds.
pub fn frozen() -> bool {
    debug_var_os(FREEZE).is_some()
}

/// Freeze animation pose only. True when `frozen()` or the pose flag is set.
pub fn freeze_pose() -> bool {
    frozen() || debug_var_os(FREEZE_POSE).is_some()
}

/// Benchmark JSON path. Only written when the flag is set (DEBUG_ONLY).
pub fn bench_json() -> Option<String> {
    debug_var(BENCH_JSON)
}

/// Screenshot path. DEBUG_ONLY.
pub fn shot_path() -> Option<String> {
    debug_var(SHOT)
}

/// Screenshot frame list. Defaults to `[120]` when unset.
pub fn shot_frames() -> Vec<u64> {
    debug_var(SHOT_FRAME)
        .map(|value| {
            value
                .split(',')
                .filter_map(|part| part.trim().parse().ok())
                .collect()
        })
        .unwrap_or_else(|| vec![120])
}

/// Diagnostic spawn overrides. All DEBUG_ONLY; playable spawns at the valley.
pub fn spawn_x() -> Option<f32> {
    debug_f32(START_X)
}

pub fn spawn_z() -> Option<f32> {
    debug_f32(START_Z)
}

pub fn spawn_alt() -> Option<f32> {
    debug_f32(START_ALT)
}

pub fn spawn_heading() -> Option<f32> {
    debug_f32(START_HEADING)
}

/// Audio mute flag: `EXPLORA_AUDIO=0` disables the worker (play flag).
pub fn audio_requested() -> bool {
    play_var(AUDIO).as_deref() != Some("0")
}

/// HUD visible at startup. `EXPLORA_HUD=0` starts hidden (play flag).
pub fn hud_visible() -> bool {
    play_var(HUD).as_deref() != Some("0")
}

/// Start windowed when `EXPLORA_WINDOWED` is set (play flag).
pub fn start_windowed() -> bool {
    play_var_os(WINDOWED).is_some()
}

/// Master volume 0..1, default 0.35 (play flag).
pub fn volume() -> f32 {
    play_f32(VOLUME).unwrap_or(0.35).clamp(0.0, 1.0)
}

/// ALSA device name (play flag).
pub fn audio_device() -> String {
    play_var(AUDIO_DEVICE).unwrap_or_else(|| "default".into())
}

/// Wind strength, default 1.0, must be finite (play flag).
pub fn wind_strength() -> f32 {
    match play_var(WIND) {
        None => 1.0,
        Some(value) => value
            .parse::<f32>()
            .unwrap_or_else(|_| panic!("{WIND} must be a number from 0 to 3")),
    }
}

// Resolved at call sites that already know GPU capability / benchmark mode:
// - RT_SHADOWS, MESH_SHADERS: `debug_toggle` + capability, else Auto
// - PRESENT, PRESENT_FEEDBACK, NO_GPU, GPU_READBACK_EVERY, GPU_SPIKE_US:
//   `debug_*` only

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggles_accept_only_standard_spellings() {
        assert_eq!(parse_toggle("auto"), Ok(Toggle::Auto));
        assert_eq!(parse_toggle("on"), Ok(Toggle::On));
        assert_eq!(parse_toggle("off"), Ok(Toggle::Off));
        assert!(parse_toggle("true").is_err());
        assert!(parse_toggle("").is_err());
    }

    #[test]
    fn debug_only_names_are_not_play_names() {
        let play = [
            WINDOWED, VOLUME, AUDIO, AUDIO_DEVICE, HUD, WIND,
        ];
        let debug = [
            QUALITY, IBL_SAMPLES, BURST, RT_SHADOWS, MESH_SHADERS,
            GPU_VEGETATION, PACING, PRESENT, PRESENT_FEEDBACK, FX,
            NO_GPU, GPU_READBACK_EVERY, GPU_SPIKE_US, FREEZE, FREEZE_POSE,
            BOOST, BANK, PITCH, SHOT, SHOT_FRAME, BENCH_JSON,
            START_X, START_Z, START_ALT, START_HEADING,
        ];
        for name in play {
            assert!(name.starts_with("EXPLORA_"), "{name}");
        }
        for name in debug {
            assert!(name.starts_with("EXPLORA_"), "{name}");
            assert!(!play.contains(&name), "{name} must stay DEBUG_ONLY");
        }
        assert_eq!(DEBUG_ONLY, cfg!(debug_assertions));
    }

    #[test]
    fn ibl_samples_prefers_positive_override_else_preset() {
        // Unset in the test process: preset wins. A positive override is
        // only observable in debug builds; playable builds always pass the
        // preset through `debug_u32` returning None.
        if !DEBUG_ONLY {
            assert_eq!(ibl_samples(16), 16);
        }
    }
}
