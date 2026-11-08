use crate::gemini_tool_invokes::error::GeminiToolInvokesError;
use crate::gemini_tool_invokes::limits::MAX_MESSAGE_BYTES;
use crate::message_content::Message;

pub fn parse_tool_invoke_message(bytes: &[u8]) -> Result<Message, GeminiToolInvokesError> {
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(GeminiToolInvokesError::MessageTooLarge {
            length: bytes.len(),
            limit: MAX_MESSAGE_BYTES,
        });
    }
    serde_json::from_slice(bytes).map_err(|source| GeminiToolInvokesError::MessageDecode { source })
}
