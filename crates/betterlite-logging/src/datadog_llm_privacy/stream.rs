use betterlite_models::decisions::Json;

use crate::fields::payload::StandardLoggingPayload;

// The payload's own stream field is set only for a completed streaming response; the request's flag rides in model_parameters
#[must_use]
pub fn is_streamed_request(payload: &StandardLoggingPayload) -> bool {
    if payload.stream == Some(true) {
        return true;
    }
    matches!(
        payload.model_parameters.get("stream"),
        Some(Json::Bool(true))
    )
}

mod tests;
