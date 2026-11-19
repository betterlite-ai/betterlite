use std::error::Error;

use betterlite_openai::api::speech_chat_bridge::audio_base64::SpeechAudioError;
use thiserror::Error;

// the status and error fields the reply carries for the Gemini response-format rejection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeechErrorFields {
    pub status: u16,
    pub error_type: Option<&'static str>,
    pub param: Option<&'static str>,
    pub code: Option<&'static str>,
}

// everything the transformation raises before the completion call or while reading its answer
#[derive(Debug, Error)]
pub enum SpeechTransformError {
    #[error(
        "Gemini TTS only produces raw PCM16 audio, so response_format='{response_format}' is not supported. Supported response formats: pcm, wav."
    )]
    UnsupportedResponseFormat { response_format: String },
    #[error("Unmapped response type. Got type: <class '{type_name}'>")]
    Unmapped { type_name: String },
    #[error("No audio part found in the response")]
    MissingAudio,
    #[error("list index out of range")]
    NoChoices,
    #[error(transparent)]
    Audio(#[from] SpeechAudioError),
    #[error("the response audio part cannot be read: {source}")]
    AudioPart {
        #[source]
        source: serde_json::Error,
    },
    #[error("the PCM16 audio is too large to wrap in a WAV container")]
    WavTooLarge,
}

impl SpeechTransformError {
    // the rejection litellm raises as a BadRequestError, whose type, param and code are null
    #[must_use]
    pub const fn error_fields(&self) -> Option<SpeechErrorFields> {
        if !matches!(self, Self::UnsupportedResponseFormat { .. }) {
            return None;
        }
        Some(SpeechErrorFields {
            status: 400,
            error_type: None,
            param: None,
            code: None,
        })
    }
}

// a missing argument fails validation exactly like LiteLLM's isinstance check, before any provider traffic
#[derive(Debug, Error)]
pub enum SpeechBridgeError<Source>
where
    Source: Error + 'static,
{
    #[error("{name} is required")]
    Required { name: &'static str },
    #[error(transparent)]
    Transform(#[from] SpeechTransformError),
    #[error(transparent)]
    Completion { source: Source },
}
