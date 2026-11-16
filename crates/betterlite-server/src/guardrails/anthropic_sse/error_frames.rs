use serde::Serialize;

use super::error::AnthropicSseError;

mod tests;

#[derive(Serialize)]
struct ErrorEvent<'text> {
    #[serde(rename = "type")]
    kind: &'static str,
    error: ErrorDetail<'text>,
}

#[derive(Serialize)]
struct ErrorDetail<'text> {
    #[serde(rename = "type")]
    kind: &'static str,
    message: &'text str,
}

// A failure found after the headers went out can only reach the client as a frame, since a raise cannot.
pub fn anthropic_sse_error_frames(message: &str) -> Result<Vec<Vec<u8>>, AnthropicSseError> {
    let event = ErrorEvent {
        kind: "error",
        error: ErrorDetail {
            kind: "guardrail_error",
            message,
        },
    };
    let body = serde_json::to_string(&event)?;
    Ok(vec![format!("event: error\ndata: {body}\n\n").into_bytes()])
}
