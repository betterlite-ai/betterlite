use indexmap::IndexMap;

use crate::api::elevenlabs::common::ElevenLabsError;

// LiteLLM's get_error_class wraps a failed transcription call in the shared ElevenLabs exception unchanged
#[must_use]
pub fn error_class(
    error_message: &str,
    status_code: u16,
    headers: IndexMap<String, String>,
) -> ElevenLabsError {
    ElevenLabsError {
        status_code,
        message: error_message.to_owned(),
        headers,
    }
}
