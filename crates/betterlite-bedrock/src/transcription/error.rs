use betterlite_openai::api::audio_file_input::AudioFileError;

use crate::signing::error::SigningError;

#[derive(Debug, thiserror::Error)]
pub enum TranscriptionError {
    #[error(transparent)]
    AudioFile(#[from] AudioFileError),
    #[error("Unsupported Bedrock audio format for file '{filename}'")]
    UnsupportedFormat { filename: String },
    #[error("expected {expected}, got {actual}")]
    InvalidType {
        expected: &'static str,
        actual: &'static str,
    },
    #[error("missing field {0}")]
    MissingField(&'static str),
    #[error("{0}")]
    InvalidRequest(&'static str),
    #[error("invalid Converse response: {0}")]
    InvalidResponse(#[source] serde_json::Error),
    #[error("Bedrock response contains non-text transcript content")]
    NonTextContent,
    #[error("the Converse request does not encode")]
    Encode(#[source] serde_json::Error),
    #[error(transparent)]
    Signing(#[from] SigningError),
}
