use std::io::Cursor;

use hound::WavReader;
use rust_decimal::Decimal;

const DEFAULT_MAX_BYTES: usize = 0x0400_0000;
const DEFAULT_MAX_FRAMES: u64 = 33_177_600_000;
const DEFAULT_MAX_SAMPLE_RATE: u32 = 384_000;

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

#[must_use]
pub fn decode_audio_duration(audio: &[u8], limits: &AudioDecodeLimits) -> Option<Decimal> {
    if audio.len() > limits.max_bytes {
        return None;
    }
    let reader = WavReader::new(Cursor::new(audio)).ok()?;
    let spec = reader.spec();
    if spec.sample_rate == 0 || spec.sample_rate > limits.max_sample_rate {
        return None;
    }
    let declared_frames = u64::from(reader.duration());
    let data_start = usize::try_from(reader.into_inner().position()).ok()?;
    let length_field = audio.get(data_start.checked_sub(4)?..data_start)?;
    let declared_bytes = u64::from(u32::from_le_bytes(length_field.try_into().ok()?));
    let data_bytes = u64::try_from(audio.len().saturating_sub(data_start)).ok()?;
    let frames = match declared_bytes.checked_div(declared_frames) {
        Some(frame_bytes) => declared_frames.min(data_bytes.checked_div(frame_bytes)?),
        None => 0,
    };
    if frames > limits.max_frames {
        return None;
    }
    Decimal::from(frames).checked_div(Decimal::from(spec.sample_rate))
}
