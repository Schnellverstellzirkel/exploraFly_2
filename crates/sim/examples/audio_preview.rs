//! Render a deterministic glide/boost/bank sound-design preview without audio hardware.
use sim::audio::{FlightSynth, SoundState, SAMPLE_RATE};
use std::io::Write;

fn main() -> std::io::Result<()> {
    let path = std::env::args().nth(1).unwrap_or_else(|| "flight-preview.wav".into());
    let frames = SAMPLE_RATE * 10;
    let data_bytes = frames * 2 * 2;
    let mut file = std::fs::File::create(path)?;
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
    for tick in 0..1000 {
        let t = tick as f32 * 0.01;
        let boost = ((t - 2.0) / 3.0).clamp(0.0, 1.0) * ((9.0 - t) / 2.0).clamp(0.0, 1.0);
        synth.render(&mut block, SoundState {
            airspeed: 70.0 + 500.0 * boost,
            spool: 0.15 + 0.85 * boost,
            load: if (6.0..8.0).contains(&t) { 5.0 } else { 1.0 },
            volume: 0.45 * ((10.0 - t) * 2.0).clamp(0.0, 1.0),
        });
        for sample in block { file.write_all(&sample.to_le_bytes())?; }
    }
    Ok(())
}
