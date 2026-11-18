use crate::api::audio_file_input::decoder::AudioDecodeLimits;
use crate::api::file_type_records::{FileType, find_type, mime_from_type};

// LiteLLM's last resort when neither the provider nor the request names the media type
pub const DEFAULT_SPEECH_MEDIA_TYPE: &str = "audio/mpeg";

// AUDIO_FILE_TYPES: the extensions a response_format may name; ogg and m4a are excluded on purpose
const AUDIO_FILE_TYPES: [FileType; 8] = [
    FileType::Aac,
    FileType::Flac,
    FileType::Mp3,
    FileType::Mpa,
    FileType::Mpga,
    FileType::Opus,
    FileType::Pcm,
    FileType::Wav,
];

const OGG_OPUS_HEAD_WINDOW: usize = 64;
const OPUS_HEAD: &[u8] = b"OpusHead";
const ADTS_SYNC_AND_LAYER_MASK: u8 = 0xF6;
const ADTS_SYNC_AND_LAYER: u8 = 0xF0;
const ADTS_SAMPLE_RATE_INDEX_LIMIT: u8 = 13;
const MPEG_SYNC_MASK: u8 = 0xE0;
const MPEG_LAYER_MASK: u8 = 0x06;
const MPEG_RESERVED_VERSION: u8 = 0x01;
const MPEG_INVALID_BITRATE_INDEX: u8 = 0x0F;
const MPEG_RESERVED_RATE_INDEX: u8 = 0x03;

// the upstream Content-Type wins whenever it names audio, so the requested format only fills in for a generic one
#[must_use]
pub fn resolve_speech_media_type(
    upstream_content_type: Option<&str>,
    response_format: Option<&str>,
) -> String {
    let upstream = upstream_content_type.unwrap_or_default();
    let upstream_media_type = upstream
        .split_once(';')
        .map_or(upstream, |(head, _parameters)| head)
        .trim()
        .to_lowercase();
    if upstream_media_type.starts_with("audio/") {
        return upstream_media_type;
    }
    response_format
        .and_then(media_type_for_response_format)
        .unwrap_or(DEFAULT_SPEECH_MEDIA_TYPE)
        .to_owned()
}

fn media_type_for_response_format(response_format: &str) -> Option<&'static str> {
    let file_type = find_type(response_format)?;
    if !AUDIO_FILE_TYPES.contains(&file_type) {
        return None;
    }
    Some(mime_from_type(file_type))
}

// the header branches LiteLLM accepts, in its order; nothing here decodes audio
#[must_use]
pub fn sniff_speech_audio(audio: &[u8], limits: &AudioDecodeLimits) -> Option<&'static str> {
    if audio.len() > limits.max_bytes {
        return None;
    }
    if audio.get(..4) == Some(b"RIFF".as_slice()) && audio.get(8..12) == Some(b"WAVE".as_slice()) {
        return Some(mime_from_type(FileType::Wav));
    }
    if audio.get(..4) == Some(b"fLaC".as_slice()) {
        return Some(mime_from_type(FileType::Flac));
    }
    if audio.get(..4) == Some(b"OggS".as_slice()) {
        let file_type = if contains_opus_head(audio) {
            FileType::Opus
        } else {
            FileType::Ogg
        };
        return Some(mime_from_type(file_type));
    }
    if audio.get(..3) == Some(b"ID3".as_slice()) {
        return Some(mime_from_type(FileType::Mp3));
    }
    frame_media_type(audio)
}

// OpusHead has to end inside the first 64 bytes, so only windows starting at 0..=56 count
fn contains_opus_head(audio: &[u8]) -> bool {
    let opus_start_limit = OGG_OPUS_HEAD_WINDOW
        .saturating_sub(OPUS_HEAD.len())
        .saturating_add(1);
    audio
        .windows(OPUS_HEAD.len())
        .take(opus_start_limit)
        .any(|window| window == OPUS_HEAD)
}

// the ADTS and MPEG guards, which read exactly the first three bytes
fn frame_media_type(audio: &[u8]) -> Option<&'static str> {
    let first = audio.first().copied()?;
    let second = audio.get(1).copied()?;
    let third = audio.get(2).copied()?;
    if first != 0xFF {
        return None;
    }
    if (second & ADTS_SYNC_AND_LAYER_MASK) == ADTS_SYNC_AND_LAYER {
        let sample_rate_index = (third >> 2) & 0x0F;
        if sample_rate_index < ADTS_SAMPLE_RATE_INDEX_LIMIT {
            return Some(mime_from_type(FileType::Aac));
        }
        return None;
    }
    mpeg_audio_frame_media_type(second, third)
}

fn mpeg_audio_frame_media_type(second: u8, third: u8) -> Option<&'static str> {
    let version = (second >> 3) & 0x03;
    let layer = second & MPEG_LAYER_MASK;
    let bitrate_index = third >> 4;
    let sample_rate_index = (third >> 2) & 0x03;
    if (second & MPEG_SYNC_MASK) != MPEG_SYNC_MASK
        || version == MPEG_RESERVED_VERSION
        || layer == 0
        || bitrate_index == MPEG_INVALID_BITRATE_INDEX
        || sample_rate_index == MPEG_RESERVED_RATE_INDEX
    {
        return None;
    }
    Some(mime_from_type(FileType::Mp3))
}
