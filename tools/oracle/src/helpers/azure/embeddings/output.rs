use betterlite_azure::embeddings::client::ParseError;
use betterlite_azure::embeddings::error::EmbeddingError;
use betterlite_azure::embeddings::reply::EmbeddingReply;
use serde_json::{Value, json};

const DECODE_PREFIX: &str = "Failed to parse raw Azure embedding response:";
const DECODE_MARKER: &str = "<json decode failure>";

// the response headers and the hidden additional_headers hold the same keys, as LiteLLM's get_response_headers keeps them
pub(super) fn reply(outcome: &EmbeddingReply) -> Value {
    json!({
        "model": outcome.model,
        "object": outcome.object,
        "data": outcome.data,
        "usage": {
            "prompt_tokens": outcome.usage.prompt_tokens,
            "completion_tokens": outcome.usage.completion_tokens,
            "total_tokens": outcome.usage.total_tokens,
        },
        "response_headers": outcome.response_headers,
        "additional_headers": outcome.response_headers,
        "hidden_headers": outcome.hidden_headers,
    })
}

// a JSON decode failure keeps only its stable prefix, since Python and serde word the position differently
pub(super) fn raised(error: &EmbeddingError) -> Value {
    let plain = |kind: &str, status: Option<u16>, message: Option<&str>| json!({"kind": kind, "status": status, "message": message, "headers": null});
    match error {
        EmbeddingError::Azure(failure) => {
            let message = if !matches!(failure.cause, Some(ParseError::Json(_))) {
                failure.message.as_str()
            } else if failure.message.starts_with(DECODE_PREFIX) {
                DECODE_PREFIX
            } else {
                DECODE_MARKER
            };
            json!({
                "kind": "azure",
                "status": failure.status_code,
                "message": message,
                "headers": failure.headers,
            })
        }
        EmbeddingError::Value { message } => plain("value", None, Some(message)),
        EmbeddingError::Response {
            status_code,
            message,
        } => plain("response", Some(*status_code), Some(message)),
        EmbeddingError::Sdk(failure) => {
            let kind = if failure.status_code.is_some() {
                "sdk_status"
            } else {
                "sdk_other"
            };
            plain(kind, failure.status_code, None)
        }
        EmbeddingError::Parse(_)
        | EmbeddingError::Endpoint(_)
        | EmbeddingError::Logging(_)
        | EmbeddingError::Encode(_) => plain("other", None, Some(&error.to_string())),
    }
}
