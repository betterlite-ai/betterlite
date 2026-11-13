use betterlite_models::decisions::Json;
use serde::Deserialize;

use crate::error::{ProviderError, ProviderErrorParts};

const BAD_REQUEST_STATUS: u16 = 400;
const LOADING_STATUS: u16 = 503;
const UNKNOWN_STATUS: u16 = 500;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RuntimeError {
    pub code: String,
    #[serde(default)]
    pub message: String,
}

#[derive(Deserialize)]
struct RuntimeReply {
    #[serde(default)]
    error: Option<RuntimeError>,
    #[serde(default)]
    answers: Option<Json>,
}

#[must_use]
pub fn runtime_error(payload: &[u8]) -> Option<RuntimeError> {
    let reply: RuntimeReply = serde_json::from_slice(payload).ok()?;
    if reply.answers.is_some() {
        return None;
    }
    reply.error
}

#[must_use]
pub fn runtime_error_status(code: &str) -> u16 {
    match code {
        "bad_request" | "invalid_request" => BAD_REQUEST_STATUS,
        "loading" => LOADING_STATUS,
        _ => UNKNOWN_STATUS,
    }
}

#[must_use]
pub fn runtime_failure(error: &RuntimeError) -> ProviderError {
    ProviderError::from(ProviderErrorParts {
        status_code: runtime_error_status(&error.code),
        message: format!(
            "Strands Decider runtime error '{}': {}",
            error.code, error.message
        ),
        headers: None,
        request: None,
        response: None,
        body: None,
        status_code_is_synthesized: false,
    })
}
