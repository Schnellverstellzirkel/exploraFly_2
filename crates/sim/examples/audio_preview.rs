//! Scripted multi-regime sound-design preview without audio hardware.
//! Sequence: idle, boost run, high-Mach pass, hard-G turn, stall buffet,
//! sideslip imbalance, mute fade.
use sim::audio::{AcousticState, FlightSynth, SAMPLE_RATE};
use std::io::Write;

fn state_at(t: f32) -> AcousticState {
    let mut s = AcousticState {
        airspeed: 70.0,
        mach: 0.2,
        altitude: 1500.0,
        spool: 0.15,
        boost: 0.0,
        load: 1.0,
        aoa: 0.02,
        sideslip: 0.0,
        pitch_rate: 0.0,
        roll_rate: 0.0,
        vertical_speed: 0.0,
        separation: 0.0,
        volume: 0.45,
    };
    // 0..4 idle spool up.
    if (0.0..4.0).contains(&t) {
        s.spool = 0.15 + 0.2 * (t / 4.0);
        s.airspeed = 70.0 + 20.0 * (t / 4.0);
        s.mach = s.airspeed / 340.0;
    }
    // 4..8 boost run.
    if (4.0..8.0).contains(&t) {
        let k = (t - 4.0) / 4.0;
        s.spool = 0.35 + 0.65 * k;
        s.boost = k;
        s.airspeed = 90.0 + 400.0 * k;
        s.mach = s.airspeed / 340.0;
        s.vertical_speed = 20.0 * k;
    }
    // 8..12 high-Mach pass.
    if (8.0..12.0).contains(&t) {
        let k = (t - 8.0) / 4.0;
        s.spool = 1.0;
        s.boost = 1.0 - 0.3 * k;
        s.airspeed = 490.0 + 200.0 * k;
        s.mach = s.airspeed / 330.0;
        s.altitude = 4000.0 + 500.0 * k;
    }
    // 12..16 hard-G turn.
    if (12.0..16.0).contains(&t) {
        let k = ((t - 12.0) / 4.0 * std::f32::consts::PI).sin().abs();
        s.airspeed = 400.0;
        s.mach = 1.2;
        s.spool = 0.9;
        s.boost = 0.4;
        s.load = 1.0 + 4.5 * k;
        s.pitch_rate = 1.0 * k;
        s.roll_rate = 1.5 * k;
        s.aoa = 0.05 + 0.2 * k;
    }
    // 16..20 stall / separation buffet.
    if (16.0..20.0).contains(&t) {
        let k = ((t - 16.0) / 4.0 * std::f32::consts::PI).sin().abs();
        s.airspeed = 90.0;
        s.mach = 0.27;
        s.spool = 0.5;
        s.boost = 0.0;
        s.load = 0.6 + 0.4 * k;
        s.aoa = 0.35 + 0.25 * k;
        s.separation = 0.4 + 0.6 * k;
        s.pitch_rate = 0.4 * k;
        s.vertical_speed = -15.0 * k;
    }
    // 20..24 sideslip.
    if (20.0..24.0).contains(&t) {
        let k = ((t - 20.0) / 4.0 * std::f32::consts::PI).sin();
        s.airspeed = 220.0;
        s.mach = 0.65;
        s.spool = 0.7;
        s.sideslip = 0.35 * k;
        s.roll_rate = 0.8 * k;
        s.load = 1.0 + 0.5 * k.abs();
    }
    // 24..27 mute fade.
    if t >= 24.0 {
        s.volume = 0.45 * ((27.0 - t) / 3.0).clamp(0.0, 1.0);
    }
    s
}

fn main() -> std::io::Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "flight-preview.wav".into());
    let frames = SAMPLE_RATE * 27;
    let data_bytes = frames * 2 * 2;
    let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
    file.write_all(b"RIFF")?;
    file.write_all(&(36 + data_bytes).to_le_bytes())?;
    file.write_all(b"WAVEfmt ")?;
    file.write_all(&16u32.to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?;
    file.write_all(&2u16.to_le_bytes())?;
    file.write_all(&SAMPLE_RATE.to_le_bytes())?;
    file.write_all(&(SAMPLE_RATE * 4).to_le_bytes())?;
    file.write_all(&4u16.to_le_bytes())?;
    file.write_all(&16u16.to_le_bytes())?;
    file.write_all(b"data")?;
    file.write_all(&data_bytes.to_le_bytes())?;
    let mut synth = FlightSynth::default();
    let mut block = [0i16; 960];
    let ticks = (frames / 480) as usize;
    for tick in 0..ticks {
        let t = tick as f32 * 0.01;
        synth.render(&mut block, state_at(t));
        for sample in block {
            file.write_all(&sample.to_le_bytes())?;
        }
    }
    file.flush()
}
