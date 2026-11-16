use betterlite_core::api::messages_chat_output::records::ResponseRecord;
use betterlite_core::api::messages_chat_output::response::response_message;
use betterlite_core::api::messages_fake_stream::MessagesFakeStream;
use betterlite_models::decisions::Json;

use super::error::AnthropicSseError;

mod tests;

// The assembled response is translated to an Anthropic message and replayed as the frames of a stream.
pub fn anthropic_sse_chunks_from_response(
    assembled: &ResponseRecord,
) -> Result<Vec<Vec<u8>>, AnthropicSseError> {
    let message = response_message(assembled, None, None)?;
    let anthropic: Json = serde_json::from_str(&serde_json::to_string(&message)?)?;
    Ok(MessagesFakeStream::new(&anthropic)?.collect())
}
