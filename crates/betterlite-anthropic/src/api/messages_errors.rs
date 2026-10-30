mod errors;
mod wire;

pub use errors::{
    AnthropicErrorDetail, AnthropicErrorResponse, AnthropicErrorSseFrame, AnthropicErrorType,
    AnthropicExceptionMapping, anthropic_error_sse_frame,
};
pub use wire::{
    AnthropicMessagesResponse, AnthropicMetadata, ContentBlock, DecodedMessagesErrors,
    MessageUsage, StopReason, WireDecodeError, WireDecodeLimits, decode_messages_errors,
};

use serde_json::{Value, from_str};

pub fn transform_messages_error(
    status_code: u16,
    raw_message: &str,
    request_id: Option<&str>,
) -> AnthropicErrorResponse {
    let parsed = from_str::<Value>(raw_message).ok();
    AnthropicExceptionMapping::transform(status_code, raw_message, parsed, request_id)
}
