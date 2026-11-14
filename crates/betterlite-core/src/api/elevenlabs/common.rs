use indexmap::IndexMap;
use thiserror::Error;

// one provider exception class covers every ElevenLabs failure, and B1541 builds it for transcription
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("{message}")]
pub struct ElevenLabsError {
    pub status_code: u16,
    pub message: String,
    pub headers: IndexMap<String, String>,
}
