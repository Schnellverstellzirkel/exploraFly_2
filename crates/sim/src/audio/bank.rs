//! Embedded, generated aircraft PCM stems.
//!
//! The WAV assets are built offline by tools/audio/generate_stems.py. This
//! module decodes them once before the audio render loop and retains their
//! backing storage for the process lifetime. It performs no I/O or allocation
//! in render().

use super::samples::{PcmLoop, SoundBank, StemId};
use super::SAMPLE_RATE;
use std::sync::OnceLock;

static BUILTIN_BANK: OnceLock<SoundBank> = OnceLock::new();

static ENGINE_LOW: &[u8] = include_bytes!("../../assets/audio/engine_low.wav");
static ENGINE_MID: &[u8] = include_bytes!("../../assets/audio/engine_mid.wav");
static ENGINE_HIGH: &[u8] = include_bytes!("../../assets/audio/engine_high.wav");
static BOOST: &[u8] = include_bytes!("../../assets/audio/boost.wav");
static AIRFRAME_BUFFET: &[u8] = include_bytes!("../../assets/audio/airframe_buffet.wav");
static STRUCTURAL_RATTLE: &[u8] = include_bytes!("../../assets/audio/structural_rattle.wav");

/// Return the shared built-in sample bank. Decoding and allocation happen once.
pub(crate) fn builtin_sound_bank() -> SoundBank {
    *BUILTIN_BANK.get_or_init(|| {
        let mut bank = SoundBank::EMPTY;
        bank.set(StemId::EngineLow, Some(decode(ENGINE_LOW, "engine_low")));
        bank.set(StemId::EngineMid, Some(decode(ENGINE_MID, "engine_mid")));
        bank.set(StemId::EngineHigh, Some(decode(ENGINE_HIGH, "engine_high")));
        bank.set(StemId::Boost, Some(decode(BOOST, "boost")));
        bank.set(
            StemId::AirframeBuffet,
            Some(decode(AIRFRAME_BUFFET, "airframe_buffet")),
        );
        bank.set(
            StemId::StructuralRattle,
            Some(decode(STRUCTURAL_RATTLE, "structural_rattle")),
        );
        bank
    })
}

fn decode(wav: &'static [u8], name: &str) -> PcmLoop {
    parse_pcm_wav(wav).unwrap_or_else(|| panic!("invalid embedded audio stem: {name}"))
}

/// Decode a mono 48 kHz signed 16-bit PCM RIFF/WAVE into retained static data.
fn parse_pcm_wav(wav: &'static [u8]) -> Option<PcmLoop> {
    if wav.get(0..4)? != b"RIFF" || wav.get(8..12)? != b"WAVE" {
        return None;
    }

    let mut format = None;
    let mut data = None;
    let mut offset = 12usize;
    while offset.checked_add(8)? <= wav.len() {
        let chunk_id = wav.get(offset..offset + 4)?;
        let chunk_len =
            u32::from_le_bytes(wav.get(offset + 4..offset + 8)?.try_into().ok()?) as usize;
        let start = offset.checked_add(8)?;
        let end = start.checked_add(chunk_len)?;
        let chunk = wav.get(start..end)?;

        match chunk_id {
            b"fmt " if chunk.len() >= 16 => {
                let audio_format = u16::from_le_bytes(chunk[0..2].try_into().ok()?);
                let channels = u16::from_le_bytes(chunk[2..4].try_into().ok()?);
                let sample_rate = u32::from_le_bytes(chunk[4..8].try_into().ok()?);
                let block_align = u16::from_le_bytes(chunk[12..14].try_into().ok()?);
                let bits_per_sample = u16::from_le_bytes(chunk[14..16].try_into().ok()?);
                if audio_format != 1
                    || channels != 1
                    || sample_rate != SAMPLE_RATE
                    || block_align != 2
                    || bits_per_sample != 16
                {
                    return None;
                }
                format = Some(());
            }
            b"data" => data = Some(chunk),
            _ => {}
        }

        offset = end.checked_add(chunk_len & 1)?;
    }

    format?;
    let bytes = data?;
    if bytes.is_empty() || !bytes.len().is_multiple_of(2) {
        return None;
    }
    let samples = bytes
        .chunks_exact(2)
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
        .collect::<Vec<_>>()
        .into_boxed_slice();
    let samples: &'static [i16] = Box::leak(samples);
    PcmLoop::new(samples, 0, samples.len(), 1.0)
}
