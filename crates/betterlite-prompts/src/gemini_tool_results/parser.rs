use crate::gemini_tool_invokes::limits::MAX_MESSAGE_BYTES;
use crate::gemini_tool_results::error::GeminiToolResultsError;
use crate::message_content::Message;

pub fn parse_tool_result_message(bytes: &[u8]) -> Result<Message, GeminiToolResultsError> {
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(GeminiToolResultsError::MessageTooLarge {
            length: bytes.len(),
            limit: MAX_MESSAGE_BYTES,
        });
    }
    serde_json::from_slice(bytes).map_err(|source| GeminiToolResultsError::MessageDecode { source })
}
