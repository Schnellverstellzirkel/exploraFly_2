//! Finite, bounded telemetry passed to the atlas-free composite HUD.
pub const VISIBLE: u32 = 1;
pub const HELP: u32 = 2;
pub const AUDIO: u32 = 4;
pub const PAUSED: u32 = 8;
pub const RESET: u32 = 16;
pub const DEFAULT: u32 = VISIBLE | HELP | AUDIO;

pub fn pack(speed: f32, altitude: f32, heading: f32, climb: f32, boost: f32, clearance: f32, flags: u32) -> [f32; 8] {
    let finite = |v: f32, low: f32, high: f32| if v.is_finite() { v.clamp(low, high) } else { 0.0 };
    let heading = if heading.is_finite() { heading.to_degrees().rem_euclid(360.0) } else { 0.0 };
    [finite(speed * 1.9438445, 0.0, 9999.0), finite(altitude, 0.0, 99999.0),
        heading, finite(climb, -999.0, 999.0), if flags & VISIBLE != 0 { 1.0 } else { 0.0 },
        finite(boost, 0.0, 1.0), (flags & 15) as f32, finite(clearance, 0.0, 99999.0)]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn telemetry_converts_units_and_wraps_heading() {
        let p = pack(100.0, 1234.0, -std::f32::consts::FRAC_PI_2, -12.0, 0.75, 45.0, DEFAULT);
        assert!((p[0] - 194.38445).abs() < 0.001);
        assert_eq!(&p[1..], &[1234.0, 270.0, -12.0, 1.0, 0.75, 7.0, 45.0]);
    }
    #[test]
    fn supersonic_speed_is_not_clipped_to_three_digits() {
        assert!((pack(900.0, 1000.0, 0.0, 0.0, 1.0, 1000.0, DEFAULT)[0] - 1749.46).abs() < 0.01);
    }
    #[test]
    fn invalid_telemetry_never_reaches_shader_or_overflows_digits() {
        let p = pack(f32::INFINITY, f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 3.0, -12.0, 0);
        assert!(p.iter().all(|v| v.is_finite()));
        assert_eq!(p, [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    }
}
