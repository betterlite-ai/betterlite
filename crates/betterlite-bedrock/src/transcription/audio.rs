use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_openai::api::audio_file_input::{
    AudioFile, AuthorizedAudioFiles, process_audio_file,
};
use serde::Serialize;

use super::error::TranscriptionError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioFormat {
    Wav,
    Mp3,
    Flac,
    Ogg,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AudioPayload {
    pub data: String,
    pub format: AudioFormat,
    pub filename: String,
}

pub fn audio_format(content_type: &str, filename: &str) -> Option<AudioFormat> {
    let suffix = filename
        .rsplit_once('.')
        .map_or_else(String::new, |(_, suffix)| suffix.to_lowercase());
    let name = match content_type {
        "audio/flac" => "flac",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/ogg" => "ogg",
        "audio/wav" | "audio/x-wav" => "wav",
        _ => suffix.as_str(),
    };
    match name {
        "wav" => Some(AudioFormat::Wav),
        "mp3" => Some(AudioFormat::Mp3),
        "flac" => Some(AudioFormat::Flac),
        "ogg" => Some(AudioFormat::Ogg),
        _ => None,
    }
}

pub fn audio_payload(
    file: AudioFile<'_>,
    files: &dyn AuthorizedAudioFiles,
) -> Result<AudioPayload, TranscriptionError> {
    let processed = process_audio_file(file, files)?;
    let Some(format) = audio_format(&processed.content_type, &processed.filename) else {
        return Err(TranscriptionError::UnsupportedFormat {
            filename: processed.filename,
        });
    };
    Ok(AudioPayload {
        data: STANDARD.encode(&processed.file_content),
        format,
        filename: processed.filename,
    })
}
