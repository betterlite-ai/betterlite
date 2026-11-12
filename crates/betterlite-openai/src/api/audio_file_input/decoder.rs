use std::io::Cursor;

use hound::WavReader;
use rust_decimal::Decimal;

const DEFAULT_MAX_BYTES: usize = 0x0400_0000;
const DEFAULT_MAX_FRAMES: u64 = 33_177_600_000;
const DEFAULT_MAX_SAMPLE_RATE: u32 = 384_000;
const RIFF_HEADER_LEN: usize = 12;
const CHUNK_ID_LEN: usize = 4;
const CHUNK_LENGTH_LEN: usize = 4;
const BLOCK_ALIGN_OFFSET: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioDecodeLimits {
    pub max_bytes: usize,
    pub max_frames: u64,
    pub max_sample_rate: u32,
}

impl Default for AudioDecodeLimits {
    fn default() -> Self {
        Self {
            max_bytes: DEFAULT_MAX_BYTES,
            max_frames: DEFAULT_MAX_FRAMES,
            max_sample_rate: DEFAULT_MAX_SAMPLE_RATE,
        }
    }
}

pub trait AudioDurationDecoder {
    fn duration(&self, audio: &[u8]) -> Option<Decimal>;
}

pub struct NativeAudioDecoder {
    pub limits: AudioDecodeLimits,
}

impl AudioDurationDecoder for NativeAudioDecoder {
    fn duration(&self, audio: &[u8]) -> Option<Decimal> {
        decode_audio_duration(audio, &self.limits)
    }
}

#[derive(Clone, Copy)]
pub enum AudioDecoder<'decoder> {
    Unavailable,
    Available(&'decoder dyn AudioDurationDecoder),
}

fn fit_data_chunk(audio: &[u8]) -> Option<Vec<u8>> {
    let mut block_align = None;
    let mut chunk = RIFF_HEADER_LEN;
    loop {
        let length_start = chunk.checked_add(CHUNK_ID_LEN)?;
        let body = length_start.checked_add(CHUNK_LENGTH_LEN)?;
        let id = audio.get(chunk..length_start)?;
        let length_field = audio.get(length_start..body)?;
        let declared = usize::try_from(u32::from_le_bytes(length_field.try_into().ok()?)).ok()?;
        if id == b"fmt " {
            let field = audio
                .get(body.checked_add(BLOCK_ALIGN_OFFSET)?..)?
                .get(..2)?;
            block_align = Some(usize::from(u16::from_le_bytes(field.try_into().ok()?)));
        }
        if id == b"data" {
            let present = audio.len().saturating_sub(body);
            let clamped = declared.min(present);
            let fitted = clamped.checked_sub(clamped.checked_rem(block_align?)?)?;
            let mut header = audio.get(..body)?.to_vec();
            header
                .get_mut(length_start..)?
                .copy_from_slice(&u32::try_from(fitted).ok()?.to_le_bytes());
            return Some(header);
        }
        chunk = body.checked_add(declared)?;
    }
}

#[must_use]
pub fn decode_audio_duration(audio: &[u8], limits: &AudioDecodeLimits) -> Option<Decimal> {
    if audio.len() > limits.max_bytes {
        return None;
    }
    let header = fit_data_chunk(audio)?;
    let reader = WavReader::new(Cursor::new(header)).ok()?;
    let spec = reader.spec();
    if spec.sample_rate == 0 || spec.sample_rate > limits.max_sample_rate {
        return None;
    }
    let frames = u64::from(reader.duration());
    if frames > limits.max_frames {
        return None;
    }
    Decimal::from(frames).checked_div(Decimal::from(spec.sample_rate))
}
