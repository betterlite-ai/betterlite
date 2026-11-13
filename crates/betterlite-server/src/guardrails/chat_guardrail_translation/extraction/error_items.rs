use betterlite_guardrails::guardrail_message_scope::Object;

use crate::guardrails::chat_guardrail_translation::block_framing::error::BlockFramingError;

mod tests;

// `error` is the object sse_error_payload built for the exception; the frame wraps it under "error".
pub fn build_stream_error_items(error: &Object) -> Result<Vec<Vec<u8>>, BlockFramingError> {
    let body =
        serde_json::to_string(error).map_err(|source| BlockFramingError::Serialize { source })?;
    Ok(vec![format!("data: {{\"error\":{body}}}\n\n").into_bytes()])
}
