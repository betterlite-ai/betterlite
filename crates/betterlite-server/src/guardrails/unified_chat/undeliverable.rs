use betterlite_core::guardrails::streaming_rewrite::failure::UndeliverableStreamRewrite;

use super::UNKNOWN_GUARDRAIL_NAME;
use crate::guardrails::responses_translation::stream_sync::error::StreamSyncError;

// B3672 keeps LiteLLM's `guardrail_name or "unknown"`, so an empty name never reaches the client
#[must_use]
pub fn from_stream_sync(error: StreamSyncError) -> UndeliverableStreamRewrite {
    match error {
        StreamSyncError::Undeliverable {
            guardrail_name,
            reason,
        } => UndeliverableStreamRewrite {
            guardrail_name: if guardrail_name.is_empty() {
                String::from(UNKNOWN_GUARDRAIL_NAME)
            } else {
                guardrail_name
            },
            reason,
        },
    }
}

mod tests;
