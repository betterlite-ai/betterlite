use std::iter::repeat_n;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use super::frames::{AudioInput, Event, InputFrame};
use crate::realtime_session::records::Setting;

pub const TRIGGER_AUDIO_SAMPLE_RATE_HERTZ: i64 = 16000;
const LEADING_SILENCE_BYTES: usize = 16_000;
const TRAILING_SILENCE_BYTES: usize = 96_000;
const CHUNK_SIZE: usize = 1024;
// The pinned ready_trigger_pcm() result, stored decompressed so no gzip decoder is needed
const READY_TRIGGER_PCM: &[u8] = include_bytes!("ready_trigger.pcm");

#[must_use]
pub fn trigger_audio_frames(prompt_name: &str, content_name: &str) -> Vec<InputFrame> {
    let mut pcm = vec![0_u8; LEADING_SILENCE_BYTES];
    pcm.extend_from_slice(READY_TRIGGER_PCM);
    pcm.extend(repeat_n(0_u8, TRAILING_SILENCE_BYTES));
    pcm.chunks(CHUNK_SIZE)
        .map(|chunk| {
            InputFrame::from(Event::AudioInput(AudioInput {
                prompt_name: prompt_name.to_owned(),
                content_name: content_name.to_owned(),
                content: Setting(Some(STANDARD.encode(chunk))),
            }))
        })
        .collect()
}
