use std::time::TryFromFloatSecsError;

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::api::audio_file_input::AudioFileError;

use crate::credentials::error::CredentialError;
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
    #[error("invalid Converse response: invalid type for `text`, expected a string")]
    InvalidTextType,
    #[error("Bedrock response contains non-text transcript content")]
    NonTextContent,
    #[error("the Converse request does not encode")]
    Encode(#[source] serde_json::Error),
    #[error(transparent)]
    Signing(#[from] SigningError),
    #[error("the AWS credential lookup failed")]
    Credentials(#[source] CredentialError),
}

#[derive(Debug, thiserror::Error)]
pub enum CallError {
    #[error(transparent)]
    Transcription(#[from] TranscriptionError),
    #[error("the extra header {name} must be a string, got {actual}")]
    HeaderType { name: String, actual: &'static str },
    #[error("the {name} header cannot be sent")]
    InvalidHeader { name: String },
    #[error("the signed endpoint is not a URL")]
    Endpoint(#[source] url::ParseError),
    #[error("the timeout of {seconds} seconds is not a duration")]
    Timeout {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error("the transcription request could not be sent")]
    Transport(#[source] TransportError),
    #[error("Bedrock answered {status}: {body}")]
    Status { status: u16, body: String },
    #[error("the native binding sends over an async transport, so only the async call runs")]
    Blocking,
}
