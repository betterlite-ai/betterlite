use betterlite_anthropic::api::messages_errors::decode::{WireDecodeError, WireDecodeLimits};
use betterlite_anthropic::api::messages_errors::frame::anthropic_error_sse_frame;

pub const INCOMPLETE_STREAM_MESSAGE: &str = "Upstream stream ended before completion";

// LiteLLM reads status and wording off a BaseLLMException or MidStreamFallbackError and gives every other failure a 500
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamFailure {
    Reported { status_code: u16, message: String },
    Unclassified { text: String },
}

#[derive(Debug, thiserror::Error)]
pub enum MessagesStreamSplitError {
    #[error("the Anthropic error frame could not be encoded: {source}")]
    Frame {
        #[from]
        source: WireDecodeError,
    },
}

#[must_use]
pub fn error_status_and_message(failure: &StreamFailure) -> (u16, String) {
    match failure {
        StreamFailure::Reported {
            status_code,
            message,
        } => (*status_code, message.clone()),
        StreamFailure::Unclassified { text } if text.is_empty() => {
            (500, INCOMPLETE_STREAM_MESSAGE.to_owned())
        }
        StreamFailure::Unclassified { text } => (500, text.clone()),
    }
}

pub fn mid_stream_error_sse_event(
    failure: &StreamFailure,
    limits: WireDecodeLimits,
) -> Result<String, MessagesStreamSplitError> {
    let (status_code, message) = error_status_and_message(failure);
    let frame = anthropic_error_sse_frame(status_code, &message, limits)?;
    Ok(frame.body)
}
